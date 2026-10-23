//! LLM-authored absolute/relative pins. No engine pin search or correction.
use anyhow::{Context, Result, ensure};
use capablanca_chess_plus::{PieceKind, Position, Variant};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use crate::grounder::{ControlGraph, piece_id};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PinType { Absolute, Relative }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pin {
    pub attacker: String,
    pub pinned: String,
    pub protected: String,
    #[serde(rename = "type")]
    pub kind: PinType,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PinReport {
    pub version: u32,
    pub pins: Vec<Pin>,
}

fn value(kind: PieceKind) -> u8 {
    match kind {
        PieceKind::Pawn => 1,
        PieceKind::Knight | PieceKind::Bishop => 3,
        PieceKind::Rook => 5,
        PieceKind::Queen => 9,
        PieceKind::King => 100,
        _ => 0,
    }
}
impl PinReport {
    /// Validates model output against actual identities and the SUPPLIED map.
    /// Does not trace rays, find missing pins or repair geometry.
    pub fn parse(position: &Position, graph: &ControlGraph, raw: &str) -> Result<Self> {
        ensure!(position.rules().name() == Variant::Classic.rules().name(),
            "pin analysis requires Classic chess");
        let mut report: Self = crate::output::parse_final_json(raw)
            .context("invalid pin-analyzer JSON")?;
        ensure!(report.version == 1, "unsupported pin report version");
        let inventory: BTreeMap<_, _> = position.board().pieces().map(|(square, piece)| {
            let cell = square.to_string();
            (piece_id(position, &cell, piece), (cell, piece))
        }).collect();
        let mut seen = BTreeSet::new();
        for pin in &report.pins {
            let (_, attacker) = inventory.get(&pin.attacker)
                .with_context(|| format!("unknown attacker {}", pin.attacker))?;
            let (pinned_sq, pinned) = inventory.get(&pin.pinned)
                .with_context(|| format!("unknown pinned piece {}", pin.pinned))?;
            let (protected_sq, protected) = inventory.get(&pin.protected)
                .with_context(|| format!("unknown protected piece {}", pin.protected))?;
            ensure!(seen.insert((pin.attacker.clone(), pin.pinned.clone(), pin.protected.clone())),
                "duplicate pin {} -> {} -> {}", pin.attacker, pin.pinned, pin.protected);
            ensure!(matches!(attacker.kind, PieceKind::Bishop | PieceKind::Rook | PieceKind::Queen),
                "pin attacker must be rook, bishop or queen: {}", pin.attacker);
            ensure!(attacker.color != pinned.color && pinned.color == protected.color,
                "pin colors do not match: {} -> {} -> {}", pin.attacker, pin.pinned, pin.protected);
            ensure!(pin.pinned != pin.protected && pinned.kind != PieceKind::King,
                "invalid pinned piece {}", pin.pinned);
            let rays = &graph.get_piece(&pin.attacker)
                .with_context(|| format!("attacker missing from control graph: {}", pin.attacker))?
                .xray;
            ensure!(rays.iter().any(|r| r.first == *pinned_sq &&
                r.second.as_deref() == Some(protected_sq.as_str())),
                "supplied xray does not contain {} -> {} -> {}",
                pin.attacker,pin.pinned,pin.protected);
            match pin.kind {
                PinType::Absolute => ensure!(protected.kind == PieceKind::King,
                    "absolute pin must protect king: {}", pin.protected),
                PinType::Relative => ensure!(
                    protected.kind != PieceKind::King && value(protected.kind) > value(pinned.kind),
                    "relative pin must protect a strictly more valuable non-king piece"
                ),
            }
        }
        report.pins.sort_by(|a,b|
            (&a.pinned, &a.attacker, &a.protected).cmp(&(&b.pinned, &b.attacker, &b.protected)));
        Ok(report)
    }
}
