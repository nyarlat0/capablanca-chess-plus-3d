use capablanca_chess_plus::{
    Color, Game, GameOutcome, Move, MoveKind, PieceKind, Position, Variant,
};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};
use terastockfish::texel::{
    MaterialFit, MaterialIntervals, MaterialSample, TUNED_PIECES, TexelOptions, bootstrap_material,
    fit_material,
};
use terastockfish::validation::{SelfPlayConfig, compare_profiles};
use terastockfish::{EvaluationParameters, SearchLimits, SearchOptions, Searcher};

const DATASET_VERSION: &str = "TERASTOCKFISH_TEXEL_DATA_V1";
const CHECKPOINT_VERSION: &str = "TERASTOCKFISH_TEXEL_RUN_V1";

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("terastockfish-texel: {error}");
            ExitCode::FAILURE
        }
    }
}

#[derive(Clone, Debug)]
struct Options {
    training_pairs: u32,
    training_nodes: u64,
    training_maximum_plies: u16,
    opening_plies: u8,
    sample_after_plies: u16,
    sample_stride: u16,
    samples_per_game: usize,
    profile_perturbation: f64,
    jobs: usize,
    hash_megabytes: usize,
    adjudication_score: i32,
    adjudication_plies: u8,
    collection_budget: Duration,
    total_budget: Duration,
    validation_minimum_pairs: u32,
    validation_maximum_pairs: u32,
    validation_nodes: u64,
    validation_maximum_plies: u16,
    seed: u64,
    dataset_path: PathBuf,
    checkpoint_path: PathBuf,
    resume: bool,
    texel: TexelOptions,
}

impl Options {
    fn preset(name: &str) -> Result<Self, String> {
        let mut options = Self {
            training_pairs: 512,
            training_nodes: 20_000,
            training_maximum_plies: 240,
            opening_plies: 8,
            sample_after_plies: 8,
            sample_stride: 4,
            samples_per_game: 32,
            profile_perturbation: 0.12,
            jobs: 12,
            hash_megabytes: 16,
            adjudication_score: 800,
            adjudication_plies: 12,
            collection_budget: Duration::from_secs(8 * 60 * 60),
            total_budget: Duration::from_secs(12 * 60 * 60),
            validation_minimum_pairs: 24,
            validation_maximum_pairs: 96,
            validation_nodes: 50_000,
            validation_maximum_plies: 300,
            seed: 0x4f56_4552_4e49_4748,
            dataset_path: PathBuf::from("target/terastockfish-texel.csv"),
            checkpoint_path: PathBuf::from("target/terastockfish-texel.chk"),
            resume: false,
            texel: TexelOptions::default(),
        };
        match name {
            "overnight" => {}
            "smoke" => {
                options.training_pairs = 1;
                options.training_nodes = 128;
                options.training_maximum_plies = 4;
                options.opening_plies = 2;
                options.sample_after_plies = 0;
                options.sample_stride = 1;
                options.samples_per_game = 4;
                options.jobs = 1;
                options.hash_megabytes = 1;
                options.adjudication_score = 0;
                options.adjudication_plies = 0;
                options.collection_budget = Duration::from_secs(30);
                options.total_budget = Duration::from_secs(60);
                options.validation_minimum_pairs = 1;
                options.validation_maximum_pairs = 1;
                options.validation_nodes = 128;
                options.validation_maximum_plies = 4;
                options.texel.epochs = 2;
                options.texel.bootstrap_epochs = 1;
                options.texel.bootstrap_replicates = 2;
            }
            _ => return Err(format!("unknown preset `{name}`")),
        }
        Ok(options)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Stage {
    Collect,
    Fit,
    Validate,
    Complete,
}

impl Stage {
    const fn name(self) -> &'static str {
        match self {
            Self::Collect => "collect",
            Self::Fit => "fit",
            Self::Validate => "validate",
            Self::Complete => "complete",
        }
    }

    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "collect" => Ok(Self::Collect),
            "fit" => Ok(Self::Fit),
            "validate" => Ok(Self::Validate),
            "complete" => Ok(Self::Complete),
            _ => Err(format!("invalid checkpoint stage `{value}`")),
        }
    }
}

#[derive(Clone, Debug)]
struct Dataset {
    completed_pairs: u32,
    collection_seconds: f64,
    samples: Vec<MaterialSample>,
}

impl Dataset {
    fn new() -> Self {
        Self {
            completed_pairs: 0,
            collection_seconds: 0.0,
            samples: Vec::new(),
        }
    }

