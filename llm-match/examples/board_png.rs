//! Offline PNG preview: cargo run -p llm-match --example board_png -- target/board.png [FEN]
use anyhow::{Context, Result};
use capablanca_chess_plus::{Position, Variant};
fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let path = args
        .next()
        .context("usage: board_png OUTPUT.png [classic FEN]")?;
    let position = match args.next() {
        Some(fen) => Position::from_fen(Variant::Classic.rules(), &fen)?,
        None => Variant::Classic.starting_position(),
    };
    std::fs::write(path, llm_match::board_image::png(&position)?)?;
    Ok(())
}
