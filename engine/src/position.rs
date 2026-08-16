use crate::mv::{CastleSide, Move, MoveKind};
use crate::rules::{
    CastleRoute, CastlingRights, PromotionRule, VariantRules, terachess_promotion_target,
};
use crate::{Board, Color, Piece, PieceKind, Square};
use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

const ORTHOGONAL_DIRECTIONS: [(i8, i8); 4] = [(1, 0), (-1, 0), (0, 1), (0, -1)];
const DIAGONAL_DIRECTIONS: [(i8, i8); 4] = [(1, 1), (1, -1), (-1, 1), (-1, -1)];
const KNIGHT_OFFSETS: [(i8, i8); 8] = [
    (1, 2),
    (2, 1),
    (2, -1),
    (1, -2),
    (-1, -2),
    (-2, -1),
    (-2, 1),
    (-1, 2),
];
const ELEPHANT_OFFSETS: [(i8, i8); 8] = [
    (1, 1),
    (1, -1),
    (-1, 1),
    (-1, -1),
    (2, 2),
    (2, -2),
    (-2, 2),
    (-2, -2),
];
const CAMEL_OFFSETS: [(i8, i8); 8] = [
    (1, 3),
    (3, 1),
    (3, -1),
    (1, -3),
    (-1, -3),
    (-3, -1),
    (-3, 1),
    (-1, 3),
];
const GIRAFFE_OFFSETS: [(i8, i8); 8] = [
    (2, 3),
    (3, 2),
    (3, -2),
    (2, -3),
    (-2, -3),
    (-3, -2),
    (-3, 2),
    (-2, 3),
];
const MACHINE_OFFSETS: [(i8, i8); 8] = [
    (1, 0),
    (-1, 0),
    (0, 1),
    (0, -1),
    (2, 0),
    (-2, 0),
    (0, 2),
    (0, -2),
];

/// A complete game position under one immutable set of variant rules.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Position {
    rules: Arc<VariantRules>,
    board: Board,
    side_to_move: Color,
    castling_rights: CastlingRights,
    king_jump_rights: [bool; 2],
    en_passant: Option<Square>,
    halfmove_clock: u32,
    fullmove_number: u32,
}

const MAX_CHANGED_MOVE_SQUARES: usize = 4;

#[derive(Clone, Copy, Debug)]
struct SquareSnapshot {
    square: Square,
    piece: Option<Piece>,
}

/// Compact state needed to restore a position after one generated move.
///
/// Search engines should keep this value on their recursion stack and pass it
/// back to [`Position::unmake_move`] after examining the child position.
#[derive(Clone, Copy, Debug)]
pub struct PositionUndo {
    squares: [Option<SquareSnapshot>; MAX_CHANGED_MOVE_SQUARES],
    castling_rights: CastlingRights,
    king_jump_rights: [bool; 2],
    en_passant: Option<Square>,
    halfmove_clock: u32,
    fullmove_number: u32,
    side_to_move: Color,
}

impl PositionUndo {
    fn new(position: &Position) -> Self {
        Self {
            squares: [None; MAX_CHANGED_MOVE_SQUARES],
            castling_rights: position.castling_rights,
            king_jump_rights: position.king_jump_rights,
            en_passant: position.en_passant,
            halfmove_clock: position.halfmove_clock,
            fullmove_number: position.fullmove_number,
            side_to_move: position.side_to_move,
        }
    }

    fn record_square(&mut self, board: &Board, square: Square) {
        if self
            .squares
            .iter()
            .flatten()
            .any(|snapshot| snapshot.square == square)
        {
            return;
        }
        let slot = self
            .squares
            .iter_mut()
            .find(|snapshot| snapshot.is_none())
            .expect("a move changes at most four board squares");
        *slot = Some(SquareSnapshot {
            square,
            piece: board.piece_at(square),
        });
    }

    /// Squares changed by the move, paired with their contents before it.
    pub fn changed_squares(&self) -> impl Iterator<Item = (Square, Option<Piece>)> + '_ {
        self.squares
            .iter()
            .flatten()
            .map(|snapshot| (snapshot.square, snapshot.piece))
    }

    #[must_use]
    pub const fn previous_castling_rights(&self) -> CastlingRights {
        self.castling_rights
    }

    #[must_use]
    pub const fn previous_king_jump_available(&self, color: Color) -> bool {
        self.king_jump_rights[color.index()]
    }

    #[must_use]
    pub const fn previous_en_passant(&self) -> Option<Square> {
        self.en_passant
    }
}

impl Position {
    pub(crate) fn from_starting_board(rules: Arc<VariantRules>, board: Board) -> Self {
        Self {
            castling_rights: CastlingRights::from_rules(rules.castling()),
            king_jump_rights: [rules.king_initial_jump(); 2],
            rules,
            board,
            side_to_move: Color::White,
            en_passant: None,
            halfmove_clock: 0,
            fullmove_number: 1,
        }
    }

