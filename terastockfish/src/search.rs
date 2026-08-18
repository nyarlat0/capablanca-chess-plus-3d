use crate::capacity::{MAX_BOARD_SQUARES, square_index};
use crate::evaluate::EvaluationParameters;
use crate::key::{position_key, position_keys};
use crate::state::SearchPosition;
use crate::tt::{Bound, TranspositionTable};
use capablanca_chess_plus::{Color, Move, MoveKind, Piece, PieceKind, Position, Square};
use std::cmp::Reverse;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, Ordering};
use std::time::{Duration, Instant};

const INFINITY: i32 = 1_100_000;
const MATE_SCORE: i32 = 1_000_000;
const MATE_THRESHOLD: i32 = MATE_SCORE - MAX_PLY as i32;
const MAX_PLY: usize = 192;
const DEFAULT_ASPIRATION: i32 = 60;

#[derive(Clone, Copy, Debug)]
pub struct SearchOptions {
    pub hash_megabytes: usize,
    /// Root-search workers. They share the lock-free transposition table and
    /// global stop/node budget while keeping local history and killer tables.
    pub threads: usize,
}

impl Default for SearchOptions {
    fn default() -> Self {
        Self {
            hash_megabytes: 128,
            threads: 1,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct SearchLimits {
    pub max_depth: u8,
    pub max_nodes: Option<u64>,
    pub move_time: Option<Duration>,
    pub remaining_time: Option<Duration>,
    pub increment: Duration,
    pub moves_to_go: Option<u32>,
    pub infinite: bool,
}

impl SearchLimits {
    #[must_use]
    pub const fn depth(depth: u8) -> Self {
        Self {
            max_depth: depth,
            max_nodes: None,
            move_time: None,
            remaining_time: None,
            increment: Duration::ZERO,
            moves_to_go: None,
            infinite: false,
        }
    }
}

impl Default for SearchLimits {
    fn default() -> Self {
        Self::depth(8)
    }
}

#[derive(Clone, Debug)]
pub struct AnalysisInfo {
    pub depth: u8,
    pub score: i32,
    pub nodes: u64,
    pub elapsed: Duration,
    pub nps: u64,
    pub hashfull: u16,
    pub principal_variation: Vec<Move>,
}

#[derive(Clone, Debug)]
pub struct AnalysisResult {
    pub best_move: Option<Move>,
    pub score: i32,
    pub completed_depth: u8,
    pub nodes: u64,
    pub elapsed: Duration,
    pub principal_variation: Vec<Move>,
    pub stopped: bool,
}

/// Repetition state accumulated by the game containing the position being
/// analyzed. Recording the current position after every real move lets search
/// recognize a third occurrence that depends on history before the root.
#[derive(Clone, Debug, Default)]
pub struct SearchHistory {
    repetition: Vec<u64>,
}

impl SearchHistory {
    #[must_use]
    pub fn new(position: &Position) -> Self {
        Self {
            repetition: vec![position_keys(position).repetition],
        }
    }

    pub fn record(&mut self, position: &Position) {
        self.repetition.push(position_keys(position).repetition);
    }
}

#[derive(Clone, Default)]
pub struct SearchControl {
    stopped: Arc<AtomicBool>,
}

impl SearchControl {
    pub fn stop(&self) {
        self.stopped.store(true, Ordering::Relaxed);
    }

    #[must_use]
    pub fn is_stopped(&self) -> bool {
        self.stopped.load(Ordering::Relaxed)
    }

    fn reset(&self) {
        self.stopped.store(false, Ordering::Relaxed);
    }
}

pub struct Searcher {
    options: SearchOptions,
    evaluation: EvaluationParameters,
    table: Arc<TranspositionTable>,
    control: SearchControl,
    history: Vec<i32>,
    deterministic_nodes: bool,
}

impl Default for Searcher {
    fn default() -> Self {
        Self::new(SearchOptions::default())
    }
}

impl Searcher {
    #[must_use]
    pub fn new(mut options: SearchOptions) -> Self {
        options.hash_megabytes = options.hash_megabytes.max(1);
        options.threads = options.threads.clamp(1, 256);
        Self {
            table: Arc::new(TranspositionTable::new(options.hash_megabytes)),
            options,
            evaluation: EvaluationParameters::production(),
            control: SearchControl::default(),
            history: vec![0; 2 * MAX_BOARD_SQUARES * MAX_BOARD_SQUARES],
            deterministic_nodes: false,
        }
    }

    #[must_use]
    pub fn with_evaluation(options: SearchOptions, evaluation: EvaluationParameters) -> Self {
        let mut searcher = Self::new(options);
        searcher.evaluation = evaluation;
        searcher
    }

    #[must_use]
    pub const fn options(&self) -> SearchOptions {
        self.options
    }

    #[must_use]
    pub const fn evaluation_parameters(&self) -> EvaluationParameters {
        self.evaluation
    }

    /// Replaces evaluation weights and invalidates score-dependent search
    /// state left by the previous profile.
    pub fn set_evaluation_parameters(&mut self, evaluation: EvaluationParameters) {
        if self.evaluation == evaluation {
            return;
        }
        self.evaluation = evaluation;
        self.table.clear();
        self.history.fill(0);
    }

    #[must_use]
    pub fn control(&self) -> SearchControl {
        self.control.clone()
    }

    pub fn clear_hash(&self) {
        self.table.clear();
    }

    pub fn resize_hash(&mut self, megabytes: usize) {
        self.options.hash_megabytes = megabytes.max(1);
        self.table = Arc::new(TranspositionTable::new(self.options.hash_megabytes));
    }

    pub fn set_threads(&mut self, threads: usize) {
        self.options.threads = threads.clamp(1, 256);
    }

    /// Makes pure node-limited searches reproducible for a fixed thread count.
    /// The default strength-oriented shared-TT mode remains enabled when this
    /// option is false or no node limit is supplied.
    pub fn set_deterministic_nodes(&mut self, enabled: bool) {
        self.deterministic_nodes = enabled;
    }

    #[must_use]
    pub const fn deterministic_nodes(&self) -> bool {
        self.deterministic_nodes
    }

    #[must_use]
    pub fn analyze(&mut self, position: &Position, limits: SearchLimits) -> AnalysisResult {
        self.analyze_with(position, limits, |_| {})
    }

    #[must_use]
    pub fn analyze_with_history(
        &mut self,
        position: &Position,
        history: &SearchHistory,
        limits: SearchLimits,
    ) -> AnalysisResult {
        self.analyze_with_optional_history(position, Some(history), limits, |_| {})
    }

    /// [`Self::analyze_with_history`] with an iterative-deepening callback.
    pub fn analyze_with_history_and<F>(
        &mut self,
        position: &Position,
        history: &SearchHistory,
        limits: SearchLimits,
        reporter: F,
    ) -> AnalysisResult
    where
        F: FnMut(&AnalysisInfo),
    {
        self.analyze_with_optional_history(position, Some(history), limits, reporter)
    }

    pub fn analyze_with<F>(
        &mut self,
        position: &Position,
        limits: SearchLimits,
        mut reporter: F,
    ) -> AnalysisResult
    where
        F: FnMut(&AnalysisInfo),
    {
        self.analyze_with_optional_history(position, None, limits, &mut reporter)
    }

    fn analyze_with_optional_history<F>(
        &mut self,
        position: &Position,
        history: Option<&SearchHistory>,
        limits: SearchLimits,
        mut reporter: F,
    ) -> AnalysisResult
    where
        F: FnMut(&AnalysisInfo),
    {
        self.control.reset();
        age_history(&mut self.history);
        let start = Instant::now();
        let budget = TimeBudget::new(start, limits);
        let generation = self.table.next_generation();
        let mut root = SearchPosition::new(position, &self.evaluation);
        let root_repetition = root.keys().repetition;
        let repetition = root_repetition_history(history, root_repetition);
        let deterministic_nodes = self.deterministic_nodes && limits.max_nodes.is_some();
        let mut context = SearchContext {
            table: Arc::clone(&self.table),
            control: self.control.clone(),
            limits,
            budget,
            generation,
            nodes: Arc::new(AtomicU64::new(0)),
            threads: self.options.threads,
            hash_megabytes: self.options.hash_megabytes,
            deterministic_nodes,
            evaluation: self.evaluation,
            history: &mut self.history,
            killers: [[None; 2]; MAX_PLY],
            repetition,
        };

        let root_moves = root.legal_moves();
        if root_moves.is_empty() {
            let score = if root.position().is_in_check(root.position().side_to_move()) {
                -MATE_SCORE
            } else {
                0
            };
            return AnalysisResult {
                best_move: None,
                score,
                completed_depth: 0,
                nodes: 0,
                elapsed: start.elapsed(),
                principal_variation: Vec::new(),
                stopped: false,
            };
        }

        let max_depth = limits.max_depth.clamp(1, (MAX_PLY - 1) as u8);
        let mut best_move = root_moves.first().copied();
        let mut best_score = root.evaluate(&self.evaluation);
        let mut completed_depth = 0;
        let mut principal_variation = best_move.into_iter().collect::<Vec<_>>();
        let mut stopped = false;

        for depth in 1..=max_depth {
            if context.should_stop_now()
                || (deterministic_nodes
                    && limits
                        .max_nodes
                        .is_some_and(|maximum| context.node_count() >= maximum))
            {
                stopped = true;
                break;
            }

            let mut window = if depth >= 3 {
                DEFAULT_ASPIRATION
            } else {
                INFINITY
            };
            let mut alpha = if window == INFINITY {
                -INFINITY
            } else {
                best_score.saturating_sub(window)
            };
            let mut beta = if window == INFINITY {
                INFINITY
            } else {
                best_score.saturating_add(window)
            };

            let iteration = loop {
                match context.search_root(&mut root, &root_moves, depth, alpha, beta, best_move) {
                    Ok(result) if result.score <= alpha && alpha > -INFINITY => {
                        window = (window * 2).min(INFINITY);
                        alpha = if window == INFINITY {
                            -INFINITY
                        } else {
                            best_score.saturating_sub(window).max(-INFINITY)
                        };
                    }
                    Ok(result) if result.score >= beta && beta < INFINITY => {
                        window = (window * 2).min(INFINITY);
                        beta = if window == INFINITY {
                            INFINITY
                        } else {
                            best_score.saturating_add(window).min(INFINITY)
                        };
                    }
                    result => break result,
                }
            };

            let Ok(iteration) = iteration else {
                stopped = true;
                break;
            };
            best_move = Some(iteration.best_move);
            best_score = iteration.score;
            completed_depth = depth;
            principal_variation = context.principal_variation(position, depth);
            let elapsed = start.elapsed();
            let info = AnalysisInfo {
                depth,
                score: best_score,
                nodes: context.node_count(),
                elapsed,
                nps: nodes_per_second(context.node_count(), elapsed),
                hashfull: context.table.hashfull(),
                principal_variation: principal_variation.clone(),
            };
            reporter(&info);

            if context.budget.soft_expired()
                || is_mate_score(best_score)
                || (deterministic_nodes
                    && limits
                        .max_nodes
                        .is_some_and(|maximum| context.node_count() >= maximum))
            {
                break;
            }
        }

        AnalysisResult {
            best_move,
            score: best_score,
            completed_depth,
            nodes: context.node_count(),
            elapsed: start.elapsed(),
            principal_variation,
            stopped,
        }
    }
}

fn root_repetition_history(history: Option<&SearchHistory>, root: u64) -> Vec<u64> {
    history
        .filter(|history| history.repetition.last() == Some(&root))
        .map_or_else(|| vec![root], |history| history.repetition.clone())
}

struct RootResult {
    score: i32,
    best_move: Move,
}

struct SearchContext<'a> {
    table: Arc<TranspositionTable>,
    control: SearchControl,
    limits: SearchLimits,
    budget: TimeBudget,
    generation: u8,
    nodes: Arc<AtomicU64>,
    threads: usize,
    hash_megabytes: usize,
    deterministic_nodes: bool,
    evaluation: EvaluationParameters,
    history: &'a mut [i32],
    killers: [[Option<Move>; 2]; MAX_PLY],
    repetition: Vec<u64>,
}

impl SearchContext<'_> {
    fn search_root(
        &mut self,
        position: &mut SearchPosition,
        root_moves: &[Move],
        depth: u8,
        mut alpha: i32,
        beta: i32,
        previous_best: Option<Move>,
    ) -> Result<RootResult, SearchAborted> {
        if self.threads > 1 && root_moves.len() > 1 && depth > 1 {
            return self.search_root_parallel(
                position,
                root_moves,
                depth,
                alpha,
                beta,
                previous_best,
            );
        }
        let original_alpha = alpha;
        let key = position.keys().analysis;
        let tt_move = self.table.probe(key).and_then(|entry| entry.best_move);
        let mut moves = root_moves.to_vec();
        self.order_moves(
            position.position(),
            &mut moves,
            tt_move.or(previous_best),
            0,
        );
        let mut best_move = moves[0];
        let mut best_score = -INFINITY;

        for (move_index, chess_move) in moves.into_iter().enumerate() {
            self.check_stop()?;
            let undo = position.make_move(chess_move, &self.evaluation);
            let score = (|| {
                if move_index == 0 {
                    return Ok(-self.negamax(
                        position,
                        i32::from(depth) - 1,
                        1,
                        -beta,
                        -alpha,
                        true,
                    )?);
                }
                let mut score =
                    -self.negamax(position, i32::from(depth) - 1, 1, -alpha - 1, -alpha, true)?;
                if score > alpha && score < beta {
                    score =
                        -self.negamax(position, i32::from(depth) - 1, 1, -beta, -alpha, true)?;
                }
                Ok::<_, SearchAborted>(score)
            })();
            position.unmake_move(undo);
            let score = score?;

            if score > best_score {
                best_score = score;
                best_move = chess_move;
            }
            alpha = alpha.max(score);
            if alpha >= beta {
                break;
            }
        }

        let bound = if best_score <= original_alpha {
            Bound::Upper
        } else if best_score >= beta {
            Bound::Lower
        } else {
            Bound::Exact
        };
        self.table.store(
            key,
            i16::from(depth),
            score_to_tt(best_score, 0),
            bound,
            Some(best_move),
            self.generation,
        );
        Ok(RootResult {
            score: best_score,
            best_move,
        })
    }

    fn search_root_parallel(
        &mut self,
        position: &SearchPosition,
        root_moves: &[Move],
        depth: u8,
        alpha: i32,
        beta: i32,
        previous_best: Option<Move>,
    ) -> Result<RootResult, SearchAborted> {
        let original_alpha = alpha;
        let key = position.keys().analysis;
        let tt_move = self.table.probe(key).and_then(|entry| entry.best_move);
        let mut moves = root_moves.to_vec();
        self.order_moves(
            position.position(),
            &mut moves,
            tt_move.or(previous_best),
            0,
        );
        let deterministic_nodes = self.deterministic_nodes;
        let mut leader = None;
        let parallel_moves = if deterministic_nodes {
            moves.as_slice()
        } else {
            let chess_move = moves[0];
            let mut leader_position = position.clone();
            let undo = leader_position.make_move(chess_move, &self.evaluation);
            let score = self
                .negamax(
                    &mut leader_position,
                    i32::from(depth) - 1,
                    1,
                    -beta,
                    -alpha,
                    true,
                )
                .map(|score| -score);
            leader_position.unmake_move(undo);
            leader = Some((chess_move, score?));
            &moves[1..]
        };
        let worker_count = self.threads.min(parallel_moves.len()).max(1);
        let base_history = self.history.to_vec();
        let base_killers = self.killers;
        let table = Arc::clone(&self.table);
        let control = self.control.clone();
        let limits = self.limits;
        let budget = self.budget;
        let generation = self.generation;
        let evaluation = self.evaluation;
        let nodes = Arc::clone(&self.nodes);
        let repetition = self.repetition.clone();
        let worker_hash = self.hash_megabytes.div_ceil(worker_count).max(1);
        let shared_alpha = Arc::new(AtomicI32::new(
            leader.map_or(alpha, |(_, score)| alpha.max(score)),
        ));

        let worker_results = std::thread::scope(|scope| {
            let mut workers = Vec::with_capacity(worker_count);
            for worker_id in 0..worker_count {
                let assigned = parallel_moves
                    .iter()
                    .copied()
                    .skip(worker_id)
                    .step_by(worker_count)
                    .collect::<Vec<_>>();
                let mut position = position.clone();
                let table = if deterministic_nodes {
                    Arc::new(TranspositionTable::new(worker_hash))
                } else {
                    Arc::clone(&table)
                };
                let control = control.clone();
                let worker_nodes = if deterministic_nodes {
                    Arc::new(AtomicU64::new(0))
                } else {
                    Arc::clone(&nodes)
                };
                let repetition = repetition.clone();
                let shared_alpha = Arc::clone(&shared_alpha);
                let mut history = base_history.clone();
                workers.push(scope.spawn(move || {
                    let mut worker = SearchContext {
                        table,
                        control,
                        limits,
                        budget,
                        generation,
                        nodes: Arc::clone(&worker_nodes),
                        threads: 1,
                        hash_megabytes: worker_hash,
                        deterministic_nodes,
                        evaluation,
                        history: &mut history,
                        killers: base_killers,
                        repetition,
                    };
                    let mut scores = Vec::with_capacity(assigned.len());
                    for chess_move in assigned {
                        worker.check_stop()?;
                        let undo = position.make_move(chess_move, &worker.evaluation);
                        let score = if deterministic_nodes {
                            worker
                                .negamax(
                                    &mut position,
                                    i32::from(depth) - 1,
                                    1,
                                    -beta,
                                    -alpha,
                                    true,
                                )
                                .map(|score| -score)
                        } else {
                            let local_alpha = shared_alpha.load(Ordering::Relaxed);
                            let mut score = -worker.negamax(
                                &mut position,
                                i32::from(depth) - 1,
                                1,
                                -local_alpha - 1,
                                -local_alpha,
                                true,
                            )?;
                            if score > local_alpha && score < beta {
                                score = -worker.negamax(
                                    &mut position,
                                    i32::from(depth) - 1,
                                    1,
                                    -beta,
                                    -local_alpha,
                                    true,
                                )?;
                            }
                            shared_alpha.fetch_max(score, Ordering::Relaxed);
                            Ok(score)
                        };
                        position.unmake_move(undo);
                        let score = score?;
                        scores.push((chess_move, score));
                    }
                    Ok::<_, SearchAborted>((
                        scores,
                        worker.killers,
                        deterministic_nodes.then(|| worker_nodes.load(Ordering::Relaxed)),
                    ))
                }));
            }
            workers
                .into_iter()
                .map(|worker| worker.join().expect("scoped root worker must not panic"))
                .collect::<Vec<_>>()
        });

        let mut best_move = leader.map_or(moves[0], |(chess_move, _)| chess_move);
        let mut best_score = leader.map_or(-INFINITY, |(_, score)| score);
        for result in worker_results {
            let (scores, killers, deterministic_worker_nodes) = result?;
            if let Some(worker_nodes) = deterministic_worker_nodes {
                self.nodes.fetch_add(worker_nodes, Ordering::Relaxed);
            }
            self.killers = killers;
            for (chess_move, score) in scores {
                if score > best_score {
                    best_score = score;
                    best_move = chess_move;
                }
            }
        }
        let bound = if best_score <= original_alpha {
            Bound::Upper
        } else if best_score >= beta {
            Bound::Lower
        } else {
            Bound::Exact
        };
        self.table.store(
            key,
            i16::from(depth),
            score_to_tt(best_score, 0),
            bound,
            Some(best_move),
            self.generation,
        );
        Ok(RootResult {
            score: best_score,
            best_move,
        })
    }

    fn negamax(
        &mut self,
        position: &mut SearchPosition,
        mut depth: i32,
        ply: usize,
        mut alpha: i32,
        beta: i32,
        allow_null: bool,
    ) -> Result<i32, SearchAborted> {
        self.visit_node()?;
        let keys = position.keys();
        if position.position().halfmove_clock() >= 100 || self.is_repetition_key(keys.repetition) {
            return Ok(0);
        }
        if ply >= MAX_PLY - 1 {
            return Ok(position.evaluate(&self.evaluation));
        }

        self.repetition.push(keys.repetition);
        let result = self.negamax_current(
            position,
            keys.analysis,
            &mut depth,
            ply,
            &mut alpha,
            beta,
            allow_null,
        );
        self.repetition.pop();
        result
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "explicit alpha-beta state keeps recursive search calls auditable"
    )]
    fn negamax_current(
        &mut self,
        position: &mut SearchPosition,
        key: u64,
        depth: &mut i32,
        ply: usize,
        alpha: &mut i32,
        beta: i32,
        allow_null: bool,
    ) -> Result<i32, SearchAborted> {
        let in_check = position
            .position()
            .is_in_check(position.position().side_to_move());
        if in_check && *depth > 0 {
            *depth += 1;
        }
        if *depth <= 0 {
            return self.quiescence_current(position, ply, *alpha, beta);
        }

        let tt_entry = self.table.probe(key);
        if let Some(entry) = tt_entry
            && i32::from(entry.depth) >= *depth
        {
            let score = score_from_tt(entry.score, ply);
            match entry.bound {
                Bound::Exact => return Ok(score),
                Bound::Lower if score >= beta => return Ok(score),
                Bound::Upper if score <= *alpha => return Ok(score),
                Bound::Lower | Bound::Upper => {}
            }
        }

        let mut moves = position.legal_moves();
        if moves.is_empty() {
            return Ok(if in_check {
                -MATE_SCORE + ply as i32
            } else {
                0
            });
        }

        let static_evaluation = position.evaluate(&self.evaluation);
        if allow_null
            && !in_check
            && *depth >= 3
            && beta < MATE_THRESHOLD
            && position.position().halfmove_clock() < 90
            && static_evaluation >= beta
            && has_non_pawn_material(position.position())
        {
            let reduction = 2 + *depth / 4;
            let undo = position.make_null_move();
            let null_repetition = vec![position.keys().repetition];
            let real_repetition = std::mem::replace(&mut self.repetition, null_repetition);
            let null_score = self
                .negamax(
                    position,
                    (*depth - 1 - reduction).max(0),
                    ply + 1,
                    -beta,
                    -beta + 1,
                    false,
                )
                .map(|score| -score);
            self.repetition = real_repetition;
            position.unmake_move(undo);
            let null_score = null_score?;
            if null_score >= beta {
                if *depth < 6 {
                    return Ok(null_score);
                }
                let mut verification_depth = (*depth - reduction).max(1);
                let mut verification_alpha = beta - 1;
                let verification = self.negamax_current(
                    position,
                    key,
                    &mut verification_depth,
                    ply,
                    &mut verification_alpha,
                    beta,
                    false,
                )?;
                if verification >= beta {
                    return Ok(verification);
                }
            }
        }
        if !in_check && *depth <= 3 {
            let margin = 140 * *depth;
            if static_evaluation - margin >= beta {
                return Ok(static_evaluation);
            }
        }
        if !in_check && *depth == 1 && static_evaluation + 180 <= *alpha {
            return self.quiescence_current(position, ply, *alpha, beta);
        }

        self.order_moves(
            position.position(),
            &mut moves,
            tt_entry.and_then(|entry| entry.best_move),
            ply,
        );
        let original_alpha = *alpha;
        let mut best_score = -INFINITY;
        let mut best_move = None;
        let moving_color = position.position().side_to_move();
        let mut searched_quiets = Vec::new();

        for (move_index, chess_move) in moves.into_iter().enumerate() {
            let capture = captured_piece(position.position(), chess_move).is_some();
            let quiet = !capture && chess_move.promotion.is_none();
            let undo = position.make_move(chess_move, &self.evaluation);
            let gives_check = position
                .position()
                .is_in_check(position.position().side_to_move());
            let futility = quiet
                && !in_check
                && !gives_check
                && move_index > 0
                && ((*depth == 1 && static_evaluation + 180 <= *alpha)
                    || (*depth == 2 && move_index >= 20 && static_evaluation + 320 <= *alpha));
            if futility {
                position.unmake_move(undo);
                continue;
            }

            let score = (|| {
                if move_index == 0 {
                    return Ok(-self.negamax(
                        position,
                        *depth - 1,
                        ply + 1,
                        -beta,
                        -*alpha,
                        true,
                    )?);
                }
                let reduction = reduction(*depth, move_index, quiet, in_check, gives_check);
                let mut score = -self.negamax(
                    position,
                    *depth - 1 - reduction,
                    ply + 1,
                    -*alpha - 1,
                    -*alpha,
                    true,
                )?;
                if reduction > 0 && score > *alpha {
                    score =
                        -self.negamax(position, *depth - 1, ply + 1, -*alpha - 1, -*alpha, true)?;
                }
                if score > *alpha && score < beta {
                    score = -self.negamax(position, *depth - 1, ply + 1, -beta, -*alpha, true)?;
                }
                Ok::<_, SearchAborted>(score)
            })();
            position.unmake_move(undo);
            let score = score?;
            if quiet {
                searched_quiets.push(chess_move);
            }

            if score > best_score {
                best_score = score;
                best_move = Some(chess_move);
            }
            if score > *alpha {
                *alpha = score;
                if quiet {
                    self.reward_history(moving_color, chess_move, *depth);
                }
            }
            if *alpha >= beta {
                if quiet {
                    for failed in searched_quiets
                        .iter()
                        .copied()
                        .filter(|failed| *failed != chess_move)
                    {
                        self.penalize_history(moving_color, failed, *depth);
                    }
                    self.record_killer(ply, chess_move);
                    self.reward_history(moving_color, chess_move, *depth + 2);
                }
                break;
            }
        }

        let bound = if best_score <= original_alpha {
            Bound::Upper
        } else if best_score >= beta {
            Bound::Lower
        } else {
            Bound::Exact
        };
        self.table.store(
            key,
            (*depth).clamp(0, 255) as i16,
            score_to_tt(best_score, ply),
            bound,
            best_move,
            self.generation,
        );
        Ok(best_score)
    }

