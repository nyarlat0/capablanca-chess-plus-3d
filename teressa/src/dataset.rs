use crate::DATASET_FORMAT_VERSION;
use crate::plan::StrategicPlan;
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use terastockfish::RootCandidate;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PolicyTarget {
    pub uci: String,
    pub probability: f32,
    pub teacher_score: i32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DatasetRecord {
    pub game_id: u64,
    pub ply: u16,
    pub position_fen: String,
    pub history: Vec<String>,
    pub legal_moves: Vec<String>,
    pub policy: Vec<PolicyTarget>,
    /// Win/draw/loss target from the side-to-move's point of view.
    pub wdl: [f32; 3],
    pub teacher_score: i32,
    pub plan: StrategicPlan,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct DatasetHeader {
    format: String,
}

pub struct DatasetShardWriter {
    final_path: PathBuf,
    temporary_path: PathBuf,
    encoder: zstd::stream::write::Encoder<'static, BufWriter<File>>,
    records: usize,
}

impl DatasetShardWriter {
    pub fn create(path: impl AsRef<Path>) -> Result<Self, String> {
        let final_path = path.as_ref().to_path_buf();
        let parent = final_path.parent().unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(parent)
            .map_err(|error| format!("cannot create dataset directory: {error}"))?;
        let file_name = final_path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| "dataset path requires a UTF-8 file name".to_owned())?;
        let temporary_path = parent.join(format!(".{file_name}.part"));
        let file = File::create(&temporary_path)
            .map_err(|error| format!("cannot create dataset shard: {error}"))?;
        let writer = BufWriter::new(file);
        let mut encoder = zstd::stream::write::Encoder::new(writer, 6)
            .map_err(|error| format!("cannot initialize zstd encoder: {error}"))?;
        let header = DatasetHeader {
            format: DATASET_FORMAT_VERSION.to_owned(),
        };
        serde_json::to_writer(&mut encoder, &header)
            .map_err(|error| format!("cannot encode dataset header: {error}"))?;
        encoder
            .write_all(b"\n")
            .map_err(|error| format!("cannot write dataset header: {error}"))?;
        Ok(Self {
            final_path,
            temporary_path,
            encoder,
            records: 0,
        })
    }

    pub fn push(&mut self, record: &DatasetRecord) -> Result<(), String> {
        validate_record(record)?;
        serde_json::to_writer(&mut self.encoder, record)
            .map_err(|error| format!("cannot encode dataset record: {error}"))?;
        self.encoder
            .write_all(b"\n")
            .map_err(|error| format!("cannot write dataset record: {error}"))?;
        self.records += 1;
        Ok(())
    }

    #[must_use]
    pub const fn records(&self) -> usize {
        self.records
    }

    pub fn finish(self) -> Result<usize, String> {
        let mut writer = self
            .encoder
            .finish()
            .map_err(|error| format!("cannot finish dataset compression: {error}"))?;
        writer
            .flush()
            .map_err(|error| format!("cannot flush dataset shard: {error}"))?;
        writer
            .get_ref()
            .sync_all()
            .map_err(|error| format!("cannot sync dataset shard: {error}"))?;
        fs::rename(&self.temporary_path, &self.final_path)
            .map_err(|error| format!("cannot publish dataset shard atomically: {error}"))?;
        Ok(self.records)
    }
}

pub struct DatasetShardReader {
    records: std::vec::IntoIter<DatasetRecord>,
}

impl DatasetShardReader {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, String> {
        let file = File::open(path.as_ref())
            .map_err(|error| format!("cannot open dataset shard: {error}"))?;
        let mut decoder = zstd::stream::read::Decoder::new(file)
            .map_err(|error| format!("cannot initialize zstd decoder: {error}"))?;
        let mut contents = String::new();
        decoder
            .read_to_string(&mut contents)
            .map_err(|error| format!("cannot decompress dataset shard: {error}"))?;
        let mut lines = contents.lines();
        let header: DatasetHeader = serde_json::from_str(
            lines
                .next()
                .ok_or_else(|| "dataset shard is empty".to_owned())?,
        )
        .map_err(|error| format!("cannot decode dataset header: {error}"))?;
        if header.format != DATASET_FORMAT_VERSION {
            return Err(format!(
                "unsupported dataset format `{}`; expected `{DATASET_FORMAT_VERSION}`",
                header.format
            ));
        }
        let records = lines
            .enumerate()
            .map(|(index, line)| {
                let record: DatasetRecord = serde_json::from_str(line).map_err(|error| {
                    format!("cannot decode dataset record {}: {error}", index + 1)
                })?;
                validate_record(&record)?;
                Ok(record)
            })
            .collect::<Result<Vec<_>, String>>()?;
        Ok(Self {
            records: records.into_iter(),
        })
    }
}

impl Iterator for DatasetShardReader {
    type Item = DatasetRecord;

    fn next(&mut self) -> Option<Self::Item> {
        self.records.next()
    }
}

#[must_use]
pub fn policy_targets(candidates: &[RootCandidate], temperature_cp: f32) -> Vec<PolicyTarget> {
    if candidates.is_empty() {
        return Vec::new();
    }
    let temperature = temperature_cp.max(1.0);
    let maximum = candidates
        .iter()
        .map(|candidate| candidate.score)
        .max()
        .unwrap_or(0);
    let weights = candidates
        .iter()
        .map(|candidate| ((candidate.score - maximum) as f32 / temperature).exp())
        .collect::<Vec<_>>();
    let sum = weights.iter().sum::<f32>().max(f32::EPSILON);
    candidates
        .iter()
        .zip(weights)
        .map(|(candidate, weight)| PolicyTarget {
            uci: candidate.chess_move.to_uci(),
            probability: weight / sum,
            teacher_score: candidate.score,
        })
        .collect()
}

fn validate_record(record: &DatasetRecord) -> Result<(), String> {
    if record.position_fen.is_empty() || record.legal_moves.is_empty() {
        return Err("dataset records require a position and legal moves".to_owned());
    }
    if record.policy.is_empty() {
        return Err("dataset records require a non-empty policy target".to_owned());
    }
    let probability = record
        .policy
        .iter()
        .map(|target| target.probability)
        .sum::<f32>();
    if !probability.is_finite() || (probability - 1.0).abs() > 1e-3 {
        return Err(format!(
            "policy target probabilities must sum to one, found {probability}"
        ));
    }
    if record
        .policy
        .iter()
        .any(|target| !record.legal_moves.contains(&target.uci))
    {
        return Err("policy target contains an illegal move".to_owned());
    }
    let wdl = record.wdl.iter().sum::<f32>();
    if !wdl.is_finite() || (wdl - 1.0).abs() > 1e-3 {
        return Err("WDL target probabilities must sum to one".to_owned());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::generate_plan_candidates;
    use capablanca_chess_plus::Variant;
    use terastockfish::{SearchLimits, SearchOptions, Searcher};

    #[test]
    fn teacher_scores_form_a_normalized_policy() {
        let position = Variant::TerachessII.starting_position();
        let mut searcher = Searcher::new(SearchOptions {
            hash_megabytes: 1,
            threads: 1,
        });
        let result = searcher.analyze(
            &position,
            SearchLimits {
                max_nodes: Some(2_000),
                ..SearchLimits::depth(4)
            },
        );
        let targets = policy_targets(&result.root_candidates, 120.0);
        assert!(!targets.is_empty());
        assert!((targets.iter().map(|target| target.probability).sum::<f32>() - 1.0).abs() < 1e-5);
    }

    #[test]
    fn dataset_shard_round_trips_atomically() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("sample.jsonl.zst");
        let position = Variant::TerachessII.starting_position();
        let legal_moves = position
            .legal_moves()
            .into_iter()
            .map(|chess_move| chess_move.to_uci())
            .collect::<Vec<_>>();
        let record = DatasetRecord {
            game_id: 7,
            ply: 0,
            position_fen: position.to_fen(),
            history: Vec::new(),
            policy: vec![PolicyTarget {
                uci: legal_moves[0].clone(),
                probability: 1.0,
                teacher_score: 0,
            }],
            legal_moves,
            wdl: [0.0, 1.0, 0.0],
            teacher_score: 0,
            plan: generate_plan_candidates(&position).remove(0),
        };
        let mut writer = DatasetShardWriter::create(&path).unwrap();
        writer.push(&record).unwrap();
        assert_eq!(writer.finish().unwrap(), 1);
        let decoded = DatasetShardReader::open(path).unwrap().collect::<Vec<_>>();
        assert_eq!(decoded, vec![record]);
    }
}
