use capablanca_chess_plus::{
    CastleSide, Color, DrawReason, Game, GameOutcome, MoveKind, Piece, PieceKind, Position, Square,
    Variant,
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
        (
            Variant::Shako,
            "c8c/ernbqkbnre/pppppppppp/10/10/10/10/PPPPPPPPPP/ERNBQKBNRE/C8C w KQkq - 0 1",
        ),
        (
            Variant::Pemba,
            "cmvzwwzvmc/ernbqkbnre/pppppppppp/10/10/10/10/PPPPPPPPPP/ERNBQKBNRE/CMVZWWZVMC w KQkq - 0 1",
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
        (Variant::Shako, 58),
        (Variant::Pemba, 34),
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
    assert_eq!(Variant::Shako.starting_position().perft(2), 3_364);
    assert_eq!(Variant::Shako.starting_position().perft(3), 185_938);
    assert_eq!(Variant::Pemba.starting_position().perft(2), 1_156);
    // Independently cross-checked against Fairy-Stockfish 14 using the Pemba
    // Betza definition bundled with the frontend.
    assert_eq!(Variant::Pemba.starting_position().perft(3), 42_444);
}

#[test]
fn pemba_starting_material_contains_thirty_pieces_of_twelve_types() {
    let position = Variant::Pemba.starting_position();
    for color in Color::ALL {
        for (kind, count) in [
            (PieceKind::King, 1),
            (PieceKind::Queen, 1),
            (PieceKind::Bishop, 2),
            (PieceKind::Knight, 2),
            (PieceKind::Camel, 2),
            (PieceKind::Rook, 2),
            (PieceKind::Cannon, 2),
            (PieceKind::Elephant, 2),
            (PieceKind::Archer, 2),
            (PieceKind::Giraffe, 2),
            (PieceKind::Machine, 2),
            (PieceKind::Pawn, 10),
        ] {
            assert_eq!(
                position.board().count(color, kind),
                count,
                "{color:?} {kind:?}"
            );
        }
        assert_eq!(position.board().count(color, PieceKind::Archbishop), 0);
        assert_eq!(position.board().count(color, PieceKind::Chancellor), 0);
    }
}

#[test]
fn pemba_camel_and_giraffe_use_their_exact_leaper_offsets() {
    let camel = Position::from_fen(
        Variant::Pemba.rules(),
        "9k/10/5P4/10/7r2/4M5/10/10/10/K9 w - - 0 1",
    )
    .unwrap();
    let camel_moves: Vec<_> = uci_moves(&camel)
        .into_iter()
        .filter(|value| value.starts_with("e5"))
        .collect();
    assert_eq!(
        camel_moves,
        ["e5b4", "e5b6", "e5d2", "e5d8", "e5f2", "e5h4", "e5h6"].map(str::to_owned)
    );

    let giraffe = Position::from_fen(
        Variant::Pemba.rules(),
        "9k/10/10/10/10/4Z5/10/10/10/K9 w - - 0 1",
    )
    .unwrap();
    let giraffe_moves: Vec<_> = uci_moves(&giraffe)
        .into_iter()
        .filter(|value| value.starts_with("e5"))
        .collect();
    assert_eq!(
        giraffe_moves,
        [
            "e5b3", "e5b7", "e5c2", "e5c8", "e5g2", "e5g8", "e5h3", "e5h7"
        ]
        .map(str::to_owned)
    );
}

#[test]
fn pemba_machine_jumps_over_the_first_orthogonal_square() {
    let position = Position::from_fen(
        Variant::Pemba.rules(),
        "9k/10/4r5/4P5/4W5/10/10/10/10/K9 w - - 0 1",
    )
    .unwrap();
    let moves: Vec<_> = uci_moves(&position)
        .into_iter()
        .filter(|value| value.starts_with("e6"))
        .collect();
    assert_eq!(
        moves,
        ["e6c6", "e6d6", "e6e4", "e6e5", "e6e8", "e6f6", "e6g6"].map(str::to_owned)
    );
    assert!(moves.contains(&"e6e8".to_owned()));
    assert!(!moves.contains(&"e6e7".to_owned()));
}

#[test]
fn pemba_archer_slides_diagonally_but_captures_only_over_one_screen() {
    let position = Position::from_fen(
        Variant::Pemba.rules(),
        "9k/8n1/7r2/10/5P4/10/3V6/10/10/K9 w - - 0 1",
    )
    .unwrap();
    let moves: Vec<_> = uci_moves(&position)
        .into_iter()
        .filter(|value| value.starts_with("d4"))
        .collect();
    assert!(moves.contains(&"d4e5".to_owned()));
    assert!(moves.contains(&"d4h8".to_owned()));
    assert!(!moves.contains(&"d4f6".to_owned()));
    assert!(!moves.contains(&"d4g7".to_owned()));
    assert!(!moves.contains(&"d4i9".to_owned()));

    let one_screen = Position::from_fen(
        Variant::Pemba.rules(),
        "9k/10/7v2/10/5P4/10/3K6/10/10/10 w - - 0 1",
    )
    .unwrap();
    assert!(one_screen.is_in_check(Color::White));

    let no_screen = Position::from_fen(
        Variant::Pemba.rules(),
        "9k/10/7v2/10/10/10/3K6/10/10/10 w - - 0 1",
    )
    .unwrap();
    assert!(!no_screen.is_in_check(Color::White));

    let two_screens = Position::from_fen(
        Variant::Pemba.rules(),
        "9k/10/7v2/6p3/5P4/10/3K6/10/10/10 w - - 0 1",
    )
    .unwrap();
    assert!(!two_screens.is_in_check(Color::White));
}

#[test]
fn pemba_new_leapers_participate_in_check_detection() {
    for fen in [
        "9k/10/10/7m2/4K5/10/10/10/10/10 w - - 0 1",
        "9k/10/7z2/10/4K5/10/10/10/10/10 w - - 0 1",
        "9k/10/10/10/4K1w3/10/10/10/10/10 w - - 0 1",
    ] {
        let position = Position::from_fen(Variant::Pemba.rules(), fen).unwrap();
        assert!(position.is_in_check(Color::White), "{fen}");
    }
}

#[test]
fn pemba_castling_uses_the_second_rank_rooks() {
    let mut position = Position::from_fen(
        Variant::Pemba.rules(),
        "10/5k4/10/10/10/10/10/10/1R3K2R1/10 w KQ - 0 1",
    )
    .unwrap();
    let moves = uci_moves(&position);
    assert!(moves.contains(&"f2d2".to_owned()));
    assert!(moves.contains(&"f2h2".to_owned()));

    position.play_uci("f2h2").unwrap();
    assert_eq!(
        position.board().piece_at("h2".parse().unwrap()),
        Some(Piece::new(Color::White, PieceKind::King))
    );
    assert_eq!(
        position.board().piece_at("g2".parse().unwrap()),
        Some(Piece::new(Color::White, PieceKind::Rook))
    );
}

#[test]
fn pemba_promotion_is_mandatory_and_offers_all_ten_non_royal_pieces() {
    let position = Position::from_fen(
        Variant::Pemba.rules(),
        "9k/P9/10/10/10/10/10/10/10/9K w - - 0 1",
    )
    .unwrap();
    let promotions: Vec<_> = uci_moves(&position)
        .into_iter()
        .filter(|value| value.starts_with("a9a10"))
        .collect();
    assert_eq!(
        promotions,
        [
            "a9a10b", "a9a10c", "a9a10e", "a9a10m", "a9a10n", "a9a10q", "a9a10r", "a9a10v",
            "a9a10w", "a9a10z",
        ]
        .map(str::to_owned)
    );
    assert!(!promotions.contains(&"a9a10".to_owned()));
}

#[test]
fn shako_starting_material_and_notation_are_unambiguous() {
    let position = Variant::Shako.starting_position();
    for color in Color::ALL {
        assert_eq!(position.board().count(color, PieceKind::King), 1);
        assert_eq!(position.board().count(color, PieceKind::Queen), 1);
        assert_eq!(position.board().count(color, PieceKind::Rook), 2);
        assert_eq!(position.board().count(color, PieceKind::Bishop), 2);
        assert_eq!(position.board().count(color, PieceKind::Knight), 2);
        assert_eq!(position.board().count(color, PieceKind::Pawn), 10);
        assert_eq!(position.board().count(color, PieceKind::Cannon), 2);
        assert_eq!(position.board().count(color, PieceKind::Elephant), 2);
        assert_eq!(position.board().count(color, PieceKind::Chancellor), 0);
    }

    let capablanca = Position::from_fen(
        Variant::Capablanca.rules(),
        "4k5/10/10/10/10/10/10/4K1C3 w - - 0 1",
    )
    .unwrap();
    assert_eq!(
        capablanca.board().piece_at("g1".parse().unwrap()),
        Some(Piece::new(Color::White, PieceKind::Chancellor))
    );
}

#[test]
fn shako_cannon_slides_without_a_screen_and_captures_only_over_one_screen() {
    let mut position = Position::from_fen(
        Variant::Shako.rules(),
        "9k/3r6/10/10/3P6/10/3C1p2n1/10/10/K9 w - - 0 1",
    )
    .unwrap();
    let from = "d4".parse::<Square>().unwrap();
    let moves: Vec<_> = uci_moves(&position)
        .into_iter()
        .filter(|value| value.starts_with("d4"))
        .collect();
    assert_eq!(
        moves,
        [
            "d4a4", "d4b4", "d4c4", "d4d1", "d4d2", "d4d3", "d4d5", "d4d9", "d4e4", "d4i4",
        ]
        .map(str::to_owned)
    );
    assert!(!moves.contains(&"d4f4".to_owned()));
    assert!(!moves.contains(&"d4g4".to_owned()));
    assert!(!moves.contains(&"d4h4".to_owned()));

    position.play_uci("d4d9").unwrap();
    assert_eq!(
        position.board().piece_at("d9".parse().unwrap()),
        Some(Piece::new(Color::White, PieceKind::Cannon))
    );
    assert_eq!(position.board().piece_at(from), None);
}

#[test]
fn shako_cannon_checks_through_exactly_one_piece() {
    let one_screen = Position::from_fen(
        Variant::Shako.rules(),
        "9k/3c6/10/10/10/3P6/10/10/10/3K6 w - - 0 1",
    )
    .unwrap();
    assert!(one_screen.is_in_check(Color::White));

    let no_screen = Position::from_fen(
        Variant::Shako.rules(),
        "9k/3c6/10/10/10/10/10/10/10/3K6 w - - 0 1",
    )
    .unwrap();
    assert!(!no_screen.is_in_check(Color::White));

    let two_screens = Position::from_fen(
        Variant::Shako.rules(),
        "9k/3c6/10/3p6/10/3B6/10/10/10/3K6 w - - 0 1",
    )
    .unwrap();
    assert!(!two_screens.is_in_check(Color::White));
    assert!(
        uci_moves(&two_screens)
            .into_iter()
            .all(|value| !value.starts_with("d5"))
    );
}

#[test]
fn shako_elephant_leaps_one_or_two_diagonal_squares() {
    let position = Position::from_fen(
        Variant::Shako.rules(),
        "9k/10/10/6r3/5P4/4E5/10/10/10/K9 w - - 0 1",
    )
    .unwrap();
    let moves: Vec<_> = uci_moves(&position)
        .into_iter()
        .filter(|value| value.starts_with("e5"))
        .collect();
    assert_eq!(
        moves,
        ["e5c3", "e5c7", "e5d4", "e5d6", "e5f4", "e5g3", "e5g7"].map(str::to_owned)
    );
    assert!(moves.contains(&"e5g7".to_owned()));
    assert!(!moves.contains(&"e5f6".to_owned()));
    assert!(!moves.contains(&"e5h8".to_owned()));

    let check = Position::from_fen(
        Variant::Shako.rules(),
        "9k/10/10/10/10/4K5/3p6/2e7/10/10 w - - 0 1",
    )
    .unwrap();
    assert!(check.is_in_check(Color::White));
}

#[test]
fn shako_castling_uses_the_second_rank_rooks() {
    let mut position = Position::from_fen(
        Variant::Shako.rules(),
        "10/5k4/10/10/10/10/10/10/1R3K2R1/10 w KQ - 0 1",
    )
    .unwrap();
    let moves = uci_moves(&position);
    assert!(moves.contains(&"f2d2".to_owned()));
    assert!(moves.contains(&"f2h2".to_owned()));

    let castle = position.parse_uci_move("f2d2").unwrap();
    assert_eq!(castle.kind, MoveKind::Castle(CastleSide::QueenSide));
    position.play(castle).unwrap();
    assert_eq!(
        position.board().piece_at("d2".parse().unwrap()),
        Some(Piece::new(Color::White, PieceKind::King))
    );
    assert_eq!(
        position.board().piece_at("e2".parse().unwrap()),
        Some(Piece::new(Color::White, PieceKind::Rook))
    );
}

#[test]
fn shako_pawns_double_from_third_rank_and_support_en_passant() {
    let mut position = Position::from_fen(
        Variant::Shako.rules(),
        "9k/10/10/10/10/1p8/10/P9/10/9K w - - 0 1",
    )
    .unwrap();
    position.play_uci("a3a5").unwrap();
    assert_eq!(position.en_passant(), Some("a4".parse().unwrap()));
    let capture = position.parse_uci_move("b5a4").unwrap();
    assert_eq!(capture.kind, MoveKind::EnPassant);
    position.play(capture).unwrap();
    assert_eq!(position.board().piece_at("a5".parse().unwrap()), None);
    assert_eq!(
        position.board().piece_at("a4".parse().unwrap()),
        Some(Piece::new(Color::Black, PieceKind::Pawn))
    );
}

#[test]
fn shako_promotion_is_mandatory_and_offers_exactly_the_six_shako_pieces() {
    let mut position = Position::from_fen(
        Variant::Shako.rules(),
        "9k/P9/10/10/10/10/10/10/10/9K w - - 0 1",
    )
    .unwrap();
    let promotions: Vec<_> = uci_moves(&position)
        .into_iter()
        .filter(|value| value.starts_with("a9a10"))
        .collect();
    assert_eq!(
        promotions,
        ["a9a10b", "a9a10c", "a9a10e", "a9a10n", "a9a10q", "a9a10r"].map(str::to_owned)
    );
    assert!(!promotions.contains(&"a9a10".to_owned()));
    assert!(!promotions.contains(&"a9a10a".to_owned()));

    position.play_uci("a9a10c").unwrap();
    assert_eq!(
        position.board().piece_at("a10".parse().unwrap()),
        Some(Piece::new(Color::White, PieceKind::Cannon))
    );
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
