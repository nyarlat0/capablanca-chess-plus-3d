//! Stage templates and explicit context allowlists. No hidden chess instructions.
use super::*;
#[derive(Clone)]
pub struct Templates {
    pub one_grounder: SystemPromptTemplate,
    pub grounder: SystemPromptTemplate,
    pub threat_analyzer: SystemPromptTemplate,
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
            threat_analyzer: if classic {
                read("LLM_THREAT_ANALYZER_TEMPLATE", "threat-analyzer")?
            } else {
                // Fairy variants do not schedule this stage.
                read("LLM_GROUNDER_TEMPLATE", "grounder")?
            },
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
            | Stage::ThreatAnalyzer
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
        Stage::ThreatAnalyzer => &config.pipeline.threat_analyzer,
        Stage::PinAnalyzer => &config.pipeline.pin_analyzer,
        Stage::Proposer => &config.pipeline.proposer,
        Stage::Verifier => &config.pipeline.verifier,
        Stage::Selector => &config.pipeline.selector,
    }
    .clone()
}

pub fn preview(state: &Match, config: &Config) -> Result<String> {
    let stage = Stage::Grounder;
    let image = crate::board_image::BoardImage::with_enabled(state.game.position(),config.send_board_image)?;
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
    request.image_token_reserve = if config.send_board_image {config.image_token_reserve} else {0};
    // Sampling/reasoning options belong to the selected stage preset.
    // In particular, never override chat_template_kwargs.enable_thinking.
    // Presets already put their kwargs inside request.parameters, and
    // prompt-core transfers them to the Chat Completion payload.

    Ok(request)
}

pub(super) fn request_template(config: &Config, stage: Stage) -> SystemPromptTemplate {
    template(config, stage)
}

fn request_data(state: &Match, config: &Config, stage: Stage) -> Result<PromptData> {
    prompt_data(state, config, stage)
}

#[cfg(test)]
mod stage_reasoning_preset_tests {
    use super::*;
    use capablanca_chess_plus::{Color, Variant};
    use prompt_core::Preset;

    fn request_for(config:&Config, stage:Stage) -> prompt_core::ChatCompletionRequest {
        let state=Match::new(Variant::Classic,Color::Black);
        let image=crate::board_image::BoardImage::with_enabled(
            state.game().position(),false).unwrap();
        build_request(config,stage,&prompt_data(&state,config,stage).unwrap(),&image)
            .unwrap()
    }

    #[test]
    fn proposer_thinking_true_is_preserved() {
        let mut config=Config::load_classic_with_profile(Some("no-history")).unwrap();
        config.send_board_image=false;
        config.stage_presets.proposer=Some(Preset::from_value(json!({
            "genamt":128,
            "chat_template_kwargs":{"enable_thinking":true,"reasoning_effort":"high"}
        })).unwrap());
        let req=request_for(&config,Stage::Proposer);
        assert_eq!(req.parameters["chat_template_kwargs"]["enable_thinking"],true);
        assert_eq!(req.parameters["chat_template_kwargs"]["reasoning_effort"],"high");
        assert!(req.chat_template_kwargs.is_empty());
    }

    #[test]
    fn proposer_thinking_false_is_preserved() {
        let mut config=Config::load_classic_with_profile(Some("no-history")).unwrap();
        config.send_board_image=false;
        config.stage_presets.proposer=Some(Preset::from_value(json!({
            "genamt":128,
            "chat_template_kwargs":{"enable_thinking":false}
        })).unwrap());
        let req=request_for(&config,Stage::Proposer);
        assert_eq!(req.parameters["chat_template_kwargs"]["enable_thinking"],false);
        assert!(req.chat_template_kwargs.is_empty());
    }

    #[test]
    fn selector_does_not_gain_unrequested_thinking_override() {
        let mut config=Config::load_classic_with_profile(Some("no-history")).unwrap();
        config.send_board_image=false;
        config.stage_presets.selector=Some(Preset::from_value(json!({
            "genamt":128
        })).unwrap());
        let req=request_for(&config,Stage::Selector);
        assert!(req.parameters.get("chat_template_kwargs").is_none());
        assert!(req.chat_template_kwargs.is_empty());
        config.stage_presets.selector=Some(Preset::from_value(json!({
            "genamt":128,
            "chat_template_kwargs":{"enable_thinking":true}
        })).unwrap());
        assert_eq!(request_for(&config,Stage::Selector).parameters
            ["chat_template_kwargs"]["enable_thinking"],true);
    }

    #[test]
    fn real_proposer_preset_thinking_is_unchanged() {
        let mut config=Config::load_classic_with_profile(Some("no-history")).unwrap();
        config.send_board_image=false;
        let expected=config.preset_for_stage(Stage::Proposer)
            .raw()["chat_template_kwargs"].clone();
        let req=request_for(&config,Stage::Proposer);
        assert_eq!(req.parameters["chat_template_kwargs"],expected);
    }
}
