use crate::capacity::{MAX_BOARD_SQUARES, square_index};
use crate::evaluate::EvaluationParameters;
use crate::key::position_key;
use crate::state::SearchPosition;
use crate::tt::{Bound, TranspositionTable};
use capablanca_chess_plus::{Color, Move, MoveKind, Piece, Position, Square};
use std::cmp::Reverse;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
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

    #[must_use]
    pub fn analyze(&mut self, position: &Position, limits: SearchLimits) -> AnalysisResult {
        self.analyze_with(position, limits, |_| {})
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
        self.control.reset();
        age_history(&mut self.history);
        let start = Instant::now();
        let budget = TimeBudget::new(start, limits);
        let generation = self.table.next_generation();
        let mut root = SearchPosition::new(position, &self.evaluation);
        let mut context = SearchContext {
            table: Arc::clone(&self.table),
            control: self.control.clone(),
            limits,
            budget,
            generation,
            nodes: Arc::new(AtomicU64::new(0)),
            threads: self.options.threads,
            evaluation: self.evaluation,
            history: &mut self.history,
            killers: [[None; 2]; MAX_PLY],
            repetition: vec![root.keys().repetition],
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
            if context.should_stop_now() {
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

            if context.budget.soft_expired() || is_mate_score(best_score) {
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
                    return Ok(-self.negamax(position, i32::from(depth) - 1, 1, -beta, -alpha)?);
                }
                let mut score =
                    -self.negamax(position, i32::from(depth) - 1, 1, -alpha - 1, -alpha)?;
                if score > alpha && score < beta {
                    score = -self.negamax(position, i32::from(depth) - 1, 1, -beta, -alpha)?;
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
        let worker_count = self.threads.min(moves.len());
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

        let worker_results = std::thread::scope(|scope| {
            let mut workers = Vec::with_capacity(worker_count);
            for worker_id in 0..worker_count {
                let assigned = moves
                    .iter()
                    .copied()
                    .skip(worker_id)
                    .step_by(worker_count)
                    .collect::<Vec<_>>();
                let mut position = position.clone();
                let table = Arc::clone(&table);
                let control = control.clone();
                let nodes = Arc::clone(&nodes);
                let repetition = repetition.clone();
                let mut history = base_history.clone();
                workers.push(scope.spawn(move || {
                    let mut worker = SearchContext {
                        table,
                        control,
                        limits,
                        budget,
                        generation,
                        nodes,
                        threads: 1,
                        evaluation,
                        history: &mut history,
                        killers: base_killers,
                        repetition,
                    };
                    let mut scores = Vec::with_capacity(assigned.len());
                    for chess_move in assigned {
                        worker.check_stop()?;
                        let undo = position.make_move(chess_move, &worker.evaluation);
                        let score = worker
                            .negamax(&mut position, i32::from(depth) - 1, 1, -beta, -alpha)
                            .map(|score| -score);
                        position.unmake_move(undo);
                        let score = score?;
                        scores.push((chess_move, score));
                    }
                    Ok::<_, SearchAborted>((scores, worker.killers))
                }));
            }
            workers
                .into_iter()
                .map(|worker| worker.join().expect("scoped root worker must not panic"))
                .collect::<Vec<_>>()
        });

        let mut best_move = moves[0];
        let mut best_score = -INFINITY;
        for result in worker_results {
            let (scores, killers) = result?;
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
        let result =
            self.negamax_current(position, keys.analysis, &mut depth, ply, &mut alpha, beta);
        self.repetition.pop();
        result
    }

    fn negamax_current(
        &mut self,
        position: &mut SearchPosition,
        key: u64,
        depth: &mut i32,
        ply: usize,
        alpha: &mut i32,
        beta: i32,
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

        for (move_index, chess_move) in moves.into_iter().enumerate() {
            let capture = captured_piece(position.position(), chess_move).is_some();
            let quiet = !capture && chess_move.promotion.is_none();
            let undo = position.make_move(chess_move, &self.evaluation);
            let gives_check = position
                .position()
                .is_in_check(position.position().side_to_move());

            let score = (|| {
                if move_index == 0 {
                    return Ok(-self.negamax(position, *depth - 1, ply + 1, -beta, -*alpha)?);
                }
                let reduction = reduction(*depth, move_index, quiet, in_check, gives_check);
                let mut score = -self.negamax(
                    position,
                    *depth - 1 - reduction,
                    ply + 1,
                    -*alpha - 1,
                    -*alpha,
                )?;
                if reduction > 0 && score > *alpha {
                    score = -self.negamax(position, *depth - 1, ply + 1, -*alpha - 1, -*alpha)?;
                }
                if score > *alpha && score < beta {
                    score = -self.negamax(position, *depth - 1, ply + 1, -beta, -*alpha)?;
                }
                Ok::<_, SearchAborted>(score)
            })();
            position.unmake_move(undo);
            let score = score?;

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

        let mut moves = position.legal_moves();
        if moves.is_empty() {
            return Ok(if in_check {
                -MATE_SCORE + ply as i32
            } else {
                0
            });
        }
        if !in_check {
            moves.retain(|chess_move| {
                captured_piece(position.position(), *chess_move).is_some()
                    || chess_move.promotion.is_some()
            });
        }
        self.order_moves(position.position(), &mut moves, None, ply);

        for chess_move in moves {
            if !in_check
                && let Some(victim) = captured_piece(position.position(), chess_move)
                && stand_pat + self.evaluation.material_value(victim.kind) + 180 < alpha
                && chess_move.promotion.is_none()
            {
                continue;
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
        if self.limits.max_nodes.is_some_and(|limit| nodes >= limit)
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

fn history_index(color: Color, chess_move: Move) -> usize {
    color.index() * MAX_BOARD_SQUARES * MAX_BOARD_SQUARES
        + square_index(chess_move.from) * MAX_BOARD_SQUARES
        + square_index(chess_move.to)
}

fn reduction(depth: i32, move_index: usize, quiet: bool, in_check: bool, gives_check: bool) -> i32 {
    if depth < 3 || move_index < 4 || !quiet || in_check || gives_check {
        return 0;
    }
    let depth_term = if depth >= 6 { 1 } else { 0 };
    let move_term = if move_index >= 12 { 1 } else { 0 };
    (1 + depth_term + move_term).min(depth - 2)
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
}
