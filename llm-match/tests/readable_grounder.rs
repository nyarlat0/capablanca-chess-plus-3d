use capablanca_chess_plus::{Position, Variant};
use llm_match::grounder::{ControlGraph, ControlMap, engine_map};
use serde_json::json;

fn position(fen: &str) -> Position {
    Position::from_fen(Variant::Classic.rules(), fen).unwrap()
}

#[test]
fn report_contains_every_square_and_uses_human_piece_names_only() {
    let p = position("r3k2r/p1ppqpb1/bn2pnp1/2pP4/1p2P3/2N2N2/PPQBBPPP/R3K2R w KQkq - 0 1");
    let map = engine_map(&p).unwrap();
    let graph = ControlGraph::new(&p, map).unwrap();
    let report = graph.readable_analysis();

    assert!(report.contains("White Queen on c2 | controls:"));
    assert!(report.contains("White Knight on c3"));
    assert!(report.contains("Black Queen on e7"));
    assert!(report.contains("c3: White Knight on c3 | White controllers ("));
    assert!(report.contains("d3: empty | White controllers ("));
    assert!(report.contains("White Queen on c2 [NE]: first White Pawn on e4; next Black Pawn on g6"));
    assert!(report.contains("White Queen on c2 [N]: first White Knight on c3; next Black Pawn on c5"));
    assert!(report.contains("CURRENT GEOMETRIC PRESSURE SUMMARY:"));
    for file in 'a'..='h' {
        for rank in 1..=8 {
            let square = format!("{file}{rank}: ");
            assert!(report.lines().any(|line| line.starts_with(&square)),
                "missing square {square}");
        }
    }
    for entry in graph.map().pieces.iter() {
        assert!(!report.contains(&entry.id), "internal piece ID leaked: {}", entry.id);
    }
}

#[test]
fn report_uses_model_authored_controls_instead_of_regenerating_engine_attacks() {
    // Intentionally incomplete hand-authored map: this report is a
    // projection of Grounder claims, never an attack oracle.
    let p = position("7k/8/8/4p3/3P4/P7/8/R1r1K3 w - - 0 1");
    let map: ControlMap = serde_json::from_value(json!({"version":1,"pieces":[
        {"id":"wR_a1","square":"a1","controls":["a2","a3","b1","c1"],"xray":[]},
        {"id":"wP_a3","square":"a3","controls":["b4"],"xray":[]},
        {"id":"wP_d4","square":"d4","controls":["c5","e5"],"xray":[]},
        {"id":"bP_e5","square":"e5","controls":["d4","f4"],"xray":[]},
        {"id":"wK_e1","square":"e1","controls":["d1","d2","e2","f1","f2"],"xray":[]},
        {"id":"bR_c1","square":"c1","controls":["a1","b1","c2","c3","c4","c5","c6","c7","c8","d1","e1"],"xray":[]},
        {"id":"bK_h8","square":"h8","controls":["g7","g8","h7"],"xray":[]}
    ]})).unwrap();
    let graph = ControlGraph::new(&p, map).unwrap();
    let text = graph.readable_analysis();

    assert!(text.contains("White Rook on a1 | controls: a2, a3, b1, c1"));
    assert!(text.contains("attacks opponents: Black Rook on c1"));
    assert!(text.contains("defends allies: White Pawn on a3"));
    assert!(text.contains("Black Rook on c1 | controls:"));
    assert!(text.contains("e1: White King on e1 | White controllers ("));
    assert!(text.contains("Black controllers (1): Black Rook on c1"));
    assert!(text.contains("Kings geometrically attacked: White King on e1 by Black Rook on c1"));
    assert!(!text.contains("wR_a1"));
    assert!(!text.contains("bR_c1"));
}

#[test]
fn report_explains_pin_and_discovered_attack_candidates_without_claiming_legality() {
    let pin = position("4r2k/8/8/8/8/8/4R3/4K3 w - - 0 1");
    let graph = ControlGraph::new(&pin, engine_map(&pin).unwrap()).unwrap();
    let text = graph.readable_analysis();
    assert!(text.contains(
        "Black Rook on e8 may geometrically pin White Rook on e2 to White King on e1 along S"
    ));
    assert!(text.contains("NOT an exchange evaluation"));
    assert!(text.contains("not a list of legal moves or safe captures"));

    let discovery = position("4r2k/8/8/8/8/8/4r3/K3Q3 b - - 0 1");
    let graph = ControlGraph::new(&discovery, engine_map(&discovery).unwrap()).unwrap();
    let text = graph.readable_analysis();
    assert!(text.contains(
        "moving Black Rook on e2 may uncover Black Rook on e8 toward White Queen on e1 along S"
    ));
}


