mod classic;
mod config;
pub use classic::ClassicConfig;
mod move_text;
mod output;
mod profile;
mod prompts;
mod representation;
mod rules;
pub use profile::{
    HistoryFormat, HistoryMode, OutputFormat, ProfileFile, Representation, StateFormat, cli_profile,
};
pub use representation::{
    numeric_square, parse_model_move, render_history, render_state, square_from_numeric,
};
pub use rules::{render_rules, variant_key};
#[cfg(test)]
mod classic_tests;
#[cfg(test)]
mod representation_tests;
#[cfg(test)]
mod tests;

use anyhow::{Context, Result, bail, ensure};
use capablanca_chess_plus::{Color, Game, GameOutcome, Move, Variant};
pub use config::{Config, normalize_endpoint};
use prompt_core::{KoboldClient, PromptBuilder, PromptData, PromptMessage, PromptRole};
pub use prompts::{board_state, chess_rules};
use serde_json::json;
use std::time::Duration;

/// Canonical history for synchronization; semantic accepted moves for prompting.
#[derive(Clone)]
pub struct Match {
    game: Game,
    variant: Variant,
    model_side: Color,
    history: Vec<PromptMessage>,
    numeric_history: Vec<PromptMessage>,
    json_history: Vec<PromptMessage>,
    san_history: Vec<classic::SanPly>,
    uci_history: Vec<String>,
    wrong_moves: Vec<String>,
}

impl Match {
    pub fn new(variant: Variant, model_side: Color) -> Self {
        Self {
            game: Game::new(variant.starting_position()),
            variant,
            model_side,
            history: Vec::new(),
            numeric_history: Vec::new(),
            json_history: Vec::new(),
            san_history: Vec::new(),
            uci_history: Vec::new(),
            wrong_moves: Vec::new(),
        }
    }
    pub fn game(&self) -> &Game {
        &self.game
    }
    pub fn history(&self) -> &[PromptMessage] {
        &self.history
    }
    pub fn uci_history(&self) -> &[String] {
        &self.uci_history
    }
    pub fn numeric_history(&self) -> &[PromptMessage] {
        &self.numeric_history
    }
    pub fn wrong_moves(&self) -> &[String] {
        &self.wrong_moves
    }
    pub fn classic_san_history(&self) -> String {
        classic::history(&self.san_history)
    }

    pub fn accept_configured_answer(&mut self, answer: &str, config: &Config) -> Result<Move> {
        if self.variant != Variant::Classic {
            ensure!(
                config.classic.is_none(),
                "classic configuration is only for classic chess"
            );
            return self.accept_answer(answer, config.representation.output_format);
        }
        ensure!(
            config.classic.is_some(),
            "classic chess requires its separate configuration"
        );
        ensure!(
            self.game.position().side_to_move() == self.model_side,
            "not the model turn"
        );
        let answer = answer.trim();
        ensure!(classic::san_shape(answer), "expected one SAN move");
        let result = self
            .game
            .position()
            .parse_san_move(answer)
            .map_err(anyhow::Error::from)
            .and_then(|mv| {
                tracing::debug!("model answer: {answer}; decoded move: {}", mv.to_uci());
                self.accept(&mv.to_uci(), PromptRole::Assistant)
            });
        if result.is_err() && !self.wrong_moves.iter().any(|m| m == answer) {
            self.wrong_moves.push(answer.into());
        }
        result
    }

    pub fn accept_human(&mut self, text: &str) -> Result<Move> {
        ensure!(
            self.game.position().side_to_move() != self.model_side,
            "not the human turn"
        );
        self.accept(text, PromptRole::User)
    }

