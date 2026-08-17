//! A large-board chess analyser built around the rules supplied by
//! `capablanca-engine`.
//!
//! The current production variant is Terachess II. Search data structures use
//! 18x18-capable square indices so adding a larger rules preset does not change
//! transposition-table or move-ordering formats.

#![forbid(unsafe_code)]

mod capacity;
mod evaluate;
mod key;
pub mod profile;
mod search;
mod state;
pub mod texel;
mod tt;
pub mod uci;
pub mod validation;

pub use capacity::{MAX_BOARD_FILES, MAX_BOARD_RANKS, MAX_BOARD_SQUARES};
pub use evaluate::{EvaluationParameters, evaluate, evaluate_with, piece_value};
pub use key::position_key;
pub use profile::MaterialProfile;
pub use search::{
    AnalysisInfo, AnalysisResult, SearchControl, SearchLimits, SearchOptions, Searcher,
    is_mate_score, mate_distance,
};
pub use tt::{Bound, TranspositionTable};
