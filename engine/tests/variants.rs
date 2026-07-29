use capablanca_chess_plus::{
    CastleSide, Color, DrawReason, Engine, Game, GameOutcome, MoveKind, Piece, PieceKind, Position,
    SearchLimits, Square, Variant,
};

fn uci_moves(position: &Position) -> Vec<String> {
    let mut moves: Vec<_> = position
        .legal_moves()
        .into_iter()
        .map(|chess_move| chess_move.to_uci())
        .collect();
    moves.sort();
    moves
}

#[test]
fn built_in_starting_arrays_and_fen_are_stable() {
    let expected = [
        (
            Variant::Capablanca,
            "rnabqkbcnr/pppppppppp/10/10/10/10/PPPPPPPPPP/RNABQKBCNR w KQkq - 0 1",
        ),
        (
            Variant::Gothic,
            "rnbqckabnr/pppppppppp/10/10/10/10/PPPPPPPPPP/RNBQCKABNR w KQkq - 0 1",
        ),
        (
            Variant::Embassy,
            "rnbqkcabnr/pppppppppp/10/10/10/10/PPPPPPPPPP/RNBQKCABNR w KQkq - 0 1",
        ),
        (
            Variant::Schoolbook,
            "rqnbakbncr/pppppppppp/10/10/10/10/PPPPPPPPPP/RQNBAKBNCR w KQkq - 0 1",
        ),
        (
            Variant::Bird,
            "rnbcqkabnr/pppppppppp/10/10/10/10/PPPPPPPPPP/RNBCQKABNR w KQkq - 0 1",
        ),
        (
            Variant::Carrera,
            "rcnbkqbnar/pppppppppp/10/10/10/10/PPPPPPPPPP/RCNBKQBNAR w - - 0 1",
        ),
        (
            Variant::Grand,
            "r8r/1nbqkcabn1/pppppppppp/10/10/10/10/PPPPPPPPPP/1NBQKCABN1/R8R w - - 0 1",
        ),
    ];

    for (variant, fen) in expected {
        let position = variant.starting_position();
        assert_eq!(position.to_fen(), fen, "{variant:?}");
        assert_eq!(
            Position::from_fen(variant.rules(), fen).unwrap(),
            position,
            "{variant:?}"
        );
    }
}

#[test]
fn opening_move_counts_cover_compound_pieces_and_grand_layout() {
    let expected = [
        (Variant::Capablanca, 28),
        (Variant::Gothic, 28),
        (Variant::Embassy, 28),
        (Variant::Schoolbook, 28),
        (Variant::Bird, 28),
        (Variant::Carrera, 28),
        (Variant::Grand, 65),
    ];

    for (variant, count) in expected {
        assert_eq!(
            variant.starting_position().legal_moves().len(),
            count,
            "{variant:?}"
        );
    }

    assert_eq!(Variant::Capablanca.starting_position().perft(2), 784);
    assert_eq!(Variant::Grand.starting_position().perft(2), 4_225);
}

#[test]
fn capablanca_and_gothic_castle_from_f_to_c_or_i() {
    for variant in [Variant::Capablanca, Variant::Gothic, Variant::Schoolbook] {
        let mut position =
            Position::from_fen(variant.rules(), "5k4/10/10/10/10/10/10/R4K3R w KQ - 0 1").unwrap();
        let moves = uci_moves(&position);
        assert!(moves.contains(&"f1c1".to_owned()), "{variant:?}");
        assert!(moves.contains(&"f1i1".to_owned()), "{variant:?}");

        let castle = position.parse_uci_move("f1c1").unwrap();
        assert_eq!(castle.kind, MoveKind::Castle(CastleSide::QueenSide));
        position.play(castle).unwrap();
        assert_eq!(
            position.board().piece_at("c1".parse().unwrap()),
            Some(Piece::new(Color::White, PieceKind::King))
        );
        assert_eq!(
            position.board().piece_at("d1".parse().unwrap()),
            Some(Piece::new(Color::White, PieceKind::Rook))
        );
        assert!(
            !position
                .castling_rights()
                .has(Color::White, CastleSide::KingSide)
        );
        assert!(
            !position
                .castling_rights()
                .has(Color::White, CastleSide::QueenSide)
        );
    }
}