    fn encode(&self) -> String {
        let mut output = format!(
            "# {DATASET_VERSION}\n# completed_pairs={}\n# collection_seconds={:.6}\n",
            self.completed_pairs, self.collection_seconds
        );
        output.push_str("game_id,outcome,fixed_score");
        for kind in PieceKind::ALL {
            output.push(',');
            output.push_str(piece_name(kind));
        }
        output.push('\n');
        for sample in &self.samples {
            output.push_str(&format!(
                "{},{:.6},{}",
                sample.game_id, sample.outcome, sample.fixed_score
            ));
            for difference in sample.piece_differences {
                output.push_str(&format!(",{difference}"));
            }
            output.push('\n');
        }
        output
    }

    fn decode(contents: &str) -> Result<Self, String> {
        let mut dataset = Self::new();
        let mut saw_version = false;
        let mut saw_pairs = false;
        let mut saw_seconds = false;
        for line in contents.lines() {
            if line.strip_prefix("# ") == Some(DATASET_VERSION) {
                saw_version = true;
            } else if let Some(value) = line.strip_prefix("# completed_pairs=") {
                dataset.completed_pairs = parse(value, "completed_pairs")?;
                saw_pairs = true;
            } else if let Some(value) = line.strip_prefix("# collection_seconds=") {
                dataset.collection_seconds = parse(value, "collection_seconds")?;
                saw_seconds = true;
            } else if line.starts_with('#') || line.starts_with("game_id,") || line.is_empty() {
                continue;
            } else {
                let fields = line.split(',').collect::<Vec<_>>();
                if fields.len() != PieceKind::COUNT + 3 {
                    return Err(format!("invalid dataset row with {} fields", fields.len()));
                }
                let mut piece_differences = [0_i16; PieceKind::COUNT];
                for (index, difference) in piece_differences.iter_mut().enumerate() {
                    *difference = parse(fields[index + 3], "piece difference")?;
                }
                let outcome = parse::<f64>(fields[1], "outcome")?;
                if !outcome.is_finite() || !(0.0..=1.0).contains(&outcome) {
                    return Err("dataset outcome must be between zero and one".to_owned());
                }
                dataset.samples.push(MaterialSample {
                    game_id: parse(fields[0], "game id")?,
                    outcome,
                    fixed_score: parse(fields[2], "fixed score")?,
                    piece_differences,
                });
            }
        }
        if !saw_version || !saw_pairs || !saw_seconds {
            return Err("dataset header is incomplete".to_owned());
        }
        Ok(dataset)
    }
}

#[derive(Clone, Debug)]
struct RunState {
    stage: Stage,
    seed: u64,
    total_seconds: f64,
    fitted: EvaluationParameters,
    intervals: [(i32, i32); PieceKind::COUNT],
    coverage: [usize; PieceKind::COUNT],
    baseline_train_loss: f64,
    baseline_holdout_loss: f64,
    fitted_train_loss: f64,
    fitted_holdout_loss: f64,
    train_samples: usize,
    holdout_samples: usize,
    validation_pair_scores: Vec<f64>,
    validation_wins: usize,
    validation_draws: usize,
    validation_losses: usize,
}

impl RunState {
    fn new(seed: u64) -> Self {
        let published = EvaluationParameters::published();
        Self {
            stage: Stage::Collect,
            seed,
            total_seconds: 0.0,
            fitted: published,
            intervals: std::array::from_fn(|index| {
                let kind = PieceKind::from_index(index).unwrap();
                let value = published.material_value(kind);
                (value, value)
            }),
            coverage: [0; PieceKind::COUNT],
            baseline_train_loss: f64::NAN,
            baseline_holdout_loss: f64::NAN,
            fitted_train_loss: f64::NAN,
            fitted_holdout_loss: f64::NAN,
            train_samples: 0,
            holdout_samples: 0,
            validation_pair_scores: Vec::new(),
            validation_wins: 0,
            validation_draws: 0,
            validation_losses: 0,
        }
    }

    fn encode(&self) -> String {
        let mut output = format!(
            "{CHECKPOINT_VERSION}\nstage={}\nseed={}\ntotal_seconds={:.6}\n",
            self.stage.name(),
            self.seed,
            self.total_seconds
        );
        output.push_str(&format!(
            "baseline_train_loss={}\nbaseline_holdout_loss={}\nfitted_train_loss={}\nfitted_holdout_loss={}\ntrain_samples={}\nholdout_samples={}\nvalidation_wins={}\nvalidation_draws={}\nvalidation_losses={}\n",
            self.baseline_train_loss,
            self.baseline_holdout_loss,
            self.fitted_train_loss,
            self.fitted_holdout_loss,
            self.train_samples,
            self.holdout_samples,
            self.validation_wins,
            self.validation_draws,
            self.validation_losses
        ));
        output.push_str("validation_pair_scores=");
        for (index, score) in self.validation_pair_scores.iter().enumerate() {
            if index != 0 {
                output.push(',');
            }
            output.push_str(&format!("{score:.6}"));
        }
        output.push('\n');
        for kind in PieceKind::ALL {
            let index = kind.index();
            output.push_str(&format!(
                "material.{}={}\ninterval.{}={},{}\ncoverage.{}={}\n",
                piece_name(kind),
                self.fitted.material_value(kind),
                piece_name(kind),
                self.intervals[index].0,
                self.intervals[index].1,
                piece_name(kind),
                self.coverage[index]
            ));
        }
        output
    }