    fn quiescence_current(
        &mut self,
        position: &mut SearchPosition,
        ply: usize,
        mut alpha: i32,
        beta: i32,
    ) -> Result<i32, SearchAborted> {
        let in_check = position
            .position()
            .is_in_check(position.position().side_to_move());
        let stand_pat = position.evaluate(&self.evaluation);
        if !in_check {
            if stand_pat >= beta {
                return Ok(stand_pat);
            }
            alpha = alpha.max(stand_pat);
        }

        let mut moves = if in_check {
            let moves = position.legal_moves();
            if moves.is_empty() {
                return Ok(-MATE_SCORE + ply as i32);
            }
            moves
        } else {
            let moves = position.legal_tactical_moves();
            if moves.is_empty() {
                return Ok(alpha);
            }
            moves
        };
        self.order_moves(position.position(), &mut moves, None, ply);

        for chess_move in moves {
            if !in_check
                && let Some(victim) = captured_piece(position.position(), chess_move)
                && stand_pat + self.evaluation.material_value(victim.kind) + 180 < alpha
                && chess_move.promotion.is_none()
            {
                continue;
            }
            if !in_check && let Some(victim) = captured_piece(position.position(), chess_move) {
                let attacker = position
                    .position()
                    .board()
                    .piece_at(chess_move.from)
                    .expect("generated capture must have an attacker");
                if self.evaluation.material_value(attacker.kind)
                    > self.evaluation.material_value(victim.kind)
                    && static_exchange_evaluation(position, chess_move, &self.evaluation) < 0
                {
                    continue;
                }
            }
            let undo = position.make_move(chess_move, &self.evaluation);
            let child_score = self.quiescence(position, ply + 1, -beta, -alpha);
            position.unmake_move(undo);
            let score = -child_score?;
            if score >= beta {
                return Ok(score);
            }
            alpha = alpha.max(score);
        }
        Ok(alpha)
    }

