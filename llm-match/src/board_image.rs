//! Deterministic presentation only: no moves, attack maps or engine analysis.
use anyhow::{Context, Result};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use capablanca_chess_plus::{Color, PieceKind, Position};
use prompt_core::{ChatCompletionMessage, ChatCompletionRequest, ContentPart};
use std::fmt::Write;

pub const IMAGE_SIZE: u32 = 768;
const MARGIN: f32 = 32.0;
const LIGHT: &str = "#f0d9b5";
const DARK: &str = "#b58863";

fn icon(kind: PieceKind, color: Color) -> Option<&'static str> {
    macro_rules! pair {
        ($name:literal) => {
            Some(if color == Color::White {
                include_str!(concat!("../assets/pieces/w", $name, ".svg"))
            } else {
                include_str!(concat!("../assets/pieces/b", $name, ".svg"))
            })
        };
    }
    match kind {
        PieceKind::King => pair!("K"),
        PieceKind::Queen => pair!("Q"),
        PieceKind::Rook => pair!("R"),
        PieceKind::Bishop => pair!("B"),
        PieceKind::Knight => pair!("N"),
        PieceKind::Pawn => pair!("P"),
        _ => None,
    }
}

/// White always at the bottom, a1 dark. Coordinates on all four margins.
pub fn svg(position: &Position) -> String {
    let size = position.board().size();
    let cell = (IMAGE_SIZE as f32 - 2.0 * MARGIN) / f32::from(size.files().max(size.ranks()));
    let left = (IMAGE_SIZE as f32 - cell * f32::from(size.files())) / 2.0;
    let top = (IMAGE_SIZE as f32 - cell * f32::from(size.ranks())) / 2.0;
    let mut svg = String::from(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="768" height="768" viewBox="0 0 768 768"><rect width="768" height="768" fill="#312e2b"/>"##,
    );
    for rank in 0..size.ranks() {
        for file in 0..size.files() {
            let x = left + f32::from(file) * cell;
            let y = top + f32::from(size.ranks() - 1 - rank) * cell;
            let fill = if (file + rank) % 2 == 0 { DARK } else { LIGHT };
            write!(
                svg,
                r#"<rect x="{x}" y="{y}" width="{cell}" height="{cell}" fill="{fill}"/>"#
            )
            .unwrap();
        }
    }
    for (square, piece) in position.board().pieces() {
        let x = left + f32::from(square.file()) * cell;
        let y = top + f32::from(size.ranks() - 1 - square.rank()) * cell;
        if let Some(icon) = icon(piece.kind, piece.color) {
            // Trusted embedded SVG assets; no external resources/fonts/network.
            let body = &icon[icon.find('>').unwrap() + 1..icon.rfind("</svg>").unwrap()];
            let scale = cell * 0.94 / 45.0;
            write!(svg, r#"<g data-square="{square}" transform="translate({} {}) scale({scale})">{body}</g>"#, x + cell * 0.03, y + cell * 0.03).unwrap();
        } else {
            // Fairy pieces have no standard Staunton silhouette. Show the exact
            // engine symbol, not a misleading borrowed orthodox piece icon.
            let fill = if piece.color == Color::White {
                "#ffffff"
            } else {
                "#171717"
            };
            let stroke = if piece.color == Color::White {
                "#171717"
            } else {
                "#ffffff"
            };
            let symbol = position.rules().piece_symbol(piece);
            write!(svg, r#"<text x="{}" y="{}" font-family="Fira Mono" font-size="{}" text-anchor="middle" fill="{fill}" stroke="{stroke}" stroke-width="0.8" paint-order="stroke">{symbol}</text>"#, x + cell/2.0, y + cell*0.72, cell*0.6).unwrap();
        }
    }
    for file in 0..size.files() {
        let label = char::from(b'a' + file);
        let x = left + (f32::from(file) + 0.5) * cell;
        for y in [top - 10.0, top + cell * f32::from(size.ranks()) + 23.0] {
            label_text(&mut svg, x, y, &label.to_string());
        }
    }
    for rank in 0..size.ranks() {
        let y = top + (f32::from(size.ranks() - rank) - 0.5) * cell + 7.0;
        for x in [left - 17.0, left + cell * f32::from(size.files()) + 17.0] {
            label_text(&mut svg, x, y, &(rank + 1).to_string());
        }
    }
    svg.push_str("</svg>");
    svg
}

fn label_text(svg: &mut String, x: f32, y: f32, label: &str) {
    write!(svg, r##"<text x="{x}" y="{y}" font-family="Fira Mono" font-size="20" text-anchor="middle" fill="#f0d9b5">{label}</text>"##).unwrap();
}

pub fn png(position: &Position) -> Result<Vec<u8>> {
    let mut options = resvg::usvg::Options::default();
    options.fontdb_mut().load_font_data(
        include_bytes!("../../bevy-front/assets/fonts/FiraMono-Medium.ttf").to_vec(),
    );
    let tree = resvg::usvg::Tree::from_str(&svg(position), &options)?;
    let mut pixmap =
        resvg::tiny_skia::Pixmap::new(IMAGE_SIZE, IMAGE_SIZE).context("PNG allocation failed")?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::identity(),
        &mut pixmap.as_mut(),
    );
    Ok(pixmap.encode_png()?)
}

/// Render once per position and reuse across stage retries. Never stored in Match.
pub(crate) struct BoardImage {
    fen: String,
    uri: String,
    enabled: bool,
}
impl BoardImage {
    pub(crate) fn new(position: &Position) -> Result<Self> {
        Self::with_enabled(position,true)
    }
    pub(crate) fn with_enabled(position: &Position, enabled: bool) -> Result<Self> {
        Ok(Self {
            fen: position.to_fen(),
            uri: if enabled {format!("data:image/png;base64,{}", STANDARD.encode(png(position)?))}
                 else {String::new()},
            enabled,
        })
    }
    pub(crate) fn attach(&self, request: &mut ChatCompletionRequest) -> Result<()> {
        if !self.enabled { return Ok(()); }
        let content = request
            .messages
            .iter_mut()
            .rev()
            .find_map(|message| match message {
                ChatCompletionMessage::User { content } => Some(content),
                _ => None,
            })
            .context("Board attachment needs a final user message")?;
        content.push(ContentPart::text(format!(
            "\nAuthoritative current position (image below, White at bottom). FEN: {}",
            self.fen
        )));
        content.push(ContentPart::image_url(&self.uri));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use capablanca_chess_plus::Variant;
    #[test]
    fn side_to_move_does_not_rotate_board_and_fairy_sizes_work() {
        let white = Variant::Classic.starting_position();
        let black = Position::from_fen(
            Variant::Classic.rules(),
            &white.to_fen().replace(" w ", " b "),
        )
        .unwrap();
        assert_eq!(png(&white).unwrap(), png(&black).unwrap());
        for variant in [Variant::Gothic, Variant::TerachessII] {
            assert_eq!(
                &png(&variant.starting_position()).unwrap()[..8],
                b"\x89PNG\r\n\x1a\n"
            );
        }
    }

    #[tokio::test]
    #[ignore = "requires KOBOLD_URL and a running vision-capable KoboldCpp"]
    async fn live_kobold_board_image() {
        let client = prompt_core::KoboldClient::connect_with_client(
            std::env::var("KOBOLD_URL").expect("set KOBOLD_URL"),
            reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(120))
                .build()
                .unwrap(),
        )
        .await
        .unwrap();
        let mut request = ChatCompletionRequest::new(vec![ChatCompletionMessage::user(
            "Describe the attached chessboard briefly.",
        )]);
        request.parameters =
            serde_json::json!({"max_tokens":64,"temperature":0,"thinking_budget_tokens":0});
        BoardImage::new(&Variant::Classic.starting_position())
            .unwrap()
            .attach(&mut request)
            .unwrap();
        let fitted = client.fit_chat(&request).await.unwrap();
        let response = client.generate_chat_full(&fitted.request).await.unwrap();
        assert!(!response.content.unwrap_or_default().trim().is_empty());
    }
    #[test]
    fn deterministic_size_orientation_and_colors() {
        let position = Variant::Classic.starting_position();
        let a = png(&position).unwrap();
        assert_eq!(a, png(&position).unwrap());
        assert_eq!(&a[..8], b"\x89PNG\r\n\x1a\n");
        assert_eq!(u32::from_be_bytes(a[16..20].try_into().unwrap()), 768);
        assert_eq!(u32::from_be_bytes(a[20..24].try_into().unwrap()), 768);
        let svg = svg(&position);
        assert!(svg.contains(r##"x="32" y="648" width="88" height="88" fill="#b58863""##));
        assert!(svg.contains(r#"data-square="a1" transform="translate(34.64 650.64)"#));
    }
    #[test]
    fn only_last_user_gets_image_and_fen() {
        let image = BoardImage::new(&Variant::Classic.starting_position()).unwrap();
        let mut request = ChatCompletionRequest::new(vec![
            ChatCompletionMessage::user("e4"),
            ChatCompletionMessage::assistant("e5"),
            ChatCompletionMessage::user("choose"),
        ]);
        image.attach(&mut request).unwrap();
        assert_eq!(request.messages[0].content(), Some("e4"));
        assert_eq!(request.messages[1].content(), Some("e5"));
        assert_eq!(request.messages[2].image_count(), 1);
        assert!(
            request.messages[2]
                .text_parts()
                .concat()
                .contains(&image.fen)
        );
    }
}
