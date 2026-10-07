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
    /// Absolute-coordinate movement, shared by experiment renderers. These are
    /// base predicates; royal safety and variant-specific rights still apply.
    pub fn geometric_description(self) -> String {
        let orth = "exactly one of dx,dy is zero; every intermediate square empty";
        let diag = "abs(dx)=abs(dy)>0; every intermediate square empty";
        let step = "max(abs(dx),abs(dy))=1";
        match self {
            Self::Knight | Self::Camel | Self::Giraffe | Self::Elephant | Self::Machine => self.movement_description(),
            Self::Rook => orth.into(), Self::Bishop => diag.into(),
            Self::Queen => format!("({orth}) OR ({diag})"),
            Self::King => format!("{step}; royal: may not stay in or enter check"),
            Self::Pawn => "quiet: dx=0, dy=forward, destination empty; capture: abs(dx)=1, dy=forward, enemy destination (or en passant under variant rules)".into(),
            Self::Archbishop => format!("({diag}) OR ({})", Self::Knight.geometric_description()),
            Self::Chancellor => format!("({orth}) OR ({})", Self::Knight.geometric_description()),
            Self::Amazon => format!("({}) OR ({})", Self::Queen.geometric_description(), Self::Knight.geometric_description()),
            Self::Centaur => format!("({step}) OR ({}); nonroyal", Self::Knight.geometric_description()),
            Self::Lion => format!("({step}) OR ({}) OR (max(abs(dx),abs(dy))=2 and (dx=0 or dy=0 or abs(dx)=abs(dy))); all jumps ignore intermediate pieces; nonroyal; one capture maximum", Self::Knight.geometric_description()),
            Self::Buffalo => format!("({}) OR ({}) OR ({})", Self::Knight.geometric_description(), Self::Camel.geometric_description(), Self::Giraffe.geometric_description()),
            Self::Admiral => format!("({orth}) OR (abs(dx)=abs(dy)=1)"),
            Self::Missionary => format!("({diag}) OR (abs(dx)+abs(dy)=1)"),
            Self::Cannon | Self::Archer | Self::Sorceress => {
                let ray = match self { Self::Cannon => "exactly one of dx,dy zero", Self::Archer => "abs(dx)=abs(dy)>0", _ => "dx=0 or dy=0 or abs(dx)=abs(dy), excluding (0,0)" };
                format!("ray: {ray}; quiet: no intervening occupied square; capture: enemy destination and exactly one intervening occupied square of either color; cannot land quietly beyond a screen")
            }
            Self::Duchess => "1<=max(abs(dx),abs(dy))<=3 and (dx=0 or dy=0 or abs(dx)=abs(dy)); ignores intermediate squares".into(),
            Self::Prince => format!("{step}; nonroyal; quiet forward double-step only as specified by variant rules"),
            Self::Troll => "(max(abs(dx),abs(dy))=3 and (dx=0 or dy=0 or abs(dx)=abs(dy)), ignoring intermediate squares) OR (dx=0,dy=forward, empty destination) OR (abs(dx)=1,dy=forward, enemy destination); no double-step or en-passant capture".into(),
            Self::Eagle => "choose sx,sy in {-1,+1}; first step (sx,sy), then optionally t>=1 steps in (sx,0) OR (0,sy). May stop/capture at first square. To continue, first square and every subsequent intermediate square must be empty; may capture only at destination".into(),
            Self::Rhinoceros => "choose s,u in {-1,+1}; first step (s,0), then optionally t>=1 steps in (s,u); OR first step (0,s), then optionally t>=1 steps in (u,s). May stop/capture at first square. To continue, first square and every subsequent intermediate square must be empty; may capture only at destination".into(),
        }
    }
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
