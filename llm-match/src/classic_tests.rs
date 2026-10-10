use super::*;

#[test]
fn common_classic_template_renders_ephemeral_user_after_san_history() {
    let mut m = Match::new(Variant::Classic, Color::Black);
    for profile in ["with-history", "no-history"] {
        let mut c = Config::load_classic_with_profile(Some(profile)).unwrap();
        assert_eq!(m.prompt_data_with(&c).unwrap().custom["last-move"], "");
        c.system.content = "BEGIN".into();
        c.system.user_prompt =
            "QUESTION {{#if last-move}}{{last-move}}{{else}}none{{/if}} {{fen}} {{ascii}}".into();
        c.system.post_history = "POST {{last-move}}".into();
        m.accept_human("e2e4").unwrap();
        let data = m.prompt_data_with(&c).unwrap();
        assert_eq!(data.custom["last-move"], "e4");
        assert_eq!(data.messages.len(), usize::from(profile == "with-history"));
        c.instruct.input_sequence = "[USER]".into();
        c.instruct.last_input_sequence = "[LAST USER]".into();
        c.instruct.system_sequence = "[SYSTEM]".into();
        c.instruct.system_same_as_user = false;
        let text = PromptBuilder::new()
            .build(&c.context, &c.instruct, &c.system, &data)
            .unwrap()
            .text;
        assert!(text.contains("[LAST USER]QUESTION e4"), "{text}");
        assert!(text.find("QUESTION e4").unwrap() < text.find("[SYSTEM]POST e4").unwrap());
        assert!(text.contains(&m.game.position().to_fen()));
        assert_eq!(m.uci_history(), ["e2e4"]);
        m = Match::new(Variant::Classic, Color::Black);
    }
    let a = Config::load_classic_with_profile(Some("with-history")).unwrap();
    let b = Config::load_classic_with_profile(Some("no-history")).unwrap();
    assert_eq!(
        serde_json::to_value(a.system).unwrap(),
        serde_json::to_value(b.system).unwrap()
    );
}

#[test]
fn final_san_suffix_accepts_prose_but_never_substitutes_an_earlier_move() {
    let mut c = Config::load_classic_with_profile(Some("with-history")).unwrap();
    c.reasoning.prefix = "<thinking>".into();
    c.reasoning.suffix = "</thinking>".into();
    let mut m = Match::new(Variant::Classic, Color::White);
    for text in [
        "<thinking>e4",
        "<thinking>e4</thinking>",
        "e4 followed by prose",
    ] {
        assert!(m.accept_configured_answer(text, &c).is_err());
        assert!(m.wrong_moves().is_empty());
        assert!(m.uci_history().is_empty());
    }
    assert!(
        m.accept_configured_answer("Could play e4, but my answer: Nf6", &c)
            .is_err()
    );
    assert_eq!(m.wrong_moves(), ["Nf6"]);
    assert!(m.uci_history().is_empty());
    m.accept_configured_answer(
        "<thinking>Consider options.</thinking>My choice: 1.e4  ",
        &c,
    )
    .unwrap();
    assert_eq!(m.uci_history(), ["e2e4"]);
    assert_eq!(m.prompt_data_with(&c).unwrap().messages[0].content, "e4");
}

