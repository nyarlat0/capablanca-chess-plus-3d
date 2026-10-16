//! Independent, bounded LLM stages. No engine candidate generation is used here.
mod protocol;
pub mod two_stage;
use crate::{Config, Match, OutputFormat, StateFormat, classic, output};
use anyhow::{Context, Result, bail, ensure};
use capablanca_chess_plus::{Move, Square};
use prompt_core::{
    ChatGenerationRequest, KoboldClient, PromptBuilder, PromptData, SystemPromptTemplate,
};
use protocol::*;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::{collections::HashSet, path::Path, time::Duration};

#[derive(Clone)]
pub struct Templates {
    pub analyst: SystemPromptTemplate,
    pub grounder: SystemPromptTemplate,
    pub proposer: SystemPromptTemplate,
    pub verifier: SystemPromptTemplate,
    pub selector: SystemPromptTemplate,
}
impl Templates {
    pub(crate) fn load(base: &Path, get: impl Fn(&str) -> Result<String>) -> Result<Self> {
        let read = |key: &str, name: &str| -> Result<SystemPromptTemplate> {
            let path =
                base.join(get(key).unwrap_or_else(|_| format!("templates/pipeline/{name}.json")));
            serde_json::from_slice(
                &std::fs::read(&path).with_context(|| format!("cannot read {}", path.display()))?,
            )
            .with_context(|| format!("invalid stage template {}", path.display()))
        };
        Ok(Self {
            analyst: read("LLM_ANALYST_TEMPLATE", "analyst")?,
            grounder: read("LLM_GROUNDER_TEMPLATE", "grounder")?,
            proposer: read("LLM_PROPOSER_TEMPLATE", "proposer")?,
            verifier: read("LLM_VERIFIER_TEMPLATE", "verifier")?,
            selector: read("LLM_SELECTOR_TEMPLATE", "selector")?,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Analyst,
    Grounder,
    Proposer,
    Verifier,
    Selector,
}

fn move_text(value: &Value, config: &Config) -> Result<String> {
    if config.classic.is_none() && config.representation.output_format == OutputFormat::NumericJson
    {
        ensure!(value.is_object(), "move must be a JSON object");
        Ok(serde_json::to_string(value)?)
    } else {
        let text = value.as_str().context("move must be a string")?;
        bounded(text, 160)?;
        ensure!(
            text == text.trim(),
            "move must not have surrounding whitespace"
        );
        Ok(text.into())
    }
}
fn check_move(value: &Value, state: &Match, config: &Config) -> Result<String> {
    let text = move_text(value, config)?;
    if config.classic.is_some() {
        ensure!(classic::san_shape(&text), "expected canonical SAN syntax");
    } else {
        crate::parse_model_move(
            &text,
            config.representation.output_format,
            state.game.position(),
        )?;
    }
    Ok(text)
}
fn square(value: &Value, state: &Match, config: &Config) -> Result<Square> {
    if config.classic.is_none() && config.representation.state_format != StateFormat::PiecesAscii {
        crate::square_from_numeric(
            serde_json::from_value(value.clone())?,
            state.game.position().board().size(),
        )
    } else {
        let text = value.as_str().context("expected square string")?;
        let square: Square = text.parse()?;
        ensure!(square.to_string() == text, "noncanonical square");
        let size = state.game.position().board().size();
        ensure!(
            square.file() < size.files() && square.rank() < size.ranks(),
            "square outside board"
        );
        Ok(square)
    }
}

fn candidate_input(c: &Candidate) -> Value {
    json!({"id":c.id,"move":c.proposed_move,"from":c.from,"to":c.to})
}

/// Stage-local data allowlist: only explicitly referenced values enter a template.
pub fn prompt_data(state: &Match, config: &Config, stage: Stage) -> Result<PromptData> {
    let mut data = state.prompt_data_with(config)?;
    let original = data.custom.clone();
    let private = matches!(stage, Stage::Grounder | Stage::Verifier | Stage::Selector);
    if private {
        data.messages.clear();
    }
    data.custom = json!({});
    for key in [
        "fen",
        "ascii",
        "board-state",
        "piece-list",
        "piece-list-num",
        "side-to-move",
        "chess-rules",
    ] {
        if let Some(value) = original.get(key) {
            data.custom[key] = value.clone();
        }
    }
    if !private {
        data.custom["move-history"] = json!(if config.classic.is_some() {
            original
                .get("classic-san-history")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned()
        } else {
            data.messages
                .iter()
                .map(|message| message.content.as_str())
                .collect::<Vec<_>>()
                .join("\n")
        });
        for key in ["last-move", "classic-san-history"] {
            if let Some(value) = original.get(key) {
                data.custom[key] = value.clone();
            }
        }
        data.custom["wrong-move"] = json!(if state.wrong_moves.is_empty() {
            String::new()
        } else {
            serde_json::to_string(&state.wrong_moves)?
        });
    }
    data.custom["classic"] = json!(config.classic.is_some());
    data.custom["numeric-coordinates"] = json!(
        config.classic.is_none() && config.representation.state_format != StateFormat::PiecesAscii
    );
    data.custom["output-uci"] =
        json!(config.classic.is_none() && config.representation.output_format == OutputFormat::Uci);
    data.custom["output-numeric-json"] = json!(
        config.classic.is_none()
            && config.representation.output_format == OutputFormat::NumericJson
    );
    data.custom["output-semantic"] = json!(
        config.classic.is_none() && config.representation.output_format == OutputFormat::Semantic
    );
    data.custom["output-numeric"] = json!(
        config.classic.is_none() && config.representation.output_format == OutputFormat::Numeric
    );
    data.custom["history-enabled"] = json!(
        !private
            && config.classic.as_ref().map_or(
                config.representation.history_mode == crate::HistoryMode::Full,
                |c| c.include_history
            )
    );
    Ok(data)
}

pub(crate) fn template(config: &Config, stage: Stage) -> SystemPromptTemplate {
    match stage {
        Stage::Analyst => &config.pipeline.analyst,
        Stage::Grounder => &config.pipeline.grounder,
        Stage::Proposer => &config.pipeline.proposer,
        Stage::Verifier => &config.pipeline.verifier,
        Stage::Selector => &config.pipeline.selector,
    }
    .clone()
}

pub fn preview(state: &Match, config: &Config) -> Result<String> {
    let stage = if config.pipeline_mode != two_stage::PipelineMode::Legacy {
        Stage::Proposer
    } else {
        Stage::Analyst
    };
    Ok(serde_json::to_string_pretty(
        &PromptBuilder::new().build_chat(
            &config.context,
            &request_template(config, stage),
            &request_data(state, config, stage)?,
        )?,
    )?)
}

fn request_template(config: &Config, stage: Stage) -> SystemPromptTemplate {
    if config.pipeline_mode != two_stage::PipelineMode::Legacy {
        return config.two_stage.stage(stage).template.clone();
    }
    template(config, stage)
}

fn request_data(state: &Match, config: &Config, stage: Stage) -> Result<PromptData> {
    prompt_data(state, config, stage)
}

async fn request<T: DeserializeOwned>(
    client: &mut Option<KoboldClient>,
    http: &reqwest::Client,
    config: &Config,
    stage: Stage,
    data: &PromptData,
    validate: impl Fn(&T) -> Result<()>,
) -> Result<T> {
    request_parsed(client, http, config, stage, data, |answer| {
        let value = if matches!(stage, Stage::Proposer | Stage::Selector) {
            output::parse_final_json(answer)?
        } else {
            output::parse_json(answer)?
        };
        validate(&value)?;
        Ok(value)
    })
    .await
}

async fn request_parsed<T>(
    client: &mut Option<KoboldClient>,
    http: &reqwest::Client,
    config: &Config,
    stage: Stage,
    data: &PromptData,
    parse: impl Fn(&str) -> Result<T>,
) -> Result<T> {
    let system = request_template(config, stage);
    let ranked_pipeline = config.pipeline_mode != two_stage::PipelineMode::Legacy;
    let preset = if ranked_pipeline {
        &config.two_stage.stage(stage).preset
    } else {
        config.preset_for_stage(stage)
    };
    let reasoning = if ranked_pipeline {
        &config.two_stage.stage(stage).reasoning
    } else {
        &config.reasoning
    };
    let mut last_error = None;
    for attempt in 1..=config.stage_max_attempts {
        let started = std::time::Instant::now();
        let mut input_tokens = None;
        tracing::info!(?stage, attempt, "LLM stage");
        let response: Result<String> = async {
            if client.is_none() {
                *client = Some(
                    KoboldClient::connect_with_client(config.endpoint.clone(), http.clone())
                        .await?,
                );
            }
            let client = client.as_ref().unwrap();
            let options = ChatGenerationRequest::from_preset(Vec::new(), preset)?;
            let fitted = PromptBuilder::new()
                .build_chat_with_budget(
                    &config.context,
                    &system,
                    data,
                    client,
                    &options,
                    client.context_budget(preset)?,
                )
                .await?;
            input_tokens = Some(fitted.preview.tokens);
            Ok(client.generate_chat(&fitted.request).await?.text)
        }
        .await;
        let raw = match response {
            Ok(raw) => raw,
            Err(e) if output::transient(&e) => {
                tracing::info!(
                    ?stage,
                    attempt,
                    retries = attempt - 1,
                    elapsed_ms = started.elapsed().as_millis() as u64,
                    ?input_tokens,
                    reason = "transport_retry",
                    "llm_stage_metrics"
                );
                last_error = Some(e);
                if attempt < config.stage_max_attempts {
                    tokio::time::sleep(Duration::from_millis(100 * (1 << (attempt - 1).min(4))))
                        .await;
                }
                continue;
            }
            Err(e) => {
                tracing::info!(
                    ?stage,
                    attempt,
                    elapsed_ms = started.elapsed().as_millis() as u64,
                    ?input_tokens,
                    reason = "fatal_request",
                    "llm_stage_metrics"
                );
                return Err(e);
            }
        };
        let output_tokens = match tokio::time::timeout(
            Duration::from_secs(2),
            client.as_ref().unwrap().count_tokens(&raw),
        )
        .await
        {
            Ok(Ok(tokens)) => Some(tokens),
            _ => None,
        };
        let record = |reason: &str| {
            tracing::info!(
                ?stage,
                attempt,
                retries = attempt - 1,
                elapsed_ms = started.elapsed().as_millis() as u64,
                ?input_tokens,
                ?output_tokens,
                output_tokens_source = "retokenized_raw_including_reasoning",
                reason,
                "llm_stage_metrics"
            )
        };
        let answer = if matches!(stage, Stage::Proposer | Stage::Selector) {
            output::final_json_answer(reasoning, &raw)?
        } else {
            output::final_answer(reasoning, &raw)?
        };
        let Some(answer) = answer else {
            record("incomplete_reasoning");
            continue;
        };
        // Bound payload size, not the analyst's choice of format.
        if answer.trim().is_empty() || answer.len() > 32_768 {
            record("empty_or_oversized_answer");
            continue;
        }
        match parse(&answer) {
            Ok(value) => {
                record("accepted_response");
                return Ok(value);
            }
            Err(e) => {
                record("invalid_protocol");
                last_error = Some(e);
            }
        }
    }
    Err(last_error
        .unwrap_or_else(|| anyhow::anyhow!("incomplete reasoning or missing structured result")))
    .with_context(|| format!("{stage:?} exhausted {} attempts", config.stage_max_attempts))
}

pub async fn generate(
    state: &mut Match,
    config: &Config,
    mut rejected: impl FnMut(usize, &str),
) -> Result<Move> {
    if config.pipeline_mode != two_stage::PipelineMode::Legacy {
        return two_stage::generate(state, config, rejected).await;
    }
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
    for stage in [
        Stage::Analyst,
        Stage::Grounder,
        Stage::Proposer,
        Stage::Verifier,
        Stage::Selector,
    ] {
        config.preset_for_stage(stage).generation_length()?;
    }
    output::final_answer(&config.reasoning, "")?;
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(config.timeout_seconds))
        .build()?;
    let mut client = None;
    let analysis: String = request_parsed(
        &mut client,
        &http,
        config,
        Stage::Analyst,
        &prompt_data(state, config, Stage::Analyst)?,
        |answer| Ok(answer.to_owned()),
    )
    .await?;
    let mut grounder_data = prompt_data(state, config, Stage::Grounder)?;
    grounder_data.custom["analysis"] = json!(analysis);
    let grounded_analysis: String = request_parsed(
        &mut client,
        &http,
        config,
        Stage::Grounder,
        &grounder_data,
        |answer| Ok(answer.to_owned()),
    )
    .await?;
    let mut excluded: HashSet<String> = state.wrong_moves.iter().cloned().collect();
    for round in 1..=config.max_attempts {
        tracing::info!(round, "LLM candidate round");
        let mut data = prompt_data(state, config, Stage::Proposer)?;
        data.custom["analysis"] = json!(grounded_analysis);
        let mut sorted: Vec<_> = excluded.iter().cloned().collect();
        sorted.sort();
        data.custom["excluded-moves"] = json!(if sorted.is_empty() {
            String::new()
        } else {
            serde_json::to_string(&sorted)?
        });
        let proposal: Proposal = request(
            &mut client,
            &http,
            config,
            Stage::Proposer,
            &data,
            |p: &Proposal| {
                ensure!(
                    (3..=5).contains(&p.candidates.len()),
                    "expected 3-5 candidates"
                );
                let mut ids = HashSet::new();
                for c in &p.candidates {
                    bounded(&c.id, 32)?;
                    ensure!(ids.insert(&c.id), "duplicate candidate id");
                    move_text(&c.proposed_move, config)?;
                }
                Ok(())
            },
        )
        .await?;
        let mut seen = HashSet::new();
        let mut accepted = Vec::new();
        for c in proposal.candidates {
            let text = move_text(&c.proposed_move, config)?;
            if excluded.contains(&text) {
                continue;
            }
            let (_, mv) = match resolve_candidate(state, config, &c) {
                Ok(resolved) => resolved,
                Err(_) => {
                    excluded.insert(text);
                    continue;
                }
            };
            if validate_metadata(state, config, &c, mv).is_err() {
                continue;
            }
            if !seen.insert(mv) {
                continue;
            }
            tracing::info!(candidate_id = %c.id, "LLM independent verification");
            let mut data = prompt_data(state, config, Stage::Verifier)?;
            data.custom["candidate"] = json!(serde_json::to_string(&json!({
                "move": c.proposed_move, "from": c.from, "to": c.to
            }))?);
            let verification: Verification = request(
                &mut client,
                &http,
                config,
                Stage::Verifier,
                &data,
                Verification::validate,
            )
            .await?;
            if verification.verdict == Verdict::ACCEPT {
                accepted.push((c, verification.tactical_result));
            } else {
                excluded.insert(text);
            }
        }
        if accepted.is_empty() {
            continue;
        }
        let mut data = prompt_data(state, config, Stage::Selector)?;
        let survivors: Vec<_> = accepted
            .iter()
            .map(|(c, note)| {
                let mut v = candidate_input(c);
                v["tactical_result"] = json!(note);
                v
            })
            .collect();
        data.custom["accepted-candidates"] = json!(serde_json::to_string(&survivors)?);
        let selection: Selection = request(
            &mut client,
            &http,
            config,
            Stage::Selector,
            &data,
            |s: &Selection| {
                let (c, _) = accepted
                    .iter()
                    .find(|(c, _)| c.id == s.candidate_id)
                    .context("unknown accepted candidate")?;
                ensure!(
                    s.final_move == c.proposed_move,
                    "selector changed candidate move"
                );
                Ok(())
            },
        )
        .await?;
        let (selected, _) = accepted
            .iter()
            .find(|(c, _)| c.id == selection.candidate_id)
            .unwrap();
        let text = move_text(&selection.final_move, config)?;
        if let Ok((trial, mv)) = validate_candidate(state, config, selected) {
            *state = trial;
            return Ok(mv);
        }
        state.record_rejected_final(&text);
        excluded.insert(text.clone());
        rejected(round, &text);
    }
    bail!(
        "No legal selected move after {} candidate rounds. Retry to continue this unchanged position.",
        config.max_attempts
    )
}

/// Validate only the proposed move, on a disposable clone; never expose engine alternatives.
fn validate_candidate(
    state: &Match,
    config: &Config,
    candidate: &Candidate,
) -> Result<(Match, Move)> {
    let (trial, mv) = resolve_candidate(state, config, candidate)?;
    validate_metadata(state, config, candidate, mv)?;
    Ok((trial, mv))
}

fn resolve_candidate(
    state: &Match,
    config: &Config,
    candidate: &Candidate,
) -> Result<(Match, Move)> {
    let mut trial = state.clone();
    check_move(&candidate.proposed_move, state, config)?;
    let text = move_text(&candidate.proposed_move, config)?;
    let mv = trial.accept_configured_answer(&text, config)?;
    Ok((trial, mv))
}

fn validate_metadata(
    state: &Match,
    config: &Config,
    candidate: &Candidate,
    mv: Move,
) -> Result<()> {
    ensure!(
        mv.from == square(&candidate.from, state, config)?
            && mv.to == square(&candidate.to, state, config)?,
        "candidate coordinates differ from move"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legality_gate_handles_special_moves_and_king_safety_on_disposable_clones() {
        use capablanca_chess_plus::{Color, Variant};
        let config = Config::load_classic_with_profile(Some("no-history")).unwrap();
        for (fen, san, from, to, valid) in [
            ("4k3/8/8/8/8/8/8/4K2R w K - 0 1", "O-O", "e1", "g1", true),
            ("4k3/8/8/8/8/8/8/4K2R w - - 0 1", "O-O", "e1", "g1", false),
            (
                "4k3/8/8/3pP3/8/8/8/4K3 w - d6 0 1",
                "exd6",
                "e5",
                "d6",
                true,
            ),
            (
                "4k3/8/8/3pP3/8/8/8/4K3 w - - 0 1",
                "exd6",
                "e5",
                "d6",
                false,
            ),
            ("7k/P7/8/8/8/8/8/4K3 w - - 0 1", "a8=Q+", "a7", "a8", true),
            ("7k/P7/8/8/8/8/8/4K3 w - - 0 1", "a8", "a7", "a8", false),
            ("k3r3/8/8/8/8/8/4R3/4K3 w - - 0 1", "Rf2", "e2", "f2", false),
        ] {
            let state = Match::from_fen(Variant::Classic, Color::White, fen).unwrap();
            let candidate: Candidate =
                serde_json::from_value(json!({"id":"c1","move":san,"from":from,"to":to})).unwrap();
            assert_eq!(
                validate_candidate(&state, &config, &candidate).is_ok(),
                valid,
                "{san}: {fen}"
            );
            assert_eq!(state.game().position().to_fen(), fen);
            assert!(state.uci_history().is_empty());
            assert!(state.wrong_moves().is_empty());
        }
    }

    #[test]
    fn chat_preview_preserves_accepted_history_roles() {
        let config = Config::load_classic_with_profile(Some("with-history")).unwrap();
        let mut state = Match::new(
            capablanca_chess_plus::Variant::Classic,
            capablanca_chess_plus::Color::Black,
        );
        state.accept_human("e2e4").unwrap();
        state.accept_model("e7e5").unwrap();
        state.accept_human("g1f3").unwrap();
        let messages: Vec<prompt_core::ChatMessage> =
            serde_json::from_str(&preview(&state, &config).unwrap()).unwrap();
        use prompt_core::ChatRole;
        assert_eq!(messages[0].role, ChatRole::System);
        for (message, (role, content)) in messages[1..4].iter().zip([
            (ChatRole::User, "e4"),
            (ChatRole::Assistant, "e5"),
            (ChatRole::User, "Nf3"),
        ]) {
            assert_eq!(message.role, role);
            assert_eq!(message.content, content);
        }
        assert_eq!(messages.last().unwrap().role, ChatRole::User);
        assert!(
            messages
                .last()
                .unwrap()
                .content
                .contains(&state.game().position().to_fen())
        );
    }

    #[test]
    fn custom_stage_templates_are_used_verbatim_without_implicit_instructions() {
        for classic in [true, false] {
            let mut config = if classic {
                Config::load_classic_with_profile(Some("no-history"))
            } else {
                Config::load_with_profile(Some("numeric-no-history"))
            }
            .unwrap();
            let state = Match::new(
                if classic {
                    capablanca_chess_plus::Variant::Classic
                } else {
                    capablanca_chess_plus::Variant::Gothic
                },
                capablanca_chess_plus::Color::White,
            );
            for stage in [
                Stage::Analyst,
                Stage::Grounder,
                Stage::Proposer,
                Stage::Verifier,
                Stage::Selector,
            ] {
                let custom: SystemPromptTemplate = serde_json::from_value(json!({
                    "name":"experiment", "content":"CUSTOM {{#if classic}}SAN{{else}}NUMERIC{{/if}}",
                    "user_prompt":"SIDE {{side-to-move}}", "post_history":"ONLY MY POST"
                })).unwrap();
                let slot = match stage {
                    Stage::Analyst => &mut config.pipeline.analyst,
                    Stage::Grounder => &mut config.pipeline.grounder,
                    Stage::Proposer => &mut config.pipeline.proposer,
                    Stage::Verifier => &mut config.pipeline.verifier,
                    Stage::Selector => &mut config.pipeline.selector,
                };
                *slot = custom.clone();
                let actual = template(&config, stage);
                assert_eq!(
                    serde_json::to_value(&actual).unwrap(),
                    serde_json::to_value(&custom).unwrap()
                );
                let data = prompt_data(&state, &config, stage).unwrap();
                for key in [
                    "move-format",
                    "stage-rules",
                    "authoritative-state",
                    "output-instructions",
                    "representation-instructions",
                    "history-instructions",
                ] {
                    assert!(data.custom.get(key).is_none(), "hidden instruction {key}");
                }
                let rendered = PromptBuilder::new()
                    .build(&config.context, &config.instruct, &actual, &data)
                    .unwrap()
                    .text;
                assert!(rendered.contains(if classic {
                    "CUSTOM SAN"
                } else {
                    "CUSTOM NUMERIC"
                }));
                assert!(rendered.contains("SIDE White"));
                assert!(rendered.contains("ONLY MY POST"));
                for unexpected in [
                    "AUTHORITATIVE",
                    "FEN:",
                    "Orthodox",
                    "from and to",
                    "JSON",
                    "must be",
                    "CURRENT PIECES",
                ] {
                    assert!(
                        !rendered.contains(unexpected),
                        "injected {unexpected}: {rendered}"
                    );
                }
            }
        }
    }
}