    fn quiescence(
        &mut self,
        position: &mut SearchPosition,
        ply: usize,
        alpha: i32,
        beta: i32,
    ) -> Result<i32, SearchAborted> {
        self.visit_node()?;
        let repetition = position.keys().repetition;
        if position.position().halfmove_clock() >= 100 || self.is_repetition_key(repetition) {
            return Ok(0);
        }
        if ply >= MAX_PLY - 1 {
            return Ok(position.evaluate(&self.evaluation));
        }
        self.repetition.push(repetition);
        let result = self.quiescence_current(position, ply, alpha, beta);
        self.repetition.pop();
        result
    }

    fn order_moves(
        &self,
        position: &Position,
        moves: &mut [Move],
        tt_move: Option<Move>,
        ply: usize,
    ) {
        moves.sort_unstable_by_key(|chess_move| {
            Reverse(self.move_order_score(position, *chess_move, tt_move, ply))
        });
    }

    fn move_order_score(
        &self,
        position: &Position,
        chess_move: Move,
        tt_move: Option<Move>,
        ply: usize,
    ) -> i32 {
        if Some(chess_move) == tt_move {
            return 2_000_000;
        }
        if let Some(victim) = captured_piece(position, chess_move) {
            let attacker = position
                .board()
                .piece_at(chess_move.from)
                .expect("generated move has an attacker");
            return 1_000_000 + self.evaluation.material_value(victim.kind) * 16
                - self.evaluation.material_value(attacker.kind);
        }
        if let Some(promotion) = chess_move.promotion {
            return 900_000 + self.evaluation.material_value(promotion);
        }
        if ply < MAX_PLY {
            if self.killers[ply][0] == Some(chess_move) {
                return 800_000;
            }
            if self.killers[ply][1] == Some(chess_move) {
                return 799_000;
            }
        }
        self.history[history_index(position.side_to_move(), chess_move)]
    }

