//! Reliable Classic chess: LLM proposes moves; Rust verifies their legality,
//! Grounder supplies ONLY geometry; LLM selector chooses among legal proposals.
//! No engine-generated candidate lists, scores, best moves, or search.
use super::*;
use crate::grounder::{ControlGraph, ControlMap};
use capablanca_chess_plus::{Game, GameOutcome, Variant};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CandidateLine {
    #[serde(default)]
    pub id: String,
    #[serde(rename = "move")]
    pub san: String,
    #[serde(default)]
    pub counterplay: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ProposerLines {
    pub candidates: Vec<CandidateLine>,
}

#[derive(Debug, Deserialize, Serialize)]
pub(super) struct SelectorChoice {
    pub candidate_id: String,
}

fn normalized_move(game:&Game, input:&str) -> Result<(Move,String)> {
    let raw=input.trim();
    ensure!(!raw.is_empty() && raw.len()<=40,"empty or oversized move");
    // Standard SAN, annotated SAN, UCI, and piece-from-to notation such
    // as Nf6xd5 are accepted, then converted to authoritative SAN.
    let text=raw.trim_end_matches(|c:char| c=='!' || c=='?').replace('0',"O");
    let p=game.position();
    let mut parsed=p.parse_san_move(&text).ok()
        .or_else(||p.parse_uci_move(&text).ok());
    if parsed.is_none() {
        let b=text.as_bytes();
        if b.len()>=5 && b[0].is_ascii_uppercase() {
            let rest=&b[1..];
            let candidate=if rest.len()==4 {
                Some(rest.to_vec())
            } else if rest.len()==5 && matches!(rest[2],b'x'|b'-') {
                Some(vec![rest[0],rest[1],rest[3],rest[4]])
            } else {None};
            if let Some(bytes)=candidate {
                if bytes.len()==4 && bytes[0].is_ascii_lowercase()
                    && (b'1'..=b'8').contains(&bytes[1])
                    && bytes[2].is_ascii_lowercase()
                    && (b'1'..=b'8').contains(&bytes[3])
                {
                    if let Ok(uci)=String::from_utf8(bytes) {
                        parsed=p.parse_uci_move(&uci).ok();
                    }
                }
            }
        }
    }
    let mv=parsed.with_context(||format!(
        "{raw:?} is not a legal SAN/UCI move for {:?}",p.side_to_move()))?;
    let canonical=p.san(mv)?;
    Ok((mv,canonical))
}

fn suggestions(value:&Value) -> Vec<CandidateLine> {
    // The API deliberately tolerates minimal output and extra reasoning
    // fields. No illegal sub-branch can invalidate a legal parent move.
    let proposals=value.get("candidates").or_else(||value.get("moves"));
    let values=proposals.and_then(Value::as_array);
    let mut found=Vec::new();
    if let Some(values)=values {
        for (i,v) in values.iter().take(16).enumerate() {
            let move_san=if let Some(s)=v.as_str() {Some(s)}
                else {v.get("move").or_else(||v.get("san")).and_then(Value::as_str)};
            if let Some(san)=move_san {
                let replies=v.get("counterplay").or_else(||v.get("replies"));
                let counterplay=match replies {
                    Some(Value::Array(arr))=>arr.iter().filter_map(|v|{
                        v.as_str().or_else(||v.get("move").and_then(Value::as_str))
                    }).take(4).map(str::to_owned).collect(),
                    Some(Value::String(s))=>vec![s.clone()],
                    _=>vec![],
                };
                found.push(CandidateLine {
                    id:v.get("id").and_then(Value::as_str).filter(|s|!s.is_empty())
                        .map(str::to_owned).unwrap_or_else(||format!("c{}",i+1)),
                    san:san.to_owned(),
                    counterplay,
                });
            }
        }
    } else if let Some(move_san)=value.get("move").and_then(Value::as_str) {
        found.push(CandidateLine{id:"c1".into(),san:move_san.into(),counterplay:vec![]});
    }
    found
}

fn legal_suggestions(root:&Match, value:&Value) -> (Vec<CandidateLine>,Vec<String>) {
    let mut valid=Vec::new();
    let mut errors=Vec::new();
    let mut seen=HashSet::new();
    for candidate in suggestions(value) {
        match normalized_move(root.game(),&candidate.san) {
            Ok((mv,san)) => {
                if !seen.insert(mv.to_uci()) {continue}
                valid.push(CandidateLine {
                    id:format!("c{}",valid.len()+1),
                    san,
                    counterplay:candidate.counterplay,
                });
                if valid.len()>=4 {break}
            }
            Err(error)=>errors.push(format!("{}: {error:#}",candidate.san)),
        }
    }
    (valid,errors)
}

async fn map_after(trial:&mut Match, config:&Config) -> Result<ControlMap> {
    if trial.auto_grounder_updates {
        // Opt-in fast test mode: Grounder geometry is calculated by the
        // engine. No scores, candidate moves or search are generated.
        let map=crate::grounder::engine_map(trial.game.position())?;
        trial.controls.complete=Some((trial.controls.positions.len()-1,map.clone()));
        return Ok(map);
    }
    // Initial map (when auto-initial is enabled) comes from the engine.
    // Every accepted move after that invalidates relevant pieces and
    // makes OneGrounder LLM requests for their updated controls and xrays.
    incremental::prepare(trial,config).await?;
    let (index,map)=trial.controls.complete.as_ref().context("Grounder map missing")?;
    ensure!(*index+1==trial.controls.positions.len(),
        "Grounder cache is not current");
    crate::grounder::validate_engine_map(trial.game.position(),map)?;
    Ok(map.clone())
}

fn applied(root:&Match, san:&str) -> Result<Match> {
    let mut trial=root.clone();
    let (mv,_)=normalized_move(trial.game(),san)?;
    trial.model_side=trial.game.position().side_to_move();
    trial.accept_model(&mv.to_uci())?;
    Ok(trial)
}

/// Initial Grounder may be engine-calculated; subsequent accepted or
/// hypothetical positions are updated by OneGrounder LLM, unless the user
/// explicitly requests automatic Grounder updates for fast testing.
pub(super) async fn choose(
    state:&mut Match,
    config:&Config,
    mut rejected:impl FnMut(usize,&str),
) -> Result<Move> {
    ensure!(state.variant==Variant::Classic && config.classic.is_some(),
        "Classic pipeline needs Classic rules/config");
    ensure!(state.game.position().side_to_move()==state.model_side,"not the model turn");
    ensure!(matches!(state.game.outcome(),GameOutcome::Ongoing | GameOutcome::Check),"game over");
    let current=map_after(state,config).await?;
    let graph=ControlGraph::new(state.game.position(),current.clone())?;
    let analysis=graph.readable_analysis();
    let image=crate::board_image::BoardImage::with_enabled(
        state.game.position(),config.send_board_image)?;
    let http=reqwest::Client::builder()
        .timeout(Duration::from_secs(config.timeout_seconds)).build()?;
    let mut client=None;
    let mut feedback=String::new();

    // One successful legal suggestion is enough. Never reject the entire
    // batch for invalid counterplay or duplicate/extra candidate proposals.
    for round in 1..=config.max_attempts {
        let mut data=prompt_data(state,config,Stage::Proposer)?;
        data.custom["analysis"]=json!(analysis);
        data.custom["validation-feedback"]=json!(feedback);
        let response:Value=request_parsed(&mut client,&http,config,Stage::Proposer,
            &data,&image,|answer| {
                // JSON is preferred, but a single bare SAN/UCI is also
                // a valid proposal. Never require a second LLM generation
                // merely to wrap a legal move in braces.
                Ok(output::parse_final_json::<Value>(answer)
                    .unwrap_or_else(|_|json!({"move":answer.trim()})))
            }).await?;
        let (candidates,errors)=legal_suggestions(state,&response);
        if candidates.is_empty() {
            feedback=if errors.is_empty() {
                "No candidate move found. Return at least one SAN or UCI move. Only move OUR side's pieces; board position is unchanged.".into()
            }else{
                format!("No legal candidate. Please replace with another SAN/UCI move for {:?}: {}",
                    state.game().position().side_to_move(),errors.join("; "))
            };
            tracing::warn!(round,feedback=%feedback,"No legal Proposer candidates");
            rejected(round,&feedback);
            continue;
        }
        if !errors.is_empty(){
            tracing::info!(round,?errors,"Skipping illegal guesses, keeping legal candidates");
        }

        let mut annotated=Vec::new();
        let mut prepared=Vec::new();
        for candidate in &candidates {
            let mut moved=applied(state,&candidate.san)?;
            let map=map_after(&mut moved,config).await?;
            let diff=delta::position_delta(state.game.position(),
                moved.game.position(),&current,&map)?;
            let mut replies=Vec::new();
            for counter in &candidate.counterplay {
                match normalized_move(moved.game(),counter) {
                    Ok((_,san))=>{
                        let mut replied=applied(&moved,&san)?;
                        let reply_map=map_after(&mut replied,config).await?;
                        let delta=delta::position_delta(moved.game.position(),
                            replied.game.position(),&map,&reply_map)?;
                        replies.push(json!({"reply":san,"delta":delta}));
                    }
                    Err(error)=>{
                        // Invalid reply is advisory and NEVER vetoes legal own move.
                        tracing::warn!(candidate=%candidate.san,counterplay=%counter,
                            error=%error,"Ignoring illegal suggested counterplay");
                    }
                }
            }
            annotated.push(json!({
                "id":candidate.id,"move":candidate.san,
                "after_move":diff,"counterplay":replies
            }));
            // Commit exactly the already grounded first-move snapshot.
            // No duplicate LLM Grounder requests for the chosen move.
            prepared.push(moved);
        }

        // One candidate: no LLM selector needed. No tactical hint or engine
        // move preference; choose the only move the model legally proposed.
        let chosen=if candidates.len()==1 {
            tracing::info!(san=%candidates[0].san,
                "One legal model candidate: skip unnecessary Selector request");
            &candidates[0]
        } else {
            let mut data=prompt_data(state,config,Stage::Selector)?;
            data.custom["verified-candidates"]=json!(serde_json::to_string(&annotated)?);
            let choice:Result<SelectorChoice>=request(
                &mut client,&http,config,Stage::Selector,&data,&image,
                |s:&SelectorChoice| {
                    ensure!(candidates.iter().any(|c|c.id==s.candidate_id),
                        "Selector id must appear in proposed candidates");
                    Ok(())
                }
            ).await;
            match choice {
                Ok(choice)=>candidates.iter().find(|c|c.id==choice.candidate_id)
                    .unwrap_or(&candidates[0]),
                Err(error)=>{
                    // Operational fallback, NOT an engine hint: first legal
                    // move from Proposer, preserving progress today.
                    tracing::warn!(error=%error,
                        "Selector failed; using first LEGAL Proposer candidate without engine ranking");
                    &candidates[0]
                }
            }
        };
        let (mv,_)=normalized_move(state.game(),&chosen.san)?;
        // Persist the selected move's verified incremental Grounder map.
        // Opponent hypotheses remain disposable snapshots.
        let selected_index=candidates.iter().position(|c|c.id==chosen.id)
            .context("selected candidate missing")?;
        *state=prepared.swap_remove(selected_index);
        return Ok(mv);
    }
    bail!("Proposer provided no legal SAN/UCI move after {} rounds",config.max_attempts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use capablanca_chess_plus::Color;
    fn root() -> Match {Match::new(Variant::Classic,Color::White)}
    #[test]
    fn accepts_legal_move_despite_bad_other_candidate_and_bad_reply() {
        let proposals=json!({"candidates":[
            {"move":"e5","counterplay":["Bb5"]},
            {"move":"e4","counterplay":["Bb5"]}]});
        let (valid,bad)=legal_suggestions(&root(),&proposals);
        assert_eq!(valid.len(),1);
        assert_eq!(valid[0].san,"e4");
        assert!(!bad.is_empty());
    }
    #[test]
    fn accepts_extended_piece_coordinates_and_minimal_json() {
        let m=Match::from_fen(Variant::Classic,Color::Black,
            "r1bqkb1r/pp2pppp/2n2n2/2PN4/5B2/8/PPP1PPPP/R2QKBNR b KQkq - 0 5").unwrap();
        let (valid,bad)=legal_suggestions(&m,&json!({"move":"Nf6xd5"}));
        assert!(bad.is_empty(),"unexpected rejection: {bad:?}");
        assert_eq!(valid.len(),1);
        assert_eq!(valid[0].san,"Nxd5");
    }
    #[test]
    fn supports_array_of_moves_and_deduplicates() {
        let (valid,bad)=legal_suggestions(&root(),&json!({"moves":["e4","e2e4","d4"]}));
        assert!(bad.is_empty());
        assert_eq!(valid.len(),2);
        assert_eq!(valid[0].san,"e4");
        assert_eq!(valid[1].san,"d4");
    }
    #[test]
    fn illegal_reply_cannot_veto_legal_candidate() {
        let (valid,bad)=legal_suggestions(&root(),
            &json!({"candidates":[{"move":"e4","counterplay":["Qh4"]}]}));
        assert!(bad.is_empty());
        assert_eq!(valid[0].counterplay,vec!["Qh4"]);
    }
}
