//! Chat Completion, bounded silent retries, reasoning stripping and protocol parsing.
use super::*;
use capablanca_chess_plus::{Position, Square};
use prompt_core::{ChatCompletionMessage, ToolCall, ToolChoice, ToolDefinition, ToolType, FunctionCall};
use serde::Deserialize;
use std::{collections::BTreeSet, sync::{Arc,Mutex}};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PieceAtArgs {
    square: String,
}

/// Read ONLY one requested square. Never compute attacks, rays or legal moves.
fn piece_at(position: &Position, call: &ToolCall) -> Value {
    let answer = (|| -> Result<Value> {
        ensure!(call.function.name == "piece_at", "unknown tool {}", call.function.name);
        let args: PieceAtArgs = call.arguments()?;
        let bytes=args.square.as_bytes();
        ensure!(bytes.len()==2 && (b'a'..=b'h').contains(&bytes[0])
            && (b'1'..=b'8').contains(&bytes[1]),
            "square must be a lowercase coordinate a1..h8");
        let square: Square=args.square.parse()?;
        let piece=position.board().piece_at(square)
            .map(|p| format!("{:?} {}", p.color, p.kind.display_name()));
        Ok(json!({"square":args.square,"piece":piece}))
    })();
    match answer {
        Ok(value)=>value,
        Err(error)=>json!({"error":error.to_string()}),
    }
}

fn occupancy_tool() -> ToolDefinition {
    ToolDefinition::new(
        "piece_at",
        "Inspect the occupant of exactly one square in the current position. Choose the square yourself; receive a human-readable piece name such as White Knight or Black Queen, or null for an empty square. Never returns move or ray information.",
        json!({
            "type":"object",
            "properties":{"square":{"type":"string","description":"One lowercase square, e.g. c3"}},
            "required":["square"],
            "additionalProperties":false
        }),
    )
}

pub(super) async fn request<T: DeserializeOwned>(
    client: &mut Option<KoboldClient>,
    http: &reqwest::Client,
    config: &Config,
    stage: Stage,
    data: &PromptData,
    image: &crate::board_image::BoardImage,
    validate: impl Fn(&T) -> Result<()>,
) -> Result<T> {
    request_parsed(client, http, config, stage, data, image, |answer| {
        let value = if matches!(stage, Stage::ThreatAnalyzer | Stage::Proposer | Stage::Selector | Stage::PinAnalyzer) {
            output::parse_final_json(answer)?
        } else {
            output::parse_json(answer)?
        };
        validate(&value)?;
        Ok(value)
    })
    .await
}

/// Classic Proposer receives read-only move simulation. Its final choice must
/// have been tried as the FIRST ply of a successful tool branch; the final SAN
/// is independently checked by the engine before reaching the caller.
pub(super) async fn request_proposer(
    client: &mut Option<KoboldClient>,
    http: &reqwest::Client,
    config: &Config,
    data: &PromptData,
    image: &crate::board_image::BoardImage,
    state: &Match,
) -> Result<FinalMove> {
    ensure!(config.classic.is_some(), "Classic Proposer tools require Classic chess");
    let inspected = Arc::new(Mutex::new(BTreeSet::<String>::new()));
    let validated = inspected.clone();
    request_parsed_inner(
        client,http,config,Stage::Proposer,data,image,None,
        Some((state, inspected)),
        move |answer| {
            let selected: FinalMove = output::parse_final_json(answer)?;
            ensure!(!selected.proposed_move.is_empty()
                    && selected.proposed_move.trim() == selected.proposed_move
                    && classic::san_shape(&selected.proposed_move),
                "final move must be canonical SAN");
            ensure!(validated.lock().unwrap().contains(&selected.proposed_move),
                "final move was not verified through inspect_line as the first ply");
            let mut trial = state.clone();
            trial.accept_configured_answer(&selected.proposed_move,config)
                .context("final move is illegal in the actual position")?;
            Ok(selected)
        }
    ).await
}

