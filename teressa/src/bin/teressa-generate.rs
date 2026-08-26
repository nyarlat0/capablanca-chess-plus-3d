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

    let mut preset = None;
    let mut index = 0;
    while index < arguments.len() {
        match arguments[index].as_str() {
            "--preset" => {
                if preset.is_some() {
                    return Err("--preset may be supplied only once".to_owned());
                }
                preset = Some(
                    arguments
                        .get(index + 1)
                        .ok_or_else(|| "missing value after --preset".to_owned())?
                        .clone(),
                );
                index += 2;
            }
            "--adopt-existing" => index += 1,
            _ => index += 2,
        }
    }
    if let Some(preset) = preset {
        apply_preset(&mut options, &preset)?;
    }

    index = 0;
    while index < arguments.len() {
        let option = &arguments[index];
        if option == "--adopt-existing" {
            options.adopt_existing = true;
            index += 1;
            continue;
        }
        let value = arguments
            .get(index + 1)
            .ok_or_else(|| format!("missing value after {option}"))?;
        match option.as_str() {
            "--preset" => {}
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
                let hours = parse::<f64>(value, option)?;
                if !hours.is_finite() || hours <= 0.0 {
                    return Err("--hours must be finite and positive".to_owned());
                }
                options.time_limit = Some(Duration::from_secs_f64(hours * 3600.0));
            }
            _ => return Err(format!("unknown option {option}; use --help")),
        }
        index += 2;
    }
    println!(
        "# generator games={} shards={} games_per_shard={} jobs={} nodes={} node_limit=strict hash_mb_per_job={} hash_mb_total={} opening_plies={} max_plies={} temperature_cp={} seed={} output={} manifest=required",
        options.games,
        options.games.div_ceil(options.games_per_shard),
        options.games_per_shard,
        options.jobs,
        options.nodes_per_move,
        options.hash_megabytes_per_job,
        options.hash_megabytes_per_job.saturating_mul(options.jobs),
        options.opening_plies,
        options.maximum_plies,
        options.temperature_cp,
        options.seed,
        options.output_directory.display()
    );
    let summary = generate_dataset(&options)?;
    println!(
        "finished: shards_written={} shards_reused={} games_written={} games_reused={} games_completed={} capped_games_written={} positions_written={} search_nodes_written={} nodes_per_position={:.0} teacher_knps_per_worker={:.1}",
        summary.shards_written,
        summary.shards_reused,
        summary.games_written,
        summary.games_reused,
        summary.games_completed(),
        summary.capped_games_written,
        summary.records_written,
        summary.search_nodes_written,
        summary.nodes_per_position(),
        summary.teacher_nodes_per_second() / 1_000.0,
    );
    Ok(())
}

fn apply_preset(options: &mut GenerationOptions, preset: &str) -> Result<(), String> {
    match preset {
        "massive-10k" => {
            options.output_directory = PathBuf::from("datasets/teressa-10k-strict");
            options.games = 4_096;
            options.games_per_shard = 4;
            options.jobs = 24;
            options.hash_megabytes_per_job = 32;
            options.nodes_per_move = 10_000;
            options.opening_plies = 16;
            options.maximum_plies = 600;
            options.temperature_cp = 120.0;
            options.seed = 0x5445_5245_3130_4b31;
            options.time_limit = None;
            options.adopt_existing = false;
            Ok(())
        }
        _ => Err(format!("unknown preset `{preset}`; expected massive-10k")),
    }
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
  --preset massive-10k    4096-game strict-10K corpus for 12C/24T CPUs\n\
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
  --hours H               stop assigning new shards after H hours\n\
  --adopt-existing        attach a manifest to legacy shards; use only with\n\
                          the exact settings that created them"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn massive_preset_has_expected_resource_limits() {
        let mut options = GenerationOptions::default();
        apply_preset(&mut options, "massive-10k").unwrap();
        assert_eq!(options.games, 4_096);
        assert_eq!(options.nodes_per_move, 10_000);
        assert_eq!(options.jobs, 24);
        assert_eq!(options.hash_megabytes_per_job, 32);
        assert_eq!(options.games_per_shard, 4);
        assert_eq!(
            options.output_directory,
            PathBuf::from("datasets/teressa-10k-strict")
        );
        assert!(options.time_limit.is_none());
    }
}
