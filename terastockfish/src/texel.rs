//! Dataset features and deterministic Texel-style material fitting.

use crate::{EvaluationParameters, evaluate_with};
use capablanca_chess_plus::{Color, PieceKind, Position};
use std::collections::BTreeMap;

pub const TUNED_PIECES: [PieceKind; PieceKind::COUNT - 2] = [
    PieceKind::Knight,
    PieceKind::Bishop,
    PieceKind::Rook,
    PieceKind::Queen,
    PieceKind::Archbishop,
    PieceKind::Chancellor,
    PieceKind::Cannon,
    PieceKind::Elephant,
    PieceKind::Camel,
    PieceKind::Giraffe,
    PieceKind::Archer,
    PieceKind::Machine,
    PieceKind::Amazon,
    PieceKind::Lion,
    PieceKind::Buffalo,
    PieceKind::Centaur,
    PieceKind::Admiral,
    PieceKind::Missionary,
    PieceKind::Eagle,
    PieceKind::Rhinoceros,
    PieceKind::Prince,
    PieceKind::Sorceress,
    PieceKind::Duchess,
    PieceKind::Troll,
];

#[derive(Clone, Debug, PartialEq)]
pub struct MaterialSample {
    pub game_id: u64,
    /// White's result: 1.0 for a win, 0.5 for a draw, 0.0 for a loss.
    pub outcome: f64,
    /// White-relative score of all evaluation terms except the 24 tuned
    /// material values. Pawn remains anchored at 100.
    pub fixed_score: i32,
    /// Original position for recomputing future evaluation features. Empty
    /// only for legacy V1 datasets that predate position retention.
    pub fen: String,
    /// White piece count minus black piece count for every PieceKind.
    pub piece_differences: [i16; PieceKind::COUNT],
}

