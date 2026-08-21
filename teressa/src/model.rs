//! Burn models used by Teressa.
//!
//! The BDH core follows the sparse multiplicative recurrence in Pathway's
//! MIT-licensed reference implementation, adapted from causal language tokens
//! to a non-causal board. The same encoder/decoder parameters are reused for
//! every reasoning step, which is the important recurrent-depth property. A
//! conventional residual MLP trunk is kept as a matched experimental control.

use crate::encode::{BOARD_FEATURES, MOVE_FEATURES};
use crate::plan::PlanKind;
use burn::nn::{LayerNorm, LayerNormConfig, Linear, LinearConfig};
use burn::prelude::*;
use burn::tensor::Int;
use burn::tensor::activation::{gelu, relu, softmax};

/// Complete network input. Move endpoints remain integer board-token indices
/// so the policy can gather contextual origin/destination embeddings without
/// materializing a huge move-by-square one-hot tensor.
#[derive(Clone, Debug)]
pub struct NetworkInput<B: Backend> {
    pub board: Tensor<B, 3>,
    pub board_mask: Tensor<B, 2>,
    pub history: Tensor<B, 3>,
    pub history_mask: Tensor<B, 2>,
    pub moves: Tensor<B, 3>,
    pub move_from: Tensor<B, 2, Int>,
    pub move_to: Tensor<B, 2, Int>,
    pub plan: Tensor<B, 2>,
}

#[derive(Clone, Debug)]
pub struct NetworkOutput<B: Backend> {
    pub policy_logits: Tensor<B, 2>,
    pub wdl_logits: Tensor<B, 2>,
    pub plan_logits: Tensor<B, 2>,
    pub score: Tensor<B, 2>,
    pub tactical_risk_logits: Tensor<B, 2>,
}

/// Common forward surface used by training, evaluation, and native inference.
pub trait StrategyNetwork<B: Backend>: Module<B> {
    fn predict(&self, input: NetworkInput<B>) -> NetworkOutput<B>;
}

#[derive(Config, Debug)]
pub struct BdhConfig {
    #[config(default = 192)]
    pub width: usize,
    #[config(default = 4)]
    pub heads: usize,
    /// Sparse dimension per head. The local preset deliberately uses a much
    /// smaller expansion than language-scale BDH.
    #[config(default = 48)]
    pub sparse_per_head: usize,
    #[config(default = 4)]
    pub reasoning_steps: usize,
}

impl BdhConfig {
    #[must_use]
    pub fn init<B: Backend>(&self, device: &B::Device) -> BdhNetwork<B> {
        assert!(self.width > 0);
        assert!(self.heads > 0);
        assert!(self.sparse_per_head > 0);
        let sparse = self.heads * self.sparse_per_head;
        BdhNetwork {
            board_projection: LinearConfig::new(BOARD_FEATURES, self.width).init(device),
            history_projection: LinearConfig::new(MOVE_FEATURES, self.width).init(device),
            plan_projection: LinearConfig::new(PlanKind::COUNT, self.width).init(device),
            core: BdhCoreConfig::new(self.width, sparse).init(device),
            heads: OutputHeadsConfig::new(self.width).init(device),
            reasoning_steps: self.reasoning_steps.clamp(1, 16),
        }
    }
}

#[derive(Module, Debug)]
pub struct BdhNetwork<B: Backend> {
    board_projection: Linear<B>,
    history_projection: Linear<B>,
    plan_projection: Linear<B>,
    core: BdhCore<B>,
    heads: OutputHeads<B>,
    reasoning_steps: usize,
}

impl<B: Backend> BdhNetwork<B> {
    pub fn forward(&self, input: NetworkInput<B>) -> NetworkOutput<B> {
        let mask = input.board_mask.clone().unsqueeze_dim::<3>(2);
        let history = pooled_history(&self.history_projection, input.history, input.history_mask);
        let mut hidden = (self.board_projection.forward(input.board)
            + history.unsqueeze_dim::<3>(1))
            * mask.clone();
        for _ in 0..self.reasoning_steps {
            hidden = self.core.forward(hidden, mask.clone());
        }
        let global = self.heads.pooled(hidden.clone(), input.board_mask.clone());
        let plan_logits = self.heads.plan_logits(global.clone());
        let plan = self.plan_projection.forward(input.plan);
        self.heads.forward(
            hidden,
            input.moves,
            input.move_from,
            input.move_to,
            global,
            plan,
            plan_logits,
        )
    }
}

impl<B: Backend> StrategyNetwork<B> for BdhNetwork<B> {
    fn predict(&self, input: NetworkInput<B>) -> NetworkOutput<B> {
        self.forward(input)
    }
}

