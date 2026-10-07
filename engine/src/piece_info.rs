//! Human-readable metadata beside the rules engine. Leaper descriptions use
//! the very same offsets as move generation (not a second geometry table).
use crate::position::{
    CAMEL_OFFSETS, ELEPHANT_OFFSETS, GIRAFFE_OFFSETS, KNIGHT_OFFSETS, MACHINE_OFFSETS,
};
use crate::{Piece, PieceKind, VariantRules};

impl VariantRules {
    /// Variant-aware symbol for inventories, diagrams and serialized positions.
    pub fn piece_symbol(&self, piece: Piece) -> char {
        self.piece_fen_char(piece)
    }
}

impl PieceKind {
    pub const fn display_name(self) -> &'static str {
        match self {
            Self::Pawn => "Pawn",
            Self::Knight => "Knight",
            Self::Bishop => "Bishop",
            Self::Rook => "Rook",
            Self::Queen => "Queen",
            Self::King => "King",
            Self::Archbishop => "Archbishop",
            Self::Chancellor => "Chancellor",
            Self::Cannon => "Cannon",
            Self::Elephant => "Elephant",
            Self::Camel => "Camel",
            Self::Giraffe => "Giraffe",
            Self::Archer => "Archer",
            Self::Machine => "Machine",
            Self::Amazon => "Amazon",
            Self::Lion => "Lion",
            Self::Buffalo => "Buffalo",
            Self::Centaur => "Centaur",
            Self::Admiral => "Admiral",
            Self::Missionary => "Missionary",
            Self::Eagle => "Eagle",
            Self::Rhinoceros => "Rhinoceros",
            Self::Prince => "Prince",
            Self::Sorceress => "Sorceress",
            Self::Duchess => "Duchess",
            Self::Troll => "Troll",
        }
    }

    /// Base movement only. Promotion, pawn double-steps and royal special
    /// moves are variant rules and must be explained separately.
    pub fn movement_description(self) -> String {
        match self {
            Self::Knight => return leaps(&KNIGHT_OFFSETS),
            Self::Camel => return leaps(&CAMEL_OFFSETS),
            Self::Giraffe => return leaps(&GIRAFFE_OFFSETS),
            Self::Elephant => return leaps(&ELEPHANT_OFFSETS),
            Self::Machine => return leaps(&MACHINE_OFFSETS),
            Self::Pawn => "one empty square forward; captures one square diagonally forward",
            Self::Bishop => "diagonal slides; cannot jump blockers",
            Self::Rook => "orthogonal slides; cannot jump blockers",
            Self::Queen => "rook + bishop",
            Self::King => "royal king: one square in any direction; may not remain in or move into check",
            Self::Archbishop => "bishop + knight",
            Self::Chancellor => "rook + knight",
            Self::Cannon => "quiet rook slides with no screen; captures along an orthogonal line across exactly one intervening piece of either color; cannot land quietly beyond a screen",
            Self::Archer => "quiet bishop slides with no screen; captures along a diagonal across exactly one intervening piece of either color; cannot land quietly beyond a screen",
            Self::Amazon => "queen + knight",
            Self::Lion => "nonroyal king + knight + exact two-square orthogonal/diagonal leaps; leaps ignore blockers; one destination/capture per move",
            Self::Buffalo => "knight + camel + giraffe (see their leap geometries)",
            Self::Centaur => "nonroyal king + knight",
            Self::Admiral => "rook + one diagonal step (not a king)",
            Self::Missionary => "bishop + one orthogonal step",
            Self::Eagle => "one diagonal step, then optionally slide outward orthogonally in either direction sharing a sign with that step; may stop/capture on first square; continuation requires first square empty and cannot jump blockers",
            Self::Rhinoceros => "one orthogonal step, then optionally slide outward diagonally in either direction sharing that step's sign; may stop/capture on first square; continuation requires first square empty and cannot jump blockers",
            Self::Prince => "nonroyal king step (not the royal king); variant may add a quiet forward double-step",
            Self::Sorceress => "quiet queen slides with no screen; captures along an orthogonal/diagonal line across exactly one intervening piece of either color; cannot land quietly beyond a screen",
            Self::Duchess => "leaps exactly 1, 2 or 3 squares orthogonally or diagonally, ignoring intervening pieces",
            Self::Troll => "exact three-square orthogonal/diagonal leaps ignoring blockers, plus one empty square forward or one-square diagonal forward capture; no double-step or en-passant capture",
        }.into()
    }
}

fn leaps(offsets: &[(i8, i8)]) -> String {
    let geometry: std::collections::BTreeSet<_> =
        offsets.iter().map(|(x, y)| (x.abs(), y.abs())).collect();
    format!(
        "leaps {}; either sign for each coordinate; ignores intervening pieces",
        geometry
            .iter()
            .map(|(x, y)| format!("({x},{y})"))
            .collect::<Vec<_>>()
            .join("/")
    )
}
