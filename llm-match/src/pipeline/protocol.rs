use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Candidate {
    pub id: String,
    #[serde(rename = "move")]
    pub proposed_move: Value,
    pub from: Value,
    pub to: Value,
    #[serde(default)]
    pub rationale: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FinalMove {
    #[serde(rename = "move")]
    pub proposed_move: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Proposal {
    pub candidates: Vec<Candidate>,
}


#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThreatAnalysis {
    pub immediate_threats: Vec<String>,
    pub opponent_plans: Vec<String>,
}
impl ThreatAnalysis {
    pub fn validate(&self) -> Result<()> {
        ensure!(self.immediate_threats.len() <= 6, "too many immediate threats");
        ensure!(self.opponent_plans.len() <= 5, "too many opponent plans");
        for note in self.immediate_threats.iter().chain(&self.opponent_plans) {
            ensure!(!note.trim().is_empty() && note.chars().count() <= 240,
                "threat note must contain 1..=240 characters");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "UPPERCASE")]
pub enum Verdict {
    Accept,
    Reject,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Verification {
    pub verdict: Verdict,
    pub tactical_result: String,
}
impl Verification {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            !self.tactical_result.trim().is_empty(),
            "tactical_result must be non-empty"
        );
        Ok(())
    }
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Selection {
    pub candidate_id: String,
    pub final_move: Value,
}
pub fn bounded(text: &str, max: usize) -> Result<()> {
    ensure!(
        !text.trim().is_empty() && text.chars().count() <= max,
        "invalid compact text length"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn final_proposer_output_is_one_strict_san_field() {
        let selected: FinalMove = serde_json::from_str(r#"{"move":"Rxe8+"}"#).unwrap();
        assert_eq!(selected.proposed_move,"Rxe8+");
        assert!(serde_json::from_str::<FinalMove>(r#"{"candidates":[]}"#).is_err());
        assert!(serde_json::from_str::<FinalMove>(r#"{"move":"e4","reason":"foo"}"#).is_err());
    }
    #[test]
    fn threat_report_has_bounded_fields_and_no_extras() {
        let valid: ThreatAnalysis = serde_json::from_str(
            r#"{"immediate_threats":[],"opponent_plans":[]}"#).unwrap();
        valid.validate().unwrap();
        assert!(serde_json::from_str::<ThreatAnalysis>(
            r#"{"immediate_threats":[],"opponent_plans":[],"extra":"x"}"#).is_err());
        let invalid = ThreatAnalysis {
            immediate_threats: vec![" ".into()],
            opponent_plans: vec![],
        };
        assert!(invalid.validate().is_err());
        let invalid = ThreatAnalysis {
            immediate_threats: vec![],
            opponent_plans: vec!["x".repeat(241)],
        };
        assert!(invalid.validate().is_err());
    }
    #[test]
    fn explanations_are_optional_and_not_length_limited() {
        let p: Proposal = serde_json::from_str(
            r#"{"candidates":[{"id":"c1","move":"e4","from":"e2","to":"e4"}]}"#,
        )
        .unwrap();
        assert!(p.candidates[0].rationale.is_empty());
    }
    #[test]
    fn verification_requires_tactical_result_and_no_echoed_identity() {
        let good: Verification = serde_json::from_str(r#"{"verdict":"ACCEPT","tactical_result":"No forcing refutation found after the recapture."}"#).unwrap();
        good.validate().unwrap();
        for bad in [
            r#"{"verdict":"ACCEPT","tactical_result":"x","candidate_id":"c1"}"#,
            r#"{"verdict":"ACCEPT"}"#,
            r#"{"verdict":"ACCEPT","reason":"x"}"#,
            r#"{"verdict":"ACCEPT","tactical_result":null}"#,
        ] {
            assert!(serde_json::from_str::<Verification>(bad).is_err());
        }
        for empty in ["", " \n\t"] {
            let v: Verification = serde_json::from_value(
                serde_json::json!({"verdict":"REJECT","tactical_result":empty}),
            )
            .unwrap();
            assert!(v.validate().is_err());
        }
    }
    #[test]
    fn structured_results_reject_invalid_verdicts_and_ambiguous_fields() {
        for bad in [
            r#"{"verdict":"maybe","tactical_result":"x"}"#,
            r#"{"verdict":"ACCEPT","tactical_result":"x","extra":0}"#,
        ] {
            assert!(serde_json::from_str::<Verification>(bad).is_err());
        }
        assert!(
            serde_json::from_str::<Selection>(
                r#"{"candidate_id":"a","final_move":"e4","final_move":"d4"}"#
            )
            .is_err()
        );
        assert!(serde_json::from_str::<Selection>("FINAL_MOVE: e4").is_err());
    }
}
