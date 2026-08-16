//! A reusable rules engine for Capablanca-family chess variants, Grand Chess,
//! Shako Chess, Pemba, and Terachess II.
//!
//! The crate deliberately separates immutable [`VariantRules`] from a mutable
//! [`Position`]. This makes non-standard starting arrays and their castling
//! routes data, rather than special cases in move generation.

#![forbid(unsafe_code)]

pub mod board;
pub mod game;
pub mod mv;
pub mod position;
pub mod rules;
pub mod types;

pub use board::{Board, BoardSize};
pub use game::{DrawReason, Game, GameOutcome};
pub use mv::{CastleSide, Move, MoveKind};
pub use position::{FenError, MoveError, Position, PositionUndo};
pub use rules::{
    CastleRoute, CastlingRights, CastlingRules, PromotionRule, RuleError, Variant, VariantRules,
};
pub use types::{Color, Piece, PieceKind, Square};