    fn record_killer(&mut self, ply: usize, chess_move: Move) {
        if ply >= MAX_PLY || self.killers[ply][0] == Some(chess_move) {
            return;
        }
        self.killers[ply][1] = self.killers[ply][0];
        self.killers[ply][0] = Some(chess_move);
    }

    fn reward_history(&mut self, color: Color, chess_move: Move, depth: i32) {
        let index = history_index(color, chess_move);
        let bonus = (depth * depth).clamp(1, 400);
        self.history[index] = (self.history[index] + bonus).min(32_000);
    }

    fn penalize_history(&mut self, color: Color, chess_move: Move, depth: i32) {
        let index = history_index(color, chess_move);
        let penalty = (depth * depth).clamp(1, 400);
        self.history[index] = (self.history[index] - penalty).max(-32_000);
    }

    fn principal_variation(&self, position: &Position, depth: u8) -> Vec<Move> {
        let mut current = position.clone();
        let mut pv = Vec::with_capacity(usize::from(depth));
        for _ in 0..depth {
            let Some(chess_move) = self
                .table
                .probe(position_key(&current))
                .and_then(|entry| entry.best_move)
            else {
                break;
            };
            let legal = current.legal_moves();
            if !legal.contains(&chess_move) {
                break;
            }
            pv.push(chess_move);
            current = current.after_legal_move(chess_move);
        }
        pv
    }

