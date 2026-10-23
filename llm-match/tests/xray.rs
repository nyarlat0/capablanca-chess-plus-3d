use capablanca_chess_plus::{Position, Variant};
use llm_match::grounder::{engine_map, validate_one, OnePieceControls, XRay};

fn position() -> Position {
    Position::from_fen(Variant::Classic.rules(),
        "r1bqkb1r/pp2pppp/2n2n2/2PN4/5B2/8/PPP1PPPP/R2QKBNR b KQkq - 0 5").unwrap()
}

#[test]
fn accepts_model_xray_and_rejects_wrong_ray_ids_and_directions() {
    let p=position();
    let map=engine_map(&p).unwrap();
    let q=map.pieces.iter().find(|p|p.id=="bQ_d8").unwrap();
    let text=serde_json::json!({"controls":q.controls,"xray":q.xray}).to_string();
    let correct=OnePieceControls::parse(&text).unwrap();
    validate_one(&p,"d8",&correct).unwrap();

    // The provided xray is model-supplied, never repaired into a valid one.
    let mut wrong=correct.clone();
    wrong.xray.iter_mut().find(|r|r.direction=="S").unwrap().second=Some("bR_a8".into());
    assert!(validate_one(&p,"d8",&wrong).is_err());

    let mut wrong=correct.clone();
    wrong.xray.iter_mut().find(|r|r.direction=="S").unwrap().first="bR_a8".into();
    assert!(validate_one(&p,"d8",&wrong).is_err());

    let mut wrong=correct.clone();
    wrong.xray.push(XRay{direction:"S".into(), first:"wN_d5".into(), second:None});
    assert!(validate_one(&p,"d8",&wrong).is_err());

    let mut wrong=correct.clone();
    wrong.xray.push(XRay{direction:"UP".into(),first:"wN_d5".into(),second:None});
    assert!(validate_one(&p,"d8",&wrong).is_err());

    // Non-sliders cannot return xray relations.
    let mut knight=OnePieceControls::parse(r#"{"controls":["b4","b6","c3","c7","e3","e7","f4","f6"],"xray":[]}"#).unwrap();
    knight.xray.push(XRay{direction:"N".into(),first:"bQ_d8".into(),second:None});
    assert!(validate_one(&p,"d5",&knight).is_err());
}

#[test]
fn new_model_response_must_explicitly_have_xray_including_empty() {
    assert!(OnePieceControls::parse(r#"{"controls":["b4"]}"#).is_err());
    assert!(OnePieceControls::parse(r#"{"controls":["b4"],"xray":[]}"#).is_ok());
    assert!(OnePieceControls::parse(r#"{"controls":["b4"],"xray":null}"#).is_err());
}
