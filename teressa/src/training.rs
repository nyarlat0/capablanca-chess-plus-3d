use crate::MODEL_FORMAT_VERSION;
use crate::dataset::{DatasetRecord, DatasetShardReader};
use crate::encode::{
    BOARD_FEATURES, BOARD_TOKENS, HISTORY_PLIES, MOVE_FEATURES, encode_position_uci_history,
};
use crate::model::{BdhConfig, NetworkInput, ResidualConfig, StrategyNetwork};
use crate::plan::PlanKind;
use burn::module::AutodiffModule;
use burn::optim::{AdamWConfig, GradientsParams, Optimizer};
use burn::prelude::*;
use burn::record::{
    BinFileRecorder, CompactRecorder, DefaultRecorder, FullPrecisionSettings, Recorder,
};
use burn::tensor::Int;
use burn::tensor::activation::softmax;
use burn::tensor::backend::AutodiffBackend;
use burn::tensor::loss::cross_entropy_with_logits;
use rand::SeedableRng;
use rand::rngs::StdRng;
use rand::seq::SliceRandom;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Once;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Architecture {
    Bdh,
    Residual,
}

impl Architecture {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value.to_ascii_lowercase().as_str() {
            "bdh" => Ok(Self::Bdh),
            "residual" | "resnet" => Ok(Self::Residual),
            _ => Err(format!(
                "unknown architecture `{value}`; expected bdh or residual"
            )),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelManifest {
    pub format: String,
    pub architecture: Architecture,
    pub width: usize,
    pub layers_or_steps: usize,
    pub sparse_per_head: usize,
    pub heads: usize,
    pub trained_epochs: usize,
    /// Epoch exported to the compact play checkpoint. Older V3 manifests did
    /// not record selection metadata, so both fields remain optional.
    #[serde(default)]
    pub selected_epoch: Option<usize>,
    #[serde(default)]
    pub validation_selection_loss: Option<f32>,
    pub training_positions: usize,
    pub validation_positions: usize,
}

impl ModelManifest {
    pub fn save(&self, prefix: &Path) -> Result<(), String> {
        let path = manifest_path(prefix);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("cannot create model directory: {error}"))?;
        }
        let temporary = path.with_extension("json.part");
        fs::write(
            &temporary,
            serde_json::to_vec_pretty(self)
                .map_err(|error| format!("cannot encode model manifest: {error}"))?,
        )
        .map_err(|error| format!("cannot write model manifest: {error}"))?;
        fs::rename(temporary, path)
            .map_err(|error| format!("cannot publish model manifest: {error}"))
    }

    pub fn load(prefix: &Path) -> Result<Self, String> {
        let bytes = fs::read(manifest_path(prefix))
            .map_err(|error| format!("cannot read model manifest: {error}"))?;
        let manifest: Self = serde_json::from_slice(&bytes)
            .map_err(|error| format!("cannot decode model manifest: {error}"))?;
        if manifest.format != MODEL_FORMAT_VERSION {
            return Err(format!(
                "unsupported model format `{}`; expected {MODEL_FORMAT_VERSION}",
                manifest.format
            ));
        }
        Ok(manifest)
    }
}

#[derive(Clone, Debug)]
pub struct TrainingOptions {
    pub inputs: Vec<PathBuf>,
    pub output_prefix: PathBuf,
    pub architecture: Architecture,
    pub epochs: usize,
    pub batch_size: usize,
    pub learning_rate: f64,
    pub warmup_fraction: f64,
    pub minimum_learning_rate_ratio: f64,
    pub plan_loss_weight: f64,
    pub seed: u64,
    pub width: usize,
    pub layers_or_steps: usize,
    pub heads: usize,
    pub sparse_per_head: usize,
    pub resume: bool,
}