#[test]
fn embassy_castling_uses_its_e_file_king() {
    let mut position = Position::from_fen(
        Variant::Embassy.rules(),
        "4k5/10/10/10/10/10/10/R3K4R w KQ - 0 1",
    )
    .unwrap();
    let moves = uci_moves(&position);
    assert!(moves.contains(&"e1b1".to_owned()));
    assert!(moves.contains(&"e1h1".to_owned()));

    position.play_uci("e1b1").unwrap();
    assert_eq!(
        position.board().piece_at("b1".parse().unwrap()),
        Some(Piece::new(Color::White, PieceKind::King))
    );
    assert_eq!(
        position.board().piece_at("c1".parse().unwrap()),
        Some(Piece::new(Color::White, PieceKind::Rook))
    );
}

#[test]
fn black_castling_routes_are_mirrored() {
    let mut position = Position::from_fen(
        Variant::Capablanca.rules(),
        "r4k3r/10/10/10/10/10/10/5K4 b kq - 0 1",
    )
    .unwrap();
    let moves = uci_moves(&position);
    assert!(moves.contains(&"f8c8".to_owned()));
    assert!(moves.contains(&"f8i8".to_owned()));

    position.play_uci("f8i8").unwrap();
    assert_eq!(
        position.board().piece_at("i8".parse().unwrap()),
        Some(Piece::new(Color::Black, PieceKind::King))
    );
    assert_eq!(
        position.board().piece_at("h8".parse().unwrap()),
        Some(Piece::new(Color::Black, PieceKind::Rook))
    );
}

#[test]
fn castling_cannot_cross_attack_or_occupied_squares() {
    let attacked = Position::from_fen(
        Variant::Capablanca.rules(),
        "k3r5/10/10/10/10/10/10/R4K3R w KQ - 0 1",
    )
    .unwrap();
    let moves = uci_moves(&attacked);
    assert!(!moves.contains(&"f1c1".to_owned()));
    assert!(moves.contains(&"f1i1".to_owned()));

    let occupied = Position::from_fen(
        Variant::Capablanca.rules(),
        "5k4/10/10/10/10/10/10/R2B1K3R w KQ - 0 1",
    )
    .unwrap();
    let moves = uci_moves(&occupied);
    assert!(!moves.contains(&"f1c1".to_owned()));
    assert!(moves.contains(&"f1i1".to_owned()));
}

#[test]
fn moving_or_capturing_a_route_rook_revokes_that_right() {
    let mut moved = Position::from_fen(
        Variant::Capablanca.rules(),
        "5k4/10/10/10/10/10/10/R4K3R w KQ - 0 1",
    )
    .unwrap();
    moved.play_uci("a1a2").unwrap();
    assert!(
        !moved
            .castling_rights()
            .has(Color::White, CastleSide::QueenSide)
    );
    assert!(
        moved
            .castling_rights()
            .has(Color::White, CastleSide::KingSide)
    );

    let mut captured = Position::from_fen(
        Variant::Capablanca.rules(),
        "r4k3r/10/10/10/10/10/10/R4K3R w KQkq - 0 1",
    )
    .unwrap();
    captured.play_uci("a1a8").unwrap();
    assert!(
        !captured
            .castling_rights()
            .has(Color::White, CastleSide::QueenSide)
    );
    assert!(
        !captured
            .castling_rights()
            .has(Color::Black, CastleSide::QueenSide)
    );
}

#[test]
fn grand_and_historical_carrera_do_not_castle() {
    for variant in [Variant::Grand, Variant::Carrera] {
        let position = variant.starting_position();
        assert!(!position.rules().castling().any());
        assert!(
            position
                .legal_moves()
                .iter()
                .all(|chess_move| !matches!(chess_move.kind, MoveKind::Castle(_)))
        );
    }
}

#[test]
fn en_passant_is_filtered_when_it_exposes_the_king() {
    let position = Position::from_fen(
        Variant::Capablanca.rules(),
        "9k/10/10/r2pPK4/10/10/10/10 w - d6 0 1",
    )
    .unwrap();
    let moves = uci_moves(&position);
    assert!(!moves.contains(&"e5d6".to_owned()));
    assert!(moves.contains(&"e5e6".to_owned()), "{moves:?}");
}

#[test]
fn compound_pieces_attack_as_sliders_and_knights() {
    let archbishop_check = Position::from_fen(
        Variant::Capablanca.rules(),
        "9k/10/10/10/10/4a5/10/5K4 w - - 0 1",
    )
    .unwrap();
    assert!(archbishop_check.is_in_check(Color::White));

    let chancellor_check = Position::from_fen(
        Variant::Capablanca.rules(),
        "9k/10/10/10/10/10/3c6/5K4 w - - 0 1",
    )
    .unwrap();
    assert!(chancellor_check.is_in_check(Color::White));
}

