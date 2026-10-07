use capablanca_chess_plus::{Color, Piece, PieceKind, Position, Variant};

#[test]
fn names_and_symbols_are_complete_and_variant_aware() {
    let names: std::collections::BTreeSet<_> = PieceKind::ALL
        .into_iter()
        .map(PieceKind::display_name)
        .collect();
    assert_eq!(names.len(), PieceKind::ALL.len());
    for kind in PieceKind::ALL {
        assert!(!kind.movement_description().is_empty());
    }
    for variant in Variant::ALL {
        let position = variant.starting_position();
        for (_, piece) in position.board().pieces() {
            assert!(position.rules().uses_piece(piece.kind));
            let symbol = position.rules().piece_symbol(piece);
            assert_eq!(symbol.is_ascii_uppercase(), piece.color == Color::White);
        }
    }
    let tera = Variant::TerachessII.rules();
    for (kind, symbol) in [
        (PieceKind::King, 'K'),
        (PieceKind::Admiral, 'S'),
        (PieceKind::Centaur, 'J'),
        (PieceKind::Prince, 'I'),
        (PieceKind::Troll, 'T'),
        (PieceKind::Archbishop, 'X'),
        (PieceKind::Chancellor, 'H'),
    ] {
        assert_eq!(tera.piece_symbol(Piece::new(Color::White, kind)), symbol);
    }
}

#[test]
fn leaper_descriptions_match_generated_displacements() {
    for kind in [
        PieceKind::Knight,
        PieceKind::Camel,
        PieceKind::Giraffe,
        PieceKind::Elephant,
        PieceKind::Machine,
    ] {
        let rules = Variant::TerachessII.rules();
        let symbol = rules.piece_symbol(Piece::new(Color::White, kind));
        let fen = format!("15k/16/16/16/16/16/16/16/7{symbol}8/16/16/16/16/16/K15/16 w - - 0 1");
        let position = Position::from_fen(rules, &fen).unwrap();
        let description = kind.movement_description();
        let from = "h8".parse().unwrap();
        let moves: Vec<_> = position
            .legal_moves()
            .into_iter()
            .filter(|m| m.from == from)
            .collect();
        assert_eq!(moves.len(), 8);
        for mv in moves {
            let (dx, dy) = (
                mv.from.file().abs_diff(mv.to.file()),
                mv.from.rank().abs_diff(mv.to.rank()),
            );
            assert!(
                description.contains(&format!("({dx},{dy})")),
                "{description} vs {}",
                mv.to_uci()
            );
        }
    }
    assert!(
        PieceKind::Giraffe
            .movement_description()
            .contains("(2,3)/(3,2)")
    );
    assert!(!PieceKind::Giraffe.movement_description().contains("(1,4)"));
}
