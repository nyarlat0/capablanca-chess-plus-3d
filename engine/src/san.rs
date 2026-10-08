//! Canonical orthodox SAN. Legal moves are consulted internally only.
use crate::{CastleSide, Move, MoveError, MoveKind, PieceKind, Position};

impl Position {
    pub fn san(&self, mv: Move) -> Result<String, MoveError> {
        let legal = self.legal_moves();
        if !legal.contains(&mv) {
            return Err(MoveError::IllegalMove(mv));
        }
        self.san_with_legal(mv, &legal)
    }

    /// Exact canonical SAN only: no zero-castling, annotations, UCI fallback,
    /// missing check suffixes or extraction from prose.
    pub fn parse_san_move(&self, value: &str) -> Result<Move, MoveError> {
        let legal = self.legal_moves();
        for &mv in &legal {
            if self
                .san_with_legal(mv, &legal)
                .is_ok_and(|san| san == value)
            {
                return Ok(mv);
            }
        }
        Err(MoveError::IllegalNotation(value.into()))
    }

    fn san_with_legal(&self, mv: Move, legal: &[Move]) -> Result<String, MoveError> {
        let piece = self
            .board()
            .piece_at(mv.from)
            .ok_or(MoveError::IllegalMove(mv))?;
        let symbol = |kind| match kind {
            PieceKind::King => Some('K'),
            PieceKind::Queen => Some('Q'),
            PieceKind::Rook => Some('R'),
            PieceKind::Bishop => Some('B'),
            PieceKind::Knight => Some('N'),
            _ => None,
        };
        let mut text = if let MoveKind::Castle(side) = mv.kind {
            if side == CastleSide::KingSide {
                "O-O".into()
            } else {
                "O-O-O".into()
            }
        } else {
            let capture = mv.kind == MoveKind::EnPassant || self.board().piece_at(mv.to).is_some();
            let mut text = String::new();
            if piece.kind != PieceKind::Pawn {
                text.push(symbol(piece.kind).ok_or(MoveError::IllegalMove(mv))?);
                let others: Vec<_> = legal
                    .iter()
                    .filter(|other| {
                        other.from != mv.from
                            && other.to == mv.to
                            && self
                                .board()
                                .piece_at(other.from)
                                .is_some_and(|p| p == piece)
                    })
                    .collect();
                if !others.is_empty() {
                    if others.iter().all(|m| m.from.file() != mv.from.file()) {
                        text.push((b'a' + mv.from.file()) as char);
                    } else if others.iter().all(|m| m.from.rank() != mv.from.rank()) {
                        text.push_str(&(mv.from.rank() + 1).to_string());
                    } else {
                        text.push_str(&mv.from.to_string());
                    }
                }
            } else if capture {
                text.push((b'a' + mv.from.file()) as char);
            }
            if capture {
                text.push('x');
            }
            text.push_str(&mv.to.to_string());
            if let Some(promotion) = mv.promotion {
                text.push('=');
                text.push(symbol(promotion).ok_or(MoveError::IllegalMove(mv))?);
            }
            text
        };
        let after = self.after_move(mv)?;
        if after.is_in_check(after.side_to_move()) {
            text.push(if after.legal_moves().is_empty() {
                '#'
            } else {
                '+'
            });
        }
        Ok(text)
    }
}
