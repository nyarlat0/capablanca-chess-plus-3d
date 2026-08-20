use crate::MODEL_FORMAT_VERSION;
use crate::dataset::{DatasetRecord, DatasetShardReader};
use crate::encode::{BOARD_FEATURES, BOARD_TOKENS, MOVE_FEATURES, encode_position_uci_history};
use crate::model::{BdhConfig, ResidualConfig, StrategyNetwork};
use crate::plan::PlanKind;
use burn::module::AutodiffModule;
use burn::optim::{AdamWConfig, GradientsParams, Optimizer};
use burn::prelude::*;
use burn::record::CompactRecorder;
use burn::tensor::backend::AutodiffBackend;
use burn::tensor::loss::cross_entropy_with_logits;
use rand::SeedableRng;
use rand::rngs::StdRng;
use rand::seq::SliceRandom;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

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
    pub seed: u64,
    pub width: usize,
    pub layers_or_steps: usize,
    pub heads: usize,
    pub sparse_per_head: usize,
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
            seed: 0x5445_5245_5353_4132,
            width: 192,
            layers_or_steps: 4,
            heads: 4,
            sparse_per_head: 48,
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct EpochMetrics {
    pub loss: f32,
    pub policy_top1: f32,
    pub wdl_accuracy: f32,
    pub plan_accuracy: f32,
    pub samples: usize,
}

pub struct TrainingBatch<B: Backend> {
    pub board: Tensor<B, 3>,
    pub board_mask: Tensor<B, 2>,
    pub moves: Tensor<B, 3>,
    pub move_mask: Tensor<B, 2>,
    pub plan_input: Tensor<B, 2>,
    pub policy_target: Tensor<B, 2>,
    pub wdl_target: Tensor<B, 2>,
    pub plan_target: Tensor<B, 2>,
    pub score_target: Tensor<B, 2>,
    pub risk_target: Tensor<B, 2>,
}

pub fn initialize_vulkan() -> burn::backend::wgpu::WgpuDevice {
    let device = burn::backend::wgpu::WgpuDevice::DiscreteGpu(0);
    // Training continuously creates upload and intermediate buffers. Exclusive
    // pages make the explicit cleanup below predictable, while a shorter task
    // queue limits how many submitted batches can retain buffers at once. The
    // main first-epoch VRAM spike was validation accidentally using autodiff;
    // these settings provide an additional bound instead of hiding that bug.
    burn::backend::wgpu::init_setup::<burn::backend::wgpu::graphics::Vulkan>(
        &device,
        burn::backend::wgpu::RuntimeOptions {
            tasks_max: 8,
            memory_config: burn::backend::wgpu::MemoryConfiguration::ExclusivePages,
        },
    );
    device
}

pub fn train_vulkan(options: &TrainingOptions) -> Result<ModelManifest, String> {
    if options.epochs == 0 || options.batch_size == 0 || options.width == 0 {
        return Err("epochs, batch-size, and width must be positive".to_owned());
    }
    let records = load_records(&options.inputs)?;
    let (mut train, validation, _) = split_records(records, options.seed);
    if train.is_empty() || validation.is_empty() {
        return Err(
            "dataset must contain at least one training and validation position".to_owned(),
        );
    }
    let device = initialize_vulkan();
    type Base = burn::backend::Wgpu;
    type Train = burn::backend::Autodiff<Base>;
    match options.architecture {
        Architecture::Bdh => {
            let model = BdhConfig::new()
                .with_width(options.width)
                .with_heads(options.heads)
                .with_sparse_per_head(options.sparse_per_head)
                .with_reasoning_steps(options.layers_or_steps)
                .init::<Train>(&device);
            train_model(model, &mut train, &validation, options, &device)?;
        }
        Architecture::Residual => {
            let model = ResidualConfig::new()
                .with_width(options.width)
                .with_layers(options.layers_or_steps)
                .with_hidden(options.width * 2)
                .init::<Train>(&device);
            train_model(model, &mut train, &validation, options, &device)?;
        }
    }
    let manifest = ModelManifest {
        format: MODEL_FORMAT_VERSION.to_owned(),
        architecture: options.architecture,
        width: options.width,
        layers_or_steps: options.layers_or_steps,
        sparse_per_head: options.sparse_per_head,
        heads: options.heads,
        trained_epochs: options.epochs,
        training_positions: train.len(),
        validation_positions: validation.len(),
    };
    manifest.save(&options.output_prefix)?;
    Ok(manifest)
}

