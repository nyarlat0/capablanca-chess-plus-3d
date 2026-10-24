//! Model-authored control maps. The engine is the validation oracle, not a
//! replacement for generated controls: incorrect outputs are rejected/retried.
//! Valid responses are canonically sorted only after parsing.
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

/// Squares of the first and second occupied blockers on a slider's ray.
/// Coordinates, not piece IDs; these are not necessarily legally attacked squares.
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
        "xray blocker is not on the selected ray");
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
            if position.board().piece_at(square).is_some() {
                let cell=square.to_string();
                if first.is_none() {first=Some(cell);}
                else {second=Some(cell);break;}
            }
        }
        if let Some(first)=first {
            rays.push(XRay{direction:direction.into(), first, second});
        }
    }
    rays
}

fn occupied(position: &Position, cell: &str) -> Result<bool> {
    square(cell)?;
    let at: Square=cell.parse()?;
    Ok(position.board().piece_at(at).is_some())
}

fn validate_xrays(position: &Position, entry: &PieceControls, piece: Piece) -> Result<()> {
    let mut seen=BTreeSet::new();
    for ray in &entry.xray {
        ensure!(allowed_ray(piece.kind,&ray.direction),
            "invalid xray direction {:?} for {}",ray.direction,entry.id);
        ensure!(seen.insert(&ray.direction),
            "duplicate xray direction {:?} for {}",ray.direction,entry.id);
        square(&ray.first)?;
        ensure!(ray.first != entry.square && occupied(position,&ray.first)?,
            "{} first blocker {} must occupy another square",entry.id,ray.first);
        ensure!(entry.controls.contains(&ray.first),
            "{} first blocker {} must be a directly controlled square",
            entry.id,ray.first);

        // A claimed first blocker cannot skip an earlier occupied square.
        let path=ray_cells(&entry.square,&ray.direction,Some(&ray.first))?;
        ensure!(path[..path.len()-1].iter().all(|s| !occupied(position,s).unwrap_or(true)),
            "{} xray first blocker {} skips a closer piece",entry.id,ray.first);

        if let Some(second)=&ray.second {
            square(second)?;
            ensure!(second != &ray.first && second != &entry.square,
                "{} xray blockers must be distinct",entry.id);
        }
        // Read the ray behind the reported first blocker and CHECK (never fill)
        // the next occupied square, including when the model reported null.
        let next=ray_cells(&ray.first,&ray.direction,None)?.into_iter()
            .find(|s| occupied(position,s).unwrap_or(false));
        ensure!(ray.second == next,
            "{} xray {:?} has second {:?}; nearest occupied square after {} is {:?}",
            entry.id,ray.direction,ray.second,ray.first,next);
    }
    Ok(())
}

/// Fail rather than silently overwrite any model-authored geometric claim.
/// Both controls and xray must match the engine exactly, regardless of order.
fn compare_engine_piece(actual: &PieceControls, expected: &PieceControls) -> Result<()> {
    ensure!(actual.id == expected.id && actual.square == expected.square,
        "engine inventory mismatch for {}",actual.id);

    let found: BTreeSet<_> = actual.controls.iter().cloned().collect();
    let wanted: BTreeSet<_> = expected.controls.iter().cloned().collect();
    ensure!(found == wanted,
        "engine controls mismatch for {}: missing {:?}, extra {:?}",
        actual.id,
        wanted.difference(&found).collect::<Vec<_>>(),
        found.difference(&wanted).collect::<Vec<_>>());

    let mut rays = actual.xray.clone();
    rays.sort_by(|a,b|a.direction.cmp(&b.direction));
    ensure!(rays == expected.xray,
        "engine xray mismatch for {}: expected {:?}, got {:?}",
        actual.id,expected.xray,rays);
    Ok(())
}

/// Validate one model-generated piece INSIDE the transport retry callback.
/// Coordinates/piece identity are trusted only after the engine cross-check.
pub fn validate_one(position: &Position, cell: &str, result: &OnePieceControls) -> Result<()> {
    square(cell)?;
    let from: Square=cell.parse()?;
    let piece=position.board().piece_at(from).context("selected square is empty")?;
    let mut entry=PieceControls{
        id:piece_id(position,cell,piece), square:cell.into(),
        controls:result.controls.clone(), xray:result.xray.clone(),
    };
    validate_controls(&entry.id,&mut entry.controls)?;
    validate_xrays(position,&entry,piece)?;

    let reference=engine_map(position)?;
    let expected=reference.pieces.iter()
        .find(|p|p.square==cell)
        .context("selected piece missing from engine control map")?;
    compare_engine_piece(&entry,expected)
}

