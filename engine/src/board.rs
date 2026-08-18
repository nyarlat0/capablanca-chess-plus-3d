use crate::{Color, Piece, PieceKind, Square};
use std::fmt;

const STORAGE_FILES: usize = 18;
const STORAGE_SQUARES: usize = STORAGE_FILES * STORAGE_FILES;
const STORAGE_WORDS: usize = STORAGE_SQUARES.div_ceil(u64::BITS as usize);

/// Rectangular board dimensions supported by the engine.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct BoardSize {
    files: u8,
    ranks: u8,
}

impl BoardSize {
    pub const CAPABLANCA: Self = Self {
        files: 10,
        ranks: 8,
    };
    pub const GRAND: Self = Self {
        files: 10,
        ranks: 10,
    };
    pub const TERACHESS: Self = Self {
        files: 16,
        ranks: 16,
    };

    pub fn new(files: u8, ranks: u8) -> Result<Self, BoardError> {
        if files == 0 || ranks == 0 || files > 18 || ranks > 18 {
            return Err(BoardError::InvalidSize { files, ranks });
        }
        Ok(Self { files, ranks })
    }

    #[must_use]
    pub const fn files(self) -> u8 {
        self.files
    }

    #[must_use]
    pub const fn ranks(self) -> u8 {
        self.ranks
    }

    #[must_use]
    pub const fn contains(self, square: Square) -> bool {
        square.file() < self.files && square.rank() < self.ranks
    }
}

/// A fixed-capacity board with runtime dimensions.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct Board {
    size: BoardSize,
    squares: [Option<Piece>; STORAGE_SQUARES],
    king_squares: [Option<Square>; 2],
    occupied: [u64; STORAGE_WORDS],
    colors: [[u64; STORAGE_WORDS]; 2],
}

impl Board {
    #[must_use]
    pub fn empty(size: BoardSize) -> Self {
        Self {
            size,
            squares: [None; STORAGE_SQUARES],
            king_squares: [None; 2],
            occupied: [0; STORAGE_WORDS],
            colors: [[0; STORAGE_WORDS]; 2],
        }
    }

    #[must_use]
    pub const fn size(&self) -> BoardSize {
        self.size
    }

    #[must_use]
    pub fn piece_at(&self, square: Square) -> Option<Piece> {
        self.size
            .contains(square)
            .then(|| self.squares[square.storage_index()])
            .flatten()
    }

    pub fn set_piece(
        &mut self,
        square: Square,
        piece: Option<Piece>,
    ) -> Result<Option<Piece>, BoardError> {
        if !self.size.contains(square) {
            return Err(BoardError::SquareOutsideBoard(square));
        }
        Ok(self.replace_piece(square, piece))
    }

    pub(crate) fn set_piece_unchecked(&mut self, square: Square, piece: Option<Piece>) {
        self.replace_piece(square, piece);
    }

    pub fn pieces(&self) -> impl Iterator<Item = (Square, Piece)> + '_ {
        self.pieces_from_mask(self.occupied)
    }

    pub(crate) fn pieces_of(&self, color: Color) -> impl Iterator<Item = (Square, Piece)> + '_ {
        self.pieces_from_mask(self.colors[color.index()])
    }

    #[must_use]
    pub fn king_square(&self, color: Color) -> Option<Square> {
        self.king_squares[color.index()]
    }

    #[must_use]
    pub fn count(&self, color: Color, kind: PieceKind) -> usize {
        self.pieces()
            .filter(|(_, piece)| piece.color == color && piece.kind == kind)
            .count()
    }

    fn replace_piece(&mut self, square: Square, piece: Option<Piece>) -> Option<Piece> {
        let index = square.storage_index();
        let word = index / u64::BITS as usize;
        let bit = 1_u64 << (index % u64::BITS as usize);
        let old = std::mem::replace(&mut self.squares[index], piece);
        if let Some(old) = old {
            self.occupied[word] &= !bit;
            self.colors[old.color.index()][word] &= !bit;
            if old.kind == PieceKind::King && self.king_squares[old.color.index()] == Some(square) {
                self.king_squares[old.color.index()] = None;
            }
        }
        if let Some(piece) = piece {
            self.occupied[word] |= bit;
            self.colors[piece.color.index()][word] |= bit;
            if piece.kind == PieceKind::King {
                self.king_squares[piece.color.index()] = Some(square);
            }
        }
        old
    }

    fn pieces_from_mask(
        &self,
        mask: [u64; STORAGE_WORDS],
    ) -> impl Iterator<Item = (Square, Piece)> + '_ {
        let mut word_index = 0_usize;
        let mut remaining = mask[0];
        std::iter::from_fn(move || {
            loop {
                if remaining != 0 {
                    let offset = remaining.trailing_zeros() as usize;
                    remaining &= remaining - 1;
                    let index = word_index * u64::BITS as usize + offset;
                    let square =
                        Square::new((index % STORAGE_FILES) as u8, (index / STORAGE_FILES) as u8);
                    let piece =
                        self.squares[index].expect("occupied mask must match board storage");
                    return Some((square, piece));
                }
                word_index += 1;
                if word_index == STORAGE_WORDS {
                    return None;
                }
                remaining = mask[word_index];
            }
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BoardError {
    InvalidSize { files: u8, ranks: u8 },
    SquareOutsideBoard(Square),
}

impl fmt::Display for BoardError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSize { files, ranks } => {
                write!(formatter, "unsupported board size {files}x{ranks}")
            }
            Self::SquareOutsideBoard(square) => write!(formatter, "{square} is outside the board"),
        }
    }
}

impl std::error::Error for BoardError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn storage_supports_the_full_future_eighteen_by_eighteen_capacity() {
        let size = BoardSize::new(18, 18).unwrap();
        let mut board = Board::empty(size);
        let corner: Square = "r18".parse().unwrap();
        let piece = Piece::new(Color::White, PieceKind::Amazon);
        assert_eq!(board.set_piece(corner, Some(piece)).unwrap(), None);
        assert_eq!(board.piece_at(corner), Some(piece));
        assert_eq!(board.pieces().count(), 1);
    }

    #[test]
    fn color_piece_indexes_follow_replacements() {
        let mut board = Board::empty(BoardSize::TERACHESS);
        let square: Square = "p16".parse().unwrap();
        let white = Piece::new(Color::White, PieceKind::Amazon);
        let black = Piece::new(Color::Black, PieceKind::Troll);
        board.set_piece(square, Some(white)).unwrap();
        assert_eq!(
            board.pieces_of(Color::White).collect::<Vec<_>>(),
            [(square, white)]
        );
        assert!(board.pieces_of(Color::Black).next().is_none());
        board.set_piece(square, Some(black)).unwrap();
        assert!(board.pieces_of(Color::White).next().is_none());
        assert_eq!(
            board.pieces_of(Color::Black).collect::<Vec<_>>(),
            [(square, black)]
        );
        board.set_piece(square, None).unwrap();
        assert!(board.pieces().next().is_none());
    }
}
