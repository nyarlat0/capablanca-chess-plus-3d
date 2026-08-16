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
mod search;
mod tt;
pub mod uci;

pub use capacity::{MAX_BOARD_FILES, MAX_BOARD_RANKS, MAX_BOARD_SQUARES};
pub use evaluate::{evaluate, piece_value};
pub use key::position_key;
pub use search::{
    AnalysisInfo, AnalysisResult, SearchControl, SearchLimits, SearchOptions, Searcher,
    is_mate_score, mate_distance,
};
pub use tt::{Bound, TranspositionTable};
