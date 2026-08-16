use capablanca_chess_plus::Variant;
use std::hint::black_box;
use std::time::Instant;
use terastockfish::{SearchLimits, SearchOptions, Searcher};

fn main() {
    let position = Variant::TerachessII.starting_position();
    let mut samples = Vec::with_capacity(5);
    let mut last_result = None;
    for _ in 0..5 {
        let mut searcher = Searcher::new(SearchOptions {
            hash_megabytes: 64,
            threads: 1,
        });
        let started = Instant::now();
        let result = black_box(searcher.analyze(&position, SearchLimits::depth(4)));
        samples.push(started.elapsed());
        last_result = Some(result);
    }
    samples.sort_unstable();
    let elapsed = samples[samples.len() / 2];
    let result = last_result.unwrap();
    let nps = if elapsed.is_zero() {
        0
    } else {
        (result.nodes as f64 / elapsed.as_secs_f64()) as u64
    };
    println!(
        "Terachess II depth {} median of {}: {} nodes in {:.3}s ({nps} nps), best move {:?}",
        result.completed_depth,
        samples.len(),
        result.nodes,
        elapsed.as_secs_f64(),
        result.best_move
    );
}
