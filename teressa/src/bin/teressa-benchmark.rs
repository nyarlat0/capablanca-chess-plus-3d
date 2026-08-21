use burn::module::AutodiffModule;
use burn::optim::{AdamWConfig, GradientsParams, Optimizer};
use burn::prelude::*;
use burn::tensor::backend::AutodiffBackend;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use teressa::encode::{BOARD_FEATURES, BOARD_TOKENS, HISTORY_PLIES, MOVE_FEATURES};
use teressa::training::{TrainingBatch, training_loss};
use teressa::{Architecture, BdhConfig, NetworkInput, PlanKind, ResidualConfig, StrategyNetwork};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BenchmarkMode {
    Inference,
    Training,
}

impl BenchmarkMode {
    fn parse(value: &str) -> Result<Self, String> {
        match value.to_ascii_lowercase().as_str() {
            "inference" | "infer" => Ok(Self::Inference),
            "training" | "train" => Ok(Self::Training),
            _ => Err(format!(
                "unknown benchmark mode `{value}`; expected inference or training"
            )),
        }
    }
}

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
            "Usage: teressa-benchmark [--mode inference|training] [--architecture bdh|residual] [--batch-size N] [--moves N] [--iterations N] [--validation-iterations N] [--width N] [--steps N] [--heads N] [--sparse-per-head N]\n\nTraining mode intentionally creates and uploads a fresh complete batch every iteration and then runs validation on the non-autodiff backend, matching the production GPU buffer lifecycle."
        );
        return Ok(());
    }
    let mut architecture = Architecture::Bdh;
    let mut mode = BenchmarkMode::Inference;
    let mut batch = 16_usize;
    let mut moves = 128_usize;
    let mut iterations = 20_usize;
    let mut validation_iterations = 20_usize;
    let mut width = 192_usize;
    let mut steps = 4_usize;
    let mut heads = 4_usize;
    let mut sparse_per_head = 48_usize;
    let mut index = 0;
    while index < arguments.len() {
        let option = &arguments[index];
        let value = arguments
            .get(index + 1)
            .ok_or_else(|| format!("missing value after {option}"))?;
        match option.as_str() {
            "--mode" => mode = BenchmarkMode::parse(value)?,
            "--architecture" => architecture = Architecture::parse(value)?,
            "--batch-size" => batch = parse(value, option)?,
            "--moves" => moves = parse(value, option)?,
            "--iterations" => iterations = parse(value, option)?,
            "--validation-iterations" => validation_iterations = parse(value, option)?,
            "--width" => width = parse(value, option)?,
            "--steps" => steps = parse(value, option)?,
            "--heads" => heads = parse(value, option)?,
            "--sparse-per-head" => sparse_per_head = parse(value, option)?,
            _ => return Err(format!("unknown option {option}")),
        }
        index += 2;
    }
    if batch == 0
        || moves == 0
        || iterations == 0
        || width == 0
        || steps == 0
        || heads == 0
        || sparse_per_head == 0
    {
        return Err(
            "batch-size, moves, iterations, width, steps, heads, and sparse-per-head must be positive"
                .to_owned(),
        );
    }
    let device = teressa::training::initialize_vulkan();
    match mode {
        BenchmarkMode::Inference => benchmark_inference_architecture(
            architecture,
            &device,
            batch,
            moves,
            iterations,
            width,
            steps,
            heads,
            sparse_per_head,
        ),
        BenchmarkMode::Training => benchmark_training_architecture(
            architecture,
            &device,
            batch,
            moves,
            iterations,
            validation_iterations,
            width,
            steps,
            heads,
            sparse_per_head,
        ),
    }
}

#[allow(clippy::too_many_arguments)]
fn benchmark_inference_architecture(
    architecture: Architecture,
    device: &burn::backend::wgpu::WgpuDevice,
    batch: usize,
    moves: usize,
    iterations: usize,
    width: usize,
    steps: usize,
    heads: usize,
    sparse_per_head: usize,
) -> Result<(), String> {
    type B = burn::backend::Wgpu;
    match architecture {
        Architecture::Bdh => benchmark_inference(
            BdhConfig::new()
                .with_width(width)
                .with_heads(heads)
                .with_sparse_per_head(sparse_per_head)
                .with_reasoning_steps(steps)
                .init::<B>(device),
            device,
            batch,
            moves,
            iterations,
        ),
        Architecture::Residual => benchmark_inference(
            ResidualConfig::new()
                .with_width(width)
                .with_hidden(width * 2)
                .with_layers(steps)
                .init::<B>(device),
            device,
            batch,
            moves,
            iterations,
        ),
    }
}

