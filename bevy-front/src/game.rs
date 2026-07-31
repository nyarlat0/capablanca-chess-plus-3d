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

#[derive(Resource)]
pub(crate) struct ChessMatch {
    pub(crate) game: Game,
    pub(crate) variant: Variant,
    pub(crate) controllers: [Controller; 2],
    pub(crate) selected: Option<Square>,
    pub(crate) pending_promotion: Option<PendingPromotion>,
    pub(crate) last_move: Option<Move>,
    pub(crate) captured_pieces: Vec<CapturedPiece>,
    pub(crate) status: String,
    pub(crate) generation: u64,
    next_capture_id: u64,
}

impl Default for ChessMatch {
    fn default() -> Self {
        let variant = Variant::Gothic;
        Self {
            game: Game::new(variant.starting_position()),
            variant,
            controllers: [Controller::Human, Controller::Human],
            selected: None,
            pending_promotion: None,
            last_move: None,
            captured_pieces: Vec::new(),
            status: "White to move.".to_owned(),
            generation: 0,
            next_capture_id: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CapturedPiece {
    pub(crate) id: u64,
    pub(crate) piece: Piece,
    pub(crate) captured_by: Side,
    pub(crate) from: Square,
    pub(crate) tray_slot: usize,
    pub(crate) generation: u64,
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
    chess_match.captured_pieces.clear();
    chess_match.next_capture_id = 0;
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
            [chess_move] if chess_move.promotion.is_none() => {
                apply_move(chess_match, *chess_move, None);
            }
            _ => {
                chess_match.status = "Choose a promotion.".to_owned();
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
    let captured_square = match chess_move.kind {
        MoveKind::EnPassant => Some(Square::new(chess_move.to.file(), chess_move.from.rank())),
        _ if is_capture => Some(chess_move.to),
        _ => None,
    };
    let captured_piece = captured_square.and_then(|square| position.board().piece_at(square));
    let description = describe_move(moving_piece, chess_move, is_capture);

    chess_match
        .game
        .play(chess_move)
        .expect("only engine-provided legal moves are applied");
    chess_match.selected = None;
    chess_match.pending_promotion = None;
    chess_match.last_move = Some(chess_move);
    chess_match.generation = chess_match.generation.wrapping_add(1);
    if let (Some(piece), Some(from)) = (captured_piece, captured_square) {
        let tray_slot = first_free_capture_slot(&chess_match.captured_pieces, moving_piece.color);
        let id = chess_match.next_capture_id;
        chess_match.captured_pieces.push(CapturedPiece {
            id,
            piece,
            captured_by: moving_piece.color,
            from,
            tray_slot,
            generation: chess_match.generation,
        });
        chess_match.next_capture_id = chess_match.next_capture_id.wrapping_add(1);
    }
    if chess_match.variant == Variant::Grand
        && let Some(promoted_kind) = chess_move.promotion
    {
        remove_resurrected_piece(
            &mut chess_match.captured_pieces,
            moving_piece.color,
            promoted_kind,
        );
    }

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

fn first_free_capture_slot(captured_pieces: &[CapturedPiece], captured_by: Side) -> usize {
    (0..)
        .find(|slot| {
            !captured_pieces
                .iter()
                .any(|captured| captured.captured_by == captured_by && captured.tray_slot == *slot)
        })
        .expect("the finite piece set always has a free capture-tray slot")
}

fn remove_resurrected_piece(
    captured_pieces: &mut Vec<CapturedPiece>,
    resurrected_side: Side,
    resurrected_kind: PieceKind,
) {
    if let Some(index) = captured_pieces
        .iter()
        .rposition(|captured| captured.piece == Piece::new(resurrected_side, resurrected_kind))
    {
        captured_pieces.remove(index);
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn applying_a_capture_records_its_piece_square_and_capturer() {
        let position = capablanca_chess_plus::Position::from_fen(
            Variant::Gothic.rules(),
            "9k/10/10/10/10/10/1n8/KR8 w - - 0 1",
        )
        .expect("test position is valid");
        let chess_move = position
            .parse_uci_move("b1b2")
            .expect("rook capture is legal");
        let mut chess_match = ChessMatch {
            game: Game::new(position),
            ..default()
        };

        apply_move(&mut chess_match, chess_move, None);

        let [captured] = chess_match.captured_pieces.as_slice() else {
            panic!("exactly one piece should have been captured");
        };
        assert_eq!(captured.piece, Piece::new(Side::Black, PieceKind::Knight));
        assert_eq!(captured.captured_by, Side::White);
        assert_eq!(captured.from, Square::new(1, 1));
        assert_eq!(captured.tray_slot, 0);
        assert_eq!(captured.generation, chess_match.generation);
    }

    #[test]
    fn grand_resurrection_removes_the_matching_captured_piece() {
        let mut captured = vec![
            CapturedPiece {
                id: 0,
                piece: Piece::new(Side::White, PieceKind::Queen),
                captured_by: Side::Black,
                from: Square::new(3, 1),
                tray_slot: 0,
                generation: 1,
            },
            CapturedPiece {
                id: 1,
                piece: Piece::new(Side::Black, PieceKind::Queen),
                captured_by: Side::White,
                from: Square::new(3, 8),
                tray_slot: 0,
                generation: 2,
            },
        ];

        remove_resurrected_piece(&mut captured, Side::White, PieceKind::Queen);

        assert_eq!(captured.len(), 1);
        assert_eq!(captured[0].piece.color, Side::Black);
    }

    #[test]
    fn grand_promotion_resurrects_a_piece_in_an_actual_game() {
        let position = capablanca_chess_plus::Position::from_fen(
            Variant::Grand.rules(),
            "9k/10/10/P9/10/10/10/10/10/K9 w - - 0 1",
        )
        .expect("test Grand position is valid");
        let chess_move = position
            .parse_uci_move("a7a8q")
            .expect("Grand queen resurrection is legal");
        let mut chess_match = ChessMatch {
            game: Game::new(position),
            variant: Variant::Grand,
            ..default()
        };
        chess_match.captured_pieces.push(CapturedPiece {
            id: 0,
            piece: Piece::new(Side::White, PieceKind::Queen),
            captured_by: Side::Black,
            from: Square::new(4, 1),
            tray_slot: 0,
            generation: 1,
        });

        apply_move(&mut chess_match, chess_move, None);

        assert!(chess_match.captured_pieces.is_empty());
        assert_eq!(
            chess_match
                .game
                .position()
                .board()
                .piece_at(Square::new(0, 7)),
            Some(Piece::new(Side::White, PieceKind::Queen))
        );
    }
}