    fn is_repetition_key(&self, key: u64) -> bool {
        self.repetition
            .iter()
            .filter(|candidate| **candidate == key)
            .count()
            >= 2
    }

    fn visit_node(&mut self) -> Result<(), SearchAborted> {
        let nodes = self.nodes.fetch_add(1, Ordering::Relaxed).saturating_add(1);
        if (!self.deterministic_nodes && self.limits.max_nodes.is_some_and(|limit| nodes >= limit))
            || (nodes & 0x3ff == 0 && self.should_stop_now())
        {
            return Err(SearchAborted);
        }
        Ok(())
    }

    fn check_stop(&self) -> Result<(), SearchAborted> {
        if self.should_stop_now() {
            Err(SearchAborted)
        } else {
            Ok(())
        }
    }

    fn should_stop_now(&self) -> bool {
        self.control.is_stopped() || self.budget.hard_expired()
    }

    fn node_count(&self) -> u64 {
        self.nodes.load(Ordering::Relaxed)
    }
}

#[derive(Clone, Copy)]
struct TimeBudget {
    soft_deadline: Option<Instant>,
    hard_deadline: Option<Instant>,
}

impl TimeBudget {
    fn new(start: Instant, limits: SearchLimits) -> Self {
        if limits.infinite {
            return Self {
                soft_deadline: None,
                hard_deadline: None,
            };
        }
        if let Some(move_time) = limits.move_time {
            let hard = move_time.saturating_sub(Duration::from_millis(2));
            return Self {
                soft_deadline: Some(start + hard.mul_f32(0.85)),
                hard_deadline: Some(start + hard),
            };
        }
        let Some(remaining) = limits.remaining_time else {
            return Self {
                soft_deadline: None,
                hard_deadline: None,
            };
        };
        let usable = remaining.saturating_sub(Duration::from_millis(20));
        let moves = limits.moves_to_go.unwrap_or(30).max(1);
        let allocation = usable / moves + limits.increment.mul_f32(0.75);
        let soft = allocation.min(usable.mul_f32(0.45));
        let hard = soft.mul_f32(3.0).min(usable);
        Self {
            soft_deadline: Some(start + soft),
            hard_deadline: Some(start + hard),
        }
    }