    /// Parses extended FEN. `A` denotes an archbishop/cardinal, `E` an
    /// elephant, and `C` is resolved as a cannon or Capablanca chancellor from
    /// the supplied rules. In Pemba, `M`, `Z`, `V`, and `W` denote camel,
    /// giraffe, archer, and machine; otherwise `M` remains accepted as a
    /// chancellor/marshal alias. Terachess II uses the piece letters from its
    /// reference diagram and `J`/`j` in the rights field for White's/Black's
    /// still-available initial king jump.
    pub fn from_fen(rules: impl Into<Arc<VariantRules>>, fen: &str) -> Result<Self, FenError> {
        let rules = rules.into();
        let fields: Vec<_> = fen.split_whitespace().collect();
        if fields.len() != 6 {
            return Err(FenError::FieldCount(fields.len()));
        }

        let size = rules.board_size();
        let rank_fields: Vec<_> = fields[0].split('/').collect();
        if rank_fields.len() != usize::from(size.ranks()) {
            return Err(FenError::RankCount(rank_fields.len()));
        }
        let mut board = Board::empty(size);
        for (fen_rank, contents) in rank_fields.into_iter().enumerate() {
            let rank = size.ranks() - 1 - fen_rank as u8;
            let mut file = 0_u8;
            let mut chars = contents.chars().peekable();
            while let Some(value) = chars.next() {
                if value.is_ascii_digit() {
                    let mut empty = value.to_digit(10).unwrap();
                    while chars.peek().is_some_and(char::is_ascii_digit) {
                        empty = empty * 10 + chars.next().unwrap().to_digit(10).unwrap();
                    }
                    if empty == 0 || empty > u32::from(size.files()) {
                        return Err(FenError::InvalidPlacement(contents.to_owned()));
                    }
                    file = file
                        .checked_add(empty as u8)
                        .ok_or_else(|| FenError::InvalidPlacement(contents.to_owned()))?;
                    continue;
                }

                let (kind, color) = rules
                    .piece_from_fen_char(value)
                    .ok_or(FenError::UnknownPiece(value))?;
                if file >= size.files() {
                    return Err(FenError::InvalidPlacement(contents.to_owned()));
                }
                board.set_piece_unchecked(Square::new(file, rank), Some(Piece::new(color, kind)));
                file += 1;
            }
            if file != size.files() {
                return Err(FenError::InvalidPlacement(contents.to_owned()));
            }
        }

        for color in Color::ALL {
            let count = board.count(color, PieceKind::King);
            if count != 1 {
                return Err(FenError::KingCount { color, count });
            }
        }

        let side_to_move = match fields[1] {
            "w" => Color::White,
            "b" => Color::Black,
            value => return Err(FenError::InvalidSide(value.to_owned())),
        };
        let (castling_rights, king_jump_rights) = parse_position_rights(&rules, fields[2])?;
        for color in Color::ALL {
            for side in CastleSide::ALL {
                if !castling_rights.has(color, side) {
                    continue;
                }
                let route = rules
                    .castling()
                    .route(color, side)
                    .expect("parsed castling right must have a route");
                if board.piece_at(route.king_from) != Some(Piece::new(color, PieceKind::King))
                    || board.piece_at(route.rook_from) != Some(Piece::new(color, PieceKind::Rook))
                {
                    return Err(FenError::InvalidCastling(fields[2].to_owned()));
                }
            }
        }
        for color in Color::ALL {
            if king_jump_rights[color.index()]
                && board.king_square(color) != rules.initial_king_square(color)
            {
                return Err(FenError::InvalidCastling(fields[2].to_owned()));
            }
        }
        let en_passant = if fields[3] == "-" {
            None
        } else {
            let square = fields[3]
                .parse::<Square>()
                .map_err(|_| FenError::InvalidEnPassant(fields[3].to_owned()))?;
            if !size.contains(square) || board.piece_at(square).is_some() {
                return Err(FenError::InvalidEnPassant(fields[3].to_owned()));
            }
            Some(square)
        };
        let halfmove_clock = fields[4]
            .parse()
            .map_err(|_| FenError::InvalidClock(fields[4].to_owned()))?;
        let fullmove_number = fields[5]
            .parse()
            .ok()
            .filter(|number| *number > 0)
            .ok_or_else(|| FenError::InvalidClock(fields[5].to_owned()))?;

        Ok(Self {
            rules,
            board,
            side_to_move,
            castling_rights,
            king_jump_rights,
            en_passant,
            halfmove_clock,
            fullmove_number,
        })
    }

    #[must_use]
    pub fn rules(&self) -> &VariantRules {
        &self.rules
    }

    #[must_use]
    pub fn board(&self) -> &Board {
        &self.board
    }

    #[must_use]
    pub const fn side_to_move(&self) -> Color {
        self.side_to_move
    }

    #[must_use]
    pub const fn castling_rights(&self) -> CastlingRights {
        self.castling_rights
    }

    /// Whether this side still has Terachess II's one-time initial king jump.
    #[must_use]
    pub const fn king_jump_available(&self, color: Color) -> bool {
        self.king_jump_rights[color.index()]
    }

    #[must_use]
    pub const fn en_passant(&self) -> Option<Square> {
        self.en_passant
    }

    #[must_use]
    pub const fn halfmove_clock(&self) -> u32 {
        self.halfmove_clock
    }

    #[must_use]
    pub const fn fullmove_number(&self) -> u32 {
        self.fullmove_number
    }

    #[must_use]
    pub fn is_in_check(&self, color: Color) -> bool {
        self.board
            .king_square(color)
            .is_some_and(|king| self.is_square_attacked(king, color.opposite()))
    }

    #[must_use]
    pub fn is_square_attacked(&self, square: Square, by: Color) -> bool {
        is_square_attacked_on(&self.board, square, by)
    }

    /// Generates every legal move for the side to move.
    #[must_use]
    pub fn legal_moves(&self) -> Vec<Move> {
        let mut position = self.clone();
        position.legal_moves_mut()
    }

    /// Generates every legal move while reusing this position as temporary
    /// make/unmake storage. The position is restored before returning.
    #[must_use]
    pub fn legal_moves_mut(&mut self) -> Vec<Move> {
        let moving_color = self.side_to_move;
        let king_square = self
            .board
            .king_square(moving_color)
            .expect("a valid position must contain its moving side's king");
        let mut pseudo = Vec::with_capacity(96);
        for (from, piece) in self.board.pieces() {
            if piece.color == moving_color {
                self.generate_piece_moves(from, piece, &mut pseudo);
            }
        }

        let mut legal = Vec::with_capacity(pseudo.len());
        for chess_move in pseudo {
            let moving_piece = self
                .board
                .piece_at(chess_move.from)
                .expect("generated move must have a moving piece");
            let undo = self.make_move_unchecked(chess_move);
            let king_after = if moving_piece.kind == PieceKind::King {
                chess_move.to
            } else {
                king_square
            };
            if !is_square_attacked_on(&self.board, king_after, moving_color.opposite()) {
                legal.push(chess_move);
            }
            self.unmake_move(undo);
        }
        legal
    }

    /// Counts leaf positions to a fixed depth. This is primarily useful for
    /// validating integrations and move-generation changes.
    #[must_use]
    pub fn perft(&self, depth: u8) -> u64 {
        let mut position = self.clone();
        position.perft_mut(depth)
    }

    #[must_use]
    pub fn is_checkmate(&self) -> bool {
        self.is_in_check(self.side_to_move) && self.legal_moves().is_empty()
    }

    #[must_use]
    pub fn is_stalemate(&self) -> bool {
        !self.is_in_check(self.side_to_move) && self.legal_moves().is_empty()
    }

    /// Resolves coordinate notation against the current legal move list.
    pub fn parse_uci_move(&self, value: &str) -> Result<Move, MoveError> {
        let normalized = value.trim().to_ascii_lowercase();
        self.legal_moves()
            .into_iter()
            .find(|chess_move| chess_move.to_uci() == normalized)
            .ok_or_else(|| MoveError::IllegalNotation(value.to_owned()))
    }

