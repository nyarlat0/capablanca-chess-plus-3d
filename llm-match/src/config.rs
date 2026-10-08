use anyhow::{Context, Result, ensure};
use prompt_core::{
    ContextTemplate, InstructTemplate, Preset, ReasoningTemplate, SystemPromptTemplate,
};
use serde::de::DeserializeOwned;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

#[derive(Clone)]
pub struct Config {
    pub classic: Option<crate::ClassicConfig>,
    pub profile_name: String,
    pub representation: crate::Representation,
    pub rules_templates: std::collections::BTreeMap<String, String>,
    pub endpoint: String,
    pub context: ContextTemplate,
    pub instruct: InstructTemplate,
    pub reasoning: ReasoningTemplate,
    pub system: SystemPromptTemplate,
    pub preset: Preset,
    pub timeout_seconds: u64,
    pub max_attempts: usize,
}

impl Config {
    pub fn load_for_variant(variant: capablanca_chess_plus::Variant) -> Result<Self> {
        if variant != capablanca_chess_plus::Variant::Classic {
            return Self::load();
        }
        // Only the dedicated classic selector applies to this configuration.
        let mut selected = None;
        let mut original = std::env::args().skip(1);
        while let Some(arg) = original.next() {
            if arg == "--llm-classic-profile" {
                selected = Some(
                    original
                        .next()
                        .context("--llm-classic-profile requires a name")?,
                );
            } else if let Some(value) = arg.strip_prefix("--llm-classic-profile=") {
                selected = Some(value.into());
            }
        }
        if let Some(name) = &selected {
            ensure!(
                !name.is_empty() && !name.starts_with('-'),
                "--llm-classic-profile requires a name"
            );
        }
        Self::load_classic_with_profile(selected.as_deref())
    }
    pub fn load_classic_with_profile(profile: Option<&str>) -> Result<Self> {
        let path = std::env::var_os("LLM_MATCH_ENV")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/.env")));
        Self::from_env_mode(&path, profile, true)
    }
    pub fn summary(&self) -> String {
        if let Some(c) = &self.classic {
            format!(
                "classic chess | history: {} | output: SAN",
                if c.include_history {
                    "full SAN"
                } else {
                    "none"
                }
            )
        } else {
            self.representation.summary()
        }
    }
    /// Relative template paths are relative to the env file. Process env wins.
    pub fn load() -> Result<Self> {
        let profile = crate::profile::cli_profile(std::env::args().skip(1))?;
        Self::load_with_profile(profile.as_deref())
    }
    pub fn load_with_profile(profile: Option<&str>) -> Result<Self> {
        let path = std::env::var_os("LLM_MATCH_ENV")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/.env")));
        Self::from_env_file_with_profile(&path, profile)
    }

    pub fn from_env_file(path: &Path) -> Result<Self> {
        Self::from_env_file_with_profile(path, None)
    }
    pub fn from_env_file_with_profile(path: &Path, profile_override: Option<&str>) -> Result<Self> {
        Self::from_env_mode(path, profile_override, false)
    }
    fn from_env_mode(
        path: &Path,
        profile_override: Option<&str>,
        classic_mode: bool,
    ) -> Result<Self> {
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
        let classic = if classic_mode {
            let env_profile = get("LLM_CLASSIC_PROFILE").ok();
            let with_history = get("LLM_CLASSIC_SYSTEM_TEMPLATE_WITH_HISTORY")
                .ok()
                .map(|p| base.join(p));
            let no_history = get("LLM_CLASSIC_SYSTEM_TEMPLATE_NO_HISTORY")
                .ok()
                .map(|p| base.join(p));
            Some(crate::ClassicConfig::load(
                &base.join(
                    get("LLM_CLASSIC_PROFILES_FILE")
                        .unwrap_or_else(|_| "classic-profiles.toml".into()),
                ),
                profile_override.or(env_profile.as_deref()),
                with_history.as_deref(),
                no_history.as_deref(),
            )?)
        } else {
            None
        };
        let (profile_name, representation) = if let Some(c) = &classic {
            (
                format!("classic-chess/{}", c.name),
                crate::Representation::default(),
            )
        } else {
            let profiles_path = base.join(
                get("LLM_PROFILES_FILE").unwrap_or_else(|_| "representation-profiles.toml".into()),
            );
            let profiles = crate::ProfileFile::parse(
                &std::fs::read_to_string(&profiles_path)
                    .with_context(|| format!("cannot read {}", profiles_path.display()))?,
            )?;
            let env_profile = get("LLM_PROFILE").ok();
            let (profile_name, representation) =
                profiles.select(profile_override.or(env_profile.as_deref()))?;
            (profile_name, representation)
        };
        let rules_dir = base.join(get("LLM_RULES_DIR").unwrap_or_else(|_| "rules".into()));
        let mut rules_templates = std::collections::BTreeMap::new();
        for variant in capablanca_chess_plus::Variant::ALL {
            if classic_mode || variant == capablanca_chess_plus::Variant::Classic {
                continue;
            }
            let key = crate::rules::variant_key(variant);
            let path = rules_dir.join(format!("{key}.txt"));
            rules_templates.insert(
                key.into(),
                std::fs::read_to_string(&path)
                    .with_context(|| format!("cannot read rules {}", path.display()))?,
            );
        }
        let context = read_json(&base.join(get("LLM_CONTEXT_TEMPLATE")?))?;
        let instruct = read_json(&base.join(get("LLM_INSTRUCT_TEMPLATE")?))?;
        let reasoning = read_json(&base.join(get("LLM_REASONING_TEMPLATE")?))?;
        let system: SystemPromptTemplate = if let Some(c) = &classic {
            c.system.clone()
        } else {
            read_json(&base.join(get("LLM_SYSTEM_TEMPLATE")?))?
        };
        if !classic_mode {
            for directive in [
                "{{representation-instructions}}",
                "{{history-instructions}}",
                "{{output-instructions}}",
            ] {
                ensure!(
                    system.content.contains(directive) || system.post_history.contains(directive),
                    "system template must contain {directive} to support representation profiles"
                );
            }
            ensure!(
                system.post_history.contains("{{board-state}}"),
                "post_history must contain board-state"
            );
            ensure!(
                system.post_history.contains("{{wrong-move}}"),
                "post_history must contain wrong-move"
            );
        }
        let preset = Preset::from_value(read_json(&base.join(get("LLM_PRESET")?))?)?;
        let timeout_seconds = get("LLM_REQUEST_TIMEOUT_SECONDS")?.parse()?;
        let max_attempts = get("LLM_MAX_ATTEMPTS")?.parse()?;
        ensure!(
            timeout_seconds > 0 && max_attempts > 0,
            "timeout and attempts must be positive"
        );
        Ok(Self {
            classic,
            profile_name,
            representation,
            rules_templates,
            endpoint: normalize_endpoint(&get("LLM_KOBOLD_URL")?)?,
            context,
            instruct,
            reasoning,
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
