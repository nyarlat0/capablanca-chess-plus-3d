//! Model-authored control maps. Validation is structural, NEVER geometric.
//! Only piece inventory and occupancy are read from Position; no move generation,
//! attack queries, evaluation, or correction of the model's controls occurs here.
use anyhow::{Context, Result, ensure};
use capablanca_chess_plus::{Color, Piece, Position, Variant};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

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

fn piece_id(position: &Position, cell: &str, piece: Piece) -> String {
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
            let mut controls = BTreeSet::new();
            for cell in &entry.controls {
                square(cell).with_context(|| format!("{}.controls", entry.id))?;
                ensure!(
                    controls.insert(cell),
                    "duplicate controlled square {cell} for {}",
                    entry.id
                );
            }
            entry.controls.sort();
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