#[derive(Config, Debug)]
struct BdhCoreConfig {
    width: usize,
    sparse: usize,
}

impl BdhCoreConfig {
    fn init<B: Backend>(&self, device: &B::Device) -> BdhCore<B> {
        BdhCore {
            norm: LayerNormConfig::new(self.width)
                .with_bias(false)
                .init(device),
            encoder: LinearConfig::new(self.width, self.sparse)
                .with_bias(false)
                .init(device),
            value_encoder: LinearConfig::new(self.width, self.sparse)
                .with_bias(false)
                .init(device),
            decoder: LinearConfig::new(self.sparse, self.width)
                .with_bias(false)
                .init(device),
        }
    }
}

#[derive(Module, Debug)]
struct BdhCore<B: Backend> {
    norm: LayerNorm<B>,
    encoder: Linear<B>,
    value_encoder: Linear<B>,
    decoder: Linear<B>,
}

impl<B: Backend> BdhCore<B> {
    fn forward(&self, input: Tensor<B, 3>, mask: Tensor<B, 3>) -> Tensor<B, 3> {
        let normalized = self.norm.forward(input.clone()) * mask.clone();
        let sparse = relu(self.encoder.forward(normalized.clone())) * mask.clone();
        // Non-causal associative state: sparse positive activations retrieve a
        // weighted mixture of board-token values. Division keeps the recurrent
        // update stable across 16x16 and future 18x18 boards.
        let token_count = mask.clone().sum_dim(1).clamp_min(1.0);
        let retrieved =
            associative_retrieval(sparse.clone(), normalized, token_count) * mask.clone();
        let retrieved_sparse = relu(self.value_encoder.forward(self.norm.forward(retrieved)));
        let update = self.decoder.forward(sparse * retrieved_sparse) * mask.clone();
        self.norm.forward(input + update) * mask
    }
}

/// Evaluate `(Q Q^T) V` as `Q (Q^T V)`. Matrix multiplication is associative,
/// but the latter order avoids materializing a token-by-token matrix. For the
/// production shape this replaces `[batch, 324, 324]` with
/// `[batch, sparse, width]`, reducing both matmul work and autodiff storage.
fn associative_retrieval<B: Backend>(
    sparse: Tensor<B, 3>,
    values: Tensor<B, 3>,
    token_count: Tensor<B, 3>,
) -> Tensor<B, 3> {
    let memory = sparse.clone().swap_dims(1, 2).matmul(values) / token_count;
    sparse.matmul(memory)
}

#[derive(Config, Debug)]
pub struct ResidualConfig {
    #[config(default = 192)]
    pub width: usize,
    #[config(default = 4)]
    pub layers: usize,
    #[config(default = 384)]
    pub hidden: usize,
}

impl ResidualConfig {
    #[must_use]
    pub fn init<B: Backend>(&self, device: &B::Device) -> ResidualNetwork<B> {
        let blocks = (0..self.layers.clamp(1, 16))
            .map(|_| ResidualBlockConfig::new(self.width, self.hidden).init(device))
            .collect();
        ResidualNetwork {
            board_projection: LinearConfig::new(BOARD_FEATURES, self.width).init(device),
            history_projection: LinearConfig::new(MOVE_FEATURES, self.width).init(device),
            plan_projection: LinearConfig::new(PlanKind::COUNT, self.width).init(device),
            blocks,
            heads: OutputHeadsConfig::new(self.width).init(device),
        }
    }
}

#[derive(Module, Debug)]
pub struct ResidualNetwork<B: Backend> {
    board_projection: Linear<B>,
    history_projection: Linear<B>,
    plan_projection: Linear<B>,
    blocks: Vec<ResidualBlock<B>>,
    heads: OutputHeads<B>,
}

impl<B: Backend> ResidualNetwork<B> {
    pub fn forward(&self, input: NetworkInput<B>) -> NetworkOutput<B> {
        let mask = input.board_mask.clone().unsqueeze_dim::<3>(2);
        let history = pooled_history(&self.history_projection, input.history, input.history_mask);
        let mut hidden = (self.board_projection.forward(input.board)
            + history.unsqueeze_dim::<3>(1))
            * mask.clone();
        for block in &self.blocks {
            hidden = block.forward(hidden, mask.clone());
        }
        let global = self.heads.pooled(hidden.clone(), input.board_mask.clone());
        let plan_logits = self.heads.plan_logits(global.clone());
        let plan = self.plan_projection.forward(input.plan);
        self.heads.forward(
            hidden,
            input.moves,
            input.move_from,
            input.move_to,
            global,
            plan,
            plan_logits,
        )
    }
}

