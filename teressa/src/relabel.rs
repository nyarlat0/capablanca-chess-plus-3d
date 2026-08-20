use crate::DATASET_FORMAT_VERSION;
use crate::dataset::{DatasetRecord, DatasetShardReader, DatasetShardWriter, PlanPolicyTarget};
use crate::plan::{PlanKind, PlanState, StrategicPlan, generate_plan_candidates};
use capablanca_chess_plus::{Color, Move, Position, Variant};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

const PLAN_LABEL_TEMPERATURE: f32 = 0.18;
const TRAJECTORY_DISCOUNT: f32 = 0.78;

#[derive(Clone, Debug)]
pub struct RelabelOptions {
    pub inputs: Vec<PathBuf>,
    pub output_directory: PathBuf,
    pub jobs: usize,
}

impl Default for RelabelOptions {
    fn default() -> Self {
        Self {
            inputs: vec![PathBuf::from("target/teressa-data-v1")],
            output_directory: PathBuf::from("target/teressa-data-v2"),
            jobs: 1,
        }
    }
}

#[derive(Clone, Debug)]
pub struct RelabelSummary {
    pub shards_written: usize,
    pub shards_reused: usize,
    pub games: usize,
    pub records: usize,
    pub selected_plans: [usize; PlanKind::COUNT],
    pub plan_probability_mass: [f64; PlanKind::COUNT],
}

impl Default for RelabelSummary {
    fn default() -> Self {
        Self {
            shards_written: 0,
            shards_reused: 0,
            games: 0,
            records: 0,
            selected_plans: [0; PlanKind::COUNT],
            plan_probability_mass: [0.0; PlanKind::COUNT],
        }
    }
}

impl RelabelSummary {
    fn merge(&mut self, other: Self) {
        self.shards_written += other.shards_written;
        self.shards_reused += other.shards_reused;
        self.games += other.games;
        self.records += other.records;
        for kind in PlanKind::ALL {
            self.selected_plans[kind.index()] += other.selected_plans[kind.index()];
            self.plan_probability_mass[kind.index()] += other.plan_probability_mass[kind.index()];
        }
    }
}

pub fn relabel_dataset(options: &RelabelOptions) -> Result<RelabelSummary, String> {
    if options.inputs.is_empty() || options.jobs == 0 {
        return Err("relabel requires at least one input and a positive job count".to_owned());
    }
    let mut inputs = Vec::new();
    for input in &options.inputs {
        collect_shards(input, &mut inputs)?;
    }
    inputs.sort();
    inputs.dedup();
    if inputs.is_empty() {
        return Err("no .jsonl.zst input shards found".to_owned());
    }
    fs::create_dir_all(&options.output_directory)
        .map_err(|error| format!("cannot create relabel output directory: {error}"))?;
    reject_in_place_output(&inputs, &options.output_directory)?;
    let mut names = HashSet::new();
    for input in &inputs {
        let name = input
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or_else(|| format!("input shard {} has no UTF-8 file name", input.display()))?;
        if !names.insert(name.to_owned()) {
            return Err(format!(
                "multiple input shards are named {name}; use separate relabel runs"
            ));
        }
    }

    let inputs = Arc::new(inputs);
    let next = Arc::new(AtomicUsize::new(0));
    let summary = Arc::new(Mutex::new(RelabelSummary::default()));
    let first_error = Arc::new(Mutex::new(None::<String>));
    let workers = options.jobs.min(inputs.len());
    std::thread::scope(|scope| {
        for _ in 0..workers {
            let inputs = Arc::clone(&inputs);
            let next = Arc::clone(&next);
            let summary = Arc::clone(&summary);
            let first_error = Arc::clone(&first_error);
            let output_directory = &options.output_directory;
            scope.spawn(move || {
                loop {
                    if first_error.lock().expect("relabel error lock").is_some() {
                        break;
                    }
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some(input) = inputs.get(index) else {
                        break;
                    };
                    let output = output_directory.join(
                        input
                            .file_name()
                            .expect("validated relabel shard file name"),
                    );
                    match relabel_shard(input, &output) {
                        Ok(shard) => {
                            eprintln!(
                                "relabel: shard {}/{} games={} positions={}",
                                index + 1,
                                inputs.len(),
                                shard.games,
                                shard.records
                            );
                            summary.lock().expect("relabel summary lock").merge(shard);
                        }
                        Err(error) => {
                            *first_error.lock().expect("relabel error lock") = Some(error);
                            break;
                        }
                    }
                }
            });
        }
    });
    if let Some(error) = first_error.lock().expect("relabel error lock").take() {
        return Err(error);
    }
    Ok(summary.lock().expect("relabel summary lock").clone())
}

