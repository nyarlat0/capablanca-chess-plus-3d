use super::*;
use capablanca_chess_plus::{PieceKind, Position};

fn profiles() -> ProfileFile {
    ProfileFile::parse(include_str!("../representation-profiles.toml")).unwrap()
}
fn config(name: &str) -> Config {
    Config::load_with_profile(Some(name)).unwrap()
}
fn replay() -> Match {
    let mut m = Match::new(Variant::Gothic, Color::Black);
    for (i, mv) in ["e2e4", "e7e5", "d2d4", "d7d5", "a2a3", "a7a6"]
        .iter()
        .enumerate()
    {
        if i % 2 == 0 {
            m.accept_human(mv).unwrap();
        } else {
            m.accept_model(mv).unwrap();
        }
    }
    m
}

#[test]
fn coordinate_conversion_is_absolute_and_round_trips_entire_boards() {
    for variant in [Variant::Gothic, Variant::TerachessII] {
        let size = variant.rules().board_size();
        for x in 1..=u16::from(size.files()) {
            for y in 1..=u16::from(size.ranks()) {
                let s = square_from_numeric([x, y], size).unwrap();
                assert_eq!(numeric_square(s), [x, y]);
            }
        }
        for bad in [
            [0, 1],
            [1, 0],
            [u16::from(size.files()) + 1, 1],
            [1, u16::from(size.ranks()) + 1],
        ] {
            assert!(square_from_numeric(bad, size).is_err());
        }
    }
    for (xy, square) in [
        ([1, 1], "a1"),
        ([10, 1], "j1"),
        ([2, 8], "b8"),
        ([3, 6], "c6"),
    ] {
        assert_eq!(
            square_from_numeric(xy, Variant::Gothic.rules().board_size())
                .unwrap()
                .to_string(),
            square
        );
    }
    assert_eq!(
        square_from_numeric([16, 16], Variant::TerachessII.rules().board_size())
            .unwrap()
            .to_string(),
        "p16"
    );
}

#[test]
fn numeric_state_rights_inventory_and_no_mirroring() {
    let mut p = Variant::Gothic.starting_position();
    p.play_uci("e2e4").unwrap();
    let s = render_state(&p, StateFormat::Numeric);
    for expected in [
        "width: 10\nheight: 8",
        "SIDE TO MOVE\nBlack",
        "En passant: none",
        "King: (6,1)",
        "King: (6,8)",
        "Chancellor: (5,1)",
        "Pawn: (5,4)",
        "White queenside: yes",
        "BLACK PIECES",
    ] {
        assert!(s.contains(expected), "{expected}");
    }
    assert!(!s.contains("ASCII BOARD"));
    assert!(!s.contains("FEN"));
    assert!(!s.contains(&p.to_fen()));
    assert!(render_state(&p, StateFormat::NumericAscii).contains("ASCII BOARD"));
    let t = render_state(
        &Variant::TerachessII.starting_position(),
        StateFormat::Numeric,
    );
    assert!(t.contains("width: 16\nheight: 16"));
    assert!(t.contains("Castling: none"));
    assert!(t.contains("Initial king jump:\nWhite: available\nBlack: available"));
    assert!(
        render_state(&Variant::Grand.starting_position(), StateFormat::Numeric)
            .contains("INITIAL MATERIAL COUNTS")
    );
}

#[test]
fn history_profiles_filter_without_affecting_canonical_history() {
    let m = replay();
    for (name, count) in [
        ("numeric-no-history", 0),
        ("classic", 6),
        ("classic-no-history", 0),
        ("numeric", 6),
        ("numeric-json", 6),
        ("uci", 6),
    ] {
        let data = m.prompt_data_with(&config(name)).unwrap();
        assert_eq!(data.messages.len(), count);
        assert_eq!(m.uci_history().len(), 6);
    }
    let c = config("numeric");
    let data = m.prompt_data_with(&c).unwrap();
    assert_eq!(data.messages[0].content, "White Pawn: (5,2) -> (5,4)");
    assert_eq!(data.messages[5].content, "Black Pawn: (1,7) -> (1,6)");
    assert_eq!(data.messages[0].role, PromptRole::User);
    assert_eq!(data.messages[3].role, PromptRole::Assistant);
    let mut c = config("classic");
    c.representation.history_format = HistoryFormat::Numeric;
    c.representation.output_format = OutputFormat::Numeric;
    assert_eq!(
        m.prompt_data_with(&c).unwrap().messages[0].content,
        "White Pawn: (5,2) -> (5,4)"
    );
    c.representation.state_format = StateFormat::Numeric;
    c.representation.output_format = OutputFormat::Uci;
    assert!(m.prompt_data_with(&c).is_err());
    c.representation.history_mode = HistoryMode::None;
    assert!(
        m.prompt_data_with(&c).unwrap().custom["output-instructions"]
            .as_str()
            .unwrap()
            .contains("UCI")
    );
}

