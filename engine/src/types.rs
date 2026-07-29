use std::fmt;
use std::str::FromStr;

/// A player's side.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Color {
    White,
    Black,
}

impl Color {
    pub const ALL: [Self; 2] = [Self::White, Self::Black];

    #[must_use]
    pub const fn opposite(self) -> Self {
        match self {
            Self::White => Self::Black,
            Self::Black => Self::White,
        }
    }

    #[must_use]
    pub const fn index(self) -> usize {
        match self {
            Self::White => 0,
            Self::Black => 1,
        }
    }

    #[must_use]
    pub const fn pawn_direction(self) -> i8 {
        match self {
            Self::White => 1,
            Self::Black => -1,
        }
    }
}

/// All piece types used by the supported variants.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum PieceKind {
    Pawn,
    Knight,
    Bishop,
    Rook,
    Queen,
    King,
    /// Bishop + knight. Also called cardinal, princess, or Janus.
    Archbishop,
    /// Rook + knight. Also called chancellor, marshal, or empress.
    Chancellor,
}

impl PieceKind {
    pub const PROMOTION_PIECES: [Self; 6] = [
        Self::Queen,
        Self::Chancellor,
        Self::Archbishop,
        Self::Rook,
        Self::Bishop,
        Self::Knight,
    ];

    #[must_use]
    pub const fn fen_char(self) -> char {
        match self {
            Self::Pawn => 'p',
            Self::Knight => 'n',
            Self::Bishop => 'b',
            Self::Rook => 'r',
            Self::Queen => 'q',
            Self::King => 'k',
            Self::Archbishop => 'a',
            Self::Chancellor => 'c',
        }
    }

    #[must_use]
    pub fn from_fen_char(value: char) -> Option<(Self, Color)> {
        let color = if value.is_ascii_uppercase() {
            Color::White
        } else {
            Color::Black
        };
        let kind = match value.to_ascii_lowercase() {
            'p' => Self::Pawn,
            'n' => Self::Knight,
            'b' => Self::Bishop,
            'r' => Self::Rook,
            'q' => Self::Queen,
            'k' => Self::King,
            'a' => Self::Archbishop,
            // `m` is accepted for Grand Chess software that writes "marshal".
            'c' | 'm' => Self::Chancellor,
            _ => return None,
        };
        Some((kind, color))
    }

    #[must_use]
    pub const fn material_value(self) -> i32 {
        match self {
            Self::Pawn => 100,
            Self::Knight => 320,
            Self::Bishop => 350,
            Self::Rook => 525,
            Self::Queen => 1_000,
            Self::King => 20_000,
            Self::Archbishop => 875,
            Self::Chancellor => 900,
        }
    }
}

/// A colored chess piece.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Piece {
    pub color: Color,
    pub kind: PieceKind,
}

impl Piece {
    #[must_use]
    pub const fn new(color: Color, kind: PieceKind) -> Self {
        Self { color, kind }
    }

    #[must_use]
    pub fn fen_char(self) -> char {
        let value = self.kind.fen_char();
        match self.color {
            Color::White => value.to_ascii_uppercase(),
            Color::Black => value,
        }
    }
}

/// A board coordinate. Files and ranks are zero based internally.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Square {
    file: u8,
    rank: u8,
}

impl Square {
    #[must_use]
    pub const fn new(file: u8, rank: u8) -> Self {
        Self { file, rank }
    }

    #[must_use]
    pub const fn file(self) -> u8 {
        self.file
    }

    #[must_use]
    pub const fn rank(self) -> u8 {
        self.rank
    }

    #[must_use]
    pub(crate) const fn storage_index(self) -> usize {
        self.rank as usize * 10 + self.file as usize
    }

    #[must_use]
    pub fn offset(self, file_delta: i8, rank_delta: i8) -> Option<Self> {
        let file = i16::from(self.file) + i16::from(file_delta);
        let rank = i16::from(self.rank) + i16::from(rank_delta);
        if (0..=u8::MAX.into()).contains(&file) && (0..=u8::MAX.into()).contains(&rank) {
            Some(Self::new(file as u8, rank as u8))
        } else {
            None
        }
    }
}

impl fmt::Display for Square {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let file = char::from(b'a' + self.file);
        write!(formatter, "{file}{}", self.rank + 1)
    }
}

impl FromStr for Square {
    type Err = ParseSquareError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let bytes = value.as_bytes();
        if !(2..=3).contains(&bytes.len()) || !bytes[0].is_ascii_alphabetic() {
            return Err(ParseSquareError(value.to_owned()));
        }

        let file = bytes[0].to_ascii_lowercase();
        if !(b'a'..=b'j').contains(&file) {
            return Err(ParseSquareError(value.to_owned()));
        }
        let rank = value[1..]
            .parse::<u8>()
            .ok()
            .filter(|rank| (1..=10).contains(rank))
            .ok_or_else(|| ParseSquareError(value.to_owned()))?;

        Ok(Self::new(file - b'a', rank - 1))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParseSquareError(String);

impl fmt::Display for ParseSquareError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid square: {}", self.0)
    }
}

impl std::error::Error for ParseSquareError {}

#[cfg(test)]
mod tests {
    use super::Square;

    #[test]
    fn square_round_trip_supports_rank_ten() {
        for value in ["a1", "j8", "e10"] {
            let square: Square = value.parse().unwrap();
            assert_eq!(square.to_string(), value);
        }
    }
}
