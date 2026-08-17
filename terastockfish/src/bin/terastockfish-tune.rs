use capablanca_chess_plus::PieceKind;
use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;
use terastockfish::EvaluationParameters;
use terastockfish::validation::{SelfPlayConfig, SelfPlaySummary, compare_profiles_with};

const CHECKPOINT_VERSION: &str = "TERASTOCKFISH_MATERIAL_V1";
const ACTIVE_PIECES: [PieceKind; PieceKind::COUNT - 2] = [
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

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("terastockfish-tune: {error}");
            ExitCode::FAILURE
        }
    }
}

#[derive(Clone, Debug)]
struct Options {
    iterations: u32,
    pairs_per_iteration: u32,
    nodes_per_move: u64,
    maximum_plies: u16,
    opening_plies: u8,
    hash_megabytes: usize,
    jobs: usize,
    adjudication_score: i32,
    adjudication_plies: u8,
    seed: u64,
    perturbation_start: f64,
    perturbation_end: f64,
    learning_rate: f64,
    checkpoint: PathBuf,
    resume: bool,
    validation_minimum_pairs: u32,
    validation_maximum_pairs: u32,
    validation_nodes_per_move: u64,
    skip_validation: bool,
}

impl Options {
    fn preset(name: &str) -> Result<Self, String> {
        let mut options = Self {
            iterations: 64,
            pairs_per_iteration: 8,
            nodes_per_move: 25_000,
            maximum_plies: 300,
            opening_plies: 6,
            hash_megabytes: 16,
            jobs: 1,
            adjudication_score: 800,
            adjudication_plies: 12,
            seed: 0x5350_5341_5445_5241,
            perturbation_start: 0.08,
            perturbation_end: 0.025,
            learning_rate: 0.12,
            checkpoint: PathBuf::from("target/terastockfish-material.chk"),
            resume: false,
            validation_minimum_pairs: 50,
            validation_maximum_pairs: 200,
            validation_nodes_per_move: 25_000,
            skip_validation: false,
        };
        match name {
            "quick" => {
                options.iterations = 12;
                options.pairs_per_iteration = 2;
                options.nodes_per_move = 2_000;
                options.maximum_plies = 80;
                options.hash_megabytes = 4;
                options.validation_minimum_pairs = 5;
                options.validation_maximum_pairs = 10;
                options.validation_nodes_per_move = 5_000;
            }
            "standard" => {}
            "deep" => {
                options.iterations = 128;
                options.pairs_per_iteration = 16;
                options.nodes_per_move = 100_000;
                options.maximum_plies = 400;
                options.opening_plies = 8;
                options.hash_megabytes = 32;
                options.validation_nodes_per_move = 100_000;
            }
            "smoke" => {
                options.iterations = 1;
                options.pairs_per_iteration = 1;
                options.nodes_per_move = 128;
                options.maximum_plies = 4;
                options.opening_plies = 2;
                options.hash_megabytes = 1;
                options.adjudication_score = 0;
                options.adjudication_plies = 0;
                options.skip_validation = true;
            }
            _ => return Err(format!("unknown preset `{name}`")),
        }
        Ok(options)
    }
}

#[derive(Clone, Debug)]
struct TuningState {
    completed_iterations: u32,
    seed: u64,
    values: [f64; PieceKind::COUNT],
}

impl TuningState {
    fn published(seed: u64) -> Self {
        let parameters = EvaluationParameters::published();
        let mut values = [0.0; PieceKind::COUNT];
        for kind in PieceKind::ALL {
            values[kind.index()] = f64::from(parameters.material_value(kind));
        }
        Self {
            completed_iterations: 0,
            seed,
            values,
        }
    }

    fn parameters(&self) -> EvaluationParameters {
        let mut parameters = EvaluationParameters::published();
        for kind in ACTIVE_PIECES {
            parameters.set_material_value(kind, self.values[kind.index()].round() as i32);
        }
        parameters.set_material_value(PieceKind::Pawn, 100);
        parameters.set_material_value(PieceKind::King, 0);
        parameters
    }

    fn checkpoint_text(&self) -> String {
        let mut output = format!(
            "{CHECKPOINT_VERSION}\ncompleted_iterations={}\nseed={}\n",
            self.completed_iterations, self.seed
        );
        for kind in PieceKind::ALL {
            output.push_str(&format!(
                "{}={:.12}\n",
                piece_name(kind),
                self.values[kind.index()]
            ));
        }
        output
    }

