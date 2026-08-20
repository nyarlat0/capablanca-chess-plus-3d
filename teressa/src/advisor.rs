use crate::encode::{BOARD_FEATURES, BOARD_TOKENS, MOVE_FEATURES, encode_position};
use crate::hybrid::{NeuralAdvice, NeuralAdvisor, PolicyCandidate};
use crate::model::{BdhConfig, BdhNetwork, ResidualConfig, ResidualNetwork, StrategyNetwork};
use crate::plan::{PlanKind, StrategicPlan};
use crate::training::{Architecture, ModelManifest, initialize_vulkan};
use burn::module::Module;
use burn::prelude::*;
use burn::record::CompactRecorder;
use burn::tensor::activation::{sigmoid, softmax};
use capablanca_chess_plus::Position;
use std::path::Path;

type InferenceBackend = burn::backend::Wgpu;

enum LoadedModel {
    Bdh(Box<BdhNetwork<InferenceBackend>>),
    Residual(Box<ResidualNetwork<InferenceBackend>>),
}

impl LoadedModel {
    fn predict(
        &self,
        board: Tensor<InferenceBackend, 3>,
        mask: Tensor<InferenceBackend, 2>,
        moves: Tensor<InferenceBackend, 3>,
        plan: Tensor<InferenceBackend, 2>,
    ) -> crate::model::NetworkOutput<InferenceBackend> {
        match self {
            Self::Bdh(model) => model.predict(board, mask, moves, plan),
            Self::Residual(model) => model.predict(board, mask, moves, plan),
        }
    }
}

/// Checkpoint-backed Vulkan advisor used by the hybrid agent and UCI binary.
pub struct VulkanAdvisor {
    model: LoadedModel,
    device: burn::backend::wgpu::WgpuDevice,
    recent_moves: Vec<capablanca_chess_plus::Move>,
}

impl VulkanAdvisor {
    pub fn load(prefix: impl AsRef<Path>) -> Result<Self, String> {
        let prefix = prefix.as_ref();
        let manifest = ModelManifest::load(prefix)?;
        let device = initialize_vulkan();
        let model = match manifest.architecture {
            Architecture::Bdh => LoadedModel::Bdh(Box::new(
                BdhConfig::new()
                    .with_width(manifest.width)
                    .with_heads(manifest.heads)
                    .with_sparse_per_head(manifest.sparse_per_head)
                    .with_reasoning_steps(manifest.layers_or_steps)
                    .init::<InferenceBackend>(&device)
                    .load_file(prefix, &CompactRecorder::new(), &device)
                    .map_err(|error| format!("cannot load BDH checkpoint: {error}"))?,
            )),
            Architecture::Residual => LoadedModel::Residual(Box::new(
                ResidualConfig::new()
                    .with_width(manifest.width)
                    .with_layers(manifest.layers_or_steps)
                    .with_hidden(manifest.width * 2)
                    .init::<InferenceBackend>(&device)
                    .load_file(prefix, &CompactRecorder::new(), &device)
                    .map_err(|error| format!("cannot load residual checkpoint: {error}"))?,
            )),
        };
        Ok(Self {
            model,
            device,
            recent_moves: Vec::new(),
        })
    }

    fn infer(&self, position: &Position, plan_kind: Option<PlanKind>) -> Result<RawAdvice, String> {
        let encoded = encode_position(position, &self.recent_moves);
        let moves_count = encoded.legal_moves.len();
        if moves_count == 0 {
            return Err("cannot infer a terminal position".to_owned());
        }
        let mut move_values = Vec::with_capacity(moves_count * MOVE_FEATURES);
        for chess_move in encoded.legal_moves {
            move_values.extend(chess_move.values);
        }
        let mut plan = vec![0.0; PlanKind::COUNT];
        if let Some(kind) = plan_kind {
            plan[kind.index()] = 1.0;
        }
        let output = self.model.predict(
            Tensor::from_data(
                TensorData::new(encoded.board, [1, BOARD_TOKENS, BOARD_FEATURES]),
                &self.device,
            ),
            Tensor::from_data(
                TensorData::new(encoded.board_mask, [1, BOARD_TOKENS]),
                &self.device,
            ),
            Tensor::from_data(
                TensorData::new(move_values, [1, moves_count, MOVE_FEATURES]),
                &self.device,
            ),
            Tensor::from_data(TensorData::new(plan, [1, PlanKind::COUNT]), &self.device),
        );
        Ok(RawAdvice {
            policy: values(softmax(output.policy_logits, 1))?,
            risk: values(sigmoid(output.tactical_risk_logits))?,
            wdl: values(softmax(output.wdl_logits, 1))?,
            plans: values(softmax(output.plan_logits, 1))?,
        })
    }
}

