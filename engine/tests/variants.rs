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

fn terachess_position(
    pieces: &[(char, &str)],
    side: Color,
    rights: &str,
    en_passant: &str,
) -> Position {
    let mut squares = [[None; 16]; 16];
    let has_white_king = pieces.iter().any(|(piece, _)| *piece == 'K');
    let has_black_king = pieces.iter().any(|(piece, _)| *piece == 'k');
    let defaults = [('K', "a1"), ('k', "p16")];
    for &(piece, coordinate) in defaults
        .iter()
        .filter(|(piece, _)| {
            (*piece == 'K' && !has_white_king) || (*piece == 'k' && !has_black_king)
        })
        .chain(pieces.iter())
    {
        let square: Square = coordinate.parse().unwrap();
        assert!(squares[usize::from(square.rank())][usize::from(square.file())].is_none());
        squares[usize::from(square.rank())][usize::from(square.file())] = Some(piece);
    }

    let mut ranks = Vec::with_capacity(16);
    for rank in (0..16).rev() {
        let mut encoded = String::new();
        let mut empty = 0;
        for square in &squares[rank] {
            if let Some(piece) = square {
                if empty != 0 {
                    encoded.push_str(&empty.to_string());
                    empty = 0;
                }
                encoded.push(*piece);
            } else {
                empty += 1;
            }
        }
        if empty != 0 {
            encoded.push_str(&empty.to_string());
        }
        ranks.push(encoded);
    }
    let side = match side {
        Color::White => 'w',
        Color::Black => 'b',
    };
    Position::from_fen(
        Variant::TerachessII.rules(),
        &format!("{} {side} {rights} {en_passant} 0 1", ranks.join("/")),
    )
    .unwrap()
}

