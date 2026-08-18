use capablanca_chess_plus::{Move, MoveKind, Position, Variant};

fn assert_round_trip(position: &Position, chess_move: Move) {
    let expected_child = position.after_legal_move(chess_move);
    let mut actual = position.clone();
    let undo = actual.make_move(chess_move);
    assert_eq!(actual, expected_child, "made move {chess_move}");
    actual.unmake_move(undo);
    assert_eq!(&actual, position, "unmade move {chess_move}");
}

#[test]
fn every_built_in_opening_move_round_trips_exactly() {
    for variant in Variant::ALL {
        let position = variant.starting_position();
        for chess_move in position.legal_moves() {
            assert_round_trip(&position, chess_move);
        }
    }
}

#[test]
fn null_move_round_trips_every_non_board_field() {
    let mut position = Variant::TerachessII.starting_position();
    position.play_uci("a4a6").unwrap();
    let original = position.clone();
    let undo = position.make_null_move();
    assert_eq!(position.side_to_move(), original.side_to_move().opposite());
    assert_eq!(position.halfmove_clock(), 0);
    assert_eq!(position.board(), original.board());
    position.unmake_move(undo);
    assert_eq!(position, original);
}

#[test]
fn tactical_generator_matches_filtered_legal_moves_across_variants() {
    for variant in Variant::ALL {
        let mut position = variant.starting_position();
        for ply in 0..12 {
            let legal = position.legal_moves();
            if legal.is_empty() {
                break;
            }
            let mut expected = legal
                .iter()
                .copied()
                .filter(|chess_move| {
                    chess_move.promotion.is_some()
                        || chess_move.kind == MoveKind::EnPassant
                        || position.board().piece_at(chess_move.to).is_some()
                })
                .map(|chess_move| chess_move.to_uci())
                .collect::<Vec<_>>();
            let mut scratch = position.clone();
            let mut actual = scratch
                .legal_tactical_moves_mut()
                .into_iter()
                .map(|chess_move| chess_move.to_uci())
                .collect::<Vec<_>>();
            expected.sort_unstable();
            actual.sort_unstable();
            assert_eq!(actual, expected, "{variant:?} at ply {ply}");
            position = position.after_legal_move(legal[(ply * 17 + 3) % legal.len()]);
        }
    }
}

#[test]
fn castling_en_passant_and_promotion_round_trip_exactly() {
    let castling = Position::from_fen(
        Variant::Gothic.rules(),
        "5k4/10/10/10/10/10/10/R4K3R w KQ - 0 1",
    )
    .unwrap();
    let castles = castling
        .legal_moves()
        .into_iter()
        .filter(|chess_move| matches!(chess_move.kind, MoveKind::Castle(_)))
        .collect::<Vec<_>>();
    assert_eq!(castles.len(), 2);
    for chess_move in castles {
        assert_round_trip(&castling, chess_move);
    }

    let en_passant = Position::from_fen(
        Variant::Gothic.rules(),
        "9k/10/10/4Pp4/10/10/10/K9 w - f6 0 1",
    )
    .unwrap();
    let chess_move = en_passant.parse_uci_move("e5f6").unwrap();
    assert_eq!(chess_move.kind, MoveKind::EnPassant);
    assert_round_trip(&en_passant, chess_move);

    let promotion =
        Position::from_fen(Variant::Gothic.rules(), "9k/P9/10/10/10/10/10/K9 w - - 0 1").unwrap();
    let promotions = promotion
        .legal_moves()
        .into_iter()
        .filter(|chess_move| chess_move.promotion.is_some())
        .collect::<Vec<_>>();
    assert_eq!(promotions.len(), 6);
    for chess_move in promotions {
        assert_round_trip(&promotion, chess_move);
    }
}

#[test]
fn terachess_initial_king_jump_round_trips_its_rights() {
    let position = Position::from_fen(
        Variant::TerachessII.rules(),
        "15k/16/16/16/16/16/16/16/16/16/16/16/16/16/7K8/16 w J - 0 1",
    )
    .unwrap();
    let jumps = position
        .legal_moves()
        .into_iter()
        .filter(|chess_move| {
            chess_move.from.to_string() == "h2"
                && (chess_move.from.file().abs_diff(chess_move.to.file()) > 1
                    || chess_move.from.rank().abs_diff(chess_move.to.rank()) > 1)
        })
        .collect::<Vec<_>>();
    assert!(!jumps.is_empty());
    for chess_move in jumps {
        assert_round_trip(&position, chess_move);
    }
}
