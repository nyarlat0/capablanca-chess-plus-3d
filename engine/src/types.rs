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
    /// Xiangqi cannon: slides like a rook, but captures only by jumping
    /// exactly one intervening piece.
    Cannon,
    /// Elephant: leaps one or two squares diagonally.
    Elephant,
    /// Camel: a (3, 1) leaper.
    Camel,
    /// Giraffe (zebra): a (3, 2) leaper.
    Giraffe,
    /// Archer (Vao): slides diagonally and captures over one screen.
    Archer,
    /// Machine: leaps one or two squares orthogonally.
    Machine,
    /// Queen + knight.
    Amazon,
    /// King steps plus knight, alfil, and dabbaba leaps.
    Lion,
    /// Knight + camel + giraffe.
    Buffalo,
    /// Knight + non-royal king.
    Centaur,
    /// Rook + one-step diagonal moves.
    Admiral,
    /// Bishop + one-step orthogonal moves.
    Missionary,
    /// One diagonal step followed by an outward orthogonal slide.
    Eagle,
    /// One orthogonal step followed by an outward diagonal slide.
    Rhinoceros,
    /// Non-royal king with a non-capturing forward double step.
    Prince,
    /// Queen mover that captures over exactly one screen.
    Sorceress,
    /// One-to-three-square queen-direction leaper.
    Duchess,
    /// Three-square queen-direction leaper with pawn-like forward moves.
    Troll,
}

impl PieceKind {
    pub const COUNT: usize = 26;
    pub const ALL: [Self; Self::COUNT] = [
        Self::Pawn,
        Self::Knight,
        Self::Bishop,
        Self::Rook,
        Self::Queen,
        Self::King,
        Self::Archbishop,
        Self::Chancellor,
        Self::Cannon,
        Self::Elephant,
        Self::Camel,
        Self::Giraffe,
        Self::Archer,
        Self::Machine,
        Self::Amazon,
        Self::Lion,
        Self::Buffalo,
        Self::Centaur,
        Self::Admiral,
        Self::Missionary,
        Self::Eagle,
        Self::Rhinoceros,
        Self::Prince,
        Self::Sorceress,
        Self::Duchess,
        Self::Troll,
    ];

    #[must_use]
    pub const fn index(self) -> usize {
        match self {
            Self::Pawn => 0,
            Self::Knight => 1,
            Self::Bishop => 2,
            Self::Rook => 3,
            Self::Queen => 4,
            Self::King => 5,
            Self::Archbishop => 6,
            Self::Chancellor => 7,
            Self::Cannon => 8,
            Self::Elephant => 9,
            Self::Camel => 10,
            Self::Giraffe => 11,
            Self::Archer => 12,
            Self::Machine => 13,
            Self::Amazon => 14,
            Self::Lion => 15,
            Self::Buffalo => 16,
            Self::Centaur => 17,
            Self::Admiral => 18,
            Self::Missionary => 19,
            Self::Eagle => 20,
            Self::Rhinoceros => 21,
            Self::Prince => 22,
            Self::Sorceress => 23,
            Self::Duchess => 24,
            Self::Troll => 25,
        }
    }

    #[must_use]
    pub const fn from_index(index: usize) -> Option<Self> {
        if index < Self::COUNT {
            Some(Self::ALL[index])
        } else {
            None
        }
    }

    pub const PROMOTION_PIECES: [Self; 6] = [
        Self::Queen,
        Self::Chancellor,
        Self::Archbishop,
        Self::Rook,
        Self::Bishop,
        Self::Knight,
    ];

    /// Promotion choices in Shako Chess.
    pub const SHAKO_PROMOTION_PIECES: [Self; 6] = [
        Self::Queen,
        Self::Rook,
        Self::Bishop,
        Self::Knight,
        Self::Elephant,
        Self::Cannon,
    ];

    /// Promotion choices in Pemba.
    pub const PEMBA_PROMOTION_PIECES: [Self; 10] = [
        Self::Queen,
        Self::Rook,
        Self::Bishop,
        Self::Knight,
        Self::Cannon,
        Self::Elephant,
        Self::Camel,
        Self::Giraffe,
        Self::Archer,
        Self::Machine,
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
            // `c` is intentionally shared with Chancellor. Extended FEN uses
            // the variant rules to disambiguate the two established notations.
            Self::Cannon => 'c',
            Self::Elephant => 'e',
            Self::Camel => 'm',
            Self::Giraffe => 'z',
            Self::Archer => 'v',
            Self::Machine => 'w',
            Self::Amazon => 'a',
            Self::Lion => 'l',
            Self::Buffalo => 'f',
            Self::Centaur => 'j',
            Self::Admiral => 's',
            Self::Missionary => 'y',
            Self::Eagle => 'g',
            Self::Rhinoceros => 'u',
            Self::Prince => 'i',
            Self::Sorceress => 'o',
            Self::Duchess => 'd',
            Self::Troll => 't',
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
            'e' => Self::Elephant,
            'z' => Self::Giraffe,
            'v' => Self::Archer,
            'w' => Self::Machine,
            'l' => Self::Lion,
            'f' => Self::Buffalo,
            'j' => Self::Centaur,
            's' => Self::Admiral,
            'y' => Self::Missionary,
            'g' => Self::Eagle,
            'u' => Self::Rhinoceros,
            'i' => Self::Prince,
            'o' => Self::Sorceress,
            'd' => Self::Duchess,
            't' => Self::Troll,
            // `m` is accepted for Grand Chess software that writes "marshal".
            'c' | 'm' => Self::Chancellor,
            _ => return None,
        };
        Some((kind, color))
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
        self.rank as usize * 18 + self.file as usize
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
        if !(b'a'..=b'r').contains(&file) {
            return Err(ParseSquareError(value.to_owned()));
        }
        let rank = value[1..]
            .parse::<u8>()
            .ok()
            .filter(|rank| (1..=18).contains(rank))
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
    fn square_round_trip_supports_eighteen_by_eighteen_boards() {
        for value in ["a1", "j8", "e10", "p16", "r18"] {
            let square: Square = value.parse().unwrap();
            assert_eq!(square.to_string(), value);
        }
    }
}