    fn soft_expired(self) -> bool {
        self.soft_deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
    }

    fn hard_expired(self) -> bool {
        self.hard_deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
    }
}

#[derive(Debug)]
struct SearchAborted;

fn captured_piece(position: &Position, chess_move: Move) -> Option<Piece> {
    match chess_move.kind {
        MoveKind::EnPassant => position
            .board()
            .piece_at(Square::new(chess_move.to.file(), chess_move.from.rank())),
        MoveKind::Normal | MoveKind::Castle(_) => position.board().piece_at(chess_move.to),
    }
}

fn static_exchange_evaluation(
    position: &mut SearchPosition,
    chess_move: Move,
    evaluation: &EvaluationParameters,
) -> i32 {
    let Some(victim) = captured_piece(position.position(), chess_move) else {
        return 0;
    };
    let attacker = position
        .position()
        .board()
        .piece_at(chess_move.from)
        .expect("generated capture must have an attacker");
    let promotion_gain = chess_move.promotion.map_or(0, |promotion| {
        evaluation.material_value(promotion) - evaluation.material_value(attacker.kind)
    });
    let gain = evaluation.material_value(victim.kind) + promotion_gain;
    let undo = position.make_move(chess_move, evaluation);
    let reply = exchange_gain(position, chess_move.to, evaluation, 0);
    position.unmake_move(undo);
    gain - reply
}