    /// No extraction from prose, case conversion or SAN fallback.
    pub fn accept_model(&mut self, text: &str) -> Result<Move> {
        ensure!(
            self.game.position().side_to_move() == self.model_side,
            "not the model turn"
        );
        match self.accept(text, PromptRole::Assistant) {
            Ok(chess_move) => Ok(chess_move),
            Err(error) => {
                let text = text.trim().to_owned();
                if output::is_coordinate_move(&text) && !self.wrong_moves.contains(&text) {
                    self.wrong_moves.push(text);
                }
                Err(error)
            }
        }
    }
    /// Final answer only, with explicit profile decoding. Feedback preserves the
    /// model's representation, never substitutes internal UCI in numeric mode.
    pub fn accept_answer(&mut self, answer: &str, format: OutputFormat) -> Result<Move> {
        ensure!(
            self.game.position().side_to_move() == self.model_side,
            "not the model turn"
        );
        let (decoded, promotion) =
            representation::decode_model_move(answer, format, self.game.position())?;
        tracing::debug!("model answer: {}", answer.trim());
        tracing::debug!("decoded move: {decoded}");
        let accepted = (|| {
            if matches!(format, OutputFormat::Semantic | OutputFormat::Numeric) {
                let mv = self.game.position().parse_uci_move(&decoded)?;
                let expected = if format == OutputFormat::Semantic {
                    prompts::describe_move(self.game.position(), mv)
                } else {
                    representation::numeric_move(self.game.position(), mv, PromptRole::Assistant)
                        .content
                };
                ensure!(
                    answer.trim() == expected,
                    "answer does not match the exact pre-move description"
                );
            }
            self.accept_exact(&decoded, PromptRole::Assistant, promotion)
        })();
        match accepted {
            Ok(mv) => Ok(mv),
            Err(error) => {
                let answer = answer.trim().to_owned();
                if !self.wrong_moves.contains(&answer) {
                    self.wrong_moves.push(answer);
                }
                Err(error)
            }
        }
    }
    fn accept(&mut self, text: &str, role: PromptRole) -> Result<Move> {
        self.accept_exact(text, role, None)
    }
    fn accept_exact(
        &mut self,
        text: &str,
        role: PromptRole,
        promotion: Option<Option<capablanca_chess_plus::PieceKind>>,
    ) -> Result<Move> {
        ensure!(
            matches!(
                self.game.outcome(),
                GameOutcome::Ongoing | GameOutcome::Check
            ),
            "game is over"
        );
        let text = text.trim();
        let chess_move = self.game.position().parse_uci_move(text)?;
        ensure!(
            chess_move.to_uci() == text,
            "expected exactly one canonical UCI move"
        );
        if let Some(expected) = promotion {
            ensure!(
                chess_move.promotion == expected,
                "promotion identity differs from requested piece"
            );
        }
        let description = prompts::describe_move(self.game.position(), chess_move);
        let san = if self.variant == Variant::Classic {
            Some(classic::SanPly {
                number: self.game.position().fullmove_number(),
                side: self.game.position().side_to_move(),
                san: self.game.position().san(chess_move)?,
            })
        } else {
            None
        };
        let numeric = representation::numeric_move(self.game.position(), chess_move, role.clone());
        self.game.play(chess_move)?;
        if let Some(san) = san {
            self.san_history.push(san);
        }
        self.uci_history.push(chess_move.to_uci());
        self.numeric_history.push(numeric);
        self.json_history.push(PromptMessage {
            role: role.clone(),
            name: None,
            content: representation::json_move(chess_move),
        });
        self.history.push(PromptMessage {
            role,
            name: None,
            content: description,
        });
        self.wrong_moves.clear();
        Ok(chess_move)
    }
    pub fn prompt_data(&self) -> PromptData {
        self.render_prompt(
            &Representation::default(),
            rules::builtin_template(self.variant),
        )
        .expect("built-in profile")
    }
    pub fn prompt_data_with(&self, config: &Config) -> Result<PromptData> {
        if self.variant == Variant::Classic {
            let c = config
                .classic
                .as_ref()
                .context("classic chess requires its separate configuration")?;
            let wrong = if self.wrong_moves.is_empty() {
                String::new()
            } else {
                format!(
                    "Rejected moves: {}. Position unchanged. Do not repeat them.",
                    serde_json::to_string(&self.wrong_moves)?
                )
            };
            return Ok(classic::prompt(
                self.game.position(),
                &self.san_history,
                &wrong,
                c,
                self.model_side,
            ));
        }
        ensure!(
            config.classic.is_none(),
            "classic configuration is only for classic chess"
        );
        let template = config
            .rules_templates
            .get(variant_key(self.variant))
            .ok_or_else(|| anyhow::anyhow!("missing rules for {}", variant_key(self.variant)))?;
        self.render_prompt(&config.representation, template)
    }
    fn render_prompt(&self, profile: &Representation, template: &str) -> Result<PromptData> {
        profile.validate()?;
        let wrong = if self.wrong_moves.is_empty() {
            String::new()
        } else {
            format!(
                "Rejected moves: {}. Position unchanged. Do not repeat them.",
                serde_json::to_string(&self.wrong_moves).expect("strings serialize")
            )
        };
        let messages = render_history(
            &self.history,
            &self.numeric_history,
            &self.uci_history,
            &self.json_history,
            profile,
        );
        let history_instruction = representation::history_instruction(profile, messages.len());
        let mut board = render_state(self.game.position(), profile.state_format);
        use std::fmt::Write;
        writeln!(
            board,
            "\nCurrent position occurrences: {}",
            self.game.current_repetition_count()
        )
        .unwrap();
        Ok(PromptData {
            user: "Human".into(),
            character: "Chess model".into(),
            messages,
            custom: json!({
                "chess-rules": render_rules(self.variant, profile, template)?,
                "board-state": board,
                "wrong-move": wrong,
                "representation-instructions": representation::grounding(profile.state_format),
                "output-instructions": representation::output_instruction(profile.output_format),
                "history-instructions": history_instruction,
            }),
            ..Default::default()
        })
    }
}

