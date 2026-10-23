//! Stage templates and explicit context allowlists. No hidden chess instructions.
use super::*;
#[derive(Clone)]
pub struct Templates {
    pub one_grounder: SystemPromptTemplate,
    pub grounder: SystemPromptTemplate,
    pub pin_analyzer: SystemPromptTemplate,
    pub proposer: SystemPromptTemplate,
    pub verifier: SystemPromptTemplate,
    pub selector: SystemPromptTemplate,
}
impl Templates {
    pub(crate) fn load_for_variant(
        base: &Path,
        get: impl Fn(&str) -> Result<String>,
        classic: bool,
    ) -> Result<Self> {
        let read = |key: &str, name: &str| -> Result<SystemPromptTemplate> {
            let path = base.join(if classic {
                get(key).unwrap_or_else(|_| format!("templates/pipeline/{name}.json"))
            } else {
                format!("compat/fairy/templates/{name}.json")
            });
            serde_json::from_slice(
                &std::fs::read(&path).with_context(|| format!("cannot read {}", path.display()))?,
            )
            .with_context(|| format!("invalid stage template {}", path.display()))
        };
        Ok(Self {
            // Never used for fairy variants; no new fairy template required.
            one_grounder: if classic {
                read("LLM_ONE_GROUNDER_TEMPLATE", "one-grounder")?
            } else {
                read("LLM_GROUNDER_TEMPLATE", "grounder")?
            },
            grounder: read("LLM_GROUNDER_TEMPLATE", "grounder")?,
            pin_analyzer: if classic { read("LLM_PIN_ANALYZER_TEMPLATE", "pin-analyzer")? }
                          else { read("LLM_GROUNDER_TEMPLATE", "grounder")? },
            proposer: read("LLM_PROPOSER_TEMPLATE", "proposer")?,
            verifier: read("LLM_VERIFIER_TEMPLATE", "verifier")?,
            selector: read("LLM_SELECTOR_TEMPLATE", "selector")?,
        })
    }
}

/// Stage-local data allowlist: only explicitly referenced values enter a template.
pub fn prompt_data(state: &Match, config: &Config, stage: Stage) -> Result<PromptData> {
    let mut data = state.prompt_data_with(config)?;
    let original = data.custom.clone();
    let private = matches!(
        stage,
        Stage::OneGrounder
            | Stage::OneGrounderFull
            | Stage::Grounder
            | Stage::PinAnalyzer
            | Stage::Verifier
            | Stage::Selector
    );
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
        Stage::OneGrounder | Stage::OneGrounderFull => &config.pipeline.one_grounder,
        Stage::Grounder => &config.pipeline.grounder,
        Stage::PinAnalyzer => &config.pipeline.pin_analyzer,
        Stage::Proposer => &config.pipeline.proposer,
        Stage::Verifier => &config.pipeline.verifier,
        Stage::Selector => &config.pipeline.selector,
    }
    .clone()
}

pub fn preview(state: &Match, config: &Config) -> Result<String> {
    let stage = Stage::Grounder;
    let image = crate::board_image::BoardImage::new(state.game.position())?;
    Ok(serde_json::to_string_pretty(
        &build_request(config, stage, &request_data(state, config, stage)?, &image)?.messages,
    )?)
}

pub(super) fn build_request(
    config: &Config,
    stage: Stage,
    data: &PromptData,
    image: &crate::board_image::BoardImage,
) -> Result<prompt_core::ChatCompletionRequest> {
    let messages =
        PromptBuilder::new().build_chat(&config.context, &request_template(config, stage), data)?;
    let mut request: prompt_core::ChatCompletionRequest =
        ChatGenerationRequest::from_preset(messages, config.preset_for_stage(stage))?.into();
    image.attach(&mut request)?;
    request.image_token_reserve = config.image_token_reserve;
    Ok(request)
}

pub(super) fn request_template(config: &Config, stage: Stage) -> SystemPromptTemplate {
    template(config, stage)
}

fn request_data(state: &Match, config: &Config, stage: Stage) -> Result<PromptData> {
    prompt_data(state, config, stage)
}
