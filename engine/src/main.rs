use capablanca_chess_plus::{Engine, SearchLimits, Variant};

fn main() {
    let position = Variant::Capablanca.starting_position();
    let result = Engine::new().search(&position, SearchLimits::depth(3));

    if let Some(result) = result {
        println!(
            "{}: {} (score {}, {} nodes)",
            position.rules().name(),
            result.best_move,
            result.score,
            result.nodes
        );
    }
}
