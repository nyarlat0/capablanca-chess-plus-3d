use crate::board::Board;
use crate::mv::CastleSide;
use crate::{BoardSize, Color, Piece, PieceKind, Position, Square};
use std::fmt;
use std::sync::Arc;

/// The source and destination squares for one castling move.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct CastleRoute {
    pub king_from: Square,
    pub rook_from: Square,
    pub king_to: Square,
    pub rook_to: Square,
}

impl CastleRoute {
    #[must_use]
    pub const fn new(
        king_from: Square,
        rook_from: Square,
        king_to: Square,
        rook_to: Square,
    ) -> Self {
        Self {
            king_from,
            rook_from,
            king_to,
            rook_to,
        }
    }
}

/// Castling routes indexed by color and side.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct CastlingRules {
    routes: [[Option<CastleRoute>; 2]; 2],
}

impl CastlingRules {
    /// Creates an arbitrary set of routes. Each array is ordered queen side,
    /// then king side.
    #[must_use]
    pub const fn new(white: [Option<CastleRoute>; 2], black: [Option<CastleRoute>; 2]) -> Self {
        Self {
            routes: [white, black],
        }
    }

    #[must_use]
    pub const fn none() -> Self {
        Self {
            routes: [[None; 2]; 2],
        }
    }

    #[must_use]
    pub const fn mirrored(ranks: u8, queen_side: CastleRoute, king_side: CastleRoute) -> Self {
        let last_rank = ranks.saturating_sub(1);
        Self {
            routes: [
                [Some(queen_side), Some(king_side)],
                [
                    Some(mirror_route(queen_side, last_rank)),
                    Some(mirror_route(king_side, last_rank)),
                ],
            ],
        }
    }

    #[must_use]
    pub const fn route(&self, color: Color, side: CastleSide) -> Option<CastleRoute> {
        self.routes[color.index()][side.index()]
    }

    #[must_use]
    pub fn any(&self) -> bool {
        self.routes.iter().flatten().any(Option::is_some)
    }
}

const fn mirror_route(route: CastleRoute, rank: u8) -> CastleRoute {
    CastleRoute::new(
        Square::new(route.king_from.file(), rank),
        Square::new(route.rook_from.file(), rank),
        Square::new(route.king_to.file(), rank),
        Square::new(route.rook_to.file(), rank),
    )
}

/// The castling rights retained by a position.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct CastlingRights(u8);

impl CastlingRights {
    const WHITE_QUEEN: u8 = 1 << 0;
    const WHITE_KING: u8 = 1 << 1;
    const BLACK_QUEEN: u8 = 1 << 2;
    const BLACK_KING: u8 = 1 << 3;

    #[must_use]
    pub const fn none() -> Self {
        Self(0)
    }

    #[must_use]
    pub fn from_rules(rules: &CastlingRules) -> Self {
        let mut rights = Self::none();
        for color in Color::ALL {
            for side in CastleSide::ALL {
                if rules.route(color, side).is_some() {
                    rights.set(color, side, true);
                }
            }
        }
        rights
    }

    #[must_use]
    pub const fn has(self, color: Color, side: CastleSide) -> bool {
        self.0 & right_bit(color, side) != 0
    }

    pub fn set(&mut self, color: Color, side: CastleSide, enabled: bool) {
        let bit = right_bit(color, side);
        if enabled {
            self.0 |= bit;
        } else {
            self.0 &= !bit;
        }
    }

    pub fn clear_color(&mut self, color: Color) {
        self.set(color, CastleSide::QueenSide, false);
        self.set(color, CastleSide::KingSide, false);
    }
}

const fn right_bit(color: Color, side: CastleSide) -> u8 {
    match (color, side) {
        (Color::White, CastleSide::QueenSide) => CastlingRights::WHITE_QUEEN,
        (Color::White, CastleSide::KingSide) => CastlingRights::WHITE_KING,
        (Color::Black, CastleSide::QueenSide) => CastlingRights::BLACK_QUEEN,
        (Color::Black, CastleSide::KingSide) => CastlingRights::BLACK_KING,
    }
}

