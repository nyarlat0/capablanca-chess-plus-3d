//! Read-only what-if calculations for the Classic Proposer.
 //! Every call starts at the authoritative root. The chess engine validates
 //! each SAN move and computes geometry; the LLM supplies all candidate moves.
use crate::{Match, grounder::{ControlGraph, ControlMap, PieceControls}};
use anyhow::{Context, Result, ensure};
use capablanca_chess_plus::{Color, GameOutcome, Position, Square};
use prompt_core::{ToolCall, ToolDefinition};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Args {
    moves: Vec<String>,
}

pub(super) fn tool() -> ToolDefinition {
    ToolDefinition::new(
        "inspect_line",
        "Validate a proposed SAN move sequence from the ORIGINAL board and return only the resulting changes in piece locations, geometric controls, xrays, and attack/defense pressure after each ply. The engine verifies legality; the original game is never changed. Use multiple calls for alternative branches.",
        json!({
            "type": "object",
            "properties": {
                "moves": {
                    "type": "array",
                    "description": "1-8 consecutive SAN moves, starting with our move, then alternating sides. Examples: [\"e4\",\"e5\",\"Nf3\"], [\"Rxe8+\"]",
                    "items": {"type": "string"},
                    "minItems": 1,
                    "maxItems": 8
                }
            },
            "required": ["moves"],
            "additionalProperties": false
        }),
    )
}

fn label(position: &Position, square: &str) -> String {
    let cell: Square = square.parse().expect("valid engine square");
    match position.board().piece_at(cell) {
        Some(piece) => format!("{:?} {} {}", piece.color, piece.kind.display_name(), square),
        None => "empty".into(),
    }
}

fn controllers(graph: &ControlGraph, sq: &str, side: Color) -> Vec<String> {
    graph.get_square(sq).expect("board square").controlled_by.iter()
        .filter_map(|id| {
            let p = graph.get_piece(id).expect("complete graph");
            if p.color == format!("{side:?}") {
                Some(format!("{} {} {}", p.color, p.kind, p.square))
            } else { None }
        }).collect()
}

fn names(position: &Position, square: &str) -> Option<String> {
    let s: Square = square.parse().expect("valid square");
    position.board().piece_at(s).map(|_| label(position,square))
}

fn map_index(map: &ControlMap) -> BTreeMap<&str, &PieceControls> {
    map.pieces.iter().map(|p|(p.id.as_str(),p)).collect()
}

