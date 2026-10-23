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


const DIRECTIONS: [(&str, i8, i8); 8] = [
    ("N",0,1), ("NE",1,1), ("E",1,0), ("SE",1,-1),
    ("S",0,-1), ("SW",-1,-1), ("W",-1,0), ("NW",-1,1),
];

/// One occupied blocking piece and optionally the next occupied piece
/// on the SAME ray. Does not imply a legal attack on `second`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct XRay {
    pub direction: String,
    pub first: String,
    pub second: Option<String>,
}

fn ray_direction(dir: &str) -> Option<(i8,i8)> {
    DIRECTIONS.iter().find(|(s,_,_)| *s == dir).map(|(_,x,y)| (*x,*y))
}

fn allowed_ray(kind: PieceKind, direction: &str) -> bool {
    let Some((dx,dy)) = ray_direction(direction) else {return false;};
    match kind {
        PieceKind::Rook => dx == 0 || dy == 0,
        PieceKind::Bishop => dx != 0 && dy != 0,
        PieceKind::Queen => true,
        _ => false,
    }
}

fn ray_cells(start: &str, direction: &str, stop: Option<&str>) -> Result<Vec<String>> {
    square(start)?;
    let (dx,dy)=ray_direction(direction).with_context(|| format!("invalid ray direction {direction}"))?;
    let mut x=i16::from(start.as_bytes()[0]-b'a');
    let mut y=i16::from(start.as_bytes()[1]-b'1');
    let mut cells=Vec::new();
    loop {
        x += i16::from(dx);
        y += i16::from(dy);
        if !(0..8).contains(&x) || !(0..8).contains(&y) {break;}
        let cell=Square::new(x as u8,y as u8).to_string();
        cells.push(cell.clone());
        if stop == Some(cell.as_str()) {break;}
    }
    ensure!(stop.is_none_or(|s| cells.last().is_some_and(|c| c == s)),
        "xray second blocker is not on the ray behind the first");
    Ok(cells)
}

/// Uses the board only when deliberately bootstrapping without an LLM.
/// Both friendly and enemy pieces are recorded as ray blockers.
fn engine_xrays(position: &Position, origin: Square, kind: PieceKind) -> Vec<XRay> {
    let mut rays=Vec::new();
    for &(direction,dx,dy) in &DIRECTIONS {
        if !allowed_ray(kind,direction) {continue;}
        let (mut x,mut y)=(i16::from(origin.file()),i16::from(origin.rank()));
        let mut first=None;
        let mut second=None;
        loop {
            x += i16::from(dx);
            y += i16::from(dy);
            if !(0..8).contains(&x) || !(0..8).contains(&y) {break;}
            let square=Square::new(x as u8,y as u8);
            if let Some(piece)=position.board().piece_at(square) {
                let id=piece_id(position,&square.to_string(),piece);
                if first.is_none() {first=Some(id);}
                else {second=Some(id);break;}
            }
        }
        if let Some(first)=first {
            rays.push(XRay{direction:direction.into(), first, second});
        }
    }
    rays
}

fn validate_xrays(position: &Position, entry: &PieceControls, piece: Piece) -> Result<()> {
    let inventory: BTreeMap<_,_> = position.board().pieces().map(|(sq,p)| {
        let label=sq.to_string();
        (piece_id(position,&label,p), label)
    }).collect();
    let mut seen=BTreeSet::new();
    for ray in &entry.xray {
        ensure!(allowed_ray(piece.kind,&ray.direction),
            "invalid xray direction {:?} for {}",ray.direction,entry.id);
        ensure!(seen.insert(&ray.direction),
            "duplicate xray direction {:?} for {}",ray.direction,entry.id);
        let first=inventory.get(&ray.first)
            .with_context(|| format!("{} refers to nonexistent first blocker {}",entry.id,ray.first))?;
        ensure!(ray.first != entry.id && entry.controls.contains(first),
            "{} xray first blocker {} is not in its direct controls",entry.id,ray.first);
        let path=ray_cells(&entry.square,&ray.direction,Some(first))?;
        ensure!(path.last()==Some(first),"invalid first ray blocker for {}",entry.id);
        if let Some(second)=&ray.second {
            ensure!(second != &ray.first && second != &entry.id,
                "xray blockers must be distinct and not be the attacker");
            let second_sq=inventory.get(second)
                .with_context(|| format!("{} refers to nonexistent second blocker {second}",entry.id))?;
            ray_cells(first,&ray.direction,Some(second_sq))?;
        }
    }
    Ok(())
}

/// Validate a single new LLM result in the retry loop, not only after merging.
pub fn validate_one(position: &Position, cell: &str, result: &OnePieceControls) -> Result<()> {
    let from: Square=cell.parse()?;
    let piece=position.board().piece_at(from).context("selected square is empty")?;
    let entry=PieceControls{
        id:piece_id(position,cell,piece), square:cell.into(),
        controls:result.controls.clone(), xray:result.xray.clone(),
    };
    validate_xrays(position,&entry,piece)
}

/// Cells whose changed occupancy/identity would alter the xray portion,
/// including EMPTY cells behind first blocker and through the second blocker.
/// The direct control map already handles changes up to and including first.
fn xray_affected(entry: &PieceControls, changed: &BTreeSet<String>) -> Result<bool> {
    for ray in &entry.xray {
        let first=ray.first.split_once('_').map(|(_,sq)|sq)
            .context("invalid xray first blocker id")?;
        let second=ray.second.as_deref().map(|id|id.split_once('_')
            .map(|(_,sq)|sq).context("invalid xray second blocker id")).transpose()?;
        if ray_cells(first,&ray.direction,second)?.iter().any(|cell|changed.contains(cell)) {
            return Ok(true);
        }
    }
    Ok(false)
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
            xray: engine_xrays(position,from,piece.kind),
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
        let ray_invalidated = if slider {
            match previous {
                Some(p) => p.controls.iter().any(|s| changed.contains(s))
                    || xray_affected(p, &changed)?,
                None => true,
            }
        } else { false };
        let invalidated = moved || previous.is_none() || ray_invalidated;
        if invalidated {
            targets.push(pieces.len());
        }
        pieces.push(PieceControls {
            id, square: cell,
            controls: if invalidated { vec![] } else { previous.unwrap().controls.clone() },
            xray: if invalidated { vec![] } else { previous.unwrap().xray.clone() },
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
    /// Empty for non-sliding pieces. Legacy version-1 map files omit this field.
    #[serde(default)]
    pub xray: Vec<XRay>,
}

/// Identity comes from the requested position, never from model output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OnePieceControls {
    pub controls: Vec<String>,
    /// Required in model output; empty for kings, pawns and knights.
    pub xray: Vec<XRay>,
}
impl OnePieceControls {
    pub fn parse(text: &str) -> Result<Self> {
        let mut result: Self =
            crate::output::parse_final_json(text).context("invalid one-grounder JSON")?;
        validate_controls("selected piece", &mut result.controls)?;
        let mut seen=BTreeSet::new();
        for ray in &result.xray {
            ensure!(ray_direction(&ray.direction).is_some(), "invalid xray direction {}",ray.direction);
            ensure!(seen.insert(&ray.direction), "duplicate xray direction {}",ray.direction);
        }
        result.xray.sort_by(|a,b| a.direction.cmp(&b.direction));
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
    pub xray: Vec<XRay>,
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
            entry.xray.sort_by(|a,b| a.direction.cmp(&b.direction));
            validate_xrays(position,entry,inventory.get(&entry.square).unwrap().1)?;
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
                xray: entry.xray.clone(),
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
