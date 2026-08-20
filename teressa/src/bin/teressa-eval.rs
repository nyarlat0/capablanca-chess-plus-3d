use burn::module::Module;
use burn::record::CompactRecorder;
use std::path::PathBuf;
use teressa::{
    Architecture, BdhConfig, ModelManifest, ResidualConfig, evaluate_model, load_records,
    split_records,
};

fn main() {
    if let Err(error) = run() {
        eprintln!("teressa-eval: {error}");
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
            "Usage: teressa-eval --model PREFIX --input PATH [--input PATH] [--batch-size N] [--seed N]"
        );
        return Ok(());
    }
    let mut model = PathBuf::from("target/teressa/teressa-bdh");
    let mut inputs = Vec::new();
    let mut batch_size = 32_usize;
    let mut seed = 0x5445_5245_5353_4132_u64;
    let mut index = 0;
    while index < arguments.len() {
        let option = &arguments[index];
        let value = arguments
            .get(index + 1)
            .ok_or_else(|| format!("missing value after {option}"))?;
        match option.as_str() {
            "--model" => model = PathBuf::from(value),
            "--input" => inputs.push(PathBuf::from(value)),
            "--batch-size" => {
                batch_size = value.parse().map_err(|_| "invalid batch size".to_owned())?
            }
            "--seed" => seed = parse_u64(value)?,
            _ => return Err(format!("unknown option {option}")),
        }
        index += 2;
    }
    if inputs.is_empty() {
        inputs.push(PathBuf::from("target/teressa-data"));
    }
    let manifest = ModelManifest::load(&model)?;
    let (_, validation, test) = split_records(load_records(&inputs)?, seed);
    let records = if test.is_empty() { &validation } else { &test };
    if records.is_empty() {
        return Err("selected holdout contains no positions".to_owned());
    }
    let device = teressa::training::initialize_vulkan();
    type B = burn::backend::Wgpu;
    let metrics = match manifest.architecture {
        Architecture::Bdh => {
            let network = BdhConfig::new()
                .with_width(manifest.width)
                .with_heads(manifest.heads)
                .with_sparse_per_head(manifest.sparse_per_head)
                .with_reasoning_steps(manifest.layers_or_steps)
                .init::<B>(&device)
                .load_file(&model, &CompactRecorder::new(), &device)
                .map_err(|error| format!("cannot load model: {error}"))?;
            evaluate_model(&network, records, batch_size, &device)?
        }
        Architecture::Residual => {
            let network = ResidualConfig::new()
                .with_width(manifest.width)
                .with_layers(manifest.layers_or_steps)
                .with_hidden(manifest.width * 2)
                .init::<B>(&device)
                .load_file(&model, &CompactRecorder::new(), &device)
                .map_err(|error| format!("cannot load model: {error}"))?;
            evaluate_model(&network, records, batch_size, &device)?
        }
    };
    let policy_available = metrics.policy_uniform_cross_entropy - metrics.policy_target_entropy;
    let policy_gain = metrics.policy_uniform_cross_entropy - metrics.policy_cross_entropy;
    let plan_available = metrics.plan_prior_cross_entropy - metrics.plan_target_entropy;
    let plan_gain = metrics.plan_prior_cross_entropy - metrics.plan_cross_entropy;
    println!(
        "holdout_positions={} loss={:.6} policy_ce={:.6} policy_entropy={:.6} policy_kl={:.6} policy_uniform_ce={:.6} policy_gain_vs_uniform={:.6} policy_explained_fraction={:.4} policy_top1={:.4} wdl_accuracy={:.4} wdl_majority={:.4} wdl_balanced={:.4} plan_ce={:.6} plan_entropy={:.6} plan_kl={:.6} plan_prior_ce={:.6} plan_gain_vs_prior={:.6} plan_explained_fraction={:.4} plan_accuracy={:.4} plan_macro_recall={:.4}",
        metrics.samples,
        metrics.loss,
        metrics.policy_cross_entropy,
        metrics.policy_target_entropy,
        metrics.policy_cross_entropy - metrics.policy_target_entropy,
        metrics.policy_uniform_cross_entropy,
        policy_gain,
        safe_fraction(policy_gain, policy_available),
        metrics.policy_top1,
        metrics.wdl_accuracy,
        metrics.wdl_majority_accuracy,
        metrics.wdl_balanced_accuracy,
        metrics.plan_cross_entropy,
        metrics.plan_target_entropy,
        metrics.plan_cross_entropy - metrics.plan_target_entropy,
        metrics.plan_prior_cross_entropy,
        plan_gain,
        safe_fraction(plan_gain, plan_available),
        metrics.plan_accuracy,
        metrics.plan_macro_recall,
    );
    println!("wdl,target_fraction,recall");
    for (index, outcome) in ["win", "draw", "loss"].into_iter().enumerate() {
        println!(
            "{outcome},{:.6},{:.6}",
            metrics.wdl_target_fraction[index], metrics.wdl_recall[index]
        );
    }
    println!("plan,target_mass,predicted_mass,argmax_recall");
    for kind in teressa::PlanKind::ALL {
        println!(
            "{:?},{:.6},{:.6},{:.6}",
            kind,
            metrics.plan_target_mass[kind.index()],
            metrics.plan_predicted_mass[kind.index()],
            metrics.plan_recall[kind.index()]
        );
    }
    Ok(())
}

fn safe_fraction(gain: f32, available: f32) -> f32 {
    if available > 1.0e-8 {
        gain / available
    } else {
        0.0
    }
}

fn parse_u64(value: &str) -> Result<u64, String> {
    if let Some(value) = value.strip_prefix("0x") {
        u64::from_str_radix(value, 16).map_err(|_| "invalid hexadecimal seed".to_owned())
    } else {
        value.parse().map_err(|_| "invalid seed".to_owned())
    }
}
