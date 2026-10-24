pub mod board_image;
mod classic;
mod config;
pub mod grounder;
pub mod pins;
pub mod pipeline;
pub use classic::ClassicConfig;
mod compat;
use compat::{move_text, profile, representation, rules};
mod output;
mod prompts;
pub use profile::{
    HistoryFormat, HistoryMode, OutputFormat, ProfileFile, Representation, StateFormat, cli_profile,
};
pub use representation::{
    numeric_square, parse_model_move, render_history, render_state, square_from_numeric,
};
pub use rules::{render_rules, variant_key};
#[cfg(test)]
mod classic_tests;

use anyhow::{Context, Result, ensure};
use capablanca_chess_plus::{Color, Game, GameOutcome, Move, Variant};
pub use config::{Config, StagePresets, normalize_endpoint};
#[cfg(test)]
use prompt_core::PromptBuilder;
#[cfg(test)]
fn render_test_chat(
    context: &prompt_core::ContextTemplate,
    system: &prompt_core::SystemPromptTemplate,
    data: &prompt_core::PromptData,
) -> prompt_core::BuiltPrompt {
    let messages = PromptBuilder::new()
        .build_chat(context, system, data)
        .unwrap();
    prompt_core::BuiltPrompt {
        text: messages
            .iter()
            .map(|message| message.content.as_str())
            .collect::<Vec<_>>()
            .join("\n"),
        stop_sequences: Vec::new(),
    }
}
use prompt_core::{PromptData, PromptMessage, PromptRole};
pub use prompts::{board_state, chess_rules};

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
    controls: grounder::Cache,
    pub(crate) auto_grounder_updates: bool,
}

impl Match {
    /// Start at an engine-parsed position. FEN does not contain prior move history.
    pub fn from_fen(variant: Variant, model_side: Color, fen: &str) -> Result<Self> {
        let position = capablanca_chess_plus::Position::from_fen(variant.rules(), fen)?;
        let mut state = Self::new(variant, model_side);
        state.game = Game::new(position);
        state.controls = grounder::Cache::new(state.game.position());
        Ok(state)
    }

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
            controls: grounder::Cache::new(&variant.starting_position()),
            auto_grounder_updates: false,
        }
    }
    pub fn game(&self) -> &Game {
        &self.game
    }
    /// Engine-calculate every simulated Grounder map (fast integration testing).
    pub fn set_auto_grounder_updates(&mut self, enabled: bool) {
        self.auto_grounder_updates = enabled;
    }
    /// Run a complete pipeline against the side to move, without relying
    /// on the interactive CLI's human/model color convention.
    pub fn set_model_side_to_move(&mut self) {
        self.model_side = self.game.position().side_to_move();
    }

    /// Initial Classic map: engine by default, or sequential one-grounder requests.
    /// Set before accepting the first move.
    pub fn set_auto_initial_grounder(&mut self, enabled: bool) {
        self.controls.auto_initial = enabled;
    }

    /// Position experiments seed before the last two accepted halfmoves.
    /// With auto=true the entire current map is computed without LLM calls.
    pub fn prepare_grounder_test(&mut self, auto: bool) {
        self.controls.test_start = Some(if auto {
            self.uci_history.len()
        } else {
            self.uci_history.len().saturating_sub(2)
        });
        self.controls.complete = None;
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
    /// Record an already parsed final rejection without replaying it into the board.
    pub fn record_rejected_final(&mut self, answer: &str) {
        if !self.wrong_moves.iter().any(|m| m == answer) {
            self.wrong_moves.push(answer.into());
        }
    }
    pub fn wrong_moves(&self) -> &[String] {
        &self.wrong_moves
    }
    pub fn classic_san_history(&self) -> String {
        classic::history(&self.san_history)
    }

    pub fn accept_configured_answer(&mut self, answer: &str, config: &Config) -> Result<Move> {
        let final_text =
            output::final_answer(&config.reasoning, answer)?.context("incomplete reasoning")?;
        let answer = final_text.as_str();
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
        let answer =
            output::trailing_move(answer, classic::san_shape).context("expected final SAN move")?;
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
        let numeric = representation::numeric_move(self.game.position(), chess_move, role);
        self.game.play(chess_move)?;
        if self.variant == Variant::Classic {
            self.controls.positions.push(std::sync::Arc::new(self.game.position().clone()));
        }
        if let Some(san) = san {
            self.san_history.push(san);
        }
        self.uci_history.push(chess_move.to_uci());
        self.numeric_history.push(numeric);
        self.json_history.push(PromptMessage {
            role,
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
}

/// Four independent roles; only the final engine-approved move is committed.
pub async fn generate_move(
    state: &mut Match,
    config: &Config,
    progress: impl FnMut(usize, &str),
) -> Result<Move> {
    pipeline::generate(state, config, progress).await
}