#[test]
fn classic_history_and_directives_are_engine_derived_and_separate() {
    let mut m = Match::new(Variant::Classic, Color::Black);
    let c = Config::load_classic_with_profile(Some("with-history")).unwrap();
    m.accept_human("e2e4").unwrap();
    assert!(m.accept_configured_answer("e6+", &c).is_err());
    assert_eq!(m.classic_san_history(), "1. e4");
    m.accept_configured_answer("e5", &c).unwrap();
    m.accept_human("g1f3").unwrap();
    m.accept_configured_answer("Nc6", &c).unwrap();
    assert_eq!(m.classic_san_history(), "1. e4 e5 2. Nf3 Nc6");
    assert_eq!(m.uci_history(), ["e2e4", "e7e5", "g1f3", "b8c6"]);
    assert!(m.wrong_moves().is_empty());
    for (name, show) in [("with-history", true), ("no-history", false)] {
        let c = Config::load_classic_with_profile(Some(name)).unwrap();
        let data = m.prompt_data_with(&c).unwrap();
        assert_eq!(data.custom["fen"], m.game.position().to_fen());
        assert_eq!(
            data.custom["ascii"],
            prompts::ascii_board(m.game.position())
        );
        assert_eq!(
            data.custom["classic-san-history"],
            if show {
                m.classic_san_history()
            } else {
                String::new()
            }
        );
        let built = PromptBuilder::new()
            .build(&c.context, &c.instruct, &c.system, &data)
            .unwrap();
        assert_eq!(data.messages.len(), if show { 4 } else { 0 });
        if show {
            for (message, (san, role)) in data.messages.iter().zip([
                ("e4", PromptRole::User),
                ("e5", PromptRole::Assistant),
                ("Nf3", PromptRole::User),
                ("Nc6", PromptRole::Assistant),
            ]) {
                assert_eq!(message.content, san);
                assert_eq!(message.role, role);
                assert!(built.text.contains(san));
            }
        }
        assert!(built.text.contains(&m.game.position().to_fen()));
        assert!(!built.text.contains("White Pawn e2-e4"));
        for mv in m.game.position().legal_moves() {
            assert!(!built.text.contains(&mv.to_uci()));
        }
    }
    let fairy = Config::load_with_profile(Some("classic")).unwrap();
    assert!(fairy.classic.is_none());
    assert_eq!(fairy.representation.output_format, OutputFormat::Semantic);
    let gothic = Match::new(Variant::Gothic, Color::White);
    assert!(gothic.prompt_data_with(&c).is_err());
    assert!(m.prompt_data_with(&fairy).is_err());
    assert!(
        !gothic
            .prompt_data_with(&fairy)
            .unwrap()
            .custom
            .as_object()
            .unwrap()
            .contains_key("fen")
    );
}

#[test]
fn classic_rejects_prose_uci_and_noncanonical_san_without_changing_state() {
    let mut m = Match::new(Variant::Classic, Color::White);
    let c = Config::load_classic_with_profile(Some("with-history")).unwrap();
    let fen = m.game.position().to_fen();
    for bad in [
        "e2e4",
        "1. e4 then stop",
        "I choose e4 then stop",
        "e4!",
        "e4 e5 then stop",
        "<thinking>e4 unfinished",
        "nf3",
    ] {
        assert!(m.accept_configured_answer(bad, &c).is_err(), "{bad}");
        assert!(m.wrong_moves().is_empty());
    }
    assert!(m.accept_configured_answer("Nf6", &c).is_err());
    assert_eq!(m.wrong_moves(), ["Nf6"]);
    assert_eq!(m.game.position().to_fen(), fen);
    assert!(m.classic_san_history().is_empty());
    assert!(m.prompt_data_with(&c).unwrap().messages.is_empty());
    assert_eq!(
        m.accept_configured_answer(" Nf3\n", &c).unwrap().to_uci(),
        "g1f3"
    );
    assert_eq!(m.classic_san_history(), "1. Nf3");
    m.accept_human("d7d5").unwrap();
    let messages = m.prompt_data_with(&c).unwrap().messages;
    assert_eq!(messages[0].content, "Nf3");
    assert_eq!(messages[0].role, PromptRole::Assistant);
    assert_eq!(messages[1].content, "d5");
    assert_eq!(messages[1].role, PromptRole::User);
    assert!(m.wrong_moves().is_empty());
}

#[test]
fn classic_history_keeps_checks_promotions_and_black_start_numbering() {
    let c = Config::load_classic_with_profile(Some("with-history")).unwrap();
    let mut m = Match::new(Variant::Classic, Color::Black);
    m.game = Game::new(
        capablanca_chess_plus::Position::from_fen(
            Variant::Classic.rules(),
            "7k/8/8/8/8/8/p7/7K b - - 0 23",
        )
        .unwrap(),
    );
    m.accept_configured_answer("a1=Q+", &c).unwrap();
    assert_eq!(m.classic_san_history(), "23... a1=Q+");
    assert_eq!(m.uci_history(), ["a2a1q"]);
    let mut m = Match::new(Variant::Classic, Color::Black);
    m.accept_human("f2f3").unwrap();
    m.accept_configured_answer("e5", &c).unwrap();
    m.accept_human("g2g4").unwrap();
    m.accept_configured_answer("Qh4#", &c).unwrap();
    assert_eq!(m.classic_san_history(), "1. f3 e5 2. g4 Qh4#");
    assert!(matches!(
        m.game().outcome(),
        GameOutcome::Win {
            winner: Color::Black
        }
    ));
}