    fn decode(contents: &str) -> Result<Self, String> {
        let mut lines = contents.lines();
        if lines.next() != Some(CHECKPOINT_VERSION) {
            return Err("unsupported or corrupt Texel checkpoint".to_owned());
        }
        let mut state = Self::new(0);
        let mut saw_stage = false;
        let mut saw_seed = false;
        for line in lines {
            let Some((name, value)) = line.split_once('=') else {
                return Err(format!("invalid checkpoint line `{line}`"));
            };
            match name {
                "stage" => {
                    state.stage = Stage::parse(value)?;
                    saw_stage = true;
                }
                "seed" => {
                    state.seed = parse(value, name)?;
                    saw_seed = true;
                }
                "total_seconds" => state.total_seconds = parse(value, name)?,
                "baseline_train_loss" => state.baseline_train_loss = parse(value, name)?,
                "baseline_holdout_loss" => state.baseline_holdout_loss = parse(value, name)?,
                "fitted_train_loss" => state.fitted_train_loss = parse(value, name)?,
                "fitted_holdout_loss" => state.fitted_holdout_loss = parse(value, name)?,
                "train_samples" => state.train_samples = parse(value, name)?,
                "holdout_samples" => state.holdout_samples = parse(value, name)?,
                "validation_wins" => state.validation_wins = parse(value, name)?,
                "validation_draws" => state.validation_draws = parse(value, name)?,
                "validation_losses" => state.validation_losses = parse(value, name)?,
                "validation_pair_scores" => {
                    state.validation_pair_scores = if value.is_empty() {
                        Vec::new()
                    } else {
                        value
                            .split(',')
                            .map(|score| parse(score, "validation score"))
                            .collect::<Result<Vec<_>, _>>()?
                    };
                }
                _ => {
                    if let Some(piece) = name.strip_prefix("material.") {
                        state
                            .fitted
                            .set_material_value(parse_piece_kind(piece)?, parse(value, name)?);
                    } else if let Some(piece) = name.strip_prefix("interval.") {
                        let kind = parse_piece_kind(piece)?;
                        let (low, high) = value
                            .split_once(',')
                            .ok_or_else(|| format!("invalid interval `{value}`"))?;
                        state.intervals[kind.index()] = (parse(low, name)?, parse(high, name)?);
                    } else if let Some(piece) = name.strip_prefix("coverage.") {
                        let kind = parse_piece_kind(piece)?;
                        state.coverage[kind.index()] = parse(value, name)?;
                    } else {
                        return Err(format!("unknown checkpoint field `{name}`"));
                    }
                }
            }
        }
        if !saw_stage || !saw_seed {
            return Err("checkpoint lacks stage or seed".to_owned());
        }
        Ok(state)
    }
}

#[derive(Clone)]
struct PendingSample(Position);

struct TrainingGame {
    samples: Vec<MaterialSample>,
}

fn run() -> Result<(), String> {
    let arguments = env::args().skip(1).collect::<Vec<_>>();
    if arguments
        .iter()
        .any(|argument| matches!(argument.as_str(), "--help" | "-h"))
    {
        print_help();
        return Ok(());
    }
    let preset = option_value(&arguments, "--preset")?.unwrap_or("overnight");
    let mut options = Options::preset(preset)?;
    parse_options(&arguments, &mut options)?;
    validate_options(&options)?;

    let (mut dataset, mut state) = if options.resume {
        let dataset = Dataset::decode(&read_file(&options.dataset_path, "dataset")?)?;
        let state = RunState::decode(&read_file(&options.checkpoint_path, "checkpoint")?)?;
        (dataset, state)
    } else {
        if options.dataset_path.exists() || options.checkpoint_path.exists() {
            return Err(
                "dataset or checkpoint already exists; use --resume or another path".to_owned(),
            );
        }
        let dataset = Dataset::new();
        let state = RunState::new(options.seed);
        write_atomic(&options.dataset_path, &dataset.encode())?;
        write_atomic(&options.checkpoint_path, &state.encode())?;
        (dataset, state)
    };
    options.seed = state.seed;

    println!("# method=Texel+paired-self-play anchor=pawn:100 king:0");
    println!(
        "# stage={} training_pairs={}/{} samples={} jobs={} training_nodes={} total_budget_hours={:.2}",
        state.stage.name(),
        dataset.completed_pairs,
        options.training_pairs,
        dataset.samples.len(),
        options.jobs,
        options.training_nodes,
        options.total_budget.as_secs_f64() / 3600.0
    );

    if state.stage == Stage::Collect {
        collect_dataset(&options, &mut dataset, &mut state)?;
    }
    if state.stage == Stage::Fit {
        fit_dataset(&options, &dataset, &mut state)?;
    }
    if state.stage == Stage::Validate {
        validate_fit(&options, &mut state)?;
    }
    print_report(&options, &dataset, &state);
    Ok(())
}

