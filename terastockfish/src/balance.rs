//! Deterministic same-engine games for measuring White's empirical advantage.

use crate::{EvaluationParameters, SearchHistory, SearchLimits, SearchOptions, Searcher, evaluate};
use capablanca_chess_plus::{Color, DrawReason, Game, GameOutcome, Position, Variant};
use std::fmt;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BalanceConfig {
    pub nodes_per_move: u64,
    pub maximum_plies: u16,
    pub opening_plies: u8,
    pub hash_megabytes: usize,
    pub adjudication_score: i32,
    pub adjudication_plies: u8,
    pub seed: u64,
}

impl Default for BalanceConfig {
    fn default() -> Self {
        Self {
            nodes_per_move: 100_000,
            maximum_plies: 0,
            opening_plies: 8,
            hash_megabytes: 32,
            adjudication_score: 0,
            adjudication_plies: 0,
            seed: 0x4241_4c41_4e43_4531,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WhiteResult {
    Win,
    Draw,
    Loss,
    Unresolved,
}

impl WhiteResult {
    #[must_use]
    pub const fn score(self) -> Option<f64> {
        match self {
            Self::Win => Some(1.0),
            Self::Draw => Some(0.5),
            Self::Loss => Some(0.0),
            Self::Unresolved => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BalanceTermination {
    Checkmate,
    Stalemate,
    FiftyMoveRule,
    ThreefoldRepetition,
    ScoreAdjudication,
    PlyLimit,
}

#[derive(Clone, Debug)]
pub struct BalanceGame {
    pub index: u32,
    pub white_result: WhiteResult,
    pub termination: BalanceTermination,
    pub plies: u16,
    pub nodes: u64,
    pub elapsed: Duration,
    pub final_score: i32,
    pub opening_fen: String,
    pub final_fen: String,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BalanceSummary {
    pub games: usize,
    pub white_wins: usize,
    pub draws: usize,
    pub white_losses: usize,
    pub unresolved: usize,
    pub white_score: Option<f64>,
    pub confidence_interval_95: Option<(f64, f64)>,
}

impl BalanceSummary {
    #[must_use]
    pub fn from_games(games: &[BalanceGame]) -> Self {
        let white_wins = games
            .iter()
            .filter(|game| game.white_result == WhiteResult::Win)
            .count();
        let draws = games
            .iter()
            .filter(|game| game.white_result == WhiteResult::Draw)
            .count();
        let white_losses = games
            .iter()
            .filter(|game| game.white_result == WhiteResult::Loss)
            .count();
        let unresolved = games
            .iter()
            .filter(|game| game.white_result == WhiteResult::Unresolved)
            .count();
        let resolved = white_wins + draws + white_losses;
        let successes = white_wins as f64 + draws as f64 * 0.5;
        let white_score = if resolved == 0 {
            None
        } else {
            Some(successes / resolved as f64)
        };
        let confidence_interval_95 = (resolved > 0).then(|| wilson_interval(successes, resolved));
        Self {
            games: games.len(),
            white_wins,
            draws,
            white_losses,
            unresolved,
            white_score,
            confidence_interval_95,
        }
    }

    #[must_use]
    pub fn unresolved_fraction(self) -> f64 {
        if self.games == 0 {
            0.0
        } else {
            self.unresolved as f64 / self.games as f64
        }
    }
}

fn wilson_interval(successes: f64, trials: usize) -> (f64, f64) {
    let trials = trials as f64;
    let probability = successes / trials;
    let z = 1.96_f64;
    let z_squared = z * z;
    let denominator = 1.0 + z_squared / trials;
    let center = (probability + z_squared / (2.0 * trials)) / denominator;
    let margin = z
        * (probability * (1.0 - probability) / trials + z_squared / (4.0 * trials * trials)).sqrt()
        / denominator;
    ((center - margin).max(0.0), (center + margin).min(1.0))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BalanceError(String);

impl fmt::Display for BalanceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for BalanceError {}

/// Plays one production-vs-production game from a reproducible randomized
/// opening derived from the standard Terachess II initial array.
pub fn play_balance_game(index: u32, config: BalanceConfig) -> Result<BalanceGame, BalanceError> {
    let (opening, mut history) = generated_opening(config.seed, index, config.opening_plies)?;
    let search_options = SearchOptions {
        hash_megabytes: config.hash_megabytes.max(1),
        threads: 1,
    };
    let profile = EvaluationParameters::production();
    let mut white = Searcher::with_evaluation(search_options, profile);
    let mut black = Searcher::with_evaluation(search_options, profile);
    let mut game = Game::new(opening.clone());
    let started = Instant::now();
    let mut nodes = 0_u64;
    let mut plies = 0_u16;
    let mut adjudication = None::<(Color, u8)>;

    loop {
        if let Some((white_result, termination)) = terminal_white_result(game.outcome()) {
            return Ok(finished_game(
                index,
                &opening,
                game.position(),
                white_result,
                termination,
                plies,
                nodes,
                white_relative_evaluation(game.position()),
                started,
            ));
        }
        if config.maximum_plies > 0 && plies >= config.maximum_plies {
            return Ok(finished_game(
                index,
                &opening,
                game.position(),
                WhiteResult::Unresolved,
                BalanceTermination::PlyLimit,
                plies,
                nodes,
                white_relative_evaluation(game.position()),
                started,
            ));
        }

        let side = game.position().side_to_move();
        let searcher = if side == Color::White {
            &mut white
        } else {
            &mut black
        };
        let analysis = searcher.analyze_with_history(
            game.position(),
            &history,
            SearchLimits {
                max_depth: 191,
                max_nodes: Some(config.nodes_per_move.max(1)),
                ..SearchLimits::default()
            },
        );
        nodes = nodes.saturating_add(analysis.nodes);
        if config.adjudication_score > 0 && config.adjudication_plies > 0 {
            let favored = if analysis.score >= config.adjudication_score {
                Some(side)
            } else if analysis.score <= -config.adjudication_score {
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
                    index,
                    &opening,
                    game.position(),
                    if winner == Color::White {
                        WhiteResult::Win
                    } else {
                        WhiteResult::Loss
                    },
                    BalanceTermination::ScoreAdjudication,
                    plies,
                    nodes,
                    if side == Color::White {
                        analysis.score
                    } else {
                        -analysis.score
                    },
                    started,
                ));
            }
        }
        let chess_move = analysis.best_move.ok_or_else(|| {
            BalanceError(format!(
                "search returned no move in a non-terminal position: {}",
                game.position().to_fen()
            ))
        })?;
        game.play(chess_move).map_err(|error| {
            BalanceError(format!(
                "search returned illegal move {}: {error}",
                chess_move.to_uci()
            ))
        })?;
        history.record(game.position());
        plies = plies.saturating_add(1);
    }
}

#[allow(clippy::too_many_arguments)]
fn finished_game(
    index: u32,
    opening: &Position,
    final_position: &Position,
    white_result: WhiteResult,
    termination: BalanceTermination,
    plies: u16,
    nodes: u64,
    final_score: i32,
    started: Instant,
) -> BalanceGame {
    BalanceGame {
        index,
        white_result,
        termination,
        plies,
        nodes,
        elapsed: started.elapsed(),
        final_score,
        opening_fen: opening.to_fen(),
        final_fen: final_position.to_fen(),
    }
}

fn white_relative_evaluation(position: &Position) -> i32 {
    let score = evaluate(position);
    if position.side_to_move() == Color::White {
        score
    } else {
        -score
    }
}

fn terminal_white_result(outcome: GameOutcome) -> Option<(WhiteResult, BalanceTermination)> {
    match outcome {
        GameOutcome::Win { winner } => Some((
            if winner == Color::White {
                WhiteResult::Win
            } else {
                WhiteResult::Loss
            },
            BalanceTermination::Checkmate,
        )),
        GameOutcome::Draw(reason) => Some((
            WhiteResult::Draw,
            match reason {
                DrawReason::Stalemate => BalanceTermination::Stalemate,
                DrawReason::FiftyMoveRule => BalanceTermination::FiftyMoveRule,
                DrawReason::ThreefoldRepetition => BalanceTermination::ThreefoldRepetition,
            },
        )),
        GameOutcome::Ongoing | GameOutcome::Check => None,
    }
}

fn generated_opening(
    seed: u64,
    index: u32,
    opening_plies: u8,
) -> Result<(Position, SearchHistory), BalanceError> {
    let mut position = Variant::TerachessII.starting_position();
    let mut history = SearchHistory::new(&position);
    let mut random = SplitMix64::new(seed ^ u64::from(index).wrapping_mul(0x9e37_79b9_7f4a_7c15));
    for _ in 0..opening_plies {
        let mut moves = position.legal_moves();
        if moves.is_empty() {
            return Err(BalanceError(
                "randomized opening reached a terminal position".to_owned(),
            ));
        }
        moves.sort_unstable_by_key(|chess_move| chess_move.to_uci());
        let chess_move = moves[random.index(moves.len())];
        position.play(chess_move).map_err(|error| {
            BalanceError(format!(
                "opening generator produced illegal move {}: {error}",
                chess_move.to_uci()
            ))
        })?;
        history.record(&position);
    }
    Ok((position, history))
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

    fn game(index: u32, result: WhiteResult) -> BalanceGame {
        BalanceGame {
            index,
            white_result: result,
            termination: BalanceTermination::PlyLimit,
            plies: 1,
            nodes: 1,
            elapsed: Duration::ZERO,
            final_score: 0,
            opening_fen: String::new(),
            final_fen: String::new(),
        }
    }

    #[test]
    fn summary_uses_white_score_as_its_statistical_unit() {
        let games = [
            game(0, WhiteResult::Win),
            game(1, WhiteResult::Draw),
            game(2, WhiteResult::Loss),
        ];
        let summary = BalanceSummary::from_games(&games);
        assert_eq!(summary.games, 3);
        assert_eq!(summary.white_wins, 1);
        assert_eq!(summary.draws, 1);
        assert_eq!(summary.white_losses, 1);
        assert_eq!(summary.white_score, Some(0.5));
        assert_eq!(summary.unresolved, 0);
    }

    #[test]
    fn unresolved_games_are_excluded_and_all_draws_keep_uncertainty() {
        let games = [
            game(0, WhiteResult::Draw),
            game(1, WhiteResult::Draw),
            game(2, WhiteResult::Unresolved),
        ];
        let summary = BalanceSummary::from_games(&games);
        assert_eq!(summary.white_score, Some(0.5));
        assert_eq!(summary.unresolved, 1);
        assert_eq!(summary.unresolved_fraction(), 1.0 / 3.0);
        let (lower, upper) = summary.confidence_interval_95.unwrap();
        assert!(lower < 0.5);
        assert!(upper > 0.5);
    }

    #[test]
    fn randomized_openings_are_deterministic_and_keep_white_to_move() {
        let first = generated_opening(7, 3, 8).unwrap();
        let again = generated_opening(7, 3, 8).unwrap();
        let different = generated_opening(8, 3, 8).unwrap();
        assert_eq!(first.0, again.0);
        assert_ne!(first.0, different.0);
        assert_eq!(first.0.side_to_move(), Color::White);
    }

    #[test]
    fn smoke_game_finishes_at_the_ply_limit() {
        let game = play_balance_game(
            0,
            BalanceConfig {
                nodes_per_move: 64,
                maximum_plies: 2,
                opening_plies: 2,
                hash_megabytes: 1,
                ..BalanceConfig::default()
            },
        )
        .unwrap();
        assert_eq!(game.termination, BalanceTermination::PlyLimit);
        assert_eq!(game.white_result, WhiteResult::Unresolved);
        assert_eq!(game.plies, 2);
    }
}
