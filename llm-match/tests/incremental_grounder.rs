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

#[test]
fn engine_bootstrap_records_friendly_and_enemy_xray_blockers() {
    let p=position("r1bqkb1r/pp2pppp/2n2n2/2PN4/5B2/8/PPP1PPPP/R2QKBNR b KQkq - 0 5");
    let map=engine_map(&p).unwrap();
    let q=map.pieces.iter().find(|p|p.id=="bQ_d8").unwrap();
    let south=q.xray.iter().find(|r|r.direction=="S").unwrap();
    assert_eq!(south.first,"wN_d5");
    assert_eq!(south.second.as_deref(),Some("wQ_d1"));
    let west=q.xray.iter().find(|r|r.direction=="W").unwrap();
    assert_eq!(west.first,"bB_c8");
    assert_eq!(west.second.as_deref(),Some("bR_a8"));
    let southeast=q.xray.iter().find(|r|r.direction=="SE").unwrap();
    assert_eq!(southeast.first,"bP_e7");
    assert_eq!(southeast.second.as_deref(),Some("bN_f6"));
    assert!(map.pieces.iter().find(|p|p.id=="wN_d5").unwrap().xray.is_empty());
}

fn incremental_equals_oracle(fen: &str, uci: &str) {
    let before=position(fen);
    let mut after=before.clone();
    let mv=after.parse_uci_move(uci).unwrap();
    after.make_move(mv);
    let old=engine_map(&before).unwrap();
    let (mut map,selected)=incremental_targets(&before,&after,&old).unwrap();
    let oracle=engine_map(&after).unwrap();
    for index in selected {
        let item=&mut map.pieces[index];
        let expected=oracle.pieces.iter().find(|p|p.id==item.id).unwrap();
        item.controls=expected.controls.clone();
        item.xray=expected.xray.clone();
    }
    let actual=ControlGraph::new(&after,map).unwrap().map().clone();
    assert_eq!(actual,oracle,"after {uci} from {fen}");
}

#[test]
fn new_piece_behind_first_blocker_invalidates_xray() {
    let fen="7k/8/8/8/8/2n5/P7/R3K3 b - - 0 1";
    let ids=targets(fen,"c3a4");
    assert!(ids.contains(&"wR_a1".to_owned()),"missing rook refresh: {ids:?}");
    incremental_equals_oracle(fen,"c3a4");
}

#[test]
fn moving_second_blocker_invalidates_xray() {
    let fen="7k/p7/8/8/8/8/P7/R3K3 b - - 0 1";
    let ids=targets(fen,"a7a6");
    assert!(ids.contains(&"wR_a1".to_owned()),"missing rook refresh: {ids:?}");
    incremental_equals_oracle(fen,"a7a6");
}

#[test]
fn incremental_selection_preserves_both_maps_on_special_moves() {
    for (fen,uci) in [
        ("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1","e2e4"),
        ("4k3/8/8/3pP3/8/8/8/R3K3 w - d6 0 1","e5d6"),
        ("4k3/P7/8/8/8/8/8/4K3 w - - 0 1","a7a8q"),
        ("4k3/8/8/8/8/8/3p4/3QK3 w - - 0 1","d1d2"),
        ("4k3/8/8/8/8/8/8/R3K2R w KQ - 0 1","e1g1"),
        ("7k/8/8/8/8/2n5/P7/R3K3 b - - 0 1","c3a4"),
        ("7k/p7/8/8/8/8/P7/R3K3 b - - 0 1","a7a6"),
    ] {
        incremental_equals_oracle(fen,uci);
    }
}

/// Cross-check every legal move in representative positions to catch
/// missed empty squares on rays, captures and type-changing moves.
#[test]
fn incremental_selection_covers_all_legal_one_ply_transitions() {
    for fen in [
        "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
        "r1bqkb1r/pp2pppp/2n2n2/2PN4/5B2/8/PPP1PPPP/R2QKBNR b KQkq - 0 5",
        "4k3/8/8/3pP3/8/8/8/R3K3 w - d6 0 1",
        "4k3/P7/8/8/8/8/8/4K3 w - - 0 1",
        "7k/8/8/8/8/2n5/P7/R3K3 b - - 0 1",
    ] {
        let p=position(fen);
        for mv in p.legal_moves() {
            incremental_equals_oracle(fen,&mv.to_uci());
        }
    }
}
