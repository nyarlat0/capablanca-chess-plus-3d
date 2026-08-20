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
use burn::tensor::activation::{gelu, relu};

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
    fn predict(
        &self,
        board: Tensor<B, 3>,
        board_mask: Tensor<B, 2>,
        moves: Tensor<B, 3>,
        plan: Tensor<B, 2>,
    ) -> NetworkOutput<B>;
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
    plan_projection: Linear<B>,
    core: BdhCore<B>,
    heads: OutputHeads<B>,
    reasoning_steps: usize,
}

impl<B: Backend> BdhNetwork<B> {
    pub fn forward(
        &self,
        board: Tensor<B, 3>,
        board_mask: Tensor<B, 2>,
        moves: Tensor<B, 3>,
        plan: Tensor<B, 2>,
    ) -> NetworkOutput<B> {
        let mask = board_mask.clone().unsqueeze_dim::<3>(2);
        let board_hidden = self.board_projection.forward(board) * mask.clone();
        let plan_logits = self
            .heads
            .plan_logits(board_hidden.clone(), board_mask.clone());
        let plan = self.plan_projection.forward(plan).unsqueeze_dim::<3>(1);
        let mut hidden = (board_hidden + plan) * mask.clone();
        for _ in 0..self.reasoning_steps {
            hidden = self.core.forward(hidden, mask.clone());
        }
        self.heads.forward(hidden, board_mask, moves, plan_logits)
    }
}

impl<B: Backend> StrategyNetwork<B> for BdhNetwork<B> {
    fn predict(
        &self,
        board: Tensor<B, 3>,
        board_mask: Tensor<B, 2>,
        moves: Tensor<B, 3>,
        plan: Tensor<B, 2>,
    ) -> NetworkOutput<B> {
        self.forward(board, board_mask, moves, plan)
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
        let tokens = input.dims()[1].max(1) as f64;
        let retrieved = associative_retrieval(sparse.clone(), normalized, tokens) * mask.clone();
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
    tokens: f64,
) -> Tensor<B, 3> {
    let memory = sparse.clone().swap_dims(1, 2).matmul(values) / tokens;
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
            plan_projection: LinearConfig::new(PlanKind::COUNT, self.width).init(device),
            blocks,
            heads: OutputHeadsConfig::new(self.width).init(device),
        }
    }
}

#[derive(Module, Debug)]
pub struct ResidualNetwork<B: Backend> {
    board_projection: Linear<B>,
    plan_projection: Linear<B>,
    blocks: Vec<ResidualBlock<B>>,
    heads: OutputHeads<B>,
}

impl<B: Backend> ResidualNetwork<B> {
    pub fn forward(
        &self,
        board: Tensor<B, 3>,
        board_mask: Tensor<B, 2>,
        moves: Tensor<B, 3>,
        plan: Tensor<B, 2>,
    ) -> NetworkOutput<B> {
        let mask = board_mask.clone().unsqueeze_dim::<3>(2);
        let board_hidden = self.board_projection.forward(board) * mask.clone();
        let plan_logits = self
            .heads
            .plan_logits(board_hidden.clone(), board_mask.clone());
        let plan = self.plan_projection.forward(plan).unsqueeze_dim::<3>(1);
        let mut hidden = (board_hidden + plan) * mask.clone();
        for block in &self.blocks {
            hidden = block.forward(hidden, mask.clone());
        }
        self.heads.forward(hidden, board_mask, moves, plan_logits)
    }
}

impl<B: Backend> StrategyNetwork<B> for ResidualNetwork<B> {
    fn predict(
        &self,
        board: Tensor<B, 3>,
        board_mask: Tensor<B, 2>,
        moves: Tensor<B, 3>,
        plan: Tensor<B, 2>,
    ) -> NetworkOutput<B> {
        self.forward(board, board_mask, moves, plan)
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
            policy: LinearConfig::new(self.width, 1).init(device),
            tactical: LinearConfig::new(self.width, 1).init(device),
            wdl: LinearConfig::new(self.width, 3).init(device),
            plan: LinearConfig::new(self.width, PlanKind::COUNT).init(device),
            score: LinearConfig::new(self.width, 1).init(device),
        }
    }
}

#[derive(Module, Debug)]
struct OutputHeads<B: Backend> {
    move_projection: Linear<B>,
    policy: Linear<B>,
    tactical: Linear<B>,
    wdl: Linear<B>,
    plan: Linear<B>,
    score: Linear<B>,
}

impl<B: Backend> OutputHeads<B> {
    fn pooled(&self, hidden: Tensor<B, 3>, board_mask: Tensor<B, 2>) -> Tensor<B, 2> {
        let mask = board_mask.clone().unsqueeze_dim::<3>(2);
        let summed = (hidden * mask).sum_dim(1).squeeze_dim::<2>(1);
        let count = board_mask
            .sum_dim(1)
            .squeeze_dim::<1>(1)
            .unsqueeze_dim::<2>(1)
            + 1e-6;
        summed / count
    }

    fn plan_logits(&self, hidden: Tensor<B, 3>, board_mask: Tensor<B, 2>) -> Tensor<B, 2> {
        self.plan.forward(self.pooled(hidden, board_mask))
    }

    fn forward(
        &self,
        hidden: Tensor<B, 3>,
        board_mask: Tensor<B, 2>,
        moves: Tensor<B, 3>,
        plan_logits: Tensor<B, 2>,
    ) -> NetworkOutput<B> {
        let global = self.pooled(hidden, board_mask);
        let move_hidden =
            gelu(self.move_projection.forward(moves) + global.clone().unsqueeze_dim::<3>(1));
        NetworkOutput {
            policy_logits: self.policy.forward(move_hidden.clone()).squeeze_dim::<2>(2),
            tactical_risk_logits: self.tactical.forward(move_hidden).squeeze_dim::<2>(2),
            wdl_logits: self.wdl.forward(global.clone()),
            plan_logits,
            score: self.score.forward(global),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use burn::backend::Flex;

    type TestBackend = Flex<f32>;

    fn inputs(
        device: &burn::backend::flex::FlexDevice,
    ) -> (
        Tensor<TestBackend, 3>,
        Tensor<TestBackend, 2>,
        Tensor<TestBackend, 3>,
        Tensor<TestBackend, 2>,
    ) {
        (
            Tensor::zeros([2, 324, BOARD_FEATURES], device),
            Tensor::ones([2, 324], device),
            Tensor::zeros([2, 96, MOVE_FEATURES], device),
            Tensor::zeros([2, PlanKind::COUNT], device),
        )
    }

    #[test]
    fn both_trunks_produce_identical_public_shapes() {
        let device = Default::default();
        let (board, mask, moves, plan) = inputs(&device);
        let bdh = BdhConfig::new()
            .with_width(32)
            .with_heads(2)
            .with_sparse_per_head(8)
            .with_reasoning_steps(2)
            .init(&device);
        let bdh_output = bdh.forward(board.clone(), mask.clone(), moves.clone(), plan.clone());
        let residual = ResidualConfig::new()
            .with_width(32)
            .with_layers(2)
            .with_hidden(64)
            .init(&device);
        let residual_output = residual.forward(board, mask, moves, plan);
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
        let compact = associative_retrieval(sparse, values, 5.0);
        let reference = reference.into_data().to_vec::<f32>().unwrap();
        let compact = compact.into_data().to_vec::<f32>().unwrap();

        assert_eq!(reference.len(), compact.len());
        for (reference, compact) in reference.into_iter().zip(compact) {
            assert!((reference - compact).abs() < 1.0e-5);
        }
    }
}