    fn from_checkpoint(contents: &str) -> Result<Self, String> {
        let mut lines = contents.lines();
        if lines.next() != Some(CHECKPOINT_VERSION) {
            return Err("unsupported or corrupt checkpoint".to_owned());
        }
        let mut state = Self::published(0);
        let mut saw_iteration = false;
        let mut saw_seed = false;
        let mut saw_piece = [false; PieceKind::COUNT];
        for line in lines {
            let (name, value) = line
                .split_once('=')
                .ok_or_else(|| format!("invalid checkpoint line `{line}`"))?;
            match name {
                "completed_iterations" => {
                    state.completed_iterations = value
                        .parse()
                        .map_err(|_| "invalid checkpoint iteration".to_owned())?;
                    saw_iteration = true;
                }
                "seed" => {
                    state.seed = value
                        .parse()
                        .map_err(|_| "invalid checkpoint seed".to_owned())?;
                    saw_seed = true;
                }
                _ => {
                    let kind = parse_piece_kind(name)?;
                    state.values[kind.index()] = value
                        .parse()
                        .map_err(|_| format!("invalid value for `{name}` in checkpoint"))?;
                    saw_piece[kind.index()] = true;
                }
            }
        }
        if !saw_iteration || !saw_seed || saw_piece.contains(&false) {
            return Err("checkpoint lacks iteration, seed, or material values".to_owned());
        }
        if ACTIVE_PIECES.iter().any(|kind| {
            let value = state.values[kind.index()];
            !value.is_finite() || value <= 0.0
        }) {
            return Err("checkpoint contains invalid material values".to_owned());
        }
        state.values[PieceKind::Pawn.index()] = 100.0;
        state.values[PieceKind::King.index()] = 0.0;
        Ok(state)
    }
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
    let preset = if arguments.iter().any(|argument| argument == "--preset") {
        find_option(&arguments, "--preset").ok_or_else(|| "--preset requires a value".to_owned())?
    } else {
        "standard"
    };
    let mut options = Options::preset(preset)?;
    parse_options(&arguments, &mut options)?;
    validate_options(&options)?;

    let mut state = if options.resume {
        let contents = fs::read_to_string(&options.checkpoint).map_err(|error| {
            format!(
                "cannot read checkpoint {}: {error}",
                options.checkpoint.display()
            )
        })?;
        TuningState::from_checkpoint(&contents)?
    } else {
        TuningState::published(options.seed)
    };
    options.seed = state.seed;

    let maximum_tuning_games = u64::from(
        options
            .iterations
            .saturating_sub(state.completed_iterations),
    ) * u64::from(options.pairs_per_iteration)
        * 2;
    println!("# algorithm=SPSA material_anchor=pawn:100 king:0");
    println!(
        "# iterations={} completed={} pairs_per_iteration={} nodes_per_move={} jobs={} maximum_remaining_tuning_games={maximum_tuning_games}",
        options.iterations,
        state.completed_iterations,
        options.pairs_per_iteration,
        options.nodes_per_move,
        options.jobs
    );
    println!("# checkpoint={}", options.checkpoint.display());
    print_iteration_header();

    while state.completed_iterations < options.iterations {
        let started = Instant::now();
        let iteration = state.completed_iterations;
        let perturbation = perturbation_for(iteration, &options);
        let learning_rate = learning_rate_for(iteration, &options);
        let deltas = perturbation_directions(state.seed, iteration);
        let (minus, plus) = perturbed_profiles(&state, &deltas, perturbation);
        let summary = compare_profiles_with(
            minus,
            plus,
            SelfPlayConfig {
                minimum_pairs: options.pairs_per_iteration,
                maximum_pairs: options.pairs_per_iteration,
                nodes_per_move: options.nodes_per_move,
                maximum_plies: options.maximum_plies,
                opening_plies: options.opening_plies,
                hash_megabytes: options.hash_megabytes,
                jobs: options.jobs,
                adjudication_score: options.adjudication_score,
                adjudication_plies: options.adjudication_plies,
                seed: state.seed ^ u64::from(iteration + 1).wrapping_mul(0xd1b5_4a32_d192_ed03),
            },
            |progress| {
                eprintln!(
                    "tuning iteration {}: {}/{} games, score {:.3}",
                    iteration + 1,
                    progress.games.len(),
                    options.pairs_per_iteration * 2,
                    progress.score()
                );
            },
        )
        .map_err(|error| error.to_string())?;
        let signal = summary.score().mul_add(2.0, -1.0);
        apply_spsa_update(&mut state, &deltas, signal, perturbation, learning_rate);
        state.completed_iterations += 1;
        write_checkpoint(&options.checkpoint, &state)?;
        print_iteration(
            state.completed_iterations,
            perturbation,
            learning_rate,
            &summary,
            started.elapsed().as_secs_f64(),
            &state,
        );
    }