#[test]
fn standalone_classic_proposer_auto_bootstraps_readable_engine_analysis() {
    use capablanca_chess_plus::Color;
    use llm_match::{Config, Match};
    use llm_match::pipeline::{Stage, debug::{self, Inputs}};

    let position = "r3k2r/p1ppqpb1/bn2pnp1/2pP4/1p2P3/2N2N2/PPQBBPPP/R3K2R w KQkq - 0 1";
    let state = Match::from_fen(Variant::Classic, Color::White, position).unwrap();
    let config = Config::load_classic_with_profile(Some("no-history"));
    let config = config.unwrap();
    // Does not call KoboldCpp. Uses engine_map => ControlGraph => model facts.
    let prompt = debug::preview(&state, &config, Stage::Proposer, &Inputs::default()).unwrap();
    assert!(prompt.contains("VERIFIED GROUNDER GEOMETRY OF THE CURRENT POSITION"));
    assert!(prompt.contains("White Queen on c2"));
    assert!(prompt.contains("d3: empty | White controllers ("));
    assert!(prompt.contains("White Queen on c2 [NE]: first White Pawn on e4; next Black Pawn on g6"));
    assert!(!prompt.contains("wQ_c2"));
    assert!(!prompt.contains("bP_g6"));
}

#[test]
fn standalone_classic_analysis_file_still_overrides_engine_bootstrap() {
    use capablanca_chess_plus::Color;
    use llm_match::{Config, Match};
    use llm_match::pipeline::{Stage, debug::{self, Inputs}};

    let state = Match::new(Variant::Classic, Color::White);
    let config = Config::load_classic_with_profile(Some("no-history")).unwrap();
    let supplied = Inputs { analysis: Some("USER_SUPPLIED_ANALYSIS".into()), ..Default::default() };
    let prompt = debug::preview(&state, &config, Stage::Proposer, &supplied).unwrap();
    assert!(prompt.contains("USER_SUPPLIED_ANALYSIS"));
    assert!(!prompt.contains("VERIFIED GROUNDER GEOMETRY OF THE CURRENT POSITION"));

    // Verifier also bootstraps when an analysis file is absent.
    let inputs = Inputs {
        candidate: Some(json!({"id":"c1","move":"e4","from":"e2","to":"e4"})),
        ..Default::default()
    };
    let prompt = debug::preview(&state, &config, Stage::Verifier, &inputs).unwrap();
    assert!(prompt.contains("VERIFIED GROUNDER GEOMETRY OF THE CURRENT POSITION"));
    assert!(prompt.contains("White Pawn on e2"));
    assert!(!prompt.contains("wP_e2"));
}

#[test]
fn threat_facts_are_compact_and_keep_attackers_defenders_and_xrays() {
    use capablanca_chess_plus::Color;
    let p = position("4r2k/8/8/8/8/8/4R3/4K3 w - - 0 1");
    let graph = ControlGraph::new(&p, engine_map(&p).unwrap()).unwrap();
    let facts = graph.threat_facts(Color::White);
    assert!(facts.contains("We play White; opponent plays Black"));
    assert!(facts.contains("White Rook on e2 | attacked by: Black Rook on e8"));
    assert!(facts.contains("defended by: White King on e1"));
    assert!(facts.contains("Black Rook on e8 [S] -> first White Rook on e2; next White King on e1"));
    assert!(facts.contains("POTENTIAL ABSOLUTE PIN ALIGNMENT"));
    assert!(!facts.contains("EVERY SQUARE"));
    assert!(!facts.contains("wR_e2"));
    assert!(!facts.contains("bR_e8"));
}

#[test]
fn isolated_threat_analyzer_uses_compact_geometry_and_proposer_accepts_report() {
    use capablanca_chess_plus::Color;
    use llm_match::{Config, Match};
    use llm_match::pipeline::{Stage, debug::{self, Inputs}};
    let state = Match::from_fen(Variant::Classic, Color::White,
        "4r2k/8/8/8/8/8/4R3/4K3 w - - 0 1").unwrap();
    let config = Config::load_classic_with_profile(Some("no-history")).unwrap();
    let prompt = debug::preview(&state, &config, Stage::ThreatAnalyzer, &Inputs::default()).unwrap();
    assert!(prompt.contains("VERIFIED CURRENT-POSITION GEOMETRY"));
    assert!(prompt.contains("Black Rook on e8 [S]"));
    assert!(!prompt.contains("EVERY SQUARE"));
    let inputs = Inputs {
        threats: Some(r#"{"immediate_threats":["GEOMETRIC: Black Rook e8 attacks e2"],"opponent_plans":[]}"#.into()),
        ..Default::default()
    };
    let proposal = debug::preview(&state, &config, Stage::Proposer, &inputs).unwrap();
    assert!(proposal.contains("GEOMETRIC: Black Rook e8 attacks e2"));
    let verifier = debug::preview(&state, &config, Stage::Verifier, &Inputs {
        candidate: Some(serde_json::json!({"id":"c1","move":"Kf1","from":"e1","to":"f1"})),
        ..Default::default()
    }).unwrap();
    assert!(!verifier.contains("Black Rook e8 attacks e2"));
}
