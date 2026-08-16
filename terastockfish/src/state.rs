use crate::evaluate::EvalState;
use crate::key::{PositionKeys, position_keys, position_keys_after_move};
use capablanca_chess_plus::{Move, Position, PositionUndo};

#[derive(Clone)]
pub(crate) struct SearchPosition {
    position: Position,
    keys: PositionKeys,
    evaluation: EvalState,
}

#[derive(Clone, Copy)]
pub(crate) struct SearchUndo {
    position: PositionUndo,
    keys: PositionKeys,
    evaluation: EvalState,
}

impl SearchPosition {
    #[must_use]
    pub fn new(position: &Position) -> Self {
        Self {
            position: position.clone(),
            keys: position_keys(position),
            evaluation: EvalState::from_position(position),
        }
    }

    #[must_use]
    pub const fn position(&self) -> &Position {
        &self.position
    }

    #[must_use]
    pub fn legal_moves(&mut self) -> Vec<Move> {
        self.position.legal_moves_mut()
    }

    #[must_use]
    pub const fn keys(&self) -> PositionKeys {
        self.keys
    }

    #[must_use]
    pub fn evaluate(&self) -> i32 {
        self.evaluation.score(&self.position)
    }

    #[must_use]
    pub fn make_move(&mut self, chess_move: Move) -> SearchUndo {
        let keys = self.keys;
        let evaluation = self.evaluation;
        let position = self.position.make_move(chess_move);
        self.keys = position_keys_after_move(&self.position, keys, &position);
        self.evaluation.apply_move(&self.position, &position);
        SearchUndo {
            position,
            keys,
            evaluation,
        }
    }

    pub fn unmake_move(&mut self, undo: SearchUndo) {
        self.position.unmake_move(undo.position);
        self.keys = undo.keys;
        self.evaluation = undo.evaluation;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evaluate::evaluate;
    use crate::key::position_keys;
    use capablanca_chess_plus::Variant;

    fn assert_incremental_state(state: &SearchPosition) {
        let rebuilt_keys = position_keys(state.position());
        assert_eq!(state.keys.analysis, rebuilt_keys.analysis);
        assert_eq!(state.keys.repetition, rebuilt_keys.repetition);
        assert_eq!(state.evaluate(), evaluate(state.position()));
    }

    #[test]
    fn incremental_state_matches_full_rebuild_through_a_line_and_every_undo() {
        let original = Variant::TerachessII.starting_position();
        let mut state = SearchPosition::new(&original);
        let mut undos = Vec::new();
        for ply in 0..24 {
            let moves = state.legal_moves();
            assert!(!moves.is_empty());
            let chess_move = moves[(ply * 17 + 3) % moves.len()];
            undos.push(state.make_move(chess_move));
            assert_incremental_state(&state);
        }
        while let Some(undo) = undos.pop() {
            state.unmake_move(undo);
            assert_incremental_state(&state);
        }
        assert_eq!(state.position(), &original);
    }

    #[test]
    fn incremental_state_handles_castling_en_passant_and_promotion() {
        let cases = [
            (Variant::Gothic, "5k4/10/10/10/10/10/10/R4K3R w KQ - 0 1"),
            (Variant::Gothic, "9k/10/10/4Pp4/10/10/10/K9 w - f6 0 1"),
            (Variant::Gothic, "9k/P9/10/10/10/10/10/K9 w - - 0 1"),
        ];
        for (variant, fen) in cases {
            let original = Position::from_fen(variant.rules(), fen).unwrap();
            for chess_move in original.legal_moves() {
                let mut state = SearchPosition::new(&original);
                let undo = state.make_move(chess_move);
                assert_incremental_state(&state);
                state.unmake_move(undo);
                assert_incremental_state(&state);
                assert_eq!(state.position(), &original);
            }
        }
    }
}