pub(super) fn position_delta(before: &Position, after: &Position, prior: &ControlMap, updated: &ControlMap) -> Result<Value> {
    let old = ControlGraph::new(before, prior.clone())?;
    let new = ControlGraph::new(after, updated.clone())?;
    let old_index = map_index(prior);
    let mut squares = Vec::new();
    let mut control_changes = Vec::new();
    let mut pressure_changes = Vec::new();
    for file in b'a'..=b'h' {
        for rank in 1..=8 {
            let sq = format!("{}{rank}",char::from(file));
            let was = names(before, &sq);
            let now = names(after, &sq);
            if was != now {
                squares.push(json!({"square":sq,"before":was,"after":now}));
            }

            let w_old = controllers(&old,&sq,Color::White);
            let w_new = controllers(&new,&sq,Color::White);
            let b_old = controllers(&old,&sq,Color::Black);
            let b_new = controllers(&new,&sq,Color::Black);
            if w_old != w_new || b_old != b_new {
                control_changes.push(json!({
                    "square":sq,
                    "white": {"before":w_old,"after":w_new},
                    "black": {"before":b_old,"after":b_new}
                }));
            }
            if let Some(current) = after.board().piece_at(sq.parse()?) {
                let attackers = if current.color == Color::White { &b_new } else { &w_new };
                let defenders = if current.color == Color::White { &w_new } else { &b_new };
                let old_attackers = if current.color == Color::White { &b_old } else { &w_old };
                let old_defenders = if current.color == Color::White { &w_old } else { &b_old };
                if was != now || attackers != old_attackers || defenders != old_defenders {
                    pressure_changes.push(json!({
                        "piece": label(after,&sq),
                        "attackers_after": attackers,
                        "defenders_after": defenders,
                        "attacker_count": attackers.len(),
                        "defender_count": defenders.len(),
                        "free_by_count": attackers.len()>defenders.len(),
                        "note": "Geometric controllers only; not a legal-capture or exchange verdict"
                    }));
                }
            }
        }
    }
    let mut xray_changes = Vec::new();
    for piece in &updated.pieces {
        if let Some(previous) = old_index.get(piece.id.as_str()) {
            if piece.xray != previous.xray {
                xray_changes.push(json!({
                    "piece":label(after,&piece.square),
                    "before":previous.xray,
                    "after":piece.xray
                }));
            }
        } else if !piece.xray.is_empty() {
            xray_changes.push(json!({
                "piece":label(after,&piece.square),
                "before":[],
                "after":piece.xray
            }));
        }
    }
    let free_targets:Vec<_>=after.board().pieces().filter_map(|(square,piece)| {
        let square=square.to_string();
        let white=controllers(&new,&square,Color::White);
        let black=controllers(&new,&square,Color::Black);
        let (a,d)=if piece.color==Color::White {(black,white)}else{(white,black)};
        (a.len()>d.len()).then(||json!({
            "piece":label(after,&square),
            "attackers":a.len(),"defenders":d.len(),
            "free_by_count":true
        }))
    }).collect();
    Ok(json!({
        "free_targets_after":free_targets,
        "occupancy":squares,
        "square_control_deltas":control_changes,
        "xray_deltas":xray_changes,
        "piece_pressure_after_changes":pressure_changes
    }))
}

fn evaluate(state: &Match, args: Args) -> Result<Value> {
    ensure!(state.variant == capablanca_chess_plus::Variant::Classic,
        "inspect_line requires Classic chess");
    ensure!((1..=8).contains(&args.moves.len()),
        "moves must contain 1-8 SAN half-moves");
    for san in &args.moves {
        ensure!(!san.is_empty() && san.len() <= 32 && san.trim() == san,
            "each move must be a non-empty SAN string without surrounding spaces (max 32 bytes)");
    }
    let mut trial = state.game().clone();
    let mut prior = match &state.controls.complete {
        Some((index,map)) if *index + 1 == state.controls.positions.len() => {
            crate::grounder::validate_engine_map(trial.position(), map)?;
            map.clone()
        }
        _ => crate::grounder::engine_map(trial.position())?
    };
    let mut steps = Vec::new();
    for (index, text) in args.moves.iter().enumerate() {
        if !matches!(trial.outcome(),GameOutcome::Ongoing | GameOutcome::Check) {
            return Ok(json!({
                "status":"illegal","at_ply":index+1,"move":text,
                "reason":"The preceding line has ended the game; no further moves are legal.",
                "valid_prefix":steps
            }));
        }
        let before = trial.position().clone();
        let mv = match before.parse_san_move(text) {
            Ok(mv) => mv,
            Err(e) => return Ok(json!({
                "status":"illegal","at_ply":index+1,"move":text,
                "side_to_move":format!("{:?}",before.side_to_move()),
                "reason":format!("SAN does not resolve to a legal move in this position: {e}"),
                "valid_prefix":steps
            }))
        };
        let normalized = before.san(mv)?;
        if normalized != *text {
            return Ok(json!({
                "status":"illegal","at_ply":index+1,"move":text,
                "side_to_move":format!("{:?}",before.side_to_move()),
                "reason":format!("Noncanonical SAN for this legal move; use {normalized}"),
                "valid_prefix":steps
            }));
        }
        if let Err(e) = trial.play(mv) {
            return Ok(json!({
                "status":"illegal","at_ply":index+1,"move":text,
                "reason":format!("Move rejected by chess engine: {e}"),
                "valid_prefix":steps
            }));
        }
        let updated = crate::grounder::engine_map(trial.position())?;
        let delta = position_delta(&before,trial.position(),&prior,&updated)?;
        steps.push(json!({
            "ply":index+1,
            "side":format!("{:?}",before.side_to_move()),
            "san":normalized,
            "uci":mv.to_uci(),
            "delta":delta,
            "check_on_side_to_move":trial.position().is_in_check(trial.position().side_to_move()),
            "outcome":format!("{:?}",trial.outcome())
        }));
        prior = updated;
    }
    Ok(json!({
        "status":"ok",
        "steps":steps,
        "final_fen":trial.position().to_fen(),
        "next_to_move":format!("{:?}",trial.position().side_to_move()),
        "outcome":format!("{:?}",trial.outcome()),
        "note":"Only differences are shown; this is engine-verified geometry, NOT a strategic or exchange evaluation."
    }))
}