fn exchange_gain(
    position: &mut SearchPosition,
    target: Square,
    evaluation: &EvaluationParameters,
    ply: u8,
) -> i32 {
    if ply >= 16 {
        return 0;
    }
    let Some(victim) = position.position().board().piece_at(target) else {
        return 0;
    };
    let victim_value = evaluation.material_value(victim.kind);
    let mut recaptures = position
        .legal_tactical_moves()
        .into_iter()
        .filter(|chess_move| chess_move.to == target)
        .collect::<Vec<_>>();
    recaptures.sort_unstable_by_key(|chess_move| {
        position
            .position()
            .board()
            .piece_at(chess_move.from)
            .map_or(i32::MAX, |piece| evaluation.material_value(piece.kind))
    });
    let mut best = 0;
    for chess_move in recaptures {
        let attacker = position
            .position()
            .board()
            .piece_at(chess_move.from)
            .expect("generated recapture must have an attacker");
        let promotion_gain = chess_move.promotion.map_or(0, |promotion| {
            evaluation.material_value(promotion) - evaluation.material_value(attacker.kind)
        });
        let undo = position.make_move(chess_move, evaluation);
        let continuation = exchange_gain(position, target, evaluation, ply + 1);
        position.unmake_move(undo);
        best = best.max(victim_value + promotion_gain - continuation);
    }
    best
}

