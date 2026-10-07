mod config;
mod output;
mod prompts;
#[cfg(test)]
mod tests;

use anyhow::{Result, bail, ensure};
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
    pub fn wrong_moves(&self) -> &[String] {
        &self.wrong_moves
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
    fn accept(&mut self, text: &str, role: PromptRole) -> Result<Move> {
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
        let description = prompts::describe_move(self.game.position(), chess_move);
        self.game.play(chess_move)?;
        self.uci_history.push(chess_move.to_uci());
        self.history.push(PromptMessage {
            role,
            name: None,
            content: description,
        });
        self.wrong_moves.clear();
        Ok(chess_move)
    }
    pub fn prompt_data(&self) -> PromptData {
        let wrong = if self.wrong_moves.is_empty() {
            String::new()
        } else {
            format!(
                "Rejected moves: {}. Position unchanged. Do not repeat them.",
                serde_json::to_string(&self.wrong_moves).expect("strings serialize")
            )
        };
        PromptData {
            user: "Human".into(),
            character: "Chess model".into(),
            messages: self.history.clone(),
            custom: json!({
                "chess-rules": chess_rules(self.variant),
                "board-state": board_state(self.game.position()),
                "wrong-move": wrong,
            }),
            ..Default::default()
        }
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
    if !system.content.contains("{{chess-rules}}") {
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
            let fitted = builder
                .build_with_budget(
                    &config.context,
                    &config.instruct,
                    &system,
                    &state.prompt_data(),
                    client,
                    client.context_budget(&config.preset)?,
                )
                .await?;
            client
                .generate(
                    &fitted.prompt.text,
                    &config.preset,
                    &fitted.prompt.stop_sequences,
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
        if !output::is_coordinate_move(&answer) {
            continue;
        }
        match state.accept_model(&answer) {
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
