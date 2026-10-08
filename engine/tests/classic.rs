use capablanca_chess_plus::{PieceKind, Position, Variant};

fn pos(fen: &str) -> Position {
    Position::from_fen(Variant::Classic.rules(), fen).unwrap()
}
fn check(fen: &str, uci: &str, san: &str) {
    let p = pos(fen);
    let mv = p.parse_uci_move(uci).unwrap();
    assert_eq!(p.san(mv).unwrap(), san);
    assert_eq!(p.parse_san_move(san).unwrap(), mv);
}

#[test]
fn classic_initial_fen_and_reference_perft() {
    let p = Variant::Classic.starting_position();
    assert_eq!(
        p.to_fen(),
        "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1"
    );
    assert_eq!(p.perft(1), 20);
    assert_eq!(p.perft(2), 400);
    assert_eq!(p.perft(3), 8902);
    let p = pos("r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1");
    assert_eq!(p.perft(1), 48);
    assert_eq!(p.perft(2), 2039);
}

#[test]
fn san_ordinary_moves_capture_and_mate() {
    let mut p = Variant::Classic.starting_position();
    for (uci, san) in [
        ("e2e4", "e4"),
        ("e7e5", "e5"),
        ("g1f3", "Nf3"),
        ("b8c6", "Nc6"),
        ("f3e5", "Nxe5"),
    ] {
        let mv = p.parse_uci_move(uci).unwrap();
        assert_eq!(p.san(mv).unwrap(), san);
        assert_eq!(p.parse_san_move(san).unwrap(), mv);
        p.play(mv).unwrap();
    }
    let mut p = Variant::Classic.starting_position();
    for uci in ["f2f3", "e7e5", "g2g4"] {
        p.play_uci(uci).unwrap();
    }
    let mv = p.parse_san_move("Qh4#").unwrap();
    assert_eq!(p.san(mv).unwrap(), "Qh4#");
    for bad in ["Qh4", "Qh4+", "Qh4#!", "d8h4", "I choose Qh4#"] {
        assert!(p.parse_san_move(bad).is_err());
    }
}

#[test]
fn san_disambiguates_only_legal_sources() {
    let fen = "7k/8/8/2N5/8/2N3N1/8/K7 w - - 0 1";
    check(fen, "c3e4", "Nc3e4");
    check(fen, "c5e4", "N5e4");
    check(fen, "g3e4", "Nge4");
    assert!(pos(fen).parse_san_move("Ne4").is_err());
    check("4r2k/8/8/8/8/2N1N3/8/4K3 w - - 0 1", "c3d5", "Nd5");
}

#[test]
fn san_castling_en_passant_and_promotion() {
    check("r3k2r/8/8/8/8/8/8/R3K2R w KQkq - 0 1", "e1g1", "O-O");
    check("r3k2r/8/8/8/8/8/8/R3K2R w KQkq - 0 1", "e1c1", "O-O-O");
    check("5k2/8/8/8/8/8/8/4K2R w K - 0 1", "e1g1", "O-O+");
    check("7k/8/8/3Pp3/8/8/8/7K w - e6 0 1", "d5e6", "dxe6");
    check("1r5k/P7/8/8/8/8/8/7K w - - 0 1", "a7b8q", "axb8=Q+");
    check("7k/P7/8/8/8/8/8/7K w - - 0 1", "a7a8n", "a8=N");
    let p = pos("7k/P7/8/8/8/8/8/7K w - - 0 1");
    let kinds: Vec<_> = p.legal_moves().iter().filter_map(|m| m.promotion).collect();
    assert_eq!(kinds.len(), 4);
    assert!(!kinds.contains(&PieceKind::Archbishop));
    let blocked = pos("r3kr1r/8/8/8/8/8/8/R3K2R w KQ - 0 1");
    assert!(blocked.parse_san_move("O-O").is_err());
    assert!(p.parse_san_move("a8=Q").is_err()); // check suffix required
}
