use crate::{HistoryFormat, HistoryMode, OutputFormat, Representation, StateFormat};
use anyhow::{Context, Result, ensure};
use capablanca_chess_plus::{BoardSize, Color, Move, MoveKind, PieceKind, Position, Square};
use prompt_core::{PromptMessage, PromptRole};
use serde::Deserialize;
use std::fmt::Write;

pub fn numeric_square(square: Square) -> [u16; 2] {
    [u16::from(square.file()) + 1, u16::from(square.rank()) + 1]
}
pub fn square_from_numeric(value: [u16; 2], size: BoardSize) -> Result<Square> {
    ensure!(
        value[0] >= 1
            && value[0] <= u16::from(size.files())
            && value[1] >= 1
            && value[1] <= u16::from(size.ranks()),
        "coordinate outside {}x{} board",
        size.files(),
        size.ranks()
    );
    Ok(Square::new((value[0] - 1) as u8, (value[1] - 1) as u8))
}
pub(crate) fn coordinate(square: Square, numeric: bool) -> String {
    if numeric {
        let [x, y] = numeric_square(square);
        format!("({x},{y})")
    } else {
        square.to_string()
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NumericMove {
    from: [u16; 2],
    to: [u16; 2],
    promotion: serde_json::Value,
}

/// Representation conversion ONLY: no legality lookup, repair, or suggestions.
pub fn parse_model_move(answer: &str, format: OutputFormat, position: &Position) -> Result<String> {
    Ok(decode_model_move(answer, format, position)?.0)
}

fn strip_optional_json_fence(input: &str) -> &str {
    let input = input.trim();

    if !input.starts_with("```") || !input.ends_with("```") {
        return input;
    }

    let inner = &input[3..input.len() - 3];
    let inner = inner.trim();

    if let Some(rest) = inner.strip_prefix("json") {
        return rest.trim_start_matches([' ', '\t', '\r', '\n']).trim();
    }

    inner
}

// Preserve the explicit JSON identity until engine validation: different fairy
// pieces can share a canonical promotion suffix across variants.
pub(crate) fn decode_model_move(
    answer: &str,
    format: OutputFormat,
    position: &Position,
) -> Result<(String, Option<Option<PieceKind>>)> {
    let answer = answer.trim();
    if matches!(format, OutputFormat::Semantic | OutputFormat::Numeric) {
        return crate::move_text::decode(answer, format, position);
    }
    if format == OutputFormat::Uci {
        ensure!(
            crate::output::is_coordinate_move(answer),
            "expected one canonical UCI move"
        );
        return Ok((answer.into(), None));
    }

    let answer = strip_optional_json_fence(answer);

    let mv: NumericMove =
        serde_json::from_str(answer).context("expected exactly one numeric move JSON object")?;
    let from = square_from_numeric(mv.from, position.board().size())?;
    let to = square_from_numeric(mv.to, position.board().size())?;
    let promotion = match mv.promotion {
        serde_json::Value::Null => None,
        serde_json::Value::String(name) => Some(
            PieceKind::ALL
                .into_iter()
                .find(|p| p.display_name() == name)
                .context("unknown exact promotion piece name")?,
        ),
        _ => anyhow::bail!("promotion must be null or an exact piece name"),
    };
    // Some variants share canonical letters (Amazon/Archbishop, Cannon/Chancellor).
    // Do not let an explicit wrong identity turn into a different legal piece.
    if let Some(kind) = promotion {
        ensure!(
            position.rules().uses_piece(kind),
            "promotion piece does not belong to this variant"
        );
    }
    Ok((
        format!(
            "{from}{to}{}",
            promotion
                .map(|p| p.fen_char().to_string())
                .unwrap_or_default()
        ),
        Some(promotion),
    ))
}

pub fn render_state(position: &Position, format: StateFormat) -> String {
    if format == StateFormat::PiecesAscii {
        let mut out = crate::board_state(position);
        writeln!(
            out,
            "\nHalfmove clock: {}\nFullmove number: {}",
            position.halfmove_clock(),
            position.fullmove_number()
        )
        .unwrap();
        append_initial_material(position, &mut out);
        return out;
    }
    let size = position.board().size();
    let mut out = String::from(
        "AUTHORITATIVE CURRENT POSITION\nCOORDINATE SYSTEM\nx increases from file a toward the right:\n",
    );
    out.push_str(
        &(0..size.files())
            .map(|f| format!("{}={}", (b'a' + f) as char, f + 1))
            .collect::<Vec<_>>()
            .join(", "),
    );
    out.push_str("\ny is the board rank (rank 1 = y=1, rank 2 = y=2, and so on).\nCoordinates are absolute and NEVER mirrored or rotated for Black.\n");
    writeln!(
        out,
        "\nBOARD\nwidth: {}\nheight: {}\n\nSIDE TO MOVE\n{:?}\n",
        size.files(),
        size.ranks(),
        position.side_to_move()
    )
    .unwrap();
    out.push_str(&crate::prompts::render_rights(position, true));
    writeln!(
        out,
        "\nHalfmove clock: {}\nFullmove number: {}",
        position.halfmove_clock(),
        position.fullmove_number()
    )
    .unwrap();
    for (color, title) in [
        (Color::White, "WHITE PIECES"),
        (Color::Black, "BLACK PIECES"),
    ] {
        writeln!(out, "\n{title}").unwrap();
        let mut pieces: Vec<_> = position
            .board()
            .pieces()
            .filter(|(_, p)| p.color == color)
            .collect();
        pieces.sort_by_key(|(s, p)| (p.kind.display_name(), s.file(), s.rank()));
        for (s, p) in pieces {
            writeln!(out, "{}: {}", p.kind.display_name(), coordinate(s, true)).unwrap();
        }
    }
    append_initial_material(position, &mut out);
    if format == StateFormat::NumericAscii {
        out.push('\n');
        out.push_str(&crate::prompts::ascii_board(position));
    }
    out
}

fn append_initial_material(position: &Position, out: &mut String) {
    // Grand restoration depends on initial material, not just generic piece rules.
    if matches!(
        position.rules().promotion(),
        capablanca_chess_plus::PromotionRule::Grand
    ) {
        out.push_str("\nINITIAL MATERIAL COUNTS (restoration limits)\n");
        for color in [Color::White, Color::Black] {
            for kind in PieceKind::ALL {
                let n = position.rules().initial_count(color, kind);
                if n > 0 {
                    writeln!(out, "{color:?} {}: {n}", kind.display_name()).unwrap();
                }
            }
        }
    }
}

pub(crate) fn numeric_move(position: &Position, mv: Move, role: PromptRole) -> PromptMessage {
    let piece = position
        .board()
        .piece_at(mv.from)
        .expect("validated source");
    let capture = mv.kind == MoveKind::EnPassant || position.board().piece_at(mv.to).is_some();
    let mut content = format!(
        "{:?} {}: {} {} {}",
        piece.color,
        piece.kind.display_name(),
        coordinate(mv.from, true),
        if capture { "x" } else { "->" },
        coordinate(mv.to, true)
    );
    if let Some(p) = mv.promotion {
        write!(content, " = {}", p.display_name()).unwrap();
    }
    match mv.kind {
        MoveKind::EnPassant => content.push_str(" (en-passant)"),
        MoveKind::Castle(side) => {
            let r = position
                .rules()
                .castling()
                .route(piece.color, side)
                .unwrap();
            write!(
                content,
                " (castling; Rook {} -> {})",
                coordinate(r.rook_from, true),
                coordinate(r.rook_to, true)
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
                content.push_str(" (initial king jump)");
            }
        }
    }
    PromptMessage {
        role,
        name: None,
        content,
    }
}

pub fn render_history(
    semantic: &[PromptMessage],
    numeric: &[PromptMessage],
    profile: &Representation,
) -> Vec<PromptMessage> {
    let source = if profile.history_format == HistoryFormat::Semantic {
        semantic
    } else {
        numeric
    };
    source[profile.history_start(source.len())..].to_vec()
}
pub fn history_instruction(profile: &Representation, shown: usize) -> String {
    if shown == 0 {
        return "MOVE HISTORY: none supplied. Decide from the current authoritative state, not an assumed previous move.".into();
    }
    match profile.history_mode {
        HistoryMode::None => unreachable!(),
        HistoryMode::Full => format!(
            "MOVE HISTORY: all {shown} accepted moves, oldest first. Informational only; use current state as authoritative."
        ),
        HistoryMode::LastN => format!(
            "{}: only the last {shown} accepted move(s), oldest first; this is NOT the complete game history. Use the current state as authoritative.",
            if shown == 1 {
                "LAST MOVE"
            } else {
                "RECENT MOVES"
            }
        ),
    }
}
pub fn output_instruction(format: OutputFormat) -> &'static str {
    match format {
        OutputFormat::Uci => {
            "Output exactly one raw canonical UCI move: source immediately followed by destination and lowercase promotion suffix when required. No SAN, hyphens, prose, markdown or extra text."
        }
        OutputFormat::NumericJson => {
            "Output exactly one JSON object with exactly these keys: {\"from\":[x,y],\"to\":[x,y],\"promotion\":null}. Use integer absolute coordinates. If promotion is required, replace null with the exact promoted piece name from the legend (for example \"Queen\"). No file letters, markdown, prose, additional keys or additional objects."
        }
        OutputFormat::Semantic => {
            "Output exactly one move in the SAME format as semantic accepted history: <Color> <PieceName> <from>-<to>. Use White or Black and exact piece names from the legend; squares use file letters and ranks. For a capture replace - with x. For promotion append =<PromotionPiece>. Append exactly ' (en-passant)' for en passant, ' (initial king jump)' for that jump, or ' (castling; Rook <rook-from>-<rook-to>)' for castling. Moving piece identity is from the CURRENT position before the move. No raw UCI, JSON, SAN, markdown, commentary or extra lines."
        }
        OutputFormat::Numeric => {
            "Output exactly one move in the SAME format as numeric accepted history: <Color> <PieceName>: (x,y) -> (x,y). Use White or Black, exact piece names from the legend and absolute integer coordinates without spaces inside parentheses. For a capture replace ' -> ' with ' x '. For promotion append ' = <PromotionPiece>'. Append exactly ' (en-passant)' for en passant, ' (initial king jump)' for that jump, or ' (castling; Rook (x,y) -> (x,y))' for castling. Moving piece identity is from the CURRENT position before the move. No file letters, raw UCI, JSON, SAN, markdown, commentary or extra lines."
        }
    }
}
pub fn grounding(format: StateFormat) -> &'static str {
    match format {
        StateFormat::PiecesAscii => {
            "CURRENT PIECES and the ASCII board are authoritative. Never reconstruct position from history, standard starting squares, familiar openings or assumed identities."
        }
        StateFormat::Numeric => {
            "The numeric coordinate state and WHITE PIECES / BLACK PIECES are authoritative. Coordinates are absolute, never mirrored. Verify source identity and exact geometric displacement from this state; never reconstruct position from history or standard chess assumptions."
        }
        StateFormat::NumericAscii => {
            "The numeric piece lists and ASCII board describe the same authoritative state. Coordinates are absolute, never mirrored. Verify actual source identity and geometric displacement. Never reconstruct position from history or standard chess assumptions."
        }
    }
}
