//! Classic chess: model-suggested candidate/counterplay branches, LLM Grounder
//! deltas for each legal hypothetical ply, then read-only Selector.
use super::*;
use crate::grounder::{ControlGraph, ControlMap};
use capablanca_chess_plus::{Game, GameOutcome, Variant};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateLine {
    pub id: String,
    #[serde(rename = "move")]
    pub san: String,
    /// Independent opponent reply SANs, each beginning after our move.
    pub counterplay: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProposerLines {
    pub candidates: Vec<CandidateLine>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SelectorChoice {
    pub candidate_id: String,
}

fn canonical(game: &Game, text: &str) -> Result<Move> {
    ensure!(!text.is_empty() && text.len() <= 32 && text.trim() == text,
        "move must be a short nonempty SAN string without spaces");
    let mv = game.position().parse_san_move(text)
        .with_context(|| format!("{text:?} is not legal for {:?}",game.position().side_to_move()))?;
    let correct = game.position().san(mv)?;
    ensure!(correct == text, "incorrect SAN {text:?}; canonical form is {correct:?}");
    Ok(mv)
}

/// Validate every candidate and every counter-move BEFORE requesting any
/// expensive model-generated geometry. One invalid branch returns actionable
/// feedback and causes a new Proposer generation, not silent elimination.
pub(super) fn preflight(root: &Match, result: &ProposerLines) -> Result<()> {
    ensure!((1..=4).contains(&result.candidates.len()),
        "Proposer must provide 1-4 candidate moves");
    let mut ids=HashSet::new();
    let mut moves=HashSet::new();
    for candidate in &result.candidates {
        bounded(&candidate.id,32)?;
        ensure!(ids.insert(candidate.id.as_str()),"duplicate candidate id {}",candidate.id);
        ensure!(moves.insert(candidate.san.as_str()),"duplicate candidate {}",candidate.san);
        let mut first=root.game().clone();
        let mv=canonical(&first,&candidate.san)
            .with_context(|| format!("candidate {} is illegal",candidate.id))?;
        first.play(mv)?;
        let is_terminal=!matches!(first.outcome(),GameOutcome::Ongoing | GameOutcome::Check);
        if is_terminal {
            ensure!(candidate.counterplay.is_empty(),
                "candidate {} ends the game; it cannot have counterplay",candidate.id);
        } else {
            ensure!((1..=2).contains(&candidate.counterplay.len()),
                "candidate {} needs 1-2 plausible opponent replies",candidate.id);
        }
        let mut seen_replies=HashSet::new();
        for response in &candidate.counterplay {
            ensure!(seen_replies.insert(response.as_str()),
                "duplicate reply {response} for {}",candidate.id);
            let mv=canonical(&first,response)
                .with_context(|| format!("candidate {} counterplay {response:?} is illegal",
                    candidate.id))?;
            // All replies are independent, not consecutive moves.
            let mut reply=first.clone();
            reply.play(mv)?;
        }
    }
    Ok(())
}

/// Explicitly labelled and isolated "count heuristic", NOT exchange search.
async fn map_after(trial: &mut Match, config:&Config) -> Result<ControlMap> {
    if trial.auto_grounder_updates {
        let map=crate::grounder::engine_map(trial.game.position())?;
        trial.controls.complete=Some((trial.controls.positions.len()-1,map.clone()));
        return Ok(map);
    }
    // Incremental invalidation and all re-grounding requests use OneGrounder
    // LLM calls, with exact engine validation of each piece's geometry.
    incremental::prepare(trial,config).await?;
    let (_,map)=trial.controls.complete.as_ref().context("missing simulated map")?;
    crate::grounder::validate_engine_map(trial.game.position(),map)?;
    Ok(map.clone())
}

fn after_san(root:&Match,config:&Config,san:&str) -> Result<Match> {
    let mut trial=root.clone();
    trial.model_side=trial.game.position().side_to_move();
    trial.accept_configured_answer(san,config)?;
    Ok(trial)
}

pub(super) async fn choose(
    state:&mut Match,
    config:&Config,
    mut rejected:impl FnMut(usize,&str),
) -> Result<Move> {
    ensure!(state.variant==Variant::Classic && config.classic.is_some(),
        "two-stage counterplay pipeline requires Classic chess");
    ensure!(state.game.position().side_to_move()==state.model_side,
        "not the model turn");
    ensure!(matches!(state.game.outcome(),GameOutcome::Ongoing | GameOutcome::Check),
        "game is over");
    config.preset_for_stage(Stage::Proposer).generation_length()?;
    config.preset_for_stage(Stage::Selector).generation_length()?;
    let base=state.controls.complete.as_ref()
        .filter(|(index,_)| *index+1==state.controls.positions.len())
        .map(|(_,map)| map.clone());
    // The CURRENT position is always bootstrapped by the engine if uncached.
    // Subsequent what-if positions are LLM-grounded unless --auto-grounder.
    let current=if let Some(map)=base {
        crate::grounder::validate_engine_map(state.game.position(),&map)?;
        map
    } else {crate::grounder::engine_map(state.game.position())?};
    state.controls.complete=Some((state.controls.positions.len()-1,current.clone()));
    let graph=ControlGraph::new(state.game.position(),current.clone())?;
    let root_analysis=graph.readable_analysis();
    let image=crate::board_image::BoardImage::with_enabled(state.game.position(),config.send_board_image)?;
    let http=reqwest::Client::builder().timeout(Duration::from_secs(config.timeout_seconds)).build()?;
    let mut client=None;
    let mut feedback=String::new();
    for attempt in 1..=config.max_attempts {
        let mut data=prompt_data(state,config,Stage::Proposer)?;
        data.custom["analysis"]=json!(root_analysis);
        data.custom["validation-feedback"]=json!(feedback);
        let proposed: ProposerLines=request(
            &mut client,&http,config,Stage::Proposer,&data,&image,
            |_p:&ProposerLines| Ok(()),
        ).await?;
        if let Err(error)=preflight(state,&proposed) {
            feedback=format!("Your previous candidates or proposed counterplay were illegal. Correct them. Engine report: {error:#}");
            tracing::warn!(attempt,error=%error,"Proposer rejected illegal proposed line");
            rejected(attempt,&feedback);
            continue;
        }
        let mut annotated=Vec::new();
        for candidate in &proposed.candidates {
            let root_before=state.game.position();
            let mut move_trial=after_san(state,config,&candidate.san)?;
            let move_map=map_after(&mut move_trial,config).await?;
            let own_delta=delta::position_delta(root_before,move_trial.game.position(),&current,&move_map)?;
            let mut counters=Vec::new();
            for reply in &candidate.counterplay {
                let mut reply_trial=after_san(&move_trial,config,reply)?;
                let reply_map=map_after(&mut reply_trial,config).await?;
                let reply_delta=delta::position_delta(
                    move_trial.game.position(),reply_trial.game.position(),&move_map,&reply_map)?;
                counters.push(json!({"reply":reply,"delta":reply_delta}));
            }
            annotated.push(json!({
                "id":candidate.id,
                "move":candidate.san,
                "after_move":own_delta,
                "counterplay":counters
            }));
        }
        let mut data=prompt_data(state,config,Stage::Selector)?;
        data.custom["verified-candidates"]=json!(serde_json::to_string(&annotated)?);
        let selection:SelectorChoice=request(
            &mut client,&http,config,Stage::Selector,&data,&image,
            |s:&SelectorChoice| {
                ensure!(proposed.candidates.iter().any(|c|c.id==s.candidate_id),
                    "Selector must copy an existing candidate_id");
                Ok(())
            },
        ).await?;
        let chosen=proposed.candidates.iter()
            .find(|c|c.id==selection.candidate_id).expect("validated id");
        // Apply only the selected FIRST move to the real game. Never apply any
        // counterplay and never let selector invent an unexamined SAN.
        let mv=canonical(state.game(),&chosen.san)?;
        state.accept_configured_answer(&chosen.san,config)?;
        return Ok(mv);
    }
    bail!("Proposer failed to return legal move/counterplay after {} attempts",config.max_attempts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use capablanca_chess_plus::Color;

    fn root() -> Match {
        Match::new(Variant::Classic,Color::White)
    }
    fn proposal(move_san:&str,reply:&str) -> ProposerLines {
        ProposerLines {candidates:vec![
            CandidateLine{id:"c1".into(),san:move_san.into(),counterplay:vec![reply.into()]},
            CandidateLine{id:"c2".into(),san:"d4".into(),counterplay:vec!["d5".into()]}
        ]}
    }
    #[test]
    fn invalid_reply_is_not_accepted() {
        let e=preflight(&root(),&proposal("e4","Qh4")).unwrap_err();
        assert!(format!("{e:#}").contains("counterplay"));
        preflight(&root(),&proposal("e4","e5")).unwrap();
    }
    #[test]
    fn wrong_side_and_duplicate_candidates_are_rejected() {
        let e=preflight(&root(),&proposal("e5","e4")).unwrap_err();
        assert!(format!("{e:#}").contains("candidate"));
        let mut p=proposal("e4","e5");
        p.candidates[1].san="e4".into();
        assert!(preflight(&root(),&p).is_err());
    }
}
