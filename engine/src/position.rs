use crate::mv::{CastleSide, Move, MoveKind};
use crate::rules::{CastleRoute, CastlingRights, PromotionRule, VariantRules};
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

/// A complete game position under one immutable set of variant rules.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Position {
    rules: Arc<VariantRules>,
    board: Board,
    side_to_move: Color,
    castling_rights: CastlingRights,
    en_passant: Option<Square>,
    halfmove_clock: u32,
    fullmove_number: u32,
}

impl Position {
    pub(crate) fn from_starting_board(rules: Arc<VariantRules>, board: Board) -> Self {
        Self {
            castling_rights: CastlingRights::from_rules(rules.castling()),
            rules,
            board,
            side_to_move: Color::White,
            en_passant: None,
            halfmove_clock: 0,
            fullmove_number: 1,
        }
    }

    /// Parses extended FEN. `A` denotes an archbishop/cardinal and `C` (or
    /// accepted input alias `M`) denotes a chancellor/marshal.
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

                let (kind, color) =
                    PieceKind::from_fen_char(value).ok_or(FenError::UnknownPiece(value))?;
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
        let castling_rights = parse_castling_rights(&rules, fields[2])?;
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
        let moving_color = self.side_to_move;
        let mut pseudo = Vec::with_capacity(96);
        for (from, piece) in self.board.pieces() {
            if piece.color == moving_color {
                self.generate_piece_moves(from, piece, &mut pseudo);
            }
        }

        pseudo
            .into_iter()
            .filter(|chess_move| {
                let mut next = self.clone();
                next.apply_unchecked(*chess_move);
                !next.is_in_check(moving_color)
            })
            .collect()
    }

    /// Counts leaf positions to a fixed depth. This is primarily useful for
    /// validating integrations and move-generation changes.
    #[must_use]
    pub fn perft(&self, depth: u8) -> u64 {
        if depth == 0 {
            return 1;
        }
        self.legal_moves()
            .into_iter()
            .map(|chess_move| self.after_move_unchecked(chess_move).perft(depth - 1))
            .fold(0, u64::saturating_add)
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

    pub(crate) fn after_move_unchecked(&self, chess_move: Move) -> Self {
        let mut next = self.clone();
        next.apply_unchecked(chess_move);
        next
    }

    fn generate_piece_moves(&self, from: Square, piece: Piece, moves: &mut Vec<Move>) {
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
        }
    }

    fn generate_pawn_moves(&self, from: Square, color: Color, moves: &mut Vec<Move>) {
        let direction = color.pawn_direction();
        if let Some(one_step) = from.offset(0, direction).filter(|square| {
            self.board.size().contains(*square) && self.board.piece_at(*square).is_none()
        }) {
            self.push_pawn_move(from, one_step, color, MoveKind::Normal, moves);

            if from.rank() == self.rules.pawn_start_rank(color)
                && let Some(two_step) = from.offset(0, direction * 2).filter(|square| {
                    self.board.size().contains(*square) && self.board.piece_at(*square).is_none()
                })
            {
                moves.push(Move::normal(from, two_step));
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
                if self.board.piece_at(captured)
                    == Some(Piece::new(color.opposite(), PieceKind::Pawn))
                {
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
        for (file_delta, rank_delta) in KNIGHT_OFFSETS {
            if let Some(to) = from
                .offset(file_delta, rank_delta)
                .filter(|square| self.board.size().contains(*square))
            {
                self.push_non_pawn_move(from, to, color, moves);
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
        for file_delta in -1..=1 {
            for rank_delta in -1..=1 {
                if file_delta == 0 && rank_delta == 0 {
                    continue;
                }
                if let Some(to) = from
                    .offset(file_delta, rank_delta)
                    .filter(|square| self.board.size().contains(*square))
                {
                    self.push_non_pawn_move(from, to, color, moves);
                }
            }
        }

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

        self.en_passant = if moving_piece.kind == PieceKind::Pawn
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
                        value.push(piece.fen_char());
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
        let castling = format_castling_rights(self.castling_rights);
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
        self.legal_moves()
            .iter()
            .find(|chess_move| chess_move.kind == MoveKind::EnPassant)
            .map(|chess_move| chess_move.to)
            .hash(state);
    }
}

fn parse_castling_rights(rules: &VariantRules, value: &str) -> Result<CastlingRights, FenError> {
    if value == "-" {
        return Ok(CastlingRights::none());
    }
    let mut rights = CastlingRights::none();
    for token in value.chars() {
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
    Ok(rights)
}

fn format_castling_rights(rights: CastlingRights) -> String {
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
    if value.is_empty() {
        value.push('-');
    }
    value
}

fn is_square_attacked_on(board: &Board, target: Square, by: Color) -> bool {
    let pawn_source_delta = -by.pawn_direction();
    for file_delta in [-1, 1] {
        if target
            .offset(file_delta, pawn_source_delta)
            .filter(|square| board.size().contains(*square))
            .is_some_and(|square| board.piece_at(square) == Some(Piece::new(by, PieceKind::Pawn)))
        {
            return true;
        }
    }

    for (file_delta, rank_delta) in KNIGHT_OFFSETS {
        if target
            .offset(file_delta, rank_delta)
            .filter(|square| board.size().contains(*square))
            .and_then(|square| board.piece_at(square))
            .is_some_and(|piece| {
                piece.color == by
                    && matches!(
                        piece.kind,
                        PieceKind::Knight | PieceKind::Archbishop | PieceKind::Chancellor
                    )
            })
        {
            return true;
        }
    }

    if attacked_along(
        board,
        target,
        by,
        &ORTHOGONAL_DIRECTIONS,
        &[PieceKind::Rook, PieceKind::Queen, PieceKind::Chancellor],
    ) || attacked_along(
        board,
        target,
        by,
        &DIAGONAL_DIRECTIONS,
        &[PieceKind::Bishop, PieceKind::Queen, PieceKind::Archbishop],
    ) {
        return true;
    }

    for file_delta in -1..=1 {
        for rank_delta in -1..=1 {
            if file_delta == 0 && rank_delta == 0 {
                continue;
            }
            if target
                .offset(file_delta, rank_delta)
                .filter(|square| board.size().contains(*square))
                .is_some_and(|square| {
                    board.piece_at(square) == Some(Piece::new(by, PieceKind::King))
                })
            {
                return true;
            }
        }
    }
    false
}

fn attacked_along(
    board: &Board,
    target: Square,
    by: Color,
    directions: &[(i8, i8)],
    attackers: &[PieceKind],
) -> bool {
    for &(file_delta, rank_delta) in directions {
        let mut current = target;
        while let Some(square) = current
            .offset(file_delta, rank_delta)
            .filter(|square| board.size().contains(*square))
        {
            if let Some(piece) = board.piece_at(square) {
                if piece.color == by && attackers.contains(&piece.kind) {
                    return true;
                }
                break;
            }
            current = square;
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