    pub fn play_uci(&mut self, value: &str) -> Result<Move, MoveError> {
        let chess_move = self.parse_uci_move(value)?;
        self.apply_unchecked(chess_move);
        Ok(chess_move)
    }

    pub fn play(&mut self, chess_move: Move) -> Result<(), MoveError> {
        if !self.legal_moves().contains(&chess_move) {
            return Err(MoveError::IllegalMove(chess_move));
        }
        self.apply_unchecked(chess_move);
        Ok(())
    }

    pub fn after_move(&self, chess_move: Move) -> Result<Self, MoveError> {
        let mut next = self.clone();
        next.play(chess_move)?;
        Ok(next)
    }

    /// Applies a move previously obtained from [`Self::legal_moves`] without
    /// regenerating that list. Search engines can use this to avoid validating
    /// the same move twice. Passing any other move is a logic error.
    #[must_use]
    pub fn after_legal_move(&self, chess_move: Move) -> Self {
        self.after_move_unchecked(chess_move)
    }

    /// Applies a move previously obtained from [`Self::legal_moves`] and
    /// returns the compact delta required to restore the exact parent state.
    /// This avoids cloning a large-board position at every search node.
    #[must_use]
    pub fn make_move(&mut self, chess_move: Move) -> PositionUndo {
        self.make_move_unchecked(chess_move)
    }

    /// Restores the exact position from before the corresponding
    /// [`Self::make_move`] call.
    pub fn unmake_move(&mut self, undo: PositionUndo) {
        for (square, piece) in undo.changed_squares() {
            self.board.set_piece_unchecked(square, piece);
        }
        self.castling_rights = undo.castling_rights;
        self.king_jump_rights = undo.king_jump_rights;
        self.en_passant = undo.en_passant;
        self.halfmove_clock = undo.halfmove_clock;
        self.fullmove_number = undo.fullmove_number;
        self.side_to_move = undo.side_to_move;
    }

    pub(crate) fn after_move_unchecked(&self, chess_move: Move) -> Self {
        let mut next = self.clone();
        next.apply_unchecked(chess_move);
        next
    }

    fn perft_mut(&mut self, depth: u8) -> u64 {
        if depth == 0 {
            return 1;
        }
        let moves = self.legal_moves_mut();
        let mut nodes = 0_u64;
        for chess_move in moves {
            let undo = self.make_move_unchecked(chess_move);
            nodes = nodes.saturating_add(self.perft_mut(depth - 1));
            self.unmake_move(undo);
        }
        nodes
    }

    fn generate_piece_moves(&self, from: Square, piece: Piece, moves: &mut Vec<Move>) {
        let first_move = moves.len();
        match piece.kind {
            PieceKind::Pawn => self.generate_pawn_moves(from, piece.color, moves),
            PieceKind::Knight => self.generate_leaps(from, piece.color, moves),
            PieceKind::Bishop => {
                self.generate_slides(from, piece.color, &DIAGONAL_DIRECTIONS, moves);
            }
            PieceKind::Rook => {
                self.generate_slides(from, piece.color, &ORTHOGONAL_DIRECTIONS, moves);
            }
            PieceKind::Queen => {
                self.generate_slides(from, piece.color, &ORTHOGONAL_DIRECTIONS, moves);
                self.generate_slides(from, piece.color, &DIAGONAL_DIRECTIONS, moves);
            }
            PieceKind::King => {
                self.generate_king_moves(from, piece.color, moves);
            }
            PieceKind::Archbishop => {
                self.generate_slides(from, piece.color, &DIAGONAL_DIRECTIONS, moves);
                self.generate_leaps(from, piece.color, moves);
            }
            PieceKind::Chancellor => {
                self.generate_slides(from, piece.color, &ORTHOGONAL_DIRECTIONS, moves);
                self.generate_leaps(from, piece.color, moves);
            }
            PieceKind::Cannon => self.generate_cannon_moves(from, piece.color, moves),
            PieceKind::Elephant => {
                self.generate_offset_moves(from, piece.color, &ELEPHANT_OFFSETS, moves);
            }
            PieceKind::Camel => {
                self.generate_offset_moves(from, piece.color, &CAMEL_OFFSETS, moves);
            }
            PieceKind::Giraffe => {
                self.generate_offset_moves(from, piece.color, &GIRAFFE_OFFSETS, moves);
            }
            PieceKind::Archer => {
                self.generate_screen_slider_moves(from, piece.color, &DIAGONAL_DIRECTIONS, moves);
            }
            PieceKind::Machine => {
                self.generate_offset_moves(from, piece.color, &MACHINE_OFFSETS, moves);
            }
            PieceKind::Amazon => {
                self.generate_slides(from, piece.color, &ORTHOGONAL_DIRECTIONS, moves);
                self.generate_slides(from, piece.color, &DIAGONAL_DIRECTIONS, moves);
                self.generate_leaps(from, piece.color, moves);
            }
            PieceKind::Lion => self.generate_lion_moves(from, piece.color, moves),
            PieceKind::Buffalo => {
                self.generate_offset_moves(from, piece.color, &KNIGHT_OFFSETS, moves);
                self.generate_offset_moves(from, piece.color, &CAMEL_OFFSETS, moves);
                self.generate_offset_moves(from, piece.color, &GIRAFFE_OFFSETS, moves);
            }
            PieceKind::Centaur => {
                self.generate_king_steps(from, piece.color, moves);
                self.generate_leaps(from, piece.color, moves);
            }
            PieceKind::Admiral => {
                self.generate_slides(from, piece.color, &ORTHOGONAL_DIRECTIONS, moves);
                self.generate_offset_moves(from, piece.color, &DIAGONAL_DIRECTIONS, moves);
            }
            PieceKind::Missionary => {
                self.generate_slides(from, piece.color, &DIAGONAL_DIRECTIONS, moves);
                self.generate_offset_moves(from, piece.color, &ORTHOGONAL_DIRECTIONS, moves);
            }
            PieceKind::Eagle => self.generate_eagle_moves(from, piece.color, moves),
            PieceKind::Rhinoceros => self.generate_rhinoceros_moves(from, piece.color, moves),
            PieceKind::Prince => self.generate_prince_moves(from, piece.color, moves),
            PieceKind::Sorceress => {
                self.generate_screen_slider_moves(from, piece.color, &ORTHOGONAL_DIRECTIONS, moves);
                self.generate_screen_slider_moves(from, piece.color, &DIAGONAL_DIRECTIONS, moves);
            }
            PieceKind::Duchess => self.generate_duchess_moves(from, piece.color, moves),
            PieceKind::Troll => self.generate_troll_moves(from, piece.color, moves),
        }

        if matches!(self.rules.promotion(), PromotionRule::TerachessII)
            && piece.kind != PieceKind::Pawn
        {
            for chess_move in &mut moves[first_move..] {
                if self.terachess_promotion_applies(piece.kind, piece.color, *chess_move) {
                    chess_move.promotion = terachess_promotion_target(piece.kind);
                }
            }
        }
    }

