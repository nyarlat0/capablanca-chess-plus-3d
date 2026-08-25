use capablanca_chess_plus::Color;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, VecDeque};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use teressa::{
    ArenaConfig, ArenaGame, ArenaPairRunner, ArenaPlayerSpec, ArenaResult, ArenaTelemetry,
    ArenaTermination, PlanKind,
};

const FORMAT_VERSION: &str = "TERESSA_ARENA_V1";
const DEFAULT_SEED: u64 = 0x5445_5245_5353_4133;

#[derive(Clone, Debug)]
struct Options {
    baseline: ArenaPlayerSpec,
    candidate: ArenaPlayerSpec,
    pairs: u32,
    jobs: usize,
    output: PathBuf,
    resume: bool,
    config: ArenaConfig,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            baseline: ArenaPlayerSpec::Model(PathBuf::from("target/teressa/bdh-context-v3")),
            candidate: ArenaPlayerSpec::Model(PathBuf::from(
                "target/teressa/bdh-context-v3-masked",
            )),
            pairs: 24,
            jobs: 1,
            output: PathBuf::from("target/teressa-arena-old-vs-masked.jsonl"),
            resume: false,
            config: ArenaConfig {
                nodes_per_move: 10_000,
                maximum_plies: 0,
                opening_plies: 8,
                hash_megabytes: 16,
                search_threads: 1,
                adjudication_score: 1_500,
                adjudication_plies: 20,
                blunder_threshold_cp: 180,
                seed: DEFAULT_SEED,
            },
        }
    }
}

