use crate::{Color, Piece, PieceKind, Square};
use std::fmt;

const STORAGE_SQUARES: usize = 100;

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

    pub fn new(files: u8, ranks: u8) -> Result<Self, BoardError> {
        if files == 0 || ranks == 0 || files > 10 || ranks > 10 {
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
}

impl Board {
    #[must_use]
    pub fn empty(size: BoardSize) -> Self {
        Self {
            size,
            squares: [None; STORAGE_SQUARES],
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
        let old = std::mem::replace(&mut self.squares[square.storage_index()], piece);
        Ok(old)
    }

    pub(crate) fn set_piece_unchecked(&mut self, square: Square, piece: Option<Piece>) {
        self.squares[square.storage_index()] = piece;
    }

    pub fn pieces(&self) -> impl Iterator<Item = (Square, Piece)> + '_ {
        (0..self.size.ranks()).flat_map(move |rank| {
            (0..self.size.files()).filter_map(move |file| {
                let square = Square::new(file, rank);
                self.piece_at(square).map(|piece| (square, piece))
            })
        })
    }

    #[must_use]
    pub fn king_square(&self, color: Color) -> Option<Square> {
        self.pieces()
            .find(|(_, piece)| piece.color == color && piece.kind == PieceKind::King)
            .map(|(square, _)| square)
    }

    #[must_use]
    pub fn count(&self, color: Color, kind: PieceKind) -> usize {
        self.pieces()
            .filter(|(_, piece)| piece.color == color && piece.kind == kind)
            .count()
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
