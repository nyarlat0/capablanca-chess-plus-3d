use capablanca_chess_plus::Variant;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;
use terastockfish::balance::{
    BalanceConfig, BalanceGame, BalanceSummary, BalanceTermination, WhiteResult, play_balance_game,
};
use terastockfish::{EvaluationParameters, SearchLimits, SearchOptions, Searcher};

const FORMAT_VERSION: &str = "TERASTOCKFISH_BALANCE_V3";

#[derive(Clone, Debug)]
struct Options {
    games: u32,
    jobs: usize,
    root_nodes: Vec<u64>,
    root_threads: usize,
    practical_margin: f64,
    maximum_unresolved_fraction: f64,
    output: PathBuf,
    resume: bool,
    skip_root: bool,
    config: BalanceConfig,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            games: 1_024,
            jobs: 12,
            root_nodes: vec![100_000, 1_000_000, 10_000_000],
            root_threads: 12,
            practical_margin: 0.03,
            maximum_unresolved_fraction: 0.10,
            output: PathBuf::from("target/terastockfish-balance.csv"),
            resume: false,
            skip_root: false,
            config: BalanceConfig::default(),
        }
    }
}

impl Options {
    fn apply_preset(&mut self, name: &str) -> Result<(), String> {
        match name {
            "eight-hour" => {
                self.games = 384;
                self.config.nodes_per_move = 50_000;
                self.config.maximum_plies = 0;
                self.config.opening_plies = 8;
                self.config.adjudication_score = 1_500;
                self.config.adjudication_plies = 20;
                self.root_nodes = vec![100_000, 1_000_000, 10_000_000];
                self.practical_margin = 0.03;
                self.maximum_unresolved_fraction = 0.10;
                Ok(())
            }
            _ => Err(format!("unknown preset `{name}`; expected `eight-hour`")),
        }
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("terastockfish-balance: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let options = parse_options()?;
    if !options.skip_root {
        analyze_exact_start(&options);
    }

    let header = header(&options);
    let mut games = if options.resume {
        let contents = fs::read_to_string(&options.output)
            .map_err(|error| format!("cannot resume from {}: {error}", options.output.display()))?;
        if !contents.starts_with(&header) {
            return Err("balance output metadata does not match this command".to_owned());
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
    games.sort_unstable_by_key(|game| game.index);
    for (expected, game) in games.iter().enumerate() {
        if game.index as usize != expected {
            return Err("balance output contains missing or duplicate game indices".to_owned());
        }
    }
    if games.len() > options.games as usize {
        return Err("balance output contains more games than requested".to_owned());
    }

    print_configuration(&options, games.len());
    while games.len() < options.games as usize {
        let start = games.len() as u32;
        let end = (start + options.jobs.max(1).min(u32::MAX as usize) as u32).min(options.games);
        let mut batch = std::thread::scope(|scope| {
            let workers = (start..end)
                .map(|index| scope.spawn(move || play_balance_game(index, options.config)))
                .collect::<Vec<_>>();
            workers
                .into_iter()
                .map(|worker| {
                    worker
                        .join()
                        .map_err(|_| "balance worker panicked".to_owned())?
                        .map_err(|error| error.to_string())
                })
                .collect::<Result<Vec<_>, String>>()
        })?;
        games.append(&mut batch);
        games.sort_unstable_by_key(|game| game.index);
        write_atomic(&options.output, &encode(&header, &games))?;
        print_summary(
            "progress",
            &BalanceSummary::from_games(&games),
            options.practical_margin,
            options.maximum_unresolved_fraction,
        );
    }

    let summary = BalanceSummary::from_games(&games);
    print_summary(
        "final",
        &summary,
        options.practical_margin,
        options.maximum_unresolved_fraction,
    );
    print_terminations(&games);
    println!("output={}", options.output.display());
    Ok(())
}

fn analyze_exact_start(options: &Options) {
    let position = Variant::TerachessII.starting_position();
    println!("# exact_start profile=production side=white");
    println!("root_nodes,score,depth,visited_nodes,elapsed_ms,best_move,pv");
    for &nodes in &options.root_nodes {
        let hash = options
            .config
            .hash_megabytes
            .saturating_mul(options.root_threads.max(1));
        let mut searcher = Searcher::new(SearchOptions {
            hash_megabytes: hash.max(1),
            threads: options.root_threads.max(1),
        });
        let result = searcher.analyze(
            &position,
            SearchLimits {
                max_depth: 191,
                max_nodes: Some(nodes),
                ..SearchLimits::default()
            },
        );
        let best_move = result
            .best_move
            .map_or_else(|| "-".to_owned(), |chess_move| chess_move.to_uci());
        let pv = result
            .principal_variation
            .iter()
            .map(|chess_move| chess_move.to_uci())
            .collect::<Vec<_>>()
            .join(" ");
        println!(
            "{nodes},{},{},{},{},{best_move},{pv}",
            result.score,
            result.completed_depth,
            result.nodes,
            result.elapsed.as_millis()
        );
    }
}

fn print_configuration(options: &Options, completed: usize) {
    println!(
        "# method=same-engine-color-balance profile=production games={} completed={} nodes_per_move={} max_plies={} opening_plies={} jobs={} hash_mb={} adjudication_score={} adjudication_plies={} seed={} fixed_sample=true max_unresolved_fraction={:.4}",
        options.games,
        completed,
        options.config.nodes_per_move,
        options.config.maximum_plies,
        options.config.opening_plies,
        options.jobs,
        options.config.hash_megabytes,
        options.config.adjudication_score,
        options.config.adjudication_plies,
        options.config.seed,
        options.maximum_unresolved_fraction
    );
}

fn print_summary(
    stage: &str,
    summary: &BalanceSummary,
    practical_margin: f64,
    maximum_unresolved_fraction: f64,
) {
    let unresolved_fraction = summary.unresolved_fraction();
    let valid = unresolved_fraction <= maximum_unresolved_fraction;
    if let (Some(score), Some((lower, upper))) =
        (summary.white_score, summary.confidence_interval_95)
    {
        println!(
            "{stage}: games={} resolved={} white_wins={} draws={} white_losses={} unresolved={} unresolved_fraction={unresolved_fraction:.4} valid={} white_score={score:.4} ci95=[{lower:.4},{upper:.4}] white_edge={} balanced_within_margin={} white_edge_over_margin={} practical_margin={:.4}",
            summary.games,
            summary.games - summary.unresolved,
            summary.white_wins,
            summary.draws,
            summary.white_losses,
            summary.unresolved,
            valid,
            valid && lower > 0.5,
            valid && lower >= 0.5 - practical_margin && upper <= 0.5 + practical_margin,
            valid && lower > 0.5 + practical_margin,
            practical_margin
        );
    } else {
        println!(
            "{stage}: games={} resolved=0 white_wins=0 draws=0 white_losses=0 unresolved={} unresolved_fraction={unresolved_fraction:.4} valid=false white_score=NA ci95=NA",
            summary.games, summary.unresolved
        );
    }
}

fn print_terminations(games: &[BalanceGame]) {
    let mut counts = [0_usize; 6];
    for game in games {
        let index = match game.termination {
            BalanceTermination::Checkmate => 0,
            BalanceTermination::Stalemate => 1,
            BalanceTermination::FiftyMoveRule => 2,
            BalanceTermination::ThreefoldRepetition => 3,
            BalanceTermination::ScoreAdjudication => 4,
            BalanceTermination::PlyLimit => 5,
        };
        counts[index] += 1;
    }
    println!(
        "terminations: checkmate={} stalemate={} fifty_move={} threefold={} score_adjudication={} ply_limit={}",
        counts[0], counts[1], counts[2], counts[3], counts[4], counts[5]
    );
}

fn header(options: &Options) -> String {
    let profile = EvaluationParameters::production();
    format!(
        "# {FORMAT_VERSION}\n# profile=production/strategic-v2\n# evaluation_fingerprint={}\n# games={}\n# nodes_per_move={}\n# maximum_plies={}\n# opening_plies={}\n# hash_megabytes={}\n# adjudication_score={}\n# adjudication_plies={}\n# seed={}\n# practical_margin={}\n# maximum_unresolved_fraction={}\ngame,white_result,termination,plies,nodes,elapsed_ms,final_score,opening_fen,final_fen\n",
        profile.research_fingerprint(),
        options.games,
        options.config.nodes_per_move,
        options.config.maximum_plies,
        options.config.opening_plies,
        options.config.hash_megabytes,
        options.config.adjudication_score,
        options.config.adjudication_plies,
        options.config.seed,
        options.practical_margin,
        options.maximum_unresolved_fraction
    )
}

fn encode(header: &str, games: &[BalanceGame]) -> String {
    let mut output = header.to_owned();
    for game in games {
        output.push_str(&format!(
            "{},{},{},{},{},{},{},{},{}\n",
            game.index,
            result_name(game.white_result),
            termination_name(game.termination),
            game.plies,
            game.nodes,
            game.elapsed.as_millis(),
            game.final_score,
            game.opening_fen,
            game.final_fen
        ));
    }
    output
}

fn decode_games(contents: &str) -> Result<Vec<BalanceGame>, String> {
    let mut games = Vec::new();
    for line in contents.lines() {
        if line.starts_with('#') || line.starts_with("game,") || line.is_empty() {
            continue;
        }
        let fields = line.splitn(9, ',').collect::<Vec<_>>();
        if fields.len() != 9 {
            return Err(format!("invalid balance row `{line}`"));
        }
        games.push(BalanceGame {
            index: parse(fields[0], "game")?,
            white_result: parse_result(fields[1])?,
            termination: parse_termination(fields[2])?,
            plies: parse(fields[3], "plies")?,
            nodes: parse(fields[4], "nodes")?,
            elapsed: Duration::from_millis(parse(fields[5], "elapsed_ms")?),
            final_score: parse(fields[6], "final_score")?,
            opening_fen: fields[7].to_owned(),
            final_fen: fields[8].to_owned(),
        });
    }
    Ok(games)
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
            "--smoke" => {
                options.games = 2;
                options.jobs = 1;
                options.root_threads = 1;
                options.root_nodes = vec![128];
                options.config.nodes_per_move = 64;
                options.config.maximum_plies = 4;
                options.config.opening_plies = 2;
                options.config.hash_megabytes = 1;
                options.output = PathBuf::from("target/terastockfish-balance-smoke.csv");
            }
            "--preset" => {
                arguments
                    .next()
                    .ok_or_else(|| "--preset requires a value".to_owned())?;
            }
            "--games" => options.games = parse_next(&mut arguments, &argument)?,
            "--jobs" => options.jobs = parse_next(&mut arguments, &argument)?,
            "--nodes" => options.config.nodes_per_move = parse_next(&mut arguments, &argument)?,
            "--max-plies" => options.config.maximum_plies = parse_next(&mut arguments, &argument)?,
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
            "--root-nodes" => {
                let value = arguments
                    .next()
                    .ok_or_else(|| "--root-nodes requires a comma-separated list".to_owned())?;
                options.root_nodes = value
                    .split(',')
                    .map(|item| parse(item, "root node budget"))
                    .collect::<Result<Vec<_>, _>>()?;
            }
            "--root-threads" => {
                options.root_threads = parse_next(&mut arguments, &argument)?;
            }
            "--practical-margin" => {
                let percentage: f64 = parse_next(&mut arguments, &argument)?;
                options.practical_margin = percentage / 100.0;
            }
            "--max-unresolved-percent" => {
                let percentage: f64 = parse_next(&mut arguments, &argument)?;
                options.maximum_unresolved_fraction = percentage / 100.0;
            }
            "--skip-root" => options.skip_root = true,
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
    if options.games == 0
        || options.jobs == 0
        || options.root_threads == 0
        || options.config.nodes_per_move == 0
        || options.config.hash_megabytes == 0
        || options.root_nodes.contains(&0)
        || !options.practical_margin.is_finite()
        || !(0.0..0.5).contains(&options.practical_margin)
        || !options.maximum_unresolved_fraction.is_finite()
        || !(0.0..=1.0).contains(&options.maximum_unresolved_fraction)
    {
        return Err("games, jobs, node limits, threads, and hash must be positive".to_owned());
    }
    if !options.config.opening_plies.is_multiple_of(2) {
        return Err(
            "--opening-plies must be even so every sampled game starts with White to move"
                .to_owned(),
        );
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

const fn result_name(result: WhiteResult) -> &'static str {
    match result {
        WhiteResult::Win => "win",
        WhiteResult::Draw => "draw",
        WhiteResult::Loss => "loss",
        WhiteResult::Unresolved => "unresolved",
    }
}

fn parse_result(value: &str) -> Result<WhiteResult, String> {
    match value {
        "win" => Ok(WhiteResult::Win),
        "draw" => Ok(WhiteResult::Draw),
        "loss" => Ok(WhiteResult::Loss),
        "unresolved" => Ok(WhiteResult::Unresolved),
        _ => Err(format!("unknown White result `{value}`")),
    }
}

const fn termination_name(termination: BalanceTermination) -> &'static str {
    match termination {
        BalanceTermination::Checkmate => "checkmate",
        BalanceTermination::Stalemate => "stalemate",
        BalanceTermination::FiftyMoveRule => "fifty_move",
        BalanceTermination::ThreefoldRepetition => "threefold",
        BalanceTermination::ScoreAdjudication => "score_adjudication",
        BalanceTermination::PlyLimit => "ply_limit",
    }
}

fn parse_termination(value: &str) -> Result<BalanceTermination, String> {
    match value {
        "checkmate" => Ok(BalanceTermination::Checkmate),
        "stalemate" => Ok(BalanceTermination::Stalemate),
        "fifty_move" => Ok(BalanceTermination::FiftyMoveRule),
        "threefold" => Ok(BalanceTermination::ThreefoldRepetition),
        "score_adjudication" => Ok(BalanceTermination::ScoreAdjudication),
        "ply_limit" => Ok(BalanceTermination::PlyLimit),
        _ => Err(format!("unknown termination `{value}`")),
    }
}

fn print_help() {
    println!(
        "TeraStockfish Terachess II color-balance study\n\
         \n\
         Usage: cargo run --release -p terastockfish --bin terastockfish-balance -- [OPTIONS]\n\
         \n\
         Options:\n\
           --preset NAME          Study preset: eight-hour\n\
           --games N               Fixed randomized-opening sample (default: 1024)\n\
           --nodes N               Nodes per move (default: 100000)\n\
           --max-plies N           Safety cap; 0 disables, capped games unresolved (default: 0)\n\
           --opening-plies N       Even random opening length (default: 8)\n\
           --jobs N                Concurrent games (default: 12)\n\
           --hash MB               Hash per game (default: 32)\n\
           --seed N|0xHEX          Opening seed\n\
           --root-nodes CSV        Exact-start node budgets (default: 100000,1000000,10000000)\n\
           --root-threads N        Threads for exact-start analysis (default: 12)\n\
           --practical-margin PCT  Near-balance equivalence margin (default: 3.0)\n\
           --max-unresolved-percent PCT  Invalid-study threshold (default: 10.0)\n\
           --skip-root             Do not repeat exact-start analysis on resume\n\
           --adjudication-score N  Sustained winning score; 0 disables (default: 0)\n\
           --adjudication-plies N  Required consecutive half-moves (default: 0)\n\
           --output PATH           Atomic CSV/checkpoint output\n\
           --resume                Resume an exactly matching output file\n\
           --smoke                 Tiny harness check, not balance evidence"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_round_trips_completed_games() {
        let options = Options {
            games: 1,
            ..Options::default()
        };
        let header = header(&options);
        let games = vec![BalanceGame {
            index: 0,
            white_result: WhiteResult::Draw,
            termination: BalanceTermination::ThreefoldRepetition,
            plies: 12,
            nodes: 345,
            elapsed: Duration::from_millis(67),
            final_score: 12,
            opening_fen: "16/16/16/16/16/16/16/16/16/16/16/16/16/16/16/16 w - - 0 1".to_owned(),
            final_fen: "16/16/16/16/16/16/16/16/16/16/16/16/16/16/16/16 w - - 12 7".to_owned(),
        }];
        let decoded = decode_games(&encode(&header, &games)).unwrap();
        assert_eq!(decoded.len(), 1);
        assert_eq!(decoded[0].index, 0);
        assert_eq!(decoded[0].white_result, WhiteResult::Draw);
        assert_eq!(
            decoded[0].termination,
            BalanceTermination::ThreefoldRepetition
        );
        assert_eq!(decoded[0].elapsed, Duration::from_millis(67));
        assert_eq!(decoded[0].final_score, 12);
    }

    #[test]
    fn eight_hour_preset_is_fixed_and_explicit_options_win() {
        let options = parse_options_from(vec![
            "--games".to_owned(),
            "7".to_owned(),
            "--preset".to_owned(),
            "eight-hour".to_owned(),
            "--nodes".to_owned(),
            "123".to_owned(),
        ])
        .unwrap();
        assert_eq!(options.games, 7);
        assert_eq!(options.config.nodes_per_move, 123);
        assert_eq!(options.config.maximum_plies, 0);
        assert_eq!(options.config.opening_plies, 8);
        assert_eq!(options.config.adjudication_score, 1_500);
        assert_eq!(options.config.adjudication_plies, 20);
        assert_eq!(options.root_nodes, [100_000, 1_000_000, 10_000_000]);
        assert_eq!(options.practical_margin, 0.03);
    }

    #[test]
    fn preset_parameters_are_frozen_in_resume_metadata() {
        let preset =
            parse_options_from(vec!["--preset".to_owned(), "eight-hour".to_owned()]).unwrap();
        let defaults = Options::default();
        assert_ne!(header(&preset), header(&defaults));
        assert!(header(&preset).contains("# games=384\n"));
        assert!(header(&preset).contains("# nodes_per_move=50000\n"));
        assert!(header(&preset).contains("# maximum_plies=0\n"));
        assert!(header(&preset).contains("# adjudication_score=1500\n"));
        assert!(header(&preset).contains("# adjudication_plies=20\n"));
        assert!(header(&preset).contains("# TERASTOCKFISH_BALANCE_V3\n"));
        assert!(header(&preset).contains(";cp=6;pp=5;rs=6;ro=12\n"));
    }
}