impl Default for TrainingOptions {
    fn default() -> Self {
        Self {
            inputs: vec![PathBuf::from("target/teressa-data")],
            output_prefix: PathBuf::from("target/teressa/teressa-bdh"),
            architecture: Architecture::Bdh,
            epochs: 20,
            batch_size: 16,
            learning_rate: 3.0e-4,
            warmup_fraction: 0.05,
            minimum_learning_rate_ratio: 0.1,
            plan_loss_weight: 1.0,
            seed: 0x5445_5245_5353_4132,
            width: 192,
            layers_or_steps: 4,
            heads: 4,
            sparse_per_head: 48,
            resume: false,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct TrainingState {
    format: String,
    architecture: Architecture,
    width: usize,
    layers_or_steps: usize,
    heads: usize,
    sparse_per_head: usize,
    seed: u64,
    completed_epochs: usize,
    optimizer_steps: usize,
    /// Two alternating slots keep the checkpoint transaction recoverable. The
    /// state file is published last and therefore always points at a complete
    /// model/optimizer pair, even if the next save is interrupted.
    checkpoint_slot: usize,
    #[serde(default)]
    best_epoch: Option<usize>,
    #[serde(default)]
    best_selection_loss: Option<f32>,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct EpochMetrics {
    pub loss: f32,
    pub policy_cross_entropy: f32,
    pub policy_target_entropy: f32,
    pub policy_uniform_cross_entropy: f32,
    pub policy_top1: f32,
    pub policy_top3: f32,
    pub policy_top8: f32,
    pub policy_target_mass_top1: f32,
    pub policy_target_mass_top3: f32,
    pub policy_target_mass_top8: f32,
    pub policy_regret_cp_top1: f32,
    pub policy_regret_cp_top3: f32,
    pub policy_regret_cp_top8: f32,
    pub wdl_cross_entropy: f32,
    pub plan_cross_entropy: f32,
    pub plan_target_entropy: f32,
    pub plan_prior_cross_entropy: f32,
    pub wdl_accuracy: f32,
    pub wdl_majority_accuracy: f32,
    pub wdl_balanced_accuracy: f32,
    pub wdl_recall: [f32; 3],
    pub wdl_target_fraction: [f32; 3],
    pub plan_accuracy: f32,
    pub plan_top2: f32,
    pub plan_top3: f32,
    pub plan_macro_recall: f32,
    pub plan_recall: [f32; PlanKind::COUNT],
    pub plan_target_mass: [f32; PlanKind::COUNT],
    pub plan_predicted_mass: [f32; PlanKind::COUNT],
    pub score_mean_squared_error: f32,
    pub tactical_risk_mean_squared_error: f32,
    pub samples: usize,
}

impl EpochMetrics {
    /// Lower is better. Entropy constants are removed from policy and plan so
    /// checkpoint selection tracks learnable signal instead of label softness;
    /// WDL is retained at a smaller weight because it gates expressive plans.
    #[must_use]
    pub fn strategic_selection_loss(&self) -> f32 {
        (self.policy_cross_entropy - self.policy_target_entropy)
            + (self.plan_cross_entropy - self.plan_target_entropy)
            + 0.25 * self.wdl_cross_entropy
    }
}

pub struct TrainingBatch<B: Backend> {
    pub board: Tensor<B, 3>,
    pub board_mask: Tensor<B, 2>,
    pub history: Tensor<B, 3>,
    pub history_mask: Tensor<B, 2>,
    pub moves: Tensor<B, 3>,
    pub move_from: Tensor<B, 2, Int>,
    pub move_to: Tensor<B, 2, Int>,
    pub move_mask: Tensor<B, 2>,
    pub teacher_score: Tensor<B, 2>,
    pub teacher_score_mask: Tensor<B, 2>,
    pub plan_input: Tensor<B, 2>,
    pub plan_mask: Tensor<B, 2>,
    pub policy_target: Tensor<B, 2>,
    pub wdl_target: Tensor<B, 2>,
    pub plan_target: Tensor<B, 2>,
    pub score_target: Tensor<B, 2>,
    pub risk_target: Tensor<B, 2>,
}

#[derive(Clone, Copy, Debug)]
pub struct LossWeights {
    pub policy: f64,
    pub wdl: f64,
    pub plan: f64,
    pub score: f64,
    pub tactical_risk: f64,
}

impl Default for LossWeights {
    fn default() -> Self {
        Self {
            policy: 1.0,
            wdl: 0.7,
            plan: 1.0,
            score: 0.25,
            tactical_risk: 0.2,
        }
    }
}

impl<B: Backend> TrainingBatch<B> {
    pub fn network_input(&self) -> NetworkInput<B> {
        NetworkInput {
            board: self.board.clone(),
            board_mask: self.board_mask.clone(),
            history: self.history.clone(),
            history_mask: self.history_mask.clone(),
            moves: self.moves.clone(),
            move_from: self.move_from.clone(),
            move_to: self.move_to.clone(),
            plan: self.plan_input.clone(),
        }
    }
}

pub fn initialize_vulkan() -> burn::backend::wgpu::WgpuDevice {
    static INITIALIZE: Once = Once::new();
    let device = burn::backend::wgpu::WgpuDevice::DiscreteGpu(0);
    // Training continuously creates upload and intermediate buffers. Exclusive
    // pages make the explicit cleanup below predictable, while a shorter task
    // queue limits how many submitted batches can retain buffers at once. The
    // main first-epoch VRAM spike was validation accidentally using autodiff;
    // these settings provide an additional bound instead of hiding that bug.
    INITIALIZE.call_once(|| {
        burn::backend::wgpu::init_setup::<burn::backend::wgpu::graphics::Vulkan>(
            &device,
            burn::backend::wgpu::RuntimeOptions {
                tasks_max: 8,
                memory_config: burn::backend::wgpu::MemoryConfiguration::ExclusivePages,
            },
        );
    });
    device
}

pub fn train_vulkan(options: &TrainingOptions) -> Result<ModelManifest, String> {
    if options.epochs == 0
        || options.batch_size == 0
        || options.width == 0
        || options.learning_rate <= 0.0
        || options.plan_loss_weight <= 0.0
        || !(0.0..1.0).contains(&options.warmup_fraction)
        || !(0.0..=1.0).contains(&options.minimum_learning_rate_ratio)
    {
        return Err(
            "epochs, batch-size, width, learning-rate, and plan-loss-weight must be positive; warmup must be in [0,1) and minimum LR ratio in [0,1]"
                .to_owned(),
        );
    }
    let records = load_records(&options.inputs)?;
    if records.iter().any(|record| record.plan_policy.is_empty()) {
        return Err(
            "legacy dataset has no trajectory plan policy; run teressa-relabel into a separate V2 directory before training"
                .to_owned(),
        );
    }
    let (mut train, validation, _) = split_records(records, options.seed);
    if train.is_empty() || validation.is_empty() {
        return Err(
            "dataset must contain at least one training and validation position".to_owned(),
        );
    }
    let device = initialize_vulkan();
    type Base = burn::backend::Wgpu;
    type Train = burn::backend::Autodiff<Base>;
    let training_result = match options.architecture {
        Architecture::Bdh => {
            let model = BdhConfig::new()
                .with_width(options.width)
                .with_heads(options.heads)
                .with_sparse_per_head(options.sparse_per_head)
                .with_reasoning_steps(options.layers_or_steps)
                .init::<Train>(&device);
            train_model(model, &mut train, &validation, options, &device)?
        }
        Architecture::Residual => {
            let model = ResidualConfig::new()
                .with_width(options.width)
                .with_layers(options.layers_or_steps)
                .with_hidden(options.width * 2)
                .init::<Train>(&device);
            train_model(model, &mut train, &validation, options, &device)?
        }
    };
    let manifest = training_manifest(options, training_result, train.len(), validation.len());
    manifest.save(&options.output_prefix)?;
    Ok(manifest)
}

fn training_manifest(
    options: &TrainingOptions,
    result: TrainingResult,
    training_positions: usize,
    validation_positions: usize,
) -> ModelManifest {
    ModelManifest {
        format: MODEL_FORMAT_VERSION.to_owned(),
        architecture: options.architecture,
        width: options.width,
        layers_or_steps: options.layers_or_steps,
        sparse_per_head: options.sparse_per_head,
        heads: options.heads,
        trained_epochs: result.trained_epochs,
        selected_epoch: Some(result.best_epoch),
        validation_selection_loss: Some(result.best_selection_loss),
        training_positions,
        validation_positions,
    }
}

#[derive(Clone, Copy, Debug)]
struct TrainingResult {
    trained_epochs: usize,
    best_epoch: usize,
    best_selection_loss: f32,
}

fn train_model<B, M>(
    mut model: M,
    train: &mut [DatasetRecord],
    validation: &[DatasetRecord],
    options: &TrainingOptions,
    device: &B::Device,
) -> Result<TrainingResult, String>
where
    B: AutodiffBackend,
    M: StrategyNetwork<B> + AutodiffModule<B>,
    M::InnerModule: StrategyNetwork<B::InnerBackend>,
{
    let mut optimizer = AdamWConfig::new().init::<B, M>();
    let (completed_epochs, previous_steps, mut best_epoch, mut best_selection_loss) = if options
        .resume
    {
        let state = load_training_state(&options.output_prefix)?;
        validate_training_state(&state, options)?;
        model = model
            .load_file(
                training_model_prefix(&options.output_prefix, state.checkpoint_slot),
                &DefaultRecorder::new(),
                device,
            )
            .map_err(|error| format!("cannot resume training model: {error}"))?;
        let optimizer_record = BinFileRecorder::<FullPrecisionSettings>::default()
            .load(
                training_optimizer_prefix(&options.output_prefix, state.checkpoint_slot),
                device,
            )
            .map_err(|error| format!("cannot resume AdamW state: {error}"))?;
        optimizer = optimizer.load_record(optimizer_record);
        println!(
            "resume completed_epochs={} optimizer_steps={} best_epoch={} best_selection_loss={}",
            state.completed_epochs,
            state.optimizer_steps,
            state
                .best_epoch
                .map_or_else(|| "unknown".to_owned(), |value| value.to_string()),
            state
                .best_selection_loss
                .map_or_else(|| "unknown".to_owned(), |value| format!("{value:.6}")),
        );
        let compact_exists = options.output_prefix.with_extension("mpk").is_file();
        let restored_best_epoch = compact_exists.then_some(state.best_epoch).flatten();
        let restored_best_loss = restored_best_epoch
            .zip(state.best_selection_loss)
            .filter(|(_, loss)| loss.is_finite())
            .map_or(f32::INFINITY, |(_, loss)| loss);
        (
            state.completed_epochs,
            state.optimizer_steps,
            restored_best_epoch,
            restored_best_loss,
        )
    } else {
        (0, 0, None, f32::INFINITY)
    };
    let training_max_moves = maximum_legal_moves(train);
    let validation_max_moves = maximum_legal_moves(validation);
    let steps_per_epoch = train.len().div_ceil(options.batch_size);
    let total_steps = steps_per_epoch.saturating_mul(options.epochs).max(1);
    let warmup_steps = ((total_steps as f64 * options.warmup_fraction).round() as usize)
        .min(total_steps.saturating_sub(1));
    let loss_weights = LossWeights {
        plan: options.plan_loss_weight,
        ..LossWeights::default()
    };
    let mut invocation_step = 0_usize;
    let mut optimizer_steps = previous_steps;
    train.sort_unstable_by_key(|record| (record.game_id, record.ply));
    let training_probe_records = train[..train.len().min(TRAINING_PROBE_POSITIONS)].to_vec();
    println!(
        "tensor_shapes training_max_moves={} validation_max_moves={}",
        training_max_moves, validation_max_moves
    );
    for epoch_offset in 0..options.epochs {
        let epoch = completed_epochs + epoch_offset;
        train.sort_unstable_by_key(|record| (record.game_id, record.ply));
        let mut rng = StdRng::seed_from_u64(splitmix64(options.seed ^ epoch as u64));
        train.shuffle(&mut rng);
        for (batch_index, records) in train.chunks(options.batch_size).enumerate() {
            let batch = make_batch_padded(records, training_max_moves, device)?;
            let output = model.predict(batch.network_input());
            let loss = training_loss_with_weights(output, &batch, loss_weights);
            let gradients = GradientsParams::from_grads(loss.backward(), &model);
            let learning_rate = scheduled_learning_rate(
                options.learning_rate,
                options.minimum_learning_rate_ratio,
                invocation_step,
                total_steps,
                warmup_steps,
            );
            model = optimizer.step(learning_rate, model, gradients);
            invocation_step += 1;
            optimizer_steps += 1;
            // Drop all per-batch upload tensors before asking CubeCL to release
            // unused pages. Keeping this batch alive made cleanup ineffective.
            drop(batch);
            if (batch_index + 1) % GPU_MAINTENANCE_INTERVAL == 0 {
                maintain_gpu::<B>(device, "training")?;
            }
        }

        // Validation must run on the inner backend. Running it on `B` creates
        // autodiff graphs that are never consumed by backward(), and used to
        // provoke a heap-sized WGPU allocation at the first epoch boundary.
        maintain_gpu::<B>(device, "before validation")?;
        let validation_model = model.valid();
        let training_probe = evaluate_model_padded(
            &validation_model,
            &training_probe_records,
            options.batch_size,
            training_max_moves,
            device,
            loss_weights,
        )?;
        let metrics = evaluate_model_padded(
            &validation_model,
            validation,
            options.batch_size,
            validation_max_moves,
            device,
            loss_weights,
        )?;
        let selection_loss = metrics.strategic_selection_loss();
        if !selection_loss.is_finite() {
            return Err(format!(
                "validation produced a non-finite checkpoint selection loss at epoch {}",
                epoch + 1
            ));
        }
        let is_best = selection_loss < best_selection_loss;
        if is_best {
            best_selection_loss = selection_loss;
            best_epoch = Some(epoch + 1);
            save_compact_checkpoint(&validation_model, &options.output_prefix)?;
        }
        drop(validation_model);
        maintain_gpu::<B>(device, "after validation")?;
        println!(
            "epoch={}/{} lr={:.8} train_probe_loss={:.6} validation_loss={:.6} selection_loss={:.6} best={} policy_ce={:.6} policy_kl={:.6} policy_gain_vs_uniform={:.6} policy_top1={:.4} policy_top3={:.4} policy_top8={:.4} policy_mass_top8={:.4} policy_regret_cp_top1={:.1} policy_regret_cp_top8={:.1} wdl_ce={:.6} wdl_accuracy={:.4} wdl_majority={:.4} wdl_balanced={:.4} plan_ce={:.6} plan_kl={:.6} plan_gain_vs_prior={:.6} plan_top1={:.4} plan_top2={:.4} plan_top3={:.4} plan_macro_recall={:.4} score_mse={:.6} risk_mse={:.6}",
            epoch + 1,
            completed_epochs + options.epochs,
            scheduled_learning_rate(
                options.learning_rate,
                options.minimum_learning_rate_ratio,
                invocation_step.saturating_sub(1),
                total_steps,
                warmup_steps,
            ),
            training_probe.loss,
            metrics.loss,
            selection_loss,
            is_best,
            metrics.policy_cross_entropy,
            metrics.policy_cross_entropy - metrics.policy_target_entropy,
            metrics.policy_uniform_cross_entropy - metrics.policy_cross_entropy,
            metrics.policy_top1,
            metrics.policy_top3,
            metrics.policy_top8,
            metrics.policy_target_mass_top8,
            metrics.policy_regret_cp_top1,
            metrics.policy_regret_cp_top8,
            metrics.wdl_cross_entropy,
            metrics.wdl_accuracy,
            metrics.wdl_majority_accuracy,
            metrics.wdl_balanced_accuracy,
            metrics.plan_cross_entropy,
            metrics.plan_cross_entropy - metrics.plan_target_entropy,
            metrics.plan_prior_cross_entropy - metrics.plan_cross_entropy,
            metrics.plan_accuracy,
            metrics.plan_top2,
            metrics.plan_top3,
            metrics.plan_macro_recall,
            metrics.score_mean_squared_error,
            metrics.tactical_risk_mean_squared_error,
        );
        maintain_gpu::<B>(device, "before training checkpoint")?;
        save_training_checkpoint(
            &model,
            &optimizer,
            options,
            epoch + 1,
            optimizer_steps,
            best_epoch,
            best_selection_loss,
        )?;
        training_manifest(
            options,
            TrainingResult {
                trained_epochs: epoch + 1,
                best_epoch: best_epoch.expect("the current run has selected a checkpoint"),
                best_selection_loss,
            },
            train.len(),
            validation.len(),
        )
        .save(&options.output_prefix)?;
    }
    Ok(TrainingResult {
        trained_epochs: completed_epochs + options.epochs,
        best_epoch: best_epoch.expect("positive epoch count always selects a checkpoint"),
        best_selection_loss,
    })
}

fn save_compact_checkpoint<B: Backend, M: Module<B>>(
    model: &M,
    prefix: &Path,
) -> Result<(), String> {
    if let Some(parent) = prefix.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("cannot create checkpoint directory: {error}"))?;
    }
    let temporary = suffixed_prefix(prefix, "-best-next");
    model
        .clone()
        .save_file(&temporary, &CompactRecorder::new())
        .map_err(|error| format!("cannot save best compact checkpoint: {error}"))?;
    fs::rename(
        temporary.with_extension("mpk"),
        prefix.with_extension("mpk"),
    )
    .map_err(|error| format!("cannot publish best compact checkpoint: {error}"))
}

fn save_training_checkpoint<B, M, O>(
    model: &M,
    optimizer: &O,
    options: &TrainingOptions,
    completed_epochs: usize,
    optimizer_steps: usize,
    best_epoch: Option<usize>,
    best_selection_loss: f32,
) -> Result<(), String>
where
    B: AutodiffBackend,
    M: StrategyNetwork<B> + AutodiffModule<B>,
    O: Optimizer<M, B>,
{
    if let Some(parent) = options.output_prefix.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("cannot create training checkpoint directory: {error}"))?;
    }
    let checkpoint_slot = completed_epochs % 2;
    let model_final = training_model_prefix(&options.output_prefix, checkpoint_slot);
    let model_temporary = suffixed_checkpoint_path(&model_final, "-next");
    model
        .clone()
        .save_file(&model_temporary, &DefaultRecorder::new())
        .map_err(|error| format!("cannot save resumable training model: {error}"))?;

