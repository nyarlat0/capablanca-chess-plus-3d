use super::*;

#[test]
fn rejected_output_never_changes_position_or_history() {
    let mut state = Match::new(Variant::Gothic, Color::Black);
    state.accept_human("e2e4").unwrap();
    let fen = state.game().position().to_fen();
    for invalid in [
        "I choose e7e5",
        "e7e5 e2e4",
        "```e7e5```",
        "E7E5",
        "e7e5.",
        "e7e4",
        "",
        "e7e5\nThanks",
    ] {
        assert!(state.accept_model(invalid).is_err(), "{invalid}");
        assert_eq!(state.game().position().to_fen(), fen);
        assert_eq!(state.history().len(), 1);
    }
    state.accept_model(" \ne7e5\t").unwrap();
    assert!(state.wrong_moves().is_empty());
    assert_eq!(state.history()[0].role, PromptRole::User);
    assert_eq!(state.history()[1].role, PromptRole::Assistant);
    assert_eq!(state.history()[1].content, "e7e5");
    assert!(state.accept_model("e5e4").is_err());
    assert!(state.wrong_moves().is_empty());
}

#[test]
fn rules_cover_every_variant_and_ascii_preserves_fen_symbols() {
    for variant in Variant::ALL {
        let position = variant.starting_position();
        let rules = chess_rules(variant);
        assert!(rules.contains(variant.rules().name()));
        assert!(rules.contains(&format!(
            "{}x{}",
            position.board().size().files(),
            position.board().size().ranks()
        )));
        let board = board_state(&position);
        assert!(board.contains(&position.to_fen()));
        let rows: Vec<_> = board
            .lines()
            .filter(|line| {
                line.trim_start()
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_digit())
            })
            .collect();
        assert_eq!(rows.len(), usize::from(position.board().size().ranks()));
        for row in rows {
            assert_eq!(
                row.split_whitespace().count(),
                usize::from(position.board().size().files()) + 1
            );
        }
    }
    assert!(board_state(&Variant::TerachessII.starting_position()).contains("X"));
}

#[test]
fn model_can_start_and_use_two_digit_ranks() {
    for variant in Variant::ALL {
        let mut state = Match::new(variant, Color::White);
        let mv = state.game().position().legal_moves()[0].to_uci();
        assert!(state.accept_human(&mv).is_err());
        state.accept_model(&mv).unwrap();
        assert_eq!(state.history()[0].role, PromptRole::Assistant);
    }
    let mut state = Match::new(Variant::TerachessII, Color::Black);
    let mv = state.game().position().legal_moves()[0].to_uci();
    state.accept_human(&mv).unwrap();
    let mv = state
        .game()
        .position()
        .legal_moves()
        .into_iter()
        .find(|m| m.from.rank() >= 9)
        .unwrap()
        .to_uci();
    state.accept_model(&mv).unwrap();
}

#[test]
fn promotion_suffix_is_required_and_canonical() {
    let mut state = Match::new(Variant::Gothic, Color::White);
    state.game = Game::new(
        capablanca_chess_plus::Position::from_fen(
            Variant::Gothic.rules(),
            "9k/P9/10/10/10/10/10/9K w - - 0 1",
        )
        .unwrap(),
    );
    assert!(state.accept_model("a7a8").is_err());
    assert!(state.accept_model("a7a8Q").is_err());
    state.accept_model("a7a8a").unwrap();
    assert_eq!(state.history()[0].content, "a7a8a");
}

#[test]
fn post_history_directives_render_after_accepted_moves_only() {
    let config = Config::load().unwrap();
    let mut state = Match::new(Variant::Gothic, Color::Black);
    state.accept_human("e2e4").unwrap();
    assert!(state.accept_model("e7e4").is_err());
    let built = PromptBuilder::new()
        .build(
            &config.context,
            &config.instruct,
            &config.system,
            &state.prompt_data(),
        )
        .unwrap();
    let history = built.text.find("e2e4").unwrap();
    let board = built.text.find("AUTHORITATIVE CURRENT POSITION").unwrap();
    let rejected = built.text.find("Rejected illegal").unwrap();
    assert!(history < board && board < rejected);
    assert!(built.text[rejected..].contains("e7e4"));
    assert!(!built.text.contains("{{board-state}}"));
    assert!(!built.text.contains("{{wrong-move}}"));
    assert!(!built.text.contains("e7e5")); // legal replies are never supplied
    state.accept_model("e7e5").unwrap();
    let built = PromptBuilder::new()
        .build(
            &config.context,
            &config.instruct,
            &config.system,
            &state.prompt_data(),
        )
        .unwrap();
    assert!(!built.text.contains("Rejected illegal"));
    assert!(!built.text.contains("e7e4"));
}

#[test]
fn endpoint_normalization() {
    assert_eq!(
        normalize_endpoint(" aistation:5001/ ").unwrap(),
        "http://aistation:5001"
    );
    for invalid in ["", "ftp://aistation", "http://aistation?key=secret"] {
        assert!(normalize_endpoint(invalid).is_err());
    }
}