fn relabel_shard(input: &Path, output: &Path) -> Result<RelabelSummary, String> {
    if output.is_file() {
        let reader = DatasetShardReader::open(output)?;
        if reader.format() != DATASET_FORMAT_VERSION {
            return Err(format!(
                "existing output {} is not a {DATASET_FORMAT_VERSION} shard",
                output.display()
            ));
        }
        let records = reader.collect::<Vec<_>>();
        let mut summary = summarize_records(&records);
        summary.shards_reused = 1;
        return Ok(summary);
    }
    let mut records = DatasetShardReader::open(input)?.collect::<Vec<_>>();
    records.sort_by_key(|record| (record.game_id, record.ply));
    let mut games = 0;
    let mut start = 0;
    while start < records.len() {
        let game_id = records[start].game_id;
        let end = records[start..]
            .iter()
            .position(|record| record.game_id != game_id)
            .map_or(records.len(), |offset| start + offset);
        relabel_game_records(&mut records[start..end])?;
        games += 1;
        start = end;
    }
    let mut writer = DatasetShardWriter::create(output)?;
    for record in &records {
        writer.push(record)?;
    }
    writer.finish()?;
    let mut summary = summarize_records(&records);
    summary.games = games;
    summary.shards_written = 1;
    Ok(summary)
}

fn summarize_records(records: &[DatasetRecord]) -> RelabelSummary {
    let mut summary = RelabelSummary {
        records: records.len(),
        games: records
            .iter()
            .map(|record| record.game_id)
            .collect::<HashSet<_>>()
            .len(),
        ..RelabelSummary::default()
    };
    for record in records {
        summary.selected_plans[record.plan.kind.index()] += 1;
        for target in &record.plan_policy {
            summary.plan_probability_mass[target.kind.index()] += f64::from(target.probability);
        }
    }
    summary
}

