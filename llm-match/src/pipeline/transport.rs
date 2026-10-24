//! Chat Completion, bounded silent retries, reasoning stripping and protocol parsing.
use super::*;
use capablanca_chess_plus::{Position, Square};
use prompt_core::{ChatCompletionMessage, ToolCall, ToolChoice, ToolDefinition};
use serde::Deserialize;

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
            .map(|p| crate::grounder::piece_id(position,&args.square,p));
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
        "Inspect the occupant of exactly one square in the current position. Choose the square yourself; receive a piece ID or null, never move or ray information.",
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
        let value = if matches!(stage, Stage::Proposer | Stage::Selector | Stage::PinAnalyzer) {
            output::parse_final_json(answer)?
        } else {
            output::parse_json(answer)?
        };
        validate(&value)?;
        Ok(value)
    })
    .await
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
    request_parsed_inner(client,http,config,stage,data,image,None,parse).await
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
    request_parsed_inner(client,http,config,stage,data,image,Some(position),parse).await
}

async fn request_parsed_inner<T>(
    client: &mut Option<KoboldClient>,
    http: &reqwest::Client,
    config: &Config,
    stage: Stage,
    data: &PromptData,
    image: &crate::board_image::BoardImage,
    position: Option<&Position>,
    parse: impl Fn(&str) -> Result<T>,
) -> Result<T> {
    let mut request = super::prompts::build_request(config, stage, data, image)?;
    if position.is_some() {
        request.tools=vec![occupancy_tool()];
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
            if let Some(position) = position {
                // No application-imposed limit on tool calls or rounds.
                // Context capacity and the backend's request timeout still apply.
                let mut conversation = request.clone();
                loop {
                    let fitted = client.fit_chat(&conversation).await?;
                    // Never lose the authoritative board image while fitting a
                    // long series of exchanges into the finite model context.
                    ensure!(fitted.request.messages.iter().any(|m|m.image_count()>0),
                        "Grounder context cannot retain the board image and tool transcript");
                    input_tokens=Some(fitted.preview.tokens);
                    let response=client.generate_chat_full(&fitted.request).await?;
                    if response.tool_calls.is_empty() {
                        break Ok(response.content.context("Grounder returned no final text")?);
                    }
                    tracing::debug!(?stage, count=response.tool_calls.len(),
                        "Grounder tool calls");
                    conversation=fitted.request;
                    conversation.messages.push(response.message);
                    for call in response.tool_calls {
                        let result=piece_at(position,&call);
                        tracing::debug!(?stage, tool=%call.function.name, observation=%result,
                            "Grounder occupancy lookup");
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
        let answer = if matches!(stage, Stage::Proposer | Stage::Selector | Stage::PinAnalyzer) {
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
                report_invalid(attempt, &raw, &e);
                last_error = Some(e);
            }
        }
    }
    Err(last_error
        .unwrap_or_else(|| anyhow::anyhow!("incomplete reasoning or missing structured result")))
    .with_context(|| format!("{stage:?} exhausted {} attempts", config.stage_max_attempts))
}
