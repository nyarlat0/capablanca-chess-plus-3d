use capablanca_chess_plus::Color;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;
use terastockfish::EvaluationParameters;
use terastockfish::validation::{
    CandidateResult, GameTermination, SelfPlayConfig, SelfPlayGame, SelfPlaySummary,
    play_profile_pair,
};

const FORMAT_VERSION: &str = "TERASTOCKFISH_STRATEGIC_V2";
const STUDY_IMPLEMENTATION: &str = "search-foundation-v1/strategic-v2-connected-rank-v1";
const DEFAULT_SEED: u64 = 0x5354_5241_5445_4732;

#[derive(Clone, Debug)]
struct Options {
    pairs: u32,
    jobs: usize,
    output: PathBuf,
    resume: bool,
    config: SelfPlayConfig,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            pairs: 48,
            jobs: available_jobs(),
            output: PathBuf::from("target/terastockfish-strategic-v2-quick.csv"),
            resume: false,
            config: SelfPlayConfig {
                minimum_pairs: 48,
                maximum_pairs: 48,
                nodes_per_move: 10_000,
                maximum_plies: 400,
                opening_plies: 8,
                hash_megabytes: 8,
                jobs: 1,
                adjudication_score: 1_500,
                adjudication_plies: 20,
                seed: DEFAULT_SEED,
            },
        }
    }
}

