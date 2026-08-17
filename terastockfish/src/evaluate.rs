use capablanca_chess_plus::{BoardSize, Color, Piece, PieceKind, Position, PositionUndo, Square};

/// Tunable static-evaluation weights. Material is expressed in centipawns,
/// with a Terachess II pawn fixed to 100 in the published profile.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EvaluationParameters {
    material: [i32; PieceKind::COUNT],
    centrality: [i32; PieceKind::COUNT],
    advancement: [i32; PieceKind::COUNT],
    doubled_pawn_penalty: i32,
    isolated_pawn_penalty: i32,
    bishop_pair_bonus: i32,
    king_shelter_bonus: i32,
    king_jump_bonus: i32,
    tempo_bonus: i32,
}

impl EvaluationParameters {
    /// The material scale published with Terachess II, plus the conservative
    /// positional weights used by the original TeraStockfish evaluator.
    #[must_use]
    pub const fn published() -> Self {
        Self {
            material: [
                100, 400, 680, 1_000, 1_660, 0, 1_060, 1_380, 1_000, 400, 360, 340, 660, 440,
                2_040, 1_200, 1_080, 820, 1_200, 880, 1_680, 1_220, 460, 1_640, 1_160, 480,
            ],
            centrality: [
                0, 2, 1, 1, 1, 0, 1, 1, 1, 2, 2, 2, 1, 2, 1, 2, 2, 2, 1, 1, 1, 1, 0, 1, 2, 2,
            ],
            advancement: [
                7, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 3, 0, 0, 3,
            ],
            doubled_pawn_penalty: 18,
            isolated_pawn_penalty: 12,
            bishop_pair_bonus: 28,
            king_shelter_bonus: 5,
            king_jump_bonus: 20,
            tempo_bonus: 12,
        }
    }

    /// The empirically fitted material profile validated against [`Self::published`]
    /// in a fixed 200-pair, 100,000-node self-play match.
    #[must_use]
    pub const fn empirical_v1() -> Self {
        let mut parameters = Self::published();
        parameters.material = [
            100, 392, 712, 997, 1_609, 0, 1_066, 1_405, 1_022, 392, 360, 347, 647, 439, 2_014,
            1_163, 1_154, 826, 1_147, 852, 1_462, 1_295, 466, 1_609, 1_178, 510,
        ];
        parameters
    }

    /// The profile used by default search and static evaluation.
    #[must_use]
    pub const fn production() -> Self {
        Self::empirical_v1()
    }

    #[must_use]
    pub const fn material_value(&self, kind: PieceKind) -> i32 {
        self.material[kind.index()]
    }

    pub fn set_material_value(&mut self, kind: PieceKind, value: i32) {
        self.material[kind.index()] = value.max(0);
    }

    #[must_use]
    pub const fn centrality_weight(&self, kind: PieceKind) -> i32 {
        self.centrality[kind.index()]
    }

    pub fn set_centrality_weight(&mut self, kind: PieceKind, value: i32) {
        self.centrality[kind.index()] = value;
    }

    #[must_use]
    pub const fn advancement_weight(&self, kind: PieceKind) -> i32 {
        self.advancement[kind.index()]
    }

    pub fn set_advancement_weight(&mut self, kind: PieceKind, value: i32) {
        self.advancement[kind.index()] = value;
    }

    #[must_use]
    pub const fn doubled_pawn_penalty(&self) -> i32 {
        self.doubled_pawn_penalty
    }

    pub fn set_doubled_pawn_penalty(&mut self, value: i32) {
        self.doubled_pawn_penalty = value;
    }

    #[must_use]
    pub const fn isolated_pawn_penalty(&self) -> i32 {
        self.isolated_pawn_penalty
    }

    pub fn set_isolated_pawn_penalty(&mut self, value: i32) {
        self.isolated_pawn_penalty = value;
    }

