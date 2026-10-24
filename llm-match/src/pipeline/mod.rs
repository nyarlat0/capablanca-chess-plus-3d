//! Independent, bounded LLM stages. No engine candidate generation is used here.
mod legality;
mod prompts;
mod protocol;
use legality::{move_text, resolve_candidate, validate_candidate, validate_metadata};
#[cfg(test)]
pub(crate) use prompts::template;
pub use prompts::{Templates, preview, prompt_data};
mod transport;
use transport::{request, request_parsed, request_parsed_grounder};
pub mod debug;
mod incremental;
pub use incremental::prepare_controls;
use crate::{Config, Match, OutputFormat, StateFormat, classic, output};
use anyhow::{Context, Result, bail, ensure};
use capablanca_chess_plus::{Move, Square};
use prompt_core::{
    ChatGenerationRequest, KoboldClient, PromptBuilder, PromptData, SystemPromptTemplate,
};
use protocol::*;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::{collections::HashSet, path::Path, time::Duration};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    /// Debug only; never scheduled by generate().
    OneGrounder,
    /// Debug-only sequential collection of every piece's controls.
    OneGrounderFull,
    Grounder,
    /// Debug-only, consumes a verified existing control map.
    PinAnalyzer,
    Proposer,
    Verifier,
    Selector,
}

impl std::str::FromStr for Stage {
    type Err = anyhow::Error;
    fn from_str(value: &str) -> Result<Self> {
        match value {
            "one-grounder" => Ok(Self::OneGrounder),
            "one-grounder-full" => Ok(Self::OneGrounderFull),
            "grounder" => Ok(Self::Grounder),
            "pin-analyzer" => Ok(Self::PinAnalyzer),
            "proposer" => Ok(Self::Proposer),
            "verifier" => Ok(Self::Verifier),
            "selector" => Ok(Self::Selector),
            _ => bail!(
                "stage must be one-grounder, one-grounder-full, grounder, pin-analyzer, proposer, verifier or selector"
            ),
        }
    }
}

fn candidate_input(c: &Candidate) -> Value {
    json!({"id":c.id,"move":c.proposed_move,"from":c.from,"to":c.to})
}

fn validate_proposal(p: &Proposal, config: &Config) -> Result<()> {
    ensure!(
        (3..=5).contains(&p.candidates.len()),
        "expected 3-5 candidates"
    );
    let mut ids = HashSet::new();
    for c in &p.candidates {
        bounded(&c.id, 32)?;
        ensure!(ids.insert(&c.id), "duplicate candidate id");
        move_text(&c.proposed_move, config)?;
    }
    Ok(())
}

