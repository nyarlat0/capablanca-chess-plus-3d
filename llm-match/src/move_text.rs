//! Strict whole-answer grammar for outputs identical to accepted history.
//! No SAN, substring extraction, source inference, or legal-move search.
use crate::{OutputFormat, numeric_square, square_from_numeric};
use anyhow::{Context, Result, ensure};
use capablanca_chess_plus::{BoardSize, PieceKind, Position, Square};

fn kind(name: &str) -> Result<PieceKind> {
    PieceKind::ALL
        .into_iter()
        .find(|p| p.display_name() == name)
        .context("expected exact piece name")
}
fn square(text: &str, numeric: bool, size: BoardSize) -> Result<Square> {
    if numeric {
        let body = text
            .strip_prefix('(')
            .and_then(|s| s.strip_suffix(')'))
            .context("expected (x,y)")?;
        let (x, y) = body.split_once(',').context("expected (x,y)")?;
        let value = square_from_numeric([x.parse()?, y.parse()?], size)?;
        let [x, y] = numeric_square(value);
        ensure!(
            text == format!("({x},{y})"),
            "expected canonical numeric coordinates"
        );
        Ok(value)
    } else {
        let value: Square = text.parse()?;
        square_from_numeric(numeric_square(value), size)?;
        ensure!(text == value.to_string(), "expected canonical square");
        Ok(value)
    }
}
pub(crate) fn decode(
    answer: &str,
    format: OutputFormat,
    position: &Position,
) -> Result<(String, Option<Option<PieceKind>>)> {
    let numeric = format == OutputFormat::Numeric;
    let size = position.board().size();
    let (color, rest) = answer
        .split_once(' ')
        .context("expected Color Piece move")?;
    ensure!(
        matches!(color, "White" | "Black"),
        "expected White or Black"
    );
    let (name, rest) = rest
        .split_once(if numeric { ": " } else { " " })
        .context("expected exact history move format")?;
    kind(name)?;
    // Numeric destination coordinates also begin with " (": only the
    // grammar's named annotation suffixes can delimit a special move.
    let annotation_start = [" (en-passant", " (initial king jump", " (castling;"]
        .into_iter()
        .filter_map(|marker| rest.find(marker))
        .min();
    let (body, annotation) =
        annotation_start.map_or((rest, None), |i| (&rest[..i], Some(&rest[i + 2..])));
    if let Some(annotation) = annotation {
        if !matches!(annotation, "en-passant)" | "initial king jump)") {
            let rook = annotation
                .strip_prefix("castling; Rook ")
                .and_then(|s| s.strip_suffix(')'))
                .context("invalid special-move annotation")?;
            let (from, to) = rook
                .split_once(if numeric { " -> " } else { "-" })
                .context("invalid rook route")?;
            square(from, numeric, size)?;
            square(to, numeric, size)?;
        }
    }
    let (body, promotion) =
        if let Some((body, name)) = body.split_once(if numeric { " = " } else { "=" }) {
            (body, Some(kind(name)?))
        } else {
            (body, None)
        };
    let (from, to) = if numeric {
        body.split_once(" -> ").or_else(|| body.split_once(" x "))
    } else {
        body.split_once('-').or_else(|| body.split_once('x'))
    }
    .context("expected exact move separator")?;
    let from = square(from, numeric, size)?;
    let to = square(to, numeric, size)?;
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