pub(super) async fn request_parsed<T>(
    client: &mut Option<KoboldClient>,
    http: &reqwest::Client,
    config: &Config,
    stage: Stage,
    data: &PromptData,
    image: &crate::board_image::BoardImage,
    parse: impl Fn(&str) -> Result<T>,
) -> Result<T> {
    request_parsed_inner(client,http,config,stage,data,image,None,None,parse).await
}

/// Grounder-specific variant that can answer model-initiated piece_at calls.
pub(super) async fn request_parsed_grounder<T>(
    client: &mut Option<KoboldClient>,
    http: &reqwest::Client,
    config: &Config,
    stage: Stage,
    data: &PromptData,
    image: &crate::board_image::BoardImage,
    position: &Position,
    parse: impl Fn(&str) -> Result<T>,
) -> Result<T> {
    ensure!(matches!(stage, Stage::OneGrounder | Stage::Grounder),
        "piece_at is only available to Grounder");
    if config.classic.is_none() {
        return request_parsed(client,http,config,stage,data,image,parse).await;
    }
    request_parsed_inner(client,http,config,stage,data,image,Some(position),None,parse).await
}

async fn request_parsed_inner<T>(
    client: &mut Option<KoboldClient>,
    http: &reqwest::Client,
    config: &Config,
    stage: Stage,
    data: &PromptData,
    image: &crate::board_image::BoardImage,
    position: Option<&Position>,
    proposer: Option<(&Match, Arc<Mutex<BTreeSet<String>>>)>,
    parse: impl Fn(&str) -> Result<T>,
) -> Result<T> {
    let mut request = super::prompts::build_request(config, stage, data, image)?;
    ensure!(position.is_none() || proposer.is_none(), "conflicting tool modes");
    if position.is_some() {
        request.tools=vec![occupancy_tool()];
        request.tool_choice=Some(ToolChoice::Auto);
    } else if proposer.is_some() {
        request.tools=vec![super::delta::tool()];
        request.tool_choice=Some(ToolChoice::Auto);
    }
    let reasoning = &config.reasoning;
    // Diagnostics never enter model context, history, or exclusions.
    let report_invalid = |attempt: usize, raw: &str, error: &anyhow::Error| {
        if matches!(stage, Stage::Grounder | Stage::OneGrounder | Stage::PinAnalyzer) && config.classic.is_some() {
            let label = match stage {
                Stage::OneGrounder => "OneGrounder",
                Stage::PinAnalyzer => "PinAnalyzer",
                _ => "Grounder",
            };
            eprintln!(
                "{label} attempt {attempt} rejected: {error:#}\n--- original model response ---\n{raw}\n--- end model response ---"
            );
        }
    };
    let mut last_error = None;
    for attempt in 1..=config.stage_max_attempts {
        // A protocol retry starts a new model transcript. Earlier inspected
        // moves must not authorize a final move without visible tool evidence.
        if let Some((_, checked)) = proposer.as_ref() {
            checked.lock().unwrap().clear();
        }
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
            if position.is_some() || proposer.is_some() {
                // No application-imposed limit on tool calls or rounds.
                // Context capacity and the backend's request timeout still apply.
                let mut conversation = request.clone();
                let mut model_called_tool = false;
                let mut auto_verified = false;
                loop {
                    let fitted = client.fit_chat(&conversation).await?;
                    // Never lose the authoritative board image while fitting a
                    // long series of exchanges into the finite model context.
                    ensure!(fitted.request.messages.iter().any(|m|m.image_count()>0),
                        "Tool context cannot retain the board image and transcript");
                    input_tokens=Some(fitted.preview.tokens);
                    let response=client.generate_chat_full(&fitted.request).await?;
                    if response.tool_calls.is_empty() {
                        let raw = response.content.context("Tool-enabled stage returned no final text")?;
                        // Thinking Qwen may generate a full answer without
                        // requesting any tool (tool_choice=auto). Do not waste
                        // that entire generation. Treat it as a provisional
                        // move, verify it, and return the geometry to the model.
                        // Only do this when the model never used tools, and at
                        // most once, so a broken backend cannot loop forever.
                        if let Some((state, checked)) = proposer.as_ref()
                            && !model_called_tool && !auto_verified
                        {
                            let suggested = output::final_json_answer(reasoning,&raw)
                                .ok().flatten()
                                .and_then(|text| output::parse_final_json::<FinalMove>(&text).ok());
                            if let Some(suggested) = suggested
                                && !checked.lock().unwrap().contains(&suggested.proposed_move)
                            {
                                auto_verified = true;
                                let check = ToolCall {
                                    id:"host-verify-final".into(),
                                    kind:ToolType::Function,
                                    function:FunctionCall {
                                        name:"inspect_line".into(),
                                        arguments:json!({"moves":[suggested.proposed_move.clone()]}).to_string(),
                                    },
                                };
                                let result = super::delta::inspect_line(state,&check);
                                if let Some(san) = result["steps"][0]["san"].as_str() {
                                    checked.lock().unwrap().insert(san.to_owned());
                                }
                                tracing::warn!(?stage,attempt,move_san=%suggested.proposed_move,
                                    status=%result["status"],
                                    "Proposer skipped inspect_line; automatically verified tentative final move");
                                conversation=fitted.request;
                                // Use a short assistant turn; do not copy 4096
                                // reasoning tokens into the next prompt.
                                conversation.messages.push(ChatCompletionMessage::assistant(raw));
                                conversation.messages.push(ChatCompletionMessage::user(
                                    format!(r#"Your previous final move was premature: it was not checked with inspect_line. Rust has now checked that exact first move on the ORIGINAL board. Treat this as tool evidence, not as an evaluation. VERIFIED RESULT: {result}
You must now reconsider the result and either call inspect_line for other candidates or return one final JSON {{"move":"<canonical SAN>"}}. Only a verified legal root move may be finalized."#)
                                ));
                                continue;
                            }
                        }
                        break Ok(raw);
                    }
                    model_called_tool = true;
                    tracing::debug!(?stage, count=response.tool_calls.len(),
                        "LLM tool calls");
                    conversation=fitted.request;
                    conversation.messages.push(response.message);
                    for call in response.tool_calls {
                        let result = if let Some(position) = position {
                            piece_at(position,&call)
                        } else {
                            let (state, checked) = proposer.as_ref().expect("tool mode");
                            let observation = super::delta::inspect_line(state,&call);
                            let first = if observation["status"] == "ok" {
                                observation["steps"][0]["san"].as_str()
                            } else {
                                observation["valid_prefix"][0]["san"].as_str()
                            };
                            if let Some(san) = first {
                                checked.lock().unwrap().insert(san.to_owned());
                            }
                            observation
                        };
                        tracing::debug!(?stage, tool=%call.function.name, observation=%result,
                            "LLM tool result");
                        conversation.messages.push(ChatCompletionMessage::tool(call.id,&result));
                    }
                }
            } else {
                let fitted=client.fit_chat(&request).await?;
                input_tokens=Some(fitted.preview.tokens);
                Ok(client.generate_chat(&fitted.request).await?.text)
            }
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
                tracing::warn!(?stage, attempt, error=%e, "LLM request failed");
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
        let answer = if matches!(stage, Stage::ThreatAnalyzer | Stage::Proposer | Stage::Selector | Stage::PinAnalyzer) {
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
            let error = anyhow::anyhow!(
                "empty or oversized final answer ({} bytes; maximum 32768)",
                answer.len()
            );
            report_invalid(attempt, &raw, &error);
            last_error = Some(error);
            continue;
        }
        match parse(&answer) {
            Ok(value) => {
                record("accepted_response");
                return Ok(value);
            }
            Err(e) => {
                record("invalid_protocol");
                tracing::warn!(?stage,attempt,error=%e,"LLM final answer rejected");
                report_invalid(attempt, &raw, &e);
                last_error = Some(e);
            }
        }
    }
    Err(last_error
        .unwrap_or_else(|| anyhow::anyhow!("incomplete reasoning or missing structured result")))
    .with_context(|| format!("{stage:?} exhausted {} attempts", config.stage_max_attempts))
}
