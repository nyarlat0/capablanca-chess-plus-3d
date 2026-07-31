use bevy::prelude::*;
use capablanca_chess_plus::{
    CastleSide, Color as Side, DrawReason, Game, GameOutcome, Move, MoveKind, Piece, PieceKind,
    SearchResult, Square, Variant,
};

pub(crate) struct GamePlugin;

impl Plugin for GamePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ChessMatch>();
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Controller {
    Human,
    Computer,
}

impl Controller {
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Human => "Human",
            Self::Computer => "Computer",
        }
    }

    pub(crate) fn toggle(&mut self) {
        *self = match self {
            Self::Human => Self::Computer,
            Self::Computer => Self::Human,
        };
    }
}

#[derive(Resource)]
pub(crate) struct ChessMatch {
    pub(crate) game: Game,
    pub(crate) variant: Variant,
    pub(crate) controllers: [Controller; 2],
    pub(crate) selected: Option<Square>,
    pub(crate) pending_promotion: Option<PendingPromotion>,
    pub(crate) last_move: Option<Move>,
    pub(crate) status: String,
    pub(crate) generation: u64,
}

impl Default for ChessMatch {
    fn default() -> Self {
        let variant = Variant::Capablanca;
        Self {
            game: Game::new(variant.starting_position()),
            variant,
            controllers: [Controller::Human, Controller::Computer],
            selected: None,
            pending_promotion: None,
            last_move: None,
            status: "White to move.".to_owned(),
            generation: 0,
        }
    }
}

#[derive(Clone)]
pub(crate) struct PendingPromotion {
    pub(crate) moves: Vec<Move>,
}

pub(crate) fn restart_match(chess_match: &mut ChessMatch, variant: Variant) {
    chess_match.game = Game::new(variant.starting_position());
    chess_match.variant = variant;
    chess_match.selected = None;
    chess_match.pending_promotion = None;
    chess_match.last_move = None;
    chess_match.status = format!("New {} game. White to move.", variant.rules().name());
    chess_match.generation = chess_match.generation.wrapping_add(1);
}

pub(crate) fn handle_square_selection(chess_match: &mut ChessMatch, square: Square) {
    if chess_match.pending_promotion.is_some() {
        return;
    }
    if !is_playable(chess_match.game.outcome()) {
        chess_match.status = outcome_message(chess_match.game.outcome());
        return;
    }

    let side = chess_match.game.position().side_to_move();
    if chess_match.controllers[side.index()] == Controller::Computer {
        chess_match.status = format!("{} is controlled by the computer.", side_name(side));
        return;
    }

    if let Some(from) = chess_match.selected {
        let candidates: Vec<_> = chess_match
            .game
            .position()
            .legal_moves()
            .into_iter()
            .filter(|chess_move| chess_move.from == from && chess_move.to == square)
            .collect();
        match candidates.as_slice() {
            [] => {
                if selectable_piece(chess_match, square) {
                    select_square(chess_match, square);
                } else {
                    chess_match.selected = None;
                    chess_match.status = format!("{square} is not a legal destination.");
                }
            }
            [chess_move] => apply_move(chess_match, *chess_move, None),
            _ => {
                chess_match.status = promotion_prompt(&candidates);
                chess_match.pending_promotion = Some(PendingPromotion { moves: candidates });
            }
        }
    } else if selectable_piece(chess_match, square) {
        select_square(chess_match, square);
    } else {
        chess_match.status = format!("Select a {} piece.", side_name(side).to_ascii_lowercase());
    }
}

fn selectable_piece(chess_match: &ChessMatch, square: Square) -> bool {
    let position = chess_match.game.position();
    position
        .board()
        .piece_at(square)
        .is_some_and(|piece| piece.color == position.side_to_move())
        && position
            .legal_moves()
            .iter()
            .any(|chess_move| chess_move.from == square)
}

fn select_square(chess_match: &mut ChessMatch, square: Square) {
    chess_match.selected = Some(square);
    let count = chess_match
        .game
        .position()
        .legal_moves()
        .iter()
        .filter(|chess_move| chess_move.from == square)
        .map(|chess_move| chess_move.to)
        .collect::<std::collections::HashSet<_>>()
        .len();
    chess_match.status = format!("{square} selected: {count} destination(s).");
}