fn collect_dataset(
    options: &Options,
    dataset: &mut Dataset,
    state: &mut RunState,
) -> Result<(), String> {
    while dataset.completed_pairs < options.training_pairs
        && dataset.collection_seconds < options.collection_budget.as_secs_f64()
        && state.total_seconds < options.total_budget.as_secs_f64()
    {
        let started = Instant::now();
        let start_pair = dataset.completed_pairs;
        let jobs = options.jobs.max(1).min(u32::MAX as usize) as u32;
        let end_pair = (start_pair + jobs).min(options.training_pairs);
        let games = training_batch(start_pair, end_pair, options)?;
        let sample_count = games.iter().map(|game| game.samples.len()).sum::<usize>();
        for game in games {
            dataset.samples.extend(game.samples);
        }
        dataset.completed_pairs = end_pair;
        let elapsed = started.elapsed().as_secs_f64();
        dataset.collection_seconds += elapsed;
        state.total_seconds += elapsed;
        write_atomic(&options.dataset_path, &dataset.encode())?;
        write_atomic(&options.checkpoint_path, &state.encode())?;
        eprintln!(
            "dataset: {}/{} pairs, {} samples (+{}), {:.2} h collection",
            dataset.completed_pairs,
            options.training_pairs,
            dataset.samples.len(),
            sample_count,
            dataset.collection_seconds / 3600.0
        );
    }
    if dataset.samples.is_empty() {
        return Err("self-play produced no quiet training positions".to_owned());
    }
    state.stage = Stage::Fit;
    write_atomic(&options.checkpoint_path, &state.encode())
}

fn training_batch(
    start_pair: u32,
    end_pair: u32,
    options: &Options,
) -> Result<Vec<TrainingGame>, String> {
    std::thread::scope(|scope| {
        let mut workers = Vec::with_capacity((end_pair - start_pair) as usize);
        for pair in start_pair..end_pair {
            workers.push(scope.spawn(move || training_pair(pair, options)));
        }
        workers
            .into_iter()
            .map(|worker| {
                worker
                    .join()
                    .map_err(|_| "training worker panicked".to_owned())?
            })
            .collect::<Result<Vec<_>, _>>()
            .map(|pairs| pairs.into_iter().flatten().collect())
    })
}

fn training_pair(pair: u32, options: &Options) -> Result<[TrainingGame; 2], String> {
    let opening = generated_opening(
        options.seed ^ u64::from(pair).wrapping_mul(0x9e37_79b9_7f4a_7c15),
        options.opening_plies,
    )?;
    let (lower, upper) = perturbed_profiles(pair, options);
    Ok([
        training_game(
            pair as u64 * 2,
            &opening,
            lower,
            upper,
            Color::White,
            options,
        )?,
        training_game(
            pair as u64 * 2 + 1,
            &opening,
            lower,
            upper,
            Color::Black,
            options,
        )?,
    ])
}