/// Promotion behavior for a variant.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum PromotionRule {
    /// Promotion is mandatory on the last rank to any listed piece.
    LastRank { choices: Vec<PieceKind> },
    /// Grand Chess: optional in the last three ranks, mandatory on the last,
    /// and restricted to captured original pieces.
    Grand,
}

/// Immutable rules and initial material for a chess variant.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VariantRules {
    name: String,
    board_size: BoardSize,
    starting_board: Board,
    pawn_start_ranks: [u8; 2],
    castling: CastlingRules,
    promotion: PromotionRule,
    initial_material: [[u8; 8]; 2],
}

impl VariantRules {
    /// Creates a conventional Capablanca-family variant from a mirrored back
    /// rank. When enabled, castling moves the king three files toward either
    /// corner rook and places that rook next to the king.
    pub fn capablanca_family(
        name: impl Into<String>,
        back_rank: [PieceKind; 10],
        castling_enabled: bool,
    ) -> Result<Self, RuleError> {
        let king_file = exactly_one_file(&back_rank, PieceKind::King)?;
        let rook_files: Vec<_> = back_rank
            .iter()
            .enumerate()
            .filter_map(|(file, kind)| (*kind == PieceKind::Rook).then_some(file as u8))
            .collect();
        if rook_files.len() != 2 || rook_files[0] >= king_file || rook_files[1] <= king_file {
            return Err(RuleError::KingMustBeBetweenTwoRooks);
        }

        let castling = if castling_enabled {
            let queen_king_file = king_file
                .checked_sub(3)
                .ok_or(RuleError::InvalidCastlingTarget)?;
            let king_king_file = king_file
                .checked_add(3)
                .filter(|file| *file < 10)
                .ok_or(RuleError::InvalidCastlingTarget)?;
            CastlingRules::mirrored(
                8,
                CastleRoute::new(
                    Square::new(king_file, 0),
                    Square::new(rook_files[0], 0),
                    Square::new(queen_king_file, 0),
                    Square::new(queen_king_file + 1, 0),
                ),
                CastleRoute::new(
                    Square::new(king_file, 0),
                    Square::new(rook_files[1], 0),
                    Square::new(king_king_file, 0),
                    Square::new(king_king_file - 1, 0),
                ),
            )
        } else {
            CastlingRules::none()
        };

        let size = BoardSize::CAPABLANCA;
        let mut board = Board::empty(size);
        for (file, kind) in back_rank.into_iter().enumerate() {
            board.set_piece_unchecked(
                Square::new(file as u8, 0),
                Some(Piece::new(Color::White, kind)),
            );
            board.set_piece_unchecked(
                Square::new(file as u8, 7),
                Some(Piece::new(Color::Black, kind)),
            );
            board.set_piece_unchecked(
                Square::new(file as u8, 1),
                Some(Piece::new(Color::White, PieceKind::Pawn)),
            );
            board.set_piece_unchecked(
                Square::new(file as u8, 6),
                Some(Piece::new(Color::Black, PieceKind::Pawn)),
            );
        }

        Self::from_parts(
            name.into(),
            board,
            [1, 6],
            castling,
            PromotionRule::LastRank {
                choices: PieceKind::PROMOTION_PIECES.to_vec(),
            },
        )
    }

    fn grand() -> Self {
        let size = BoardSize::GRAND;
        let mut board = Board::empty(size);
        let inner = [
            PieceKind::Knight,
            PieceKind::Bishop,
            PieceKind::Queen,
            PieceKind::King,
            PieceKind::Chancellor,
            PieceKind::Archbishop,
            PieceKind::Bishop,
            PieceKind::Knight,
        ];

        for color in Color::ALL {
            let (rook_rank, piece_rank, pawn_rank) = match color {
                Color::White => (0, 1, 2),
                Color::Black => (9, 8, 7),
            };
            for file in 0..10 {
                board.set_piece_unchecked(
                    Square::new(file, pawn_rank),
                    Some(Piece::new(color, PieceKind::Pawn)),
                );
            }
            for file in [0, 9] {
                board.set_piece_unchecked(
                    Square::new(file, rook_rank),
                    Some(Piece::new(color, PieceKind::Rook)),
                );
            }
            for (offset, kind) in inner.into_iter().enumerate() {
                board.set_piece_unchecked(
                    Square::new(offset as u8 + 1, piece_rank),
                    Some(Piece::new(color, kind)),
                );
            }
        }

        Self::from_parts(
            "Grand Chess".to_owned(),
            board,
            [2, 7],
            CastlingRules::none(),
            PromotionRule::Grand,
        )
        .expect("built-in Grand Chess rules must be valid")
    }

