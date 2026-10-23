use capablanca_chess_plus::{Position, Variant};
use llm_match::grounder::{ControlGraph, engine_map};
use llm_match::pins::PinReport;
use serde_json::json;
fn setup(fen: &str) -> (Position, ControlGraph) {
    let p = Position::from_fen(Variant::Classic.rules(), fen).unwrap();
    let graph = ControlGraph::new(&p, engine_map(&p).unwrap()).unwrap();
    (p, graph)
}
#[test]
fn relative_pin_to_queen_in_example_fen() {
    let (p,g)=setup("r1bqkb1r/pp2pppp/2n2n2/2PN4/5B2/8/PPP1PPPP/R2QKBNR b KQkq - 0 5");
    let answer="The d5 knight shields the d1 queen.\n{\"version\":1,\"pins\":[{\"attacker\":\"bQ_d8\",\"pinned\":\"wN_d5\",\"protected\":\"wQ_d1\",\"type\":\"relative\"}]}";
    let result=PinReport::parse(&p,&g,answer).unwrap();
    assert_eq!(result.pins.len(),1);
    assert_eq!(result.pins[0].pinned,"wN_d5");
}
#[test]
fn absolute_pin_and_empty_result() {
    let (p,g)=setup("4r1k1/8/8/8/8/8/4N3/4K3 w - - 0 1");
    let answer=r#"{"version":1,"pins":[{"attacker":"bR_e8","pinned":"wN_e2","protected":"wK_e1","type":"absolute"}]}"#;
    assert_eq!(PinReport::parse(&p,&g,answer).unwrap().pins.len(),1);
    assert!(PinReport::parse(&p,&g,r#"{"version":1,"pins":[]}"#).is_ok());
}
#[test]
fn rejects_invented_pieces_wrong_types_and_duplicate_pins() {
    let (p,g)=setup("4r1k1/8/8/8/8/8/4N3/4K3 w - - 0 1");
    let valid=json!({"version":1,"pins":[
        {"attacker":"bR_e8","pinned":"wN_e2","protected":"wK_e1","type":"absolute"}]});
    let cases=[
        json!({"version":2,"pins":[]}),
        json!({"version":1,"pins":[{"attacker":"bN_f6","pinned":"wN_e2","protected":"wK_e1","type":"absolute"}]}),
        json!({"version":1,"pins":[{"attacker":"bK_g8","pinned":"wN_e2","protected":"wK_e1","type":"absolute"}]}),
        json!({"version":1,"pins":[{"attacker":"bR_e8","pinned":"wN_e2","protected":"wK_e1","type":"relative"}]}),
        json!({"version":1,"pins":[{"attacker":"bR_e8","pinned":"wN_e2","protected":"wN_e2","type":"absolute"}]}),
        json!({"version":1,"pins":[{"attacker":"bR_e8","pinned":"wN_e2","protected":"wK_e1","type":"absolute","extra":1}]}),
        json!({"version":1,"pins":[{"attacker":"bR_e8","pinned":"wN_e2","protected":"wK_e1","type":"absolute"},{"attacker":"bR_e8","pinned":"wN_e2","protected":"wK_e1","type":"absolute"}]}),
    ];
    for value in cases {assert!(PinReport::parse(&p,&g,&value.to_string()).is_err(),"{value}");}
    assert!(PinReport::parse(&p,&g,&(valid.to_string()+" trailing")).is_err());
}