pub(super) fn inspect_line(state: &Match, call: &ToolCall) -> Value {
    let result = (|| -> Result<Value> {
        ensure!(call.function.name == "inspect_line",
            "unknown Proposer tool {}",call.function.name);
        let args: Args = call.arguments().context("invalid inspect_line arguments")?;
        evaluate(state,args)
    })();
    match result {
        Ok(value)=>value,
        Err(error)=>json!({"status":"invalid_input","reason":error.to_string()})
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use capablanca_chess_plus::{Color,Variant};
    fn state(fen: &str) -> Match {
        Match::from_fen(Variant::Classic, Color::White, fen).unwrap()
    }
    fn inspect(state:&Match, moves:&[&str])->Value {
        evaluate(state,Args { moves:moves.iter().map(|s|(*s).to_owned()).collect() }).unwrap()
    }
    #[test]
    fn basic_delta_does_not_mutate_the_original_position() {
        let root = state("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1");
        let original = root.game().position().to_fen();
        let result = inspect(&root,&["e4","e5","Nf3"]);
        assert_eq!(result["status"],"ok");
        assert_eq!(result["steps"].as_array().unwrap().len(),3);
        assert_eq!(result["steps"][0]["san"],"e4");
        let changes=&result["steps"][0]["delta"]["occupancy"];
        assert!(changes.to_string().contains("e2"));
        assert!(changes.to_string().contains("e4"));
        assert!(!result.to_string().contains("wP_e4"));
        assert_eq!(root.game().position().to_fen(),original);
    }
    #[test]
    fn illegal_move_reports_ply_and_valid_prefix() {
        let root=state("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1");
        let result=inspect(&root,&["e4","e5","Qh5","Bh6"]);
        assert_eq!(result["status"],"illegal");
        assert_eq!(result["at_ply"],4);
        assert_eq!(result["valid_prefix"].as_array().unwrap().len(),3);
        let again=inspect(&root,&["e4"]);
        assert_eq!(again["status"],"ok");
        assert_eq!(root.game().position().to_fen(),"rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1");
    }
    #[test]
    fn castling_and_en_passant_changes_are_not_lost() {
        let castle=state("4k3/8/8/8/8/8/8/4K2R w K - 0 1");
        let result=inspect(&castle,&["O-O"]);
        assert_eq!(result["status"],"ok");
        let occupied=&result["steps"][0]["delta"]["occupancy"];
        for sq in ["e1","f1","g1","h1"] {assert!(occupied.to_string().contains(sq));}
        let ep=state("4k3/8/8/3pP3/8/8/8/4K3 w - d6 0 1");
        let result=inspect(&ep,&["exd6"]);
        assert_eq!(result["status"],"ok");
        let occupied=&result["steps"][0]["delta"]["occupancy"];
        for sq in ["e5","d5","d6"] {assert!(occupied.to_string().contains(sq));}
    }
    #[test]
    fn promotion_and_king_safety() {
        let p=state("7k/P7/8/8/8/8/8/4K3 w - - 0 1");
        assert_eq!(inspect(&p,&["a8=Q+"])["status"],"ok");
        assert_eq!(inspect(&p,&["a8"])["status"],"illegal");
        let pin=state("k3r3/8/8/8/8/8/4R3/4K3 w - - 0 1");
        assert_eq!(inspect(&pin,&["Rf2"])["status"],"illegal");
    }
}