    fn generate_pawn_moves(&self, from: Square, color: Color, moves: &mut Vec<Move>) {
        let direction = color.pawn_direction();
        if let Some(one_step) = from.offset(0, direction).filter(|square| {
            self.board.size().contains(*square) && self.board.piece_at(*square).is_none()
        }) {
            self.push_pawn_move(from, one_step, color, MoveKind::Normal, moves);

            if self.rules.pawn_can_double_from(color, from.rank())
                && let Some(two_step) = from.offset(0, direction * 2).filter(|square| {
                    self.board.size().contains(*square) && self.board.piece_at(*square).is_none()
                })
            {
                self.push_pawn_move(from, two_step, color, MoveKind::Normal, moves);
            }
        }

        for file_delta in [-1, 1] {
            let Some(to) = from
                .offset(file_delta, direction)
                .filter(|square| self.board.size().contains(*square))
            else {
                continue;
            };

            if self
                .board
                .piece_at(to)
                .is_some_and(|piece| piece.color != color && piece.kind != PieceKind::King)
            {
                self.push_pawn_move(from, to, color, MoveKind::Normal, moves);
            } else if self.en_passant == Some(to) {
                let captured = Square::new(to.file(), from.rank());
                if self.board.piece_at(captured).is_some_and(|piece| {
                    piece.color == color.opposite()
                        && ((piece.kind == PieceKind::Pawn
                            || (self.rules.rapid_pawns() && piece.kind == PieceKind::Prince))
                            || self.terachess_promoted_en_passant_victim(piece, captured))
                }) {
                    self.push_pawn_move(from, to, color, MoveKind::EnPassant, moves);
                }
            }
        }
    }

    fn push_pawn_move(
        &self,
        from: Square,
        to: Square,
        color: Color,
        kind: MoveKind,
        moves: &mut Vec<Move>,
    ) {
        let relative_rank = match color {
            Color::White => to.rank() + 1,
            Color::Black => self.board.size().ranks() - to.rank(),
        };

        match self.rules.promotion() {
            PromotionRule::LastRank { choices } if relative_rank == self.board.size().ranks() => {
                moves.extend(choices.iter().copied().map(|promotion| Move {
                    from,
                    to,
                    promotion: Some(promotion),
                    kind,
                }));
            }
            PromotionRule::Grand if relative_rank >= 8 => {
                let choices = self.grand_promotion_choices(color);
                let mandatory = relative_rank == self.board.size().ranks();
                if !mandatory {
                    moves.push(Move {
                        from,
                        to,
                        promotion: None,
                        kind,
                    });
                }
                moves.extend(choices.into_iter().map(|promotion| Move {
                    from,
                    to,
                    promotion: Some(promotion),
                    kind,
                }));
            }
            PromotionRule::TerachessII if relative_rank == self.board.size().ranks() => {
                moves.push(Move {
                    from,
                    to,
                    promotion: Some(PieceKind::Queen),
                    kind,
                });
            }
            _ => moves.push(Move {
                from,
                to,
                promotion: None,
                kind,
            }),
        }
    }

    fn grand_promotion_choices(&self, color: Color) -> Vec<PieceKind> {
        PieceKind::PROMOTION_PIECES
            .into_iter()
            .filter(|kind| {
                self.board.count(color, *kind) < usize::from(self.rules.initial_count(color, *kind))
            })
            .collect()
    }

    fn generate_leaps(&self, from: Square, color: Color, moves: &mut Vec<Move>) {
        self.generate_offset_moves(from, color, &KNIGHT_OFFSETS, moves);
    }

    fn generate_king_steps(&self, from: Square, color: Color, moves: &mut Vec<Move>) {
        for file_delta in -1..=1 {
            for rank_delta in -1..=1 {
                if file_delta != 0 || rank_delta != 0 {
                    self.generate_offset_moves(from, color, &[(file_delta, rank_delta)], moves);
                }
            }
        }
    }

    fn generate_lion_moves(&self, from: Square, color: Color, moves: &mut Vec<Move>) {
        self.generate_king_steps(from, color, moves);
        self.generate_leaps(from, color, moves);
        for &(file_delta, rank_delta) in ORTHOGONAL_DIRECTIONS
            .iter()
            .chain(DIAGONAL_DIRECTIONS.iter())
        {
            self.generate_offset_moves(from, color, &[(file_delta * 2, rank_delta * 2)], moves);
        }
    }

    fn generate_prince_moves(&self, from: Square, color: Color, moves: &mut Vec<Move>) {
        self.generate_king_steps(from, color, moves);
        if !self.rules.rapid_pawns() {
            return;
        }
        let direction = color.pawn_direction();
        let one_step_is_clear = from.offset(0, direction).is_some_and(|square| {
            self.board.size().contains(square) && self.board.piece_at(square).is_none()
        });
        if one_step_is_clear
            && let Some(two_steps) = from.offset(0, direction * 2).filter(|square| {
                self.board.size().contains(*square) && self.board.piece_at(*square).is_none()
            })
        {
            moves.push(Move::normal(from, two_steps));
        }
    }

    fn generate_troll_moves(&self, from: Square, color: Color, moves: &mut Vec<Move>) {
        for &(file_delta, rank_delta) in ORTHOGONAL_DIRECTIONS
            .iter()
            .chain(DIAGONAL_DIRECTIONS.iter())
        {
            self.generate_offset_moves(from, color, &[(file_delta * 3, rank_delta * 3)], moves);
        }

        let direction = color.pawn_direction();
        if let Some(to) = from.offset(0, direction).filter(|square| {
            self.board.size().contains(*square) && self.board.piece_at(*square).is_none()
        }) {
            moves.push(Move::normal(from, to));
        }
        for file_delta in [-1, 1] {
            if let Some(to) = from
                .offset(file_delta, direction)
                .filter(|square| self.board.size().contains(*square))
                && self
                    .board
                    .piece_at(to)
                    .is_some_and(|piece| piece.color != color && piece.kind != PieceKind::King)
            {
                moves.push(Move::normal(from, to));
            }
        }
    }