    #[must_use]
    pub const fn bishop_pair_bonus(&self) -> i32 {
        self.bishop_pair_bonus
    }

    pub fn set_bishop_pair_bonus(&mut self, value: i32) {
        self.bishop_pair_bonus = value;
    }

    #[must_use]
    pub const fn king_shelter_bonus(&self) -> i32 {
        self.king_shelter_bonus
    }

    pub fn set_king_shelter_bonus(&mut self, value: i32) {
        self.king_shelter_bonus = value;
    }

    #[must_use]
    pub const fn king_jump_bonus(&self) -> i32 {
        self.king_jump_bonus
    }

    pub fn set_king_jump_bonus(&mut self, value: i32) {
        self.king_jump_bonus = value;
    }

    #[must_use]
    pub const fn tempo_bonus(&self) -> i32 {
        self.tempo_bonus
    }

    pub fn set_tempo_bonus(&mut self, value: i32) {
        self.tempo_bonus = value;
    }
}

impl Default for EvaluationParameters {
    fn default() -> Self {
        Self::production()
    }
}

/// Returns the production Terachess II material value for a piece kind.
#[must_use]
pub const fn piece_value(kind: PieceKind) -> i32 {
    EvaluationParameters::production().material_value(kind)
}

/// Returns a static score using the production profile, from the side-to-move's
/// point of view.
#[must_use]
pub fn evaluate(position: &Position) -> i32 {
    evaluate_with(position, &EvaluationParameters::production())
}

