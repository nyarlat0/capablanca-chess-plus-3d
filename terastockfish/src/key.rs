use crate::capacity::square_index;
use capablanca_chess_plus::{CastleSide, Color, MoveKind, Piece, Position, PositionUndo, Square};

const KEY_SEED: u64 = 0x5445_5241_5354_4f43;

/// A deterministic analysis key. Unlike the repetition key, the half-move
/// clock is included because it can change the fifty-move terminal score.
#[must_use]
pub fn position_key(position: &Position) -> u64 {
    position_keys(position).analysis
}

#[derive(Clone, Copy)]
pub(crate) struct PositionKeys {
    pub analysis: u64,
    pub repetition: u64,
    effective_en_passant: Option<Square>,
}

/// Computes both keys in one board traversal. Search visits this hot path at
/// every node, so it must not hash the (large) board twice.
#[must_use]
pub(crate) fn position_keys(position: &Position) -> PositionKeys {
    let effective_en_passant = effective_en_passant(position);
    let repetition = state_key(position, effective_en_passant);
    PositionKeys {
        repetition,
        analysis: repetition ^ halfmove_component(position.halfmove_clock()),
        effective_en_passant,
    }
}

#[must_use]
pub(crate) fn position_keys_after_move(
    position: &Position,
    previous: PositionKeys,
    undo: &PositionUndo,
) -> PositionKeys {
    let mut repetition = previous.repetition ^ side_component();
    for (square, old_piece) in undo.changed_squares() {
        if let Some(piece) = old_piece {
            repetition ^= piece_component(square, piece);
        }
        if let Some(piece) = position.board().piece_at(square) {
            repetition ^= piece_component(square, piece);
        }
    }
    for color in Color::ALL {
        for side in CastleSide::ALL {
            if undo.previous_castling_rights().has(color, side)
                != position.castling_rights().has(color, side)
            {
                repetition ^= castling_component(color, side);
            }
        }
        if undo.previous_king_jump_available(color) != position.king_jump_available(color) {
            repetition ^= king_jump_component(color);
        }
    }
    if let Some(square) = previous.effective_en_passant {
        repetition ^= en_passant_component(square);
    }
    let effective_en_passant = effective_en_passant(position);
    if let Some(square) = effective_en_passant {
        repetition ^= en_passant_component(square);
    }
    PositionKeys {
        repetition,
        analysis: repetition ^ halfmove_component(position.halfmove_clock()),
        effective_en_passant,
    }
}

fn state_key(position: &Position, effective_en_passant: Option<Square>) -> u64 {
    let mut key = mix64(
        KEY_SEED
            ^ u64::from(position.board().size().files())
            ^ (u64::from(position.board().size().ranks()) << 8),
    );
    for byte in position.rules().name().bytes() {
        key = mix64(key ^ u64::from(byte));
    }
    for (square, piece) in position.board().pieces() {
        key ^= piece_component(square, piece);
    }
    if position.side_to_move() == Color::Black {
        key ^= side_component();
    }
    for color in Color::ALL {
        for side in CastleSide::ALL {
            if position.castling_rights().has(color, side) {
                key ^= castling_component(color, side);
            }
        }
        if position.king_jump_available(color) {
            key ^= king_jump_component(color);
        }
    }
    if let Some(square) = effective_en_passant {
        key ^= en_passant_component(square);
    }
    key
}

fn effective_en_passant(position: &Position) -> Option<Square> {
    let square = position.en_passant()?;
    position
        .legal_moves()
        .iter()
        .any(|chess_move| chess_move.kind == MoveKind::EnPassant)
        .then_some(square)
}

fn piece_component(square: Square, piece: Piece) -> u64 {
    let piece_id = piece.kind.index() as u64 | ((piece.color.index() as u64) << 6);
    mix64(KEY_SEED ^ square_index(square) as u64 ^ (piece_id << 16))
}

fn side_component() -> u64 {
    mix64(KEY_SEED ^ 0x1000_0000)
}

fn castling_component(color: Color, side: CastleSide) -> u64 {
    mix64(KEY_SEED ^ 0x2000_0000 ^ ((color.index() * 2 + side.index()) as u64))
}

fn king_jump_component(color: Color) -> u64 {
    mix64(KEY_SEED ^ 0x3000_0000 ^ color.index() as u64)
}

fn en_passant_component(square: Square) -> u64 {
    mix64(KEY_SEED ^ 0x4000_0000 ^ square_index(square) as u64)
}

fn halfmove_component(halfmove_clock: u32) -> u64 {
    mix64(KEY_SEED ^ 0x5000_0000 ^ u64::from(halfmove_clock.min(100)))
}

#[must_use]
pub(crate) const fn mix64(mut value: u64) -> u64 {
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

#[cfg(test)]
mod tests {
    use super::*;
    use capablanca_chess_plus::Variant;

    fn gothic(fen: &str) -> Position {
        Position::from_fen(Variant::Gothic.rules(), fen).unwrap()
    }

    #[test]
    fn repetition_key_uses_only_a_legally_available_en_passant_capture() {
        let legal = gothic("9k/10/10/4Pp4/10/10/10/K9 w - f6 0 1");
        let legal_without_marker = gothic("9k/10/10/4Pp4/10/10/10/K9 w - - 0 1");
        assert_ne!(
            position_keys(&legal).repetition,
            position_keys(&legal_without_marker).repetition
        );

        let unavailable = gothic("9k/10/10/5p4/10/10/10/K9 w - f6 0 1");
        let unavailable_without_marker = gothic("9k/10/10/5p4/10/10/10/K9 w - - 0 1");
        assert_eq!(
            position_keys(&unavailable).repetition,
            position_keys(&unavailable_without_marker).repetition
        );
    }
}