    fn generate_duchess_moves(&self, from: Square, color: Color, moves: &mut Vec<Move>) {
        for distance in 1..=3 {
            for &(file_delta, rank_delta) in ORTHOGONAL_DIRECTIONS
                .iter()
                .chain(DIAGONAL_DIRECTIONS.iter())
            {
                self.generate_offset_moves(
                    from,
                    color,
                    &[(file_delta * distance, rank_delta * distance)],
                    moves,
                );
            }
        }
    }

    fn generate_eagle_moves(&self, from: Square, color: Color, moves: &mut Vec<Move>) {
        for &(file_delta, rank_delta) in &DIAGONAL_DIRECTIONS {
            self.generate_bent_slider_branch(
                from,
                color,
                (file_delta, rank_delta),
                &[(file_delta, 0), (0, rank_delta)],
                moves,
            );
        }
    }

    fn generate_rhinoceros_moves(&self, from: Square, color: Color, moves: &mut Vec<Move>) {
        for &(file_delta, rank_delta) in &ORTHOGONAL_DIRECTIONS {
            let continuations = if file_delta == 0 {
                [(1, rank_delta), (-1, rank_delta)]
            } else {
                [(file_delta, 1), (file_delta, -1)]
            };
            self.generate_bent_slider_branch(
                from,
                color,
                (file_delta, rank_delta),
                &continuations,
                moves,
            );
        }
    }

    fn generate_bent_slider_branch(
        &self,
        from: Square,
        color: Color,
        first_delta: (i8, i8),
        continuations: &[(i8, i8)],
        moves: &mut Vec<Move>,
    ) {
        let Some(first) = from
            .offset(first_delta.0, first_delta.1)
            .filter(|square| self.board.size().contains(*square))
        else {
            return;
        };
        match self.board.piece_at(first) {
            None => moves.push(Move::normal(from, first)),
            Some(piece) if piece.color != color && piece.kind != PieceKind::King => {
                moves.push(Move::normal(from, first));
                return;
            }
            Some(_) => return,
        }

        for &(file_delta, rank_delta) in continuations {
            let mut current = first;
            while let Some(to) = current
                .offset(file_delta, rank_delta)
                .filter(|square| self.board.size().contains(*square))
            {
                match self.board.piece_at(to) {
                    None => moves.push(Move::normal(from, to)),
                    Some(piece) if piece.color != color && piece.kind != PieceKind::King => {
                        moves.push(Move::normal(from, to));
                        break;
                    }
                    Some(_) => break,
                }
                current = to;
            }
        }
    }

    fn terachess_promotion_applies(&self, kind: PieceKind, color: Color, chess_move: Move) -> bool {
        let final_rank = match color {
            Color::White => self.board.size().ranks() - 1,
            Color::Black => 0,
        };
        if chess_move.to.rank() != final_rank || terachess_promotion_target(kind).is_none() {
            return false;
        }
        kind != PieceKind::Troll
            || (i16::from(chess_move.to.rank()) - i16::from(chess_move.from.rank())
                == i16::from(color.pawn_direction())
                && chess_move.from.file().abs_diff(chess_move.to.file()) <= 1)
    }

    fn terachess_promoted_en_passant_victim(&self, piece: Piece, square: Square) -> bool {
        if !self.rules.rapid_pawns() {
            return false;
        }
        let promotion_rank = match piece.color {
            Color::White => self.board.size().ranks() - 1,
            Color::Black => 0,
        };
        square.rank() == promotion_rank
            && matches!(piece.kind, PieceKind::Queen | PieceKind::Amazon)
    }

    fn generate_offset_moves(
        &self,
        from: Square,
        color: Color,
        offsets: &[(i8, i8)],
        moves: &mut Vec<Move>,
    ) {
        for &(file_delta, rank_delta) in offsets {
            if let Some(to) = from
                .offset(file_delta, rank_delta)
                .filter(|square| self.board.size().contains(*square))
            {
                self.push_non_pawn_move(from, to, color, moves);
            }
        }
    }

    fn generate_cannon_moves(&self, from: Square, color: Color, moves: &mut Vec<Move>) {
        self.generate_screen_slider_moves(from, color, &ORTHOGONAL_DIRECTIONS, moves);
    }

    fn generate_screen_slider_moves(
        &self,
        from: Square,
        color: Color,
        directions: &[(i8, i8)],
        moves: &mut Vec<Move>,
    ) {
        for &(file_delta, rank_delta) in directions {
            let mut current = from;
            let mut found_screen = false;
            while let Some(to) = current
                .offset(file_delta, rank_delta)
                .filter(|square| self.board.size().contains(*square))
            {
                match (found_screen, self.board.piece_at(to)) {
                    (false, None) => moves.push(Move::normal(from, to)),
                    (false, Some(_)) => found_screen = true,
                    (true, None) => {}
                    (true, Some(piece)) => {
                        if piece.color != color && piece.kind != PieceKind::King {
                            moves.push(Move::normal(from, to));
                        }
                        break;
                    }
                }
                current = to;
            }
        }
    }

    fn generate_slides(
        &self,
        from: Square,
        color: Color,
        directions: &[(i8, i8)],
        moves: &mut Vec<Move>,
    ) {
        for &(file_delta, rank_delta) in directions {
            let mut current = from;
            while let Some(to) = current
                .offset(file_delta, rank_delta)
                .filter(|square| self.board.size().contains(*square))
            {
                match self.board.piece_at(to) {
                    None => moves.push(Move::normal(from, to)),
                    Some(piece) if piece.color != color && piece.kind != PieceKind::King => {
                        moves.push(Move::normal(from, to));
                        break;
                    }
                    Some(_) => break,
                }
                current = to;
            }
        }
    }

    fn generate_king_moves(&self, from: Square, color: Color, moves: &mut Vec<Move>) {
        self.generate_king_steps(from, color, moves);

        for side in CastleSide::ALL {
            if self.can_castle(color, side, from)
                && let Some(route) = self.rules.castling().route(color, side)
            {
                moves.push(Move {
                    from,
                    to: route.king_to,
                    promotion: None,
                    kind: MoveKind::Castle(side),
                });
            }
        }

        if self.king_jump_available(color) && !self.is_in_check(color) {
            for file_delta in -2_i8..=2 {
                for rank_delta in -2_i8..=2 {
                    if file_delta.abs().max(rank_delta.abs()) != 2 {
                        continue;
                    }
                    let Some(to) = from.offset(file_delta, rank_delta).filter(|square| {
                        self.board.size().contains(*square)
                            && self.board.piece_at(*square).is_none()
                    }) else {
                        continue;
                    };
                    if self.king_jump_intermediate_is_safe(from, color, file_delta, rank_delta) {
                        moves.push(Move::normal(from, to));
                    }
                }
            }
        }
    }

