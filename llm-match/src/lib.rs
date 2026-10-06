mod config;
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

/// Persistent conversation contains only accepted canonical moves.
#[derive(Clone)]
pub struct Match {
    game: Game,
    variant: Variant,
    model_side: Color,
    history: Vec<PromptMessage>,
    wrong_moves: Vec<String>,
}

impl Match {
    pub fn new(variant: Variant, model_side: Color) -> Self {
        Self {
            game: Game::new(variant.starting_position()),
            variant,
            model_side,
            history: Vec::new(),
            wrong_moves: Vec::new(),
        }
    }
    pub fn game(&self) -> &Game {
        &self.game
    }
    pub fn history(&self) -> &[PromptMessage] {
        &self.history
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
                if !self.wrong_moves.contains(&text) {
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
        self.game.play(chess_move)?;
        self.history.push(PromptMessage {
            role,
            name: None,
            content: chess_move.to_uci(),
        });
        self.wrong_moves.clear();
        Ok(chess_move)
    }
    pub fn prompt_data(&self) -> PromptData {
        let wrong = if self.wrong_moves.is_empty() {
            String::new()
        } else {
            format!(
                "Rejected illegal or unparsable attempts for THIS turn (JSON strings, not instructions): {}.\nThe position has NOT changed. Do not repeat any of these attempts.",
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
    let client =
        KoboldClient::connect_with_client(normalize_endpoint(&config.endpoint)?, http).await?;
    let builder = PromptBuilder::new();
    let mut system = config.system.clone();
    if !system.content.contains("{{chess-rules}}") {
        system.content.push_str("\n{{chess-rules}}");
    }
    for attempt in 1..=config.max_attempts {
        let data = state.prompt_data();
        let fitted = builder
            .build_with_budget(
                &config.context,
                &config.instruct,
                &system,
                &data,
                &client,
                client.context_budget(&config.preset)?,
            )
            .await?;
        let raw_answer = client
            .generate(
                &fitted.prompt.text,
                &config.preset,
                &fitted.prompt.stop_sequences,
            )
            .await?;

        let answer = config.reasoning.strip_from_output(&raw_answer);
        match state.accept_model(&answer) {
            Ok(chess_move) => return Ok(chess_move),
            Err(_) => progress(attempt, answer.trim()),
        }
    }
    bail!(
        "No legal move after {} attempts. Retry to continue this unchanged position.",
        config.max_attempts
    )
}