pub(crate) fn apply_move(
    chess_match: &mut ChessMatch,
    chess_move: Move,
    analysis: Option<&SearchResult>,
) {
    let position = chess_match.game.position();
    let moving_piece = position
        .board()
        .piece_at(chess_move.from)
        .expect("a legal move has a source piece");
    let is_capture = matches!(chess_move.kind, MoveKind::EnPassant)
        || position.board().piece_at(chess_move.to).is_some();
    let description = describe_move(moving_piece, chess_move, is_capture);

    chess_match
        .game
        .play(chess_move)
        .expect("only engine-provided legal moves are applied");
    chess_match.selected = None;
    chess_match.pending_promotion = None;
    chess_match.last_move = Some(chess_move);
    chess_match.generation = chess_match.generation.wrapping_add(1);

    let analysis_text = analysis.map_or_else(String::new, |result| {
        format!(
            "  Evaluation {:+.2}, depth {}, {} nodes.",
            f64::from(result.score) / 100.0,
            result.depth,
            result.nodes
        )
    });
    let outcome = chess_match.game.outcome();
    chess_match.status = format!("{description}.{analysis_text} {}", outcome_message(outcome));
}

fn describe_move(piece: Piece, chess_move: Move, capture: bool) -> String {
    if let MoveKind::Castle(side) = chess_move.kind {
        return format!(
            "{} castles {}",
            side_name(piece.color),
            match side {
                CastleSide::QueenSide => "queen-side",
                CastleSide::KingSide => "king-side",
            }
        );
    }
    let separator = if capture { "x" } else { "-" };
    let mut value = format!(
        "{} {} {}{}{}",
        side_name(piece.color),
        piece_name(piece.kind),
        chess_move.from,
        separator,
        chess_move.to
    );
    if let Some(promotion) = chess_move.promotion {
        value.push_str(&format!(" promotes to {}", piece_name(promotion)));
    }
    value
}

pub(crate) const fn side_name(side: Side) -> &'static str {
    match side {
        Side::White => "White",
        Side::Black => "Black",
    }
}

pub(crate) const fn piece_name(kind: PieceKind) -> &'static str {
    match kind {
        PieceKind::Pawn => "pawn",
        PieceKind::Knight => "knight",
        PieceKind::Bishop => "bishop",
        PieceKind::Rook => "rook",
        PieceKind::Queen => "queen",
        PieceKind::King => "king",
        PieceKind::Archbishop => "archbishop",
        PieceKind::Chancellor => "chancellor",
    }
}

pub(crate) fn side_to_move_message(chess_match: &ChessMatch) -> String {
    let side = chess_match.game.position().side_to_move();
    format!("{} to move.", side_name(side))
}

pub(crate) const fn is_playable(outcome: GameOutcome) -> bool {
    matches!(outcome, GameOutcome::Ongoing | GameOutcome::Check)
}

pub(crate) fn outcome_message(outcome: GameOutcome) -> String {
    match outcome {
        GameOutcome::Ongoing => "Game in progress.".to_owned(),
        GameOutcome::Check => "Check.".to_owned(),
        GameOutcome::Win { winner } => {
            format!("Checkmate — {} wins.", side_name(winner))
        }
        GameOutcome::Draw(reason) => match reason {
            DrawReason::Stalemate => "Draw by stalemate.".to_owned(),
            DrawReason::FiftyMoveRule => "Draw by the fifty-move rule.".to_owned(),
            DrawReason::ThreefoldRepetition => "Draw by threefold repetition.".to_owned(),
        },
    }
}

pub(crate) fn promotion_prompt(moves: &[Move]) -> String {
    let mut choices = Vec::new();
    for chess_move in moves {
        let choice = match chess_move.promotion {
            Some(PieceKind::Queen) => "Q queen",
            Some(PieceKind::Chancellor) => "C chancellor",
            Some(PieceKind::Archbishop) => "A archbishop",
            Some(PieceKind::Rook) => "R rook",
            Some(PieceKind::Bishop) => "B bishop",
            Some(PieceKind::Knight) => "N knight",
            Some(PieceKind::Pawn | PieceKind::King) => continue,
            None => "Space no promotion",
        };
        if !choices.contains(&choice) {
            choices.push(choice);
        }
    }
    format!("Choose promotion: {}.", choices.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn promotion_prompt_exposes_compound_piece_keys() {
        let from = Square::new(0, 6);
        let to = Square::new(0, 7);
        let moves = [
            Move::promotion(from, to, PieceKind::Queen),
            Move::promotion(from, to, PieceKind::Chancellor),
            Move::promotion(from, to, PieceKind::Archbishop),
        ];
        let prompt = promotion_prompt(&moves);
        assert!(prompt.contains("Q queen"));
        assert!(prompt.contains("C chancellor"));
        assert!(prompt.contains("A archbishop"));
    }
}