    let tuned = state.parameters();
    println!("# final_material_values");
    print_profile(&tuned);
    if !options.skip_validation {
        println!("# final_validation=published_vs_tuned");
        let summary = compare_profiles_with(
            EvaluationParameters::published(),
            tuned,
            SelfPlayConfig {
                minimum_pairs: options.validation_minimum_pairs,
                maximum_pairs: options.validation_maximum_pairs,
                nodes_per_move: options.validation_nodes_per_move,
                maximum_plies: options.maximum_plies,
                opening_plies: options.opening_plies,
                hash_megabytes: options.hash_megabytes,
                jobs: options.jobs,
                adjudication_score: options.adjudication_score,
                adjudication_plies: options.adjudication_plies,
                seed: state.seed ^ 0xa076_1d64_78bd_642f,
            },
            |progress| {
                let (lower, upper) = progress.confidence_interval_95();
                eprintln!(
                    "final validation: {} games, score {:.3}, ci95 [{lower:.3}, {upper:.3}]",
                    progress.games.len(),
                    progress.score()
                );
            },
        )
        .map_err(|error| error.to_string())?;
        print_validation(&summary);
    }
    Ok(())
}

fn perturbed_profiles(
    state: &TuningState,
    deltas: &[i8; PieceKind::COUNT],
    perturbation: f64,
) -> (EvaluationParameters, EvaluationParameters) {
    let mut minus = EvaluationParameters::published();
    let mut plus = EvaluationParameters::published();
    for kind in ACTIVE_PIECES {
        let direction = f64::from(deltas[kind.index()]);
        let current = state.values[kind.index()];
        minus.set_material_value(
            kind,
            (current * (-perturbation * direction).exp()).round() as i32,
        );
        plus.set_material_value(
            kind,
            (current * (perturbation * direction).exp()).round() as i32,
        );
    }
    (minus, plus)
}

fn apply_spsa_update(
    state: &mut TuningState,
    deltas: &[i8; PieceKind::COUNT],
    signal: f64,
    perturbation: f64,
    learning_rate: f64,
) {
    let common_gradient = signal / (2.0 * perturbation);
    for kind in ACTIVE_PIECES {
        let direction = f64::from(deltas[kind.index()]);
        let log_step = (learning_rate * common_gradient * direction).clamp(-0.08, 0.08);
        state.values[kind.index()] =
            (state.values[kind.index()] * log_step.exp()).clamp(50.0, 4_000.0);
    }
    state.values[PieceKind::Pawn.index()] = 100.0;
    state.values[PieceKind::King.index()] = 0.0;
}

fn perturbation_directions(seed: u64, iteration: u32) -> [i8; PieceKind::COUNT] {
    let mut random =
        SplitMix64::new(seed ^ u64::from(iteration + 1).wrapping_mul(0x9e37_79b9_7f4a_7c15));
    let mut deltas = [0; PieceKind::COUNT];
    for kind in ACTIVE_PIECES {
        deltas[kind.index()] = if random.next() & 1 == 0 { -1 } else { 1 };
    }
    deltas
}

fn perturbation_for(iteration: u32, options: &Options) -> f64 {
    (options.perturbation_start / f64::from(iteration + 1).powf(0.2)).max(options.perturbation_end)
}

fn learning_rate_for(iteration: u32, options: &Options) -> f64 {
    options.learning_rate / (1.0 + f64::from(iteration) / 10.0).powf(0.602)
}

fn write_checkpoint(path: &Path, state: &TuningState) -> Result<(), String> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)
            .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
    }
    let temporary = path.with_extension("chk.tmp");
    fs::write(&temporary, state.checkpoint_text()).map_err(|error| {
        format!(
            "cannot write temporary checkpoint {}: {error}",
            temporary.display()
        )
    })?;
    fs::rename(&temporary, path)
        .map_err(|error| format!("cannot replace checkpoint {}: {error}", path.display()))
}

