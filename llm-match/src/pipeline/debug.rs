//! One-stage experiments. Supplied results are explicit, never generated here.
//! Calls share the production transport/retry/parser and never commit a move.
use super::*;

#[derive(Default)]
pub struct Inputs {
    pub analysis: Option<String>,
    /// One Proposer candidate: id, move, from, to, optional rationale.
    pub candidate: Option<Value>,
    /// Array of accepted candidates with tactical_result instead of rationale.
    pub accepted_candidates: Option<Value>,
}

fn candidates(state: &Match, config: &Config, value: &Value) -> Result<Vec<Candidate>> {
    let values = value
        .as_array()
        .context("accepted candidates must be a JSON array")?;
    ensure!(!values.is_empty(), "accepted candidates must not be empty");
    let mut ids = HashSet::new();
    let mut moves = HashSet::new();
    let mut result = Vec::new();
    for value in values {
        let mut fields = value
            .as_object()
            .context("candidate must be an object")?
            .clone();
        let note = fields
            .remove("tactical_result")
            .context("missing tactical_result")?;
        ensure!(
            !note
                .as_str()
                .context("tactical_result must be text")?
                .trim()
                .is_empty(),
            "empty tactical_result"
        );
        let candidate: Candidate = serde_json::from_value(Value::Object(fields))?;
        bounded(&candidate.id, 32)?;
        ensure!(ids.insert(candidate.id.clone()), "duplicate candidate id");
        let (_, mv) = validate_candidate(state, config, &candidate)?;
        ensure!(moves.insert(mv), "duplicate accepted move");
        result.push(candidate);
    }
    Ok(result)
}

fn data(state: &Match, config: &Config, stage: Stage, inputs: &Inputs) -> Result<PromptData> {
    ensure!(
        inputs.analysis.is_some() == matches!(stage, Stage::Proposer | Stage::Verifier),
        "analysis-file is required only for proposer and verifier"
    );
    ensure!(
        inputs.candidate.is_some() == (stage == Stage::Verifier),
        "candidate-file is required only for verifier"
    );
    ensure!(
        inputs.accepted_candidates.is_some() == (stage == Stage::Selector),
        "accepted-candidates-file is required only for selector"
    );
    let mut data = prompt_data(state, config, stage)?;
    if let Some(analysis) = &inputs.analysis {
        ensure!(
            !analysis.trim().is_empty() && analysis.len() <= 32768,
            "analysis must be non-empty compact text (max 32768 bytes)"
        );
        data.custom["analysis"] = json!(analysis);
    }
    if let Some(value) = &inputs.candidate {
        let candidate: Candidate = serde_json::from_value(value.clone())?;
        validate_candidate(state, config, &candidate)
            .context("debug candidate is illegal or has incorrect metadata")?;
        // Never pass the proposer rationale or id to the calculator.
        data.custom["candidate"] = json!(serde_json::to_string(&json!({
            "move":candidate.proposed_move,"from":candidate.from,"to":candidate.to
        }))?);
    }
    if let Some(value) = &inputs.accepted_candidates {
        let validated = candidates(state, config, value)?;
        let payload: Vec<_> = validated
            .iter()
            .zip(value.as_array().unwrap())
            .map(|(candidate, input)| {
                let mut v = candidate_input(candidate);
                v["tactical_result"] = input["tactical_result"].clone();
                v
            })
            .collect();
        data.custom["accepted-candidates"] = json!(serde_json::to_string(&payload)?);
    }
    if stage == Stage::Proposer {
        data.custom["excluded-moves"] = json!(if state.wrong_moves.is_empty() {
            String::new()
        } else {
            serde_json::to_string(&state.wrong_moves)?
        });
    }
    Ok(data)
}

pub fn preview(state: &Match, config: &Config, stage: Stage, inputs: &Inputs) -> Result<String> {
    let mut debug_state = state.clone();
    debug_state.model_side = debug_state.game.position().side_to_move();
    let state = &debug_state;
    let data = data(state, config, stage, inputs)?;
    Ok(serde_json::to_string_pretty(
        &PromptBuilder::new().build_chat(&config.context, &template(config, stage), &data)?,
    )?)
}

pub async fn run(state: &Match, config: &Config, stage: Stage, inputs: &Inputs) -> Result<String> {
    let data = data(state, config, stage, inputs)?;
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(config.timeout_seconds))
        .build()?;
    let mut client = None;
    match stage {
        Stage::Grounder => {
            request_parsed(&mut client, &http, config, stage, &data, |text| {
                Ok(text.to_owned())
            })
            .await
        }
        Stage::Proposer => {
            let result: Proposal =
                request(&mut client, &http, config, stage, &data, |p: &Proposal| {
                    validate_proposal(p, config)
                })
                .await?;
            Ok(serde_json::to_string_pretty(&result)?)
        }
        Stage::Verifier => {
            let result: Verification = request(
                &mut client,
                &http,
                config,
                stage,
                &data,
                Verification::validate,
            )
            .await?;
            Ok(serde_json::to_string_pretty(&result)?)
        }
        Stage::Selector => {
            let accepted = candidates(state, config, inputs.accepted_candidates.as_ref().unwrap())?;
            let result: Selection =
                request(&mut client, &http, config, stage, &data, |s: &Selection| {
                    let c = accepted
                        .iter()
                        .find(|c| c.id == s.candidate_id)
                        .context("unknown accepted candidate")?;
                    ensure!(
                        c.proposed_move == s.final_move,
                        "selector changed candidate move"
                    );
                    Ok(())
                })
                .await?;
            Ok(serde_json::to_string_pretty(&result)?)
        }
    }
}