    let optimizer_final = training_optimizer_prefix(&options.output_prefix, checkpoint_slot);
    let optimizer_temporary = suffixed_checkpoint_path(&optimizer_final, "-next");
    BinFileRecorder::<FullPrecisionSettings>::default()
        .record(optimizer.to_record(), optimizer_temporary.clone())
        .map_err(|error| format!("cannot save AdamW state: {error}"))?;

    fs::rename(
        model_temporary.with_extension("mpk"),
        model_final.with_extension("mpk"),
    )
    .map_err(|error| format!("cannot publish resumable training model: {error}"))?;
    fs::rename(
        optimizer_temporary.with_extension("bin"),
        optimizer_final.with_extension("bin"),
    )
    .map_err(|error| format!("cannot publish AdamW state: {error}"))?;

    let state = TrainingState {
        format: MODEL_FORMAT_VERSION.to_owned(),
        architecture: options.architecture,
        width: options.width,
        layers_or_steps: options.layers_or_steps,
        heads: options.heads,
        sparse_per_head: options.sparse_per_head,
        seed: options.seed,
        completed_epochs,
        optimizer_steps,
        checkpoint_slot,
        best_epoch,
        best_selection_loss: best_epoch.map(|_| best_selection_loss),
    };
    let state_final = training_state_path(&options.output_prefix);
    let state_temporary = state_final.with_extension("json.part");
    fs::write(
        &state_temporary,
        serde_json::to_vec_pretty(&state)
            .map_err(|error| format!("cannot encode training state: {error}"))?,
    )
    .map_err(|error| format!("cannot write training state: {error}"))?;
    fs::rename(state_temporary, state_final)
        .map_err(|error| format!("cannot publish training state: {error}"))
}

