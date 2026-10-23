use capablanca_chess_plus::{Position, Variant};
use llm_match::grounder::{ControlGraph, engine_map, incremental_targets};

fn position(fen: &str) -> Position {
    Position::from_fen(Variant::Classic.rules(), fen).unwrap()
}

fn targets(fen: &str, uci: &str) -> Vec<String> {
    let before = position(fen);
    let mut after = before.clone();
    let mv = after.parse_uci_move(uci).unwrap();
    after.make_move(mv);
    let old = engine_map(&before).unwrap();
    let (map, selected) = incremental_targets(&before, &after, &old).unwrap();
    // Survivors with unchanged controls still form a valid occupancy graph.
    ControlGraph::new(&after, map.clone()).unwrap();
    let mut ids: Vec<_> = selected.iter().map(|&i| map.pieces[i].id.clone()).collect();
    ids.sort();
    ids
}

#[test]
fn bootstrap_is_geometric_including_allies_pawns_and_pins() {
    let p = position("4r1k1/8/8/8/8/8/4R3/4K3 w - - 0 1");
    let graph = ControlGraph::new(&p, engine_map(&p).unwrap()).unwrap();
    let rook = graph.get_piece("wR_e2").unwrap();
    assert!(rook.controls.contains(&"a2".into())); // pinned, still controls sideways
    assert!(rook.controls.contains(&"e1".into())); // friendly king
    assert!(rook.controls.contains(&"e8".into()));
    let p = Variant::Classic.starting_position();
    let graph = ControlGraph::new(&p, engine_map(&p).unwrap()).unwrap();
    assert_eq!(graph.get_piece("wP_e2").unwrap().controls, ["d3", "f3"]);
    assert_eq!(graph.get_piece("wR_a1").unwrap().controls, ["a2", "b1"]);
}

#[test]
fn normal_move_recalculates_mover_and_only_affected_sliders() {
    assert_eq!(targets(&Variant::Classic.starting_position().to_fen(), "e2e4"),
        ["wB_f1", "wP_e4", "wQ_d1"]);
}

#[test]
fn castling_relocates_both_pieces_and_invalidates_old_rays() {
    let ids = targets("4k3/8/8/8/8/8/8/R3K2R w KQ - 0 1", "e1g1");
    assert_eq!(ids, ["wK_g1", "wR_a1", "wR_f1"]);
}

#[test]
fn_en_passant_placeholder() {}
