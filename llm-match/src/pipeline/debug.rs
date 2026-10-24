//! One-stage experiments. Supplied results are explicit, never generated here.
//! Calls share the production transport/retry/parser and never commit a move.
use super::*;

#[derive(Default)]
pub struct Inputs {
    pub selected_piece: Option<String>,
    pub control_map: Option<String>,
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
    if stage == Stage::OneGrounderFull {
        ensure!(
            config.classic.is_some() && state.variant == capablanca_chess_plus::Variant::Classic,
            "one-grounder-full requires Classic chess"
        );
    }
    ensure!(
        inputs.selected_piece.is_some() == (stage == Stage::OneGrounder),
        "--selected-piece is required only for --stage one-grounder"
    );
    ensure!(inputs.control_map.is_some() == (stage == Stage::PinAnalyzer),
        "--grounder-map-file required only for --stage pin-analyzer");
    let selected = inputs
        .selected_piece
        .as_deref()
        .map(|piece| {
            ensure!(
                config.classic.is_some(),
                "one-grounder requires Classic chess"
            );
            crate::grounder::selected_piece(state.game.position(), piece)
        })
        .transpose()?;
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
    if let Some(selected) = selected {
        data.custom["selected-piece"] = json!(selected);
    }
    if let Some(map) = &inputs.control_map {
        ensure!(state.variant == capablanca_chess_plus::Variant::Classic && config.classic.is_some(),
            "pin-analyzer requires Classic chess");
        let graph = crate::grounder::ControlGraph::parse(state.game.position(), map)?;
        data.custom["control-map"] = json!(graph.normalized_json()?);
    }
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

fn full_targets(state: &Match) -> Vec<crate::grounder::PieceControls> {
    let position = state.game.position();
    let mut targets: Vec<_> = position
        .board()
        .pieces()
        .map(|(square, piece)| {
            let square = square.to_string();
            crate::grounder::PieceControls {
                id: crate::grounder::piece_id(position, &square, piece),
                square,
                controls: vec![],
                xray: vec![],
            }
        })
        .collect();
    targets.sort_by(|a, b| a.id.cmp(&b.id));
    targets
}

fn one_inputs(square: &str) -> Inputs {
    Inputs {
        selected_piece: Some(square.into()),
        ..Default::default()
    }
}

pub fn preview(state: &Match, config: &Config, stage: Stage, inputs: &Inputs) -> Result<String> {
    let mut debug_state = state.clone();
    debug_state.model_side = debug_state.game.position().side_to_move();
    let state = &debug_state;
    let data = data(state, config, stage, inputs)?;
    let image = crate::board_image::BoardImage::new(state.game.position())?;
    if stage == Stage::OneGrounderFull {
        let requests = full_targets(state)
            .into_iter()
            .map(|piece| {
                let data = self::data(
                    state,
                    config,
                    Stage::OneGrounder,
                    &one_inputs(&piece.square),
                )?;
                let request =
                    super::prompts::build_request(config, Stage::OneGrounder, &data, &image)?;
                Ok(json!({"id":piece.id,"square":piece.square,"messages":request.messages}))
            })
            .collect::<Result<Vec<_>>>()?;
        return Ok(serde_json::to_string_pretty(&requests)?);
    }
    Ok(serde_json::to_string_pretty(
        &super::prompts::build_request(config, stage, &data, &image)?.messages,
    )?)
}

pub async fn run(state: &Match, config: &Config, stage: Stage, inputs: &Inputs) -> Result<String> {
    let data = data(state, config, stage, inputs)?;
    let image = crate::board_image::BoardImage::new(state.game.position())?;
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(config.timeout_seconds))
        .build()?;
    let mut client = None;
    match stage {
        Stage::PinAnalyzer => {
            let graph = crate::grounder::ControlGraph::parse(
                state.game.position(), inputs.control_map.as_deref().context("missing control map")?
            )?;
            request_parsed(&mut client, &http, config, stage, &data, &image, |answer| {
                let report = crate::pins::PinReport::parse(state.game.position(), &graph, answer)?;
                Ok(serde_json::to_string_pretty(&report)?)
            }).await
        }
        Stage::OneGrounderFull => {
            let mut pieces = full_targets(state);
            let total = pieces.len();
            for (index, piece) in pieces.iter_mut().enumerate() {
                tracing::info!(piece_id = %piece.id, index = index + 1, total, "one-grounder-full piece");
                // Fresh model context for each piece; previous results stay host-side.
                let data = self::data(
                    state,
                    config,
                    Stage::OneGrounder,
                    &one_inputs(&piece.square),
                )?;
                let result = request_parsed_grounder(
                    &mut client,
                    &http,
                    config,
                    Stage::OneGrounder,
                    &data,
                    &image,
                    state.game.position(),
                    |text| {
                        let result=crate::grounder::OnePieceControls::parse(text)?;
                        crate::grounder::validate_one(state.game.position(),&piece.square,&result)?;
                        Ok(result)
                    },
                )
                .await
                .with_context(|| {
                    format!(
                        "one-grounder-full failed for {} ({}/{total}); no complete map produced",
                        piece.id,
                        index + 1
                    )
                })?;
                crate::grounder::validate_one(state.game.position(),&piece.square,&result)?;
                piece.controls = result.controls;
                piece.xray = result.xray;
            }
            crate::grounder::ControlGraph::new(
                state.game.position(),
                crate::grounder::ControlMap { version: 1, pieces },
            )?
            .normalized_json()
        }
        Stage::OneGrounder => {
            request_parsed_grounder(&mut client, &http, config, stage, &data, &image,
                state.game.position(), |text| {
                let result=crate::grounder::OnePieceControls::parse(text)?;
                let piece=inputs.selected_piece.as_deref().context("selected piece is missing")?;
                let sq=piece.split_once('_').map_or(piece,|(_,s)|s);
                crate::grounder::validate_one(state.game.position(),sq,&result)?;
                Ok(serde_json::to_string_pretty(&result)?)
            })
            .await
        }
        Stage::Grounder => {
            request_parsed_grounder(&mut client, &http, config, stage, &data, &image,
                state.game.position(), |text| {
                grounder_response(state, config, text)
            })
            .await
        }
        Stage::Proposer => {
            let result: Proposal = request(
                &mut client,
                &http,
                config,
                stage,
                &data,
                &image,
                |p: &Proposal| validate_proposal(p, config),
            )
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
                &image,
                Verification::validate,
            )
            .await?;
            Ok(serde_json::to_string_pretty(&result)?)
        }
        Stage::Selector => {
            let accepted = candidates(state, config, inputs.accepted_candidates.as_ref().unwrap())?;
            let result: Selection = request(
                &mut client,
                &http,
                config,
                stage,
                &data,
                &image,
                |s: &Selection| {
                    let c = accepted
                        .iter()
                        .find(|c| c.id == s.candidate_id)
                        .context("unknown accepted candidate")?;
                    ensure!(
                        c.proposed_move == s.final_move,
                        "selector changed candidate move"
                    );
                    Ok(())
                },
            )
            .await?;
            Ok(serde_json::to_string_pretty(&result)?)
        }
    }
}