    fn king_jump_intermediate_is_safe(
        &self,
        from: Square,
        color: Color,
        file_delta: i8,
        rank_delta: i8,
    ) -> bool {
        let file_step = file_delta.signum();
        let rank_step = rank_delta.signum();
        if file_delta == 0 || rank_delta == 0 || file_delta.abs() == rank_delta.abs() {
            return from
                .offset(file_step, rank_step)
                .is_some_and(|square| !self.is_square_attacked(square, color.opposite()));
        }

        let intermediates = if file_delta.abs() == 2 {
            [from.offset(file_step, 0), from.offset(file_step, rank_step)]
        } else {
            [from.offset(0, rank_step), from.offset(file_step, rank_step)]
        };
        intermediates.into_iter().flatten().any(|square| {
            self.board.size().contains(square) && !self.is_square_attacked(square, color.opposite())
        })
    }

    fn push_non_pawn_move(&self, from: Square, to: Square, color: Color, moves: &mut Vec<Move>) {
        match self.board.piece_at(to) {
            None => moves.push(Move::normal(from, to)),
            Some(piece) if piece.color != color && piece.kind != PieceKind::King => {
                moves.push(Move::normal(from, to));
            }
            Some(_) => {}
        }
    }

    fn can_castle(&self, color: Color, side: CastleSide, king_square: Square) -> bool {
        if !self.castling_rights.has(color, side) {
            return false;
        }
        let Some(route) = self.rules.castling().route(color, side) else {
            return false;
        };
        if route.king_from != king_square
            || self.board.piece_at(route.king_from) != Some(Piece::new(color, PieceKind::King))
            || self.board.piece_at(route.rook_from) != Some(Piece::new(color, PieceKind::Rook))
        {
            return false;
        }

        let low = route.king_from.file().min(route.rook_from.file()) + 1;
        let high = route.king_from.file().max(route.rook_from.file());
        if (low..high).any(|file| {
            let square = Square::new(file, route.king_from.rank());
            self.board.piece_at(square).is_some()
        }) {
            return false;
        }
        for destination in [route.king_to, route.rook_to] {
            if destination != route.king_from
                && destination != route.rook_from
                && self.board.piece_at(destination).is_some()
            {
                return false;
            }
        }

        self.king_castling_path_is_safe(color, route)
    }

    fn king_castling_path_is_safe(&self, color: Color, route: CastleRoute) -> bool {
        let step = if route.king_to.file() > route.king_from.file() {
            1
        } else {
            -1
        };
        let mut board = self.board.clone();
        let mut current = route.king_from;
        loop {
            if current != route.king_from {
                board.set_piece_unchecked(current.offset(-step, 0).unwrap(), None);
                board.set_piece_unchecked(current, Some(Piece::new(color, PieceKind::King)));
            }
            if is_square_attacked_on(&board, current, color.opposite()) {
                return false;
            }
            if current == route.king_to {
                return true;
            }
            current = current.offset(step, 0).unwrap();
        }
    }

    fn make_move_unchecked(&mut self, chess_move: Move) -> PositionUndo {
        let moving_piece = self
            .board
            .piece_at(chess_move.from)
            .expect("unchecked move must have a moving piece");
        let mut undo = PositionUndo::new(self);
        match chess_move.kind {
            MoveKind::Castle(side) => {
                let route = self
                    .rules
                    .castling()
                    .route(moving_piece.color, side)
                    .expect("unchecked castling move must have a route");
                for square in [
                    route.king_from,
                    route.rook_from,
                    route.king_to,
                    route.rook_to,
                ] {
                    undo.record_square(&self.board, square);
                }
            }
            MoveKind::Normal => {
                undo.record_square(&self.board, chess_move.from);
                undo.record_square(&self.board, chess_move.to);
            }
            MoveKind::EnPassant => {
                undo.record_square(&self.board, chess_move.from);
                undo.record_square(&self.board, chess_move.to);
                undo.record_square(
                    &self.board,
                    Square::new(chess_move.to.file(), chess_move.from.rank()),
                );
            }
        }
        self.apply_unchecked(chess_move);
        undo
    }

    fn apply_unchecked(&mut self, chess_move: Move) {
        let moving_piece = self
            .board
            .piece_at(chess_move.from)
            .expect("unchecked move must have a moving piece");
        let captured_square = match chess_move.kind {
            MoveKind::EnPassant => Some(Square::new(chess_move.to.file(), chess_move.from.rank())),
            _ if self.board.piece_at(chess_move.to).is_some() => Some(chess_move.to),
            _ => None,
        };
        let captured_piece = captured_square.and_then(|square| self.board.piece_at(square));

        self.update_castling_rights(
            moving_piece,
            chess_move.from,
            captured_piece,
            captured_square,
        );
        if moving_piece.kind == PieceKind::King {
            self.king_jump_rights[moving_piece.color.index()] = false;
        }

        match chess_move.kind {
            MoveKind::Castle(side) => {
                let route = self
                    .rules
                    .castling()
                    .route(moving_piece.color, side)
                    .expect("unchecked castling move must have a route");
                self.board.set_piece_unchecked(route.king_from, None);
                self.board.set_piece_unchecked(route.rook_from, None);
                self.board.set_piece_unchecked(
                    route.king_to,
                    Some(Piece::new(moving_piece.color, PieceKind::King)),
                );
                self.board.set_piece_unchecked(
                    route.rook_to,
                    Some(Piece::new(moving_piece.color, PieceKind::Rook)),
                );
            }
            MoveKind::Normal | MoveKind::EnPassant => {
                self.board.set_piece_unchecked(chess_move.from, None);
                if let Some(captured_square) = captured_square {
                    self.board.set_piece_unchecked(captured_square, None);
                }
                self.board.set_piece_unchecked(
                    chess_move.to,
                    Some(Piece::new(
                        moving_piece.color,
                        chess_move.promotion.unwrap_or(moving_piece.kind),
                    )),
                );
            }
        }

        self.en_passant = if (moving_piece.kind == PieceKind::Pawn
            || (self.rules.rapid_pawns() && moving_piece.kind == PieceKind::Prince))
            && chess_move.from.rank().abs_diff(chess_move.to.rank()) == 2
        {
            Some(Square::new(
                chess_move.from.file(),
                (chess_move.from.rank() + chess_move.to.rank()) / 2,
            ))
        } else {
            None
        };
        if moving_piece.kind == PieceKind::Pawn || captured_piece.is_some() {
            self.halfmove_clock = 0;
        } else {
            self.halfmove_clock = self.halfmove_clock.saturating_add(1);
        }
        if self.side_to_move == Color::Black {
            self.fullmove_number = self.fullmove_number.saturating_add(1);
        }
        self.side_to_move = self.side_to_move.opposite();
    }

