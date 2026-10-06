use anyhow::{Context, Result, ensure};
use prompt_core::{ContextTemplate, InstructTemplate, Preset, SystemPromptTemplate};
use serde::de::DeserializeOwned;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

#[derive(Clone)]
pub struct Config {
    pub endpoint: String,
    pub context: ContextTemplate,
    pub instruct: InstructTemplate,
    pub system: SystemPromptTemplate,
    pub preset: Preset,
    pub timeout_seconds: u64,
    pub max_attempts: usize,
}

impl Config {
    /// Relative template paths are relative to the env file. Process env wins.
    pub fn load() -> Result<Self> {
        let path = std::env::var_os("LLM_MATCH_ENV")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/.env")));
        Self::from_env_file(&path)
    }

    pub fn from_env_file(path: &Path) -> Result<Self> {
        let values: HashMap<String, String> = dotenvy::from_path_iter(path)
            .with_context(|| format!("cannot read {}", path.display()))?
            .collect::<std::result::Result<_, _>>()?;
        let get = |key: &str| -> Result<String> {
            std::env::var(key)
                .ok()
                .or_else(|| values.get(key).cloned())
                .with_context(|| format!("missing {key} in {}", path.display()))
        };
        let base = path.parent().unwrap_or(Path::new("."));
        let context = read_json(&base.join(get("LLM_CONTEXT_TEMPLATE")?))?;
        let instruct = read_json(&base.join(get("LLM_INSTRUCT_TEMPLATE")?))?;
        let system: SystemPromptTemplate = read_json(&base.join(get("LLM_SYSTEM_TEMPLATE")?))?;
        ensure!(
            system.post_history.contains("{{board-state}}"),
            "post_history must contain board-state"
        );
        ensure!(
            system.post_history.contains("{{wrong-move}}"),
            "post_history must contain wrong-move"
        );
        let preset = Preset::from_value(read_json(&base.join(get("LLM_PRESET")?))?)?;
        let timeout_seconds = get("LLM_REQUEST_TIMEOUT_SECONDS")?.parse()?;
        let max_attempts = get("LLM_MAX_ATTEMPTS")?.parse()?;
        ensure!(
            timeout_seconds > 0 && max_attempts > 0,
            "timeout and attempts must be positive"
        );
        Ok(Self {
            endpoint: normalize_endpoint(&get("LLM_KOBOLD_URL")?)?,
            context,
            instruct,
            system,
            preset,
            timeout_seconds,
            max_attempts,
        })
    }
}

pub fn normalize_endpoint(value: &str) -> Result<String> {
    let value = value.trim();
    ensure!(!value.is_empty(), "KoboldCPP address is empty");
    let value = if value.contains("://") {
        value.to_owned()
    } else {
        format!("http://{value}")
    };
    let url = reqwest::Url::parse(&value).context("invalid KoboldCPP address")?;
    ensure!(
        matches!(url.scheme(), "http" | "https") && url.host_str().is_some(),
        "use an HTTP(S) KoboldCPP address"
    );
    ensure!(
        url.query().is_none() && url.fragment().is_none(),
        "KoboldCPP address must not contain a query or fragment"
    );
    Ok(value.trim_end_matches('/').to_owned())
}

fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T> {
    serde_json::from_slice(
        &std::fs::read(path).with_context(|| format!("cannot read {}", path.display()))?,
    )
    .with_context(|| format!("invalid JSON in {}", path.display()))
}