impl Options {
    fn apply_preset(&mut self, preset: &str) -> Result<(), String> {
        match preset {
            "smoke" => {
                self.pairs = 1;
                self.jobs = 1;
                self.config.nodes_per_move = 128;
                self.config.maximum_plies = 4;
                self.config.opening_plies = 2;
                self.config.hash_megabytes = 1;
                self.config.search_threads = 1;
                self.config.adjudication_score = 0;
                self.config.adjudication_plies = 0;
            }
            "quick" => *self = Self::default(),
            "overnight" => {
                self.pairs = 96;
                self.config.nodes_per_move = 50_000;
                self.config.maximum_plies = 0;
                self.config.opening_plies = 8;
                self.config.hash_megabytes = 32;
                self.config.adjudication_score = 1_500;
                self.config.adjudication_plies = 20;
            }
            "deep" => {
                self.pairs = 192;
                self.config.nodes_per_move = 100_000;
                self.config.maximum_plies = 0;
                self.config.opening_plies = 8;
                self.config.hash_megabytes = 32;
                self.config.adjudication_score = 1_500;
                self.config.adjudication_plies = 20;
            }
            _ => {
                return Err(format!(
                    "unknown preset `{preset}`; expected smoke, quick, overnight, or deep"
                ));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct FileHeader {
    format: String,
    method: String,
    baseline: String,
    candidate: String,
    pairs: u32,
    config: ArenaConfig,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum StoredColor {
    White,
    Black,
}

impl From<Color> for StoredColor {
    fn from(value: Color) -> Self {
        match value {
            Color::White => Self::White,
            Color::Black => Self::Black,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct StoredGame {
    pair: u32,
    candidate_color: StoredColor,
    result: ArenaResult,
    termination: ArenaTermination,
    plies: u16,
    elapsed_ms: u64,
    opening_fen: String,
    final_fen: String,
    baseline: ArenaTelemetry,
    candidate: ArenaTelemetry,
}

impl From<ArenaGame> for StoredGame {
    fn from(game: ArenaGame) -> Self {
        Self {
            pair: game.pair,
            candidate_color: game.candidate_color.into(),
            result: game.result,
            termination: game.termination,
            plies: game.plies,
            elapsed_ms: game.elapsed.as_millis().min(u128::from(u64::MAX)) as u64,
            opening_fen: game.opening_fen,
            final_fen: game.final_fen,
            baseline: game.baseline,
            candidate: game.candidate,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "record", rename_all = "snake_case")]
enum FileRecord {
    Header { value: FileHeader },
    Game { value: Box<StoredGame> },
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("teressa-arena: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let options = parse_options()?;
    validate_options(&options)?;
    let header = FileHeader {
        format: FORMAT_VERSION.to_owned(),
        method: "fixed-sample-color-swapped-hybrid-arena".to_owned(),
        baseline: options.baseline.label(),
        candidate: options.candidate.label(),
        pairs: options.pairs,
        config: options.config,
    };
    let mut games = if options.resume {
        read_output(&options.output, &header)?
    } else {
        if options.output.exists() {
            return Err(format!(
                "{} already exists; use --resume or another --output",
                options.output.display()
            ));
        }
        Vec::new()
    };
    normalize_and_validate(&mut games, options.pairs)?;
    let completed = completed_pairs(&games);
    let pending = (0..options.pairs)
        .filter(|pair| !completed.contains_key(pair))
        .collect::<VecDeque<_>>();

    print_configuration(&options, completed.len());
    if options.jobs > 4
        && matches!(options.baseline, ArenaPlayerSpec::Model(_))
        && matches!(options.candidate, ArenaPlayerSpec::Model(_))
    {
        eprintln!(
            "warning: each arena job loads both Vulkan models; start with --jobs 1 or 2 if VRAM usage is high"
        );
    }
    if pending.is_empty() {
        print_final(&games, &options);
        return Ok(());
    }

    let queue = Arc::new(Mutex::new(pending));
    let stopped = Arc::new(AtomicBool::new(false));
    let (sender, receiver) = mpsc::channel::<(u32, Result<[ArenaGame; 2], String>)>();
    let worker_count = options.jobs.min(options.pairs as usize).max(1);
    let baseline = options.baseline.clone();
    let candidate = options.candidate.clone();
    let config = options.config;
    let mut first_error = None;

    std::thread::scope(|scope| {
        for _ in 0..worker_count {
            let queue = Arc::clone(&queue);
            let stopped = Arc::clone(&stopped);
            let sender = sender.clone();
            let baseline = baseline.clone();
            let candidate = candidate.clone();
            scope.spawn(move || {
                let mut runner = match ArenaPairRunner::new(&baseline, &candidate, config) {
                    Ok(runner) => runner,
                    Err(error) => {
                        stopped.store(true, Ordering::Relaxed);
                        let _ = sender.send((u32::MAX, Err(error)));
                        return;
                    }
                };
                while !stopped.load(Ordering::Relaxed) {
                    let pair = match queue.lock() {
                        Ok(mut queue) => queue.pop_front(),
                        Err(_) => {
                            stopped.store(true, Ordering::Relaxed);
                            let _ = sender
                                .send((u32::MAX, Err("arena work queue was poisoned".to_owned())));
                            return;
                        }
                    };
                    let Some(pair) = pair else {
                        return;
                    };
                    let result = runner.play_pair(pair);
                    if result.is_err() {
                        stopped.store(true, Ordering::Relaxed);
                    }
                    if sender.send((pair, result)).is_err() {
                        return;
                    }
                }
            });
        }
        drop(sender);
        for (pair, result) in receiver {
            match result {
                Ok(pair_games) => {
                    games.extend(pair_games.into_iter().map(StoredGame::from));
                    if let Err(error) = normalize_and_validate(&mut games, options.pairs)
                        .and_then(|()| write_output(&options.output, &header, &games))
                    {
                        stopped.store(true, Ordering::Relaxed);
                        first_error.get_or_insert(error);
                    } else {
                        print_summary("progress", &games);
                    }
                }
                Err(error) => {
                    stopped.store(true, Ordering::Relaxed);
                    first_error.get_or_insert_with(|| {
                        if pair == u32::MAX {
                            error
                        } else {
                            format!("pair {pair} failed: {error}")
                        }
                    });
                }
            }
        }
    });

    if let Some(error) = first_error {
        return Err(error);
    }
    if games.len() != options.pairs as usize * 2 {
        return Err("arena stopped before every requested pair completed".to_owned());
    }
    print_final(&games, &options);
    Ok(())
}

fn validate_options(options: &Options) -> Result<(), String> {
    if options.pairs == 0 || options.jobs == 0 {
        return Err("pairs and jobs must be positive".to_owned());
    }
    if options.baseline == options.candidate {
        return Err("baseline and candidate must be different players".to_owned());
    }
    if options.config.nodes_per_move == 0
        || options.config.hash_megabytes == 0
        || options.config.search_threads == 0
        || options.config.blunder_threshold_cp <= 0
    {
        return Err("nodes, hash, threads, and blunder threshold must be positive".to_owned());
    }
    if (options.config.adjudication_score == 0) != (options.config.adjudication_plies == 0) {
        return Err(
            "adjudication score and adjudication plies must both be zero or both be positive"
                .to_owned(),
        );
    }
    Ok(())
}

fn print_configuration(options: &Options, completed_pairs: usize) {
    println!(
        "# method=fixed-sample-color-swapped-hybrid-arena baseline={} candidate={} pairs={} completed_pairs={} nodes_per_move={} max_plies={} opening_plies={} jobs={} search_threads={} hash_mb_per_player={} adjudication_score={} adjudication_plies={} blunder_threshold_cp={} seed={} fixed_sample=true",
        options.baseline.label(),
        options.candidate.label(),
        options.pairs,
        completed_pairs,
        options.config.nodes_per_move,
        options.config.maximum_plies,
        options.config.opening_plies,
        options.jobs,
        options.config.search_threads,
        options.config.hash_megabytes,
        options.config.adjudication_score,
        options.config.adjudication_plies,
        options.config.blunder_threshold_cp,
        options.config.seed,
    );
}

fn print_final(games: &[StoredGame], options: &Options) {
    print_summary("final", games);
    print_telemetry("baseline", &aggregate(games, false));
    print_telemetry("candidate", &aggregate(games, true));
    print_plan_kinds("baseline", &aggregate(games, false));
    print_plan_kinds("candidate", &aggregate(games, true));
    print_terminations(games);
    println!("output={}", options.output.display());
}

fn print_summary(stage: &str, games: &[StoredGame]) {
    let wins = games
        .iter()
        .filter(|game| game.result == ArenaResult::Win)
        .count();
    let draws = games
        .iter()
        .filter(|game| game.result == ArenaResult::Draw)
        .count();
    let losses = games
        .iter()
        .filter(|game| game.result == ArenaResult::Loss)
        .count();
    let score = if games.is_empty() {
        0.5
    } else {
        (wins as f64 + draws as f64 * 0.5) / games.len() as f64
    };
    let (lower, upper) = confidence_interval_95(games);
    let ply_caps = games
        .iter()
        .filter(|game| game.termination == ArenaTermination::PlyLimit)
        .count();
    let decision = if stage != "final" {
        "provisional_only"
    } else if ply_caps > 0 {
        "invalid_ply_caps"
    } else if lower > 0.5 {
        "candidate_stronger"
    } else if upper < 0.5 {
        "candidate_weaker"
    } else {
        "inconclusive"
    };
    println!(
        "{stage}: pairs={} games={} wins={} draws={} losses={} score={score:.4} ci95=[{lower:.4},{upper:.4}] ply_caps={ply_caps} decision={decision}",
        games.len() / 2,
        games.len(),
        wins,
        draws,
        losses,
    );
}

fn confidence_interval_95(games: &[StoredGame]) -> (f64, f64) {
    let mut by_pair = BTreeMap::<u32, [Option<f64>; 2]>::new();
    for game in games {
        let color = match game.candidate_color {
            StoredColor::White => 0,
            StoredColor::Black => 1,
        };
        by_pair.entry(game.pair).or_insert([None, None])[color] = Some(game.result.score());
    }
    let scores = by_pair
        .values()
        .filter_map(|pair| Some((pair[0]? + pair[1]?) * 0.5))
        .collect::<Vec<_>>();
    if scores.len() < 2 {
        return (0.0, 1.0);
    }
    let count = scores.len() as f64;
    let mean = scores.iter().sum::<f64>() / count;
    let variance = scores
        .iter()
        .map(|score| (score - mean).powi(2))
        .sum::<f64>()
        / (count - 1.0);
    let margin = 1.96 * (variance / count).sqrt();
    ((mean - margin).max(0.0), (mean + margin).min(1.0))
}

fn aggregate(games: &[StoredGame], candidate: bool) -> ArenaTelemetry {
    let mut aggregate = ArenaTelemetry::default();
    for game in games {
        aggregate.add_assign(if candidate {
            &game.candidate
        } else {
            &game.baseline
        });
    }
    aggregate
}

fn print_telemetry(label: &str, telemetry: &ArenaTelemetry) {
    if telemetry.neural_moves == 0 {
        println!(
            "{label}_telemetry: moves={} nodes={} neural=false",
            telemetry.moves, telemetry.nodes
        );
        return;
    }
    let ended = telemetry.ended_plans.max(1) as f64;
    println!(
        "{label}_telemetry: moves={} nodes={} neural_moves={} vetoes={} veto_rate={:.4} blunders={} blunder_rate={:.4} mean_loss_cp={:.2} plan_starts={} continuations={} completions={} completion_rate={:.4} horizon_expirations={} invalidations={} supersessions={} safety_cancellations={} mean_ended_plan_duration={:.2}",
        telemetry.moves,
        telemetry.nodes,
        telemetry.neural_moves,
        telemetry.safety_vetoes,
        telemetry.veto_rate(),
        telemetry.blunders,
        telemetry.blunder_rate(),
        telemetry.mean_centipawn_loss(),
        telemetry.plan_starts,
        telemetry.plan_continuations,
        telemetry.plan_completions,
        telemetry.plan_completions as f64 / ended,
        telemetry.plan_horizon_expirations,
        telemetry.plan_invalidations,
        telemetry.plan_supersessions,
        telemetry.plan_safety_cancellations,
        telemetry.mean_ended_plan_duration(),
    );
}

fn print_plan_kinds(label: &str, telemetry: &ArenaTelemetry) {
    if telemetry.neural_moves == 0 {
        return;
    }
    println!("{label}_plans: kind,moves,fraction");
    for kind in PlanKind::ALL {
        let moves = telemetry.plan_kind_moves[kind.index()];
        println!(
            "{},{kind:?},{moves},{:.6}",
            label,
            moves as f64 / telemetry.neural_moves as f64
        );
    }
}

fn print_terminations(games: &[StoredGame]) {
    let mut counts = [0_usize; 6];
    for game in games {
        let index = match game.termination {
            ArenaTermination::Checkmate => 0,
            ArenaTermination::Stalemate => 1,
            ArenaTermination::FiftyMoveRule => 2,
            ArenaTermination::ThreefoldRepetition => 3,
            ArenaTermination::ScoreAdjudication => 4,
            ArenaTermination::PlyLimit => 5,
        };
        counts[index] += 1;
    }
    println!(
        "terminations: checkmate={} stalemate={} fifty_move={} threefold={} score_adjudication={} ply_limit={}",
        counts[0], counts[1], counts[2], counts[3], counts[4], counts[5]
    );
}

fn completed_pairs(games: &[StoredGame]) -> BTreeMap<u32, ()> {
    games
        .chunks_exact(2)
        .map(|pair| (pair[0].pair, ()))
        .collect()
}

fn normalize_and_validate(games: &mut [StoredGame], pairs: u32) -> Result<(), String> {
    games.sort_unstable_by_key(|game| (game.pair, game.candidate_color));
    if !games.len().is_multiple_of(2) || games.len() > pairs as usize * 2 {
        return Err("arena output contains an incomplete or oversized pair set".to_owned());
    }
    for pair in games.chunks_exact(2) {
        if pair[0].pair != pair[1].pair
            || pair[0].pair >= pairs
            || pair[0].candidate_color != StoredColor::White
            || pair[1].candidate_color != StoredColor::Black
            || pair[0].opening_fen != pair[1].opening_fen
        {
            return Err(
                "arena output contains missing, duplicate, unpaired, or mismatched games"
                    .to_owned(),
            );
        }
    }
    Ok(())
}

fn read_output(path: &Path, expected: &FileHeader) -> Result<Vec<StoredGame>, String> {
    let contents = fs::read_to_string(path)
        .map_err(|error| format!("cannot resume from {}: {error}", path.display()))?;
    let mut lines = contents.lines();
    let first = lines
        .next()
        .ok_or_else(|| format!("{} is empty", path.display()))?;
    let FileRecord::Header { value: actual } =
        serde_json::from_str(first).map_err(|error| format!("invalid arena header: {error}"))?
    else {
        return Err("arena output does not start with a header".to_owned());
    };
    if &actual != expected {
        return Err("arena output metadata does not match this command".to_owned());
    }
    lines
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let record: FileRecord = serde_json::from_str(line)
                .map_err(|error| format!("invalid arena game row: {error}"))?;
            match record {
                FileRecord::Game { value } => Ok(*value),
                FileRecord::Header { .. } => Err("duplicate arena header".to_owned()),
            }
        })
        .collect()
}

fn write_output(path: &Path, header: &FileHeader, games: &[StoredGame]) -> Result<(), String> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)
            .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
    }
    let mut output = serde_json::to_string(&FileRecord::Header {
        value: header.clone(),
    })
    .map_err(|error| format!("cannot encode arena header: {error}"))?;
    output.push('\n');
    for game in games {
        output.push_str(
            &serde_json::to_string(&FileRecord::Game {
                value: Box::new(game.clone()),
            })
            .map_err(|error| format!("cannot encode arena result: {error}"))?,
        );
        output.push('\n');
    }
    let temporary = path.with_extension("tmp");
    fs::write(&temporary, output)
        .map_err(|error| format!("cannot write {}: {error}", temporary.display()))?;
    fs::rename(&temporary, path)
        .map_err(|error| format!("cannot replace {}: {error}", path.display()))
}

fn parse_options() -> Result<Options, String> {
    let raw = env::args().skip(1).collect::<Vec<_>>();
    if raw
        .iter()
        .any(|argument| argument == "--help" || argument == "-h")
    {
        print_help();
        std::process::exit(0);
    }
    let mut options = Options::default();
    let mut preset = None;
    let mut index = 0;
    while index < raw.len() {
        if raw[index] == "--preset" {
            if preset.is_some() {
                return Err("--preset may be supplied only once".to_owned());
            }
            preset = Some(
                raw.get(index + 1)
                    .ok_or_else(|| "--preset requires a value".to_owned())?
                    .clone(),
            );
            index += 2;
        } else {
            index += 1;
        }
    }
    if let Some(preset) = preset {
        options.apply_preset(&preset)?;
    }

    let mut arguments = raw.into_iter();
    while let Some(argument) = arguments.next() {
        let next = |arguments: &mut std::vec::IntoIter<String>, name: &str| {
            arguments
                .next()
                .ok_or_else(|| format!("{name} requires a value"))
        };
        match argument.as_str() {
            "--preset" => {
                next(&mut arguments, "--preset")?;
            }
            "--baseline" => {
                options.baseline = ArenaPlayerSpec::parse(&next(&mut arguments, "--baseline")?)?;
            }
            "--candidate" => {
                options.candidate = ArenaPlayerSpec::parse(&next(&mut arguments, "--candidate")?)?;
            }
            "--pairs" => options.pairs = parse(&next(&mut arguments, "--pairs")?, "pairs")?,
            "--jobs" => options.jobs = parse(&next(&mut arguments, "--jobs")?, "jobs")?,
            "--nodes" => {
                options.config.nodes_per_move = parse(&next(&mut arguments, "--nodes")?, "nodes")?;
            }
            "--max-plies" => {
                options.config.maximum_plies =
                    parse(&next(&mut arguments, "--max-plies")?, "max plies")?;
            }
            "--opening-plies" => {
                options.config.opening_plies =
                    parse(&next(&mut arguments, "--opening-plies")?, "opening plies")?;
            }
            "--hash" => {
                options.config.hash_megabytes = parse(&next(&mut arguments, "--hash")?, "hash")?;
            }
            "--search-threads" => {
                options.config.search_threads =
                    parse(&next(&mut arguments, "--search-threads")?, "search threads")?;
            }
            "--adjudication-score" => {
                options.config.adjudication_score = parse(
                    &next(&mut arguments, "--adjudication-score")?,
                    "adjudication score",
                )?;
            }
            "--adjudication-plies" => {
                options.config.adjudication_plies = parse(
                    &next(&mut arguments, "--adjudication-plies")?,
                    "adjudication plies",
                )?;
            }
            "--blunder-threshold" => {
                options.config.blunder_threshold_cp = parse(
                    &next(&mut arguments, "--blunder-threshold")?,
                    "blunder threshold",
                )?;
            }
            "--seed" => options.config.seed = parse_u64(&next(&mut arguments, "--seed")?)?,
            "--output" => options.output = PathBuf::from(next(&mut arguments, "--output")?),
            "--resume" => options.resume = true,
            unknown => return Err(format!("unknown option `{unknown}`; use --help")),
        }
    }
    Ok(options)
}

fn parse<T: std::str::FromStr>(value: &str, name: &str) -> Result<T, String> {
    value
        .parse()
        .map_err(|_| format!("invalid {name} `{value}`"))
}

fn parse_u64(value: &str) -> Result<u64, String> {
    let value = value.replace('_', "");
    if let Some(hex) = value.strip_prefix("0x") {
        u64::from_str_radix(hex, 16).map_err(|_| format!("invalid seed `{value}`"))
    } else {
        parse(&value, "seed")
    }
}

fn print_help() {
    println!(
        "Teressa fixed-sample, color-swapped arena\n\
\n\
Usage: teressa-arena [OPTIONS]\n\
\n\
Players are `stockfish` or a Teressa checkpoint prefix.\n\
\n\
  --baseline PLAYER          baseline [target/teressa/bdh-context-v3]\n\
  --candidate PLAYER         candidate [target/teressa/bdh-context-v3-masked]\n\
  --preset NAME              smoke, quick, overnight, or deep [quick]\n\
  --pairs N                  color-swapped opening pairs\n\
  --jobs N                   concurrent pairs; each job loads its own model(s) [1]\n\
  --nodes N                  safety/search nodes per move\n\
  --hash MB                  hash per player in each job\n\
  --search-threads N         CPU search threads per player [1]\n\
  --opening-plies N          deterministic randomized opening length\n\
  --max-plies N              zero uses only chess rules/adjudication [0]\n\
  --adjudication-score CP    sustained score threshold; zero disables\n\
  --adjudication-plies N     consecutive half-moves; zero disables\n\
  --blunder-threshold CP     tactical loss counted as a blunder [180]\n\
  --seed N                   decimal or 0x-prefixed reproducibility seed\n\
  --output PATH              atomic JSONL result/checkpoint path\n\
  --resume                   continue an exactly matching output file\n"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paired_confidence_uses_pairs_instead_of_individual_games() {
        let telemetry = ArenaTelemetry::default();
        let games = vec![
            stored(0, StoredColor::White, ArenaResult::Win, &telemetry),
            stored(0, StoredColor::Black, ArenaResult::Loss, &telemetry),
            stored(1, StoredColor::White, ArenaResult::Draw, &telemetry),
            stored(1, StoredColor::Black, ArenaResult::Draw, &telemetry),
        ];
        assert_eq!(confidence_interval_95(&games), (0.5, 0.5));
    }

    fn stored(
        pair: u32,
        color: StoredColor,
        result: ArenaResult,
        telemetry: &ArenaTelemetry,
    ) -> StoredGame {
        StoredGame {
            pair,
            candidate_color: color,
            result,
            termination: ArenaTermination::ThreefoldRepetition,
            plies: 1,
            elapsed_ms: 1,
            opening_fen: "fen".to_owned(),
            final_fen: "fen".to_owned(),
            baseline: telemetry.clone(),
            candidate: telemetry.clone(),
        }
    }
}
