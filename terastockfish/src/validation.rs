//! Deterministic paired self-play for comparing evaluation profiles.

use crate::{EvaluationParameters, SearchHistory, SearchLimits, SearchOptions, Searcher};
use capablanca_chess_plus::{Color, DrawReason, Game, GameOutcome, Position, Variant};
use std::error::Error;
use std::fmt;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SelfPlayConfig {
    /// No statistical decision is made before this many color-swapped pairs.
    pub minimum_pairs: u32,
    /// Hard limit for the comparison.
    pub maximum_pairs: u32,
    pub nodes_per_move: u64,
    /// Optional safety cap. Zero disables the cap and lets chess termination
    /// rules or score adjudication end the game.
    pub maximum_plies: u16,
    pub opening_plies: u8,
    pub hash_megabytes: usize,
    /// Independent game pairs evaluated concurrently. Search inside each game
    /// remains single-threaded and deterministic.
    pub jobs: usize,
    /// A position is adjudicated when both engines keep reporting the same
    /// side above this absolute score for `adjudication_plies` half-moves. Set
    /// either value to zero to disable score adjudication.
    pub adjudication_score: i32,
    pub adjudication_plies: u8,
    pub seed: u64,
}

impl SelfPlayConfig {
    /// A deliberately tiny run intended to verify the harness, not playing
    /// strength.
    #[must_use]
    pub const fn smoke() -> Self {
        Self {
            minimum_pairs: 1,
            maximum_pairs: 1,
            nodes_per_move: 128,
            maximum_plies: 4,
            opening_plies: 2,
            hash_megabytes: 1,
            jobs: 1,
            adjudication_score: 0,
            adjudication_plies: 0,
            seed: 0x5445_5241_4348_4553,
        }
    }
}

