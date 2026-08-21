use crate::encode::{BOARD_FEATURES, BOARD_TOKENS, HISTORY_PLIES, MOVE_FEATURES, encode_position};
use crate::hybrid::{NeuralAdvice, NeuralAdvisor, PolicyCandidate};
use crate::model::{
    BdhConfig, BdhNetwork, NetworkInput, ResidualConfig, ResidualNetwork, StrategyNetwork,
};
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
        input: NetworkInput<InferenceBackend>,
    ) -> crate::model::NetworkOutput<InferenceBackend> {
        match self {
            Self::Bdh(model) => model.predict(input),
            Self::Residual(model) => model.predict(input),
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
        let mut move_from = Vec::with_capacity(moves_count);
        let mut move_to = Vec::with_capacity(moves_count);
        for chess_move in encoded.legal_moves {
            move_values.extend(chess_move.values);
            move_from.push(chess_move.from_token as i64);
            move_to.push(chess_move.to_token as i64);
        }
        let mut history_values = Vec::with_capacity(HISTORY_PLIES * MOVE_FEATURES);
        let mut history_mask = Vec::with_capacity(HISTORY_PLIES);
        for chess_move in encoded.history {
            history_mask.push(chess_move.values[11]);
            history_values.extend(chess_move.values);
        }
        let mut plan = vec![0.0; PlanKind::COUNT];
        if let Some(kind) = plan_kind {
            plan[kind.index()] = 1.0;
        }
        let output = self.model.predict(NetworkInput {
            board: Tensor::from_data(
                TensorData::new(encoded.board, [1, BOARD_TOKENS, BOARD_FEATURES]),
                &self.device,
            ),
            board_mask: Tensor::from_data(
                TensorData::new(encoded.board_mask, [1, BOARD_TOKENS]),
                &self.device,
            ),
            history: Tensor::from_data(
                TensorData::new(history_values, [1, HISTORY_PLIES, MOVE_FEATURES]),
                &self.device,
            ),
            history_mask: Tensor::from_data(
                TensorData::new(history_mask, [1, HISTORY_PLIES]),
                &self.device,
            ),
            moves: Tensor::from_data(
                TensorData::new(move_values, [1, moves_count, MOVE_FEATURES]),
                &self.device,
            ),
            move_from: Tensor::from_data(
                TensorData::new(move_from, [1, moves_count]),
                &self.device,
            ),
            move_to: Tensor::from_data(TensorData::new(move_to, [1, moves_count]), &self.device),
            plan: Tensor::from_data(TensorData::new(plan, [1, PlanKind::COUNT]), &self.device),
        });
        Ok(RawAdvice {
            policy: values(softmax(output.policy_logits, 1))?,
            risk: values(sigmoid(output.tactical_risk_logits))?,
            wdl: values(softmax(output.wdl_logits, 1))?,
            plan_logits: values(output.plan_logits)?,
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
        let plan_probabilities = candidate_plan_probabilities(candidates, &raw.plan_logits)?;
        let selected_kind =
            candidates
                .iter()
                .map(|candidate| candidate.kind)
                .max_by(|left, right| {
                    plan_kind_score(*left, previous, &plan_probabilities)
                        .total_cmp(&plan_kind_score(*right, previous, &plan_probabilities))
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
        selected.confidence = plan_probabilities[selected.kind.index()];
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
    plan_logits: Vec<f32>,
}

fn candidate_plan_probabilities(
    candidates: &[StrategicPlan],
    logits: &[f32],
) -> Result<[f32; PlanKind::COUNT], String> {
    if logits.len() != PlanKind::COUNT {
        return Err("checkpoint plan output has the wrong size".to_owned());
    }
    let mut available = [false; PlanKind::COUNT];
    for candidate in candidates {
        available[candidate.kind.index()] = true;
    }
    let maximum = PlanKind::ALL
        .into_iter()
        .filter(|kind| available[kind.index()])
        .map(|kind| logits[kind.index()])
        .reduce(f32::max)
        .ok_or_else(|| "symbolic planner supplied no candidate kinds".to_owned())?;
    let mut probabilities = [0.0_f32; PlanKind::COUNT];
    let mut total = 0.0_f32;
    for kind in PlanKind::ALL {
        if available[kind.index()] {
            let probability = (logits[kind.index()] - maximum).exp();
            probabilities[kind.index()] = probability;
            total += probability;
        }
    }
    if !total.is_finite() || total <= 0.0 {
        return Err("checkpoint produced invalid plan logits".to_owned());
    }
    for probability in &mut probabilities {
        *probability /= total;
    }
    Ok(probabilities)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::generate_plan_candidates;
    use capablanca_chess_plus::Variant;

    #[test]
    fn plan_probabilities_are_normalized_only_over_applicable_kinds() {
        let candidates = generate_plan_candidates(&Variant::TerachessII.starting_position());
        let logits = (0..PlanKind::COUNT)
            .map(|index| index as f32 * 0.1)
            .collect::<Vec<_>>();
        let probabilities = candidate_plan_probabilities(&candidates, &logits).unwrap();
        let mut available = [false; PlanKind::COUNT];
        for candidate in candidates {
            available[candidate.kind.index()] = true;
        }
        assert!((probabilities.iter().sum::<f32>() - 1.0).abs() < 1.0e-6);
        for kind in PlanKind::ALL {
            assert_eq!(probabilities[kind.index()] > 0.0, available[kind.index()]);
        }
    }
}