    /// Builds fully custom rules. This is intended for applications that need
    /// positions beyond the built-in presets.
    pub fn custom(
        name: impl Into<String>,
        starting_board: Board,
        pawn_start_ranks: [u8; 2],
        castling: CastlingRules,
        promotion: PromotionRule,
    ) -> Result<Self, RuleError> {
        Self::from_parts(
            name.into(),
            starting_board,
            pawn_start_ranks,
            castling,
            promotion,
        )
    }

    fn from_parts(
        name: String,
        starting_board: Board,
        pawn_start_ranks: [u8; 2],
        castling: CastlingRules,
        promotion: PromotionRule,
    ) -> Result<Self, RuleError> {
        if name.trim().is_empty() {
            return Err(RuleError::EmptyName);
        }
        let size = starting_board.size();
        for color in Color::ALL {
            if starting_board.count(color, PieceKind::King) != 1 {
                return Err(RuleError::ExactlyOneKing(color));
            }
            if pawn_start_ranks[color.index()] >= size.ranks() {
                return Err(RuleError::InvalidPawnStartRank {
                    color,
                    rank: pawn_start_ranks[color.index()],
                });
            }
            for side in CastleSide::ALL {
                if let Some(route) = castling.route(color, side) {
                    validate_route(size, color, route, &starting_board)?;
                }
            }
        }
        if let PromotionRule::LastRank { choices } = &promotion
            && (choices.is_empty()
                || choices
                    .iter()
                    .any(|kind| matches!(kind, PieceKind::Pawn | PieceKind::King)))
        {
            return Err(RuleError::InvalidPromotionChoices);
        }

        let mut initial_material = [[0; 8]; 2];
        for (_, piece) in starting_board.pieces() {
            initial_material[piece.color.index()][piece_index(piece.kind)] += 1;
        }

        Ok(Self {
            name,
            board_size: size,
            starting_board,
            pawn_start_ranks,
            castling,
            promotion,
            initial_material,
        })
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub const fn board_size(&self) -> BoardSize {
        self.board_size
    }

    #[must_use]
    pub const fn pawn_start_rank(&self, color: Color) -> u8 {
        self.pawn_start_ranks[color.index()]
    }

    #[must_use]
    pub const fn castling(&self) -> &CastlingRules {
        &self.castling
    }

    #[must_use]
    pub const fn promotion(&self) -> &PromotionRule {
        &self.promotion
    }

    #[must_use]
    pub(crate) fn initial_count(&self, color: Color, kind: PieceKind) -> u8 {
        self.initial_material[color.index()][piece_index(kind)]
    }

    #[must_use]
    pub fn starting_position(self: &Arc<Self>) -> Position {
        Position::from_starting_board(Arc::clone(self), self.starting_board.clone())
    }

    #[must_use]
    pub fn into_starting_position(self) -> Position {
        Arc::new(self).starting_position()
    }
}

fn validate_route(
    size: BoardSize,
    color: Color,
    route: CastleRoute,
    board: &Board,
) -> Result<(), RuleError> {
    let squares = [
        route.king_from,
        route.rook_from,
        route.king_to,
        route.rook_to,
    ];
    if squares.iter().any(|square| !size.contains(*square))
        || squares
            .iter()
            .any(|square| square.rank() != route.king_from.rank())
    {
        return Err(RuleError::InvalidCastlingRoute { color });
    }
    if board.piece_at(route.king_from) != Some(Piece::new(color, PieceKind::King))
        || board.piece_at(route.rook_from) != Some(Piece::new(color, PieceKind::Rook))
    {
        return Err(RuleError::InvalidCastlingRoute { color });
    }
    Ok(())
}

fn exactly_one_file(back_rank: &[PieceKind; 10], kind: PieceKind) -> Result<u8, RuleError> {
    let files: Vec<_> = back_rank
        .iter()
        .enumerate()
        .filter_map(|(file, candidate)| (*candidate == kind).then_some(file as u8))
        .collect();
    match files.as_slice() {
        [file] => Ok(*file),
        _ => Err(RuleError::ExactlyOneBackRankPiece(kind)),
    }
}

pub(crate) const fn piece_index(kind: PieceKind) -> usize {
    match kind {
        PieceKind::Pawn => 0,
        PieceKind::Knight => 1,
        PieceKind::Bishop => 2,
        PieceKind::Rook => 3,
        PieceKind::Queen => 4,
        PieceKind::King => 5,
        PieceKind::Archbishop => 6,
        PieceKind::Chancellor => 7,
    }
}

/// Built-in variants.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Variant {
    Capablanca,
    Gothic,
    Embassy,
    Schoolbook,
    Bird,
    /// Carrera's historical rules, which do not include castling.
    Carrera,
    Grand,
}