#[test]
fn numeric_json_is_strict_and_does_not_repair_moves() {
    let p = Variant::Gothic.starting_position();
    let good = r#"{"from":[2,8],"to":[3,6],"promotion":null}"#;
    assert_eq!(
        parse_model_move(good, OutputFormat::NumericJson, &p).unwrap(),
        "b8c6"
    );
    // Black's move in a White position: decoding succeeds; only engine validation rejects.
    let mut m = Match::new(Variant::Gothic, Color::White);
    assert!(m.accept_answer(good, OutputFormat::NumericJson).is_err());
    assert_eq!(m.wrong_moves(), [good]);
    assert!(m.uci_history().is_empty());
    for bad in [
        "Nc6",
        "b8c6",
        "{}",
        r#"{"from":[2,8],"to":[30,6],"promotion":null}"#,
        r#"{"from":[0,8],"to":[3,6],"promotion":null}"#,
        r#"{"from":[2.0,8],"to":[3,6],"promotion":null}"#,
        r#"{"from":[2,8],"to":[3,6]}"#,
        r#"{"from":[2,8],"to":[3,6],"promotion":"queen"}"#,
        r#"{"from":[2,8],"to":[3,6],"promotion":null,"extra":1}"#,
        r#"{"from":[2,8],"from":[1,1],"to":[3,6],"promotion":null}"#,
        r#"[{"from":[2,8],"to":[3,6],"promotion":null}]"#,
    ] {
        assert!(
            parse_model_move(bad, OutputFormat::NumericJson, &p).is_err(),
            "{bad}"
        );
    }
    assert!(parse_model_move(&(good.to_owned() + good), OutputFormat::NumericJson, &p).is_err());
    let promotion = r#"{"from":[5,7],"to":[5,8],"promotion":"Queen"}"#;
    assert_eq!(
        parse_model_move(promotion, OutputFormat::NumericJson, &p).unwrap(),
        "e7e8q"
    );
    // Amazon must not alias Archbishop's 'a' suffix in Gothic.
    assert!(
        parse_model_move(
            &promotion.replace("Queen", "Amazon"),
            OutputFormat::NumericJson,
            &p
        )
        .is_err()
    );
}

#[test]
fn numeric_history_uses_pre_move_identity_and_special_move_annotations() {
    for (fen, mv, expected) in [
        (
            "1r7k/P9/10/10/10/10/10/9K w - - 0 1",
            "a7b8q",
            "White Pawn: (1,7) x (2,8) = Queen",
        ),
        (
            "9k/10/10/4Pp4/10/10/10/9K w - f6 0 1",
            "e5f6",
            "White Pawn: (5,5) x (6,6) (en-passant)",
        ),
        (
            "5k4/10/10/10/10/10/10/R4K3R w KQ - 0 1",
            "f1c1",
            "White King: (6,1) -> (3,1) (castling; Rook (1,1) -> (4,1))",
        ),
    ] {
        let mut m = Match::new(Variant::Gothic, Color::White);
        m.game = Game::new(Position::from_fen(Variant::Gothic.rules(), fen).unwrap());
        m.accept_model(mv).unwrap();
        assert_eq!(m.numeric_history()[0].content, expected);
        assert_eq!(m.uci_history(), [mv]);
    }
}