fn training_game(
    game_id: u64,
    opening: &Position,
    first: EvaluationParameters,
    second: EvaluationParameters,
    first_color: Color,
    options: &Options,
) -> Result<TrainingGame, String> {
    let search_options = SearchOptions {
        hash_megabytes: options.hash_megabytes.max(1),
        threads: 1,
    };
    let mut first_searcher = Searcher::with_evaluation(search_options, first);
    let mut second_searcher = Searcher::with_evaluation(search_options, second);
    let mut game = Game::new(opening.clone());
    let mut plies = 0_u16;
    let mut pending = Vec::<PendingSample>::new();
    let mut eligible_samples = 0_u64;
    let mut random = SplitMix64::new(options.seed ^ game_id.wrapping_mul(0xd1b5_4a32_d192_ed03));
    let mut adjudication = None::<(Color, u8)>;

    let outcome = loop {
        if let Some(outcome) = terminal_white_outcome(game.outcome()) {
            break outcome;
        }
        if plies >= options.training_maximum_plies.max(1) {
            break 0.5;
        }
        let sample_position = (plies >= options.sample_after_plies
            && (plies - options.sample_after_plies).is_multiple_of(options.sample_stride.max(1))
            && !game.position().is_in_check(game.position().side_to_move()))
        .then(|| PendingSample(game.position().clone()));

        let side = game.position().side_to_move();
        let searcher = if side == first_color {
            &mut first_searcher
        } else {
            &mut second_searcher
        };
        let analysis = searcher.analyze(
            game.position(),
            SearchLimits {
                max_depth: 191,
                max_nodes: Some(options.training_nodes.max(1)),
                ..SearchLimits::default()
            },
        );
        let chess_move = analysis.best_move.ok_or_else(|| {
            "search returned no move in a non-terminal training position".to_owned()
        })?;
        if let Some(sample) = sample_position
            && !is_capture(game.position(), chess_move)
        {
            eligible_samples += 1;
            reservoir_add(
                &mut pending,
                sample,
                eligible_samples,
                options.samples_per_game,
                &mut random,
            );
        }
        if options.adjudication_score > 0 && options.adjudication_plies > 0 {
            let favored = if analysis.score >= options.adjudication_score {
                Some(side)
            } else if analysis.score <= -options.adjudication_score {
                Some(side.opposite())
            } else {
                None
            };
            adjudication = update_adjudication(adjudication, favored);
            if let Some((winner, count)) = adjudication
                && count >= options.adjudication_plies
            {
                break if winner == Color::White { 1.0 } else { 0.0 };
            }
        }
        game.play(chess_move)
            .map_err(|error| format!("training search returned an illegal move: {error}"))?;
        plies = plies.saturating_add(1);
    };

    Ok(TrainingGame {
        samples: pending
            .into_iter()
            .map(|pending| MaterialSample::from_position(game_id, outcome, &pending.0))
            .collect(),
    })
}

fn is_capture(position: &Position, chess_move: Move) -> bool {
    chess_move.kind == MoveKind::EnPassant || position.board().piece_at(chess_move.to).is_some()
}

fn reservoir_add(
    samples: &mut Vec<PendingSample>,
    sample: PendingSample,
    eligible: u64,
    capacity: usize,
    random: &mut SplitMix64,
) {
    if capacity == 0 {
        return;
    }
    if samples.len() < capacity {
        samples.push(sample);
        return;
    }
    let selected = random.next() % eligible;
    if selected < capacity as u64 {
        samples[selected as usize] = sample;
    }
}

fn perturbed_profiles(
    pair: u32,
    options: &Options,
) -> (EvaluationParameters, EvaluationParameters) {
    let mut lower = EvaluationParameters::published();
    let mut upper = EvaluationParameters::published();
    let mut random =
        SplitMix64::new(options.seed ^ u64::from(pair + 1).wrapping_mul(0xa076_1d64_78bd_642f));
    for kind in TUNED_PIECES {
        let direction = if random.next() & 1 == 0 { -1.0 } else { 1.0 };
        let value = f64::from(EvaluationParameters::published().material_value(kind));
        lower.set_material_value(
            kind,
            (value * (-options.profile_perturbation * direction).exp()).round() as i32,
        );
        upper.set_material_value(
            kind,
            (value * (options.profile_perturbation * direction).exp()).round() as i32,
        );
    }
    (lower, upper)
}

fn fit_dataset(options: &Options, dataset: &Dataset, state: &mut RunState) -> Result<(), String> {
    let started = Instant::now();
    eprintln!("fitting {} positions...", dataset.samples.len());
    let fit = fit_material(&dataset.samples, options.texel);
    let intervals = bootstrap_material(&dataset.samples, &fit, options.texel);
    store_fit(state, &fit, &intervals);
    state.total_seconds += started.elapsed().as_secs_f64();
    state.stage = Stage::Validate;
    write_atomic(&options.checkpoint_path, &state.encode())
}

fn store_fit(state: &mut RunState, fit: &MaterialFit, intervals: &MaterialIntervals) {
    state.fitted = fit.parameters;
    state.intervals = intervals.bounds;
    state.coverage = fit.coverage;
    state.baseline_train_loss = fit.baseline_train_loss;
    state.baseline_holdout_loss = fit.baseline_holdout_loss;
    state.fitted_train_loss = fit.fitted_train_loss;
    state.fitted_holdout_loss = fit.fitted_holdout_loss;
    state.train_samples = fit.train_samples;
    state.holdout_samples = fit.holdout_samples;
}

