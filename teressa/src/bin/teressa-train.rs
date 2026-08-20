use std::path::PathBuf;
use teressa::{Architecture, TrainingOptions, train_vulkan};

fn main() {
    if let Err(error) = run() {
        eprintln!("teressa-train: {error}");
        std::process::exit(2);
    }
}

fn run() -> Result<(), String> {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    if arguments
        .iter()
        .any(|value| value == "-h" || value == "--help")
    {
        print_help();
        return Ok(());
    }
    let mut options = TrainingOptions::default();
    let mut explicit_inputs = Vec::new();
    let mut index = 0;
    while index < arguments.len() {
        let option = &arguments[index];
        if option == "--resume" {
            options.resume = true;
            index += 1;
            continue;
        }
        let value = arguments
            .get(index + 1)
            .ok_or_else(|| format!("missing value after {option}"))?;
        match option.as_str() {
            "--input" => explicit_inputs.push(PathBuf::from(value)),
            "--output" => options.output_prefix = PathBuf::from(value),
            "--architecture" => options.architecture = Architecture::parse(value)?,
            "--epochs" => options.epochs = parse(value, option)?,
            "--batch-size" => options.batch_size = parse(value, option)?,
            "--learning-rate" => options.learning_rate = parse(value, option)?,
            "--warmup-fraction" => options.warmup_fraction = parse(value, option)?,
            "--minimum-lr-ratio" => options.minimum_learning_rate_ratio = parse(value, option)?,
            "--plan-loss-weight" => options.plan_loss_weight = parse(value, option)?,
            "--seed" => options.seed = parse_u64(value, option)?,
            "--width" => options.width = parse(value, option)?,
            "--steps" => options.layers_or_steps = parse(value, option)?,
            "--heads" => options.heads = parse(value, option)?,
            "--sparse-per-head" => options.sparse_per_head = parse(value, option)?,
            _ => return Err(format!("unknown option {option}; use --help")),
        }
        index += 2;
    }
    if !explicit_inputs.is_empty() {
        options.inputs = explicit_inputs;
    }
    println!(
        "# architecture={:?} epochs={} batch_size={} learning_rate={} warmup_fraction={} minimum_lr_ratio={} plan_loss_weight={} width={} steps={} heads={} sparse_per_head={} resume={} output={}",
        options.architecture,
        options.epochs,
        options.batch_size,
        options.learning_rate,
        options.warmup_fraction,
        options.minimum_learning_rate_ratio,
        options.plan_loss_weight,
        options.width,
        options.layers_or_steps,
        options.heads,
        options.sparse_per_head,
        options.resume,
        options.output_prefix.display()
    );
    let manifest = train_vulkan(&options)?;
    println!(
        "checkpoint={} architecture={:?} train_positions={} validation_positions={}",
        options.output_prefix.display(),
        manifest.architecture,
        manifest.training_positions,
        manifest.validation_positions
    );
    Ok(())
}

fn parse<T: std::str::FromStr>(value: &str, option: &str) -> Result<T, String> {
    value
        .parse()
        .map_err(|_| format!("invalid value `{value}` for {option}"))
}

fn parse_u64(value: &str, option: &str) -> Result<u64, String> {
    if let Some(value) = value.strip_prefix("0x") {
        u64::from_str_radix(value, 16)
            .map_err(|_| format!("invalid hexadecimal value for {option}"))
    } else {
        parse(value, option)
    }
}

fn print_help() {
    println!(
        "Train a Teressa network on Vulkan\n\
Usage: teressa-train [OPTIONS]\n\
  --input PATH             shard or directory; repeatable\n\
  --output PREFIX          checkpoint prefix [target/teressa/teressa-bdh]\n\
  --architecture NAME     bdh or residual [bdh]\n\
  --epochs N              complete dataset passes [20]\n\
  --batch-size N          positions per optimizer step [16]\n\
  --learning-rate RATE    AdamW learning rate [0.0003]\n\
  --warmup-fraction RATE  Fraction of updates spent warming up [0.05]\n\
  --minimum-lr-ratio RATE Final cosine LR / peak LR [0.1]\n\
  --plan-loss-weight RATE Weight of the soft strategic-plan loss [1.0]\n\
  --resume                 Continue the output checkpoint, including AdamW state\n\
  --seed N|0xHEX          deterministic split/shuffle seed\n\
  --width N               trunk width [192]\n\
  --steps N               BDH recurrences or residual blocks [4]\n\
  --heads N               BDH sparse heads [4]\n\
  --sparse-per-head N     BDH sparse units per head [48]"
    );
}