fn train_model<B, M>(
    mut model: M,
    train: &mut [DatasetRecord],
    validation: &[DatasetRecord],
    options: &TrainingOptions,
    device: &B::Device,
) -> Result<(), String>
where
    B: AutodiffBackend,
    M: StrategyNetwork<B> + AutodiffModule<B>,
    M::InnerModule: StrategyNetwork<B::InnerBackend>,
{
    let mut optimizer = AdamWConfig::new().init::<B, M>();
    let mut rng = StdRng::seed_from_u64(options.seed);
    let training_max_moves = maximum_legal_moves(train);
    let validation_max_moves = maximum_legal_moves(validation);
    println!(
        "tensor_shapes training_max_moves={} validation_max_moves={}",
        training_max_moves, validation_max_moves
    );
    for epoch in 0..options.epochs {
        train.shuffle(&mut rng);
        for (batch_index, records) in train.chunks(options.batch_size).enumerate() {
            let batch = make_batch_padded(records, training_max_moves, device)?;
            let output = model.predict(
                batch.board.clone(),
                batch.board_mask.clone(),
                batch.moves.clone(),
                batch.plan_input.clone(),
            );
            let loss = training_loss(output, &batch);
            let gradients = GradientsParams::from_grads(loss.backward(), &model);
            model = optimizer.step(options.learning_rate, model, gradients);
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
            &train[..train.len().min(TRAINING_PROBE_POSITIONS)],
            options.batch_size,
            training_max_moves,
            device,
        )?;
        let metrics = evaluate_model_padded(
            &validation_model,
            validation,
            options.batch_size,
            validation_max_moves,
            device,
        )?;
        drop(validation_model);
        maintain_gpu::<B>(device, "after validation")?;
        println!(
            "epoch={}/{} train_probe_loss={:.6} validation_loss={:.6} policy_top1={:.4} wdl_accuracy={:.4} plan_accuracy={:.4}",
            epoch + 1,
            options.epochs,
            training_probe.loss,
            metrics.loss,
            metrics.policy_top1,
            metrics.wdl_accuracy,
            metrics.plan_accuracy,
        );
    }
    if let Some(parent) = options.output_prefix.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("cannot create checkpoint directory: {error}"))?;
    }
    model
        .valid()
        .save_file(&options.output_prefix, &CompactRecorder::new())
        .map_err(|error| format!("cannot save Burn checkpoint: {error}"))
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
    )
}

