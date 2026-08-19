use std::path::PathBuf;
use std::time::Duration;
use teressa::{GenerationOptions, generate_dataset};

fn main() {
    if let Err(error) = run() {
        eprintln!("teressa-generate: {error}");
        std::process::exit(2);
    }
}

fn run() -> Result<(), String> {
    let mut options = GenerationOptions::default();
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    if arguments
        .iter()
        .any(|value| value == "-h" || value == "--help")
    {
        print_help();
        return Ok(());
    }
    let mut index = 0;
    while index < arguments.len() {
        let option = &arguments[index];
        let value = arguments
            .get(index + 1)
            .ok_or_else(|| format!("missing value after {option}"))?;
        match option.as_str() {
            "--output" => options.output_directory = PathBuf::from(value),
            "--games" => options.games = parse(value, option)?,
            "--games-per-shard" => options.games_per_shard = parse(value, option)?,
            "--jobs" => options.jobs = parse(value, option)?,
            "--hash" => options.hash_megabytes_per_job = parse(value, option)?,
            "--nodes" => options.nodes_per_move = parse(value, option)?,
            "--opening-plies" => options.opening_plies = parse(value, option)?,
            "--max-plies" => options.maximum_plies = parse(value, option)?,
            "--temperature" => options.temperature_cp = parse(value, option)?,
            "--seed" => options.seed = parse_u64(value, option)?,
            "--hours" => {
                options.time_limit = Some(Duration::from_secs_f64(
                    parse::<f64>(value, option)? * 3600.0,
                ))
            }
            _ => return Err(format!("unknown option {option}; use --help")),
        }
        index += 2;
    }
    println!(
        "# generator games={} jobs={} nodes={} hash_mb_per_job={} output={}",
        options.games,
        options.jobs,
        options.nodes_per_move,
        options.hash_megabytes_per_job,
        options.output_directory.display()
    );
    let summary = generate_dataset(&options)?;
    println!(
        "finished: shards_written={} shards_reused={} games_written={} positions={}",
        summary.shards_written,
        summary.shards_reused,
        summary.games_written,
        summary.records_written
    );
    Ok(())
}

fn parse<T: std::str::FromStr>(value: &str, option: &str) -> Result<T, String> {
    value
        .parse()
        .map_err(|_| format!("invalid value `{value}` for {option}"))
}

fn parse_u64(value: &str, option: &str) -> Result<u64, String> {
    let parsed = value.strip_prefix("0x").unwrap_or(value);
    if value.starts_with("0x") {
        u64::from_str_radix(parsed, 16)
            .map_err(|_| format!("invalid hexadecimal value `{value}` for {option}"))
    } else {
        parse(value, option)
    }
}

fn print_help() {
    println!(
        "Teressa teacher/self-play dataset generator\n\
Usage: teressa-generate [OPTIONS]\n\
  --output DIR             shard directory [target/teressa-data]\n\
  --games N               maximum games [256]\n\
  --games-per-shard N     atomic resume unit [8]\n\
  --jobs N                parallel games [1]\n\
  --hash MB               TeraStockfish hash per job [32]\n\
  --nodes N               teacher nodes per move [20000]\n\
  --opening-plies N       sampled opening length [10]\n\
  --max-plies N           storage safeguard [600]\n\
  --temperature CP        root-policy temperature [120]\n\
  --seed N|0xHEX          deterministic master seed\n\
  --hours H               stop assigning new shards after H hours"
    );
}