/// Validate a complete model-generated control map against the engine without
/// replacing any data in it. Structural parsing and normalization stay separate,
/// so offline map inspection and existing consumers retain their behavior.
pub fn validate_engine_map(position: &Position, map: &ControlMap) -> Result<()> {
    let model=ControlGraph::new(position,map.clone())?;
    let reference=engine_map(position)?;
    for (actual,expected) in model.map().pieces.iter().zip(&reference.pieces) {
        compare_engine_piece(actual,expected)?;
    }
    Ok(())
}

/// Cells whose changed occupancy/identity would alter the xray portion,
/// including EMPTY cells behind first blocker and through the second blocker.
/// The direct control map already handles changes up to and including first.
fn xray_affected(entry: &PieceControls, changed: &BTreeSet<String>) -> Result<bool> {
    for ray in &entry.xray {
        if ray_cells(&ray.first,&ray.direction,ray.second.as_deref())?
            .iter().any(|cell|changed.contains(cell)) {
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
            square(&ray.first)?;
            if let Some(second)=&ray.second { square(second)?; }
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


impl ControlGraph {
    /// Read-only tactical facts for Proposer and Verifier. Derived exclusively
    /// from the supplied (already verified) Grounder map and its occupancy
    /// indexes. Never asks the engine for moves, attacks or evaluations.
    ///
    /// Internal piece IDs must not leak into this model-facing representation.
    pub fn readable_analysis(&self) -> String {
        use std::fmt::Write as _;

        let human = |id: &str| {
            let piece = &self.pieces[id];
            format!("{} {} on {}", piece.color, piece.kind, piece.square)
        };
        let names = |ids: &[String]| {
            if ids.is_empty() {
                "none".to_owned()
            } else {
                ids.iter().map(|id| human(id)).collect::<Vec<_>>().join(", ")
            }
        };

        let mut result = String::new();
        writeln!(result, "VERIFIED GROUNDER GEOMETRY OF THE CURRENT POSITION").unwrap();
        writeln!(result, "This is geometric control, not a list of legal moves or safe captures.").unwrap();
        writeln!(result, "A pinned piece still controls geometric squares; occupied friendly squares are defended.").unwrap();
        writeln!(result, "All pressure, king threats and alignments refer ONLY to the current board, before any candidate move.").unwrap();
        writeln!(result).unwrap();

        let mut white_squares = 0usize;
        let mut black_squares = 0usize;
        let mut contested_squares = Vec::new();
        for (square, info) in &self.squares {
            let white = info.controlled_by.iter().any(|id| self.pieces[id].color == "White");
            let black = info.controlled_by.iter().any(|id| self.pieces[id].color == "Black");
            white_squares += usize::from(white);
            black_squares += usize::from(black);
            if white && black {
                contested_squares.push(square.clone());
            }
        }
        let white_pieces = self.pieces.values().filter(|p| p.color == "White").count();
        let black_pieces = self.pieces.len() - white_pieces;
        writeln!(result, "SUMMARY: White pieces {white_pieces}; Black pieces {black_pieces}; \
            distinct squares controlled by White {white_squares}/64, by Black {black_squares}/64; \
            contested squares {}: {}.",
            contested_squares.len(),
            if contested_squares.is_empty() { "none".into() } else { contested_squares.join(", ") }
        ).unwrap();
        writeln!(result).unwrap();

        let mut attacked_undefended = Vec::new();
        let mut attackers_outnumber_defenders = Vec::new();
        let mut checks = Vec::new();
        let mut unprotected = Vec::new();
        writeln!(result, "EVERY PIECE (complete controls, opponents attacked, allies defended, current attackers and defenders):").unwrap();
        for piece in self.pieces.values() {
            let all = &self.squares[&piece.square].controlled_by;
            let attackers: Vec<String> = all.iter()
                .filter(|id| self.pieces[*id].color != piece.color).cloned().collect();
            let label = human(&piece.id);
            let status = if attackers.is_empty() {
                "not geometrically attacked"
            } else if piece.defended_by.is_empty() {
                "ATTACKED AND GEOMETRICALLY UNDEFENDED"
            } else if attackers.len() > piece.defended_by.len() {
                "MORE GEOMETRIC ATTACKERS THAN DEFENDERS"
            } else {
                "attacked and defended"
            };
            if piece.defended_by.is_empty() {
                unprotected.push(label.clone());
                if !attackers.is_empty() {
                    attacked_undefended.push(label.clone());
                }
            }
            if attackers.len() > piece.defended_by.len() {
                attackers_outnumber_defenders.push(format!(
                    "{} ({} attackers, {} defenders)",
                    label, attackers.len(), piece.defended_by.len()
                ));
            }
            if piece.kind == "King" && !attackers.is_empty() {
                checks.push(format!("{} by {}", label, names(&attackers)));
            }
            writeln!(result, "{} | controls: {} | attacks opponents: {} | defends allies: {} | \
                attacked by: {} | defended by: {} | status: {}",
                label,
                if piece.controls.is_empty() { "none".into() } else { piece.controls.join(", ") },
                names(&piece.attacks_enemies), names(&piece.defends_allies),
                names(&attackers), names(&piece.defended_by), status
            ).unwrap();
        }
        writeln!(result).unwrap();

        writeln!(result, "EVERY SQUARE (occupant, geometric controllers from each side, including empty squares):").unwrap();
        // Iterate by ranks so this section follows a chessboard rather than
        // the BTreeMap's file-major ordering.
        for rank in (1..=8).rev() {
            for file in b'a'..=b'h' {
                let square = format!("{}{rank}", char::from(file));
                let info = &self.squares[&square];
                let white: Vec<String> = info.controlled_by.iter()
                    .filter(|id| self.pieces[*id].color == "White").cloned().collect();
                let black: Vec<String> = info.controlled_by.iter()
                    .filter(|id| self.pieces[*id].color == "Black").cloned().collect();
                let occupant = info.occupant.as_deref().map(&human)
                    .unwrap_or_else(|| "empty".into());
                writeln!(result, "{}: {} | White controllers ({}): {} | Black controllers ({}): {}",
                    square, occupant, white.len(), names(&white), black.len(), names(&black)
                ).unwrap();
            }
        }
        writeln!(result).unwrap();

        writeln!(result, "XRAY ALIGNMENTS (first and next occupied blockers, including allies):").unwrap();
        let mut xray_count = 0usize;
        let mut pin_candidates = Vec::new();
        let mut discovered_candidates = Vec::new();
        for piece in self.pieces.values() {
            for ray in &piece.xray {
                xray_count += 1;
                let first_id = self.squares[&ray.first].occupant.as_deref()
                    .expect("validated xray first blocker");
                let first = &self.pieces[first_id];
                let second_id = ray.second.as_ref().and_then(|cell| {
                    self.squares[cell].occupant.as_deref()
                });
                let second = second_id.map(&human).unwrap_or_else(|| "none".into());
                writeln!(result, "{} [{}]: first {}; next {}",
                    human(&piece.id), ray.direction, human(first_id), second
                ).unwrap();

                if let Some(second_id) = second_id {
                    let behind = &self.pieces[second_id];
                    if piece.color != first.color && first.color == behind.color
                        && behind.kind == "King"
                    {
                        pin_candidates.push(format!(
                            "{} may geometrically pin {} to {} along {}",
                            human(&piece.id), human(first_id), human(second_id), ray.direction
                        ));
                    }
                    if piece.color == first.color && piece.color != behind.color {
                        discovered_candidates.push(format!(
                            "moving {} may uncover {} toward {} along {}",
                            human(first_id), human(&piece.id), human(second_id), ray.direction
                        ));
                    }
                }
            }
        }
        if xray_count == 0 { writeln!(result, "none").unwrap(); }
        writeln!(result).unwrap();

        let display = |entries: &[String]| if entries.is_empty() {
            "none".to_owned()
        } else {
            entries.join("; ")
        };
        writeln!(result, "CURRENT GEOMETRIC PRESSURE SUMMARY:").unwrap();
        writeln!(result, "Kings geometrically attacked: {}", display(&checks)).unwrap();
        writeln!(result, "Attacked and without geometric defenders: {}",
            display(&attacked_undefended)).unwrap();
        writeln!(result, "More geometric attackers than defenders (NOT an exchange evaluation): {}",
            display(&attackers_outnumber_defenders)).unwrap();
        writeln!(result, "Pieces with no geometric defenders (even when not attacked): {}",
            display(&unprotected)).unwrap();
        writeln!(result, "Potential absolute pin alignments (geometry only): {}",
            display(&pin_candidates)).unwrap();
        writeln!(result, "Potential discovered-attack alignments (geometry only): {}",
            display(&discovered_candidates)).unwrap();
        writeln!(result, "Do NOT interpret these relations as legality, profitable captures, \
            confirmed pins, tactical evaluations or facts about future positions.").unwrap();
        result
    }
}