#[test]
fn numeric_promotion_preserves_explicit_identity_and_canonical_history() {
    let mut m = Match::new(Variant::TerachessII, Color::White);
    let fen = format!("15k/I15/{}/15K w - - 0 1", vec!["16"; 13].join("/"));
    m.game = Game::new(Position::from_fen(Variant::TerachessII.rules(), &fen).unwrap());
    let before = m.game.position().to_fen();
    let wrong = r#"{"from":[1,15],"to":[1,16],"promotion":"Archbishop"}"#;
    assert!(m.accept_answer(wrong, OutputFormat::NumericJson).is_err());
    assert_eq!(m.game.position().to_fen(), before);
    assert!(m.uci_history().is_empty());
    let good = wrong.replace("Archbishop", "Amazon");
    m.accept_answer(&good, OutputFormat::NumericJson).unwrap();
    assert_eq!(m.uci_history(), ["a15a16a"]);
    assert_eq!(
        m.numeric_history()[0].content,
        "White Prince: (1,15) -> (1,16) = Amazon"
    );
    assert!(m.wrong_moves().is_empty());
}

#[test]
fn history_outputs_round_trip_exactly_including_capture_promotion_and_special_moves() {
    let cases = [
        (
            Variant::Gothic,
            "1r7k/P9/10/10/10/10/10/9K w - - 0 1",
            "a7b8q",
        ),
        (
            Variant::Gothic,
            "9k/10/10/4Pp4/10/10/10/9K w - f6 0 1",
            "e5f6",
        ),
        (
            Variant::Gothic,
            "5k4/10/10/10/10/10/10/R4K3R w KQ - 0 1",
            "f1c1",
        ),
        (
            Variant::TerachessII,
            "15k/16/16/16/16/16/16/16/16/16/16/16/16/16/7K8/16 w J - 0 1",
            "h2h4",
        ),
    ];
    for (variant, fen, uci) in cases {
        for format in [
            OutputFormat::Semantic,
            OutputFormat::Numeric,
            OutputFormat::Uci,
            OutputFormat::NumericJson,
        ] {
            let mut source = Match::new(variant, Color::White);
            source.game = Game::new(Position::from_fen(variant.rules(), fen).unwrap());
            let mut target = source.clone();
            source.accept_model(uci).unwrap();
            let text = match format {
                OutputFormat::Semantic => &source.history()[0].content,
                OutputFormat::Numeric => &source.numeric_history()[0].content,
                OutputFormat::Uci => &source.uci_history()[0],
                OutputFormat::NumericJson => &source.json_history[0].content,
            };
            assert_eq!(target.accept_answer(text, format).unwrap().to_uci(), uci);
            assert_eq!(target.uci_history(), source.uci_history());
            assert_eq!(target.history()[0].content, source.history()[0].content);
            assert_eq!(
                target.numeric_history()[0].content,
                source.numeric_history()[0].content
            );
        }
    }
}

#[test]
fn history_output_is_strict_and_checks_identity_color_capture_and_annotations() {
    for format in [OutputFormat::Semantic, OutputFormat::Numeric] {
        let mut m = Match::new(Variant::Gothic, Color::White);
        let good = if format == OutputFormat::Semantic {
            "White Pawn e2-e4"
        } else {
            "White Pawn: (5,2) -> (5,4)"
        };
        let before = m.game.position().to_fen();
        for bad in [
            "e2e4".into(),
            r#"{"from":[5,2],"to":[5,4],"promotion":null}"#.into(),
            format!("I choose {good}"),
            format!("{good}\n{good}"),
            good.replace("White", "white"),
            good.replace("White", "Black"),
            good.replace("Pawn", "Queen"),
            good.replace("-", "x"),
            good.replace(" -> ", " x ").replace("e2-e4", "e2xe4"),
            format!("{good} (en-passant)"),
            format!("{good} and then something else"),
        ] {
            assert!(m.accept_answer(&bad, format).is_err(), "{bad}");
            assert_eq!(m.game.position().to_fen(), before);
            assert!(m.uci_history().is_empty());
        }
        assert_eq!(m.accept_answer(good, format).unwrap().to_uci(), "e2e4");
    }
    let mut c = config("numeric");
    c.representation.output_format = OutputFormat::NumericJson;
    assert!(
        c.representation
            .validate()
            .unwrap_err()
            .to_string()
            .contains("requires output_format=numeric")
    );
    c.representation.history_format = HistoryFormat::Semantic;
    c.representation.output_format = OutputFormat::Numeric;
    assert!(c.representation.validate().is_err());
}