fn load_training_state(prefix: &Path) -> Result<TrainingState, String> {
    let bytes = fs::read(training_state_path(prefix)).map_err(|error| {
        format!(
            "cannot read resumable training state for {}: {error}",
            prefix.display()
        )
    })?;
    serde_json::from_slice(&bytes)
        .map_err(|error| format!("cannot decode resumable training state: {error}"))
}

fn validate_training_state(state: &TrainingState, options: &TrainingOptions) -> Result<(), String> {
    if state.format != MODEL_FORMAT_VERSION
        || state.architecture != options.architecture
        || state.width != options.width
        || state.layers_or_steps != options.layers_or_steps
        || state.heads != options.heads
        || state.sparse_per_head != options.sparse_per_head
        || state.seed != options.seed
        || state.checkpoint_slot > 1
    {
        return Err(
            "resume checkpoint does not match model format, architecture, dimensions, or split seed"
                .to_owned(),
        );
    }
    Ok(())
}

fn training_model_prefix(prefix: &Path, slot: usize) -> PathBuf {
    suffixed_prefix(prefix, &format!("-training-{slot}"))
}

fn training_optimizer_prefix(prefix: &Path, slot: usize) -> PathBuf {
    suffixed_prefix(prefix, &format!("-optimizer-{slot}"))
}

fn training_state_path(prefix: &Path) -> PathBuf {
    suffixed_prefix(prefix, "-training-state").with_extension("json")
}

fn suffixed_prefix(prefix: &Path, suffix: &str) -> PathBuf {
    let mut name = prefix
        .file_name()
        .unwrap_or_else(|| std::ffi::OsStr::new("teressa"))
        .to_os_string();
    name.push(suffix);
    prefix.parent().unwrap_or_else(|| Path::new(".")).join(name)
}

fn suffixed_checkpoint_path(prefix: &Path, suffix: &str) -> PathBuf {
    suffixed_prefix(prefix, suffix)
}

pub fn evaluate_model<B: Backend, M: StrategyNetwork<B>>(
    model: &M,
    records: &[DatasetRecord],
    batch_size: usize,
    device: &B::Device,
) -> Result<EpochMetrics, String> {
    evaluate_model_padded(
        model,
        records,
        batch_size,
        maximum_legal_moves(records),
        device,
        LossWeights::default(),
    )
}

