use crate::capacity::{MAX_BOARD_FILES, MAX_BOARD_SQUARES, square_index};
use capablanca_chess_plus::{CastleSide, Move, MoveKind, PieceKind, Square};
use std::mem::size_of;
use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};

const VALID_BIT: u64 = 1 << 63;
const MOVE_MASK: u64 = (1 << 25) - 1;
const SCORE_BITS: u32 = 21;
const SCORE_MASK: u64 = (1 << SCORE_BITS) - 1;
const MIN_STORED_SCORE: i32 = -(1 << (SCORE_BITS - 1));
const MAX_STORED_SCORE: i32 = (1 << (SCORE_BITS - 1)) - 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Bound {
    Exact,
    Lower,
    Upper,
}

impl Bound {
    const fn bits(self) -> u64 {
        match self {
            Self::Exact => 0,
            Self::Lower => 1,
            Self::Upper => 2,
        }
    }

    const fn from_bits(value: u64) -> Self {
        match value {
            1 => Self::Lower,
            2 => Self::Upper,
            _ => Self::Exact,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct TtData {
    pub depth: i16,
    pub score: i32,
    pub bound: Bound,
    pub best_move: Option<Move>,
    pub generation: u8,
}

#[derive(Default)]
struct AtomicEntry {
    key_xor_data: AtomicU64,
    data: AtomicU64,
}

/// A fixed-size, lock-free transposition table. Each slot uses the standard
/// key-xor-data validation scheme, allowing search threads to share it without
/// mutexes while rejecting torn reads.
pub struct TranspositionTable {
    entries: Box<[AtomicEntry]>,
    mask: usize,
    generation: AtomicU8,
}

impl TranspositionTable {
    #[must_use]
    pub fn new(megabytes: usize) -> Self {
        let requested = megabytes
            .max(1)
            .saturating_mul(1024 * 1024)
            .checked_div(size_of::<AtomicEntry>())
            .unwrap_or(1)
            .max(1);
        let capacity = floor_power_of_two(requested);
        let entries = (0..capacity)
            .map(|_| AtomicEntry::default())
            .collect::<Vec<_>>()
            .into_boxed_slice();
        Self {
            entries,
            mask: capacity - 1,
            generation: AtomicU8::new(0),
        }
    }

    #[must_use]
    pub const fn capacity(&self) -> usize {
        self.entries.len()
    }

    pub fn clear(&self) {
        for entry in &self.entries {
            entry.key_xor_data.store(0, Ordering::Relaxed);
            entry.data.store(0, Ordering::Relaxed);
        }
    }

    pub(crate) fn next_generation(&self) -> u8 {
        self.generation
            .fetch_add(1, Ordering::Relaxed)
            .wrapping_add(1)
            & 0x7f
    }

    pub(crate) fn probe(&self, key: u64) -> Option<TtData> {
        let entry = &self.entries[key as usize & self.mask];
        let data = entry.data.load(Ordering::Acquire);
        if data & VALID_BIT == 0 {
            return None;
        }
        let stored_key = entry.key_xor_data.load(Ordering::Relaxed) ^ data;
        (stored_key == key).then(|| unpack_data(data))
    }

    pub(crate) fn store(
        &self,
        key: u64,
        depth: i16,
        score: i32,
        bound: Bound,
        best_move: Option<Move>,
        generation: u8,
    ) {
        let entry = &self.entries[key as usize & self.mask];
        let old_data = entry.data.load(Ordering::Relaxed);
        if old_data & VALID_BIT != 0 {
            let old_key = entry.key_xor_data.load(Ordering::Relaxed) ^ old_data;
            let old = unpack_data(old_data);
            let exact_bonus = if bound == Bound::Exact { 1 } else { 0 };
            let replace =
                old_key == key || old.generation != generation || depth >= old.depth - exact_bonus;
            if !replace {
                return;
            }
        }

        let data = pack_data(depth, score, bound, best_move, generation);
        entry.key_xor_data.store(key ^ data, Ordering::Relaxed);
        entry.data.store(data, Ordering::Release);
    }

    /// Approximate occupancy in permille, following the UCI `hashfull` scale.
    #[must_use]
    pub fn hashfull(&self) -> u16 {
        let sample = self.entries.len().min(1_000);
        if sample == 0 {
            return 0;
        }
        let generation = self.generation.load(Ordering::Relaxed) & 0x7f;
        let used = self.entries[..sample]
            .iter()
            .filter(|entry| {
                let data = entry.data.load(Ordering::Relaxed);
                data & VALID_BIT != 0 && unpack_data(data).generation == generation
            })
            .count();
        (used * 1_000 / sample) as u16
    }
}

fn pack_data(depth: i16, score: i32, bound: Bound, best_move: Option<Move>, generation: u8) -> u64 {
    let encoded_move = best_move.map_or(0, |chess_move| encode_move(chess_move) + 1);
    let score = score.clamp(MIN_STORED_SCORE, MAX_STORED_SCORE) as u32;
    VALID_BIT
        | encoded_move
        | ((u64::from(score) & SCORE_MASK) << 25)
        | (u64::from(depth.clamp(0, 255) as u8) << 46)
        | (bound.bits() << 54)
        | (u64::from(generation & 0x7f) << 56)
}

fn unpack_data(data: u64) -> TtData {
    let encoded_move = data & MOVE_MASK;
    let raw_score = ((data >> 25) & SCORE_MASK) as i32;
    let score = (raw_score << (32 - SCORE_BITS)) >> (32 - SCORE_BITS);
    TtData {
        depth: ((data >> 46) & 0xff) as i16,
        score,
        bound: Bound::from_bits((data >> 54) & 0x3),
        best_move: (encoded_move != 0).then(|| decode_move(encoded_move - 1)),
        generation: ((data >> 56) & 0x7f) as u8,
    }
}

fn encode_move(chess_move: Move) -> u64 {
    let from = square_index(chess_move.from) as u64;
    let to = square_index(chess_move.to) as u64;
    let promotion = chess_move
        .promotion
        .map_or(0, |kind| kind.index() as u64 + 1);
    let move_kind = match chess_move.kind {
        MoveKind::Normal => 0,
        MoveKind::EnPassant => 1,
        MoveKind::Castle(CastleSide::QueenSide) => 2,
        MoveKind::Castle(CastleSide::KingSide) => 3,
    };
    from | (to << 9) | (promotion << 18) | (move_kind << 23)
}

fn decode_move(value: u64) -> Move {
    let from = decode_square((value & 0x1ff) as usize);
    let to = decode_square(((value >> 9) & 0x1ff) as usize);
    let promotion = ((value >> 18) & 0x1f) as usize;
    let kind = match (value >> 23) & 0x3 {
        1 => MoveKind::EnPassant,
        2 => MoveKind::Castle(CastleSide::QueenSide),
        3 => MoveKind::Castle(CastleSide::KingSide),
        _ => MoveKind::Normal,
    };
    Move {
        from,
        to,
        promotion: (promotion != 0)
            .then(|| PieceKind::from_index(promotion - 1).expect("encoded promotion must exist")),
        kind,
    }
}

fn decode_square(index: usize) -> Square {
    debug_assert!(index < MAX_BOARD_SQUARES);
    Square::new(
        (index % MAX_BOARD_FILES) as u8,
        (index / MAX_BOARD_FILES) as u8,
    )
}

const fn floor_power_of_two(value: usize) -> usize {
    1usize << (usize::BITS - 1 - value.leading_zeros())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn move_encoding_supports_the_last_square_of_an_eighteen_by_eighteen_board() {
        let chess_move = Move {
            from: "r18".parse().unwrap(),
            to: "a1".parse().unwrap(),
            promotion: Some(PieceKind::Amazon),
            kind: MoveKind::EnPassant,
        };
        assert_eq!(decode_move(encode_move(chess_move)), chess_move);
    }

    #[test]
    fn table_round_trips_entries_and_rejects_other_keys() {
        let table = TranspositionTable::new(1);
        let best_move = Move::normal("a1".parse().unwrap(), "b2".parse().unwrap());
        table.store(17, 9, -1_234, Bound::Lower, Some(best_move), 4);
        let hit = table.probe(17).unwrap();
        assert_eq!(hit.depth, 9);
        assert_eq!(hit.score, -1_234);
        assert_eq!(hit.bound, Bound::Lower);
        assert_eq!(hit.best_move, Some(best_move));
        assert!(table.probe(18).is_none());
    }

    #[test]
    fn table_preserves_large_board_material_and_mate_scores() {
        for score in [MAX_STORED_SCORE, MIN_STORED_SCORE, 1_000_000, -1_000_000] {
            assert_eq!(
                unpack_data(pack_data(12, score, Bound::Exact, None, 127)).score,
                score
            );
        }
    }
}
