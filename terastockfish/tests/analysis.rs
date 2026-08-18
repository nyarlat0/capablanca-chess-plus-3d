use capablanca_chess_plus::{Color, Position, Square, Variant};
use terastockfish::{
    EvaluationParameters, SearchLimits, SearchOptions, Searcher, evaluate, evaluate_with,
    position_key,
};

fn position(pieces: &[(char, &str)], side: Color) -> Position {
    let mut squares = [[None; 16]; 16];
    for &(piece, coordinate) in pieces {
        let square: Square = coordinate.parse().unwrap();
        squares[usize::from(square.rank())][usize::from(square.file())] = Some(piece);
    }
    let ranks = (0..16)
        .rev()
        .map(|rank| {
            let mut encoded = String::new();
            let mut empty = 0;
            for square in squares[rank] {
                if let Some(piece) = square {
                    if empty != 0 {
                        encoded.push_str(&empty.to_string());
                        empty = 0;
                    }
                    encoded.push(piece);
                } else {
                    empty += 1;
                }
            }
            if empty != 0 {
                encoded.push_str(&empty.to_string());
            }
            encoded
        })
        .collect::<Vec<_>>();
    let side = if side == Color::White { 'w' } else { 'b' };
    Position::from_fen(
        Variant::TerachessII.rules(),
        &format!("{} {side} - - 0 1", ranks.join("/")),
    )
    .unwrap()
}

fn test_searcher() -> Searcher {
    Searcher::new(SearchOptions {
        hash_megabytes: 2,
        threads: 1,
    })
}

#[test]
fn analysis_returns_a_legal_move_from_the_full_starting_array() {
    let position = Variant::TerachessII.starting_position();
    let legal = position.legal_moves();
    let result = test_searcher().analyze(&position, SearchLimits::depth(2));
    assert_eq!(result.completed_depth, 2);
    assert!(
        result
            .best_move
            .is_some_and(|chess_move| legal.contains(&chess_move))
    );
    assert_eq!(
        result.principal_variation.first(),
        result.best_move.as_ref()
    );
}

#[test]
fn search_finds_an_unprotected_high_value_capture() {
    let position = position(
        &[('K', "a1"), ('R', "h8"), ('q', "h9"), ('k', "p16")],
        Color::White,
    );
    let result = test_searcher().analyze(&position, SearchLimits::depth(2));
    assert_eq!(result.best_move.unwrap().to_uci(), "h8h9");
    assert!(result.score > 0);
}

#[test]
fn search_rejects_a_losing_material_exchange() {
    let position = position(
        &[
            ('K', "a1"),
            ('Q', "h8"),
            ('p', "h9"),
            ('r', "h16"),
            ('k', "p16"),
        ],
        Color::White,
    );
    let result = test_searcher().analyze(&position, SearchLimits::depth(2));
    assert_ne!(result.best_move.unwrap().to_uci(), "h8h9");
}

#[test]
fn search_values_compulsory_terachess_promotion() {
    let position = position(&[('K', "a1"), ('P', "a15"), ('k', "p16")], Color::White);
    let result = test_searcher().analyze(&position, SearchLimits::depth(1));
    assert_eq!(result.best_move.unwrap().to_uci(), "a15a16q");
}

#[test]
fn search_recognizes_a_large_board_mate_in_one() {
    let position = position(&[('K', "n14"), ('Q', "o13"), ('k', "p16")], Color::White);
    let result = test_searcher().analyze(&position, SearchLimits::depth(2));
    assert_eq!(result.best_move.unwrap().to_uci(), "o13o15");
    assert_eq!(terastockfish::mate_distance(result.score), Some(1));
}

#[test]
fn analysis_key_covers_clocks_without_changing_after_a_clone() {
    let position = Variant::TerachessII.starting_position();
    assert_eq!(position_key(&position), position_key(&position.clone()));
    let changed_clock = Position::from_fen(
        Variant::TerachessII.rules(),
        &position.to_fen().replace(" 0 1", " 1 1"),
    )
    .unwrap();
    assert_ne!(position_key(&position), position_key(&changed_clock));
}

#[test]
fn parallel_root_workers_share_a_correct_result() {
    let position = position(
        &[('K', "a1"), ('R', "h8"), ('q', "h9"), ('k', "p16")],
        Color::White,
    );
    let mut searcher = Searcher::new(SearchOptions {
        hash_megabytes: 2,
        threads: 4,
    });
    let result = searcher.analyze(&position, SearchLimits::depth(3));
    assert_eq!(result.best_move.unwrap().to_uci(), "h8h9");
}

#[test]
fn production_evaluation_profile_is_the_stable_default() {
    let position = Variant::TerachessII.starting_position();
    let parameters = EvaluationParameters::production();
    assert_eq!(evaluate(&position), evaluate_with(&position, &parameters));

    let options = SearchOptions {
        hash_megabytes: 2,
        threads: 1,
    };
    let default_result = Searcher::new(options).analyze(&position, SearchLimits::depth(2));
    let explicit_result =
        Searcher::with_evaluation(options, parameters).analyze(&position, SearchLimits::depth(2));
    assert_eq!(default_result.best_move, explicit_result.best_move);
    assert_eq!(default_result.score, explicit_result.score);
    assert_eq!(default_result.nodes, explicit_result.nodes);
}

#[test]
fn production_and_published_profiles_remain_explicitly_available() {
    assert_ne!(
        EvaluationParameters::production(),
        EvaluationParameters::published()
    );
    assert_eq!(
        EvaluationParameters::production(),
        EvaluationParameters::strategic_v2()
    );
    assert_eq!(
        EvaluationParameters::production().material_value(capablanca_chess_plus::PieceKind::Eagle),
        1_462
    );
    assert_eq!(
        EvaluationParameters::published().material_value(capablanca_chess_plus::PieceKind::Eagle),
        1_680
    );
}

#[test]
fn replacing_evaluation_parameters_changes_material_scoring() {
    let position = position(
        &[('K', "a1"), ('Q', "h8"), ('k', "p16"), ('r', "i9")],
        Color::White,
    );
    let mut parameters = EvaluationParameters::published();
    let published = evaluate_with(&position, &parameters);
    parameters.set_material_value(capablanca_chess_plus::PieceKind::Queen, 1_200);
    assert_eq!(evaluate_with(&position, &parameters), published - 460);

    let mut searcher = test_searcher();
    searcher.set_evaluation_parameters(parameters);
    assert_eq!(searcher.evaluation_parameters(), parameters);
}