fn evaluate_model_padded<B: Backend, M: StrategyNetwork<B>>(
    model: &M,
    records: &[DatasetRecord],
    batch_size: usize,
    max_moves: usize,
    device: &B::Device,
    loss_weights: LossWeights,
) -> Result<EpochMetrics, String> {
    let mut aggregate = EpochMetrics::default();
    let mut plan_correct_by_kind = [0.0_f32; PlanKind::COUNT];
    let mut plan_total_by_kind = [0.0_f32; PlanKind::COUNT];
    let mut wdl_correct_by_outcome = [0.0_f32; 3];
    let mut wdl_total_by_outcome = [0.0_f32; 3];
    let mut policy_regret_sums = [0.0_f32; 3];
    let plan_prior = empirical_plan_prior(records);
    for (batch_index, records) in records.chunks(batch_size.max(1)).enumerate() {
        let batch = make_batch_padded(records, max_moves, device)?;
        let output = model.predict(batch.network_input());
        let components = loss_components(output.clone(), &batch);
        let loss = components.weighted_total(loss_weights);
        let policy_logits = masked_logits(output.policy_logits, batch.move_mask.clone());
        let policy_target_entropy = -(batch.policy_target.clone()
            * batch.policy_target.clone().clamp_min(1.0e-12).log())
        .sum()
            / records.len() as f64;
        let policy_uniform_cross_entropy = batch
            .move_mask
            .clone()
            .sum_dim(1)
            .clamp_min(1.0)
            .log()
            .mean();
        let plan_logits = masked_logits(output.plan_logits, batch.plan_mask.clone());
        let plan_target_entropy =
            -(batch.plan_target.clone() * batch.plan_target.clone().clamp_min(1.0e-12).log()).sum()
                / records.len() as f64;
        let prior_logits = Tensor::<B, 2>::from_data(
            TensorData::new(plan_prior.to_vec(), [1, PlanKind::COUNT]),
            device,
        )
        .repeat_dim(0, records.len())
        .clamp_min(1.0e-12)
        .log();
        let plan_prior_cross_entropy = cross_entropy_with_logits(
            masked_logits(prior_logits, batch.plan_mask.clone()),
            batch.plan_target.clone(),
        );

        let target_policy = batch.policy_target.clone().argmax(1);
        let best_teacher_score = (batch.teacher_score.clone()
            + (batch.teacher_score_mask.clone() - 1.0) * 1.0e9)
            .max_dim(1);
        let mut policy_metrics = Vec::with_capacity(3);
        for k in [1_usize, 3, 8] {
            let (_, indices) = policy_logits.clone().topk_with_indices(k.min(max_moves), 1);
            let hits = topk_hits(indices.clone(), target_policy.clone());
            let target_mass = batch.policy_target.clone().gather(1, indices.clone()).sum();
            let selected_score_mask = batch.teacher_score_mask.clone().gather(1, indices.clone());
            let selected_score = (batch.teacher_score.clone().gather(1, indices)
                + (selected_score_mask - 1.0) * 1.0e9)
                .max_dim(1);
            // Mate-scale engine scores would otherwise dominate an average
            // expressed in centipawns. Two thousand cp is already a decisive
            // practical miss and keeps the statistic interpretable.
            let regret_sum = (best_teacher_score.clone() - selected_score)
                .clamp(0.0, 2_000.0)
                .sum();
            policy_metrics.push((hits, target_mass, regret_sum));
        }

        let predicted_wdl = output.wdl_logits.argmax(1);
        let target_wdl = batch.wdl_target.clone().argmax(1);
        let wdl_correct_mask = predicted_wdl.equal(target_wdl.clone()).float();
        let wdl_correct = wdl_correct_mask.clone().sum();
        let target_plan = batch.plan_target.clone().argmax(1);
        let mut plan_hits = Vec::with_capacity(3);
        let mut plan_correct_mask = None;
        for k in [1_usize, 2, 3] {
            let (_, indices) = plan_logits.clone().topk_with_indices(k, 1);
            let hit_mask = topk_hit_mask(indices, target_plan.clone());
            if k == 1 {
                plan_correct_mask = Some(hit_mask.clone());
            }
            plan_hits.push(hit_mask.sum());
        }
        let plan_correct_mask = plan_correct_mask.expect("top-1 plan metric is always present");
        let mut metric_tensors = vec![
            loss,
            components.policy,
            components.wdl,
            components.plan,
            components.score,
            components.tactical_risk,
            policy_target_entropy,
            policy_uniform_cross_entropy,
            plan_target_entropy,
            plan_prior_cross_entropy,
        ];
        for (hits, target_mass, regret_sum) in policy_metrics {
            metric_tensors.extend([hits, target_mass, regret_sum]);
        }
        metric_tensors.push(wdl_correct);
        metric_tensors.extend(plan_hits);
        for kind in PlanKind::ALL {
            let target_mask = target_plan.clone().equal_elem(kind.index() as i64).float();
            metric_tensors.push((plan_correct_mask.clone() * target_mask.clone()).sum());
            metric_tensors.push(target_mask.sum());
        }
        for outcome in 0..3 {
            let target_mask = target_wdl.clone().equal_elem(outcome as i64).float();
            metric_tensors.push((wdl_correct_mask.clone() * target_mask.clone()).sum());
            metric_tensors.push(target_mask.sum());
        }
        metric_tensors.push(batch.plan_target.clone().sum_dim(0).squeeze_dim::<1>(0));
        metric_tensors.push(softmax(plan_logits, 1).sum_dim(0).squeeze_dim::<1>(0));
        // A single small transfer replaces separate transfers of full
        // prediction and target arrays for every validation batch.
        let values = Tensor::cat(metric_tensors, 0)
            .into_data()
            .to_vec::<f32>()
            .map_err(|error| format!("cannot read validation metrics: {error}"))?;
        let sample_count = records.len() as f32;
        let mut cursor = 0;
        aggregate.loss += values[cursor] * sample_count;
        cursor += 1;
        aggregate.policy_cross_entropy += values[cursor] * sample_count;
        cursor += 1;
        aggregate.wdl_cross_entropy += values[cursor] * sample_count;
        cursor += 1;
        aggregate.plan_cross_entropy += values[cursor] * sample_count;
        cursor += 1;
        aggregate.score_mean_squared_error += values[cursor] * sample_count;
        cursor += 1;
        aggregate.tactical_risk_mean_squared_error += values[cursor] * sample_count;
        cursor += 1;
        aggregate.policy_target_entropy += values[cursor] * sample_count;
        cursor += 1;
        aggregate.policy_uniform_cross_entropy += values[cursor] * sample_count;
        cursor += 1;
        aggregate.plan_target_entropy += values[cursor] * sample_count;
        cursor += 1;
        aggregate.plan_prior_cross_entropy += values[cursor] * sample_count;
        cursor += 1;
        let policy_hits = [
            &mut aggregate.policy_top1,
            &mut aggregate.policy_top3,
            &mut aggregate.policy_top8,
        ];
        let policy_masses = [
            &mut aggregate.policy_target_mass_top1,
            &mut aggregate.policy_target_mass_top3,
            &mut aggregate.policy_target_mass_top8,
        ];
        for (index, (hits, mass)) in policy_hits.into_iter().zip(policy_masses).enumerate() {
            *hits += values[cursor];
            *mass += values[cursor + 1];
            policy_regret_sums[index] += values[cursor + 2];
            cursor += 3;
        }
        aggregate.wdl_accuracy += values[cursor];
        cursor += 1;
        aggregate.plan_accuracy += values[cursor];
        aggregate.plan_top2 += values[cursor + 1];
        aggregate.plan_top3 += values[cursor + 2];
        cursor += 3;
        for kind in PlanKind::ALL {
            plan_correct_by_kind[kind.index()] += values[cursor];
            plan_total_by_kind[kind.index()] += values[cursor + 1];
            cursor += 2;
        }
        for outcome in 0..3 {
            wdl_correct_by_outcome[outcome] += values[cursor];
            wdl_total_by_outcome[outcome] += values[cursor + 1];
            cursor += 2;
        }
        for kind in PlanKind::ALL {
            aggregate.plan_target_mass[kind.index()] += values[cursor + kind.index()];
        }
        cursor += PlanKind::COUNT;
        for kind in PlanKind::ALL {
            aggregate.plan_predicted_mass[kind.index()] += values[cursor + kind.index()];
        }
        aggregate.samples += records.len();
        drop(batch);
        if (batch_index + 1) % GPU_MAINTENANCE_INTERVAL == 0 {
            maintain_gpu::<B>(device, "validation")?;
        }
    }
    let count = aggregate.samples.max(1) as f32;
    aggregate.loss /= count;
    aggregate.policy_cross_entropy /= count;
    aggregate.wdl_cross_entropy /= count;
    aggregate.score_mean_squared_error /= count;
    aggregate.tactical_risk_mean_squared_error /= count;
    aggregate.policy_target_entropy /= count;
    aggregate.policy_uniform_cross_entropy /= count;
    aggregate.plan_cross_entropy /= count;
    aggregate.plan_target_entropy /= count;
    aggregate.plan_prior_cross_entropy /= count;
    aggregate.policy_top1 /= count;
    aggregate.policy_top3 /= count;
    aggregate.policy_top8 /= count;
    aggregate.policy_target_mass_top1 /= count;
    aggregate.policy_target_mass_top3 /= count;
    aggregate.policy_target_mass_top8 /= count;
    aggregate.policy_regret_cp_top1 = policy_regret_sums[0] / count;
    aggregate.policy_regret_cp_top3 = policy_regret_sums[1] / count;
    aggregate.policy_regret_cp_top8 = policy_regret_sums[2] / count;
    aggregate.wdl_accuracy /= count;
    aggregate.plan_accuracy /= count;
    aggregate.plan_top2 /= count;
    aggregate.plan_top3 /= count;
    let mut represented_outcomes = 0_usize;
    for outcome in 0..3 {
        aggregate.wdl_target_fraction[outcome] = wdl_total_by_outcome[outcome] / count;
        if wdl_total_by_outcome[outcome] > 0.0 {
            aggregate.wdl_recall[outcome] =
                wdl_correct_by_outcome[outcome] / wdl_total_by_outcome[outcome];
            aggregate.wdl_balanced_accuracy += aggregate.wdl_recall[outcome];
            represented_outcomes += 1;
        }
    }
    aggregate.wdl_balanced_accuracy /= represented_outcomes.max(1) as f32;
    aggregate.wdl_majority_accuracy = aggregate
        .wdl_target_fraction
        .iter()
        .copied()
        .fold(0.0, f32::max);
    let mut represented = 0_usize;
    for kind in PlanKind::ALL {
        aggregate.plan_target_mass[kind.index()] /= count;
        aggregate.plan_predicted_mass[kind.index()] /= count;
        if plan_total_by_kind[kind.index()] > 0.0 {
            aggregate.plan_recall[kind.index()] =
                plan_correct_by_kind[kind.index()] / plan_total_by_kind[kind.index()];
            aggregate.plan_macro_recall += aggregate.plan_recall[kind.index()];
            represented += 1;
        }
    }
    aggregate.plan_macro_recall /= represented.max(1) as f32;
    Ok(aggregate)
}