impl<B: Backend> StrategyNetwork<B> for ResidualNetwork<B> {
    fn predict(&self, input: NetworkInput<B>) -> NetworkOutput<B> {
        self.forward(input)
    }
}

#[derive(Config, Debug)]
struct ResidualBlockConfig {
    width: usize,
    hidden: usize,
}

impl ResidualBlockConfig {
    fn init<B: Backend>(&self, device: &B::Device) -> ResidualBlock<B> {
        ResidualBlock {
            norm: LayerNormConfig::new(self.width).init(device),
            first: LinearConfig::new(self.width, self.hidden).init(device),
            second: LinearConfig::new(self.hidden, self.width).init(device),
        }
    }
}

#[derive(Module, Debug)]
struct ResidualBlock<B: Backend> {
    norm: LayerNorm<B>,
    first: Linear<B>,
    second: Linear<B>,
}

impl<B: Backend> ResidualBlock<B> {
    fn forward(&self, input: Tensor<B, 3>, mask: Tensor<B, 3>) -> Tensor<B, 3> {
        let update = self
            .second
            .forward(gelu(self.first.forward(self.norm.forward(input.clone()))));
        (input + update) * mask
    }
}

#[derive(Config, Debug)]
struct OutputHeadsConfig {
    width: usize,
}

impl OutputHeadsConfig {
    fn init<B: Backend>(&self, device: &B::Device) -> OutputHeads<B> {
        OutputHeads {
            move_projection: LinearConfig::new(MOVE_FEATURES, self.width).init(device),
            from_projection: LinearConfig::new(self.width, self.width).init(device),
            to_projection: LinearConfig::new(self.width, self.width).init(device),
            global_projection: LinearConfig::new(self.width, self.width).init(device),
            pool_score: LinearConfig::new(self.width, 1).init(device),
            policy: LinearConfig::new(self.width, 1).init(device),
            tactical: LinearConfig::new(self.width, 1).init(device),
            wdl: LinearConfig::new(self.width, 3).init(device),
            plan_hidden: LinearConfig::new(self.width, self.width).init(device),
            plan: LinearConfig::new(self.width, PlanKind::COUNT).init(device),
            score: LinearConfig::new(self.width, 1).init(device),
        }
    }
}

#[derive(Module, Debug)]
struct OutputHeads<B: Backend> {
    move_projection: Linear<B>,
    from_projection: Linear<B>,
    to_projection: Linear<B>,
    global_projection: Linear<B>,
    pool_score: Linear<B>,
    policy: Linear<B>,
    tactical: Linear<B>,
    wdl: Linear<B>,
    plan_hidden: Linear<B>,
    plan: Linear<B>,
    score: Linear<B>,
}

impl<B: Backend> OutputHeads<B> {
    fn pooled(&self, hidden: Tensor<B, 3>, board_mask: Tensor<B, 2>) -> Tensor<B, 2> {
        let attention = self.pool_score.forward(hidden.clone()).squeeze_dim::<2>(2)
            + (board_mask - 1.0) * 1.0e9;
        let attention = softmax(attention, 1).unsqueeze_dim::<3>(2);
        (hidden * attention).sum_dim(1).squeeze_dim::<2>(1)
    }

    fn plan_logits(&self, global: Tensor<B, 2>) -> Tensor<B, 2> {
        self.plan.forward(gelu(self.plan_hidden.forward(global)))
    }

    #[allow(clippy::too_many_arguments)]
    fn forward(
        &self,
        hidden: Tensor<B, 3>,
        moves: Tensor<B, 3>,
        move_from: Tensor<B, 2, Int>,
        move_to: Tensor<B, 2, Int>,
        global: Tensor<B, 2>,
        plan: Tensor<B, 2>,
        plan_logits: Tensor<B, 2>,
    ) -> NetworkOutput<B> {
        let from = gather_squares(hidden.clone(), move_from);
        let to = gather_squares(hidden, move_to);
        let move_hidden = gelu(
            self.move_projection.forward(moves)
                + self.from_projection.forward(from)
                + self.to_projection.forward(to)
                + self
                    .global_projection
                    .forward(global.clone())
                    .unsqueeze_dim::<3>(1)
                + plan.unsqueeze_dim::<3>(1),
        );
        NetworkOutput {
            policy_logits: self.policy.forward(move_hidden.clone()).squeeze_dim::<2>(2),
            tactical_risk_logits: self.tactical.forward(move_hidden).squeeze_dim::<2>(2),
            wdl_logits: self.wdl.forward(global.clone()),
            plan_logits,
            score: self.score.forward(global),
        }
    }
}