pub async fn generate(
    state: &mut Match,
    config: &Config,
    mut rejected: impl FnMut(usize, &str),
) -> Result<Move> {
    ensure!(
        state.game.position().side_to_move() == state.model_side,
        "not the model turn"
    );
    ensure!(
        matches!(
            state.game.outcome(),
            capablanca_chess_plus::GameOutcome::Ongoing | capablanca_chess_plus::GameOutcome::Check
        ),
        "game is over"
    );
    ensure!(
        config.max_attempts > 0 && config.stage_max_attempts > 0,
        "attempt limits must be positive"
    );
    for stage in [
        Stage::Grounder,
        Stage::Proposer,
        Stage::Verifier,
        Stage::Selector,
    ] {
        config.preset_for_stage(stage).generation_length()?;
    }
    output::final_answer(&config.reasoning, "")?;
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(config.timeout_seconds))
        .build()?;
    let mut client = None;
    let image = crate::board_image::BoardImage::new(state.game.position())?;
    let grounder_data = prompt_data(state, config, Stage::Grounder)?;
    let grounded_analysis: String = if state.variant == capablanca_chess_plus::Variant::Classic {
        // Keep the versioned, internally identified map for caching and
        // incremental updates. Only its human-readable derived report is
        // ever given to the Proposer and Verifier.
        incremental::prepare(state, config).await?;
        let (_, map) = state.controls.complete.as_ref()
            .context("Grounder did not publish a complete current-position map")?;
        crate::grounder::validate_engine_map(state.game.position(), map)
            .context("Grounder map must be verified before proposing moves")?;
        crate::grounder::ControlGraph::new(state.game.position(), map.clone())?
            .readable_analysis()
    } else { request_parsed(
        &mut client,
        &http,
        config,
        Stage::Grounder,
        &grounder_data,
        &image,
        |answer| grounder_response(state, config, answer),
    )
    .await? };
    let mut excluded: HashSet<String> = state.wrong_moves.iter().cloned().collect();
    for round in 1..=config.max_attempts {
        tracing::info!(round, "LLM candidate round");
        let mut data = prompt_data(state, config, Stage::Proposer)?;
        data.custom["analysis"] = json!(grounded_analysis);
        let mut sorted: Vec<_> = excluded.iter().cloned().collect();
        sorted.sort();
        data.custom["excluded-moves"] = json!(if sorted.is_empty() {
            String::new()
        } else {
            serde_json::to_string(&sorted)?
        });
        let proposal: Proposal = request(
            &mut client,
            &http,
            config,
            Stage::Proposer,
            &data,
            &image,
            |p: &Proposal| validate_proposal(p, config),
        )
        .await?;
        let mut seen = HashSet::new();
        let mut accepted = Vec::new();
        for c in proposal.candidates {
            let text = move_text(&c.proposed_move, config)?;
            if excluded.contains(&text) {
                continue;
            }
            let (_, mv) = match resolve_candidate(state, config, &c) {
                Ok(resolved) => resolved,
                Err(_) => {
                    excluded.insert(text);
                    continue;
                }
            };
            if validate_metadata(state, config, &c, mv).is_err() {
                continue;
            }
            if !seen.insert(mv) {
                continue;
            }
            tracing::info!(candidate_id = %c.id, "LLM independent verification");
            let mut data = prompt_data(state, config, Stage::Verifier)?;
            data.custom["analysis"] = json!(grounded_analysis);
            data.custom["candidate"] = json!(serde_json::to_string(&json!({
                "move": c.proposed_move, "from": c.from, "to": c.to
            }))?);
            let verification: Verification = request(
                &mut client,
                &http,
                config,
                Stage::Verifier,
                &data,
                &image,
                Verification::validate,
            )
            .await?;
            if verification.verdict == Verdict::Accept {
                accepted.push((c, verification.tactical_result));
            } else {
                excluded.insert(text);
            }
        }
        if accepted.is_empty() {
            continue;
        }
        let mut data = prompt_data(state, config, Stage::Selector)?;
        let survivors: Vec<_> = accepted
            .iter()
            .map(|(c, note)| {
                let mut v = candidate_input(c);
                v["tactical_result"] = json!(note);
                v
            })
            .collect();
        data.custom["accepted-candidates"] = json!(serde_json::to_string(&survivors)?);
        let selection: Selection = request(
            &mut client,
            &http,
            config,
            Stage::Selector,
            &data,
            &image,
            |s: &Selection| {
                let (c, _) = accepted
                    .iter()
                    .find(|(c, _)| c.id == s.candidate_id)
                    .context("unknown accepted candidate")?;
                ensure!(
                    s.final_move == c.proposed_move,
                    "selector changed candidate move"
                );
                Ok(())
            },
        )
        .await?;
        let (selected, _) = accepted
            .iter()
            .find(|(c, _)| c.id == selection.candidate_id)
            .unwrap();
        let text = move_text(&selection.final_move, config)?;
        if let Ok((trial, mv)) = validate_candidate(state, config, selected) {
            *state = trial;
            return Ok(mv);
        }
        state.record_rejected_final(&text);
        excluded.insert(text.clone());
        rejected(round, &text);
    }
    bail!(
        "No legal selected move after {} candidate rounds. Retry to continue this unchanged position.",
        config.max_attempts
    )
}

