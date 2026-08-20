//! Plan-conditioned neural research engine for Terachess II.
//!
//! Teressa separates exact chess rules, symbolic and verbalizable plans,
//! learned policy/value inference, and tactical safety filtering. The first
//! native implementation uses Burn so training can run through Vulkan/WGPU on
//! consumer AMD hardware without coupling the rest of the workspace to a
//! vendor-specific ML runtime.

#![forbid(unsafe_code)]

pub mod advisor;
pub mod dataset;
pub mod encode;
pub mod generate;
pub mod hybrid;
pub mod model;
pub mod plan;
pub mod relabel;
pub mod training;
pub mod uci;

pub use advisor::VulkanAdvisor;
pub use dataset::{
    DatasetRecord, DatasetShardReader, DatasetShardWriter, PlanPolicyTarget, PolicyTarget,
};
pub use encode::{EncodedMove, EncodedPosition, encode_position, encode_position_uci_history};
pub use generate::{GenerationOptions, GenerationSummary, generate_dataset};
pub use hybrid::{
    HybridAgent, HybridDecision, HybridOptions, NeuralAdvice, NeuralAdvisor, PolicyCandidate,
};
pub use model::{
    BdhConfig, BdhNetwork, NetworkInput, NetworkOutput, ResidualConfig, ResidualNetwork,
    StrategyNetwork,
};
pub use plan::{
    BoardRegion, CancelCondition, PlanActor, PlanKind, PlanMethod, PlanReason, PlanState,
    PlanTarget, StrategicPlan, generate_plan_candidates,
};
pub use relabel::{RelabelOptions, RelabelSummary, relabel_dataset, relabel_game_records};
pub use training::{
    Architecture, EpochMetrics, ModelManifest, TrainingOptions, evaluate_model, load_records,
    make_batch, split_records, train_vulkan,
};

pub const MODEL_FORMAT_VERSION: &str = "TERESSA_MODEL_V3";
pub const DATASET_FORMAT_VERSION: &str = "TERESSA_DATASET_V2";
pub const LEGACY_DATASET_FORMAT_VERSION: &str = "TERESSA_DATASET_V1";