fn has_non_pawn_material(position: &Position) -> bool {
    let side = position.side_to_move();
    position.board().pieces().any(|(_, piece)| {
        piece.color == side && !matches!(piece.kind, PieceKind::Pawn | PieceKind::King)
    })
}

fn history_index(color: Color, chess_move: Move) -> usize {
    color.index() * MAX_BOARD_SQUARES * MAX_BOARD_SQUARES
        + square_index(chess_move.from) * MAX_BOARD_SQUARES
        + square_index(chess_move.to)
}

fn reduction(depth: i32, move_index: usize, quiet: bool, in_check: bool, gives_check: bool) -> i32 {
    if depth < 3 || move_index < 3 || !quiet || in_check || gives_check {
        return 0;
    }
    let reduction = 1
        + i32::from(depth >= 6)
        + i32::from(depth >= 10)
        + i32::from(move_index >= 10)
        + i32::from(move_index >= 24);
    reduction.min(depth - 2)
}

fn age_history(history: &mut [i32]) {
    for value in history {
        *value /= 2;
    }
}

fn score_to_tt(score: i32, ply: usize) -> i32 {
    if score >= MATE_THRESHOLD {
        score + ply as i32
    } else if score <= -MATE_THRESHOLD {
        score - ply as i32
    } else {
        score
    }
}

fn score_from_tt(score: i32, ply: usize) -> i32 {
    if score >= MATE_THRESHOLD {
        score - ply as i32
    } else if score <= -MATE_THRESHOLD {
        score + ply as i32
    } else {
        score
    }
}

#[must_use]
pub fn is_mate_score(score: i32) -> bool {
    score.unsigned_abs() >= MATE_THRESHOLD as u32
}

#[must_use]
pub fn mate_distance(score: i32) -> Option<i32> {
    is_mate_score(score).then(|| {
        let plies = MATE_SCORE - score.abs();
        let moves = (plies + 1) / 2;
        if score > 0 { moves } else { -moves }
    })
}

fn nodes_per_second(nodes: u64, elapsed: Duration) -> u64 {
    let micros = elapsed.as_micros().max(1);
    (u128::from(nodes) * 1_000_000 / micros).min(u128::from(u64::MAX)) as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use capablanca_chess_plus::Variant;

    #[test]
    fn mate_scores_round_trip_through_the_table_at_different_plies() {
        for score in [MATE_SCORE - 7, -MATE_SCORE + 11, 123, -456] {
            assert_eq!(score_from_tt(score_to_tt(score, 13), 13), score);
        }
    }

    #[test]
    fn node_limit_interrupts_without_discarding_the_last_completed_iteration() {
        let mut searcher = Searcher::new(SearchOptions {
            hash_megabytes: 1,
            threads: 1,
        });
        let result = searcher.analyze(
            &Variant::TerachessII.starting_position(),
            SearchLimits {
                max_depth: 8,
                max_nodes: Some(500),
                ..SearchLimits::default()
            },
        );
        assert!(result.stopped);
        assert!(result.completed_depth >= 1);
        assert!(result.best_move.is_some());
    }

    #[test]
    fn matching_history_is_used_once_and_stale_history_is_rejected() {
        let mut position = Variant::TerachessII.starting_position();
        let root = position_keys(&position).repetition;
        let mut history = SearchHistory::new(&position);
        history.record(&position);
        assert_eq!(
            root_repetition_history(Some(&history), root),
            vec![root, root]
        );

        let chess_move = position.legal_moves()[0];
        position.play(chess_move).unwrap();
        let different_root = position_keys(&position).repetition;
        assert_eq!(
            root_repetition_history(Some(&history), different_root),
            vec![different_root]
        );
    }

    #[test]
    fn static_exchange_rejects_a_defended_low_value_capture() {
        let position = Position::from_fen(
            Variant::Gothic.rules(),
            "9k/4r5/10/10/4p5/4Q5/10/K9 w - - 0 1",
        )
        .unwrap();
        let chess_move = position.parse_uci_move("e3e4").unwrap();
        let evaluation = EvaluationParameters::production();
        let mut search_position = SearchPosition::new(&position, &evaluation);
        assert!(static_exchange_evaluation(&mut search_position, chess_move, &evaluation) < 0);
        assert_eq!(search_position.position(), &position);
    }

    #[test]
    fn deterministic_node_mode_repeats_with_parallel_workers() {
        let position = Variant::TerachessII.starting_position();
        let run = |threads| {
            let mut searcher = Searcher::new(SearchOptions {
                hash_megabytes: 4,
                threads,
            });
            assert!(!searcher.deterministic_nodes());
            searcher.set_deterministic_nodes(true);
            let result = searcher.analyze(
                &position,
                SearchLimits {
                    max_depth: 8,
                    max_nodes: Some(5_000),
                    ..SearchLimits::default()
                },
            );
            (
                result.best_move,
                result.score,
                result.completed_depth,
                result.nodes,
                result.principal_variation,
            )
        };
        for threads in [1, 2, 4, 12] {
            let expected = run(threads);
            assert!(expected.3 >= 5_000);
            for _ in 0..3 {
                assert_eq!(run(threads), expected, "Threads={threads}");
            }
        }
    }
}