fn evaluate_model_padded<B: Backend, M: StrategyNetwork<B>>(
    model: &M,
    records: &[DatasetRecord],
    batch_size: usize,
    max_moves: usize,
    device: &B::Device,
) -> Result<EpochMetrics, String> {
    let mut aggregate = EpochMetrics::default();
    for (batch_index, records) in records.chunks(batch_size.max(1)).enumerate() {
        let batch = make_batch_padded(records, max_moves, device)?;
        let output = model.predict(
            batch.board.clone(),
            batch.board_mask.clone(),
            batch.moves.clone(),
            batch.plan_input.clone(),
        );
        let loss = training_loss(output.clone(), &batch);
        let policy_correct = masked_logits(output.policy_logits, batch.move_mask.clone())
            .argmax(1)
            .equal(batch.policy_target.clone().argmax(1))
            .float()
            .sum();
        let wdl_correct = output
            .wdl_logits
            .argmax(1)
            .equal(batch.wdl_target.clone().argmax(1))
            .float()
            .sum();
        let plan_correct = output
            .plan_logits
            .argmax(1)
            .equal(batch.plan_target.clone().argmax(1))
            .float()
            .sum();
        // One four-float transfer replaces seven separate transfers of full
        // prediction and target arrays for every validation batch.
        let values = Tensor::cat(vec![loss, policy_correct, wdl_correct, plan_correct], 0)
            .into_data()
            .to_vec::<f32>()
            .map_err(|error| format!("cannot read validation metrics: {error}"))?;
        aggregate.loss += values[0] * records.len() as f32;
        aggregate.policy_top1 += values[1];
        aggregate.wdl_accuracy += values[2];
        aggregate.plan_accuracy += values[3];
        aggregate.samples += records.len();
        drop(batch);
        if (batch_index + 1) % GPU_MAINTENANCE_INTERVAL == 0 {
            maintain_gpu::<B>(device, "validation")?;
        }
    }
    let count = aggregate.samples.max(1) as f32;
    aggregate.loss /= count;
    aggregate.policy_top1 /= count;
    aggregate.wdl_accuracy /= count;
    aggregate.plan_accuracy /= count;
    Ok(aggregate)
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
    let policy = cross_entropy_with_logits(
        masked_logits(output.policy_logits, batch.move_mask.clone()),
        batch.policy_target.clone(),
    );
    let wdl = cross_entropy_with_logits(output.wdl_logits, batch.wdl_target.clone());
    let plan = cross_entropy_with_logits(output.plan_logits, batch.plan_target.clone());
    let score = (output.score - batch.score_target.clone()).square().mean();
    let risk_prob = burn::tensor::activation::sigmoid(output.tactical_risk_logits);
    let risk_error = (risk_prob - batch.risk_target.clone()).square() * batch.move_mask.clone();
    let risk = risk_error.sum() / batch.move_mask.clone().sum().clamp_min(1.0);
    policy + wdl * 0.7 + plan * 0.35 + score * 0.25 + risk * 0.2
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
    let mut moves = vec![0.0; batch * max_moves * MOVE_FEATURES];
    let mut move_masks = vec![0.0; batch * max_moves];
    let mut plan_inputs = vec![0.0; batch * PlanKind::COUNT];
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
        for (move_index, (chess_move, encoded_move)) in
            legal.iter().zip(encoded.legal_moves).enumerate()
        {
            let flat = (sample * max_moves + move_index) * MOVE_FEATURES;
            moves[flat..flat + MOVE_FEATURES].copy_from_slice(&encoded_move.values);
            move_masks[sample * max_moves + move_index] = 1.0;
            if let Some(target) = policy_by_move.get(chess_move.to_uci().as_str()) {
                policy_targets[sample * max_moves + move_index] = target.probability;
                risk_targets[sample * max_moves + move_index] =
                    f32::from(best_score - target.teacher_score > 180);
            } else {
                risk_targets[sample * max_moves + move_index] = 1.0;
            }
        }
        let plan_index = record.plan.kind.index();
        plan_inputs[sample * PlanKind::COUNT + plan_index] = 1.0;
        plan_targets[sample * PlanKind::COUNT + plan_index] = 1.0;
        wdl_targets.extend(record.wdl);
        score_targets.push((record.teacher_score as f32 / 2_000.0).clamp(-1.0, 1.0));
    }
    Ok(TrainingBatch {
        board: Tensor::from_data(
            TensorData::new(boards, [batch, BOARD_TOKENS, BOARD_FEATURES]),
            device,
        ),
        board_mask: Tensor::from_data(TensorData::new(board_masks, [batch, BOARD_TOKENS]), device),
        moves: Tensor::from_data(
            TensorData::new(moves, [batch, max_moves, MOVE_FEATURES]),
            device,
        ),
        move_mask: Tensor::from_data(TensorData::new(move_masks, [batch, max_moves]), device),
        plan_input: Tensor::from_data(
            TensorData::new(plan_inputs, [batch, PlanKind::COUNT]),
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
    use crate::dataset::PolicyTarget;
    use crate::plan::generate_plan_candidates;
    use burn::backend::Flex;
    use capablanca_chess_plus::Variant;

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
            plan: generate_plan_candidates(&position).remove(0),
        };
        let device = Default::default();
        let batch = make_batch::<Flex>(&[record.clone()], &device).unwrap();
        assert_eq!(batch.board.dims(), [1, BOARD_TOKENS, BOARD_FEATURES]);
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
            plan: generate_plan_candidates(&position).remove(0),
        };
        let mut second = base.clone();
        second.ply = 1;
        let (train, validation, test) = split_records(vec![base, second], 11);
        assert!([train.len(), validation.len(), test.len()].contains(&2));
    }
}