/// Replace legacy fixed-priority labels for one chronological game with
/// trajectory-derived soft labels and a persistent concrete plan per side.
pub fn relabel_game_records(records: &mut [DatasetRecord]) -> Result<(), String> {
    if records.is_empty() {
        return Ok(());
    }
    let game_id = records[0].game_id;
    if records.iter().any(|record| record.game_id != game_id)
        || records
            .windows(2)
            .any(|pair| pair[0].ply.checked_add(1) != Some(pair[1].ply))
    {
        return Err(
            "relabel_game_records requires one game with contiguous ascending plies".to_owned(),
        );
    }
    let positions = records
        .iter()
        .map(|record| {
            Position::from_fen(Variant::TerachessII.rules(), &record.position_fen).map_err(
                |error| {
                    format!(
                        "cannot parse game {} ply {} during relabel: {error}",
                        record.game_id, record.ply
                    )
                },
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    let steps = trajectory_steps(records, &positions)?;
    let mut white = PlanState::default();
    let mut black = PlanState::default();
    for index in 0..records.len() {
        let (mut proposed, policy) = trajectory_label(index, &positions, &steps)?;
        proposed.confidence = policy
            .iter()
            .find(|target| target.kind == proposed.kind)
            .map_or(0.0, |target| target.probability);
        let side = positions[index].side_to_move();
        let state = match side {
            Color::White => &mut white,
            Color::Black => &mut black,
        };
        records[index].plan = state.choose(&positions[index], proposed).clone();
        records[index].plan_policy = policy;
        if let Some(step) = steps.get(index) {
            state.record_chosen_move(step.chess_move);
        }
    }
    Ok(())
}

#[derive(Clone)]
struct TrajectoryStep {
    chess_move: Move,
    after: Position,
}

fn trajectory_steps(
    records: &[DatasetRecord],
    positions: &[Position],
) -> Result<Vec<TrajectoryStep>, String> {
    let mut steps = Vec::with_capacity(records.len());
    for index in 0..records.len() {
        let uci = records
            .get(index + 1)
            .and_then(|next| next.history.last())
            .or_else(|| {
                records[index]
                    .policy
                    .iter()
                    .max_by_key(|target| target.teacher_score)
                    .map(|target| &target.uci)
            })
            .ok_or_else(|| {
                format!(
                    "game {} ply {} has neither a following move nor a teacher move",
                    records[index].game_id, records[index].ply
                )
            })?;
        let chess_move = positions[index].parse_uci_move(uci).map_err(|error| {
            format!(
                "cannot parse trajectory move {uci} in game {} ply {}: {error}",
                records[index].game_id, records[index].ply
            )
        })?;
        let after = if let Some(position) = positions.get(index + 1) {
            position.clone()
        } else {
            let mut position = positions[index].clone();
            position.play(chess_move).map_err(|error| {
                format!(
                    "cannot apply final trajectory move in game {}: {error}",
                    records[index].game_id
                )
            })?;
            position
        };
        steps.push(TrajectoryStep { chess_move, after });
    }
    Ok(steps)
}

fn trajectory_label(
    index: usize,
    positions: &[Position],
    steps: &[TrajectoryStep],
) -> Result<(StrategicPlan, Vec<PlanPolicyTarget>), String> {
    let candidates = generate_plan_candidates(&positions[index]);
    if candidates.is_empty() {
        return Err("symbolic planner produced no candidates during relabel".to_owned());
    }
    let side = positions[index].side_to_move();
    let mut best_by_kind = Vec::<(StrategicPlan, f32)>::new();
    for candidate in candidates {
        let score = score_trajectory(candidate.clone(), side, index, positions, steps);
        if let Some((current, current_score)) = best_by_kind
            .iter_mut()
            .find(|(current, _)| current.kind == candidate.kind)
        {
            if score > *current_score {
                *current = candidate;
                *current_score = score;
            }
        } else {
            best_by_kind.push((candidate, score));
        }
    }
    best_by_kind.sort_by_key(|(plan, _)| plan.kind.index());
    let maximum = best_by_kind
        .iter()
        .map(|(_, score)| *score)
        .fold(f32::NEG_INFINITY, f32::max);
    let weights = best_by_kind
        .iter()
        .map(|(_, score)| ((*score - maximum) / PLAN_LABEL_TEMPERATURE).exp())
        .collect::<Vec<_>>();
    let sum = weights.iter().sum::<f32>().max(f32::EPSILON);
    let policy = best_by_kind
        .iter()
        .zip(&weights)
        .map(|((plan, _), weight)| PlanPolicyTarget {
            kind: plan.kind,
            probability: *weight / sum,
        })
        .collect::<Vec<_>>();
    let selected = best_by_kind
        .into_iter()
        .max_by(|left, right| left.1.total_cmp(&right.1))
        .expect("candidate list was checked")
        .0;
    Ok((selected, policy))
}

fn score_trajectory(
    mut plan: StrategicPlan,
    side: Color,
    start: usize,
    positions: &[Position],
    steps: &[TrajectoryStep],
) -> f32 {
    let mut own_moves = 0_u8;
    let mut discount = 1.0_f32;
    let mut score = 0.0_f32;
    let mut weight = 0.0_f32;
    for index in start..steps.len() {
        if positions[index].side_to_move() != side {
            continue;
        }
        score += discount
            * plan.observe_own_move(
                &positions[index],
                steps[index].chess_move,
                &steps[index].after,
            );
        weight += discount;
        own_moves += 1;
        if own_moves >= plan.horizon.clamp(2, 6) {
            break;
        }
        discount *= TRAJECTORY_DISCOUNT;
    }
    score / weight.max(f32::EPSILON)
}

fn collect_shards(path: &Path, output: &mut Vec<PathBuf>) -> Result<(), String> {
    if path.is_file() {
        if path.to_string_lossy().ends_with(".jsonl.zst") {
            output.push(path.to_path_buf());
        }
        return Ok(());
    }
    let entries = fs::read_dir(path)
        .map_err(|error| format!("cannot read dataset path {}: {error}", path.display()))?;
    for entry in entries {
        let child = entry
            .map_err(|error| format!("cannot read dataset entry: {error}"))?
            .path();
        if child.is_dir() {
            collect_shards(&child, output)?;
        } else if child.to_string_lossy().ends_with(".jsonl.zst") {
            output.push(child);
        }
    }
    Ok(())
}

fn reject_in_place_output(inputs: &[PathBuf], output: &Path) -> Result<(), String> {
    let output = output
        .canonicalize()
        .unwrap_or_else(|_| output.to_path_buf());
    if inputs.iter().any(|input| {
        input
            .parent()
            .and_then(|parent| parent.canonicalize().ok())
            .is_some_and(|parent| parent == output)
    }) {
        return Err("relabel output must differ from every input directory".to_owned());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dataset::PolicyTarget;

    #[test]
    fn trajectory_labels_are_soft_normalized_and_persistent() {
        let mut position = Variant::TerachessII.starting_position();
        let mut history = Vec::<String>::new();
        let mut records = Vec::new();
        for ply in 0..6_u16 {
            let mut legal = position.legal_moves();
            legal.sort_unstable_by_key(|chess_move| chess_move.to_uci());
            let chosen = if ply == 0 {
                position.parse_uci_move("d2g5").unwrap()
            } else {
                legal[0]
            };
            records.push(DatasetRecord {
                game_id: 11,
                ply,
                position_fen: position.to_fen(),
                history: history.iter().rev().take(8).rev().cloned().collect(),
                legal_moves: legal.iter().map(|chess_move| chess_move.to_uci()).collect(),
                policy: vec![PolicyTarget {
                    uci: chosen.to_uci(),
                    probability: 1.0,
                    teacher_score: 0,
                }],
                wdl: [0.0, 1.0, 0.0],
                teacher_score: 0,
                plan_policy: Vec::new(),
                plan: generate_plan_candidates(&position).remove(0),
            });
            history.push(chosen.to_uci());
            position.play(chosen).unwrap();
        }

        relabel_game_records(&mut records).unwrap();

        for record in &records {
            assert!(record.plan_policy.len() > 1);
            assert!(
                (record
                    .plan_policy
                    .iter()
                    .map(|target| target.probability)
                    .sum::<f32>()
                    - 1.0)
                    .abs()
                    < 1.0e-5
            );
            let position =
                Position::from_fen(Variant::TerachessII.rules(), &record.position_fen).unwrap();
            assert!(record.plan.applicable(&position));
        }
        assert_eq!(records[0].plan.kind, records[2].plan.kind);
    }

    #[test]
    fn malformed_game_order_is_rejected() {
        let position = Variant::TerachessII.starting_position();
        let legal = position.legal_moves();
        let plan = generate_plan_candidates(&position).remove(0);
        let record = |ply| DatasetRecord {
            game_id: 3,
            ply,
            position_fen: position.to_fen(),
            history: Vec::new(),
            legal_moves: legal.iter().map(|chess_move| chess_move.to_uci()).collect(),
            policy: vec![PolicyTarget {
                uci: legal[0].to_uci(),
                probability: 1.0,
                teacher_score: 0,
            }],
            wdl: [0.0, 1.0, 0.0],
            teacher_score: 0,
            plan_policy: Vec::new(),
            plan: plan.clone(),
        };
        assert!(relabel_game_records(&mut [record(2), record(1)]).is_err());
        assert!(relabel_game_records(&mut [record(1), record(3)]).is_err());
    }
}
