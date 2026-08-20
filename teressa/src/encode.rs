use capablanca_chess_plus::{CastleSide, Color, Move, MoveKind, PieceKind, Position, Square};
use std::str::FromStr;

pub const MAX_FILES: usize = 18;
pub const MAX_RANKS: usize = 18;
pub const BOARD_TOKENS: usize = MAX_FILES * MAX_RANKS;
pub const BOARD_FEATURES: usize = 64;
/// Twelve move-geometry/special-move values plus a relative history-age value.
/// Legal moves leave the age at zero; stored history moves use `(slot + 1) / 8`
/// so the network can distinguish move order instead of receiving an unordered
/// bag of coordinates.
pub const MOVE_FEATURES: usize = 13;
pub const HISTORY_PLIES: usize = 8;

#[derive(Clone, Debug, PartialEq)]
pub struct EncodedMove {
    pub values: [f32; MOVE_FEATURES],
    /// Indices into the fixed row-major 18x18 board-token array.
    pub from_token: usize,
    pub to_token: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct EncodedPosition {
    /// Row-major 18x18 padded tokens, each containing `BOARD_FEATURES` values.
    pub board: Vec<f32>,
    pub board_mask: Vec<f32>,
    pub history: [EncodedMove; HISTORY_PLIES],
    pub legal_moves: Vec<EncodedMove>,
}

#[must_use]
pub fn encode_position(position: &Position, history: &[Move]) -> EncodedPosition {
    let size = position.board().size();
    let side = position.side_to_move();
    let mut board = vec![0.0; BOARD_TOKENS * BOARD_FEATURES];
    let mut board_mask = vec![0.0; BOARD_TOKENS];
    let last_move = history.last().copied();
    for rank in 0..MAX_RANKS as u8 {
        for file in 0..MAX_FILES as u8 {
            let token = rank as usize * MAX_FILES + file as usize;
            if file >= size.files() || rank >= size.ranks() {
                continue;
            }
            board_mask[token] = 1.0;
            let values = &mut board[token * BOARD_FEATURES..(token + 1) * BOARD_FEATURES];
            let square = Square::new(file, rank);
            match position.board().piece_at(square) {
                Some(piece) => {
                    let relative_color = usize::from(piece.color != side);
                    values[1 + relative_color * PieceKind::COUNT + piece.kind.index()] = 1.0;
                }
                None => values[0] = 1.0,
            }
            values[53] = normalized(file, size.files());
            let relative_rank = if side == Color::White {
                rank
            } else {
                size.ranks() - 1 - rank
            };
            values[54] = normalized(relative_rank, size.ranks());
            values[55] = 1.0;
            values[56] = f32::from(position.is_square_attacked(square, side));
            values[57] = f32::from(position.is_square_attacked(square, side.opposite()));
            values[58] = f32::from(position.en_passant() == Some(square));
            values[59] = f32::from(position.king_jump_available(side));
            values[60] = f32::from(position.king_jump_available(side.opposite()));
            values[61] = (position.halfmove_clock().min(100) as f32) / 100.0;
            values[62] = f32::from(last_move.is_some_and(|chess_move| chess_move.from == square));
            values[63] = f32::from(last_move.is_some_and(|chess_move| chess_move.to == square));
        }
    }
    let empty_move = EncodedMove {
        values: [0.0; MOVE_FEATURES],
        from_token: 0,
        to_token: 0,
    };
    let mut encoded_history = std::array::from_fn(|_| empty_move.clone());
    for (slot, chess_move) in encoded_history
        .iter_mut()
        .rev()
        .zip(history.iter().rev().take(HISTORY_PLIES))
    {
        *slot = encode_move(*chess_move, size.files(), size.ranks(), side);
    }
    for (index, slot) in encoded_history.iter_mut().enumerate() {
        if slot.values[11] > 0.0 {
            slot.values[12] = (index + 1) as f32 / HISTORY_PLIES as f32;
        }
    }
    let mut legal = position.legal_moves();
    legal.sort_unstable_by_key(|chess_move| chess_move.to_uci());
    let legal_moves = legal
        .into_iter()
        .map(|chess_move| encode_move(chess_move, size.files(), size.ranks(), side))
        .collect();
    EncodedPosition {
        board,
        board_mask,
        history: encoded_history,
        legal_moves,
    }
}

/// Reconstructs the geometric part of recent UCI moves stored in dataset
/// shards. Piece identity is already present in the board tokens; history only
/// needs origin, destination, and relative displacement.
#[must_use]
pub fn encode_position_uci_history(position: &Position, history: &[String]) -> EncodedPosition {
    let moves = history
        .iter()
        .filter_map(|value| uci_geometry(value))
        .collect::<Vec<_>>();
    encode_position(position, &moves)
}

fn uci_geometry(value: &str) -> Option<Move> {
    let value = value.trim();
    let destination = value
        .char_indices()
        .skip(1)
        .find(|(_, character)| character.is_ascii_alphabetic())?
        .0;
    let destination_end = value[destination + 1..]
        .char_indices()
        .find(|(_, character)| !character.is_ascii_digit())
        .map_or(value.len(), |(index, _)| destination + 1 + index);
    let from = Square::from_str(&value[..destination]).ok()?;
    let to = Square::from_str(&value[destination..destination_end]).ok()?;
    Some(Move::normal(from, to))
}

#[must_use]
pub fn encode_move(chess_move: Move, files: u8, ranks: u8, perspective: Color) -> EncodedMove {
    let mut values = [0.0; MOVE_FEATURES];
    let from_rank = relative_rank(chess_move.from.rank(), ranks, perspective);
    let to_rank = relative_rank(chess_move.to.rank(), ranks, perspective);
    values[0] = normalized(chess_move.from.file(), files);
    values[1] = normalized(from_rank, ranks);
    values[2] = normalized(chess_move.to.file(), files);
    values[3] = normalized(to_rank, ranks);
    values[4] = normalized_delta(chess_move.to.file(), chess_move.from.file(), files);
    values[5] = normalized_delta(to_rank, from_rank, ranks);
    values[6] = f32::from(chess_move.promotion.is_some());
    values[7] = chess_move.promotion.map_or(0.0, |kind| {
        (kind.index() + 1) as f32 / PieceKind::COUNT as f32
    });
    values[8] = f32::from(chess_move.kind == MoveKind::EnPassant);
    values[9] = f32::from(chess_move.kind == MoveKind::Castle(CastleSide::QueenSide));
    values[10] = f32::from(chess_move.kind == MoveKind::Castle(CastleSide::KingSide));
    values[11] = 1.0;
    EncodedMove {
        values,
        from_token: chess_move.from.rank() as usize * MAX_FILES + chess_move.from.file() as usize,
        to_token: chess_move.to.rank() as usize * MAX_FILES + chess_move.to.file() as usize,
    }
}

const fn relative_rank(rank: u8, ranks: u8, perspective: Color) -> u8 {
    match perspective {
        Color::White => rank,
        Color::Black => ranks - 1 - rank,
    }
}

fn normalized(value: u8, extent: u8) -> f32 {
    if extent <= 1 {
        0.0
    } else {
        value as f32 / (extent - 1) as f32
    }
}

fn normalized_delta(to: u8, from: u8, extent: u8) -> f32 {
    if extent <= 1 {
        0.0
    } else {
        (i16::from(to) - i16::from(from)) as f32 / (extent - 1) as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use capablanca_chess_plus::Variant;

    #[test]
    fn terachess_encoding_is_padded_to_eighteen_by_eighteen() {
        let position = Variant::TerachessII.starting_position();
        let encoded = encode_position(&position, &[]);
        assert_eq!(encoded.board.len(), BOARD_TOKENS * BOARD_FEATURES);
        assert_eq!(
            encoded
                .board_mask
                .iter()
                .filter(|value| **value == 1.0)
                .count(),
            256
        );
        assert!(encoded.legal_moves.len() > 40);
    }

    #[test]
    fn legal_moves_are_encoded_in_stable_uci_order() {
        let position = Variant::TerachessII.starting_position();
        let first = encode_position(&position, &[]);
        let second = encode_position(&position, &[]);
        assert_eq!(first.legal_moves, second.legal_moves);
    }

    #[test]
    fn stored_uci_history_marks_the_last_move() {
        let mut position = Variant::TerachessII.starting_position();
        let chess_move = position.parse_uci_move("d2g5").unwrap();
        position.play(chess_move).unwrap();
        let encoded = encode_position_uci_history(&position, &["d2g5".to_owned()]);
        let from = (MAX_FILES + 3) * BOARD_FEATURES;
        let to = (4 * MAX_FILES + 6) * BOARD_FEATURES;
        assert_eq!(encoded.board[from + 62], 1.0);
        assert_eq!(encoded.board[to + 63], 1.0);
    }

    #[test]
    fn history_encoding_preserves_recency_and_board_endpoints() {
        let position = Variant::TerachessII.starting_position();
        let first = Move::normal(Square::new(3, 1), Square::new(6, 4));
        let second = Move::normal(Square::new(3, 14), Square::new(6, 11));
        let encoded = encode_position(&position, &[first, second]);
        assert_eq!(encoded.history[6].values[12], 7.0 / 8.0);
        assert_eq!(encoded.history[7].values[12], 1.0);
        assert_eq!(encoded.history[7].from_token, 14 * MAX_FILES + 3);
        assert_eq!(encoded.history[7].to_token, 11 * MAX_FILES + 6);
    }

    #[test]
    fn move_geometry_is_canonical_for_the_side_to_move() {
        let white = encode_move(
            Move::normal(Square::new(3, 1), Square::new(6, 4)),
            16,
            16,
            Color::White,
        );
        let black = encode_move(
            Move::normal(Square::new(3, 14), Square::new(6, 11)),
            16,
            16,
            Color::Black,
        );
        assert_eq!(&white.values[..12], &black.values[..12]);
        assert_ne!(white.from_token, black.from_token);
        assert_ne!(white.to_token, black.to_token);
    }
}
