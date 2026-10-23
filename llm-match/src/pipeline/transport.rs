//! Chat Completion, bounded silent retries, reasoning stripping and protocol parsing.
use super::*;
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

pub(super) async fn request_parsed<T>(
    client: &mut Option<KoboldClient>,
    http: &reqwest::Client,
    config: &Config,
    stage: Stage,
    data: &PromptData,
    image: &crate::board_image::BoardImage,
    parse: impl Fn(&str) -> Result<T>,
) -> Result<T> {
    let request = super::prompts::build_request(config, stage, data, image)?;
    let reasoning = &config.reasoning;
    // Diagnostics never enter model context, history, or exclusions.
    let report_invalid = |attempt: usize, raw: &str, error: &anyhow::Error| {
        if matches!(stage, Stage::Grounder | Stage::OneGrounder) && config.classic.is_some() {
            let label = if stage == Stage::OneGrounder {
                "OneGrounder"
            } else {
                "Grounder"
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
            let fitted = client.fit_chat(&request).await?;
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
