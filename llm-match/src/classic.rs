//! Isolated orthodox-chess configuration; fairy representation profiles are untouched.
use anyhow::{Context, Result};
use capablanca_chess_plus::{Color, Position};
use prompt_core::{PromptData, PromptMessage, PromptRole, SystemPromptTemplate};
use serde::Deserialize;
use serde_json::json;
use std::{collections::BTreeMap, path::Path};

#[derive(Clone)]
pub struct ClassicConfig {
    pub name: String,
    pub include_history: bool,
    pub system: SystemPromptTemplate,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Profiles {
    active_profile: String,
    system_template: String,
    profiles: BTreeMap<String, Profile>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Profile {
    include_history: bool,
}
impl ClassicConfig {
    pub(crate) fn load(
        path: &Path,
        selected: Option<&str>,
        common_template: Option<&Path>,
    ) -> Result<Self> {
        let file: Profiles = toml::from_str(
            &std::fs::read_to_string(path)
                .with_context(|| format!("cannot read {}", path.display()))?,
        )?;
        let name = selected.unwrap_or(&file.active_profile);
        let profile = file
            .profiles
            .get(name)
            .with_context(|| format!("unknown classic chess profile {name}"))?;
        let fallback = path
            .parent()
            .unwrap_or(Path::new("."))
            .join(&file.system_template);
        let template_path = common_template.unwrap_or(&fallback);
        let system: SystemPromptTemplate =
            serde_json::from_slice(&std::fs::read(template_path).with_context(|| {
                format!(
                    "cannot read classic system template {}",
                    template_path.display()
                )
            })?)
            .with_context(|| {
                format!(
                    "invalid classic system template {}",
                    template_path.display()
                )
            })?;
        Ok(Self {
            name: name.into(),
            include_history: profile.include_history,
            system,
        })
    }
}

#[derive(Clone)]
pub(crate) struct SanPly {
    pub number: u32,
    pub side: Color,
    pub san: String,
}

pub(crate) fn history(plies: &[SanPly]) -> String {
    let mut words = Vec::new();
    for (i, p) in plies.iter().enumerate() {
        if p.side == Color::White {
            words.push(format!("{}.", p.number));
        } else if i == 0 || plies[i - 1].side != Color::White || plies[i - 1].number != p.number {
            words.push(format!("{}...", p.number));
        }
        words.push(p.san.clone());
    }
    words.join(" ")
}

pub(crate) fn prompt(
    position: &Position,
    plies: &[SanPly],
    wrong: &str,
    config: &ClassicConfig,
    model_side: Color,
) -> PromptData {
    PromptData {
        user: "Human".into(),
        character: "Chess model".into(),
        messages: if config.include_history {
            plies
                .iter()
                .map(|ply| PromptMessage {
                    role: if ply.side == model_side {
                        PromptRole::Assistant
                    } else {
                        PromptRole::User
                    },
                    name: None,
                    content: ply.san.clone(),
                })
                .collect()
        } else {
            Vec::new()
        },
        custom: json!({"ascii":crate::prompts::ascii_board(position), "fen":position.to_fen(),
            "side-to-move":match position.side_to_move() { Color::White => "White", Color::Black => "Black" },
            "last-move":plies.last().map(|p| p.san.as_str()).unwrap_or(""),
            "classic-san-history":if config.include_history { history(plies) } else { String::new() },
            "wrong-move":wrong}),
        ..Default::default()
    }
}

/// Syntactic final-answer gate only; legality and canonical disambiguation are
/// exclusively determined by engine SAN parsing. Never extract from prose.
pub(crate) fn san_shape(answer: &str) -> bool {
    let s = answer
        .strip_suffix('+')
        .or_else(|| answer.strip_suffix('#'))
        .unwrap_or(answer);
    if matches!(s, "O-O" | "O-O-O") {
        return true;
    }
    let s = if let Some((body, promo)) = s.split_once('=') {
        if !matches!(promo, "Q" | "R" | "B" | "N") {
            return false;
        }
        body
    } else {
        s
    };
    let b = s.as_bytes();
    if b.len() < 2 || !b.is_ascii() {
        return false;
    }
    let n = b.len();
    if !(b'a'..=b'h').contains(&b[n - 2]) || !(b'1'..=b'8').contains(&b[n - 1]) {
        return false;
    }
    let prefix = &s[..n - 2];
    if prefix.is_empty() {
        return true;
    }
    if prefix.len() == 2 && (b'a'..=b'h').contains(&prefix.as_bytes()[0]) && prefix.ends_with('x') {
        return true;
    }
    if !matches!(prefix.as_bytes()[0], b'K' | b'Q' | b'R' | b'B' | b'N') {
        return false;
    }
    let dis = prefix[1..]
        .strip_suffix('x')
        .unwrap_or(&prefix[1..])
        .as_bytes();
    match dis {
        [] => true,
        [a] => (b'a'..=b'h').contains(a) || (b'1'..=b'8').contains(a),
        [a, b] => (b'a'..=b'h').contains(a) && (b'1'..=b'8').contains(b),
        _ => false,
    }
}
