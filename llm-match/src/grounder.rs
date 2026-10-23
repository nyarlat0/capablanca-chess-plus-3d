//! Model-authored control maps. Validation is structural, NEVER geometric.
//! Only piece inventory and occupancy are read from Position; no move generation,
//! attack queries, evaluation, or correction of the model's controls occurs here.
use anyhow::{Context, Result, ensure};
use capablanca_chess_plus::{Color, Piece, PieceKind, Position, Square, Variant};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Accepted position snapshots are shared by legality-trial clones. Only complete
/// maps are published; a failed request never leaves a partially updated graph.
#[derive(Clone)]
pub(crate) struct Cache {
    pub positions: Vec<std::sync::Arc<Position>>,
    pub complete: Option<(usize, ControlMap)>,
    pub auto_initial: bool,
    pub test_start: Option<usize>,
}
impl Cache {
    pub fn new(position: &Position) -> Self {
        Self {
            positions: vec![std::sync::Arc::new(position.clone())],
            complete: None,
            auto_initial: true,
            test_start: None,
        }
    }
}

/// Explicit engine bootstrap, not part of model-output validation.
pub fn engine_map(position: &Position) -> Result<ControlMap> {
    let mut pieces = Vec::new();
    for (from, piece) in position.board().pieces() {
        let mut controls = Vec::new();
        for file in 0..8 {
            for rank in 0..8 {
                let target = Square::new(file, rank);
                if position.piece_controls(from, target) {
                    controls.push(target.to_string());
                }
            }
        }
        pieces.push(PieceControls {
            id: piece_id(position, &from.to_string(), piece),
            square: from.to_string(),
            controls,
        });
    }
    Ok(ControlGraph::new(position, ControlMap { version: 1, pieces })?.map().clone())
}

