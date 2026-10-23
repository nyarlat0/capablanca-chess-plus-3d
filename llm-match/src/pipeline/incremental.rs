//! Classic cached control maps: engine bootstrap, model-authored delta updates.
use super::*;
use crate::grounder::{ControlGraph, ControlMap, OnePieceControls};

fn at_position(state: &Match, index: usize) -> Match {
    let mut snapshot = state.clone();
    snapshot.game = capablanca_chess_plus::Game::new((*state.controls.positions[index]).clone());
    snapshot.model_side = snapshot.game.position().side_to_move();
    snapshot.history.truncate(index);
    snapshot.numeric_history.truncate(index);
    snapshot.json_history.truncate(index);
    snapshot.san_history.truncate(index);
    snapshot.uci_history.truncate(index);
    snapshot.wrong_moves.clear();
    snapshot
}

async fn fill(
    state: &Match, config: &Config, map: &mut ControlMap, targets: &[usize],
) -> Result<()> {
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(config.timeout_seconds)).build()?;
    let mut client = None;
    let image = crate::board_image::BoardImage::new(state.game.position())?;
    for &index in targets {
        let piece = &mut map.pieces[index];
        tracing::info!(piece_id = %piece.id, "incremental one-grounder");
        let mut data = prompt_data(state, config, Stage::OneGrounder)?;
        data.custom["selected-piece"] = json!(crate::grounder::selected_piece(
            state.game.position(), &piece.square,
        )?);
        piece.controls = request_parsed(
            &mut client, &http, config, Stage::OneGrounder, &data, &image,
            OnePieceControls::parse,
        ).await.with_context(|| format!("control map update failed for {}", piece.id))?.controls;
    }
    Ok(())
}

pub(super) async fn prepare(state: &mut Match, config: &Config) -> Result<String> {
    if state.controls.complete.is_none() {
        let index = state.controls.test_start.unwrap_or(0);
        let snapshot = at_position(state, index);
        let map = if state.controls.auto_initial || state.controls.test_start.is_some() {
            crate::grounder::engine_map(snapshot.game.position())?
        } else {
            // No engine geometry is queried when the initial checkbox is off.
            let raw = debug::run(&snapshot, config, Stage::OneGrounderFull, &debug::Inputs::default()).await?;
            ControlGraph::parse(snapshot.game.position(), &raw)?.map().clone()
        };
        state.controls.complete = Some((index, map));
    }
    loop {
        let (index, old) = state.controls.complete.as_ref().unwrap();
        if *index + 1 >= state.controls.positions.len() { break; }
        let next = *index + 1;
        let snapshot = at_position(state, next);
        let (mut map, targets) = crate::grounder::incremental_targets(
            &state.controls.positions[*index], snapshot.game.position(), old,
        )?;
        fill(&snapshot, config, &mut map, &targets).await?;
        let graph = ControlGraph::new(snapshot.game.position(), map)?;
        state.controls.complete = Some((next, graph.map().clone()));
    }
    ControlGraph::new(state.game.position(), state.controls.complete.as_ref().unwrap().1.clone())?.normalized_json()
}

/// Explicit preparation for CLI/real-game callers. Never applies a chess move.
pub async fn prepare_controls(state: &mut Match, config: &Config) -> Result<String> {
    ensure!(state.variant == capablanca_chess_plus::Variant::Classic, "incremental control map requires Classic chess");
    prepare(state, config).await
}
