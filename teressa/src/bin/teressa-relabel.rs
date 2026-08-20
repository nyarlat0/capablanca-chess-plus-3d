use std::path::PathBuf;
use teressa::{PlanKind, RelabelOptions, relabel_dataset};

fn main() {
    if let Err(error) = run() {
        eprintln!("teressa-relabel: {error}");
        std::process::exit(2);
    }
}

fn run() -> Result<(), String> {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    if arguments
        .iter()
        .any(|value| value == "-h" || value == "--help")
    {
        println!(
            "Relabel existing Teressa games with trajectory-derived soft strategic plans\n\
Usage: teressa-relabel [OPTIONS]\n\
  --input PATH     V1/V2 shard or directory; repeatable [target/teressa-data-v1]\n\
  --output DIR     separate V2 output directory [target/teressa-data-v2]\n\
  --jobs N         parallel shards [1]"
        );
        return Ok(());
    }
    let mut options = RelabelOptions::default();
    let mut inputs = Vec::new();
    let mut index = 0;
    while index < arguments.len() {
        let option = &arguments[index];
        let value = arguments
            .get(index + 1)
            .ok_or_else(|| format!("missing value after {option}"))?;
        match option.as_str() {
            "--input" => inputs.push(PathBuf::from(value)),
            "--output" => options.output_directory = PathBuf::from(value),
            "--jobs" => {
                options.jobs = value
                    .parse()
                    .map_err(|_| format!("invalid value `{value}` for {option}"))?
            }
            _ => return Err(format!("unknown option {option}; use --help")),
        }
        index += 2;
    }
    if !inputs.is_empty() {
        options.inputs = inputs;
    }
    println!(
        "# method=trajectory-soft-plans format=TERESSA_DATASET_V2 jobs={} output={}",
        options.jobs,
        options.output_directory.display()
    );
    let summary = relabel_dataset(&options)?;
    println!(
        "finished: shards_written={} shards_reused={} games={} positions={}",
        summary.shards_written, summary.shards_reused, summary.games, summary.records
    );
    println!("plan,selected,selected_fraction,probability_mass_fraction");
    for kind in PlanKind::ALL {
        println!(
            "{:?},{},{:.6},{:.6}",
            kind,
            summary.selected_plans[kind.index()],
            summary.selected_plans[kind.index()] as f64 / summary.records.max(1) as f64,
            summary.plan_probability_mass[kind.index()] / summary.records.max(1) as f64,
        );
    }
    Ok(())
}
