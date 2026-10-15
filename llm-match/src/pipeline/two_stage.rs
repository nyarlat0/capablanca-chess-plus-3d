//! Ranked proposer with optional independent selector; engine use is legality and commit only.
use super::*;
use prompt_core::{Preset, ReasoningTemplate};
use serde::Deserialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PipelineMode {
    SingleStage,
    TwoStage,
    Legacy,
}
impl std::str::FromStr for PipelineMode {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self> {
        match s {
            "single-stage" => Ok(Self::SingleStage),
            "two-stage" => Ok(Self::TwoStage),
            "legacy" => Ok(Self::Legacy),
            _ => bail!("LLM_PIPELINE must be single-stage, two-stage or legacy"),
        }
    }
}

#[derive(Clone)]
pub struct StageSettings {
    pub template: SystemPromptTemplate,
    pub preset: Preset,
    pub reasoning: ReasoningTemplate,
}
#[derive(Clone)]
pub struct Settings {
    pub proposer: StageSettings,
    pub selector: StageSettings,
}
impl Settings {
    pub(crate) fn load(base: &Path, get: impl Fn(&str) -> Result<String>) -> Result<Self> {
        fn read<T: DeserializeOwned>(
            base: &Path,
            get: &impl Fn(&str) -> Result<String>,
            key: &str,
            default: &str,
        ) -> Result<T> {
            let path = base.join(get(key).unwrap_or_else(|_| default.into()));
            serde_json::from_slice(
                &std::fs::read(&path).with_context(|| format!("cannot read {}", path.display()))?,
            )
            .with_context(|| format!("invalid {key}: {}", path.display()))
        }
        let role = |name: &str| -> Result<StageSettings> {
            let key = format!("LLM_TWO_{}", name.to_uppercase());
            let preset = Preset::from_value(read(
                base,
                &get,
                &format!("{key}_PRESET"),
                &format!("presets/two_{name}.json"),
            )?)?;
            preset.generation_length()?;
            let reasoning = read(
                base,
                &get,
                &format!("{key}_REASONING_TEMPLATE"),
                "templates/reasoning/Think_XML.json",
            )?;
            output::final_answer(&reasoning, "")?;
            Ok(StageSettings {
                template: read(
                    base,
                    &get,
                    &format!("{key}_TEMPLATE"),
                    &format!("templates/two_stage/{name}.json"),
                )?,
                preset,
                reasoning,
            })
        };
        Ok(Self {
            proposer: role("proposer")?,
            selector: role("selector")?,
        })
    }
    pub fn stage(&self, stage: Stage) -> &StageSettings {
        match stage {
            Stage::Proposer => &self.proposer,
            Stage::Selector => &self.selector,
            _ => panic!("not a two-stage role"),
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RankedCandidate {
    #[serde(rename = "move")]
    proposed_move: Value,
    rationale: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RankedProposal {
    candidates: Vec<RankedCandidate>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Choice {
    choice: String,
}

#[cfg(test)]
mod protocol_tests {
    use super::*;
    #[test]
    fn simplified_protocol_does_not_accept_metadata_or_missing_choices() {
        for answer in [
            "{}",
            r#"{"choice":null}"#,
            r#"{"choice":"c1","final_move":"e4"}"#,
            r#"{"choice":"c1","choice":"c2"}"#,
        ] {
            assert!(output::parse_json_repaired::<Choice>(answer).is_err());
        }
        assert!(
            output::parse_json_repaired::<RankedProposal>(
                r#"{"candidates":[{"move":"e4","from":"e2","rationale":"idea"}]}"#
            )
            .is_err()
        );
        assert_eq!(
            "legacy".parse::<PipelineMode>().unwrap(),
            PipelineMode::Legacy
        );
        assert!("unknown".parse::<PipelineMode>().is_err());
    }
}

pub(super) async fn generate(
    state: &mut Match,
    config: &Config,
    mut rejected: impl FnMut(usize, &str),
) -> Result<Move> {
    ensure!(
        state.game.position().side_to_move() == state.model_side,
        "not the model turn"
    );
    ensure!(
        matches!(
            state.game.outcome(),
            capablanca_chess_plus::GameOutcome::Ongoing | capablanca_chess_plus::GameOutcome::Check
        ),
        "game is over"
    );
    ensure!(
        config.max_attempts > 0 && config.stage_max_attempts > 0,
        "attempt limits must be positive"
    );
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(config.timeout_seconds))
        .build()?;
    let mut client = None;
    let mut excluded: HashSet<String> = state.wrong_moves.iter().cloned().collect();
    for round in 1..=config.max_attempts {
        tracing::info!(round, pipeline = ?config.pipeline_mode, "LLM candidate round");
        let mut data = request_data(state, config, Stage::Proposer)?;
        let mut exclusions: Vec<_> = excluded.iter().cloned().collect();
        exclusions.sort();
        data.custom["excluded-moves"] = json!(serde_json::to_string(&exclusions)?);
        let proposal: RankedProposal = request_parsed(
            &mut client,
            &http,
            config,
            Stage::Proposer,
            &data,
            |answer| {
                let proposal: RankedProposal = output::parse_json_repaired(answer)?;
                ensure!(
                    (3..=5).contains(&proposal.candidates.len()),
                    "expected 3-5 ranked candidates"
                );
                for c in &proposal.candidates {
                    move_text(&c.proposed_move, config)?;
                    ensure!(
                        !c.rationale.trim().is_empty(),
                        "missing candidate rationale"
                    );
                }
                Ok(proposal)
            },
        )
        .await?;
        let mut seen = HashSet::new();
        let mut legal = Vec::new();
        for c in proposal.candidates {
            let text = move_text(&c.proposed_move, config)?;
            if excluded.contains(&text) {
                tracing::info!(round, reason = "already_excluded", "llm_candidate_rejected");
                continue;
            }
            let candidate = Candidate {
                id: String::new(),
                proposed_move: c.proposed_move,
                from: Value::Null,
                to: Value::Null,
                rationale: String::new(),
            };
            match resolve_candidate(state, config, &candidate) {
                Ok((trial, mv)) if config.pipeline_mode == PipelineMode::SingleStage => {
                    // Commit the first legal move in proposer order; do not inspect later candidates.
                    *state = trial;
                    return Ok(mv);
                }
                Ok((_, mv)) if seen.insert(mv) => legal.push((candidate, mv)),
                Ok(_) => tracing::info!(
                    round,
                    reason = "duplicate_engine_move",
                    "llm_candidate_rejected"
                ),
                Err(_) => {
                    excluded.insert(text);
                    tracing::info!(round, reason = "illegal_move", "llm_candidate_rejected");
                }
            }
        }
        if legal.is_empty() {
            continue;
        }
        // Stable filtering preserves proposer preference; rationale stays private.
        for (i, (c, _)) in legal.iter_mut().enumerate() {
            c.id = format!("c{}", i + 1);
        }
        let mut data = request_data(state, config, Stage::Selector)?;
        data.custom["accepted-candidates"] = json!(serde_json::to_string(
            &legal
                .iter()
                .map(|(c, _)| json!({"id":c.id,"move":c.proposed_move}))
                .collect::<Vec<_>>()
        )?);
        let selected: Choice = request_parsed(
            &mut client,
            &http,
            config,
            Stage::Selector,
            &data,
            |answer| {
                let choice: Choice = output::parse_json_repaired(answer)?;
                ensure!(
                    choice.choice == "reject_all"
                        || legal.iter().any(|(c, _)| c.id == choice.choice),
                    "unknown candidate ID"
                );
                Ok(choice)
            },
        )
        .await?;
        if selected.choice == "reject_all" {
            for (c, _) in &legal {
                excluded.insert(move_text(&c.proposed_move, config)?);
            }
            tracing::info!(
                round,
                reason = "selector_reject_all",
                "llm_candidate_rejected"
            );
            continue;
        }
        let (candidate, expected) = legal.iter().find(|(c, _)| c.id == selected.choice).unwrap();
        match resolve_candidate(state, config, candidate) {
            Ok((trial, mv)) if mv == *expected => {
                *state = trial;
                return Ok(mv);
            }
            _ => {
                let text = move_text(&candidate.proposed_move, config)?;
                state.record_rejected_final(&text);
                excluded.insert(text.clone());
                rejected(round, &text);
                tracing::info!(
                    round,
                    reason = "final_engine_rejection",
                    "llm_candidate_rejected"
                );
            }
        }
    }
    bail!(
        "No legal selected move after {} ranked-proposer rounds",
        config.max_attempts
    )
}