fn empirical_plan_prior(records: &[DatasetRecord]) -> [f32; PlanKind::COUNT] {
    let mut prior = [0.0_f32; PlanKind::COUNT];
    for record in records {
        for target in &record.plan_policy {
            prior[target.kind.index()] += target.probability;
        }
    }
    let total = prior.iter().sum::<f32>().max(f32::EPSILON);
    for probability in &mut prior {
        *probability = (*probability / total).max(1.0e-12);
    }
    prior
}

fn topk_hits<B: Backend>(indices: Tensor<B, 2, Int>, target: Tensor<B, 2, Int>) -> Tensor<B, 1> {
    topk_hit_mask(indices, target).sum()
}

fn topk_hit_mask<B: Backend>(
    indices: Tensor<B, 2, Int>,
    target: Tensor<B, 2, Int>,
) -> Tensor<B, 2> {
    let k = indices.dims()[1];
    indices
        .equal(target.repeat_dim(1, k))
        .float()
        .sum_dim(1)
        .clamp_max(1.0)
}

const GPU_MAINTENANCE_INTERVAL: usize = 64;
const TRAINING_PROBE_POSITIONS: usize = 512;

fn maintain_gpu<B: Backend>(device: &B::Device, stage: &str) -> Result<(), String> {
    B::sync(device)
        .map_err(|error| format!("GPU synchronization failed during {stage}: {error}"))?;
    B::memory_cleanup(device);
    Ok(())
}

/// Computes the complete multi-head training objective.
///
/// This is public so the GPU lifecycle benchmark can exercise the exact same
/// graph as the real trainer instead of a smaller synthetic approximation.
pub fn training_loss<B: Backend>(
    output: crate::model::NetworkOutput<B>,
    batch: &TrainingBatch<B>,
) -> Tensor<B, 1> {
    training_loss_with_weights(output, batch, LossWeights::default())
}

pub fn training_loss_with_weights<B: Backend>(
    output: crate::model::NetworkOutput<B>,
    batch: &TrainingBatch<B>,
    weights: LossWeights,
) -> Tensor<B, 1> {
    loss_components(output, batch).weighted_total(weights)
}

struct LossComponents<B: Backend> {
    policy: Tensor<B, 1>,
    wdl: Tensor<B, 1>,
    plan: Tensor<B, 1>,
    score: Tensor<B, 1>,
    tactical_risk: Tensor<B, 1>,
}

impl<B: Backend> LossComponents<B> {
    fn weighted_total(&self, weights: LossWeights) -> Tensor<B, 1> {
        self.policy.clone() * weights.policy
            + self.wdl.clone() * weights.wdl
            + self.plan.clone() * weights.plan
            + self.score.clone() * weights.score
            + self.tactical_risk.clone() * weights.tactical_risk
    }
}

fn loss_components<B: Backend>(
    output: crate::model::NetworkOutput<B>,
    batch: &TrainingBatch<B>,
) -> LossComponents<B> {
    let policy = cross_entropy_with_logits(
        masked_logits(output.policy_logits, batch.move_mask.clone()),
        batch.policy_target.clone(),
    );
    let wdl = cross_entropy_with_logits(output.wdl_logits, batch.wdl_target.clone());
    let plan = cross_entropy_with_logits(
        masked_logits(output.plan_logits, batch.plan_mask.clone()),
        batch.plan_target.clone(),
    );
    let score = (output.score - batch.score_target.clone()).square().mean();
    let risk_prob = burn::tensor::activation::sigmoid(output.tactical_risk_logits);
    let risk_error = (risk_prob - batch.risk_target.clone()).square() * batch.move_mask.clone();
    let risk = risk_error.sum() / batch.move_mask.clone().sum().clamp_min(1.0);
    LossComponents {
        policy,
        wdl,
        plan,
        score,
        tactical_risk: risk,
    }
}

fn scheduled_learning_rate(
    maximum: f64,
    minimum_ratio: f64,
    step: usize,
    total_steps: usize,
    warmup_steps: usize,
) -> f64 {
    if total_steps <= 1 {
        return maximum;
    }
    if warmup_steps > 0 && step < warmup_steps {
        return maximum * (step + 1) as f64 / warmup_steps as f64;
    }
    let decay_intervals = total_steps
        .saturating_sub(warmup_steps)
        .saturating_sub(1)
        .max(1);
    let decay_step = step.saturating_sub(warmup_steps).min(decay_intervals);
    let progress = decay_step as f64 / decay_intervals as f64;
    let minimum = maximum * minimum_ratio;
    minimum + 0.5 * (maximum - minimum) * (1.0 + (std::f64::consts::PI * progress).cos())
}

fn masked_logits<B: Backend>(logits: Tensor<B, 2>, mask: Tensor<B, 2>) -> Tensor<B, 2> {
    logits + (mask - 1.0) * 1.0e9
}

pub fn make_batch<B: Backend>(
    records: &[DatasetRecord],
    device: &B::Device,
) -> Result<TrainingBatch<B>, String> {
    if records.is_empty() {
        return Err("cannot create an empty batch".to_owned());
    }
    make_batch_padded(records, maximum_legal_moves(records), device)
}