fn pooled_history<B: Backend>(
    projection: &Linear<B>,
    history: Tensor<B, 3>,
    history_mask: Tensor<B, 2>,
) -> Tensor<B, 2> {
    let mask = history_mask.clone().unsqueeze_dim::<3>(2);
    let summed = (gelu(projection.forward(history)) * mask)
        .sum_dim(1)
        .squeeze_dim::<2>(1);
    let count = history_mask
        .sum_dim(1)
        .squeeze_dim::<1>(1)
        .unsqueeze_dim::<2>(1)
        .clamp_min(1.0);
    summed / count
}

fn gather_squares<B: Backend>(hidden: Tensor<B, 3>, indices: Tensor<B, 2, Int>) -> Tensor<B, 3> {
    let width = hidden.dims()[2];
    hidden.gather(1, indices.unsqueeze_dim::<3>(2).repeat_dim(2, width))
}

#[cfg(test)]
mod tests {
    use super::*;
    use burn::backend::Flex;

    type TestBackend = Flex<f32>;

    fn inputs(device: &burn::backend::flex::FlexDevice) -> NetworkInput<TestBackend> {
        NetworkInput {
            board: Tensor::zeros([2, 324, BOARD_FEATURES], device),
            board_mask: Tensor::ones([2, 324], device),
            history: Tensor::zeros([2, 8, MOVE_FEATURES], device),
            history_mask: Tensor::zeros([2, 8], device),
            moves: Tensor::zeros([2, 96, MOVE_FEATURES], device),
            move_from: Tensor::zeros([2, 96], device),
            move_to: Tensor::zeros([2, 96], device),
            plan: Tensor::zeros([2, PlanKind::COUNT], device),
        }
    }

    #[test]
    fn both_trunks_produce_identical_public_shapes() {
        let device = Default::default();
        let input = inputs(&device);
        let bdh = BdhConfig::new()
            .with_width(32)
            .with_heads(2)
            .with_sparse_per_head(8)
            .with_reasoning_steps(2)
            .init(&device);
        let bdh_output = bdh.forward(input.clone());
        let residual = ResidualConfig::new()
            .with_width(32)
            .with_layers(2)
            .with_hidden(64)
            .init(&device);
        let residual_output = residual.forward(input);
        assert_eq!(bdh_output.policy_logits.dims(), [2, 96]);
        assert_eq!(residual_output.policy_logits.dims(), [2, 96]);
        assert_eq!(bdh_output.wdl_logits.dims(), [2, 3]);
        assert_eq!(residual_output.plan_logits.dims(), [2, PlanKind::COUNT]);
    }

    #[test]
    fn compact_associative_retrieval_matches_token_matrix_reference() {
        let device = Default::default();
        let sparse_values = (0..30)
            .map(|index| (index % 7) as f32 / 7.0)
            .collect::<Vec<_>>();
        let value_values = (0..40)
            .map(|index| ((index * 3) % 11) as f32 / 11.0)
            .collect::<Vec<_>>();
        let sparse =
            Tensor::<TestBackend, 3>::from_data(TensorData::new(sparse_values, [2, 5, 3]), &device);
        let values =
            Tensor::<TestBackend, 3>::from_data(TensorData::new(value_values, [2, 5, 4]), &device);
        let reference = sparse
            .clone()
            .matmul(sparse.clone().swap_dims(1, 2))
            .matmul(values.clone())
            / 5.0;
        let token_count = Tensor::<TestBackend, 3>::full([2, 1, 1], 5.0, &device);
        let compact = associative_retrieval(sparse, values, token_count);
        let reference = reference.into_data().to_vec::<f32>().unwrap();
        let compact = compact.into_data().to_vec::<f32>().unwrap();

        assert_eq!(reference.len(), compact.len());
        for (reference, compact) in reference.into_iter().zip(compact) {
            assert!((reference - compact).abs() < 1.0e-5);
        }
    }

    #[test]
    fn move_endpoint_gather_returns_contextual_square_tokens() {
        let device = Default::default();
        let hidden = Tensor::<TestBackend, 3>::from_data(
            TensorData::new(vec![1.0, 10.0, 2.0, 20.0, 3.0, 30.0, 4.0, 40.0], [1, 4, 2]),
            &device,
        );
        let indices = Tensor::<TestBackend, 2, Int>::from_data(
            TensorData::new(vec![3_i64, 1_i64], [1, 2]),
            &device,
        );
        assert_eq!(
            gather_squares(hidden, indices)
                .into_data()
                .to_vec::<f32>()
                .unwrap(),
            vec![4.0, 40.0, 2.0, 20.0]
        );
    }
}