impl MaterialSample {
    #[must_use]
    pub fn from_position(game_id: u64, outcome: f64, position: &Position) -> Self {
        let mut piece_differences = [0_i16; PieceKind::COUNT];
        for (_, piece) in position.board().pieces() {
            let direction = if piece.color == Color::White { 1 } else { -1 };
            piece_differences[piece.kind.index()] += direction;
        }
        let mut fixed_parameters = EvaluationParameters::published();
        for kind in TUNED_PIECES {
            fixed_parameters.set_material_value(kind, 0);
        }
        let relative = evaluate_with(position, &fixed_parameters);
        let fixed_score = match position.side_to_move() {
            Color::White => relative,
            Color::Black => -relative,
        };
        Self {
            game_id,
            outcome,
            fixed_score,
            fen: position.to_fen(),
            piece_differences,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct TexelOptions {
    pub epochs: u16,
    pub bootstrap_epochs: u16,
    pub batch_size: usize,
    pub material_learning_rate: f64,
    pub scale_learning_rate: f64,
    pub regularization: f64,
    pub patience: u16,
    pub bootstrap_replicates: u16,
    /// Assigns whole games to train or holdout. Keep this fixed when testing
    /// optimizer convergence.
    pub split_seed: u64,
    /// Controls training-order shuffles without changing the data split.
    pub optimizer_seed: u64,
    /// Controls game-level bootstrap resampling.
    pub bootstrap_seed: u64,
}

impl Default for TexelOptions {
    fn default() -> Self {
        Self {
            epochs: 48,
            bootstrap_epochs: 12,
            batch_size: 256,
            material_learning_rate: 1.5,
            scale_learning_rate: 0.01,
            regularization: 0.01,
            patience: 8,
            bootstrap_replicates: 32,
            split_seed: 0x5445_5845_4c54_4552,
            optimizer_seed: 0x5445_5845_4c54_4552,
            bootstrap_seed: 0x5445_5845_4c54_4552,
        }
    }
}

#[derive(Clone, Debug)]
pub struct MaterialFit {
    pub parameters: EvaluationParameters,
    pub logistic_scale: f64,
    pub baseline_train_loss: f64,
    pub baseline_holdout_loss: f64,
    pub fitted_train_loss: f64,
    pub fitted_holdout_loss: f64,
    pub train_samples: usize,
    pub holdout_samples: usize,
    pub coverage: [usize; PieceKind::COUNT],
}

#[derive(Clone, Debug)]
pub struct MaterialIntervals {
    pub bounds: [(i32, i32); PieceKind::COUNT],
    pub replicates: u16,
}

/// Fits all non-pawn material values. The holdout split is made by game id so
/// correlated positions from one game never appear on both sides.
#[must_use]
pub fn fit_material(samples: &[MaterialSample], options: TexelOptions) -> MaterialFit {
    let mut train = Vec::new();
    let mut holdout = Vec::new();
    let mut coverage = [0; PieceKind::COUNT];
    for (index, sample) in samples.iter().enumerate() {
        for kind in TUNED_PIECES {
            if sample.piece_differences[kind.index()] != 0 {
                coverage[kind.index()] += 1;
            }
        }
        if is_holdout_game(sample.game_id, options.split_seed) {
            holdout.push(index);
        } else {
            train.push(index);
        }
    }
    if train.is_empty() {
        train.extend(holdout.iter().copied());
    }
    if holdout.is_empty() {
        holdout.extend(train.iter().copied().take((train.len() / 5).max(1)));
    }

    let published = EvaluationParameters::published();
    let initial_values = material_values(&published);
    let initial_log_scale = calibrate_log_scale(samples, &train, &initial_values);
    let baseline_train_loss = loss(samples, &train, &initial_values, initial_log_scale);
    let baseline_holdout_loss = loss(samples, &holdout, &initial_values, initial_log_scale);
    let optimized = optimize(
        samples,
        &train,
        &holdout,
        initial_values,
        initial_log_scale,
        options,
        options.epochs,
    );
    MaterialFit {
        parameters: parameters_from_values(&optimized.values),
        logistic_scale: optimized.log_scale.exp(),
        baseline_train_loss,
        baseline_holdout_loss,
        fitted_train_loss: loss(samples, &train, &optimized.values, optimized.log_scale),
        fitted_holdout_loss: loss(samples, &holdout, &optimized.values, optimized.log_scale),
        train_samples: train.len(),
        holdout_samples: holdout.len(),
        coverage,
    }
}

/// Refits game-level bootstrap resamples and returns percentile confidence
/// bounds for every material value.
#[must_use]
pub fn bootstrap_material(
    samples: &[MaterialSample],
    fit: &MaterialFit,
    options: TexelOptions,
) -> MaterialIntervals {
    let mut groups = BTreeMap::<u64, Vec<usize>>::new();
    for (index, sample) in samples.iter().enumerate() {
        groups.entry(sample.game_id).or_default().push(index);
    }
    let groups = groups.into_values().collect::<Vec<_>>();
    let replicates = options.bootstrap_replicates.max(1);
    let mut estimates = std::array::from_fn::<_, { PieceKind::COUNT }, _>(|_| {
        Vec::with_capacity(usize::from(replicates))
    });
    let initial_values = material_values(&fit.parameters);
    let initial_log_scale = fit.logistic_scale.ln();
    for replicate in 0..replicates {
        let mut random = SplitMix64::new(
            options.bootstrap_seed ^ u64::from(replicate + 1).wrapping_mul(0xd1b5_4a32_d192_ed03),
        );
        let mut selected = Vec::new();
        if groups.is_empty() {
            selected.extend(0..samples.len());
        } else {
            for _ in 0..groups.len() {
                let group = &groups[random.index(groups.len())];
                selected.extend(group.iter().copied());
            }
        }
        let optimized = optimize(
            samples,
            &selected,
            &[],
            initial_values,
            initial_log_scale,
            TexelOptions {
                optimizer_seed: options.optimizer_seed ^ u64::from(replicate),
                ..options
            },
            options.bootstrap_epochs,
        );
        for kind in PieceKind::ALL {
            estimates[kind.index()].push(optimized.values[kind.index()]);
        }
    }

    let mut bounds = [(0, 0); PieceKind::COUNT];
    for kind in PieceKind::ALL {
        let values = &mut estimates[kind.index()];
        values.sort_by(f64::total_cmp);
        let low = percentile(values, 0.025).round() as i32;
        let high = percentile(values, 0.975).round() as i32;
        bounds[kind.index()] = (low, high);
    }
    MaterialIntervals { bounds, replicates }
}

#[derive(Clone, Copy)]
struct Optimized {
    values: [f64; PieceKind::COUNT],
    log_scale: f64,
}

#[allow(clippy::too_many_arguments)]
fn optimize(
    samples: &[MaterialSample],
    train: &[usize],
    holdout: &[usize],
    mut values: [f64; PieceKind::COUNT],
    mut log_scale: f64,
    options: TexelOptions,
    epochs: u16,
) -> Optimized {
    if train.is_empty() {
        return Optimized { values, log_scale };
    }
    let published = material_values(&EvaluationParameters::published());
    let mut first_moment = [0.0; PieceKind::COUNT];
    let mut second_moment = [0.0; PieceKind::COUNT];
    let mut scale_first = 0.0;
    let mut scale_second = 0.0;
    let mut step = 0_i32;
    let mut best = Optimized { values, log_scale };
    let mut best_loss = if holdout.is_empty() {
        loss(samples, train, &values, log_scale)
    } else {
        loss(samples, holdout, &values, log_scale)
    };
    let mut stale_epochs = 0;
    let mut order = train.to_vec();
    let mut random = SplitMix64::new(options.optimizer_seed);

    for _ in 0..epochs.max(1) {
        shuffle(&mut order, &mut random);
        for batch in order.chunks(options.batch_size.max(1)) {
            step += 1;
            let scale = log_scale.exp();
            let mut gradients = [0.0; PieceKind::COUNT];
            let mut scale_gradient = 0.0;
            for &index in batch {
                let sample = &samples[index];
                let score = sample_score(sample, &values);
                let probability = sigmoid(scale * score);
                let residual = probability - sample.outcome;
                let score_gradient = residual * scale;
                for kind in TUNED_PIECES {
                    gradients[kind.index()] +=
                        score_gradient * f64::from(sample.piece_differences[kind.index()]);
                }
                scale_gradient += residual * scale * score;
            }
            let divisor = batch.len() as f64;
            for kind in TUNED_PIECES {
                let index = kind.index();
                let base = published[index].max(1.0);
                gradients[index] = gradients[index] / divisor
                    + options.regularization * (values[index] - base) / (base * base);
                adam_update(
                    &mut values[index],
                    gradients[index],
                    &mut first_moment[index],
                    &mut second_moment[index],
                    step,
                    options.material_learning_rate,
                );
                values[index] = values[index].clamp(50.0, 4_000.0);
            }
            scale_gradient /= divisor;
            adam_update(
                &mut log_scale,
                scale_gradient,
                &mut scale_first,
                &mut scale_second,
                step,
                options.scale_learning_rate,
            );
            log_scale = log_scale.clamp((0.0001_f64).ln(), (0.1_f64).ln());
        }
        let current_loss = if holdout.is_empty() {
            loss(samples, train, &values, log_scale)
        } else {
            loss(samples, holdout, &values, log_scale)
        };
        if current_loss + 1.0e-8 < best_loss {
            best_loss = current_loss;
            best = Optimized { values, log_scale };
            stale_epochs = 0;
        } else {
            stale_epochs += 1;
            if !holdout.is_empty() && stale_epochs >= options.patience.max(1) {
                break;
            }
        }
    }
    best
}

fn adam_update(
    value: &mut f64,
    gradient: f64,
    first: &mut f64,
    second: &mut f64,
    step: i32,
    learning_rate: f64,
) {
    const BETA_ONE: f64 = 0.9;
    const BETA_TWO: f64 = 0.999;
    *first = BETA_ONE.mul_add(*first, (1.0 - BETA_ONE) * gradient);
    *second = BETA_TWO.mul_add(*second, (1.0 - BETA_TWO) * gradient * gradient);
    let corrected_first = *first / (1.0 - BETA_ONE.powi(step));
    let corrected_second = *second / (1.0 - BETA_TWO.powi(step));
    *value -= learning_rate * corrected_first / (corrected_second.sqrt() + 1.0e-8);
}

fn calibrate_log_scale(
    samples: &[MaterialSample],
    indices: &[usize],
    values: &[f64; PieceKind::COUNT],
) -> f64 {
    let mut log_scale = (std::f64::consts::LN_10 / 400.0).ln();
    let mut first = 0.0;
    let mut second = 0.0;
    for step in 1..=200 {
        let scale = log_scale.exp();
        let gradient = indices
            .iter()
            .map(|&index| {
                let sample = &samples[index];
                let score = sample_score(sample, values);
                (sigmoid(scale * score) - sample.outcome) * scale * score
            })
            .sum::<f64>()
            / indices.len().max(1) as f64;
        adam_update(
            &mut log_scale,
            gradient,
            &mut first,
            &mut second,
            step,
            0.01,
        );
        log_scale = log_scale.clamp((0.0001_f64).ln(), (0.1_f64).ln());
    }
    log_scale
}

fn loss(
    samples: &[MaterialSample],
    indices: &[usize],
    values: &[f64; PieceKind::COUNT],
    log_scale: f64,
) -> f64 {
    if indices.is_empty() {
        return f64::NAN;
    }
    let scale = log_scale.exp();
    indices
        .iter()
        .map(|&index| {
            let sample = &samples[index];
            let probability =
                sigmoid(scale * sample_score(sample, values)).clamp(1.0e-9, 1.0 - 1.0e-9);
            -sample.outcome * probability.ln() - (1.0 - sample.outcome) * (1.0 - probability).ln()
        })
        .sum::<f64>()
        / indices.len() as f64
}

fn sample_score(sample: &MaterialSample, values: &[f64; PieceKind::COUNT]) -> f64 {
    let mut score = f64::from(sample.fixed_score);
    for kind in TUNED_PIECES {
        score += f64::from(sample.piece_differences[kind.index()]) * values[kind.index()];
    }
    score
}

fn material_values(parameters: &EvaluationParameters) -> [f64; PieceKind::COUNT] {
    std::array::from_fn(|index| {
        f64::from(parameters.material_value(PieceKind::from_index(index).unwrap()))
    })
}

fn parameters_from_values(values: &[f64; PieceKind::COUNT]) -> EvaluationParameters {
    let mut parameters = EvaluationParameters::published();
    for kind in TUNED_PIECES {
        parameters.set_material_value(kind, values[kind.index()].round() as i32);
    }
    parameters.set_material_value(PieceKind::Pawn, 100);
    parameters.set_material_value(PieceKind::King, 0);
    parameters
}

fn is_holdout_game(game_id: u64, seed: u64) -> bool {
    splitmix_once(game_id ^ seed).is_multiple_of(5)
}

fn sigmoid(value: f64) -> f64 {
    if value >= 0.0 {
        1.0 / (1.0 + (-value.min(40.0)).exp())
    } else {
        let exponential = value.max(-40.0).exp();
        exponential / (1.0 + exponential)
    }
}

fn percentile(values: &[f64], fraction: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let position = fraction * (values.len() - 1) as f64;
    let lower = position.floor() as usize;
    let upper = position.ceil() as usize;
    values[lower] + (values[upper] - values[lower]) * (position - lower as f64)
}

fn shuffle(values: &mut [usize], random: &mut SplitMix64) {
    for index in (1..values.len()).rev() {
        values.swap(index, random.index(index + 1));
    }
}

fn splitmix_once(seed: u64) -> u64 {
    let mut random = SplitMix64::new(seed);
    random.next()
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
    use capablanca_chess_plus::{Square, Variant};

    fn position(pieces: &[(char, &str)], side: Color) -> Position {
        let mut squares = [[None; 16]; 16];
        for &(piece, coordinate) in pieces {
            let square: Square = coordinate.parse().unwrap();
            squares[usize::from(square.rank())][usize::from(square.file())] = Some(piece);
        }
        let ranks = (0..16)
            .rev()
            .map(|rank| {
                let mut encoded = String::new();
                let mut empty = 0;
                for square in squares[rank] {
                    if let Some(piece) = square {
                        if empty != 0 {
                            encoded.push_str(&empty.to_string());
                            empty = 0;
                        }
                        encoded.push(piece);
                    } else {
                        empty += 1;
                    }
                }
                if empty != 0 {
                    encoded.push_str(&empty.to_string());
                }
                encoded
            })
            .collect::<Vec<_>>();
        let side = if side == Color::White { 'w' } else { 'b' };
        Position::from_fen(
            Variant::TerachessII.rules(),
            &format!("{} {side} - - 0 1", ranks.join("/")),
        )
        .unwrap()
    }

    #[test]
    fn features_count_white_minus_black_and_preserve_anchors() {
        let position = position(
            &[
                ('K', "a1"),
                ('Q', "h8"),
                ('N', "g7"),
                ('k', "p16"),
                ('r', "i9"),
            ],
            Color::White,
        );
        let sample = MaterialSample::from_position(7, 1.0, &position);
        assert_eq!(sample.piece_differences[PieceKind::Queen.index()], 1);
        assert_eq!(sample.piece_differences[PieceKind::Knight.index()], 1);
        assert_eq!(sample.piece_differences[PieceKind::Rook.index()], -1);
        assert_eq!(sample.piece_differences[PieceKind::King.index()], 0);
        assert!(sample.fixed_score.abs() < 100);
    }

    #[test]
    fn color_rotated_features_reverse_the_white_relative_sign() {
        let original = position(
            &[('K', "a1"), ('Q', "g7"), ('k', "p16"), ('r', "m13")],
            Color::White,
        );
        let rotated = position(
            &[('k', "p16"), ('q', "j10"), ('K', "a1"), ('R', "d4")],
            Color::Black,
        );
        let first = MaterialSample::from_position(1, 1.0, &original);
        let second = MaterialSample::from_position(2, 0.0, &rotated);
        for kind in PieceKind::ALL {
            assert_eq!(
                first.piece_differences[kind.index()],
                -second.piece_differences[kind.index()],
                "{kind:?}"
            );
        }
        assert_eq!(first.fixed_score, -second.fixed_score);
    }

    #[test]
    fn holdout_split_keeps_whole_games_together() {
        let seed = 17;
        for game_id in 0..100 {
            assert_eq!(
                is_holdout_game(game_id, seed),
                is_holdout_game(game_id, seed)
            );
        }
    }

    #[test]
    fn synthetic_fit_moves_values_toward_known_material() {
        let published = EvaluationParameters::published();
        let target_queen = 1_900.0;
        let target_rook = 900.0;
        let scale = std::f64::consts::LN_10 / 400.0;
        let mut samples = Vec::new();
        for game_id in 0..400_u64 {
            let queen_difference = (game_id % 5) as i16 - 2;
            let rook_difference = ((game_id * 3) % 5) as i16 - 2;
            let score = f64::from(queen_difference) * target_queen
                + f64::from(rook_difference) * target_rook;
            let mut differences = [0; PieceKind::COUNT];
            differences[PieceKind::Queen.index()] = queen_difference;
            differences[PieceKind::Rook.index()] = rook_difference;
            samples.push(MaterialSample {
                game_id,
                outcome: sigmoid(scale * score),
                fixed_score: 0,
                fen: String::new(),
                piece_differences: differences,
            });
        }
        let fit = fit_material(
            &samples,
            TexelOptions {
                epochs: 80,
                bootstrap_replicates: 1,
                regularization: 0.0001,
                ..TexelOptions::default()
            },
        );
        assert!(
            (f64::from(fit.parameters.material_value(PieceKind::Queen)) - target_queen).abs()
                < (f64::from(published.material_value(PieceKind::Queen)) - target_queen).abs()
        );
        assert!(
            (f64::from(fit.parameters.material_value(PieceKind::Rook)) - target_rook).abs()
                < (f64::from(published.material_value(PieceKind::Rook)) - target_rook).abs()
        );
        assert_eq!(fit.parameters.material_value(PieceKind::Pawn), 100);
        assert_eq!(fit.parameters.material_value(PieceKind::King), 0);
    }

    #[test]
    fn optimizer_seed_does_not_change_the_game_level_split() {
        let samples = (0..100_u64)
            .map(|game_id| MaterialSample {
                game_id,
                outcome: 0.5,
                fixed_score: 0,
                fen: String::new(),
                piece_differences: [0; PieceKind::COUNT],
            })
            .collect::<Vec<_>>();
        let first = fit_material(
            &samples,
            TexelOptions {
                epochs: 1,
                optimizer_seed: 1,
                ..TexelOptions::default()
            },
        );
        let second = fit_material(
            &samples,
            TexelOptions {
                epochs: 1,
                optimizer_seed: 2,
                ..TexelOptions::default()
            },
        );
        assert_eq!(first.train_samples, second.train_samples);
        assert_eq!(first.holdout_samples, second.holdout_samples);
    }
}