fn validate_fit(options: &Options, state: &mut RunState) -> Result<(), String> {
    while state.validation_pair_scores.len() < options.validation_maximum_pairs as usize
        && state.total_seconds < options.total_budget.as_secs_f64()
    {
        let started = Instant::now();
        let completed = state.validation_pair_scores.len() as u32;
        let batch_pairs = options
            .jobs
            .max(1)
            .min((options.validation_maximum_pairs - completed) as usize)
            as u32;
        let summary = compare_profiles(
            EvaluationParameters::published(),
            state.fitted,
            SelfPlayConfig {
                minimum_pairs: batch_pairs,
                maximum_pairs: batch_pairs,
                nodes_per_move: options.validation_nodes,
                maximum_plies: options.validation_maximum_plies,
                opening_plies: options.opening_plies,
                hash_megabytes: options.hash_megabytes,
                jobs: options.jobs,
                adjudication_score: options.adjudication_score,
                adjudication_plies: options.adjudication_plies,
                seed: options.seed ^ u64::from(completed + 1).wrapping_mul(0xe703_7ed1_a0b4_28db),
            },
        )
        .map_err(|error| error.to_string())?;
        state.validation_wins += summary.wins();
        state.validation_draws += summary.draws();
        state.validation_losses += summary.losses();
        for pair in summary.games.chunks_exact(2) {
            state
                .validation_pair_scores
                .push((pair[0].result.score() + pair[1].result.score()) * 0.5);
        }
        state.total_seconds += started.elapsed().as_secs_f64();
        write_atomic(&options.checkpoint_path, &state.encode())?;
        let (lower, upper) = confidence_interval(&state.validation_pair_scores);
        eprintln!(
            "validation: {}/{} pairs, score {:.3}, ci95 [{lower:.3}, {upper:.3}]",
            state.validation_pair_scores.len(),
            options.validation_maximum_pairs,
            mean(&state.validation_pair_scores)
        );
        if state.validation_pair_scores.len() >= options.validation_minimum_pairs as usize
            && (lower > 0.5 || upper < 0.5)
        {
            break;
        }
    }
    state.stage = Stage::Complete;
    write_atomic(&options.checkpoint_path, &state.encode())
}

fn print_report(options: &Options, dataset: &Dataset, state: &RunState) {
    println!(
        "dataset_games={} dataset_positions={} train_positions={} holdout_positions={}",
        dataset.completed_pairs * 2,
        dataset.samples.len(),
        state.train_samples,
        state.holdout_samples
    );
    println!(
        "loss baseline_train={} fitted_train={} baseline_holdout={} fitted_holdout={}",
        state.baseline_train_loss,
        state.fitted_train_loss,
        state.baseline_holdout_loss,
        state.fitted_holdout_loss
    );
    println!("piece,published,fitted,ci95_low,ci95_high,coverage");
    let published = EvaluationParameters::published();
    for kind in PieceKind::ALL {
        let index = kind.index();
        println!(
            "{},{},{},{},{},{}",
            piece_name(kind),
            published.material_value(kind),
            state.fitted.material_value(kind),
            state.intervals[index].0,
            state.intervals[index].1,
            state.coverage[index]
        );
    }
    let (lower, upper) = confidence_interval(&state.validation_pair_scores);
    let coverage_complete = TUNED_PIECES
        .iter()
        .all(|kind| state.coverage[kind.index()] >= 100);
    let holdout_improved = state.fitted_holdout_loss < state.baseline_holdout_loss;
    let enough_validation =
        state.validation_pair_scores.len() >= options.validation_minimum_pairs as usize;
    let recommendation =
        if enough_validation && lower > 0.5 && coverage_complete && holdout_improved {
            "true"
        } else if enough_validation && upper < 0.5 {
            "false"
        } else {
            "inconclusive"
        };
    println!(
        "validation_pairs={} wins={} draws={} losses={} score={:.4} ci95_lower={lower:.4} ci95_upper={upper:.4}",
        state.validation_pair_scores.len(),
        state.validation_wins,
        state.validation_draws,
        state.validation_losses,
        mean(&state.validation_pair_scores)
    );
    for kind in PieceKind::ALL {
        println!(
            "material.{}={}",
            piece_name(kind),
            state.fitted.material_value(kind)
        );
    }
    println!("recommended={recommendation}");
}

fn generated_opening(seed: u64, opening_plies: u8) -> Result<Position, String> {
    let mut position = Variant::TerachessII.starting_position();
    let mut random = SplitMix64::new(seed);
    for _ in 0..opening_plies {
        let mut moves = position.legal_moves();
        if moves.is_empty() {
            break;
        }
        moves.sort_unstable_by_key(|chess_move| chess_move.to_uci());
        let chess_move = moves[random.index(moves.len())];
        position
            .play(chess_move)
            .map_err(|error| format!("opening generator produced an illegal move: {error}"))?;
    }
    Ok(position)
}

fn terminal_white_outcome(outcome: GameOutcome) -> Option<f64> {
    match outcome {
        GameOutcome::Win {
            winner: Color::White,
        } => Some(1.0),
        GameOutcome::Win {
            winner: Color::Black,
        } => Some(0.0),
        GameOutcome::Draw(_) => Some(0.5),
        GameOutcome::Ongoing | GameOutcome::Check => None,
    }
}