impl Options {
    fn apply_preset(&mut self, name: &str) -> Result<(), String> {
        match name {
            "smoke" => {
                self.pairs = 1;
                self.jobs = 1;
                self.output = PathBuf::from("target/terastockfish-strategic-v2-smoke.csv");
                self.config.nodes_per_move = 128;
                self.config.maximum_plies = 4;
                self.config.opening_plies = 2;
                self.config.hash_megabytes = 1;
                self.config.adjudication_score = 0;
                self.config.adjudication_plies = 0;
            }
            "quick" => *self = Self::default(),
            "overnight" => {
                self.pairs = 192;
                self.jobs = available_jobs();
                self.output = PathBuf::from("target/terastockfish-strategic-v2-overnight.csv");
                self.config.nodes_per_move = 50_000;
                self.config.maximum_plies = 0;
                self.config.opening_plies = 8;
                self.config.hash_megabytes = 16;
                self.config.adjudication_score = 1_500;
                self.config.adjudication_plies = 20;
            }
            "deep" => {
                self.pairs = 256;
                self.jobs = available_jobs();
                self.output = PathBuf::from("target/terastockfish-strategic-v2-deep.csv");
                self.config.nodes_per_move = 100_000;
                self.config.maximum_plies = 0;
                self.config.opening_plies = 8;
                self.config.hash_megabytes = 32;
                self.config.adjudication_score = 1_500;
                self.config.adjudication_plies = 20;
            }
            _ => {
                return Err(format!(
                    "unknown preset `{name}`; expected smoke, quick, overnight, or deep"
                ));
            }
        }
        self.config.minimum_pairs = self.pairs;
        self.config.maximum_pairs = self.pairs;
        Ok(())
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("terastockfish-strategic: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let options = parse_options()?;
    let baseline = EvaluationParameters::empirical_v1();
    let candidate = EvaluationParameters::strategic_v2();
    let header = header(&options, baseline, candidate);
    let mut games = if options.resume {
        let contents = fs::read_to_string(&options.output)
            .map_err(|error| format!("cannot resume from {}: {error}", options.output.display()))?;
        if !contents.starts_with(&header) {
            return Err("strategic output metadata does not match this command".to_owned());
        }
        decode_games(&contents)?
    } else {
        if options.output.exists() {
            return Err(format!(
                "{} already exists; use --resume or another --output",
                options.output.display()
            ));
        }
        Vec::new()
    };
    normalize_and_validate_games(&mut games, options.pairs)?;

    print_configuration(&options, games.len() / 2);
    while games.len() / 2 < options.pairs as usize {
        let start = (games.len() / 2) as u32;
        let end = (start + options.jobs.max(1).min(u32::MAX as usize) as u32).min(options.pairs);
        let batch = std::thread::scope(|scope| {
            let workers = (start..end)
                .map(|pair| {
                    scope
                        .spawn(move || play_profile_pair(pair, baseline, candidate, options.config))
                })
                .collect::<Vec<_>>();
            workers
                .into_iter()
                .map(|worker| {
                    worker
                        .join()
                        .map_err(|_| "strategic self-play worker panicked".to_owned())?
                        .map_err(|error| error.to_string())
                })
                .collect::<Result<Vec<_>, String>>()
        })?;
        for pair in batch {
            games.extend(pair);
        }
        normalize_and_validate_games(&mut games, options.pairs)?;
        write_atomic(&options.output, &encode(&header, &games))?;
        print_summary("progress", &games);
    }

    print_summary("final", &games);
    print_terminations(&games);
    println!("output={}", options.output.display());
    Ok(())
}

fn print_configuration(options: &Options, completed_pairs: usize) {
    println!(
        "# method=fixed-sample-color-swapped baseline=empirical-v1 candidate=strategic-v2 pairs={} completed_pairs={} nodes_per_move={} max_plies={} opening_plies={} jobs={} hash_mb_per_player={} adjudication_score={} adjudication_plies={} seed={} early_stopping=false",
        options.pairs,
        completed_pairs,
        options.config.nodes_per_move,
        options.config.maximum_plies,
        options.config.opening_plies,
        options.jobs,
        options.config.hash_megabytes,
        options.config.adjudication_score,
        options.config.adjudication_plies,
        options.config.seed
    );
}

fn print_summary(stage: &str, games: &[SelfPlayGame]) {
    let summary = summary(games);
    let (lower, upper) = summary.confidence_interval_95();
    let capped = games
        .iter()
        .filter(|game| game.termination == GameTermination::PlyLimit)
        .count();
    let valid = capped == 0;
    let decision = if stage != "final" {
        "provisional_only"
    } else if !valid {
        "invalid_ply_caps"
    } else if lower > 0.5 {
        "strategic_v2_stronger"
    } else if upper < 0.5 {
        "strategic_v2_weaker"
    } else {
        "inconclusive"
    };
    println!(
        "{stage}: pairs={} games={} wins={} draws={} losses={} score={:.4} ci95=[{lower:.4},{upper:.4}] ply_caps={capped} valid={valid} decision={decision}",
        games.len() / 2,
        games.len(),
        summary.wins(),
        summary.draws(),
        summary.losses(),
        summary.score(),
    );
}

fn print_terminations(games: &[SelfPlayGame]) {
    let mut counts = [0_usize; 6];
    for game in games {
        let index = match game.termination {
            GameTermination::Checkmate => 0,
            GameTermination::Stalemate => 1,
            GameTermination::FiftyMoveRule => 2,
            GameTermination::ThreefoldRepetition => 3,
            GameTermination::ScoreAdjudication => 4,
            GameTermination::PlyLimit => 5,
        };
        counts[index] += 1;
    }
    println!(
        "terminations: checkmate={} stalemate={} fifty_move={} threefold={} score_adjudication={} ply_limit={}",
        counts[0], counts[1], counts[2], counts[3], counts[4], counts[5]
    );
}

fn summary(games: &[SelfPlayGame]) -> SelfPlaySummary {
    SelfPlaySummary {
        games: games.to_vec(),
        statistically_decisive: false,
    }
}

fn header(
    options: &Options,
    baseline: EvaluationParameters,
    candidate: EvaluationParameters,
) -> String {
    format!(
        "# {FORMAT_VERSION}\n# study_implementation={STUDY_IMPLEMENTATION}\n# baseline=empirical-v1\n# candidate=strategic-v2\n# baseline_fingerprint={}\n# candidate_fingerprint={}\n# pairs={}\n# nodes_per_move={}\n# maximum_plies={}\n# opening_plies={}\n# hash_megabytes={}\n# adjudication_score={}\n# adjudication_plies={}\n# seed={}\npair,candidate_color,result,termination,plies,nodes,elapsed_ms,opening_fen\n",
        baseline.research_fingerprint(),
        candidate.research_fingerprint(),
        options.pairs,
        options.config.nodes_per_move,
        options.config.maximum_plies,
        options.config.opening_plies,
        options.config.hash_megabytes,
        options.config.adjudication_score,
        options.config.adjudication_plies,
        options.config.seed,
    )
}

fn encode(header: &str, games: &[SelfPlayGame]) -> String {
    let mut output = header.to_owned();
    for game in games {
        output.push_str(&format!(
            "{},{},{},{},{},{},{},{}\n",
            game.pair,
            color_name(game.candidate_color),
            result_name(game.result),
            termination_name(game.termination),
            game.plies,
            game.nodes,
            game.elapsed.as_millis(),
            game.opening_fen,
        ));
    }
    output
}

fn decode_games(contents: &str) -> Result<Vec<SelfPlayGame>, String> {
    let mut games = Vec::new();
    for line in contents.lines() {
        if line.starts_with('#') || line.starts_with("pair,") || line.is_empty() {
            continue;
        }
        let fields = line.splitn(8, ',').collect::<Vec<_>>();
        if fields.len() != 8 {
            return Err(format!("invalid strategic row `{line}`"));
        }
        games.push(SelfPlayGame {
            pair: parse(fields[0], "pair")?,
            candidate_color: parse_color(fields[1])?,
            result: parse_result(fields[2])?,
            termination: parse_termination(fields[3])?,
            plies: parse(fields[4], "plies")?,
            nodes: parse(fields[5], "nodes")?,
            elapsed: Duration::from_millis(parse(fields[6], "elapsed_ms")?),
            opening_fen: fields[7].to_owned(),
        });
    }
    Ok(games)
}

fn normalize_and_validate_games(games: &mut [SelfPlayGame], pairs: u32) -> Result<(), String> {
    games.sort_unstable_by_key(|game| (game.pair, color_index(game.candidate_color)));
    if !games.len().is_multiple_of(2) || games.len() > pairs as usize * 2 {
        return Err("strategic output contains an incomplete or oversized pair set".to_owned());
    }
    for (expected_pair, pair) in games.chunks_exact(2).enumerate() {
        if pair[0].pair as usize != expected_pair
            || pair[1].pair as usize != expected_pair
            || pair[0].candidate_color != Color::White
            || pair[1].candidate_color != Color::Black
            || pair[0].opening_fen != pair[1].opening_fen
        {
            return Err(
                "strategic output contains missing, duplicate, or unpaired games".to_owned(),
            );
        }
    }
    Ok(())
}

fn write_atomic(path: &Path, contents: &str) -> Result<(), String> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)
            .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
    }
    let temporary = path.with_extension("tmp");
    fs::write(&temporary, contents)
        .map_err(|error| format!("cannot write {}: {error}", temporary.display()))?;
    fs::rename(&temporary, path)
        .map_err(|error| format!("cannot replace {}: {error}", path.display()))
}