fn make_batch_padded<B: Backend>(
    records: &[DatasetRecord],
    max_moves: usize,
    device: &B::Device,
) -> Result<TrainingBatch<B>, String> {
    if records.is_empty() {
        return Err("cannot create an empty batch".to_owned());
    }
    let required_moves = maximum_legal_moves(records);
    if required_moves > max_moves {
        return Err(format!(
            "batch requires {required_moves} legal-move slots but fixed padding provides {max_moves}"
        ));
    }
    let max_moves = max_moves.max(1);
    let batch = records.len();
    let mut boards = Vec::with_capacity(batch * BOARD_TOKENS * BOARD_FEATURES);
    let mut board_masks = Vec::with_capacity(batch * BOARD_TOKENS);
    let mut histories = Vec::with_capacity(batch * HISTORY_PLIES * MOVE_FEATURES);
    let mut history_masks = Vec::with_capacity(batch * HISTORY_PLIES);
    let mut moves = vec![0.0; batch * max_moves * MOVE_FEATURES];
    let mut move_from = vec![0_i64; batch * max_moves];
    let mut move_to = vec![0_i64; batch * max_moves];
    let mut move_masks = vec![0.0; batch * max_moves];
    let mut teacher_scores = vec![0.0; batch * max_moves];
    let mut teacher_score_masks = vec![0.0; batch * max_moves];
    let mut plan_inputs = vec![0.0; batch * PlanKind::COUNT];
    let mut plan_masks = vec![0.0; batch * PlanKind::COUNT];
    let mut policy_targets = vec![0.0; batch * max_moves];
    let mut wdl_targets = Vec::with_capacity(batch * 3);
    let mut plan_targets = vec![0.0; batch * PlanKind::COUNT];
    let mut score_targets = Vec::with_capacity(batch);
    let mut risk_targets = vec![0.0; batch * max_moves];

    for (sample, record) in records.iter().enumerate() {
        let position = capablanca_chess_plus::Position::from_fen(
            capablanca_chess_plus::Variant::TerachessII.rules(),
            &record.position_fen,
        )
        .map_err(|error| format!("invalid dataset FEN in game {}: {error}", record.game_id))?;
        let encoded = encode_position_uci_history(&position, &record.history);
        boards.extend(encoded.board);
        board_masks.extend(encoded.board_mask);
        for history in encoded.history {
            history_masks.push(history.values[11]);
            histories.extend(history.values);
        }
        let mut legal = position.legal_moves();
        legal.sort_unstable_by_key(|chess_move| chess_move.to_uci());
        let policy_by_move = record
            .policy
            .iter()
            .map(|target| (target.uci.as_str(), target))
            .collect::<HashMap<_, _>>();
        let best_score = record
            .policy
            .iter()
            .map(|target| target.teacher_score)
            .max()
            .unwrap_or(record.teacher_score);
        for target in &record.plan_policy {
            plan_masks[sample * PlanKind::COUNT + target.kind.index()] = 1.0;
        }
        for (move_index, (chess_move, encoded_move)) in
            legal.iter().zip(encoded.legal_moves).enumerate()
        {
            let flat = (sample * max_moves + move_index) * MOVE_FEATURES;
            moves[flat..flat + MOVE_FEATURES].copy_from_slice(&encoded_move.values);
            move_from[sample * max_moves + move_index] = encoded_move.from_token as i64;
            move_to[sample * max_moves + move_index] = encoded_move.to_token as i64;
            move_masks[sample * max_moves + move_index] = 1.0;
            if let Some(target) = policy_by_move.get(chess_move.to_uci().as_str()) {
                policy_targets[sample * max_moves + move_index] = target.probability;
                teacher_scores[sample * max_moves + move_index] = target.teacher_score as f32;
                teacher_score_masks[sample * max_moves + move_index] = 1.0;
                risk_targets[sample * max_moves + move_index] =
                    f32::from(best_score - target.teacher_score > 180);
            } else {
                risk_targets[sample * max_moves + move_index] = 1.0;
            }
        }
        let plan_index = record.plan.kind.index();
        plan_inputs[sample * PlanKind::COUNT + plan_index] = 1.0;
        for target in &record.plan_policy {
            plan_targets[sample * PlanKind::COUNT + target.kind.index()] = target.probability;
        }
        wdl_targets.extend(record.wdl);
        score_targets.push((record.teacher_score as f32 / 2_000.0).clamp(-1.0, 1.0));
    }
    Ok(TrainingBatch {
        board: Tensor::from_data(
            TensorData::new(boards, [batch, BOARD_TOKENS, BOARD_FEATURES]),
            device,
        ),
        board_mask: Tensor::from_data(TensorData::new(board_masks, [batch, BOARD_TOKENS]), device),
        history: Tensor::from_data(
            TensorData::new(histories, [batch, HISTORY_PLIES, MOVE_FEATURES]),
            device,
        ),
        history_mask: Tensor::from_data(
            TensorData::new(history_masks, [batch, HISTORY_PLIES]),
            device,
        ),
        moves: Tensor::from_data(
            TensorData::new(moves, [batch, max_moves, MOVE_FEATURES]),
            device,
        ),
        move_from: Tensor::from_data(TensorData::new(move_from, [batch, max_moves]), device),
        move_to: Tensor::from_data(TensorData::new(move_to, [batch, max_moves]), device),
        move_mask: Tensor::from_data(TensorData::new(move_masks, [batch, max_moves]), device),
        teacher_score: Tensor::from_data(
            TensorData::new(teacher_scores, [batch, max_moves]),
            device,
        ),
        teacher_score_mask: Tensor::from_data(
            TensorData::new(teacher_score_masks, [batch, max_moves]),
            device,
        ),
        plan_input: Tensor::from_data(
            TensorData::new(plan_inputs, [batch, PlanKind::COUNT]),
            device,
        ),
        plan_mask: Tensor::from_data(
            TensorData::new(plan_masks, [batch, PlanKind::COUNT]),
            device,
        ),
        policy_target: Tensor::from_data(
            TensorData::new(policy_targets, [batch, max_moves]),
            device,
        ),
        wdl_target: Tensor::from_data(TensorData::new(wdl_targets, [batch, 3]), device),
        plan_target: Tensor::from_data(
            TensorData::new(plan_targets, [batch, PlanKind::COUNT]),
            device,
        ),
        score_target: Tensor::from_data(TensorData::new(score_targets, [batch, 1]), device),
        risk_target: Tensor::from_data(TensorData::new(risk_targets, [batch, max_moves]), device),
    })
}

fn maximum_legal_moves(records: &[DatasetRecord]) -> usize {
    records
        .iter()
        .map(|record| record.legal_moves.len())
        .max()
        .unwrap_or(1)
        .max(1)
}

pub fn load_records(inputs: &[PathBuf]) -> Result<Vec<DatasetRecord>, String> {
    let mut paths = Vec::new();
    for input in inputs {
        collect_shards(input, &mut paths)?;
    }
    paths.sort();
    paths.dedup();
    if paths.is_empty() {
        return Err("no .jsonl.zst dataset shards found".to_owned());
    }
    let mut records = Vec::new();
    for path in paths {
        records.extend(DatasetShardReader::open(&path)?);
    }
    Ok(records)
}

