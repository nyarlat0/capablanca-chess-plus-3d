use capablanca_chess_plus::{
    CastleSide, Color, Move, MoveKind, Piece, PieceKind, Position, Square, Variant,
};
use std::fmt::Write;

fn kinds(position: &Position) -> Vec<PieceKind> {
    let mut kinds: Vec<_> = PieceKind::ALL
        .into_iter()
        .filter(|kind| {
            position.rules().uses_piece(*kind)
                || position
                    .board()
                    .pieces()
                    .any(|(_, piece)| piece.kind == *kind)
        })
        .collect();
    kinds.sort_by_key(|kind| kind.display_name());
    kinds
}

pub fn chess_rules(variant: Variant) -> String {
    crate::rules::render_rules(
        variant,
        &crate::Representation::default(),
        crate::rules::builtin_template(variant),
    )
    .expect("built-in rules template")
}

pub fn board_state(position: &Position) -> String {
    let rules = position.rules();
    let mut text = format!(
        "AUTHORITATIVE CURRENT POSITION\nSide to move: {:?}\n\n",
        position.side_to_move()
    );
    text.push_str(&render_rights(position, false));
    text.push_str("\nCURRENT PIECES\n");
    for color in [Color::White, Color::Black] {
        writeln!(text, "\n{color:?}:").unwrap();
        for kind in kinds(position) {
            // File, then numeric rank: a2 precedes a10.
            let mut squares: Vec<_> = position
                .board()
                .pieces()
                .filter(|(_, p)| p.color == color && p.kind == kind)
                .map(|(s, _)| s)
                .collect();
            squares.sort_by_key(|s| (s.file(), s.rank()));
            if !squares.is_empty() {
                writeln!(
                    text,
                    "{} {}: {}",
                    rules.piece_symbol(Piece::new(Color::White, kind)),
                    kind.display_name(),
                    squares
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join(", ")
                )
                .unwrap();
            }
        }
    }
    text.push_str(&ascii_board(position));
    text
}

pub(crate) fn ascii_board(position: &Position) -> String {
    let mut text = String::new();
    let size = position.board().size();
    let rules = position.rules();
    text.push_str("\nASCII BOARD\n");
    for rank in (0..size.ranks()).rev() {
        write!(text, "{:>2}", rank + 1).unwrap();
        for file in 0..size.files() {
            let symbol = position
                .board()
                .piece_at(Square::new(file, rank))
                .map(|p| rules.piece_symbol(p))
                .unwrap_or('.');
            write!(text, " {symbol}").unwrap();
        }
        text.push('\n');
    }
    text.push_str("  ");
    for file in 0..size.files() {
        write!(text, " {}", (b'a' + file) as char).unwrap();
    }
    text
}

/// Call on the pre-move position, after legality validation.
pub(crate) fn describe_move(position: &Position, mv: Move) -> String {
    let piece = position
        .board()
        .piece_at(mv.from)
        .expect("validated source");
    let captures = mv.kind == MoveKind::EnPassant || position.board().piece_at(mv.to).is_some();
    let mut text = format!(
        "{:?} {} {}{}{}",
        piece.color,
        piece.kind.display_name(),
        mv.from,
        if captures { 'x' } else { '-' },
        mv.to
    );
    if let Some(kind) = mv.promotion {
        write!(text, "={}", kind.display_name()).unwrap();
    }
    match mv.kind {
        MoveKind::EnPassant => text.push_str(" (en-passant)"),
        MoveKind::Castle(side) => {
            let route = position
                .rules()
                .castling()
                .route(piece.color, side)
                .unwrap();
            write!(
                text,
                " (castling; Rook {}-{})",
                route.rook_from, route.rook_to
            )
            .unwrap();
        }
        MoveKind::Normal => {
            if piece.kind == PieceKind::King
                && mv
                    .from
                    .file()
                    .abs_diff(mv.to.file())
                    .max(mv.from.rank().abs_diff(mv.to.rank()))
                    == 2
            {
                text.push_str(" (initial king jump)");
            }
        }
    }
    text
}

pub(crate) fn render_rights(position: &Position, numeric: bool) -> String {
    let rules = position.rules();
    let mut text = String::new();
    if rules.castling().any() {
        text.push_str("Castling:\n");
        for color in [Color::White, Color::Black] {
            for (side, name) in [
                (CastleSide::QueenSide, "queenside"),
                (CastleSide::KingSide, "kingside"),
            ] {
                let available = rules.castling().route(color, side).is_some()
                    && position.castling_rights().has(color, side);
                writeln!(
                    text,
                    "{color:?} {name}: {}",
                    if available { "yes" } else { "no" }
                )
                .unwrap();
            }
        }
    } else {
        text.push_str("Castling: none\n");
    }
    let legal_en_passant = position
        .legal_moves()
        .into_iter()
        .find(|mv| mv.kind == MoveKind::EnPassant);

    match legal_en_passant {
        Some(mv) => {
            writeln!(
                text,
                "\nEn passant: available to {}",
                crate::representation::coordinate(mv.to, numeric)
            )
            .unwrap();
        }
        None => {
            text.push_str("\nEn passant: none\n");
        }
    }
    if rules.king_initial_jump() {
        text.push_str("\nInitial king jump:\n");
        for color in [Color::White, Color::Black] {
            writeln!(
                text,
                "{color:?}: {}",
                if position.king_jump_available(color) {
                    "available"
                } else {
                    "unavailable"
                }
            )
            .unwrap();
        }
    }
    text
}