fn parse_options() -> Result<Options, String> {
    parse_options_from(env::args().skip(1).collect())
}

fn parse_options_from(raw_arguments: Vec<String>) -> Result<Options, String> {
    let mut options = Options::default();
    let mut preset = None;
    for (index, argument) in raw_arguments.iter().enumerate() {
        if argument == "--preset" {
            if preset.is_some() {
                return Err("--preset may be supplied only once".to_owned());
            }
            preset = Some(
                raw_arguments
                    .get(index + 1)
                    .ok_or_else(|| "--preset requires a value".to_owned())?
                    .clone(),
            );
        }
    }
    if let Some(preset) = preset {
        options.apply_preset(&preset)?;
    }

    let mut arguments = raw_arguments.into_iter();
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--help" | "-h" => {
                print_help();
                std::process::exit(0);
            }
            "--preset" => {
                arguments
                    .next()
                    .ok_or_else(|| "--preset requires a value".to_owned())?;
            }
            "--pairs" => options.pairs = parse_next(&mut arguments, &argument)?,
            "--jobs" => options.jobs = parse_next(&mut arguments, &argument)?,
            "--nodes" => options.config.nodes_per_move = parse_next(&mut arguments, &argument)?,
            "--max-plies" => {
                options.config.maximum_plies = parse_next(&mut arguments, &argument)?;
            }
            "--opening-plies" => {
                options.config.opening_plies = parse_next(&mut arguments, &argument)?;
            }
            "--hash" => options.config.hash_megabytes = parse_next(&mut arguments, &argument)?,
            "--adjudication-score" => {
                options.config.adjudication_score = parse_next(&mut arguments, &argument)?;
            }
            "--adjudication-plies" => {
                options.config.adjudication_plies = parse_next(&mut arguments, &argument)?;
            }
            "--seed" => {
                let value = arguments
                    .next()
                    .ok_or_else(|| "--seed requires a value".to_owned())?;
                options.config.seed = parse_seed(&value)?;
            }
            "--output" => {
                options.output = PathBuf::from(
                    arguments
                        .next()
                        .ok_or_else(|| "--output requires a path".to_owned())?,
                );
            }
            "--resume" => options.resume = true,
            _ => return Err(format!("unknown argument `{argument}`; use --help")),
        }
    }
    options.config.minimum_pairs = options.pairs;
    options.config.maximum_pairs = options.pairs;
    options.config.jobs = 1;
    if options.pairs == 0
        || options.jobs == 0
        || options.config.nodes_per_move == 0
        || options.config.hash_megabytes == 0
    {
        return Err("pairs, jobs, nodes, and hash must be positive".to_owned());
    }
    if !options.config.opening_plies.is_multiple_of(2) {
        return Err("--opening-plies must be even to keep paired colors comparable".to_owned());
    }
    if (options.config.adjudication_score == 0) != (options.config.adjudication_plies == 0) {
        return Err("set both adjudication options to zero or both to positive values".to_owned());
    }
    Ok(options)
}

