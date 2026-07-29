use crate::{PieceKind, Square};
use std::fmt;

/// The rook side involved in castling.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum CastleSide {
    /// The rook on the lower-file side of the king.
    QueenSide,
    /// The rook on the higher-file side of the king.
    KingSide,
}

impl CastleSide {
    pub const ALL: [Self; 2] = [Self::QueenSide, Self::KingSide];

    #[must_use]
    pub const fn index(self) -> usize {
        match self {
            Self::QueenSide => 0,
            Self::KingSide => 1,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum MoveKind {
    Normal,
    EnPassant,
    Castle(CastleSide),
}

/// A fully resolved move.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Move {
    pub from: Square,
    pub to: Square,
    pub promotion: Option<PieceKind>,
    pub kind: MoveKind,
}

impl Move {
    #[must_use]
    pub const fn normal(from: Square, to: Square) -> Self {
        Self {
            from,
            to,
            promotion: None,
            kind: MoveKind::Normal,
        }
    }

    #[must_use]
    pub const fn promotion(from: Square, to: Square, promotion: PieceKind) -> Self {
        Self {
            from,
            to,
            promotion: Some(promotion),
            kind: MoveKind::Normal,
        }
    }

    #[must_use]
    pub fn to_uci(self) -> String {
        let mut value = format!("{}{}", self.from, self.to);
        if let Some(promotion) = self.promotion {
            value.push(promotion.fen_char());
        }
        value
    }
}

impl fmt::Display for Move {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.to_uci())
    }
}
