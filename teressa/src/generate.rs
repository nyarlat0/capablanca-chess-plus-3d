use crate::DATASET_FORMAT_VERSION;
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

const GENERATION_MANIFEST_FILE: &str = "generation-manifest.json";
const GENERATION_MANIFEST_FORMAT: &str = "TERESSA_GENERATION_MANIFEST_V1";
const GENERATOR_IMPLEMENTATION: &str =
    "trajectory-v2/strict-node-cap-v1/worker-independent-state-v1/capped-wdl-mask-v1";
const GAME_SEED_DOMAIN: u64 = 0x4741_4d45_5f53_4545;
const GAME_ID_DOMAIN: u64 = 0x4741_4d45_5f49_4421;

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
    /// Explicitly attach a manifest to an older non-empty shard directory.
    /// The caller is responsible for supplying the settings that created it.
    pub adopt_existing: bool,
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
            adopt_existing: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct GenerationManifest {
    format: String,
    dataset_format: String,
    implementation: String,
    games_per_shard: usize,
    hash_megabytes_per_job: usize,
    nodes_per_move: u64,
    opening_plies: u16,
    maximum_plies: u16,
    temperature_cp: f32,
    seed: u64,
}

impl GenerationManifest {
    fn from_options(options: &GenerationOptions) -> Self {
        Self {
            format: GENERATION_MANIFEST_FORMAT.to_owned(),
            dataset_format: DATASET_FORMAT_VERSION.to_owned(),
            implementation: GENERATOR_IMPLEMENTATION.to_owned(),
            games_per_shard: options.games_per_shard,
            hash_megabytes_per_job: options.hash_megabytes_per_job,
            nodes_per_move: options.nodes_per_move,
            opening_plies: options.opening_plies,
            maximum_plies: options.maximum_plies,
            temperature_cp: options.temperature_cp,
            seed: options.seed,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct GenerationSummary {
    pub shards_written: usize,
    pub shards_reused: usize,
    pub games_written: usize,
    pub games_reused: usize,
    /// Newly generated games that reached `maximum_plies` without a known
    /// chess result. Existing reused shards are intentionally not rescanned.
    pub capped_games_written: usize,
    pub records_written: usize,
    pub search_nodes_written: u64,
    pub search_microseconds_written: u128,
}

impl GenerationSummary {
    #[must_use]
    pub const fn games_completed(self) -> usize {
        self.games_written + self.games_reused
    }

    #[must_use]
    pub fn nodes_per_position(self) -> f64 {
        self.search_nodes_written as f64 / self.records_written.max(1) as f64
    }

    #[must_use]
    pub fn teacher_nodes_per_second(self) -> f64 {
        self.search_nodes_written as f64
            / (self.search_microseconds_written.max(1) as f64 / 1_000_000.0)
    }
}

#[derive(Clone, Debug)]
struct PendingPosition {
    record: DatasetRecord,
    side_to_move: Color,
}

#[derive(Debug)]
struct GeneratedGame {
    records: Vec<DatasetRecord>,
    search_nodes: u64,
    search_microseconds: u128,
}

#[derive(Clone, Copy, Debug, Default)]
struct GeneratedShard {
    games: usize,
    records: usize,
    capped_games: usize,
    search_nodes: u64,
    search_microseconds: u128,
}

pub fn generate_dataset(options: &GenerationOptions) -> Result<GenerationSummary, String> {
    if options.games == 0
        || options.games_per_shard == 0
        || options.jobs == 0
        || options.hash_megabytes_per_job == 0
        || options.nodes_per_move == 0
        || options.maximum_plies == 0
        || !options.temperature_cp.is_finite()
        || options.temperature_cp <= 0.0
    {
        return Err(
            "games, games-per-shard, jobs, hash, nodes, max-plies, and temperature must be positive"
                .to_owned(),
        );
    }
    if !options.games.is_multiple_of(options.games_per_shard) {
        return Err(
            "games must be a multiple of games-per-shard so the corpus can be safely extended later"
                .to_owned(),
        );
    }
    fs::create_dir_all(&options.output_directory)
        .map_err(|error| format!("cannot create generation directory: {error}"))?;
    ensure_generation_manifest(options)?;
    let shard_count = options.games.div_ceil(options.games_per_shard);
    let next_shard = Arc::new(AtomicUsize::new(0));
    let summary = Arc::new(Mutex::new(GenerationSummary::default()));
    let error = Arc::new(Mutex::new(None::<String>));
    let started = Instant::now();
    let workers = options.jobs.min(shard_count.max(1));

    std::thread::scope(|scope| {
        for _ in 0..workers {
            let next_shard = Arc::clone(&next_shard);
            let summary = Arc::clone(&summary);
            let error = Arc::clone(&error);
            scope.spawn(move || {
                let mut searcher = generation_searcher(options.hash_megabytes_per_job);
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
                        locked.games_reused += last_game - first_game;
                        continue;
                    }
                    let result = write_shard(&path, first_game, last_game, options, &mut searcher);
                    match result {
                        Ok(shard_summary) => {
                            let mut locked = summary.lock().expect("generation summary lock");
                            locked.shards_written += 1;
                            locked.games_written += shard_summary.games;
                            locked.capped_games_written += shard_summary.capped_games;
                            locked.records_written += shard_summary.records;
                            locked.search_nodes_written = locked
                                .search_nodes_written
                                .saturating_add(shard_summary.search_nodes);
                            locked.search_microseconds_written = locked
                                .search_microseconds_written
                                .saturating_add(shard_summary.search_microseconds);
                            let snapshot = *locked;
                            drop(locked);
                            print_progress(
                                shard,
                                shard_count,
                                shard_summary.games,
                                shard_summary.records,
                                snapshot,
                                options.games,
                                workers,
                                started.elapsed(),
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

fn generation_searcher(hash_megabytes: usize) -> Searcher {
    // This intentionally matches both Bevy TeraStockfish backends: one search
    // thread, a strict max_nodes budget, and DeterministicNodes left disabled.
    // A single-threaded traversal is deterministic without allowing the last
    // iterative-deepening pass to run past the requested node budget.
    Searcher::new(SearchOptions {
        hash_megabytes: hash_megabytes.max(1),
        threads: 1,
    })
}

fn shard_path(directory: &Path, shard: usize) -> PathBuf {
    directory.join(format!("teressa-{shard:05}.jsonl.zst"))
}

fn write_shard(
    path: &Path,
    first_game: usize,
    last_game: usize,
    options: &GenerationOptions,
    searcher: &mut Searcher,
) -> Result<GeneratedShard, String> {
    let mut writer = DatasetShardWriter::create(path)?;
    let mut summary = GeneratedShard {
        games: last_game - first_game,
        ..GeneratedShard::default()
    };
    for game_index in first_game..last_game {
        searcher.reset_for_new_game();
        let game_index = game_index as u64;
        let game_seed = deterministic_game_seed(options.seed, game_index);
        let game_id = deterministic_game_id(options.seed, game_index);
        let generated = generate_game(game_id, game_seed, options, searcher)?;
        summary.capped_games += usize::from(
            generated
                .records
                .first()
                .is_some_and(|record| !record.outcome_known),
        );
        summary.search_nodes = summary.search_nodes.saturating_add(generated.search_nodes);
        summary.search_microseconds = summary
            .search_microseconds
            .saturating_add(generated.search_microseconds);
        for record in generated.records {
            writer.push(&record)?;
        }
    }
    summary.records = writer.finish()?;
    Ok(summary)
}

fn deterministic_game_seed(master_seed: u64, game_index: u64) -> u64 {
    splitmix64(master_seed ^ GAME_SEED_DOMAIN ^ game_index.wrapping_mul(0x9e37_79b9_7f4a_7c15))
}

fn deterministic_game_id(master_seed: u64, game_index: u64) -> u64 {
    splitmix64(master_seed ^ GAME_ID_DOMAIN ^ game_index.wrapping_mul(0xbf58_476d_1ce4_e5b9))
}

#[allow(clippy::too_many_arguments)]
fn print_progress(
    shard: usize,
    shard_count: usize,
    games: usize,
    records: usize,
    summary: GenerationSummary,
    target_games: usize,
    workers: usize,
    elapsed: Duration,
) {
    let elapsed_hours = elapsed.as_secs_f64() / 3_600.0;
    let games_per_hour = if elapsed_hours > 0.0 {
        summary.games_written as f64 / elapsed_hours
    } else {
        0.0
    };
    let positions_per_hour = if elapsed_hours > 0.0 {
        summary.records_written as f64 / elapsed_hours
    } else {
        0.0
    };
    let wall_nodes_per_second =
        summary.search_nodes_written as f64 / elapsed.as_secs_f64().max(f64::EPSILON);
    let remaining = target_games.saturating_sub(summary.games_completed());
    let eta_hours = if games_per_hour > 0.0 {
        format!("{:.1}", remaining as f64 / games_per_hour)
    } else {
        "unknown".to_owned()
    };
    let warmup_shards = workers.saturating_mul(2).min(shard_count);
    let eta = if summary.shards_written < warmup_shards {
        "warming-up".to_owned()
    } else {
        eta_hours
    };
    eprintln!(
        "dataset: shard {}/{} games={} positions={} completed_games={}/{} capped_new={} games_per_hour={:.1} positions_per_hour={:.0} nodes_per_position={:.0} teacher_knps_per_worker={:.1} wall_knps={:.1} elapsed_hours={:.2} eta_hours={}",
        shard + 1,
        shard_count,
        games,
        records,
        summary.games_completed(),
        target_games,
        summary.capped_games_written,
        games_per_hour,
        positions_per_hour,
        summary.nodes_per_position(),
        summary.teacher_nodes_per_second() / 1_000.0,
        wall_nodes_per_second / 1_000.0,
        elapsed_hours,
        eta,
    );
}

fn ensure_generation_manifest(options: &GenerationOptions) -> Result<(), String> {
    let expected = GenerationManifest::from_options(options);
    let path = options.output_directory.join(GENERATION_MANIFEST_FILE);
    if path.is_file() {
        let contents = fs::read_to_string(&path)
            .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
        let actual: GenerationManifest = serde_json::from_str(&contents)
            .map_err(|error| format!("cannot decode {}: {error}", path.display()))?;
        if actual != expected {
            return Err(format!(
                "generation settings do not match {}; use a new output directory instead of mixing incompatible shards\nexpected: {}\nactual:   {}",
                path.display(),
                serde_json::to_string(&expected).unwrap_or_default(),
                serde_json::to_string(&actual).unwrap_or_default(),
            ));
        }
        return Ok(());
    }

    let has_completed_shards = fs::read_dir(&options.output_directory)
        .map_err(|error| format!("cannot inspect generation directory: {error}"))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .any(|path| {
            path.is_file()
                && path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| {
                        name.starts_with("teressa-") && name.ends_with(".jsonl.zst")
                    })
        });
    if has_completed_shards && !options.adopt_existing {
        return Err(format!(
            "{} contains legacy shards without {}; rerun once with --adopt-existing only if every existing shard was created with the supplied settings",
            options.output_directory.display(),
            GENERATION_MANIFEST_FILE,
        ));
    }
    write_manifest_atomic(&path, &expected)
}

fn write_manifest_atomic(path: &Path, manifest: &GenerationManifest) -> Result<(), String> {
    let temporary = path.with_extension("json.part");
    let mut contents = serde_json::to_string_pretty(manifest)
        .map_err(|error| format!("cannot encode generation manifest: {error}"))?;
    contents.push('\n');
    fs::write(&temporary, contents)
        .map_err(|error| format!("cannot write {}: {error}", temporary.display()))?;
    fs::rename(&temporary, path)
        .map_err(|error| format!("cannot publish {}: {error}", path.display()))
}

fn generate_game(
    game_id: u64,
    seed: u64,
    options: &GenerationOptions,
    searcher: &mut Searcher,
) -> Result<GeneratedGame, String> {
    let mut game = Game::new(Variant::TerachessII.starting_position());
    let mut search_history = SearchHistory::new(game.position());
    let mut move_history = Vec::new();
    let mut pending = Vec::new();
    let mut rng = StdRng::seed_from_u64(seed);
    let mut search_nodes = 0_u64;
    let mut search_microseconds = 0_u128;

    for ply in 0..options.maximum_plies {
        match game.outcome() {
            GameOutcome::Ongoing | GameOutcome::Check => {}
            outcome => {
                return finalize(pending, outcome, true, search_nodes, search_microseconds);
            }
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
        if analysis.nodes > options.nodes_per_move {
            return Err(format!(
                "teacher exceeded strict node budget in game {game_id}, ply {ply}: {} > {}",
                analysis.nodes, options.nodes_per_move
            ));
        }
        search_nodes = search_nodes.saturating_add(analysis.nodes);
        search_microseconds = search_microseconds.saturating_add(analysis.elapsed.as_micros());
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
                outcome_known: true,
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
    match game.outcome() {
        GameOutcome::Ongoing | GameOutcome::Check => {
            // A cap is a data-production safeguard, not a chess result.
            // Preserve all non-WDL targets without inventing a result.
            finalize(
                pending,
                GameOutcome::Draw(capablanca_chess_plus::DrawReason::Stalemate),
                false,
                search_nodes,
                search_microseconds,
            )
        }
        outcome => finalize(pending, outcome, true, search_nodes, search_microseconds),
    }
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
    outcome_known: bool,
    search_nodes: u64,
    search_microseconds: u128,
) -> Result<GeneratedGame, String> {
    let mut records = pending
        .into_iter()
        .map(|mut pending| {
            pending.record.wdl = match outcome {
                GameOutcome::Win { winner } if winner == pending.side_to_move => [1.0, 0.0, 0.0],
                GameOutcome::Win { .. } => [0.0, 0.0, 1.0],
                _ => [0.0, 1.0, 0.0],
            };
            pending.record.outcome_known = outcome_known;
            pending.record
        })
        .collect::<Vec<_>>();
    relabel_game_records(&mut records)?;
    Ok(GeneratedGame {
        records,
        search_nodes,
        search_microseconds,
    })
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
    use crate::dataset::DatasetShardReader;

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
            adopt_existing: true,
            ..GenerationOptions::default()
        };
        let summary = generate_dataset(&options).unwrap();
        assert_eq!(summary.shards_reused, 1);
        assert_eq!(summary.games_reused, 1);
        assert_eq!(fs::read(path).unwrap(), b"already complete");
    }

    #[test]
    fn manifest_rejects_mixed_teacher_settings() {
        let directory = tempfile::tempdir().unwrap();
        let options = GenerationOptions {
            output_directory: directory.path().to_path_buf(),
            games: 1,
            games_per_shard: 1,
            jobs: 1,
            nodes_per_move: 1,
            maximum_plies: 1,
            ..GenerationOptions::default()
        };
        generate_dataset(&options).unwrap();
        let incompatible = GenerationOptions {
            nodes_per_move: 2,
            ..options
        };
        let error = generate_dataset(&incompatible).unwrap_err();
        assert!(error.contains("do not match"));
    }

    #[test]
    fn game_identity_is_stable_and_seed_namespaced() {
        assert_eq!(
            deterministic_game_seed(7, 11),
            deterministic_game_seed(7, 11)
        );
        assert_eq!(deterministic_game_id(7, 11), deterministic_game_id(7, 11));
        assert_ne!(deterministic_game_id(7, 11), deterministic_game_id(8, 11));
        assert_ne!(deterministic_game_id(7, 11), deterministic_game_id(7, 12));
    }

    #[test]
    fn generation_search_uses_a_repeatable_strict_node_cap() {
        let position = Variant::TerachessII.starting_position();
        let run = || {
            let mut searcher = generation_searcher(1);
            assert!(!searcher.deterministic_nodes());
            searcher.reset_for_new_game();
            let result = searcher.analyze(
                &position,
                SearchLimits {
                    max_nodes: Some(500),
                    ..SearchLimits::depth(32)
                },
            );
            assert!(result.nodes <= 500);
            (
                result.best_move,
                result.score,
                result.completed_depth,
                result.nodes,
                result.principal_variation,
                result.root_candidates,
            )
        };
        assert_eq!(run(), run());
    }

    #[test]
    fn generated_games_do_not_depend_on_worker_assignment() {
        let sequential = tempfile::tempdir().unwrap();
        let parallel = tempfile::tempdir().unwrap();
        let base = GenerationOptions {
            games: 4,
            games_per_shard: 1,
            jobs: 1,
            hash_megabytes_per_job: 1,
            nodes_per_move: 1,
            opening_plies: 2,
            maximum_plies: 2,
            seed: 17,
            ..GenerationOptions::default()
        };
        generate_dataset(&GenerationOptions {
            output_directory: sequential.path().to_path_buf(),
            ..base.clone()
        })
        .unwrap();
        generate_dataset(&GenerationOptions {
            output_directory: parallel.path().to_path_buf(),
            jobs: 2,
            ..base
        })
        .unwrap();

        let read = |directory: &Path| {
            (0..4)
                .flat_map(|shard| DatasetShardReader::open(shard_path(directory, shard)).unwrap())
                .collect::<Vec<_>>()
        };
        assert_eq!(read(sequential.path()), read(parallel.path()));
    }
}