fn moves_from(position: &Position, from: &str) -> Vec<String> {
    let mut moves: Vec<_> = uci_moves(position)
        .into_iter()
        .filter(|chess_move| chess_move.starts_with(from))
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
        (
            Variant::TerachessII,
            "sjyhxfdoodfxhyjs/cmztuvlkalvutzmc/ernbwigqqgiwbnre/pppppppppppppppp/16/16/16/16/16/16/16/16/PPPPPPPPPPPPPPPP/ERNBWIGQQGIWBNRE/CMZTUVLKALVUTZMC/SJYHXFDOODFXHYJS w Jj - 0 1",
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
        (Variant::TerachessII, 54),
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
    assert_eq!(Variant::TerachessII.starting_position().perft(2), 2_916);
    assert_eq!(Variant::TerachessII.starting_position().perft(3), 175_508);
}

#[test]
fn terachess_starting_material_has_sixty_four_pieces_and_all_twenty_six_types() {
    let position = Variant::TerachessII.starting_position();
    let paired = [
        PieceKind::Knight,
        PieceKind::Bishop,
        PieceKind::Rook,
        PieceKind::Queen,
        PieceKind::Archbishop,
        PieceKind::Chancellor,
        PieceKind::Cannon,
        PieceKind::Elephant,
        PieceKind::Camel,
        PieceKind::Giraffe,
        PieceKind::Archer,
        PieceKind::Machine,
        PieceKind::Lion,
        PieceKind::Buffalo,
        PieceKind::Centaur,
        PieceKind::Admiral,
        PieceKind::Missionary,
        PieceKind::Eagle,
        PieceKind::Rhinoceros,
        PieceKind::Prince,
        PieceKind::Sorceress,
        PieceKind::Duchess,
        PieceKind::Troll,
    ];
    for color in Color::ALL {
        assert_eq!(
            position
                .board()
                .pieces()
                .filter(|(_, p)| p.color == color)
                .count(),
            64
        );
        assert_eq!(position.board().count(color, PieceKind::Pawn), 16);
        assert_eq!(position.board().count(color, PieceKind::King), 1);
        assert_eq!(position.board().count(color, PieceKind::Amazon), 1);
        for kind in paired {
            assert_eq!(position.board().count(color, kind), 2, "{color:?} {kind:?}");
        }
    }
    assert!(!position.rules().castling().any());
    assert!(
        position
            .legal_moves()
            .iter()
            .all(|chess_move| !matches!(chess_move.kind, MoveKind::Castle(_)))
    );
}

#[test]
fn terachess_compound_and_multi_leaper_moves_match_the_rules() {
    let cases = [
        ('L', PieceKind::Lion, 24),
        ('F', PieceKind::Buffalo, 24),
        ('J', PieceKind::Centaur, 16),
    ];
    for (letter, kind, expected_count) in cases {
        let position = terachess_position(&[(letter, "h8")], Color::White, "-", "-");
        let moves = moves_from(&position, "h8");
        assert_eq!(moves.len(), expected_count, "{kind:?}: {moves:?}");
    }

    let amazon = terachess_position(&[('A', "h8")], Color::White, "-", "-");
    let moves = moves_from(&amazon, "h8");
    assert!(moves.contains(&"h8h15".to_owned()));
    assert!(moves.contains(&"h8i10".to_owned()));
    assert!(!moves.contains(&"h8j11".to_owned()));

    let admiral = terachess_position(&[('S', "h8")], Color::White, "-", "-");
    let moves = moves_from(&admiral, "h8");
    assert!(moves.contains(&"h8h15".to_owned()));
    assert!(moves.contains(&"h8i9".to_owned()));
    assert!(!moves.contains(&"h8j10".to_owned()));

    let missionary = terachess_position(&[('Y', "h8")], Color::White, "-", "-");
    let moves = moves_from(&missionary, "h8");
    assert!(moves.contains(&"h8o15".to_owned()));
    assert!(moves.contains(&"h8h9".to_owned()));
    assert!(!moves.contains(&"h8h10".to_owned()));
}

#[test]
fn terachess_duchess_jumps_up_to_three_squares_but_not_four() {
    let position = terachess_position(
        &[('D', "h8"), ('P', "h9"), ('r', "h11")],
        Color::White,
        "-",
        "-",
    );
    let moves = moves_from(&position, "h8");
    assert!(!moves.contains(&"h8h9".to_owned()));
    assert!(moves.contains(&"h8h10".to_owned()));
    assert!(moves.contains(&"h8h11".to_owned()));
    assert!(!moves.contains(&"h8h12".to_owned()));
}

#[test]
fn terachess_eagle_and_rhinoceros_follow_their_bent_unobstructed_paths() {
    let eagle = terachess_position(&[('G', "h8")], Color::White, "-", "-");
    let moves = moves_from(&eagle, "h8");
    for expected in ["h8i9", "h8j9", "h8p9", "h8i10", "h8i16"] {
        assert!(
            moves.contains(&expected.to_owned()),
            "missing {expected}: {moves:?}"
        );
    }
    assert!(!moves.contains(&"h8j10".to_owned()));

    let blocked_eagle = terachess_position(&[('G', "h8"), ('P', "i9")], Color::White, "-", "-");
    let moves = moves_from(&blocked_eagle, "h8");
    assert!(!moves.contains(&"h8i9".to_owned()));
    assert!(!moves.contains(&"h8j9".to_owned()));
    assert!(!moves.contains(&"h8i10".to_owned()));

    let rhinoceros = terachess_position(&[('U', "h8")], Color::White, "-", "-");
    let moves = moves_from(&rhinoceros, "h8");
    for expected in ["h8i8", "h8j9", "h8k10", "h8j7", "h8k6"] {
        assert!(
            moves.contains(&expected.to_owned()),
            "missing {expected}: {moves:?}"
        );
    }
    assert!(!moves.contains(&"h8i9".to_owned()));

    let eagle_check = terachess_position(&[('K', "f8"), ('g', "e6")], Color::White, "-", "-");
    assert!(eagle_check.is_in_check(Color::White));
    let blocked_eagle_check = terachess_position(
        &[('K', "f8"), ('g', "e6"), ('P', "f7")],
        Color::White,
        "-",
        "-",
    );
    assert!(!blocked_eagle_check.is_in_check(Color::White));

    let rhinoceros_check = terachess_position(&[('K', "h8"), ('u', "e6")], Color::White, "-", "-");
    assert!(rhinoceros_check.is_in_check(Color::White));
    let blocked_rhinoceros_check = terachess_position(
        &[('K', "h8"), ('u', "e6"), ('P', "f6")],
        Color::White,
        "-",
        "-",
    );
    assert!(!blocked_rhinoceros_check.is_in_check(Color::White));
}

#[test]
fn terachess_sorceress_moves_quietly_like_a_queen_but_captures_over_one_screen() {
    let position = terachess_position(
        &[
            ('O', "d4"),
            ('P', "d6"),
            ('r', "d9"),
            ('P', "f6"),
            ('b', "h8"),
        ],
        Color::White,
        "-",
        "-",
    );
    let moves = moves_from(&position, "d4");
    assert!(moves.contains(&"d4d5".to_owned()));
    assert!(!moves.contains(&"d4d6".to_owned()));
    assert!(!moves.contains(&"d4d7".to_owned()));
    assert!(moves.contains(&"d4d9".to_owned()));
    assert!(moves.contains(&"d4h8".to_owned()));

    let one_screen = terachess_position(
        &[('K', "d4"), ('o', "d9"), ('P', "d6")],
        Color::White,
        "-",
        "-",
    );
    assert!(one_screen.is_in_check(Color::White));
    let two_screens = terachess_position(
        &[('K', "d4"), ('o', "d9"), ('P', "d6"), ('P', "d7")],
        Color::White,
        "-",
        "-",
    );
    assert!(!two_screens.is_in_check(Color::White));
}

#[test]
fn terachess_troll_combines_three_square_jumps_with_pawn_moves() {
    let position = terachess_position(
        &[
            ('T', "h8"),
            ('P', "h9"),
            ('P', "h10"),
            ('r', "h11"),
            ('b', "g9"),
            ('P', "i9"),
        ],
        Color::White,
        "-",
        "-",
    );
    let moves = moves_from(&position, "h8");
    assert!(moves.contains(&"h8h11".to_owned()));
    assert!(!moves.contains(&"h8h9".to_owned()));
    assert!(moves.contains(&"h8g9".to_owned()));
    assert!(!moves.contains(&"h8i9".to_owned()));
    assert!(moves.contains(&"h8e5".to_owned()));
    assert!(!moves.contains(&"h8f6".to_owned()));
}

#[test]
fn terachess_initial_king_jump_obeys_every_safety_condition_and_is_spent_once() {
    let position = terachess_position(&[('K', "h2"), ('k', "p16")], Color::White, "J", "-");
    let moves = moves_from(&position, "h2");
    let jumps: Vec<_> = moves
        .iter()
        .filter(|chess_move| {
            let to: Square = chess_move[2..].parse().unwrap();
            "h2".parse::<Square>()
                .unwrap()
                .file()
                .abs_diff(to.file())
                .max("h2".parse::<Square>().unwrap().rank().abs_diff(to.rank()))
                == 2
        })
        .cloned()
        .collect();
    assert_eq!(
        jumps,
        [
            "h2f1", "h2f2", "h2f3", "h2f4", "h2g4", "h2h4", "h2i4", "h2j1", "h2j2", "h2j3", "h2j4",
        ]
        .map(str::to_owned)
    );

    let occupied_destination = terachess_position(
        &[('K', "h2"), ('k', "p16"), ('r', "h4")],
        Color::White,
        "J",
        "-",
    );
    assert!(!moves_from(&occupied_destination, "h2").contains(&"h2h4".to_owned()));

    let occupied_intermediate = terachess_position(
        &[('K', "h2"), ('k', "p16"), ('P', "h3")],
        Color::White,
        "J",
        "-",
    );
    assert!(moves_from(&occupied_intermediate, "h2").contains(&"h2h4".to_owned()));

    let straight_intermediate_attacked = terachess_position(
        &[('K', "h2"), ('k', "p16"), ('r', "a3")],
        Color::White,
        "J",
        "-",
    );
    let moves = moves_from(&straight_intermediate_attacked, "h2");
    assert!(!moves.contains(&"h2h4".to_owned()));
    assert!(!moves.contains(&"h2j4".to_owned()));

    let one_knight_intermediate_safe = terachess_position(
        &[('K', "h2"), ('k', "p16"), ('n', "g3")],
        Color::White,
        "J",
        "-",
    );
    assert!(moves_from(&one_knight_intermediate_safe, "h2").contains(&"h2j3".to_owned()));
    let both_knight_intermediates_attacked = terachess_position(
        &[('K', "h2"), ('k', "p16"), ('n', "g3"), ('n', "g2")],
        Color::White,
        "J",
        "-",
    );
    assert!(!moves_from(&both_knight_intermediates_attacked, "h2").contains(&"h2j3".to_owned()));

    let destination_attacked = terachess_position(
        &[('K', "h2"), ('k', "p16"), ('r', "j16")],
        Color::White,
        "J",
        "-",
    );
    assert!(!moves_from(&destination_attacked, "h2").contains(&"h2j3".to_owned()));

    let in_check = terachess_position(
        &[('K', "h2"), ('k', "p16"), ('r', "h16")],
        Color::White,
        "J",
        "-",
    );
    assert!(in_check.is_in_check(Color::White));
    assert!(moves_from(&in_check, "h2").iter().all(|chess_move| {
        let to: Square = chess_move[2..].parse().unwrap();
        "h2".parse::<Square>()
            .unwrap()
            .file()
            .abs_diff(to.file())
            .max("h2".parse::<Square>().unwrap().rank().abs_diff(to.rank()))
            < 2
    }));

    let mut spent = position;
    spent.play_uci("h2h3").unwrap();
    assert!(!spent.king_jump_available(Color::White));
    assert!(!spent.king_jump_available(Color::Black));
    assert!(spent.to_fen().contains(" b - "));
    spent.play_uci("p16p15").unwrap();
    assert!(!moves_from(&spent, "h3").contains(&"h3h5".to_owned()));

    assert!(
        Position::from_fen(
            Variant::TerachessII.rules(),
            "15k/16/16/16/16/16/16/16/16/16/16/16/16/7K8/16/16 w J - 0 1",
        )
        .is_err()
    );
}

#[test]
fn terachess_rapid_pawns_and_princes_double_anywhere_with_exact_en_passant_rules() {
    let position = terachess_position(&[('P', "d8"), ('I', "f8")], Color::White, "-", "-");
    assert!(moves_from(&position, "d8").contains(&"d8d10".to_owned()));
    assert!(moves_from(&position, "f8").contains(&"f8f10".to_owned()));

    let mut prince_double =
        terachess_position(&[('I', "d8"), ('p', "e10")], Color::White, "-", "-");
    prince_double.play_uci("d8d10").unwrap();
    assert_eq!(prince_double.en_passant(), Some("d9".parse().unwrap()));
    let capture = prince_double.parse_uci_move("e10d9").unwrap();
    assert_eq!(capture.kind, MoveKind::EnPassant);
    prince_double.play(capture).unwrap();
    assert_eq!(prince_double.board().piece_at("d10".parse().unwrap()), None);

    let mut pawn_double = terachess_position(&[('P', "d8"), ('i', "e10")], Color::White, "-", "-");
    pawn_double.play_uci("d8d10").unwrap();
    let prince_move = pawn_double.parse_uci_move("e10d9").unwrap();
    assert_eq!(prince_move.kind, MoveKind::Normal);
    pawn_double.play(prince_move).unwrap();
    assert_eq!(
        pawn_double.board().piece_at("d10".parse().unwrap()),
        Some(Piece::new(Color::White, PieceKind::Pawn))
    );

    for (mover, double_move, promoted_kind) in [
        ('P', "d14d16q", PieceKind::Queen),
        ('I', "d14d16a", PieceKind::Amazon),
    ] {
        let mut promotes_while_double_stepping =
            terachess_position(&[(mover, "d14"), ('p', "e16")], Color::White, "-", "-");
        promotes_while_double_stepping
            .play_uci(double_move)
            .unwrap();
        assert_eq!(
            promotes_while_double_stepping
                .board()
                .piece_at("d16".parse().unwrap()),
            Some(Piece::new(Color::White, promoted_kind))
        );
        assert_eq!(
            promotes_while_double_stepping.en_passant(),
            Some("d15".parse().unwrap())
        );
        let capture = promotes_while_double_stepping
            .parse_uci_move("e16d15")
            .unwrap();
        assert_eq!(capture.kind, MoveKind::EnPassant);
        promotes_while_double_stepping.play(capture).unwrap();
        assert_eq!(
            promotes_while_double_stepping
                .board()
                .piece_at("d16".parse().unwrap()),
            None
        );
    }
}

#[test]
fn terachess_promotions_are_immediate_compulsory_and_piece_specific() {
    let cases = [
        ('P', "a15", "a15a16q", PieceKind::Queen),
        ('I', "b15", "b15b16a", PieceKind::Amazon),
        ('N', "c14", "c14d16f", PieceKind::Buffalo),
        ('M', "d13", "d13e16f", PieceKind::Buffalo),
        ('Z', "e13", "e13g16f", PieceKind::Buffalo),
        ('E', "f15", "f15g16l", PieceKind::Lion),
        ('W', "g15", "g15g16l", PieceKind::Lion),
        ('J', "h15", "h15h16l", PieceKind::Lion),
        ('T', "i15", "i15i16q", PieceKind::Queen),
    ];
    for (letter, from, expected_move, promoted_kind) in cases {
        let mut position = terachess_position(&[(letter, from)], Color::White, "-", "-");
        let bare_move = expected_move
            .strip_suffix(promoted_kind.fen_char())
            .unwrap();
        assert!(!moves_from(&position, from).contains(&bare_move.to_owned()));
        position.play_uci(expected_move).unwrap();
        let to: Square = bare_move[from.len()..].parse().unwrap();
        assert_eq!(
            position.board().piece_at(to),
            Some(Piece::new(Color::White, promoted_kind))
        );
    }

    let mut jumping_troll = terachess_position(&[('T', "j13")], Color::White, "-", "-");
    assert!(moves_from(&jumping_troll, "j13").contains(&"j13j16".to_owned()));
    assert!(!moves_from(&jumping_troll, "j13").contains(&"j13j16q".to_owned()));
    jumping_troll.play_uci("j13j16").unwrap();
    assert_eq!(
        jumping_troll.board().piece_at("j16".parse().unwrap()),
        Some(Piece::new(Color::White, PieceKind::Troll))
    );

    let black = terachess_position(
        &[('K', "p1"), ('k', "p16"), ('p', "a2")],
        Color::Black,
        "-",
        "-",
    );
    assert!(moves_from(&black, "a2").contains(&"a2a1q".to_owned()));
    assert!(!moves_from(&black, "a2").contains(&"a2a1".to_owned()));
}

#[test]
fn every_new_terachess_piece_participates_in_check_detection() {
    let checks = [
        ('a', "f7", "h8"),
        ('l', "f7", "h8"),
        ('f', "e7", "h8"),
        ('j', "f7", "h8"),
        ('s', "h3", "h8"),
        ('y', "c3", "h8"),
        ('i', "g7", "h8"),
        ('d', "e5", "h8"),
        ('t', "e5", "h8"),
    ];
    for (attacker, from, king) in checks {
        let position = terachess_position(&[('K', king), (attacker, from)], Color::White, "-", "-");
        assert!(
            position.is_in_check(Color::White),
            "{attacker} on {from} should attack {king}"
        );
    }
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
