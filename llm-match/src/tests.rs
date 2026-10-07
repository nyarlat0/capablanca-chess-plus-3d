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
    assert_eq!(state.history()[1].content, "Black Pawn e7-e5");
    assert_eq!(state.uci_history(), ["e2e4", "e7e5"]);
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
        assert!(!board.contains(&position.to_fen()));
        assert!(!board.contains("FEN"));
        assert!(board.contains("CURRENT PIECES"));
        assert!(board.contains("En passant: none"));
        assert_eq!(board, board_state(&position));
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
    assert_eq!(state.history()[0].content, "White Pawn a7-a8=Archbishop");
    assert_eq!(state.uci_history(), ["a7a8a"]);
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
    let history = built.text.find("White Pawn e2-e4").unwrap();
    let board = built.text.find("AUTHORITATIVE CURRENT POSITION").unwrap();
    let rejected = built.text.find("Rejected moves").unwrap();
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
    assert!(!built.text.contains("Rejected moves"));
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

#[test]
fn reasoning_is_stripped_without_splicing_or_extraction() {
    let t = prompt_core::ReasoningTemplate {
        name: "test".into(),
        prefix: "<thinking>".into(),
        suffix: "</thinking>".into(),
        separator: "\n".into(),
    };
    for raw in [
        "<thinking>source and path</thinking>\ne7e5",
        " <thinking>first</thinking><thinking>second</thinking> e7e5 ",
    ] {
        assert_eq!(
            output::final_answer(&t, raw).unwrap().as_deref(),
            Some("e7e5")
        );
    }
    for raw in [
        "<thinking>cut off e7e5",
        "<thinking>done</thinking>  ",
        "<thinking>done</thinking><thinking>unfinished",
        "e7<thinking>x</thinking>e5",
    ] {
        assert!(output::final_answer(&t, raw).unwrap().is_none());
    }
    let prose = output::final_answer(&t, "<thinking>x</thinking>I choose e7e5")
        .unwrap()
        .unwrap();
    assert!(!output::is_coordinate_move(&prose));
    let artemis: prompt_core::ReasoningTemplate =
        serde_json::from_str(include_str!("../templates/reasoning/Artemis_think.json")).unwrap();
    assert_eq!(
        output::final_answer(
            &artemis,
            &format!("{}secret</thinking>\ne7e5", artemis.prefix)
        )
        .unwrap()
        .as_deref(),
        Some("e7e5")
    );
    assert!(
        output::final_answer(&artemis, "<thinking>secret")
            .unwrap()
            .is_none()
    );
    assert_eq!(
        output::final_answer(&artemis, "<thinking>secret</thinking>e7e5")
            .unwrap()
            .as_deref(),
        Some("e7e5")
    );
    for invalid in [
        "E7E5",
        "e7e5.",
        "e7e5 e2e4",
        "e07e5",
        "e7e05",
        "<thinking>e7e5",
        "e7e5\nThanks",
        "",
    ] {
        assert!(!output::is_coordinate_move(invalid));
    }
    for valid in ["e7e5", "d15g12", "a7a8q"] {
        assert!(output::is_coordinate_move(valid));
    }
}

fn semantic(variant: Variant, fen: &str, uci: &str) -> String {
    let p = capablanca_chess_plus::Position::from_fen(variant.rules(), fen).unwrap();
    let mut state = Match::new(variant, p.side_to_move());
    state.game = Game::new(p);
    state.accept_model(uci).unwrap();
    assert_eq!(state.uci_history(), [uci]);
    state.history()[0].content.clone()
}

#[test]
fn semantic_capture_promotion_castling_and_en_passant() {
    assert_eq!(
        semantic(
            Variant::Gothic,
            "9k/10/10/4p5/10/10/1B8/9K w - - 0 1",
            "b2e5"
        ),
        "White Bishop b2xe5"
    );
    assert_eq!(
        semantic(
            Variant::Gothic,
            "1r7k/P9/10/10/10/10/10/9K w - - 0 1",
            "a7b8q"
        ),
        "White Pawn a7xb8=Queen"
    );
    assert_eq!(
        semantic(
            Variant::Gothic,
            "5k4/10/10/10/10/10/10/R4K3R w KQ - 0 1",
            "f1c1"
        ),
        "White King f1-c1 (castling; Rook a1-d1)"
    );
    assert_eq!(
        semantic(
            Variant::Gothic,
            "9k/10/10/4Pp4/10/10/10/9K w - f6 0 1",
            "e5f6"
        ),
        "White Pawn e5xf6 (en-passant)"
    );
    assert_eq!(
        semantic(
            Variant::Gothic,
            "9k/10/10/10/10/10/10/4C4K w - - 0 1",
            "e1f3"
        ),
        "White Chancellor e1-f3"
    );
}

#[test]
fn explicit_rights_and_authoritative_legend() {
    let mut p = Variant::Gothic.starting_position();
    let initial = board_state(&p);
    for right in [
        "White queenside: yes",
        "White kingside: yes",
        "Black queenside: yes",
        "Black kingside: yes",
        "K King: f1",
        "C Chancellor: e1",
    ] {
        assert!(initial.contains(right));
    }
    p.play_uci("e2e4").unwrap();
    assert!(board_state(&p).contains("En passant: e3"));
    assert!(board_state(&Variant::Grand.starting_position()).contains("Castling: none"));
    let t = Variant::TerachessII.starting_position();
    assert!(board_state(&t).contains("Initial king jump:\nWhite: available\nBlack: available"));
    let rules = chess_rules(Variant::TerachessII);
    for entry in [
        "K = King:",
        "S = Admiral:",
        "J = Centaur:",
        "I = Prince:",
        "T = Troll:",
        "Z = Giraffe: leaps (2,3)/(3,2)",
        "X = Archbishop:",
        "H = Chancellor:",
        "A = Amazon:",
    ] {
        assert!(rules.contains(entry), "{entry}");
    }
    assert!(!rules.contains("(1,4)"));
    assert!(chess_rules(Variant::Pemba).contains("(2,3)/(3,2)"));
    assert!(chess_rules(Variant::Gothic).contains("a-j: R N B Q C K A B N R"));
    assert!(rules.contains("Never infer piece identities"));
    let fen = "15k/16/16/16/16/16/16/16/16/16/16/16/16/16/7K8/16 w J - 0 1";
    let mut p =
        capablanca_chess_plus::Position::from_fen(Variant::TerachessII.rules(), fen).unwrap();
    assert!(board_state(&p).contains("White: available\nBlack: unavailable"));
    assert_eq!(
        semantic(Variant::TerachessII, fen, "h2h4"),
        "White King h2-h4 (initial king jump)"
    );
    p.play_uci("h2h4").unwrap();
    assert!(board_state(&p).contains("White: unavailable\nBlack: unavailable"));
}

#[test]
fn no_position_serialization_or_legal_move_list_in_any_variant_prompt() {
    let config = Config::load().unwrap();
    for variant in Variant::ALL {
        let state = Match::new(variant, Color::White);
        let built = PromptBuilder::new()
            .build(
                &config.context,
                &config.instruct,
                &config.system,
                &state.prompt_data(),
            )
            .unwrap();
        assert!(!built.text.contains("FEN"));
        assert!(!built.text.contains(&state.game().position().to_fen()));
        for mv in state.game().position().legal_moves() {
            assert!(
                !built.text.contains(&mv.to_uci()),
                "leaked {} for {variant:?}",
                mv.to_uci()
            );
        }
    }
}