fn print_iteration_header() {
    print!("iteration,plus_score,wins,draws,losses,perturbation,learning_rate,seconds");
    for kind in ACTIVE_PIECES {
        print!(",{}", piece_name(kind));
    }
    println!();
}

fn print_iteration(
    iteration: u32,
    perturbation: f64,
    learning_rate: f64,
    summary: &SelfPlaySummary,
    seconds: f64,
    state: &TuningState,
) {
    print!(
        "{iteration},{:.4},{},{},{},{perturbation:.6},{learning_rate:.6},{seconds:.3}",
        summary.score(),
        summary.wins(),
        summary.draws(),
        summary.losses()
    );
    for kind in ACTIVE_PIECES {
        print!(",{}", state.values[kind.index()].round() as i32);
    }
    println!();
    let _ = io::stdout().flush();
}

fn print_profile(parameters: &EvaluationParameters) {
    for kind in PieceKind::ALL {
        println!(
            "material.{}={}",
            piece_name(kind),
            parameters.material_value(kind)
        );
    }
}

fn print_validation(summary: &SelfPlaySummary) {
    let (lower, upper) = summary.confidence_interval_95();
    println!(
        "validation_games={} wins={} draws={} losses={} score={:.4} ci95_lower={lower:.4} ci95_upper={upper:.4} decisive={} recommended={}",
        summary.games.len(),
        summary.wins(),
        summary.draws(),
        summary.losses(),
        summary.score(),
        summary.statistically_decisive,
        summary.candidate_is_stronger()
    );
}

fn find_option<'a>(arguments: &'a [String], option: &str) -> Option<&'a str> {
    arguments
        .windows(2)
        .find(|window| window[0] == option)
        .map(|window| window[1].as_str())
}

fn parse_options(arguments: &[String], options: &mut Options) -> Result<(), String> {
    let mut cursor = 0;
    while cursor < arguments.len() {
        let argument = arguments[cursor].as_str();
        match argument {
            "--preset" => {
                cursor += 2;
                continue;
            }
            "--resume" => {
                options.resume = true;
                cursor += 1;
                continue;
            }
            "--skip-validation" => {
                options.skip_validation = true;
                cursor += 1;
                continue;
            }
            _ => {}
        }
        let value = arguments
            .get(cursor + 1)
            .ok_or_else(|| format!("{argument} requires a value"))?;
        match argument {
            "--iterations" => options.iterations = parse(value, argument)?,
            "--pairs" => options.pairs_per_iteration = parse(value, argument)?,
            "--nodes" => options.nodes_per_move = parse(value, argument)?,
            "--max-plies" => options.maximum_plies = parse(value, argument)?,
            "--opening-plies" => options.opening_plies = parse(value, argument)?,
            "--hash" => options.hash_megabytes = parse(value, argument)?,
            "--jobs" => options.jobs = parse(value, argument)?,
            "--adjudication-score" => options.adjudication_score = parse(value, argument)?,
            "--adjudication-plies" => options.adjudication_plies = parse(value, argument)?,
            "--seed" => options.seed = parse_seed(value)?,
            "--perturbation-start" => options.perturbation_start = parse(value, argument)?,
            "--perturbation-end" => options.perturbation_end = parse(value, argument)?,
            "--learning-rate" => options.learning_rate = parse(value, argument)?,
            "--checkpoint" => options.checkpoint = PathBuf::from(value),
            "--validation-min-pairs" => options.validation_minimum_pairs = parse(value, argument)?,
            "--validation-max-pairs" => options.validation_maximum_pairs = parse(value, argument)?,
            "--validation-nodes" => options.validation_nodes_per_move = parse(value, argument)?,
            _ => return Err(format!("unknown argument `{argument}`; use --help")),
        }
        cursor += 2;
    }
    Ok(())
}