/// Rejected attempts survive a request error so a manual retry can continue.
pub async fn generate_move(
    state: &mut Match,
    config: &Config,
    mut progress: impl FnMut(usize, &str),
) -> Result<Move> {
    ensure!(
        state.game.position().side_to_move() == state.model_side,
        "not the model turn"
    );
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(config.timeout_seconds))
        .build()?;
    ensure!(config.max_attempts > 0, "max_attempts must be positive");
    config.preset.generation_length()?;
    // Validate deterministic configuration before consuming any attempts.
    output::final_answer(&config.reasoning, "")?;
    let endpoint = normalize_endpoint(&config.endpoint)?;
    let mut client = None;
    let builder = PromptBuilder::new();
    let mut system = config.system.clone();
    if config.classic.is_none() && !system.content.contains("{{chess-rules}}") {
        system.content.push_str("\n{{chess-rules}}");
    }
    let mut last_transport_error = None;
    for attempt in 1..=config.max_attempts {
        let result: Result<String> = async {
            if client.is_none() {
                client =
                    Some(KoboldClient::connect_with_client(endpoint.clone(), http.clone()).await?);
            }
            let client = client.as_ref().unwrap();
            let data = state.prompt_data_with(config)?;
            let fitted = builder
                .build_with_budget(
                    &config.context,
                    &config.instruct,
                    &system,
                    &data,
                    client,
                    client.context_budget(&config.preset)?,
                ).await?;
            if fitted.dropped_messages > 0 {
                tracing::debug!("LLM context fitting: dropped {} oldest history messages from this request only", fitted.dropped_messages);
            }
            let prompt = fitted.prompt;
            client
                .generate(
                    &prompt.text,
                    &config.preset,
                    &prompt.stop_sequences,
                )
                .await
        }
        .await;
        let raw = match result {
            Ok(raw) => {
                last_transport_error = None;
                raw
            }
            Err(error) if output::transient(&error) => {
                last_transport_error = Some(error);
                if attempt < config.max_attempts {
                    tokio::time::sleep(Duration::from_millis(100 * (1u64 << (attempt - 1).min(4))))
                        .await;
                }
                continue;
            }
            Err(error) => return Err(error),
        };
        let Some(answer) = output::final_answer(&config.reasoning, &raw)? else {
            continue;
        };
        if if config.classic.is_some() {
            !classic::san_shape(&answer)
        } else {
            parse_model_move(
                &answer,
                config.representation.output_format,
                state.game.position(),
            )
            .is_err()
        } {
            continue;
        }
        match state.accept_configured_answer(&answer, config) {
            Ok(chess_move) => return Ok(chess_move),
            Err(_) => progress(attempt, &answer),
        }
    }
    if let Some(error) = last_transport_error {
        return Err(error.context(format!(
            "KoboldCPP unavailable after {} attempts",
            config.max_attempts
        )));
    }
    bail!(
        "No legal final move after {} attempts. Retry to continue this unchanged position.",
        config.max_attempts
    )
}