fn benchmark_inference<M: StrategyNetwork<burn::backend::Wgpu>>(
    model: M,
    device: &burn::backend::wgpu::WgpuDevice,
    batch: usize,
    moves: usize,
    iterations: usize,
) -> Result<(), String> {
    let board = Tensor::zeros([batch, BOARD_TOKENS, BOARD_FEATURES], device);
    let mask = Tensor::ones([batch, BOARD_TOKENS], device);
    let history = Tensor::zeros([batch, HISTORY_PLIES, MOVE_FEATURES], device);
    let history_mask = Tensor::zeros([batch, HISTORY_PLIES], device);
    let legal = Tensor::zeros([batch, moves, MOVE_FEATURES], device);
    let move_from = Tensor::zeros([batch, moves], device);
    let move_to = Tensor::zeros([batch, moves], device);
    let plan = Tensor::zeros([batch, PlanKind::COUNT], device);
    let input = || NetworkInput {
        board: board.clone(),
        board_mask: mask.clone(),
        history: history.clone(),
        history_mask: history_mask.clone(),
        moves: legal.clone(),
        move_from: move_from.clone(),
        move_to: move_to.clone(),
        plan: plan.clone(),
    };
    // Warm up pipeline compilation before measuring steady-state inference.
    model.predict(input()).wdl_logits.into_data();
    let started = Instant::now();
    for _ in 0..iterations {
        model.predict(input()).wdl_logits.into_data();
    }
    let elapsed = started.elapsed();
    let positions = batch * iterations;
    println!(
        "mode=inference batch={} moves={} iterations={} elapsed_ms={:.2} positions_per_second={:.1} ms_per_batch={:.3}",
        batch,
        moves,
        iterations,
        elapsed.as_secs_f64() * 1000.0,
        positions as f64 / elapsed.as_secs_f64(),
        elapsed.as_secs_f64() * 1000.0 / iterations as f64
    );
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn benchmark_training_architecture(
    architecture: Architecture,
    device: &burn::backend::wgpu::WgpuDevice,
    batch: usize,
    moves: usize,
    iterations: usize,
    validation_iterations: usize,
    width: usize,
    steps: usize,
    heads: usize,
    sparse_per_head: usize,
) -> Result<(), String> {
    type B = burn::backend::Autodiff<burn::backend::Wgpu>;
    match architecture {
        Architecture::Bdh => benchmark_training::<B, _>(
            BdhConfig::new()
                .with_width(width)
                .with_heads(heads)
                .with_sparse_per_head(sparse_per_head)
                .with_reasoning_steps(steps)
                .init::<B>(device),
            device,
            batch,
            moves,
            iterations,
            validation_iterations,
        ),
        Architecture::Residual => benchmark_training::<B, _>(
            ResidualConfig::new()
                .with_width(width)
                .with_hidden(width * 2)
                .with_layers(steps)
                .init::<B>(device),
            device,
            batch,
            moves,
            iterations,
            validation_iterations,
        ),
    }
}

fn benchmark_training<B, M>(
    mut model: M,
    device: &B::Device,
    batch: usize,
    moves: usize,
    iterations: usize,
    validation_iterations: usize,
) -> Result<(), String>
where
    B: AutodiffBackend,
    M: StrategyNetwork<B> + AutodiffModule<B>,
    M::InnerModule: StrategyNetwork<B::InnerBackend>,
{
    let mut optimizer = AdamWConfig::new().init::<B, M>();
    let sampler = VramSampler::start();
    let started = Instant::now();
    for iteration in 0..iterations {
        // The production trainer builds every tensor from fresh host vectors.
        // Reusing GPU tensors here hid accumulation in upload/readback buffers.
        let training_batch = synthetic_training_batch::<B>(batch, moves, device);
        let output = model.predict(training_batch.network_input());
        let loss = training_loss(output, &training_batch);
        let gradients = GradientsParams::from_grads(loss.backward(), &model);
        model = optimizer.step(3.0e-4, model, gradients);
        drop(training_batch);
        if (iteration + 1) % 64 == 0 {
            maintain_gpu::<B>(device, "training benchmark")?;
        }
    }
    maintain_gpu::<B>(device, "before validation benchmark")?;
    let validation_model = model.valid();
    benchmark_validation::<B::InnerBackend, _>(
        &validation_model,
        device,
        batch,
        moves,
        validation_iterations,
    )?;
    drop(validation_model);
    maintain_gpu::<B>(device, "after validation benchmark")?;
    let elapsed = started.elapsed();
    let total_iterations = iterations + validation_iterations;
    let positions = batch * total_iterations;
    let memory = sampler.map(VramSampler::finish);
    println!(
        "mode=training batch={} moves={} iterations={} validation_iterations={} elapsed_ms={:.2} positions_per_second={:.1} ms_per_batch={:.3}{}",
        batch,
        moves,
        iterations,
        validation_iterations,
        elapsed.as_secs_f64() * 1000.0,
        positions as f64 / elapsed.as_secs_f64(),
        elapsed.as_secs_f64() * 1000.0 / total_iterations.max(1) as f64,
        memory.map_or_else(String::new, VramSample::description),
    );
    Ok(())
}

fn benchmark_validation<B: Backend, M: StrategyNetwork<B>>(
    model: &M,
    device: &B::Device,
    batch: usize,
    moves: usize,
    iterations: usize,
) -> Result<(), String> {
    for iteration in 0..iterations {
        let validation_batch = synthetic_training_batch::<B>(batch, moves, device);
        let output = model.predict(validation_batch.network_input());
        let loss = training_loss(output.clone(), &validation_batch);
        let policy_correct = output
            .policy_logits
            .argmax(1)
            .equal(validation_batch.policy_target.clone().argmax(1))
            .float()
            .sum();
        let wdl_correct = output
            .wdl_logits
            .argmax(1)
            .equal(validation_batch.wdl_target.clone().argmax(1))
            .float()
            .sum();
        let plan_correct = output
            .plan_logits
            .argmax(1)
            .equal(validation_batch.plan_target.clone().argmax(1))
            .float()
            .sum();
        Tensor::cat(vec![loss, policy_correct, wdl_correct, plan_correct], 0).into_data();
        drop(validation_batch);
        if (iteration + 1) % 64 == 0 {
            maintain_gpu::<B>(device, "validation benchmark")?;
        }
    }
    Ok(())
}

fn maintain_gpu<B: Backend>(device: &B::Device, stage: &str) -> Result<(), String> {
    B::sync(device)
        .map_err(|error| format!("GPU synchronization failed during {stage}: {error}"))?;
    B::memory_cleanup(device);
    Ok(())
}

fn synthetic_training_batch<B: Backend>(
    batch: usize,
    moves: usize,
    device: &B::Device,
) -> TrainingBatch<B> {
    let mut plan_input = vec![0.0; batch * PlanKind::COUNT];
    let mut policy_target = vec![0.0; batch * moves];
    let mut wdl_target = vec![0.0; batch * 3];
    let mut plan_target = vec![0.0; batch * PlanKind::COUNT];
    for sample in 0..batch {
        plan_input[sample * PlanKind::COUNT] = 1.0;
        policy_target[sample * moves] = 1.0;
        wdl_target[sample * 3 + 1] = 1.0;
        plan_target[sample * PlanKind::COUNT] = 1.0;
    }
    TrainingBatch {
        board: Tensor::from_data(
            TensorData::new(
                vec![0.0; batch * BOARD_TOKENS * BOARD_FEATURES],
                [batch, BOARD_TOKENS, BOARD_FEATURES],
            ),
            device,
        ),
        board_mask: Tensor::from_data(
            TensorData::new(vec![1.0; batch * BOARD_TOKENS], [batch, BOARD_TOKENS]),
            device,
        ),
        history: Tensor::from_data(
            TensorData::new(
                vec![0.0; batch * HISTORY_PLIES * MOVE_FEATURES],
                [batch, HISTORY_PLIES, MOVE_FEATURES],
            ),
            device,
        ),
        history_mask: Tensor::from_data(
            TensorData::new(vec![0.0; batch * HISTORY_PLIES], [batch, HISTORY_PLIES]),
            device,
        ),
        moves: Tensor::from_data(
            TensorData::new(
                vec![0.0; batch * moves * MOVE_FEATURES],
                [batch, moves, MOVE_FEATURES],
            ),
            device,
        ),
        move_from: Tensor::from_data(
            TensorData::new(vec![0_i64; batch * moves], [batch, moves]),
            device,
        ),
        move_to: Tensor::from_data(
            TensorData::new(vec![0_i64; batch * moves], [batch, moves]),
            device,
        ),
        move_mask: Tensor::from_data(
            TensorData::new(vec![1.0; batch * moves], [batch, moves]),
            device,
        ),
        teacher_score: Tensor::from_data(
            TensorData::new(vec![0.0; batch * moves], [batch, moves]),
            device,
        ),
        teacher_score_mask: Tensor::from_data(
            TensorData::new(vec![1.0; batch * moves], [batch, moves]),
            device,
        ),
        plan_input: Tensor::from_data(
            TensorData::new(plan_input, [batch, PlanKind::COUNT]),
            device,
        ),
        plan_mask: Tensor::from_data(
            TensorData::new(vec![1.0; batch * PlanKind::COUNT], [batch, PlanKind::COUNT]),
            device,
        ),
        policy_target: Tensor::from_data(TensorData::new(policy_target, [batch, moves]), device),
        wdl_target: Tensor::from_data(TensorData::new(wdl_target, [batch, 3]), device),
        plan_target: Tensor::from_data(
            TensorData::new(plan_target, [batch, PlanKind::COUNT]),
            device,
        ),
        score_target: Tensor::from_data(TensorData::new(vec![0.0; batch], [batch, 1]), device),
        risk_target: Tensor::from_data(
            TensorData::new(vec![0.0; batch * moves], [batch, moves]),
            device,
        ),
    }
}

struct VramSampler {
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
    baseline: u64,
    peak: Arc<AtomicU64>,
    total: u64,
}

struct VramSample {
    baseline: u64,
    peak: u64,
    total: u64,
}

impl VramSampler {
    fn start() -> Option<Self> {
        let (used_path, total) = amdgpu_vram_paths()?;
        let baseline = read_u64(&used_path)?;
        let peak = Arc::new(AtomicU64::new(baseline));
        let stop = Arc::new(AtomicBool::new(false));
        let worker_peak = Arc::clone(&peak);
        let worker_stop = Arc::clone(&stop);
        let worker = std::thread::spawn(move || {
            while !worker_stop.load(Ordering::Relaxed) {
                if let Some(used) = read_u64(&used_path) {
                    worker_peak.fetch_max(used, Ordering::Relaxed);
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            if let Some(used) = read_u64(&used_path) {
                worker_peak.fetch_max(used, Ordering::Relaxed);
            }
        });
        Some(Self {
            stop,
            worker: Some(worker),
            baseline,
            peak,
            total,
        })
    }

    fn finish(mut self) -> VramSample {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        VramSample {
            baseline: self.baseline,
            peak: self.peak.load(Ordering::Relaxed),
            total: self.total,
        }
    }
}

impl VramSample {
    fn description(self) -> String {
        format!(
            " vram_baseline_mib={:.1} vram_peak_mib={:.1} vram_delta_mib={:.1} vram_total_mib={:.1}",
            mib(self.baseline),
            mib(self.peak),
            mib(self.peak.saturating_sub(self.baseline)),
            mib(self.total),
        )
    }
}

fn amdgpu_vram_paths() -> Option<(PathBuf, u64)> {
    let mut best = None;
    for entry in fs::read_dir("/sys/class/drm").ok()?.flatten() {
        let device = entry.path().join("device");
        let used = device.join("mem_info_vram_used");
        let total_path = device.join("mem_info_vram_total");
        let Some(total) = read_u64(&total_path) else {
            continue;
        };
        if used.is_file() && best.as_ref().is_none_or(|(_, current)| total > *current) {
            best = Some((used, total));
        }
    }
    best
}

fn read_u64(path: &PathBuf) -> Option<u64> {
    fs::read_to_string(path).ok()?.trim().parse().ok()
}

fn mib(bytes: u64) -> f64 {
    bytes as f64 / 1_048_576.0
}

fn parse<T: std::str::FromStr>(value: &str, option: &str) -> Result<T, String> {
    value
        .parse()
        .map_err(|_| format!("invalid value `{value}` for {option}"))
}