impl NeuralAdvisor for VulkanAdvisor {
    fn set_recent_moves(&mut self, history: &[capablanca_chess_plus::Move]) {
        self.recent_moves.clear();
        self.recent_moves.extend(
            history
                .iter()
                .rev()
                .take(crate::encode::HISTORY_PLIES)
                .rev()
                .copied(),
        );
    }

    fn propose_plan(
        &mut self,
        position: &Position,
        candidates: &[StrategicPlan],
        previous: Option<&StrategicPlan>,
    ) -> Result<StrategicPlan, String> {
        if candidates.is_empty() {
            return Err("symbolic planner supplied no candidates".to_owned());
        }
        let raw = self.infer(position, None)?;
        let selected_kind = candidates
            .iter()
            .map(|candidate| candidate.kind)
            .max_by(|left, right| {
                plan_kind_score(*left, previous, &raw.plans)
                    .total_cmp(&plan_kind_score(*right, previous, &raw.plans))
            })
            .expect("candidate set was checked");
        let mut selected = candidates
            .iter()
            .filter(|candidate| candidate.kind == selected_kind)
            .max_by(|left, right| {
                left.confidence
                    .total_cmp(&right.confidence)
                    .then_with(|| left.progress.total_cmp(&right.progress))
            })
            .expect("selected plan kind came from the candidate set")
            .clone();
        selected.confidence = raw.plans[selected.kind.index()].clamp(0.0, 1.0);
        Ok(selected)
    }

    fn evaluate(
        &mut self,
        position: &Position,
        plan: &StrategicPlan,
    ) -> Result<NeuralAdvice, String> {
        let raw = self.infer(position, Some(plan.kind))?;
        let mut legal = position.legal_moves();
        legal.sort_unstable_by_key(|chess_move| chess_move.to_uci());
        if legal.len() != raw.policy.len() || raw.risk.len() != legal.len() || raw.wdl.len() != 3 {
            return Err("checkpoint output shape does not match the position".to_owned());
        }
        let root_expected = raw.wdl[0] + raw.wdl[1] * 0.5;
        let policy = legal
            .into_iter()
            .zip(raw.policy.into_iter().zip(raw.risk))
            .map(|(chess_move, (probability, risk))| PolicyCandidate {
                chess_move,
                probability,
                expected_score: (root_expected - risk * 0.15).clamp(0.0, 1.0),
            })
            .collect();
        Ok(NeuralAdvice {
            policy,
            wdl: [raw.wdl[0], raw.wdl[1], raw.wdl[2]],
        })
    }
}

struct RawAdvice {
    policy: Vec<f32>,
    risk: Vec<f32>,
    wdl: Vec<f32>,
    plans: Vec<f32>,
}

fn plan_kind_score(kind: PlanKind, previous: Option<&StrategicPlan>, probabilities: &[f32]) -> f32 {
    probabilities[kind.index()]
        + f32::from(previous.is_some_and(|previous| previous.kind == kind)) * 0.08
}

fn values<const D: usize>(tensor: Tensor<InferenceBackend, D>) -> Result<Vec<f32>, String> {
    tensor
        .into_data()
        .to_vec::<f32>()
        .map_err(|error| format!("cannot read Vulkan inference result: {error}"))
}