fn parse_next<T>(arguments: &mut impl Iterator<Item = String>, name: &str) -> Result<T, String>
where
    T: std::str::FromStr,
{
    let value = arguments
        .next()
        .ok_or_else(|| format!("{name} requires a value"))?;
    parse(&value, name)
}

fn parse<T>(value: &str, name: &str) -> Result<T, String>
where
    T: std::str::FromStr,
{
    value
        .parse()
        .map_err(|_| format!("invalid {name} `{value}`"))
}

fn parse_seed(value: &str) -> Result<u64, String> {
    value
        .strip_prefix("0x")
        .map_or_else(|| value.parse(), |hex| u64::from_str_radix(hex, 16))
        .map_err(|_| format!("invalid seed `{value}`"))
}

const fn color_name(color: Color) -> &'static str {
    match color {
        Color::White => "white",
        Color::Black => "black",
    }
}

const fn color_index(color: Color) -> u8 {
    match color {
        Color::White => 0,
        Color::Black => 1,
    }
}

fn parse_color(value: &str) -> Result<Color, String> {
    match value {
        "white" => Ok(Color::White),
        "black" => Ok(Color::Black),
        _ => Err(format!("unknown candidate color `{value}`")),
    }
}

const fn result_name(result: CandidateResult) -> &'static str {
    match result {
        CandidateResult::Win => "win",
        CandidateResult::Draw => "draw",
        CandidateResult::Loss => "loss",
    }
}

fn parse_result(value: &str) -> Result<CandidateResult, String> {
    match value {
        "win" => Ok(CandidateResult::Win),
        "draw" => Ok(CandidateResult::Draw),
        "loss" => Ok(CandidateResult::Loss),
        _ => Err(format!("unknown candidate result `{value}`")),
    }
}

const fn termination_name(termination: GameTermination) -> &'static str {
    match termination {
        GameTermination::Checkmate => "checkmate",
        GameTermination::Stalemate => "stalemate",
        GameTermination::FiftyMoveRule => "fifty_move",
        GameTermination::ThreefoldRepetition => "threefold",
        GameTermination::ScoreAdjudication => "score_adjudication",
        GameTermination::PlyLimit => "ply_limit",
    }
}