fn grounder_response(state: &Match, config: &Config, answer: &str) -> Result<String> {
    if config.classic.is_some() {
        let graph=crate::grounder::ControlGraph::parse(state.game.position(),answer)?;
        crate::grounder::validate_engine_map(state.game.position(),graph.map())?;
        graph.normalized_json()
    } else {
        Ok(answer.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legality_gate_handles_special_moves_and_king_safety_on_disposable_clones() {
        use capablanca_chess_plus::{Color, Variant};
        let config = Config::load_classic_with_profile(Some("no-history")).unwrap();
        for (fen, san, from, to, valid) in [
            ("4k3/8/8/8/8/8/8/4K2R w K - 0 1", "O-O", "e1", "g1", true),
            ("4k3/8/8/8/8/8/8/4K2R w - - 0 1", "O-O", "e1", "g1", false),
            (
                "4k3/8/8/3pP3/8/8/8/4K3 w - d6 0 1",
                "exd6",
                "e5",
                "d6",
                true,
            ),
            (
                "4k3/8/8/3pP3/8/8/8/4K3 w - - 0 1",
                "exd6",
                "e5",
                "d6",
                false,
            ),
            ("7k/P7/8/8/8/8/8/4K3 w - - 0 1", "a8=Q+", "a7", "a8", true),
            ("7k/P7/8/8/8/8/8/4K3 w - - 0 1", "a8", "a7", "a8", false),
            ("k3r3/8/8/8/8/8/4R3/4K3 w - - 0 1", "Rf2", "e2", "f2", false),
        ] {
            let state = Match::from_fen(Variant::Classic, Color::White, fen).unwrap();
            let candidate: Candidate =
                serde_json::from_value(json!({"id":"c1","move":san,"from":from,"to":to})).unwrap();
            assert_eq!(
                validate_candidate(&state, &config, &candidate).is_ok(),
                valid,
                "{san}: {fen}"
            );
            assert_eq!(state.game().position().to_fen(), fen);
            assert!(state.uci_history().is_empty());
            assert!(state.wrong_moves().is_empty());
        }
    }

    #[test]
    fn chat_preview_preserves_accepted_history_roles() {
        let config = Config::load_classic_with_profile(Some("with-history")).unwrap();
        let mut state = Match::new(
            capablanca_chess_plus::Variant::Classic,
            capablanca_chess_plus::Color::Black,
        );
        state.accept_human("e2e4").unwrap();
        state.accept_model("e7e5").unwrap();
        state.accept_human("g1f3").unwrap();
        let messages: Vec<prompt_core::ChatMessage> = PromptBuilder::new()
            .build_chat(
                &config.context,
                &template(&config, Stage::Proposer),
                &prompt_data(&state, &config, Stage::Proposer).unwrap(),
            )
            .unwrap();
        use prompt_core::ChatRole;
        assert_eq!(messages[0].role, ChatRole::System);
        for (message, (role, content)) in messages[1..4].iter().zip([
            (ChatRole::User, "e4"),
            (ChatRole::Assistant, "e5"),
            (ChatRole::User, "Nf3"),
        ]) {
            assert_eq!(message.role, role);
            assert_eq!(message.content, content);
        }
        assert_eq!(messages.last().unwrap().role, ChatRole::User);
        assert!(
            messages
                .last()
                .unwrap()
                .content
                .contains(&state.game().position().to_fen())
        );
    }

    #[test]
    fn custom_stage_templates_are_used_verbatim_without_implicit_instructions() {
        for classic in [true, false] {
            let mut config = if classic {
                Config::load_classic_with_profile(Some("no-history"))
            } else {
                Config::load_with_profile(Some("numeric-no-history"))
            }
            .unwrap();
            let state = Match::new(
                if classic {
                    capablanca_chess_plus::Variant::Classic
                } else {
                    capablanca_chess_plus::Variant::Gothic
                },
                capablanca_chess_plus::Color::White,
            );
            for stage in [
                Stage::Grounder,
                Stage::Proposer,
                Stage::Verifier,
                Stage::Selector,
            ] {
                let custom: SystemPromptTemplate = serde_json::from_value(json!({
                    "name":"experiment", "content":"CUSTOM {{#if classic}}SAN{{else}}NUMERIC{{/if}}",
                    "user_prompt":"SIDE {{side-to-move}}", "post_history":"ONLY MY POST"
                })).unwrap();
                let slot = match stage {
                    Stage::OneGrounder | Stage::OneGrounderFull => {
                        &mut config.pipeline.one_grounder
                    }
                    Stage::Grounder => &mut config.pipeline.grounder,
                    Stage::PinAnalyzer => &mut config.pipeline.pin_analyzer,
                    Stage::Proposer => &mut config.pipeline.proposer,
                    Stage::Verifier => &mut config.pipeline.verifier,
                    Stage::Selector => &mut config.pipeline.selector,
                };
                *slot = custom.clone();
                let actual = template(&config, stage);
                assert_eq!(
                    serde_json::to_value(&actual).unwrap(),
                    serde_json::to_value(&custom).unwrap()
                );
                let data = prompt_data(&state, &config, stage).unwrap();
                for key in [
                    "move-format",
                    "stage-rules",
                    "authoritative-state",
                    "output-instructions",
                    "representation-instructions",
                    "history-instructions",
                ] {
                    assert!(data.custom.get(key).is_none(), "hidden instruction {key}");
                }
                let rendered = crate::render_test_chat(&config.context, &actual, &data).text;
                assert!(rendered.contains(if classic {
                    "CUSTOM SAN"
                } else {
                    "CUSTOM NUMERIC"
                }));
                assert!(rendered.contains("SIDE White"));
                assert!(rendered.contains("ONLY MY POST"));
                for unexpected in [
                    "AUTHORITATIVE",
                    "FEN:",
                    "Orthodox",
                    "from and to",
                    "JSON",
                    "must be",
                    "CURRENT PIECES",
                ] {
                    assert!(
                        !rendered.contains(unexpected),
                        "injected {unexpected}: {rendered}"
                    );
                }
            }
        }
    }
}
