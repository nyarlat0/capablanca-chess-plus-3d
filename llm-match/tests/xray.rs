use capablanca_chess_plus::{Position, Variant};
use llm_match::grounder::{engine_map, validate_one, OnePieceControls, XRay};

fn position() -> Position {
    Position::from_fen(Variant::Classic.rules(),
        "r1bqkb1r/pp2pppp/2n2n2/2PN4/5B2/8/PPP1PPPP/R2QKBNR b KQkq - 0 5").unwrap()
}

fn queen() -> (Position, OnePieceControls) {
    let p=position();
    let map=engine_map(&p).unwrap();
    let queen=map.pieces.iter().find(|p|p.id=="bQ_d8").unwrap();
    let payload=serde_json::json!({"controls":queen.controls,"xray":queen.xray}).to_string();
    let parsed=OnePieceControls::parse(&payload).unwrap();
    (p,parsed)
}

#[test]
fn model_needs_only_coordinates_not_piece_ids() {
    let (p,model)=queen();
    validate_one(&p,"d8",&model).unwrap();
    let south=model.xray.iter().find(|r|r.direction=="S").unwrap();
    assert_eq!(south.first,"d5");
    assert_eq!(south.second.as_deref(),Some("d1"));
    assert!(model.xray.iter().any(|r|r.direction=="E" && r.first=="e8" && r.second.as_deref()==Some("f8")));
    let json=serde_json::to_string(&model).unwrap();
    assert!(!json.contains("wN_d5"));
    assert!(!json.contains("bK_e8"));
}

#[test]
fn reject_wrong_or_missing_second_even_when_valid_coordinates() {
    let (p,correct)=queen();
    let mut wrong=correct.clone();
    wrong.xray.iter_mut().find(|r|r.direction=="S").unwrap().second=Some("d2".into());
    assert!(validate_one(&p,"d8",&wrong).is_err());

    let mut wrong=correct.clone();
    wrong.xray.iter_mut().find(|r|r.direction=="S").unwrap().second=None;
    assert!(validate_one(&p,"d8",&wrong).is_err());

    let mut wrong=correct.clone();
    wrong.xray.iter_mut().find(|r|r.direction=="S").unwrap().second=Some("a8".into());
    assert!(validate_one(&p,"d8",&wrong).is_err());

    let mut wrong=correct.clone();
    wrong.xray.iter_mut().find(|r|r.direction=="S").unwrap().first="d1".into();
    assert!(validate_one(&p,"d8",&wrong).is_err());
}

#[test]
fn reject_invalid_ray_duplicate_directions_and_old_piece_ids() {
    let (p,correct)=queen();
    let mut wrong=correct.clone();
    wrong.xray.push(XRay{direction:"S".into(),first:"d5".into(),second:None});
    assert!(validate_one(&p,"d8",&wrong).is_err());

    let mut wrong=correct.clone();
    wrong.xray.push(XRay{direction:"UP".into(),first:"d5".into(),second:None});
    assert!(validate_one(&p,"d8",&wrong).is_err());

    assert!(OnePieceControls::parse(r#"{"controls":["d5"],"xray":[{"direction":"S","first":"wN_d5","second":"wQ_d1"}]}"#).is_err());

    let mut knight=OnePieceControls::parse(r#"{"controls":["b4","b6","c3","c7","e3","e7","f4","f6"],"xray":[]}"#).unwrap();
    knight.xray.push(XRay{direction:"N".into(),first:"d8".into(),second:None});
    assert!(validate_one(&p,"d5",&knight).is_err());
}

#[test]
fn response_requires_xray_even_when_empty() {
    assert!(OnePieceControls::parse(r#"{"controls":["b4"]}"#).is_err());
    assert!(OnePieceControls::parse(r#"{"controls":["b4"],"xray":[]}"#).is_ok());
    assert!(OnePieceControls::parse(r#"{"controls":["b4"],"xray":null}"#).is_err());
}

#[test]
fn engine_rejects_incomplete_controls_even_with_valid_xray() {
    let (p,mut actual)=queen();
    // Omitting the entire empty southwest ray was a real model failure.
    actual.controls.retain(|cell|cell!="a5" && cell!="b6" && cell!="c7");
    let error=llm_match::grounder::validate_one(&p,"d8",&actual).unwrap_err();
    let message=format!("{error:#}");
    assert!(message.contains("engine controls mismatch"),"{message}");
    assert!(message.contains("a5"),"{message}");
}

#[test]
fn engine_rejects_missing_xray_direction_and_accepts_arbitrary_order() {
    let (p,mut actual)=queen();
    actual.xray.retain(|ray|ray.direction!="SE");
    let error=validate_one(&p,"d8",&actual).unwrap_err();
    assert!(format!("{error:#}").contains("engine xray mismatch"));

    let (p,mut valid)=queen();
    valid.controls.reverse();
    valid.xray.reverse();
    validate_one(&p,"d8",&valid).unwrap();
}
