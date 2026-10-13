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
    pub candidate_id: String,
    pub verdict: Verdict,
    #[serde(default)]
    pub reason: String,
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
        let v: Verification =
            serde_json::from_str(r#"{"candidate_id":"c1","verdict":"ACCEPT"}"#).unwrap();
        assert!(v.reason.is_empty());
        let v: Verification = serde_json::from_value(serde_json::json!({"candidate_id":"c1","verdict":"ACCEPT","reason":"free explanation ".repeat(100)})).unwrap();
        assert!(v.reason.len() > 320);
    }
    #[test]
    fn structured_results_reject_invalid_verdicts_and_ambiguous_fields() {
        for bad in [
            r#"{"candidate_id":"a","verdict":"maybe","reason":"x"}"#,
            r#"{"candidate_id":"a","verdict":"ACCEPT","reason":"x","extra":0}"#,
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
