use burn::prelude::*;
use std::time::Instant;
use teressa::encode::{BOARD_FEATURES, BOARD_TOKENS, MOVE_FEATURES};
use teressa::{Architecture, BdhConfig, PlanKind, ResidualConfig, StrategyNetwork};

fn main() {
    if let Err(error) = run() {
        eprintln!("teressa-benchmark: {error}");
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
            "Usage: teressa-benchmark [--architecture bdh|residual] [--batch-size N] [--moves N] [--iterations N] [--width N] [--steps N]"
        );
        return Ok(());
    }
    let mut architecture = Architecture::Bdh;
    let mut batch = 16_usize;
    let mut moves = 128_usize;
    let mut iterations = 20_usize;
    let mut width = 192_usize;
    let mut steps = 4_usize;
    let mut index = 0;
    while index < arguments.len() {
        let option = &arguments[index];
        let value = arguments
            .get(index + 1)
            .ok_or_else(|| format!("missing value after {option}"))?;
        match option.as_str() {
            "--architecture" => architecture = Architecture::parse(value)?,
            "--batch-size" => batch = parse(value, option)?,
            "--moves" => moves = parse(value, option)?,
            "--iterations" => iterations = parse(value, option)?,
            "--width" => width = parse(value, option)?,
            "--steps" => steps = parse(value, option)?,
            _ => return Err(format!("unknown option {option}")),
        }
        index += 2;
    }
    if batch == 0 || moves == 0 || iterations == 0 {
        return Err("batch-size, moves, and iterations must be positive".to_owned());
    }
    let device = teressa::training::initialize_vulkan();
    type B = burn::backend::Wgpu;
    match architecture {
        Architecture::Bdh => benchmark(
            BdhConfig::new()
                .with_width(width)
                .with_reasoning_steps(steps)
                .init::<B>(&device),
            &device,
            batch,
            moves,
            iterations,
        ),
        Architecture::Residual => benchmark(
            ResidualConfig::new()
                .with_width(width)
                .with_hidden(width * 2)
                .with_layers(steps)
                .init::<B>(&device),
            &device,
            batch,
            moves,
            iterations,
        ),
    }
}

fn benchmark<M: StrategyNetwork<burn::backend::Wgpu>>(
    model: M,
    device: &burn::backend::wgpu::WgpuDevice,
    batch: usize,
    moves: usize,
    iterations: usize,
) -> Result<(), String> {
    let board = Tensor::zeros([batch, BOARD_TOKENS, BOARD_FEATURES], device);
    let mask = Tensor::ones([batch, BOARD_TOKENS], device);
    let legal = Tensor::zeros([batch, moves, MOVE_FEATURES], device);
    let plan = Tensor::zeros([batch, PlanKind::COUNT], device);
    // Warm up pipeline compilation before measuring steady-state inference.
    model
        .predict(board.clone(), mask.clone(), legal.clone(), plan.clone())
        .wdl_logits
        .into_data();
    let started = Instant::now();
    for _ in 0..iterations {
        model
            .predict(board.clone(), mask.clone(), legal.clone(), plan.clone())
            .wdl_logits
            .into_data();
    }
    let elapsed = started.elapsed();
    let positions = batch * iterations;
    println!(
        "batch={} moves={} iterations={} elapsed_ms={:.2} positions_per_second={:.1} ms_per_batch={:.3}",
        batch,
        moves,
        iterations,
        elapsed.as_secs_f64() * 1000.0,
        positions as f64 / elapsed.as_secs_f64(),
        elapsed.as_secs_f64() * 1000.0 / iterations as f64
    );
    Ok(())
}

fn parse<T: std::str::FromStr>(value: &str, option: &str) -> Result<T, String> {
    value
        .parse()
        .map_err(|_| format!("invalid value `{value}` for {option}"))
}