fn update_adjudication(
    previous: Option<(Color, u8)>,
    favored: Option<Color>,
) -> Option<(Color, u8)> {
    match (previous, favored) {
        (Some((previous, count)), Some(current)) if previous == current => {
            Some((current, count.saturating_add(1)))
        }
        (_, Some(current)) => Some((current, 1)),
        (_, None) => None,
    }
}

fn confidence_interval(scores: &[f64]) -> (f64, f64) {
    if scores.len() < 2 {
        return (0.0, 1.0);
    }
    let average = mean(scores);
    let variance = scores
        .iter()
        .map(|score| (score - average).powi(2))
        .sum::<f64>()
        / (scores.len() - 1) as f64;
    let margin = 1.96 * (variance / scores.len() as f64).sqrt();
    ((average - margin).max(0.0), (average + margin).min(1.0))
}

fn mean(values: &[f64]) -> f64 {
    if values.is_empty() {
        0.5
    } else {
        values.iter().sum::<f64>() / values.len() as f64
    }
}

fn write_atomic(path: &Path, contents: &str) -> Result<(), String> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)
            .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
    }
    let extension = path.extension().map_or_else(String::new, |extension| {
        extension.to_string_lossy().into_owned()
    });
    let temporary = path.with_extension(format!("{extension}.tmp"));
    fs::write(&temporary, contents)
        .map_err(|error| format!("cannot write {}: {error}", temporary.display()))?;
    fs::rename(&temporary, path)
        .map_err(|error| format!("cannot replace {}: {error}", path.display()))
}

fn read_file(path: &Path, kind: &str) -> Result<String, String> {
    fs::read_to_string(path)
        .map_err(|error| format!("cannot read {kind} {}: {error}", path.display()))
}

fn option_value<'a>(arguments: &'a [String], option: &str) -> Result<Option<&'a str>, String> {
    if let Some(index) = arguments.iter().position(|argument| argument == option) {
        return arguments
            .get(index + 1)
            .map(String::as_str)
            .map(Some)
            .ok_or_else(|| format!("{option} requires a value"));
    }
    Ok(None)
}

fn parse_options(arguments: &[String], options: &mut Options) -> Result<(), String> {
    let mut cursor = 0;
    while cursor < arguments.len() {
        let argument = arguments[cursor].as_str();
        if argument == "--resume" {
            options.resume = true;
            cursor += 1;
            continue;
        }
        let value = arguments
            .get(cursor + 1)
            .ok_or_else(|| format!("{argument} requires a value"))?;
        match argument {
            "--preset" => {}
            "--jobs" => options.jobs = parse(value, argument)?,
            "--hash" => options.hash_megabytes = parse(value, argument)?,
            "--training-pairs" => options.training_pairs = parse(value, argument)?,
            "--training-nodes" => options.training_nodes = parse(value, argument)?,
            "--training-max-plies" => options.training_maximum_plies = parse(value, argument)?,
            "--validation-min-pairs" => options.validation_minimum_pairs = parse(value, argument)?,
            "--validation-max-pairs" => options.validation_maximum_pairs = parse(value, argument)?,
            "--validation-nodes" => options.validation_nodes = parse(value, argument)?,
            "--collection-hours" => options.collection_budget = parse_hours(value, argument)?,
            "--total-hours" => options.total_budget = parse_hours(value, argument)?,
            "--seed" => options.seed = parse_seed(value)?,
            "--dataset" => options.dataset_path = PathBuf::from(value),
            "--checkpoint" => options.checkpoint_path = PathBuf::from(value),
            _ => return Err(format!("unknown argument `{argument}`; use --help")),
        }
        cursor += 2;
    }
    Ok(())
}

fn validate_options(options: &Options) -> Result<(), String> {
    if options.training_pairs == 0
        || options.training_nodes == 0
        || options.jobs == 0
        || options.hash_megabytes == 0
        || options.samples_per_game == 0
        || options.collection_budget.is_zero()
        || options.total_budget <= options.collection_budget
        || options.validation_minimum_pairs == 0
        || options.validation_maximum_pairs < options.validation_minimum_pairs
        || options.validation_nodes == 0
    {
        return Err("invalid zero or inconsistent overnight limits".to_owned());
    }
    Ok(())
}

fn parse<T>(value: &str, name: &str) -> Result<T, String>
where
    T: std::str::FromStr,
{
    value
        .parse()
        .map_err(|_| format!("invalid value `{value}` for {name}"))
}

fn parse_seed(value: &str) -> Result<u64, String> {
    value
        .strip_prefix("0x")
        .map_or_else(|| value.parse(), |hex| u64::from_str_radix(hex, 16))
        .map_err(|_| format!("invalid seed `{value}`"))
}

fn parse_hours(value: &str, name: &str) -> Result<Duration, String> {
    let hours = parse::<f64>(value, name)?;
    if !hours.is_finite() || hours <= 0.0 || hours > 24.0 * 365.0 {
        return Err(format!("invalid hour budget `{value}` for {name}"));
    }
    Ok(Duration::from_secs_f64(hours * 3600.0))
}