impl Variant {
    pub const ALL: [Self; 7] = [
        Self::Capablanca,
        Self::Gothic,
        Self::Embassy,
        Self::Schoolbook,
        Self::Bird,
        Self::Carrera,
        Self::Grand,
    ];

    #[must_use]
    pub fn rules(self) -> VariantRules {
        use PieceKind::{Archbishop as A, Bishop as B, Chancellor as C};
        use PieceKind::{King as K, Knight as N, Queen as Q, Rook as R};

        match self {
            Self::Capablanca => VariantRules::capablanca_family(
                "Capablanca Chess",
                [R, N, A, B, Q, K, B, C, N, R],
                true,
            ),
            Self::Gothic => VariantRules::capablanca_family(
                "Gothic Chess",
                [R, N, B, Q, C, K, A, B, N, R],
                true,
            ),
            Self::Embassy => VariantRules::capablanca_family(
                "Embassy Chess",
                [R, N, B, Q, K, C, A, B, N, R],
                true,
            ),
            Self::Schoolbook => VariantRules::capablanca_family(
                "Schoolbook Chess",
                [R, Q, N, B, A, K, B, N, C, R],
                true,
            ),
            Self::Bird => VariantRules::capablanca_family(
                "Bird's Chess",
                [R, N, B, C, Q, K, A, B, N, R],
                true,
            ),
            Self::Carrera => VariantRules::capablanca_family(
                "Carrera's Chess",
                [R, C, N, B, K, Q, B, N, A, R],
                false,
            ),
            Self::Grand => return VariantRules::grand(),
        }
        .expect("built-in variant rules must be valid")
    }

    #[must_use]
    pub fn starting_position(self) -> Position {
        self.rules().into_starting_position()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RuleError {
    EmptyName,
    ExactlyOneBackRankPiece(PieceKind),
    KingMustBeBetweenTwoRooks,
    InvalidCastlingTarget,
    ExactlyOneKing(Color),
    InvalidPawnStartRank { color: Color, rank: u8 },
    InvalidCastlingRoute { color: Color },
    InvalidPromotionChoices,
}

impl fmt::Display for RuleError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyName => formatter.write_str("variant name cannot be empty"),
            Self::ExactlyOneBackRankPiece(kind) => {
                write!(formatter, "back rank must contain exactly one {kind:?}")
            }
            Self::KingMustBeBetweenTwoRooks => {
                formatter.write_str("the king must be between exactly two rooks")
            }
            Self::InvalidCastlingTarget => {
                formatter.write_str("three-square castling target is outside the board")
            }
            Self::ExactlyOneKing(color) => {
                write!(
                    formatter,
                    "starting board must contain exactly one {color:?} king"
                )
            }
            Self::InvalidPawnStartRank { color, rank } => {
                write!(formatter, "invalid {color:?} pawn start rank {}", rank + 1)
            }
            Self::InvalidCastlingRoute { color } => {
                write!(formatter, "invalid {color:?} castling route")
            }
            Self::InvalidPromotionChoices => formatter.write_str("invalid promotion choices"),
        }
    }
}

impl std::error::Error for RuleError {}