/// Returns a static score using explicit evaluation parameters, from the
/// side-to-move's point of view.
#[must_use]
pub fn evaluate_with(position: &Position, parameters: &EvaluationParameters) -> i32 {
    EvalState::from_position(position, parameters).score(position, parameters)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct EvalState {
    piece_score: [i32; 2],
    pawn_files: [[u8; 18]; 2],
    bishops: [u8; 2],
}

impl EvalState {
    #[must_use]
    pub fn from_position(position: &Position, parameters: &EvaluationParameters) -> Self {
        let mut state = Self {
            piece_score: [0; 2],
            pawn_files: [[0; 18]; 2],
            bishops: [0; 2],
        };
        let size = position.board().size();
        for (square, piece) in position.board().pieces() {
            state.add_piece(square, piece, size, parameters);
        }
        state
    }

    pub fn apply_move(
        &mut self,
        position: &Position,
        undo: &PositionUndo,
        parameters: &EvaluationParameters,
    ) {
        let size = position.board().size();
        for (square, old_piece) in undo.changed_squares() {
            if let Some(piece) = old_piece {
                self.remove_piece(square, piece, size, parameters);
            }
            if let Some(piece) = position.board().piece_at(square) {
                self.add_piece(square, piece, size, parameters);
            }
        }
    }

    #[must_use]
    pub fn score(&self, position: &Position, parameters: &EvaluationParameters) -> i32 {
        let size = position.board().size();
        let mut score = self.piece_score;
        for color in Color::ALL {
            let side = color.index();
            score[side] += pawn_structure(
                &self.pawn_files[side],
                usize::from(size.files()),
                parameters,
            );
            if self.bishops[side] >= 2 {
                score[side] += parameters.bishop_pair_bonus;
            }
            score[side] += king_shelter(position, color, parameters);
        }

        let white_score = score[Color::White.index()] - score[Color::Black.index()];
        let relative = match position.side_to_move() {
            Color::White => white_score,
            Color::Black => -white_score,
        };
        relative + parameters.tempo_bonus
    }

    fn add_piece(
        &mut self,
        square: Square,
        piece: Piece,
        size: BoardSize,
        parameters: &EvaluationParameters,
    ) {
        let side = piece.color.index();
        self.piece_score[side] += positional_piece_value(square, piece, size, parameters);
        if piece.kind == PieceKind::Pawn {
            self.pawn_files[side][usize::from(square.file())] += 1;
        } else if piece.kind == PieceKind::Bishop {
            self.bishops[side] += 1;
        }
    }

    fn remove_piece(
        &mut self,
        square: Square,
        piece: Piece,
        size: BoardSize,
        parameters: &EvaluationParameters,
    ) {
        let side = piece.color.index();
        self.piece_score[side] -= positional_piece_value(square, piece, size, parameters);
        if piece.kind == PieceKind::Pawn {
            self.pawn_files[side][usize::from(square.file())] -= 1;
        } else if piece.kind == PieceKind::Bishop {
            self.bishops[side] -= 1;
        }
    }
}

fn positional_piece_value(
    square: Square,
    piece: Piece,
    size: BoardSize,
    parameters: &EvaluationParameters,
) -> i32 {
    parameters.material_value(piece.kind)
        + centrality_bonus(square, size.files(), size.ranks(), piece.kind, parameters)
        + advancement_bonus(square, size.ranks(), piece.color, piece.kind, parameters)
}

fn centrality_bonus(
    square: Square,
    files: u8,
    ranks: u8,
    kind: PieceKind,
    parameters: &EvaluationParameters,
) -> i32 {
    let file_distance = (i16::from(square.file()) * 2 - i16::from(files - 1)).unsigned_abs();
    let rank_distance = (i16::from(square.rank()) * 2 - i16::from(ranks - 1)).unsigned_abs();
    let span = u16::from(files - 1) + u16::from(ranks - 1);
    let centrality = span.saturating_sub(file_distance + rank_distance);
    i32::from(centrality) * parameters.centrality_weight(kind)
}

fn advancement_bonus(
    square: Square,
    ranks: u8,
    color: Color,
    kind: PieceKind,
    parameters: &EvaluationParameters,
) -> i32 {
    let relative_rank = match color {
        Color::White => square.rank(),
        Color::Black => ranks - 1 - square.rank(),
    };
    i32::from(relative_rank) * parameters.advancement_weight(kind)
}

fn pawn_structure(files: &[u8; 18], board_files: usize, parameters: &EvaluationParameters) -> i32 {
    let mut score = 0;
    for file in 0..board_files {
        let count = files[file];
        if count > 1 {
            score -= i32::from(count - 1) * parameters.doubled_pawn_penalty;
        }
        if count > 0 {
            let left = file.checked_sub(1).is_some_and(|left| files[left] > 0);
            let right = file + 1 < board_files && files[file + 1] > 0;
            if !left && !right {
                score -= i32::from(count) * parameters.isolated_pawn_penalty;
            }
        }
    }
    score
}

fn king_shelter(position: &Position, color: Color, parameters: &EvaluationParameters) -> i32 {
    let Some(king) = position.board().king_square(color) else {
        return 0;
    };
    let mut shelter = 0;
    for file_delta in -1..=1 {
        for rank_delta in -1..=1 {
            if file_delta == 0 && rank_delta == 0 {
                continue;
            }
            if king
                .offset(file_delta, rank_delta)
                .filter(|square| position.board().size().contains(*square))
                .and_then(|square| position.board().piece_at(square))
                .is_some_and(|piece| piece.color == color)
            {
                shelter += parameters.king_shelter_bonus;
            }
        }
    }
    if position.king_jump_available(color) {
        shelter += parameters.king_jump_bonus;
    }
    shelter
}

#[cfg(test)]
mod tests {
    use super::*;
    use capablanca_chess_plus::Variant;

    fn position(pieces: &[(char, &str)], side: Color) -> Position {
        let mut squares = [[None; 16]; 16];
        for &(piece, coordinate) in pieces {
            let square: Square = coordinate.parse().unwrap();
            squares[usize::from(square.rank())][usize::from(square.file())] = Some(piece);
        }
        let ranks = (0..16)
            .rev()
            .map(|rank| {
                let mut encoded = String::new();
                let mut empty = 0;
                for square in squares[rank] {
                    if let Some(piece) = square {
                        if empty != 0 {
                            encoded.push_str(&empty.to_string());
                            empty = 0;
                        }
                        encoded.push(piece);
                    } else {
                        empty += 1;
                    }
                }
                if empty != 0 {
                    encoded.push_str(&empty.to_string());
                }
                encoded
            })
            .collect::<Vec<_>>();
        let side = if side == Color::White { 'w' } else { 'b' };
        Position::from_fen(
            Variant::TerachessII.rules(),
            &format!("{} {side} - - 0 1", ranks.join("/")),
        )
        .unwrap()
    }

    fn material_only() -> EvaluationParameters {
        let mut parameters = EvaluationParameters::published();
        for kind in PieceKind::ALL {
            parameters.set_centrality_weight(kind, 0);
            parameters.set_advancement_weight(kind, 0);
        }
        parameters.set_doubled_pawn_penalty(0);
        parameters.set_isolated_pawn_penalty(0);
        parameters.set_bishop_pair_bonus(0);
        parameters.set_king_shelter_bonus(0);
        parameters.set_king_jump_bonus(0);
        parameters.set_tempo_bonus(0);
        parameters
    }

    #[test]
    fn published_material_table_covers_every_piece_kind() {
        let expected = [
            100, 400, 680, 1_000, 1_660, 0, 1_060, 1_380, 1_000, 400, 360, 340, 660, 440, 2_040,
            1_200, 1_080, 820, 1_200, 880, 1_680, 1_220, 460, 1_640, 1_160, 480,
        ];
        let parameters = EvaluationParameters::published();
        for (kind, expected) in PieceKind::ALL.into_iter().zip(expected) {
            assert_eq!(parameters.material_value(kind), expected, "{kind:?}");
        }
    }

    #[test]
    fn production_material_table_covers_every_piece_kind() {
        let expected = [
            100, 392, 712, 997, 1_609, 0, 1_066, 1_405, 1_022, 392, 360, 347, 647, 439, 2_014,
            1_163, 1_154, 826, 1_147, 852, 1_462, 1_295, 466, 1_609, 1_178, 510,
        ];
        let parameters = EvaluationParameters::empirical_v1();
        for (kind, expected) in PieceKind::ALL.into_iter().zip(expected) {
            assert_eq!(parameters.material_value(kind), expected, "{kind:?}");
            assert_eq!(piece_value(kind), expected, "{kind:?}");
        }
    }

    #[test]
    fn material_profile_scores_an_exchange_by_the_configured_values() {
        let position = position(
            &[('K', "a1"), ('Q', "h8"), ('k', "p16"), ('r', "i9")],
            Color::White,
        );
        let mut parameters = material_only();
        assert_eq!(evaluate_with(&position, &parameters), 660);
        parameters.set_material_value(PieceKind::Queen, 1_500);
        assert_eq!(evaluate_with(&position, &parameters), 500);
    }

    #[test]
    fn color_rotated_positions_have_the_same_side_relative_score() {
        let original = position(
            &[('K', "a1"), ('Q', "g7"), ('k', "p16"), ('r', "m13")],
            Color::White,
        );
        let rotated = position(
            &[('k', "p16"), ('q', "j10"), ('K', "a1"), ('R', "d4")],
            Color::Black,
        );
        let parameters = EvaluationParameters::published();
        assert_eq!(
            evaluate_with(&original, &parameters),
            evaluate_with(&rotated, &parameters)
        );
    }

    #[test]
    fn a_balanced_position_contains_only_the_tempo_term() {
        let position = position(
            &[
                ('K', "a1"),
                ('R', "d4"),
                ('P', "e5"),
                ('k', "p16"),
                ('r', "m13"),
                ('p', "l12"),
            ],
            Color::White,
        );
        let parameters = EvaluationParameters::published();
        assert_eq!(
            evaluate_with(&position, &parameters),
            parameters.tempo_bonus()
        );
    }
}
