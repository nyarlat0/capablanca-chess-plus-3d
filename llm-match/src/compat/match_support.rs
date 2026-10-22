//! Representation-specific matching and prompt rendering, kept out of Classic.
use crate::*;
use serde_json::json;

impl Match {
    /// Final answer only, with explicit profile decoding. Feedback preserves the
    /// model's representation, never substitutes internal UCI in numeric mode.
    pub fn accept_answer(&mut self, answer: &str, format: OutputFormat) -> Result<Move> {
        ensure!(
            self.game.position().side_to_move() == self.model_side,
            "not the model turn"
        );
        let answer = output::trailing_move(answer, |s| {
            parse_model_move(s, format, self.game.position()).is_ok()
        })
        .context("expected final move in configured format")?;
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
    pub(crate) fn render_prompt(
        &self,
        profile: &Representation,
        template: &str,
    ) -> Result<PromptData> {
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
        let mut full_profile = profile.clone();
        full_profile.history_mode = HistoryMode::Full;
        let full_history = render_history(
            &self.history,
            &self.numeric_history,
            &self.uci_history,
            &self.json_history,
            &full_profile,
        );
        let last_move = full_history
            .last()
            .map(|m| m.content.as_str())
            .unwrap_or("");
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
                "side-to-move": match self.game.position().side_to_move() { Color::White => "White", Color::Black => "Black" },
                "chess-rules": render_rules(self.variant, profile, template)?,
                "last-move": last_move,
                "board-state": board,
                "piece-list": prompts::piece_list(self.game.position()),
                "piece-list-num": prompts::piece_list_num(self.game.position()),
                "wrong-move": wrong,
                "representation-instructions": representation::grounding(profile.state_format),
                "output-instructions": representation::output_instruction(profile.output_format),
                "history-instructions": history_instruction,
            }),
            ..Default::default()
        })
    }
}