/// Build a post-move inventory and select invalidated entries using ONLY the
/// old map and occupancy differences. Captures, EP, castling and promotions all
/// change occupancy; castling therefore selects both relocated pieces.
pub fn incremental_targets(
    before: &Position,
    after: &Position,
    old: &ControlMap,
) -> Result<(ControlMap, Vec<usize>)> {
    ControlGraph::new(before, old.clone())?;
    let mut changed = BTreeSet::new();
    for file in 0..8 {
        for rank in 0..8 {
            let square = Square::new(file, rank);
            if before.board().piece_at(square) != after.board().piece_at(square) {
                changed.insert(square.to_string());
            }
        }
    }
    let mut pieces = Vec::new();
    let mut targets = Vec::new();
    for (square, piece) in after.board().pieces() {
        let cell = square.to_string();
        let id = piece_id(after, &cell, piece);
        let previous = old.pieces.iter().find(|entry| entry.id == id);
        let moved = changed.contains(&cell);
        let slider = matches!(piece.kind, PieceKind::Bishop | PieceKind::Rook | PieceKind::Queen);
        let invalidated = moved || previous.is_none() || (slider && previous.is_some_and(|p| {
            p.controls.iter().any(|s| changed.contains(s))
        }));
        if invalidated {
            targets.push(pieces.len());
        }
        pieces.push(PieceControls {
            id, square: cell,
            controls: if invalidated { vec![] } else { previous.unwrap().controls.clone() },
        });
    }
    Ok((ControlMap { version: 1, pieces }, targets))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControlMap {
    pub version: u32,
    pub pieces: Vec<PieceControls>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PieceControls {
    pub id: String,
    pub square: String,
    pub controls: Vec<String>,
}

/// Identity comes from the requested position, never from model output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OnePieceControls {
    pub controls: Vec<String>,
}
impl OnePieceControls {
    pub fn parse(text: &str) -> Result<Self> {
        let mut result: Self =
            crate::output::parse_final_json(text).context("invalid one-grounder JSON")?;
        validate_controls("selected piece", &mut result.controls)?;
        Ok(result)
    }
}

/// Accept an occupied square or exact piece id. Never compute attacks.
pub fn selected_piece(position: &Position, selection: &str) -> Result<String> {
    ensure!(
        position.rules().name() == Variant::Classic.rules().name(),
        "one-grounder requires Classic chess"
    );
    let cell = selection
        .split_once('_')
        .map_or(selection, |(_, cell)| cell);
    square(cell)?;
    let (_, piece) = position
        .board()
        .pieces()
        .find(|(s, _)| s.to_string() == cell)
        .with_context(|| format!("selected square {cell} is empty"))?;
    let id = piece_id(position, cell, piece);
    ensure!(
        selection == cell || selection == id,
        "selected piece {selection:?} does not match {id}"
    );
    Ok(format!(
        "{:?} {}: {cell}",
        piece.color,
        piece.kind.display_name()
    ))
}
fn validate_controls(owner: &str, controls: &mut [String]) -> Result<()> {
    let mut seen = BTreeSet::new();
    for cell in controls.iter() {
        square(cell).with_context(|| format!("{owner}.controls"))?;
        ensure!(
            seen.insert(cell),
            "duplicate controlled square {cell} for {owner}"
        );
    }
    controls.sort();
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
pub struct PieceView {
    pub id: String,
    pub square: String,
    pub color: String,
    pub kind: String,
    pub controls: Vec<String>,
    pub attacks_enemies: Vec<String>,
    pub defends_allies: Vec<String>,
    pub defended_by: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SquareView {
    pub square: String,
    pub occupant: Option<String>,
    /// All geometric controllers, irrespective of their color or occupancy.
    pub controlled_by: Vec<String>,
}

#[derive(Debug)]
pub struct ControlGraph {
    map: ControlMap,
    pieces: BTreeMap<String, PieceView>,
    squares: BTreeMap<String, SquareView>,
}

fn square(text: &str) -> Result<()> {
    let bytes = text.as_bytes();
    ensure!(
        bytes.len() == 2 && (b'a'..=b'h').contains(&bytes[0]) && (b'1'..=b'8').contains(&bytes[1]),
        "invalid Classic coordinate {text:?}; expected a1..h8 in lowercase"
    );
    Ok(())
}

pub(crate) fn piece_id(position: &Position, cell: &str, piece: Piece) -> String {
    format!(
        "{}{}_{}",
        if piece.color == Color::White {
            'w'
        } else {
            'b'
        },
        position
            .rules()
            .piece_symbol(Piece::new(Color::White, piece.kind)),
        cell
    )
}

impl ControlGraph {
    /// Parse a complete JSON response (optionally one JSON Markdown fence), then
    /// check inventory. Incorrect/missing geometric attacks are NOT detected.
    pub fn parse(position: &Position, text: &str) -> Result<Self> {
        let map = crate::output::parse_json(text).context("invalid Grounder JSON")?;
        Self::new(position, map)
    }

    pub fn new(position: &Position, mut map: ControlMap) -> Result<Self> {
        ensure!(
            position.rules().name() == Variant::Classic.rules().name(),
            "control maps require Classic chess"
        );
        ensure!(
            map.version == 1,
            "unsupported Grounder version {}; expected 1",
            map.version
        );
        let inventory: BTreeMap<_, _> = position
            .board()
            .pieces()
            .map(|(cell, piece)| {
                let cell = cell.to_string();
                (cell.clone(), (piece_id(position, &cell, piece), piece))
            })
            .collect();
        let mut ids = BTreeSet::new();
        let mut occupied = BTreeSet::new();
        for entry in &mut map.pieces {
            square(&entry.square).with_context(|| format!("piece {}", entry.id))?;
            ensure!(
                ids.insert(entry.id.clone()),
                "duplicate piece id {}",
                entry.id
            );
            ensure!(
                occupied.insert(entry.square.clone()),
                "duplicate piece square {}",
                entry.square
            );
            let (expected, _) = inventory
                .get(&entry.square)
                .with_context(|| format!("extra piece {}: {} is empty", entry.id, entry.square))?;
            ensure!(
                &entry.id == expected,
                "piece at {} has id {:?}; expected {expected} (color, type and square must match)",
                entry.square,
                entry.id
            );
            validate_controls(&entry.id, &mut entry.controls)?;
        }
        let missing: Vec<_> = inventory
            .values()
            .filter(|(id, _)| !ids.contains(id))
            .map(|(id, _)| id.as_str())
            .collect();
        ensure!(missing.is_empty(), "missing pieces: {}", missing.join(", "));
        map.pieces.sort_by(|a, b| a.id.cmp(&b.id));

        let mut squares = BTreeMap::new();
        for file in b'a'..=b'h' {
            for rank in 1..=8 {
                let cell = format!("{}{rank}", char::from(file));
                squares.insert(
                    cell.clone(),
                    SquareView {
                        square: cell.clone(),
                        occupant: inventory.get(&cell).map(|(id, _)| id.clone()),
                        controlled_by: Vec::new(),
                    },
                );
            }
        }
        let mut pieces = BTreeMap::new();
        for entry in &map.pieces {
            let (_, piece) = &inventory[&entry.square];
            let mut view = PieceView {
                id: entry.id.clone(),
                square: entry.square.clone(),
                color: format!("{:?}", piece.color),
                kind: piece.kind.display_name().into(),
                controls: entry.controls.clone(),
                attacks_enemies: Vec::new(),
                defends_allies: Vec::new(),
                defended_by: Vec::new(),
            };
            for cell in &entry.controls {
                squares
                    .get_mut(cell)
                    .unwrap()
                    .controlled_by
                    .push(entry.id.clone());
                if let Some((other_id, other_piece)) = inventory.get(cell) {
                    if other_piece.color == piece.color {
                        view.defends_allies.push(other_id.clone());
                    } else {
                        view.attacks_enemies.push(other_id.clone());
                    }
                }
            }
            view.attacks_enemies.sort();
            view.defends_allies.sort();
            pieces.insert(entry.id.clone(), view);
        }
        for id in ids {
            for ally in pieces[&id].defends_allies.clone() {
                pieces.get_mut(&ally).unwrap().defended_by.push(id.clone());
            }
        }
        Ok(Self {
            map,
            pieces,
            squares,
        })
    }

    pub fn map(&self) -> &ControlMap {
        &self.map
    }
    pub fn normalized_json(&self) -> Result<String> {
        Ok(serde_json::to_string_pretty(&self.map)?)
    }
    pub fn get_piece(&self, id: &str) -> Option<&PieceView> {
        self.pieces.get(id)
    }
    /// Includes allied defenders: these are geometric controllers, not legal captures.
    pub fn get_attackers(&self, cell: &str) -> Result<&[String]> {
        Ok(&self.get_square(cell)?.controlled_by)
    }
    pub fn get_square(&self, cell: &str) -> Result<&SquareView> {
        square(cell)?;
        Ok(&self.squares[cell])
    }
}