#[test]
fn capablanca_promotion_is_mandatory_and_has_six_choices() {
    let position = Position::from_fen(
        Variant::Capablanca.rules(),
        "9k/P9/10/10/10/10/10/9K w - - 0 1",
    )
    .unwrap();
    let promotions: Vec<_> = position
        .legal_moves()
        .into_iter()
        .filter(|chess_move| {
            chess_move.from == "a7".parse::<Square>().unwrap()
                && chess_move.to == "a8".parse::<Square>().unwrap()
        })
        .collect();
    assert_eq!(promotions.len(), 6);
    assert!(
        promotions
            .iter()
            .all(|chess_move| chess_move.promotion.is_some())
    );
}

fn grand_promotion_fen(pawn_rank: u8, include_queen: bool) -> String {
    let rank_two = if include_queen {
        "1NBQKCABN1"
    } else {
        "1NB1KCABN1"
    };
    let mut ranks = vec!["10".to_owned(); 10];
    ranks[0] = "9k".to_owned();
    ranks[usize::from(10 - pawn_rank)] = "P9".to_owned();
    ranks[8] = rank_two.to_owned();
    ranks[9] = "R8R".to_owned();
    format!("{} w - - 0 1", ranks.join("/"))
}

#[test]
fn grand_promotion_is_optional_then_mandatory_and_inventory_limited() {
    let optional =
        Position::from_fen(Variant::Grand.rules(), &grand_promotion_fen(7, false)).unwrap();
    let moves = uci_moves(&optional);
    assert!(moves.contains(&"a7a8".to_owned()));
    assert!(moves.contains(&"a7a8q".to_owned()));
    assert!(!moves.iter().any(|value| value == "a7a8c"));

    let mandatory =
        Position::from_fen(Variant::Grand.rules(), &grand_promotion_fen(9, false)).unwrap();
    let moves = uci_moves(&mandatory);
    assert!(!moves.contains(&"a9a10".to_owned()));
    assert!(moves.contains(&"a9a10q".to_owned()));
    assert_eq!(
        moves
            .iter()
            .filter(|value| value.starts_with("a9a10"))
            .count(),
        1
    );

    let no_captured_material =
        Position::from_fen(Variant::Grand.rules(), &grand_promotion_fen(9, true)).unwrap();
    assert!(
        !uci_moves(&no_captured_material)
            .iter()
            .any(|value| value.starts_with("a9a10"))
    );
}

#[test]
fn repetition_history_and_halfmove_draws_are_reported() {
    let mut game = Game::new(Variant::Capablanca.starting_position());
    for _ in 0..2 {
        game.play_uci("b1c3").unwrap();
        game.play_uci("b8c6").unwrap();
        game.play_uci("c3b1").unwrap();
        game.play_uci("c6b8").unwrap();
    }
    assert_eq!(
        game.outcome(),
        GameOutcome::Draw(DrawReason::ThreefoldRepetition)
    );

    let fifty_move = Position::from_fen(
        Variant::Capablanca.rules(),
        "9k/10/10/10/10/10/10/K8R w - - 100 51",
    )
    .unwrap();
    assert_eq!(
        Game::new(fifty_move).outcome(),
        GameOutcome::Draw(DrawReason::FiftyMoveRule)
    );
}

#[test]
fn mate_stalemate_and_invalid_fen_rights_are_recognized() {
    let checkmate = Position::from_fen(
        Variant::Capablanca.rules(),
        "k9/1Q8/2K7/10/10/10/10/10 b - - 0 1",
    )
    .unwrap();
    assert!(checkmate.is_checkmate());
    assert_eq!(
        Game::new(checkmate).outcome(),
        GameOutcome::Win {
            winner: Color::White
        }
    );

    let stalemate = Position::from_fen(
        Variant::Capablanca.rules(),
        "k9/2Q7/2K7/10/10/10/10/10 b - - 0 1",
    )
    .unwrap();
    assert!(stalemate.is_stalemate());
    assert_eq!(
        Game::new(stalemate).outcome(),
        GameOutcome::Draw(DrawReason::Stalemate)
    );

    assert!(
        Position::from_fen(
            Variant::Capablanca.rules(),
            "5k4/10/10/10/10/10/10/5K4 w KQ - 0 1",
        )
        .is_err()
    );
}

#[test]
fn search_returns_a_legal_principal_variation() {
    let position = Variant::Gothic.starting_position();
    let legal = position.legal_moves();
    let result = Engine::new()
        .search(&position, SearchLimits::depth(2))
        .unwrap();

    assert!(legal.contains(&result.best_move));
    assert_eq!(result.depth, 2);
    assert!(result.nodes > 0);
    assert_eq!(result.principal_variation.first(), Some(&result.best_move));
}