fn parse_termination(value: &str) -> Result<GameTermination, String> {
    match value {
        "checkmate" => Ok(GameTermination::Checkmate),
        "stalemate" => Ok(GameTermination::Stalemate),
        "fifty_move" => Ok(GameTermination::FiftyMoveRule),
        "threefold" => Ok(GameTermination::ThreefoldRepetition),
        "score_adjudication" => Ok(GameTermination::ScoreAdjudication),
        "ply_limit" => Ok(GameTermination::PlyLimit),
        _ => Err(format!("unknown termination `{value}`")),
    }
}

fn available_jobs() -> usize {
    std::thread::available_parallelism().map_or(1, usize::from)
}

fn print_help() {
    println!(
        "TeraStockfish strategic-v2 fixed-sample validation\n\
         \n\
         Usage: cargo run --release -p terastockfish --bin terastockfish-strategic -- [OPTIONS]\n\
         \n\
         Options:\n\
           --preset NAME          smoke|quick|overnight|deep (default: quick)\n\
           --pairs N              Fixed color-swapped pair count\n\
           --nodes N              Node limit per move\n\
           --max-plies N          Safety cap; 0 disables (any cap makes the result invalid)\n\
           --opening-plies N      Even deterministic random opening length\n\
           --jobs N               Concurrent pairs (default: logical CPU count)\n\
           --hash MB              Hash per player; memory is about 2 * jobs * MB\n\
           --seed N|0xHEX         Opening seed\n\
           --adjudication-score N Sustained winning score; 0 disables\n\
           --adjudication-plies N Required consecutive half-moves\n\
           --output PATH          Atomic CSV/checkpoint output\n\
           --resume               Continue an exactly matching output\n\
         \n\
         The sample size is fixed before play and the tool never stops early.\n\
         A valid CI entirely above 50% supports strategic-v2; an interval\n\
         crossing 50% is inconclusive. Smoke/quick are harness diagnostics,\n\
         not promotion evidence."
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn example_game(pair: u32, color: Color) -> SelfPlayGame {
        SelfPlayGame {
            pair,
            candidate_color: color,
            result: CandidateResult::Draw,
            termination: GameTermination::ThreefoldRepetition,
            plies: 42,
            nodes: 1234,
            elapsed: Duration::from_millis(56),
            opening_fen: "16/16/16/16/16/16/16/16/16/16/16/16/16/16/16/16 w - - 0 1".to_owned(),
        }
    }

    #[test]
    fn output_round_trips_complete_pairs() {
        let options = Options {
            pairs: 1,
            ..Options::default()
        };
        let header = header(
            &options,
            EvaluationParameters::empirical_v1(),
            EvaluationParameters::strategic_v2(),
        );
        let mut games = vec![example_game(0, Color::Black), example_game(0, Color::White)];
        normalize_and_validate_games(&mut games, 1).unwrap();
        let decoded = decode_games(&encode(&header, &games)).unwrap();
        assert_eq!(decoded.len(), 2);
        assert_eq!(decoded[0].candidate_color, Color::White);
        assert_eq!(decoded[1].candidate_color, Color::Black);
        assert_eq!(decoded[0].opening_fen, decoded[1].opening_fen);
    }

    #[test]
    fn preset_is_applied_before_explicit_overrides() {
        let options = parse_options_from(vec![
            "--preset".to_owned(),
            "overnight".to_owned(),
            "--pairs".to_owned(),
            "7".to_owned(),
            "--nodes".to_owned(),
            "123".to_owned(),
        ])
        .unwrap();
        assert_eq!(options.pairs, 7);
        assert_eq!(options.config.nodes_per_move, 123);
        assert_eq!(options.config.maximum_plies, 0);
        assert_eq!(options.config.opening_plies, 8);
        assert_eq!(options.config.adjudication_score, 1_500);
        assert_eq!(options.config.adjudication_plies, 20);
    }

    #[test]
    fn profile_fingerprint_covers_strategic_weights() {
        assert_ne!(
            EvaluationParameters::empirical_v1().research_fingerprint(),
            EvaluationParameters::strategic_v2().research_fingerprint()
        );
    }
}
