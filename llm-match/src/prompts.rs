use capablanca_chess_plus::{
    CastleSide, Color, Move, MoveKind, Piece, PieceKind, Position, PromotionRule, Square, Variant,
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
    let position = variant.starting_position();
    let rules = position.rules();
    let size = rules.board_size();
    let mut text = format!(
        "You are playing {} on a {}x{} board, files a-{}, ranks 1-{}. White advances toward increasing ranks; Black toward decreasing ranks. Uppercase symbols are White, lowercase Black.\n",
        rules.name(),
        size.files(),
        size.ranks(),
        (b'a' + size.files() - 1) as char,
        size.ranks()
    );
    text.push_str("PIECE LEGEND\nThese symbol meanings are authoritative. Never infer piece identities from standard chess, memory, starting squares, or the letter itself.\n");
    for kind in kinds(&position) {
        writeln!(
            text,
            "{} = {}: {}",
            rules.piece_symbol(Piece::new(Color::White, kind)),
            kind.display_name(),
            kind.movement_description()
        )
        .unwrap();
    }
    text.push_str("\nVARIANT RULES\n");
    if variant == Variant::Gothic {
        text.push_str("Initial back rank (not the current position), a-j: R N B Q C K A B N R\n");
    }
    if !rules.castling().any() {
        text.push_str("Castling: none\n");
    }
    for color in [Color::White, Color::Black] {
        for side in CastleSide::ALL {
            if let Some(route) = rules.castling().route(color, side) {
                writeln!(text, "{color:?} {side:?} castling: king {}->{}, rook {}->{}; send only the king coordinates. Requires retained rights, an empty route and no check on the king's start, transit or destination.", route.king_from, route.king_to, route.rook_from, route.rook_to).unwrap();
            }
        }
    }
    if rules.rapid_pawns() {
        text.push_str("Pawns and princes may advance two squares from any rank if both squares are empty. A pawn can capture a double-stepped pawn or prince en passant on the next move, even if that move promoted the victim. Princes and trolls cannot capture en passant.\n");
    } else {
        writeln!(text, "Pawn double-step only from White rank {} / Black rank {}, with both squares empty. Ordinary en passant applies.", rules.pawn_start_rank(Color::White)+1, rules.pawn_start_rank(Color::Black)+1).unwrap();
    }
    if rules.king_initial_jump() {
        text.push_str("Initial king jump: while its right remains, a king not in check may jump to an empty square at Chebyshev distance two; destination must be safe. Straight/diagonal jumps require the intermediate square safe; knight-shaped jumps require at least one of the two intermediate squares safe. Intervening pieces do not block this jump. Any king move consumes the right.\n");
    }
    match rules.promotion() {
        PromotionRule::LastRank { choices } => {
            let choices = choices.iter().map(|k| format!("{}={}", k.fen_char(), k.display_name())).collect::<Vec<_>>().join(", ");
            writeln!(text, "Pawn promotion on the final rank is mandatory; append one suffix: {choices}.").unwrap();
        }
        PromotionRule::Grand => text.push_str("Grand promotion is optional on the last three ranks and compulsory on the final rank; restore only a missing originally available non-pawn/non-king piece. If none is missing, a pawn cannot enter the final rank, but still attacks it. Suffixes: q=Queen, r=Rook, b=Bishop, n=Knight, a=Archbishop, c=Chancellor; availability follows missing original material.\n"),
        PromotionRule::TerachessII => text.push_str("Compulsory final-rank promotion: Pawn/Troll -> Queen (q); Prince -> Amazon (a); Knight/Camel/Giraffe -> Buffalo (f); Elephant/Machine/Centaur -> Lion (l). Append the indicated suffix. Troll promotion applies only to its pawn-like move, not its three-square leap. Other pieces do not promote.\n"),
    }
    text
}

pub fn board_state(position: &Position) -> String {
    let size = position.board().size();
    let rules = position.rules();
    let mut text = format!(
        "AUTHORITATIVE CURRENT POSITION\nSide to move: {:?}\n\n",
        position.side_to_move()
    );
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
    writeln!(
        text,
        "\nEn passant: {}",
        position
            .en_passant()
            .map(|s| s.to_string())
            .unwrap_or_else(|| "none".into())
    )
    .unwrap();
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