const fn piece_name(kind: PieceKind) -> &'static str {
    match kind {
        PieceKind::Pawn => "pawn",
        PieceKind::Knight => "knight",
        PieceKind::Bishop => "bishop",
        PieceKind::Rook => "rook",
        PieceKind::Queen => "queen",
        PieceKind::King => "king",
        PieceKind::Archbishop => "archbishop",
        PieceKind::Chancellor => "chancellor",
        PieceKind::Cannon => "cannon",
        PieceKind::Elephant => "elephant",
        PieceKind::Camel => "camel",
        PieceKind::Giraffe => "giraffe",
        PieceKind::Archer => "archer",
        PieceKind::Machine => "machine",
        PieceKind::Amazon => "amazon",
        PieceKind::Lion => "lion",
        PieceKind::Buffalo => "buffalo",
        PieceKind::Centaur => "centaur",
        PieceKind::Admiral => "admiral",
        PieceKind::Missionary => "missionary",
        PieceKind::Eagle => "eagle",
        PieceKind::Rhinoceros => "rhinoceros",
        PieceKind::Prince => "prince",
        PieceKind::Sorceress => "sorceress",
        PieceKind::Duchess => "duchess",
        PieceKind::Troll => "troll",
    }
}

fn parse_piece_kind(name: &str) -> Result<PieceKind, String> {
    PieceKind::ALL
        .into_iter()
        .find(|kind| piece_name(*kind) == name)
        .ok_or_else(|| format!("unknown piece kind `{name}`"))
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

fn print_help() {
    println!(
        "TeraStockfish overnight Texel material tuner\n\
         \n\
         Usage: cargo run --release -p terastockfish --bin terastockfish-texel -- [OPTIONS]\n\
         \n\
         Options:\n\
           --preset overnight|smoke   overnight is the default\n\
           --jobs N                   Concurrent self-play pairs (default: 12)\n\
           --hash MB                  Hash per engine and job (default: 16)\n\
           --training-pairs N         Dataset pair cap (default: 512)\n\
           --training-nodes N         Nodes per training move (default: 20000)\n\
           --training-max-plies N     Training game draw limit (default: 240)\n\
           --validation-min-pairs N   Earliest validation stop (default: 24)\n\
           --validation-max-pairs N   Validation pair cap (default: 96)\n\
           --validation-nodes N       Nodes per validation move (default: 50000)\n\
           --collection-hours H       Dataset time budget (default: 8)\n\
           --total-hours H            Total cumulative budget (default: 12)\n\
           --seed N|0xHEX             Reproducible seed\n\
           --dataset PATH             Dataset CSV path\n\
           --checkpoint PATH          Run checkpoint path\n\
           --resume                   Resume dataset, fit, or validation"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dataset_round_trips_metadata_and_samples() {
        let mut dataset = Dataset::new();
        dataset.completed_pairs = 3;
        dataset.collection_seconds = 4.5;
        let mut differences = [0; PieceKind::COUNT];
        differences[PieceKind::Queen.index()] = 1;
        dataset.samples.push(MaterialSample {
            game_id: 5,
            outcome: 1.0,
            fixed_score: 12,
            piece_differences: differences,
        });
        assert_eq!(
            Dataset::decode(&dataset.encode()).unwrap().samples,
            dataset.samples
        );
        assert_eq!(
            Dataset::decode(&dataset.encode()).unwrap().completed_pairs,
            3
        );
    }

    #[test]
    fn checkpoint_round_trips_fit_and_validation_progress() {
        let mut state = RunState::new(42);
        state.stage = Stage::Validate;
        state.fitted.set_material_value(PieceKind::Queen, 1_723);
        state.intervals[PieceKind::Queen.index()] = (1_650, 1_790);
        state.coverage[PieceKind::Queen.index()] = 123;
        state.validation_pair_scores = vec![0.5, 1.0];
        let decoded = RunState::decode(&state.encode()).unwrap();
        assert_eq!(decoded.stage, Stage::Validate);
        assert_eq!(decoded.seed, 42);
        assert_eq!(decoded.fitted.material_value(PieceKind::Queen), 1_723);
        assert_eq!(decoded.validation_pair_scores, vec![0.5, 1.0]);
    }

    #[test]
    fn reservoir_sampling_never_exceeds_capacity() {
        let position = Variant::TerachessII.starting_position();
        let mut samples = Vec::new();
        let mut random = SplitMix64::new(1);
        for eligible in 1..=100 {
            reservoir_add(
                &mut samples,
                PendingSample(position.clone()),
                eligible,
                7,
                &mut random,
            );
        }
        assert_eq!(samples.len(), 7);
    }
}
