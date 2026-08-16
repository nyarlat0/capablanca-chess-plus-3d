use crate::capacity::square_index;
use capablanca_chess_plus::{CastleSide, Color, Position};

const KEY_SEED: u64 = 0x5445_5241_5354_4f43;

/// A deterministic analysis key. Unlike the repetition key, the half-move
/// clock is included because it can change the fifty-move terminal score.
#[must_use]
pub fn position_key(position: &Position) -> u64 {
    position_keys(position).analysis
}

#[must_use]
pub(crate) fn repetition_key(position: &Position) -> u64 {
    position_keys(position).repetition
}

#[derive(Clone, Copy)]
pub(crate) struct PositionKeys {
    pub analysis: u64,
    pub repetition: u64,
}

/// Computes both keys in one board traversal. Search visits this hot path at
/// every node, so it must not hash the (large) board twice.
#[must_use]
pub(crate) fn position_keys(position: &Position) -> PositionKeys {
    let repetition = state_key(position);
    PositionKeys {
        repetition,
        analysis: repetition
            ^ mix64(KEY_SEED ^ 0x5000_0000 ^ u64::from(position.halfmove_clock().min(100))),
    }
}

fn state_key(position: &Position) -> u64 {
    let mut key = mix64(
        KEY_SEED
            ^ u64::from(position.board().size().files())
            ^ (u64::from(position.board().size().ranks()) << 8),
    );
    for (square, piece) in position.board().pieces() {
        let piece_id = piece.kind.index() as u64 | ((piece.color.index() as u64) << 6);
        key ^= mix64(KEY_SEED ^ square_index(square) as u64 ^ (piece_id << 16));
    }
    if position.side_to_move() == Color::Black {
        key ^= mix64(KEY_SEED ^ 0x1000_0000);
    }
    for color in Color::ALL {
        for side in CastleSide::ALL {
            if position.castling_rights().has(color, side) {
                key ^= mix64(KEY_SEED ^ 0x2000_0000 ^ ((color.index() * 2 + side.index()) as u64));
            }
        }
        if position.king_jump_available(color) {
            key ^= mix64(KEY_SEED ^ 0x3000_0000 ^ color.index() as u64);
        }
    }
    if let Some(square) = position.en_passant() {
        key ^= mix64(KEY_SEED ^ 0x4000_0000 ^ square_index(square) as u64);
    }
    for byte in position.rules().name().bytes() {
        key = mix64(key ^ u64::from(byte));
    }
    key
}

#[must_use]
pub(crate) const fn mix64(mut value: u64) -> u64 {
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}
