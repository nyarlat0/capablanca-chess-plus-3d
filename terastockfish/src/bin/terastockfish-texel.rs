use capablanca_chess_plus::{
    Color, Game, GameOutcome, Move, MoveKind, PieceKind, Position, Variant,
};
use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};
use terastockfish::profile::{MaterialProfile, parse_piece_kind, piece_name};
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
    validation_seed: Option<u64>,
    dataset_path: PathBuf,
    checkpoint_path: PathBuf,
    input_dataset_paths: Vec<PathBuf>,
    profile_output_path: Option<PathBuf>,
    profile_name: String,
    diagnostics_output_path: Option<PathBuf>,
    fit_only: bool,
    fit_restarts: u16,
    cross_validation_splits: u16,
    regularization_sweep: Vec<f64>,
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
            validation_seed: None,
            dataset_path: PathBuf::from("target/terastockfish-texel.csv"),
            checkpoint_path: PathBuf::from("target/terastockfish-texel.chk"),
            input_dataset_paths: Vec::new(),
            profile_output_path: None,
            profile_name: "texel-candidate".to_owned(),
            diagnostics_output_path: None,
            fit_only: false,
            fit_restarts: 1,
            cross_validation_splits: 0,
            regularization_sweep: Vec::new(),
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

    fn merge(datasets: &[Self]) -> Result<Self, String> {
        if datasets.is_empty() {
            return Err("at least one input dataset is required".to_owned());
        }
        let mut merged = Self::new();
        let mut next_game_id = 0_u64;
        for dataset in datasets {
            merged.completed_pairs = merged
                .completed_pairs
                .checked_add(dataset.completed_pairs)
                .ok_or_else(|| "merged pair count overflow".to_owned())?;
            merged.collection_seconds += dataset.collection_seconds;
            let mut remapped = BTreeMap::<u64, u64>::new();
            for sample in &dataset.samples {
                let game_id = if let Some(game_id) = remapped.get(&sample.game_id) {
                    *game_id
                } else {
                    let game_id = next_game_id;
                    next_game_id = next_game_id
                        .checked_add(1)
                        .ok_or_else(|| "merged game id overflow".to_owned())?;
                    remapped.insert(sample.game_id, game_id);
                    game_id
                };
                let mut sample = sample.clone();
                sample.game_id = game_id;
                merged.samples.push(sample);
            }
        }
        Ok(merged)
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

struct DiagnosticFit {
    kind: &'static str,
    index: usize,
    split_seed: u64,
    optimizer_seed: u64,
    regularization: f64,
    fit: MaterialFit,
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
        if options.dataset_path.exists()
            || options.checkpoint_path.exists()
            || options
                .profile_output_path
                .as_ref()
                .is_some_and(|path| path.exists())
            || options
                .diagnostics_output_path
                .as_ref()
                .is_some_and(|path| path.exists())
        {
            return Err(
                "dataset, checkpoint, profile, or diagnostics output already exists; use --resume or another path"
                    .to_owned(),
            );
        }
        let dataset = if options.input_dataset_paths.is_empty() {
            Dataset::new()
        } else {
            let inputs = options
                .input_dataset_paths
                .iter()
                .map(|path| Dataset::decode(&read_file(path, "input dataset")?))
                .collect::<Result<Vec<_>, _>>()?;
            Dataset::merge(&inputs)?
        };
        let mut state = RunState::new(options.seed);
        if !options.input_dataset_paths.is_empty() {
            state.stage = Stage::Fit;
        }
        write_atomic(&options.dataset_path, &dataset.encode())?;
        write_atomic(&options.checkpoint_path, &state.encode())?;
        (dataset, state)
    };
    options.seed = state.seed;

    println!("# method=Texel+paired-self-play anchor=pawn:100 king:0");
    println!(
        "# stage={} dataset_pairs={} training_pair_target={} input_datasets={} samples={} jobs={} training_nodes={} fit_only={} split_seed={} optimizer_seed={} bootstrap_seed={} total_budget_hours={:.2}",
        state.stage.name(),
        dataset.completed_pairs,
        options.training_pairs,
        options.input_dataset_paths.len(),
        dataset.samples.len(),
        options.jobs,
        options.training_nodes,
        options.fit_only,
        options.texel.split_seed,
        options.texel.optimizer_seed,
        options.texel.bootstrap_seed,
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
    if let Some(path) = &options.profile_output_path {
        write_atomic(
            path,
            &MaterialProfile::new(options.profile_name.clone(), state.fitted).encode(),
        )?;
        println!("profile_output={}", path.display());
    }
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
    let diagnostics = fit_diagnostics(options, &dataset.samples, &fit);
    if let Some(path) = &options.diagnostics_output_path {
        write_atomic(path, &encode_diagnostics(options, &diagnostics))?;
        eprintln!("fit diagnostics written to {}", path.display());
    }
    let intervals = bootstrap_material(&dataset.samples, &fit, options.texel);
    store_fit(state, &fit, &intervals);
    state.total_seconds += started.elapsed().as_secs_f64();
    state.stage = if options.fit_only {
        Stage::Complete
    } else {
        Stage::Validate
    };
    write_atomic(&options.checkpoint_path, &state.encode())
}

fn fit_diagnostics(
    options: &Options,
    samples: &[MaterialSample],
    primary: &MaterialFit,
) -> Vec<DiagnosticFit> {
    let mut diagnostics = vec![DiagnosticFit {
        kind: "restart",
        index: 0,
        split_seed: options.texel.split_seed,
        optimizer_seed: options.texel.optimizer_seed,
        regularization: options.texel.regularization,
        fit: primary.clone(),
    }];
    for index in 1..usize::from(options.fit_restarts) {
        let optimizer_seed =
            options.texel.optimizer_seed ^ (index as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15);
        let fit = fit_material(
            samples,
            TexelOptions {
                optimizer_seed,
                ..options.texel
            },
        );
        diagnostics.push(DiagnosticFit {
            kind: "restart",
            index,
            split_seed: options.texel.split_seed,
            optimizer_seed,
            regularization: options.texel.regularization,
            fit,
        });
    }
    for (index, &regularization) in options.regularization_sweep.iter().enumerate() {
        let fit = if regularization == options.texel.regularization {
            primary.clone()
        } else {
            fit_material(
                samples,
                TexelOptions {
                    regularization,
                    ..options.texel
                },
            )
        };
        diagnostics.push(DiagnosticFit {
            kind: "regularization",
            index,
            split_seed: options.texel.split_seed,
            optimizer_seed: options.texel.optimizer_seed,
            regularization,
            fit,
        });
    }
    for split_index in 0..usize::from(options.cross_validation_splits) {
        let split_seed =
            options.texel.split_seed ^ (split_index as u64 + 1).wrapping_mul(0xd1b5_4a32_d192_ed03);
        for (regularization_index, &regularization) in
            options.regularization_sweep.iter().enumerate()
        {
            let fit = fit_material(
                samples,
                TexelOptions {
                    split_seed,
                    regularization,
                    ..options.texel
                },
            );
            diagnostics.push(DiagnosticFit {
                kind: "cross_validation",
                index: split_index * options.regularization_sweep.len() + regularization_index,
                split_seed,
                optimizer_seed: options.texel.optimizer_seed,
                regularization,
                fit,
            });
        }
    }
    diagnostics
}

fn encode_diagnostics(options: &Options, diagnostics: &[DiagnosticFit]) -> String {
    let mut output = format!(
        "# TERASTOCKFISH_TEXEL_DIAGNOSTICS_V1\n# split_seed={} bootstrap_seed={} fit_epochs={} bootstrap_epochs={} bootstrap_replicates={}\n",
        options.texel.split_seed,
        options.texel.bootstrap_seed,
        options.texel.epochs,
        options.texel.bootstrap_epochs,
        options.texel.bootstrap_replicates
    );
    output.push_str(
        "kind,index,split_seed,optimizer_seed,regularization,train_loss,holdout_loss,logistic_scale",
    );
    for kind in TUNED_PIECES {
        output.push(',');
        output.push_str(piece_name(kind));
    }
    output.push('\n');
    for diagnostic in diagnostics {
        output.push_str(&format!(
            "{},{},{},{},{:.8},{:.12},{:.12},{:.12}",
            diagnostic.kind,
            diagnostic.index,
            diagnostic.split_seed,
            diagnostic.optimizer_seed,
            diagnostic.regularization,
            diagnostic.fit.fitted_train_loss,
            diagnostic.fit.fitted_holdout_loss,
            diagnostic.fit.logistic_scale
        ));
        for kind in TUNED_PIECES {
            output.push_str(&format!(
                ",{}",
                diagnostic.fit.parameters.material_value(kind)
            ));
        }
        output.push('\n');
    }
    output
        .push_str("regularization,cv_splits,mean_holdout_loss,min_holdout_loss,max_holdout_loss\n");
    for &regularization in &options.regularization_sweep {
        let losses = diagnostics
            .iter()
            .filter(|diagnostic| {
                diagnostic.kind == "cross_validation" && diagnostic.regularization == regularization
            })
            .map(|diagnostic| diagnostic.fit.fitted_holdout_loss)
            .collect::<Vec<_>>();
        if !losses.is_empty() {
            let mean = losses.iter().sum::<f64>() / losses.len() as f64;
            let minimum = losses.iter().copied().fold(f64::INFINITY, f64::min);
            let maximum = losses.iter().copied().fold(f64::NEG_INFINITY, f64::max);
            output.push_str(&format!(
                "{regularization:.8},{},{mean:.12},{minimum:.12},{maximum:.12}\n",
                losses.len()
            ));
        }
    }
    output.push_str("piece,restart_min,restart_max,restart_span,sweep_min,sweep_max,sweep_span\n");
    for piece in TUNED_PIECES {
        let range = |kind: &str| {
            diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.kind == kind)
                .map(|diagnostic| diagnostic.fit.parameters.material_value(piece))
                .fold(None, |range, value| match range {
                    None => Some((value, value)),
                    Some((minimum, maximum)) => Some((minimum.min(value), maximum.max(value))),
                })
        };
        let (restart_minimum, restart_maximum) = range("restart").unwrap_or((0, 0));
        let (sweep_minimum, sweep_maximum) =
            range("regularization").unwrap_or((restart_minimum, restart_maximum));
        output.push_str(&format!(
            "{},{},{},{},{},{},{}\n",
            piece_name(piece),
            restart_minimum,
            restart_maximum,
            restart_maximum - restart_minimum,
            sweep_minimum,
            sweep_maximum,
            sweep_maximum - sweep_minimum
        ));
    }
    output
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
                seed: options
                    .validation_seed
                    .unwrap_or(options.seed ^ 0x5641_4c49_4441_5445)
                    ^ u64::from(completed + 1).wrapping_mul(0xe703_7ed1_a0b4_28db),
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
        if matches!(argument, "--resume" | "--fit-only") {
            if argument == "--resume" {
                options.resume = true;
            } else {
                options.fit_only = true;
            }
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
            "--validation-seed" => options.validation_seed = Some(parse_seed(value)?),
            "--collection-hours" => options.collection_budget = parse_hours(value, argument)?,
            "--total-hours" => options.total_budget = parse_hours(value, argument)?,
            "--seed" => options.seed = parse_seed(value)?,
            "--dataset" => options.dataset_path = PathBuf::from(value),
            "--checkpoint" => options.checkpoint_path = PathBuf::from(value),
            "--input-dataset" => options.input_dataset_paths.push(PathBuf::from(value)),
            "--profile-output" => options.profile_output_path = Some(PathBuf::from(value)),
            "--profile-name" => options.profile_name = value.clone(),
            "--diagnostics-output" => {
                options.diagnostics_output_path = Some(PathBuf::from(value));
            }
            "--fit-restarts" => options.fit_restarts = parse(value, argument)?,
            "--cross-validation-splits" => {
                options.cross_validation_splits = parse(value, argument)?;
            }
            "--fit-epochs" => options.texel.epochs = parse(value, argument)?,
            "--bootstrap-epochs" => options.texel.bootstrap_epochs = parse(value, argument)?,
            "--bootstrap-replicates" => {
                options.texel.bootstrap_replicates = parse(value, argument)?;
            }
            "--split-seed" => options.texel.split_seed = parse_seed(value)?,
            "--optimizer-seed" => options.texel.optimizer_seed = parse_seed(value)?,
            "--bootstrap-seed" => options.texel.bootstrap_seed = parse_seed(value)?,
            "--regularization" => options.texel.regularization = parse(value, argument)?,
            "--regularization-sweep" => {
                options.regularization_sweep = parse_f64_list(value, argument)?;
            }
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
        || options.texel.bootstrap_replicates == 0
        || options.texel.epochs == 0
        || options.texel.bootstrap_epochs == 0
        || options.fit_restarts == 0
        || (options.cross_validation_splits > 0 && options.regularization_sweep.is_empty())
        || !options.texel.regularization.is_finite()
        || options.texel.regularization < 0.0
        || options
            .regularization_sweep
            .iter()
            .any(|value| !value.is_finite() || *value < 0.0)
        || options.profile_name.is_empty()
        || options.profile_name.contains(['\n', '\r'])
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

fn parse_f64_list(value: &str, name: &str) -> Result<Vec<f64>, String> {
    if value.is_empty() {
        return Err(format!("{name} requires a comma-separated list"));
    }
    value.split(',').map(|value| parse(value, name)).collect()
}

fn parse_hours(value: &str, name: &str) -> Result<Duration, String> {
    let hours = parse::<f64>(value, name)?;
    if !hours.is_finite() || hours <= 0.0 || hours > 24.0 * 365.0 {
        return Err(format!("invalid hour budget `{value}` for {name}"));
    }
    Ok(Duration::from_secs_f64(hours * 3600.0))
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
           --validation-seed N|0xHEX  Independent validation opening seed\n\
           --collection-hours H       Dataset time budget (default: 8)\n\
           --total-hours H            Total cumulative budget (default: 12)\n\
           --seed N|0xHEX             Reproducible seed\n\
           --dataset PATH             Dataset CSV path\n\
           --checkpoint PATH          Run checkpoint path\n\
           --input-dataset PATH       Merge an existing dataset; repeatable\n\
           --fit-only                 Fit/bootstrap without self-play validation\n\
           --fit-restarts N           Optimizer shuffles on one fixed split\n\
           --cross-validation-splits N  Game-level splits per regularization\n\
           --fit-epochs N             Primary/restart epoch cap (default: 48)\n\
           --split-seed N|0xHEX       Game-level train/holdout split seed\n\
           --optimizer-seed N|0xHEX   Primary optimizer shuffle seed\n\
           --bootstrap-seed N|0xHEX   Game resampling seed\n\
           --bootstrap-epochs N       Epochs per bootstrap fit (default: 12)\n\
           --bootstrap-replicates N   Game-level bootstrap fits (default: 32)\n\
           --regularization X         Published-value regularization (default: 0.01)\n\
           --regularization-sweep CSV Diagnostic regularization values\n\
           --diagnostics-output PATH  Write restart/sensitivity CSV\n\
           --profile-output PATH      Write the fitted material profile\n\
           --profile-name NAME        Name stored in that profile\n\
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
    fn merging_datasets_remaps_colliding_game_ids_without_splitting_games() {
        let sample = |game_id, difference| {
            let mut piece_differences = [0; PieceKind::COUNT];
            piece_differences[PieceKind::Queen.index()] = difference;
            MaterialSample {
                game_id,
                outcome: 0.5,
                fixed_score: 0,
                piece_differences,
            }
        };
        let first = Dataset {
            completed_pairs: 1,
            collection_seconds: 2.0,
            samples: vec![sample(0, 1), sample(0, 2)],
        };
        let second = Dataset {
            completed_pairs: 2,
            collection_seconds: 3.0,
            samples: vec![sample(0, 3), sample(1, 4)],
        };
        let merged = Dataset::merge(&[first, second]).unwrap();
        let ids = merged
            .samples
            .iter()
            .map(|sample| sample.game_id)
            .collect::<Vec<_>>();
        assert_eq!(ids, vec![0, 0, 1, 2]);
        assert_eq!(merged.completed_pairs, 3);
        assert_eq!(merged.collection_seconds, 5.0);
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

    #[test]
    fn comma_separated_regularization_values_are_parsed() {
        assert_eq!(
            parse_f64_list("0.005,0.01,0.02", "test").unwrap(),
            vec![0.005, 0.01, 0.02]
        );
        assert!(parse_f64_list("", "test").is_err());
        assert!(parse_f64_list("0.01,nope", "test").is_err());
    }
}