pub fn split_records(
    records: Vec<DatasetRecord>,
    seed: u64,
) -> (Vec<DatasetRecord>, Vec<DatasetRecord>, Vec<DatasetRecord>) {
    let mut train = Vec::new();
    let mut validation = Vec::new();
    let mut test = Vec::new();
    for record in records {
        let bucket = splitmix64(seed ^ record.game_id) % 100;
        if bucket < 80 {
            train.push(record);
        } else if bucket < 90 {
            validation.push(record);
        } else {
            test.push(record);
        }
    }
    (train, validation, test)
}

fn collect_shards(path: &Path, output: &mut Vec<PathBuf>) -> Result<(), String> {
    if path.is_file() {
        if path.to_string_lossy().ends_with(".jsonl.zst") {
            output.push(path.to_path_buf());
        }
        return Ok(());
    }
    let entries = fs::read_dir(path)
        .map_err(|error| format!("cannot read dataset path {}: {error}", path.display()))?;
    for entry in entries {
        let entry = entry.map_err(|error| format!("cannot read dataset entry: {error}"))?;
        let child = entry.path();
        if child.is_dir() {
            collect_shards(&child, output)?;
        } else if child.to_string_lossy().ends_with(".jsonl.zst") {
            output.push(child);
        }
    }
    Ok(())
}

fn manifest_path(prefix: &Path) -> PathBuf {
    prefix.with_extension("json")
}

fn splitmix64(mut value: u64) -> u64 {
    value = value.wrapping_add(0x9e37_79b9_7f4a_7c15);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dataset::{PlanPolicyTarget, PolicyTarget};
    use crate::plan::generate_plan_candidates;
    use burn::backend::Flex;
    use capablanca_chess_plus::Variant;

    #[test]
    fn warmup_cosine_schedule_reaches_peak_and_floor() {
        let peak = 6.0e-4;
        assert!((scheduled_learning_rate(peak, 0.1, 0, 100, 10) - peak * 0.1).abs() < 1e-12);
        assert!((scheduled_learning_rate(peak, 0.1, 9, 100, 10) - peak).abs() < 1e-12);
        assert!((scheduled_learning_rate(peak, 0.1, 10, 100, 10) - peak).abs() < 1e-12);
        assert!((scheduled_learning_rate(peak, 0.1, 99, 100, 10) - peak * 0.1).abs() < 1e-12);
    }

    #[test]
    fn legacy_v3_manifest_without_selection_metadata_still_loads() {
        let manifest: ModelManifest = serde_json::from_str(
            r#"{
                "format":"TERESSA_MODEL_V3",
                "architecture":"bdh",
                "width":192,
                "layers_or_steps":4,
                "sparse_per_head":48,
                "heads":4,
                "trained_epochs":8,
                "training_positions":100,
                "validation_positions":10
            }"#,
        )
        .unwrap();
        assert_eq!(manifest.selected_epoch, None);
        assert_eq!(manifest.validation_selection_loss, None);
    }

    #[test]
    fn topk_hit_mask_matches_target_membership() {
        let device = Default::default();
        let indices = Tensor::<Flex, 2, Int>::from_data(
            TensorData::new(vec![4_i64, 2, 1, 0, 3, 2], [2, 3]),
            &device,
        );
        let targets =
            Tensor::<Flex, 2, Int>::from_data(TensorData::new(vec![2_i64, 4], [2, 1]), &device);
        assert_eq!(
            topk_hit_mask(indices, targets)
                .into_data()
                .to_vec::<f32>()
                .unwrap(),
            vec![1.0, 0.0]
        );
    }

    #[test]
    fn batches_pad_variable_move_lists_and_keep_targets_normalized() {
        let position = Variant::TerachessII.starting_position();
        let mut legal = position
            .legal_moves()
            .into_iter()
            .map(|chess_move| chess_move.to_uci())
            .collect::<Vec<_>>();
        legal.sort();
        let record = DatasetRecord {
            game_id: 1,
            ply: 0,
            position_fen: position.to_fen(),
            history: Vec::new(),
            policy: vec![PolicyTarget {
                uci: legal[0].clone(),
                probability: 1.0,
                teacher_score: 10,
            }],
            legal_moves: legal,
            wdl: [0.0, 1.0, 0.0],
            teacher_score: 10,
            plan_policy: vec![
                PlanPolicyTarget {
                    kind: PlanKind::DevelopPiece,
                    probability: 0.7,
                },
                PlanPolicyTarget {
                    kind: PlanKind::KingSafety,
                    probability: 0.3,
                },
            ],
            plan: generate_plan_candidates(&position).remove(0),
        };
        let device = Default::default();
        let batch = make_batch::<Flex>(std::slice::from_ref(&record), &device).unwrap();
        assert_eq!(batch.board.dims(), [1, BOARD_TOKENS, BOARD_FEATURES]);
        let plan_target = batch.plan_target.to_data().to_vec::<f32>().unwrap();
        let plan_mask = batch.plan_mask.to_data().to_vec::<f32>().unwrap();
        assert!((plan_target[PlanKind::DevelopPiece.index()] - 0.7).abs() < 1.0e-6);
        assert!((plan_target[PlanKind::KingSafety.index()] - 0.3).abs() < 1.0e-6);
        assert_eq!(plan_mask[PlanKind::DevelopPiece.index()], 1.0);
        assert_eq!(plan_mask[PlanKind::KingSafety.index()], 1.0);
        assert_eq!(
            batch
                .policy_target
                .to_data()
                .to_vec::<f32>()
                .unwrap()
                .iter()
                .sum::<f32>(),
            1.0
        );

        let padded_moves = batch.policy_target.dims()[1] + 17;
        let padded = make_batch_padded::<Flex>(&[record], padded_moves, &device).unwrap();
        assert_eq!(padded.policy_target.dims(), [1, padded_moves]);
        assert_eq!(padded.move_mask.dims(), [1, padded_moves]);
    }

    #[test]
    fn game_split_never_leaks_one_game_between_partitions() {
        let position = Variant::TerachessII.starting_position();
        let legal = position.legal_moves();
        let base = DatasetRecord {
            game_id: 7,
            ply: 0,
            position_fen: position.to_fen(),
            history: Vec::new(),
            legal_moves: legal.iter().map(|chess_move| chess_move.to_uci()).collect(),
            policy: vec![PolicyTarget {
                uci: legal[0].to_uci(),
                probability: 1.0,
                teacher_score: 0,
            }],
            wdl: [0.0, 1.0, 0.0],
            teacher_score: 0,
            plan_policy: vec![PlanPolicyTarget {
                kind: PlanKind::DevelopPiece,
                probability: 1.0,
            }],
            plan: generate_plan_candidates(&position).remove(0),
        };
        let mut second = base.clone();
        second.ply = 1;
        let (train, validation, test) = split_records(vec![base, second], 11);
        assert!([train.len(), validation.len(), test.len()].contains(&2));
    }
}