    fn update_castling_rights(
        &mut self,
        moving_piece: Piece,
        from: Square,
        captured_piece: Option<Piece>,
        captured_square: Option<Square>,
    ) {
        if moving_piece.kind == PieceKind::King {
            self.castling_rights.clear_color(moving_piece.color);
        } else if moving_piece.kind == PieceKind::Rook {
            for side in CastleSide::ALL {
                if self
                    .rules
                    .castling()
                    .route(moving_piece.color, side)
                    .is_some_and(|route| route.rook_from == from)
                {
                    self.castling_rights.set(moving_piece.color, side, false);
                }
            }
        }

        if let (Some(piece), Some(square)) = (captured_piece, captured_square)
            && piece.kind == PieceKind::Rook
        {
            for side in CastleSide::ALL {
                if self
                    .rules
                    .castling()
                    .route(piece.color, side)
                    .is_some_and(|route| route.rook_from == square)
                {
                    self.castling_rights.set(piece.color, side, false);
                }
            }
        }
    }

    /// Serializes the position using extended FEN piece letters.
    #[must_use]
    pub fn to_fen(&self) -> String {
        let size = self.board.size();
        let mut ranks = Vec::with_capacity(usize::from(size.ranks()));
        for rank in (0..size.ranks()).rev() {
            let mut value = String::new();
            let mut empty = 0;
            for file in 0..size.files() {
                match self.board.piece_at(Square::new(file, rank)) {
                    Some(piece) => {
                        if empty > 0 {
                            value.push_str(&empty.to_string());
                            empty = 0;
                        }
                        value.push(self.rules.piece_fen_char(piece));
                    }
                    None => empty += 1,
                }
            }
            if empty > 0 {
                value.push_str(&empty.to_string());
            }
            ranks.push(value);
        }

        let side = match self.side_to_move {
            Color::White => "w",
            Color::Black => "b",
        };
        let castling = format_position_rights(self.castling_rights, self.king_jump_rights);
        let en_passant = self
            .en_passant
            .map_or_else(|| "-".to_owned(), |square| square.to_string());
        format!(
            "{} {side} {castling} {en_passant} {} {}",
            ranks.join("/"),
            self.halfmove_clock,
            self.fullmove_number
        )
    }

    /// Hashes the state relevant to legal move repetition. Move clocks and an
    /// en passant marker that offers no legal capture are excluded.
    pub(crate) fn hash_repetition_state<H: Hasher>(&self, state: &mut H) {
        self.board.hash(state);
        self.side_to_move.hash(state);
        self.castling_rights.hash(state);
        self.king_jump_rights.hash(state);
        self.legal_moves()
            .iter()
            .find(|chess_move| chess_move.kind == MoveKind::EnPassant)
            .map(|chess_move| chess_move.to)
            .hash(state);
    }
}

fn parse_position_rights(
    rules: &VariantRules,
    value: &str,
) -> Result<(CastlingRights, [bool; 2]), FenError> {
    if value == "-" {
        return Ok((CastlingRights::none(), [false; 2]));
    }
    let mut rights = CastlingRights::none();
    let mut king_jump_rights = [false; 2];
    for token in value.chars() {
        match token {
            'J' | 'j' => {
                if !rules.king_initial_jump() {
                    return Err(FenError::InvalidCastling(value.to_owned()));
                }
                let color = if token == 'J' {
                    Color::White
                } else {
                    Color::Black
                };
                king_jump_rights[color.index()] = true;
            }
            _ => {
                let (color, side) = match token {
                    'K' => (Color::White, CastleSide::KingSide),
                    'Q' => (Color::White, CastleSide::QueenSide),
                    'k' => (Color::Black, CastleSide::KingSide),
                    'q' => (Color::Black, CastleSide::QueenSide),
                    _ => return Err(FenError::InvalidCastling(value.to_owned())),
                };
                if rules.castling().route(color, side).is_none() {
                    return Err(FenError::InvalidCastling(value.to_owned()));
                }
                rights.set(color, side, true);
            }
        }
    }
    Ok((rights, king_jump_rights))
}

fn format_position_rights(rights: CastlingRights, king_jump_rights: [bool; 2]) -> String {
    let mut value = String::new();
    for (color, side, token) in [
        (Color::White, CastleSide::KingSide, 'K'),
        (Color::White, CastleSide::QueenSide, 'Q'),
        (Color::Black, CastleSide::KingSide, 'k'),
        (Color::Black, CastleSide::QueenSide, 'q'),
    ] {
        if rights.has(color, side) {
            value.push(token);
        }
    }
    if king_jump_rights[Color::White.index()] {
        value.push('J');
    }
    if king_jump_rights[Color::Black.index()] {
        value.push('j');
    }
    if value.is_empty() {
        value.push('-');
    }
    value
}

fn is_square_attacked_on(board: &Board, target: Square, by: Color) -> bool {
    board
        .pieces()
        .filter(|(_, piece)| piece.color == by)
        .any(|(from, piece)| piece_attacks_square(board, from, piece, target))
}

