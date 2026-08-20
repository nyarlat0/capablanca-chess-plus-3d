use crate::dataset::{DatasetRecord, DatasetShardWriter, policy_targets};
use crate::plan::generate_plan_candidates;
use crate::relabel::relabel_game_records;
use capablanca_chess_plus::{Color, Game, GameOutcome, Variant};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use terastockfish::{SearchHistory, SearchLimits, SearchOptions, Searcher};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GenerationOptions {
    pub output_directory: PathBuf,
    pub games: usize,
    pub games_per_shard: usize,
    pub jobs: usize,
    pub hash_megabytes_per_job: usize,
    pub nodes_per_move: u64,
    pub opening_plies: u16,
    pub maximum_plies: u16,
    pub temperature_cp: f32,
    pub seed: u64,
    pub time_limit: Option<Duration>,
}

impl Default for GenerationOptions {
    fn default() -> Self {
        Self {
            output_directory: PathBuf::from("target/teressa-data"),
            games: 256,
            games_per_shard: 8,
            jobs: 1,
            hash_megabytes_per_job: 32,
            nodes_per_move: 20_000,
            opening_plies: 10,
            maximum_plies: 600,
            temperature_cp: 120.0,
            seed: 0x5445_5245_5353_4131,
            time_limit: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct GenerationSummary {
    pub shards_written: usize,
    pub shards_reused: usize,
    pub games_written: usize,
    pub records_written: usize,
}

#[derive(Clone, Debug)]
struct PendingPosition {
    record: DatasetRecord,
    side_to_move: Color,
}

pub fn generate_dataset(options: &GenerationOptions) -> Result<GenerationSummary, String> {
    if options.games == 0
        || options.games_per_shard == 0
        || options.jobs == 0
        || options.nodes_per_move == 0
        || options.maximum_plies == 0
    {
        return Err(
            "games, games-per-shard, jobs, nodes, and max-plies must be positive".to_owned(),
        );
    }
    fs::create_dir_all(&options.output_directory)
        .map_err(|error| format!("cannot create generation directory: {error}"))?;
    let shard_count = options.games.div_ceil(options.games_per_shard);
    let next_shard = Arc::new(AtomicUsize::new(0));
    let summary = Arc::new(Mutex::new(GenerationSummary::default()));
    let error = Arc::new(Mutex::new(None::<String>));
    let started = Instant::now();
    let workers = options.jobs.min(shard_count.max(1));

    std::thread::scope(|scope| {
        for worker in 0..workers {
            let next_shard = Arc::clone(&next_shard);
            let summary = Arc::clone(&summary);
            let error = Arc::clone(&error);
            scope.spawn(move || {
                let mut searcher = Searcher::new(SearchOptions {
                    hash_megabytes: options.hash_megabytes_per_job.max(1),
                    threads: 1,
                });
                searcher.set_deterministic_nodes(true);
                loop {
                    if error.lock().expect("generation error lock").is_some()
                        || options
                            .time_limit
                            .is_some_and(|limit| started.elapsed() >= limit)
                    {
                        break;
                    }
                    let shard = next_shard.fetch_add(1, Ordering::Relaxed);
                    if shard >= shard_count {
                        break;
                    }
                    let path = shard_path(&options.output_directory, shard);
                    let first_game = shard * options.games_per_shard;
                    let last_game = (first_game + options.games_per_shard).min(options.games);
                    if path.is_file() {
                        let mut locked = summary.lock().expect("generation summary lock");
                        locked.shards_reused += 1;
                        continue;
                    }
                    let result =
                        write_shard(&path, first_game, last_game, worker, options, &mut searcher);
                    match result {
                        Ok((games, records)) => {
                            let mut locked = summary.lock().expect("generation summary lock");
                            locked.shards_written += 1;
                            locked.games_written += games;
                            locked.records_written += records;
                            eprintln!(
                                "dataset: shard {}/{} games={} positions={}",
                                shard + 1,
                                shard_count,
                                games,
                                records
                            );
                        }
                        Err(message) => {
                            *error.lock().expect("generation error lock") = Some(message);
                            break;
                        }
                    }
                }
            });
        }
    });
    if let Some(error) = error.lock().expect("generation error lock").take() {
        return Err(error);
    }
    let result = *summary.lock().expect("generation summary lock");
    Ok(result)
}

fn shard_path(directory: &Path, shard: usize) -> PathBuf {
    directory.join(format!("teressa-{shard:05}.jsonl.zst"))
}

fn write_shard(
    path: &Path,
    first_game: usize,
    last_game: usize,
    worker: usize,
    options: &GenerationOptions,
    searcher: &mut Searcher,
) -> Result<(usize, usize), String> {
    let mut writer = DatasetShardWriter::create(path)?;
    for game_index in first_game..last_game {
        searcher.clear_hash();
        let game_seed = splitmix64(options.seed ^ game_index as u64 ^ ((worker as u64) << 48));
        for record in generate_game(game_index as u64, game_seed, options, searcher)? {
            writer.push(&record)?;
        }
    }
    let records = writer.finish()?;
    Ok((last_game - first_game, records))
}

fn generate_game(
    game_id: u64,
    seed: u64,
    options: &GenerationOptions,
    searcher: &mut Searcher,
) -> Result<Vec<DatasetRecord>, String> {
    let mut game = Game::new(Variant::TerachessII.starting_position());
    let mut search_history = SearchHistory::new(game.position());
    let mut move_history = Vec::new();
    let mut pending = Vec::new();
    let mut rng = StdRng::seed_from_u64(seed);

    for ply in 0..options.maximum_plies {
        match game.outcome() {
            GameOutcome::Ongoing | GameOutcome::Check => {}
            outcome => return finalize(pending, outcome),
        }
        let position = game.position();
        let analysis = searcher.analyze_with_history(
            position,
            &search_history,
            SearchLimits {
                max_nodes: Some(options.nodes_per_move),
                ..SearchLimits::depth(32)
            },
        );
        let best_move = analysis
            .best_move
            .ok_or_else(|| format!("teacher returned no move in game {game_id}, ply {ply}"))?;
        let mut policy = policy_targets(&analysis.root_candidates, options.temperature_cp);
        if policy.is_empty() {
            policy.push(crate::dataset::PolicyTarget {
                uci: best_move.to_uci(),
                probability: 1.0,
                teacher_score: analysis.score,
            });
        }
        let mut legal_moves = position
            .legal_moves()
            .into_iter()
            .map(|chess_move| chess_move.to_uci())
            .collect::<Vec<_>>();
        legal_moves.sort_unstable();
        let plan = generate_plan_candidates(position)
            .into_iter()
            .max_by(|left, right| {
                left.confidence
                    .total_cmp(&right.confidence)
                    .then_with(|| left.progress.total_cmp(&right.progress))
            })
            .ok_or_else(|| format!("no plan candidates in game {game_id}, ply {ply}"))?;
        pending.push(PendingPosition {
            side_to_move: position.side_to_move(),
            record: DatasetRecord {
                game_id,
                ply,
                position_fen: position.to_fen(),
                history: move_history
                    .iter()
                    .rev()
                    .take(crate::encode::HISTORY_PLIES)
                    .rev()
                    .cloned()
                    .collect(),
                legal_moves,
                policy: policy.clone(),
                wdl: [0.0, 1.0, 0.0],
                teacher_score: analysis.score,
                plan_policy: Vec::new(),
                plan,
            },
        });
        let chess_move = if ply < options.opening_plies {
            sample_policy(&policy, position, &mut rng).unwrap_or(best_move)
        } else {
            best_move
        };
        move_history.push(chess_move.to_uci());
        game.play(chess_move)
            .map_err(|error| format!("teacher produced illegal move {chess_move}: {error}"))?;
        search_history.record(game.position());
    }
    // A cap is a data-production safeguard, not a chess result. Treating it as
    // a neutral target avoids inventing a winner when an exceptionally long
    // game reaches the configured storage budget.
    finalize(
        pending,
        GameOutcome::Draw(capablanca_chess_plus::DrawReason::Stalemate),
    )
}

fn sample_policy(
    policy: &[crate::dataset::PolicyTarget],
    position: &capablanca_chess_plus::Position,
    rng: &mut StdRng,
) -> Option<capablanca_chess_plus::Move> {
    let mut needle = rng.random::<f32>();
    for target in policy {
        needle -= target.probability;
        if needle <= 0.0 {
            return position.parse_uci_move(&target.uci).ok();
        }
    }
    policy
        .last()
        .and_then(|target| position.parse_uci_move(&target.uci).ok())
}

fn finalize(
    pending: Vec<PendingPosition>,
    outcome: GameOutcome,
) -> Result<Vec<DatasetRecord>, String> {
    let mut records = pending
        .into_iter()
        .map(|mut pending| {
            pending.record.wdl = match outcome {
                GameOutcome::Win { winner } if winner == pending.side_to_move => [1.0, 0.0, 0.0],
                GameOutcome::Win { .. } => [0.0, 0.0, 1.0],
                _ => [0.0, 1.0, 0.0],
            };
            pending.record
        })
        .collect::<Vec<_>>();
    relabel_game_records(&mut records)?;
    Ok(records)
}

fn splitmix64(mut value: u64) -> u64 {
    value = value.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut mixed = value;
    mixed = (mixed ^ (mixed >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    mixed = (mixed ^ (mixed >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    mixed ^ (mixed >> 31)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_work_is_rejected() {
        let options = GenerationOptions {
            games: 0,
            ..GenerationOptions::default()
        };
        assert!(generate_dataset(&options).is_err());
    }

    #[test]
    fn finished_shards_are_resumed_without_overwrite() {
        let directory = tempfile::tempdir().unwrap();
        let path = shard_path(directory.path(), 0);
        fs::write(&path, b"already complete").unwrap();
        let options = GenerationOptions {
            output_directory: directory.path().to_path_buf(),
            games: 1,
            games_per_shard: 1,
            jobs: 1,
            nodes_per_move: 1,
            maximum_plies: 1,
            ..GenerationOptions::default()
        };
        let summary = generate_dataset(&options).unwrap();
        assert_eq!(summary.shards_reused, 1);
        assert_eq!(fs::read(path).unwrap(), b"already complete");
    }
}
