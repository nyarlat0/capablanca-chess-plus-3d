use crate::{Color, Move, MoveError, Position};
use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::Hasher;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DrawReason {
    Stalemate,
    FiftyMoveRule,
    ThreefoldRepetition,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GameOutcome {
    Ongoing,
    Check,
    Win { winner: Color },
    Draw(DrawReason),
}

/// A position plus repetition history.
#[derive(Clone, Debug)]
pub struct Game {
    position: Position,
    repetitions: HashMap<u64, u8>,
}

impl Game {
    #[must_use]
    pub fn new(position: Position) -> Self {
        let mut game = Self {
            position,
            repetitions: HashMap::new(),
        };
        game.record_position();
        game
    }

    #[must_use]
    pub const fn position(&self) -> &Position {
        &self.position
    }

    pub fn play(&mut self, chess_move: Move) -> Result<(), MoveError> {
        self.position.play(chess_move)?;
        self.record_position();
        Ok(())
    }

    pub fn play_uci(&mut self, value: &str) -> Result<Move, MoveError> {
        let chess_move = self.position.parse_uci_move(value)?;
        self.position.play(chess_move)?;
        self.record_position();
        Ok(chess_move)
    }

    #[must_use]
    pub fn outcome(&self) -> GameOutcome {
        let in_check = self.position.is_in_check(self.position.side_to_move());
        let no_moves = self.position.legal_moves().is_empty();
        if no_moves {
            return if in_check {
                GameOutcome::Win {
                    winner: self.position.side_to_move().opposite(),
                }
            } else {
                GameOutcome::Draw(DrawReason::Stalemate)
            };
        }
        if self.position.halfmove_clock() >= 100 {
            return GameOutcome::Draw(DrawReason::FiftyMoveRule);
        }
        if self
            .repetitions
            .get(&position_hash(&self.position))
            .is_some_and(|count| *count >= 3)
        {
            return GameOutcome::Draw(DrawReason::ThreefoldRepetition);
        }
        if in_check {
            GameOutcome::Check
        } else {
            GameOutcome::Ongoing
        }
    }

    fn record_position(&mut self) {
        let hash = position_hash(&self.position);
        let count = self.repetitions.entry(hash).or_default();
        *count = count.saturating_add(1);
    }
}

fn position_hash(position: &Position) -> u64 {
    let mut hasher = DefaultHasher::new();
    position.hash_repetition_state(&mut hasher);
    hasher.finish()
}
