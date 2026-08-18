use capablanca_chess_plus::{Position, Variant};
use std::hint::black_box;
use std::time::{Duration, Instant};
use terastockfish::{SearchLimits, SearchOptions, Searcher};

fn main() {
    let starting = Variant::TerachessII.starting_position();
    let mut cases = vec![("start", starting.clone(), 4_u8)];
    cases.extend(generated_snapshots(starting));

    let mut total_nodes = 0_u64;
    let mut total_elapsed = Duration::ZERO;
    for (name, position, depth) in cases {
        let (elapsed, nodes, best_move) = benchmark_case(&position, depth);
        let nps = if elapsed.is_zero() {
            0
        } else {
            (nodes as f64 / elapsed.as_secs_f64()) as u64
        };
        println!(
            "{name}: depth {depth}, {nodes} nodes in {:.3}s ({nps} nps), best {best_move:?}",
            elapsed.as_secs_f64()
        );
        total_nodes = total_nodes.saturating_add(nodes);
        total_elapsed += elapsed;
    }
    let aggregate_nps = (total_nodes as f64 / total_elapsed.as_secs_f64()) as u64;
    println!(
        "aggregate: {total_nodes} nodes in {:.3}s ({aggregate_nps} nps)",
        total_elapsed.as_secs_f64()
    );
}

fn benchmark_case(
    position: &Position,
    depth: u8,
) -> (Duration, u64, Option<capablanca_chess_plus::Move>) {
    let mut samples = Vec::with_capacity(3);
    let mut last_result = None;
    for _ in 0..3 {
        let mut searcher = Searcher::new(SearchOptions {
            hash_megabytes: 64,
            threads: 1,
        });
        let started = Instant::now();
        let result = black_box(searcher.analyze(position, SearchLimits::depth(depth)));
        samples.push((started.elapsed(), result.nodes));
        last_result = Some(result);
    }
    samples.sort_unstable_by_key(|sample| sample.0);
    let (elapsed, nodes) = samples[samples.len() / 2];
    (elapsed, nodes, last_result.unwrap().best_move)
}

fn generated_snapshots(mut position: Position) -> Vec<(&'static str, Position, u8)> {
    let mut snapshots = Vec::new();
    let mut random = SplitMix64(0x5354_5245_4e47_5448);
    for ply in 1..=48 {
        let mut moves = position.legal_moves();
        if moves.is_empty() {
            break;
        }
        moves.sort_unstable_by_key(|chess_move| chess_move.to_uci());
        let chess_move = moves[(random.next() % moves.len() as u64) as usize];
        position = position.after_legal_move(chess_move);
        match ply {
            8 => snapshots.push(("opening-8", position.clone(), 3)),
            24 => snapshots.push(("middlegame-24", position.clone(), 3)),
            48 => snapshots.push(("middlegame-48", position.clone(), 3)),
            _ => {}
        }
    }
    snapshots
}

struct SplitMix64(u64);

impl SplitMix64 {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut value = self.0;
        value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        value ^ (value >> 31)
    }
}