impl Default for SelfPlayConfig {
    fn default() -> Self {
        Self {
            minimum_pairs: 50,
            maximum_pairs: 200,
            nodes_per_move: 100_000,
            maximum_plies: 400,
            opening_plies: 6,
            hash_megabytes: 64,
            jobs: 1,
            adjudication_score: 800,
            adjudication_plies: 12,
            seed: 0x5445_5241_4348_4553,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CandidateResult {
    Win,
    Draw,
    Loss,
}

impl CandidateResult {
    #[must_use]
    pub const fn score(self) -> f64 {
        match self {
            Self::Win => 1.0,
            Self::Draw => 0.5,
            Self::Loss => 0.0,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GameTermination {
    Checkmate,
    Stalemate,
    FiftyMoveRule,
    ThreefoldRepetition,
    ScoreAdjudication,
    PlyLimit,
}

#[derive(Clone, Debug)]
pub struct SelfPlayGame {
    pub pair: u32,
    pub candidate_color: Color,
    pub result: CandidateResult,
    pub termination: GameTermination,
    pub plies: u16,
    pub nodes: u64,
    pub elapsed: Duration,
    pub opening_fen: String,
}

#[derive(Clone, Debug)]
pub struct SelfPlaySummary {
    pub games: Vec<SelfPlayGame>,
    pub statistically_decisive: bool,
}

impl SelfPlaySummary {
    #[must_use]
    pub fn wins(&self) -> usize {
        self.games
            .iter()
            .filter(|game| game.result == CandidateResult::Win)
            .count()
    }

    #[must_use]
    pub fn draws(&self) -> usize {
        self.games
            .iter()
            .filter(|game| game.result == CandidateResult::Draw)
            .count()
    }

    #[must_use]
    pub fn losses(&self) -> usize {
        self.games
            .iter()
            .filter(|game| game.result == CandidateResult::Loss)
            .count()
    }

    #[must_use]
    pub fn score(&self) -> f64 {
        if self.games.is_empty() {
            return 0.5;
        }
        self.games
            .iter()
            .map(|game| game.result.score())
            .sum::<f64>()
            / self.games.len() as f64
    }

    /// A normal-approximation confidence interval over paired scores. Treating
    /// each color-swapped pair as one sample preserves the pairing design.
    #[must_use]
    pub fn confidence_interval_95(&self) -> (f64, f64) {
        let pair_count = self.games.len() / 2;
        if pair_count < 2 {
            return (0.0, 1.0);
        }
        let pair_scores = self
            .games
            .chunks_exact(2)
            .map(|pair| (pair[0].result.score() + pair[1].result.score()) * 0.5);
        let (sum, squared_sum) = pair_scores.fold((0.0, 0.0), |(sum, squared_sum), score| {
            (sum + score, squared_sum + score * score)
        });
        let count = pair_count as f64;
        let mean = sum / count;
        let variance = ((squared_sum - count * mean * mean) / (count - 1.0)).max(0.0);
        let margin = 1.96 * (variance / count).sqrt();
        ((mean - margin).max(0.0), (mean + margin).min(1.0))
    }

    #[must_use]
    pub fn candidate_is_stronger(&self) -> bool {
        self.confidence_interval_95().0 > 0.5
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SelfPlayError(String);

impl fmt::Display for SelfPlayError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for SelfPlayError {}

/// Compares a candidate profile with the published baseline. Each generated
/// opening is played twice with colors swapped. The run stops after the
/// configured minimum when the 95% interval no longer crosses 50%, or at the
/// hard pair limit.
pub fn compare_with_published(
    candidate: EvaluationParameters,
    config: SelfPlayConfig,
) -> Result<SelfPlaySummary, SelfPlayError> {
    compare_profiles(EvaluationParameters::published(), candidate, config)
}

/// Compares two explicit profiles. Results and scores are always reported
/// from the candidate's point of view.
pub fn compare_profiles(
    baseline: EvaluationParameters,
    candidate: EvaluationParameters,
    config: SelfPlayConfig,
) -> Result<SelfPlaySummary, SelfPlayError> {
    compare_profiles_with(baseline, candidate, config, |_| {})
}

/// [`compare_profiles`] with a progress callback after every completed batch
/// of at most `config.jobs` pairs.
pub fn compare_profiles_with<F>(
    baseline: EvaluationParameters,
    candidate: EvaluationParameters,
    config: SelfPlayConfig,
    mut reporter: F,
) -> Result<SelfPlaySummary, SelfPlayError>
where
    F: FnMut(&SelfPlaySummary),
{
    let minimum_pairs = config.minimum_pairs.max(1);
    let maximum_pairs = config.maximum_pairs.max(minimum_pairs);
    let mut summary = SelfPlaySummary {
        games: Vec::with_capacity(maximum_pairs as usize * 2),
        statistically_decisive: false,
    };

    let mut next_pair = 0;
    while next_pair < maximum_pairs {
        let jobs = config.jobs.max(1).min(u32::MAX as usize) as u32;
        let batch_end = (next_pair + jobs).min(maximum_pairs);
        let batch = std::thread::scope(|scope| {
            let mut workers = Vec::with_capacity((batch_end - next_pair) as usize);
            for pair in next_pair..batch_end {
                workers.push(scope.spawn(move || {
                    play_profile_pair(pair, baseline, candidate, config).map(Vec::from)
                }));
            }
            workers
                .into_iter()
                .map(|worker| {
                    worker
                        .join()
                        .map_err(|_| SelfPlayError("self-play worker panicked".to_owned()))?
                })
                .collect::<Result<Vec<_>, SelfPlayError>>()
        })?;
        for pair_games in batch {
            summary.games.extend(pair_games);
        }
        reporter(&summary);
        next_pair = batch_end;
        if next_pair >= minimum_pairs {
            let (lower, upper) = summary.confidence_interval_95();
            if lower > 0.5 || upper < 0.5 {
                summary.statistically_decisive = true;
                break;
            }
        }
    }

    Ok(summary)
}

/// Plays one reproducible color-swapped pair. This is the resumable building
/// block used by fixed-sample research tools; pair indices are independent and
/// may be evaluated in any order.
pub fn play_profile_pair(
    pair: u32,
    baseline: EvaluationParameters,
    candidate: EvaluationParameters,
    config: SelfPlayConfig,
) -> Result<[SelfPlayGame; 2], SelfPlayError> {
    let (opening, history) = generated_opening(config.seed, pair, config.opening_plies)?;
    let [first_color, second_color] = Color::ALL;
    Ok([
        play_game(
            &opening,
            &history,
            pair,
            first_color,
            baseline,
            candidate,
            config,
        )?,
        play_game(
            &opening,
            &history,
            pair,
            second_color,
            baseline,
            candidate,
            config,
        )?,
    ])
}

fn play_game(
    opening: &Position,
    opening_history: &SearchHistory,
    pair: u32,
    candidate_color: Color,
    baseline: EvaluationParameters,
    candidate: EvaluationParameters,
    config: SelfPlayConfig,
) -> Result<SelfPlayGame, SelfPlayError> {
    let options = SearchOptions {
        hash_megabytes: config.hash_megabytes.max(1),
        threads: 1,
    };
    let mut baseline_searcher = Searcher::with_evaluation(options, baseline);
    let mut candidate_searcher = Searcher::with_evaluation(options, candidate);
    let mut game = Game::new(opening.clone());
    let mut history = opening_history.clone();
    let started = Instant::now();
    let mut nodes = 0_u64;
    let mut plies = 0_u16;
    let mut adjudication = None::<(Color, u8)>;

    loop {
        let outcome = game.outcome();
        if let Some((result, termination)) = terminal_result(outcome, candidate_color) {
            return Ok(finished_game(
                opening,
                pair,
                candidate_color,
                result,
                termination,
                plies,
                nodes,
                started,
            ));
        }
        if config.maximum_plies > 0 && plies >= config.maximum_plies {
            return Ok(finished_game(
                opening,
                pair,
                candidate_color,
                CandidateResult::Draw,
                GameTermination::PlyLimit,
                plies,
                nodes,
                started,
            ));
        }

        let searcher = if game.position().side_to_move() == candidate_color {
            &mut candidate_searcher
        } else {
            &mut baseline_searcher
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
            let side = game.position().side_to_move();
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
                    opening,
                    pair,
                    candidate_color,
                    if winner == candidate_color {
                        CandidateResult::Win
                    } else {
                        CandidateResult::Loss
                    },
                    GameTermination::ScoreAdjudication,
                    plies,
                    nodes,
                    started,
                ));
            }
        }
        let chess_move = analysis.best_move.ok_or_else(|| {
            SelfPlayError(format!(
                "search returned no move in a non-terminal position: {}",
                game.position().to_fen()
            ))
        })?;
        game.play(chess_move).map_err(|error| {
            SelfPlayError(format!(
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
    opening: &Position,
    pair: u32,
    candidate_color: Color,
    result: CandidateResult,
    termination: GameTermination,
    plies: u16,
    nodes: u64,
    started: Instant,
) -> SelfPlayGame {
    SelfPlayGame {
        pair,
        candidate_color,
        result,
        termination,
        plies,
        nodes,
        elapsed: started.elapsed(),
        opening_fen: opening.to_fen(),
    }
}

fn terminal_result(
    outcome: GameOutcome,
    candidate_color: Color,
) -> Option<(CandidateResult, GameTermination)> {
    match outcome {
        GameOutcome::Win { winner } => Some((
            if winner == candidate_color {
                CandidateResult::Win
            } else {
                CandidateResult::Loss
            },
            GameTermination::Checkmate,
        )),
        GameOutcome::Draw(reason) => Some((
            CandidateResult::Draw,
            match reason {
                DrawReason::Stalemate => GameTermination::Stalemate,
                DrawReason::FiftyMoveRule => GameTermination::FiftyMoveRule,
                DrawReason::ThreefoldRepetition => GameTermination::ThreefoldRepetition,
            },
        )),
        GameOutcome::Ongoing | GameOutcome::Check => None,
    }
}

fn generated_opening(
    seed: u64,
    pair: u32,
    opening_plies: u8,
) -> Result<(Position, SearchHistory), SelfPlayError> {
    let mut position = Variant::TerachessII.starting_position();
    let mut history = SearchHistory::new(&position);
    let mut random = SplitMix64::new(seed ^ u64::from(pair).wrapping_mul(0x9e37_79b9_7f4a_7c15));
    for _ in 0..opening_plies {
        let mut moves = position.legal_moves();
        if moves.is_empty() {
            break;
        }
        moves.sort_unstable_by_key(|chess_move| chess_move.to_uci());
        let chess_move = moves[random.index(moves.len())];
        position.play(chess_move).map_err(|error| {
            SelfPlayError(format!(
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

    #[test]
    fn opening_generation_is_deterministic_and_seeded() {
        let first = generated_opening(7, 3, 6).unwrap();
        let again = generated_opening(7, 3, 6).unwrap();
        let different = generated_opening(8, 3, 6).unwrap();
        assert_eq!(first.0, again.0);
        assert_ne!(first.0, different.0);
    }

    #[test]
    fn smoke_comparison_plays_one_color_swapped_pair() {
        let summary =
            compare_with_published(EvaluationParameters::published(), SelfPlayConfig::smoke())
                .unwrap();
        assert_eq!(summary.games.len(), 2);
        assert_eq!(summary.games[0].opening_fen, summary.games[1].opening_fen);
        assert_ne!(
            summary.games[0].candidate_color,
            summary.games[1].candidate_color
        );
    }

    #[test]
    fn confidence_interval_detects_a_decisive_sample() {
        let games = (0..20)
            .map(|pair| SelfPlayGame {
                pair,
                candidate_color: Color::White,
                result: CandidateResult::Win,
                termination: GameTermination::Checkmate,
                plies: 1,
                nodes: 1,
                elapsed: Duration::ZERO,
                opening_fen: String::new(),
            })
            .collect();
        let summary = SelfPlaySummary {
            games,
            statistically_decisive: true,
        };
        assert!(summary.candidate_is_stronger());
        assert_eq!(summary.confidence_interval_95(), (1.0, 1.0));
    }
}