#[test]
fn every_profile_has_an_identical_no_history_partner_and_matching_output() {
    let p = profiles();
    for (name, with) in &p.profiles {
        if name.ends_with("-no-history") {
            continue;
        }
        let mut without = p.profiles[&format!("{name}-no-history")].clone();
        assert_eq!(without.history_mode, HistoryMode::None);
        without.history_mode = HistoryMode::Full;
        assert_eq!(&without, with);
        assert_eq!(with.history_format.name(), with.output_format.name());
        let m = replay();
        let data = m.prompt_data_with(&config(name)).unwrap();
        assert_eq!(data.messages.len(), m.uci_history().len());
        let mut receiver = Match::new(Variant::Gothic, Color::White);
        let first = &data.messages[0].content;
        assert_eq!(
            receiver
                .accept_answer(first, with.output_format)
                .unwrap()
                .to_uci(),
            m.uci_history()[0]
        );
        if with.output_format == OutputFormat::NumericJson {
            assert_eq!(first, r#"{"from":[5,2],"to":[5,4],"promotion":null}"#);
        }
        if with.output_format == OutputFormat::Uci {
            assert_eq!(first, "e2e4");
        }
    }
}

#[tokio::test]
async fn context_fitting_only_trims_requests_when_necessary_and_preserves_state() {
    struct Counter(bool);
    impl prompt_core::TokenCounter for Counter {
        fn count_tokens<'a>(
            &'a self,
            text: &'a str,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<u64>> + Send + 'a>>
        {
            Box::pin(async move {
                Ok(
                    if self.0
                        && (text.contains("White Pawn e2-e4") || text.contains("Black Pawn e7-e5"))
                    {
                        20000
                    } else {
                        512
                    },
                )
            })
        }
    }
    let m = replay();
    let c = config("classic");
    let data = m.prompt_data_with(&c).unwrap();
    let before = m.game.position().to_fen();
    for trim in [false, true] {
        let fitted = PromptBuilder::new()
            .build_with_budget(
                &c.context,
                &c.instruct,
                &c.system,
                &data,
                &Counter(trim),
                prompt_core::ContextBudget::new(16384, 4000),
            )
            .await
            .unwrap();
        assert_eq!(fitted.dropped_messages, if trim { 2 } else { 0 });
        assert!(fitted.prompt.text.contains("Black Pawn a7-a6"));
        assert!(
            fitted
                .prompt
                .text
                .contains("AUTHORITATIVE CURRENT POSITION")
        );
        assert_eq!(fitted.prompt.text.contains("White Pawn e2-e4"), !trim);
        assert_eq!(data.messages.len(), 6);
        assert_eq!(m.uci_history().len(), 6);
        assert_eq!(m.game.position().to_fen(), before);
    }
}

#[test]
fn profile_schema_validation_and_override_selection() {
    let p = profiles();
    assert_eq!(p.select(None).unwrap().0, "classic");
    assert_eq!(
        p.select(Some("numeric-no-history")).unwrap().0,
        "numeric-no-history"
    );
    assert!(p.select(Some("missing")).is_err());
    assert!(
        ProfileFile::parse(&include_str!("../representation-profiles.toml").replace(
            "history_mode = \"full\"",
            "history_mode = \"full\"\nhistory_count = 4"
        ))
        .is_err()
    );
    for count in ["", "history_count=0", "history_count=-1"] {
        assert!(ProfileFile::parse(&format!("active_profile='x'\n[profiles.x]\nstate_format='numeric'\nhistory_mode='last_n'\nhistory_format='numeric'\noutput_format='numeric_json'\n{count}")).is_err());
    }
    assert!(
        ProfileFile::parse(
            &include_str!("../representation-profiles.toml").replace("state_format", "state_typo")
        )
        .is_err()
    );
    assert_eq!(
        cli_profile(["--llm-profile", "classic"].map(str::to_owned)).unwrap(),
        Some("classic".into())
    );
    assert!(cli_profile(["--llm-profile".to_owned()]).is_err());
    assert!(cli_profile(["--llm-profile", "--black"].map(str::to_owned)).is_err());
}

#[test]
fn classic_compatibility_and_all_profiles_have_no_assistance_leaks() {
    let mut c = config("classic");
    let m = replay();
    let data = m.prompt_data_with(&c).unwrap();
    assert_eq!(
        data.messages.iter().map(|m| &m.content).collect::<Vec<_>>(),
        m.history().iter().map(|m| &m.content).collect::<Vec<_>>()
    );
    assert!(
        data.custom["board-state"]
            .as_str()
            .unwrap()
            .contains(&board_state(m.game().position()))
    );
    for (_, profile) in profiles().profiles {
        for variant in Variant::ALL {
            if variant == Variant::Classic {
                continue;
            } // separate SAN configuration
            c.representation = profile.clone();
            let m = Match::new(variant, Color::White);
            let data = m.prompt_data_with(&c).unwrap();
            let built = PromptBuilder::new()
                .build(&c.context, &c.instruct, &c.system, &data)
                .unwrap();
            assert!(!built.text.contains("FEN"));
            for mv in m.game().position().legal_moves() {
                assert!(!built.text.contains(&mv.to_uci()), "{} leaked", mv.to_uci());
            }
            if profile.output_format == OutputFormat::NumericJson {
                assert!(!built.text.contains("canonical UCI"));
            }
            if profile.state_format == StateFormat::Numeric {
                assert!(!built.text.contains("ASCII board"));
                assert!(!built.text.contains("ASCII BOARD"));
            }
        }
    }
}

#[test]
fn pure_numeric_prompts_never_mention_letter_coordinates() {
    for name in [
        "numeric",
        "numeric-no-history",
        "numeric-json",
        "numeric-json-no-history",
    ] {
        let mut c = config(name);
        for template in [
            "Chess.json",
            "Chess_artemis_think.json",
            "Chess_nothink.json",
        ] {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("templates/sysprompt")
                .join(template);
            if !path.exists() {
                continue;
            }
            c.system = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
            for variant in [Variant::Gothic, Variant::TerachessII] {
                let mut m = Match::new(variant, Color::Black);
                if variant == Variant::Gothic {
                    m.accept_human("e2e4").unwrap();
                }
                let data = m.prompt_data_with(&c).unwrap();
                let prompt = PromptBuilder::new()
                    .build(&c.context, &c.instruct, &c.system, &data)
                    .unwrap()
                    .text;
                for forbidden in [
                    "a=1",
                    "b=2",
                    "file a",
                    "file letters",
                    "file/rank",
                    "SAN",
                    "UCI",
                    "letter itself",
                    "a b c",
                    "e2e4",
                    "e2-e4",
                ] {
                    assert!(
                        !prompt.contains(forbidden),
                        "{name}/{template}/{variant:?}: {forbidden}"
                    );
                }
                assert!(prompt.contains("increases left to right"));
                assert!(prompt.contains("NEVER mirrored or rotated for Black"));
            }
        }
    }
    // The explicitly hybrid experiment still has its alphabetic ASCII axis.
    assert!(
        render_state(
            &Variant::Gothic.starting_position(),
            StateFormat::NumericAscii
        )
        .contains("a b c")
    );
}

#[test]
fn rules_templates_are_editable_and_geometry_is_engine_owned() {
    let mut c = config("numeric-no-history");
    c.rules_templates.insert(
        "gothic".into(),
        "Experiment marker\n{{board-header}}\n{{piece-legend}}".into(),
    );
    let m = Match::new(Variant::Gothic, Color::White);
    assert!(
        m.prompt_data_with(&c).unwrap().custom["chess-rules"]
            .as_str()
            .unwrap()
            .starts_with("Experiment marker")
    );
    let t = Match::new(Variant::TerachessII, Color::White)
        .prompt_data_with(&c)
        .unwrap();
    let rules = t.custom["chess-rules"].as_str().unwrap();
    assert!(rules.contains(&PieceKind::Giraffe.geometric_description()));
    assert!(rules.contains("(2,3)/(3,2)"));
    assert!(!rules.contains("(1,4)"));
    for kind in PieceKind::ALL {
        assert!(!kind.geometric_description().is_empty());
    }
}
