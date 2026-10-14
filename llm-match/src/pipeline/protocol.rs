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
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Proposal {
    pub candidates: Vec<Candidate>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum Verdict {
    ACCEPT,
    REJECT,
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
#[derive(Debug, Deserialize)]
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
