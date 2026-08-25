//! Fixed-sample, color-swapped matches for Teressa checkpoints.

use crate::{HybridAgent, HybridOptions, PlanKind, PlanTransition, VulkanAdvisor};
use capablanca_chess_plus::{Color, DrawReason, Game, GameOutcome, Move, Position, Variant};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use terastockfish::{SearchHistory, SearchLimits, SearchOptions, Searcher};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ArenaPlayerSpec {
    TeraStockfish,
    Model(PathBuf),
}

impl ArenaPlayerSpec {
    pub fn parse(value: &str) -> Result<Self, String> {
        if value.eq_ignore_ascii_case("stockfish")
            || value.eq_ignore_ascii_case("terastockfish")
            || value.eq_ignore_ascii_case("pure")
        {
            Ok(Self::TeraStockfish)
        } else if value.trim().is_empty() {
            Err("arena player cannot be empty".to_owned())
        } else {
            Ok(Self::Model(PathBuf::from(value)))
        }
    }

    #[must_use]
    pub fn label(&self) -> String {
        match self {
            Self::TeraStockfish => "terastockfish".to_owned(),
            Self::Model(path) => path.display().to_string(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ArenaConfig {
    pub nodes_per_move: u64,
    pub maximum_plies: u16,
    pub opening_plies: u8,
    pub hash_megabytes: usize,
    pub search_threads: usize,
    pub adjudication_score: i32,
    pub adjudication_plies: u8,
    pub blunder_threshold_cp: i32,
    pub seed: u64,
}

impl Default for ArenaConfig {
    fn default() -> Self {
        Self {
            nodes_per_move: 50_000,
            maximum_plies: 0,
            opening_plies: 8,
            hash_megabytes: 32,
            search_threads: 1,
            adjudication_score: 1_500,
            adjudication_plies: 20,
            blunder_threshold_cp: 180,
            seed: 0x5445_5245_5353_4133,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArenaResult {
    Win,
    Draw,
    Loss,
}

impl ArenaResult {
    #[must_use]
    pub const fn score(self) -> f64 {
        match self {
            Self::Win => 1.0,
            Self::Draw => 0.5,
            Self::Loss => 0.0,
        }
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Win => "win",
            Self::Draw => "draw",
            Self::Loss => "loss",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArenaTermination {
    Checkmate,
    Stalemate,
    FiftyMoveRule,
    ThreefoldRepetition,
    ScoreAdjudication,
    PlyLimit,
}

impl ArenaTermination {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Checkmate => "checkmate",
            Self::Stalemate => "stalemate",
            Self::FiftyMoveRule => "fifty_move",
            Self::ThreefoldRepetition => "threefold",
            Self::ScoreAdjudication => "score_adjudication",
            Self::PlyLimit => "ply_limit",
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ArenaTelemetry {
    pub moves: u64,
    pub nodes: u64,
    pub neural_moves: u64,
    pub safety_vetoes: u64,
    pub blunders: u64,
    pub centipawn_loss_sum: u64,
    pub plan_starts: u64,
    pub plan_continuations: u64,
    pub plan_completions: u64,
    pub plan_horizon_expirations: u64,
    pub plan_invalidations: u64,
    pub plan_supersessions: u64,
    pub plan_safety_cancellations: u64,
    pub ended_plan_duration_sum: u64,
    pub ended_plans: u64,
    pub plan_kind_moves: [u64; PlanKind::COUNT],
}

impl ArenaTelemetry {
    pub fn add_assign(&mut self, other: &Self) {
        self.moves += other.moves;
        self.nodes += other.nodes;
        self.neural_moves += other.neural_moves;
        self.safety_vetoes += other.safety_vetoes;
        self.blunders += other.blunders;
        self.centipawn_loss_sum += other.centipawn_loss_sum;
        self.plan_starts += other.plan_starts;
        self.plan_continuations += other.plan_continuations;
        self.plan_completions += other.plan_completions;
        self.plan_horizon_expirations += other.plan_horizon_expirations;
        self.plan_invalidations += other.plan_invalidations;
        self.plan_supersessions += other.plan_supersessions;
        self.plan_safety_cancellations += other.plan_safety_cancellations;
        self.ended_plan_duration_sum += other.ended_plan_duration_sum;
        self.ended_plans += other.ended_plans;
        for (total, value) in self.plan_kind_moves.iter_mut().zip(other.plan_kind_moves) {
            *total += value;
        }
    }

    #[must_use]
    pub fn veto_rate(&self) -> f64 {
        ratio(self.safety_vetoes, self.neural_moves)
    }

    #[must_use]
    pub fn blunder_rate(&self) -> f64 {
        ratio(self.blunders, self.neural_moves)
    }

    #[must_use]
    pub fn mean_centipawn_loss(&self) -> f64 {
        ratio(self.centipawn_loss_sum, self.neural_moves)
    }

    #[must_use]
    pub fn mean_ended_plan_duration(&self) -> f64 {
        ratio(self.ended_plan_duration_sum, self.ended_plans)
    }
}

fn ratio(numerator: u64, denominator: u64) -> f64 {
    if denominator == 0 {
        0.0
    } else {
        numerator as f64 / denominator as f64
    }
}

#[derive(Clone, Debug)]
pub struct ArenaGame {
    pub pair: u32,
    pub candidate_color: Color,
    pub result: ArenaResult,
    pub termination: ArenaTermination,
    pub plies: u16,
    pub elapsed: Duration,
    pub opening_fen: String,
    pub final_fen: String,
    pub baseline: ArenaTelemetry,
    pub candidate: ArenaTelemetry,
}

pub struct ArenaPairRunner {
    baseline: ArenaPlayer,
    candidate: ArenaPlayer,
    config: ArenaConfig,
}

impl ArenaPairRunner {
    pub fn new(
        baseline: &ArenaPlayerSpec,
        candidate: &ArenaPlayerSpec,
        config: ArenaConfig,
    ) -> Result<Self, String> {
        if baseline == candidate {
            return Err("baseline and candidate must be different players".to_owned());
        }
        if config.nodes_per_move == 0
            || config.hash_megabytes == 0
            || config.search_threads == 0
            || config.blunder_threshold_cp <= 0
        {
            return Err(
                "nodes, hash, search threads, and blunder threshold must be positive".to_owned(),
            );
        }
        if (config.adjudication_score == 0) != (config.adjudication_plies == 0) {
            return Err(
                "adjudication score and plies must either both be zero or both be positive"
                    .to_owned(),
            );
        }
        Ok(Self {
            baseline: ArenaPlayer::load(baseline, config)?,
            candidate: ArenaPlayer::load(candidate, config)?,
            config,
        })
    }

    pub fn play_pair(&mut self, pair: u32) -> Result<[ArenaGame; 2], String> {
        let (opening, history, moves) =
            generated_opening(self.config.seed, pair, self.config.opening_plies)?;
        Ok([
            play_game(
                &mut self.baseline,
                &mut self.candidate,
                &opening,
                &history,
                &moves,
                pair,
                Color::White,
                self.config,
            )?,
            play_game(
                &mut self.baseline,
                &mut self.candidate,
                &opening,
                &history,
                &moves,
                pair,
                Color::Black,
                self.config,
            )?,
        ])
    }
}

enum ArenaPlayer {
    TeraStockfish(Box<Searcher>),
    Model(Box<HybridAgent<VulkanAdvisor>>),
}

impl ArenaPlayer {
    fn load(spec: &ArenaPlayerSpec, config: ArenaConfig) -> Result<Self, String> {
        let search_options = SearchOptions {
            hash_megabytes: config.hash_megabytes,
            threads: config.search_threads,
        };
        match spec {
            ArenaPlayerSpec::TeraStockfish => {
                Ok(Self::TeraStockfish(Box::new(Searcher::new(search_options))))
            }
            ArenaPlayerSpec::Model(path) => {
                validate_model_prefix(path)?;
                Ok(Self::Model(Box::new(HybridAgent::new(
                    VulkanAdvisor::load(path)?,
                    HybridOptions {
                        safety_nodes: config.nodes_per_move,
                        hash_megabytes: config.hash_megabytes,
                        search_threads: config.search_threads,
                        ..HybridOptions::default()
                    },
                ))))
            }
        }
    }

    fn reset(&mut self) {
        match self {
            Self::TeraStockfish(searcher) => searcher.clear_hash(),
            Self::Model(agent) => agent.reset(),
        }
    }

    fn choose(
        &mut self,
        position: &Position,
        history: &SearchHistory,
        recent_moves: &[Move],
        config: ArenaConfig,
    ) -> Result<MoveReport, String> {
        match self {
            Self::TeraStockfish(searcher) => {
                let analysis = searcher.analyze_with_history(
                    position,
                    history,
                    SearchLimits {
                        max_depth: 191,
                        max_nodes: Some(config.nodes_per_move),
                        ..SearchLimits::default()
                    },
                );
                Ok(MoveReport {
                    chess_move: analysis.best_move.ok_or_else(|| {
                        "TeraStockfish returned no move in a non-terminal position".to_owned()
                    })?,
                    score: analysis.score,
                    nodes: analysis.nodes,
                    neural: None,
                })
            }
            Self::Model(agent) => {
                let decision =
                    agent.decide_with_recent_moves(position, Some(history), recent_moves)?;
                Ok(MoveReport {
                    chess_move: decision.chess_move,
                    score: decision.safety_score,
                    nodes: decision.safety_nodes,
                    neural: Some(NeuralMoveReport {
                        vetoed: decision.safety_vetoed_policy_best,
                        centipawn_loss: decision
                            .tactical_best_score
                            .saturating_sub(decision.safety_score)
                            .max(0),
                        transition: decision.plan_transition,
                        safety_cancelled: decision.plan_cancelled_by_safety,
                        cancelled_plan_duration: decision.cancelled_plan_duration,
                        plan_kind: decision.plan.kind,
                    }),
                })
            }
        }
    }
}

fn validate_model_prefix(path: &Path) -> Result<(), String> {
    let manifest = path.with_extension("json");
    let model = path.with_extension("mpk");
    if !manifest.is_file() || !model.is_file() {
        return Err(format!(
            "model prefix {} requires both {} and {}",
            path.display(),
            manifest.display(),
            model.display()
        ));
    }
    Ok(())
}

struct MoveReport {
    chess_move: Move,
    score: i32,
    nodes: u64,
    neural: Option<NeuralMoveReport>,
}

struct NeuralMoveReport {
    vetoed: bool,
    centipawn_loss: i32,
    transition: PlanTransition,
    safety_cancelled: bool,
    cancelled_plan_duration: Option<u8>,
    plan_kind: PlanKind,
}

#[allow(clippy::too_many_arguments)]
fn play_game(
    baseline: &mut ArenaPlayer,
    candidate: &mut ArenaPlayer,
    opening: &Position,
    opening_history: &SearchHistory,
    opening_moves: &[Move],
    pair: u32,
    candidate_color: Color,
    config: ArenaConfig,
) -> Result<ArenaGame, String> {
    baseline.reset();
    candidate.reset();
    let mut game = Game::new(opening.clone());
    let mut history = opening_history.clone();
    let mut recent_moves = opening_moves.to_vec();
    let mut baseline_telemetry = ArenaTelemetry::default();
    let mut candidate_telemetry = ArenaTelemetry::default();
    let mut plies = 0_u16;
    let mut adjudication = None::<(Color, u8)>;
    let started = Instant::now();

    loop {
        if let Some((result, termination)) = terminal_result(game.outcome(), candidate_color) {
            return Ok(finished_game(
                pair,
                candidate_color,
                result,
                termination,
                plies,
                started,
                opening,
                game.position(),
                baseline_telemetry,
                candidate_telemetry,
            ));
        }
        if config.maximum_plies > 0 && plies >= config.maximum_plies {
            return Ok(finished_game(
                pair,
                candidate_color,
                ArenaResult::Draw,
                ArenaTermination::PlyLimit,
                plies,
                started,
                opening,
                game.position(),
                baseline_telemetry,
                candidate_telemetry,
            ));
        }

        let side = game.position().side_to_move();
        let (player, telemetry) = if side == candidate_color {
            (&mut *candidate, &mut candidate_telemetry)
        } else {
            (&mut *baseline, &mut baseline_telemetry)
        };
        let report = player.choose(game.position(), &history, &recent_moves, config)?;
        telemetry.record(&report, config.blunder_threshold_cp);
        if config.adjudication_score > 0 {
            let favored = if report.score >= config.adjudication_score {
                Some(side)
            } else if report.score <= -config.adjudication_score {
                Some(side.opposite())
            } else {
                None
            };
            adjudication = match (adjudication, favored) {
                (Some((previous, count)), Some(current)) if previous == current => {
                    Some((current, count.saturating_add(1)))
                }
                (_, Some(current)) => Some((current, 1)),
                (_, None) => None,
            };
            if let Some((winner, count)) = adjudication
                && count >= config.adjudication_plies
            {
                return Ok(finished_game(
                    pair,
                    candidate_color,
                    if winner == candidate_color {
                        ArenaResult::Win
                    } else {
                        ArenaResult::Loss
                    },
                    ArenaTermination::ScoreAdjudication,
                    plies,
                    started,
                    opening,
                    game.position(),
                    baseline_telemetry,
                    candidate_telemetry,
                ));
            }
        }
        game.play(report.chess_move).map_err(|error| {
            format!(
                "arena player returned illegal move {}: {error}",
                report.chess_move.to_uci()
            )
        })?;
        recent_moves.push(report.chess_move);
        history.record(game.position());
        plies = plies.saturating_add(1);
    }
}

impl ArenaTelemetry {
    fn record(&mut self, report: &MoveReport, blunder_threshold_cp: i32) {
        self.moves += 1;
        self.nodes = self.nodes.saturating_add(report.nodes);
        let Some(neural) = &report.neural else {
            return;
        };
        self.neural_moves += 1;
        self.safety_vetoes += u64::from(neural.vetoed);
        self.blunders += u64::from(neural.centipawn_loss >= blunder_threshold_cp);
        self.centipawn_loss_sum = self
            .centipawn_loss_sum
            .saturating_add(neural.centipawn_loss as u64);
        self.plan_kind_moves[neural.plan_kind.index()] += 1;
        match neural.transition {
            PlanTransition::Started => self.plan_starts += 1,
            PlanTransition::Continued => self.plan_continuations += 1,
            PlanTransition::Completed { duration } => {
                self.plan_starts += 1;
                self.plan_completions += 1;
                self.record_ended_plan(duration);
            }
            PlanTransition::HorizonExpired { duration } => {
                self.plan_starts += 1;
                self.plan_horizon_expirations += 1;
                self.record_ended_plan(duration);
            }
            PlanTransition::BecameInapplicable { duration } => {
                self.plan_starts += 1;
                self.plan_invalidations += 1;
                self.record_ended_plan(duration);
            }
            PlanTransition::Superseded { duration } => {
                self.plan_starts += 1;
                self.plan_supersessions += 1;
                self.record_ended_plan(duration);
            }
        }
        if neural.safety_cancelled {
            self.plan_safety_cancellations += 1;
            self.record_ended_plan(neural.cancelled_plan_duration.unwrap_or(1));
        }
    }

    fn record_ended_plan(&mut self, duration: u8) {
        self.ended_plans += 1;
        self.ended_plan_duration_sum += u64::from(duration);
    }
}

#[allow(clippy::too_many_arguments)]
fn finished_game(
    pair: u32,
    candidate_color: Color,
    result: ArenaResult,
    termination: ArenaTermination,
    plies: u16,
    started: Instant,
    opening: &Position,
    final_position: &Position,
    baseline: ArenaTelemetry,
    candidate: ArenaTelemetry,
) -> ArenaGame {
    ArenaGame {
        pair,
        candidate_color,
        result,
        termination,
        plies,
        elapsed: started.elapsed(),
        opening_fen: opening.to_fen(),
        final_fen: final_position.to_fen(),
        baseline,
        candidate,
    }
}

fn terminal_result(
    outcome: GameOutcome,
    candidate_color: Color,
) -> Option<(ArenaResult, ArenaTermination)> {
    match outcome {
        GameOutcome::Win { winner } => Some((
            if winner == candidate_color {
                ArenaResult::Win
            } else {
                ArenaResult::Loss
            },
            ArenaTermination::Checkmate,
        )),
        GameOutcome::Draw(reason) => Some((
            ArenaResult::Draw,
            match reason {
                DrawReason::Stalemate => ArenaTermination::Stalemate,
                DrawReason::FiftyMoveRule => ArenaTermination::FiftyMoveRule,
                DrawReason::ThreefoldRepetition => ArenaTermination::ThreefoldRepetition,
            },
        )),
        GameOutcome::Ongoing | GameOutcome::Check => None,
    }
}

fn generated_opening(
    seed: u64,
    pair: u32,
    opening_plies: u8,
) -> Result<(Position, SearchHistory, Vec<Move>), String> {
    let mut position = Variant::TerachessII.starting_position();
    let mut history = SearchHistory::new(&position);
    let mut played = Vec::with_capacity(opening_plies as usize);
    let mut random = SplitMix64::new(seed ^ u64::from(pair).wrapping_mul(0x9e37_79b9_7f4a_7c15));
    for _ in 0..opening_plies {
        let mut moves = position.legal_moves();
        if moves.is_empty() {
            return Err("randomized opening reached a terminal position".to_owned());
        }
        moves.sort_unstable_by_key(|chess_move| chess_move.to_uci());
        let chess_move = moves[random.index(moves.len())];
        position.play(chess_move).map_err(|error| {
            format!(
                "opening generator produced illegal move {}: {error}",
                chess_move.to_uci()
            )
        })?;
        played.push(chess_move);
        history.record(&position);
    }
    Ok((position, history, played))
}

struct SplitMix64(u64);

impl SplitMix64 {
    const fn new(seed: u64) -> Self {
        Self(seed)
    }

    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut value = self.0;
        value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        value ^ (value >> 31)
    }

    fn index(&mut self, length: usize) -> usize {
        (self.next() % length as u64) as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn telemetry_rates_have_safe_zero_denominators() {
        let telemetry = ArenaTelemetry::default();
        assert_eq!(telemetry.veto_rate(), 0.0);
        assert_eq!(telemetry.blunder_rate(), 0.0);
        assert_eq!(telemetry.mean_centipawn_loss(), 0.0);
        assert_eq!(telemetry.mean_ended_plan_duration(), 0.0);
    }

    #[test]
    fn openings_are_reproducible_and_pair_specific() {
        let first = generated_opening(7, 3, 8).unwrap().0.to_fen();
        let repeated = generated_opening(7, 3, 8).unwrap().0.to_fen();
        let other = generated_opening(7, 4, 8).unwrap().0.to_fen();
        assert_eq!(first, repeated);
        assert_ne!(first, other);
    }
}