fn validate_options(options: &Options) -> Result<(), String> {
    if options.iterations == 0
        || options.pairs_per_iteration == 0
        || options.nodes_per_move == 0
        || options.jobs == 0
        || !options.perturbation_start.is_finite()
        || !options.perturbation_end.is_finite()
        || !options.learning_rate.is_finite()
        || options.perturbation_start <= 0.0
        || options.perturbation_end <= 0.0
        || options.perturbation_end > options.perturbation_start
        || options.learning_rate <= 0.0
    {
        return Err(
            "iterations, pairs, nodes, jobs, perturbations, and learning rate must be positive"
                .to_owned(),
        );
    }
    if options.validation_minimum_pairs == 0
        || options.validation_maximum_pairs < options.validation_minimum_pairs
    {
        return Err("validation pair limits are invalid".to_owned());
    }
    Ok(())
}

fn parse<T>(value: &str, option: &str) -> Result<T, String>
where
    T: std::str::FromStr,
{
    value
        .parse()
        .map_err(|_| format!("invalid value `{value}` for {option}"))
}

fn parse_seed(value: &str) -> Result<u64, String> {
    value
        .strip_prefix("0x")
        .map_or_else(|| value.parse(), |hex| u64::from_str_radix(hex, 16))
        .map_err(|_| format!("invalid seed `{value}`"))
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
}

fn print_help() {
    println!(
        "TeraStockfish automatic material tuner\n\
         \n\
         Usage: cargo run --release -p terastockfish --bin terastockfish-tune -- [OPTIONS]\n\
         \n\
         Presets:\n\
           --preset quick|standard|deep|smoke   standard is the default\n\
         \n\
         Main options:\n\
           --iterations N            SPSA iterations\n\
           --pairs N                 Color-swapped pairs per iteration\n\
           --nodes N                 Search nodes per move\n\
           --jobs N                  Concurrent game pairs\n\
           --max-plies N             Draw adjudication limit\n\
           --opening-plies N         Deterministic randomized opening length\n\
           --hash MB                 Hash per player in each concurrent game\n\
           --adjudication-score N    Sustained winning score; 0 disables\n\
           --adjudication-plies N    Required consecutive half-moves\n\
           --seed N|0xHEX            Reproducible perturbation/opening seed\n\
           --checkpoint PATH         Checkpoint path\n\
           --resume                  Continue an existing checkpoint\n\
           --skip-validation         Do not match final values against published\n\
         \n\
         Advanced SPSA options:\n\
           --perturbation-start F    Initial multiplicative log perturbation\n\
           --perturbation-end F      Final perturbation\n\
           --learning-rate F         Initial SPSA learning rate\n\
           --validation-min-pairs N  Earliest final statistical stop\n\
           --validation-max-pairs N  Final validation hard limit\n\
           --validation-nodes N      Nodes per move in final validation"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checkpoint_round_trips_every_material_value() {
        let mut state = TuningState::published(42);
        state.completed_iterations = 7;
        state.values[PieceKind::Queen.index()] = 1_712.25;
        let decoded = TuningState::from_checkpoint(&state.checkpoint_text()).unwrap();
        assert_eq!(decoded.completed_iterations, 7);
        assert_eq!(decoded.seed, 42);
        assert_eq!(decoded.values, state.values);
    }

    #[test]
    fn perturbations_are_reproducible_and_keep_material_anchors() {
        let state = TuningState::published(9);
        let deltas = perturbation_directions(state.seed, 3);
        assert_eq!(deltas, perturbation_directions(state.seed, 3));
        let (minus, plus) = perturbed_profiles(&state, &deltas, 0.08);
        assert_eq!(minus.material_value(PieceKind::Pawn), 100);
        assert_eq!(plus.material_value(PieceKind::Pawn), 100);
        assert_eq!(minus.material_value(PieceKind::King), 0);
        assert_eq!(plus.material_value(PieceKind::King), 0);
        for kind in ACTIVE_PIECES {
            assert_ne!(minus.material_value(kind), plus.material_value(kind));
        }
    }

    #[test]
    fn positive_signal_moves_every_value_toward_the_winning_perturbation() {
        let mut state = TuningState::published(11);
        let before = state.values;
        let deltas = perturbation_directions(state.seed, 0);
        apply_spsa_update(&mut state, &deltas, 0.25, 0.08, 0.1);
        for kind in ACTIVE_PIECES {
            let changed_up = state.values[kind.index()] > before[kind.index()];
            assert_eq!(changed_up, deltas[kind.index()] > 0, "{kind:?}");
        }
        assert_eq!(state.values[PieceKind::Pawn.index()], 100.0);
        assert_eq!(state.values[PieceKind::King.index()], 0.0);
    }
}