fn piece_attacks_square(board: &Board, from: Square, piece: Piece, target: Square) -> bool {
    let file_delta = i16::from(target.file()) - i16::from(from.file());
    let rank_delta = i16::from(target.rank()) - i16::from(from.rank());
    if file_delta == 0 && rank_delta == 0 {
        return false;
    }
    let abs_file = file_delta.unsigned_abs();
    let abs_rank = rank_delta.unsigned_abs();
    let orthogonal = file_delta == 0 || rank_delta == 0;
    let diagonal = abs_file == abs_rank;
    let king_step = abs_file <= 1 && abs_rank <= 1;
    let knight_leap = (abs_file == 1 && abs_rank == 2) || (abs_file == 2 && abs_rank == 1);
    let camel_leap = (abs_file == 1 && abs_rank == 3) || (abs_file == 3 && abs_rank == 1);
    let giraffe_leap = (abs_file == 2 && abs_rank == 3) || (abs_file == 3 && abs_rank == 2);

    match piece.kind {
        PieceKind::Pawn => abs_file == 1 && rank_delta == i16::from(piece.color.pawn_direction()),
        PieceKind::Knight => knight_leap,
        PieceKind::Bishop => diagonal && line_is_clear(board, from, target),
        PieceKind::Rook => orthogonal && line_is_clear(board, from, target),
        PieceKind::Queen => (orthogonal || diagonal) && line_is_clear(board, from, target),
        PieceKind::King => king_step,
        PieceKind::Archbishop => knight_leap || (diagonal && line_is_clear(board, from, target)),
        PieceKind::Chancellor => knight_leap || (orthogonal && line_is_clear(board, from, target)),
        PieceKind::Cannon => orthogonal && screen_count_between(board, from, target) == Some(1),
        PieceKind::Elephant => diagonal && abs_file <= 2,
        PieceKind::Camel => camel_leap,
        PieceKind::Giraffe => giraffe_leap,
        PieceKind::Archer => diagonal && screen_count_between(board, from, target) == Some(1),
        PieceKind::Machine => orthogonal && abs_file.max(abs_rank) <= 2,
        PieceKind::Amazon => {
            knight_leap || ((orthogonal || diagonal) && line_is_clear(board, from, target))
        }
        PieceKind::Lion => {
            king_step
                || knight_leap
                || (orthogonal && abs_file.max(abs_rank) == 2)
                || (diagonal && abs_file == 2)
        }
        PieceKind::Buffalo => knight_leap || camel_leap || giraffe_leap,
        PieceKind::Centaur => king_step || knight_leap,
        PieceKind::Admiral => {
            (orthogonal && line_is_clear(board, from, target)) || (diagonal && abs_file == 1)
        }
        PieceKind::Missionary => {
            (diagonal && line_is_clear(board, from, target))
                || (orthogonal && abs_file.max(abs_rank) == 1)
        }
        PieceKind::Eagle => bent_slider_attacks(board, from, target, true),
        PieceKind::Rhinoceros => bent_slider_attacks(board, from, target, false),
        PieceKind::Prince => king_step,
        PieceKind::Sorceress => {
            (orthogonal || diagonal) && screen_count_between(board, from, target) == Some(1)
        }
        PieceKind::Duchess => (orthogonal || diagonal) && abs_file.max(abs_rank) <= 3,
        PieceKind::Troll => {
            ((orthogonal || diagonal) && abs_file.max(abs_rank) == 3)
                || (abs_file == 1 && rank_delta == i16::from(piece.color.pawn_direction()))
        }
    }
}

fn line_is_clear(board: &Board, from: Square, target: Square) -> bool {
    screen_count_between(board, from, target) == Some(0)
}

fn screen_count_between(board: &Board, from: Square, target: Square) -> Option<usize> {
    let file_delta = i16::from(target.file()) - i16::from(from.file());
    let rank_delta = i16::from(target.rank()) - i16::from(from.rank());
    if file_delta != 0 && rank_delta != 0 && file_delta.unsigned_abs() != rank_delta.unsigned_abs()
    {
        return None;
    }
    let file_step = file_delta.signum() as i8;
    let rank_step = rank_delta.signum() as i8;
    let mut current = from;
    let mut screens = 0;
    while let Some(square) = current.offset(file_step, rank_step) {
        if square == target {
            return Some(screens);
        }
        if board.piece_at(square).is_some() {
            screens += 1;
        }
        current = square;
    }
    None
}

fn bent_slider_attacks(board: &Board, from: Square, target: Square, eagle: bool) -> bool {
    let first_directions: &[(i8, i8)] = if eagle {
        &DIAGONAL_DIRECTIONS
    } else {
        &ORTHOGONAL_DIRECTIONS
    };
    for &(file_delta, rank_delta) in first_directions {
        let Some(first) = from
            .offset(file_delta, rank_delta)
            .filter(|square| board.size().contains(*square))
        else {
            continue;
        };
        if first == target {
            return true;
        }
        if board.piece_at(first).is_some() {
            continue;
        }
        let continuations = if eagle {
            [(file_delta, 0), (0, rank_delta)]
        } else if file_delta == 0 {
            [(1, rank_delta), (-1, rank_delta)]
        } else {
            [(file_delta, 1), (file_delta, -1)]
        };
        for (file_step, rank_step) in continuations {
            let mut current = first;
            while let Some(square) = current
                .offset(file_step, rank_step)
                .filter(|square| board.size().contains(*square))
            {
                if square == target {
                    return true;
                }
                if board.piece_at(square).is_some() {
                    break;
                }
                current = square;
            }
        }
    }
    false
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MoveError {
    IllegalMove(Move),
    IllegalNotation(String),
}

impl fmt::Display for MoveError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::IllegalMove(chess_move) => write!(formatter, "illegal move: {chess_move}"),
            Self::IllegalNotation(value) => write!(formatter, "illegal move notation: {value}"),
        }
    }
}

impl std::error::Error for MoveError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FenError {
    FieldCount(usize),
    RankCount(usize),
    InvalidPlacement(String),
    UnknownPiece(char),
    KingCount { color: Color, count: usize },
    InvalidSide(String),
    InvalidCastling(String),
    InvalidEnPassant(String),
    InvalidClock(String),
}

impl fmt::Display for FenError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FieldCount(count) => write!(formatter, "FEN has {count} fields; expected 6"),
            Self::RankCount(count) => write!(formatter, "FEN has {count} ranks"),
            Self::InvalidPlacement(value) => write!(formatter, "invalid FEN rank: {value}"),
            Self::UnknownPiece(value) => write!(formatter, "unknown FEN piece: {value}"),
            Self::KingCount { color, count } => {
                write!(formatter, "FEN has {count} {color:?} kings; expected 1")
            }
            Self::InvalidSide(value) => write!(formatter, "invalid side to move: {value}"),
            Self::InvalidCastling(value) => write!(formatter, "invalid castling rights: {value}"),
            Self::InvalidEnPassant(value) => {
                write!(formatter, "invalid en passant square: {value}")
            }
            Self::InvalidClock(value) => write!(formatter, "invalid move clock: {value}"),
        }
    }
}

impl std::error::Error for FenError {}
