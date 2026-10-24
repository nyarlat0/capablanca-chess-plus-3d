use anyhow::{Context, Result, ensure};
use prompt_core::{ContextTemplate, Preset, ReasoningTemplate};
use serde::de::DeserializeOwned;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

#[derive(Clone)]
pub struct Config {
    /// Present for Classic (the default). Fairy-only fields below are inert then.
    pub classic: Option<crate::ClassicConfig>,
    pub pipeline: crate::pipeline::Templates,
    pub stage_max_attempts: usize,
    pub profile_name: String,
    /// Compatibility only: used when `classic` is None.
    pub representation: crate::Representation,
    /// Compatibility only: Classic never reads fairy rule files.
    pub rules_templates: std::collections::BTreeMap<String, String>,
    pub endpoint: String,
    pub context: ContextTemplate,
    pub reasoning: ReasoningTemplate,
    pub preset: Preset,
    pub stage_presets: StagePresets,
    pub timeout_seconds: u64,
    pub max_attempts: usize,
    /// Reserve for vision embeddings, in addition to Kobold's text token count.
    pub image_token_reserve: u64,
}

/// Missing stage overrides use the shared fallback preset.
#[derive(Clone, Default)]
pub struct StagePresets {
    pub one_grounder: Option<Preset>,
    pub pin_analyzer: Option<Preset>,
    pub grounder: Option<Preset>,
    pub threat_analyzer: Option<Preset>,
    pub proposer: Option<Preset>,
    pub verifier: Option<Preset>,
    pub selector: Option<Preset>,
}

impl StagePresets {
    fn load(base: &Path, get: impl Fn(&str) -> Result<String>) -> Result<Self> {
        let read = |key: &str| -> Result<Option<Preset>> {
            let Ok(value) = get(key) else { return Ok(None) };
            ensure!(!value.trim().is_empty(), "{key} must name a preset file");
            let path = base.join(value);
            let preset = Preset::from_value(read_json(&path)?)
                .with_context(|| format!("invalid {key}: {}", path.display()))?;
            preset
                .generation_length()
                .with_context(|| format!("invalid generation budget in {key}"))?;
            Ok(Some(preset))
        };
        Ok(Self {
            grounder: read("LLM_GROUNDER_PRESET")?,
            threat_analyzer: read("LLM_THREAT_ANALYZER_PRESET")?,
            one_grounder: read("LLM_ONE_GROUNDER_PRESET")?,
            pin_analyzer: read("LLM_PIN_ANALYZER_PRESET")?,
            proposer: read("LLM_PROPOSER_PRESET")?,
            verifier: read("LLM_VERIFIER_PRESET")?,
            selector: read("LLM_SELECTOR_PRESET")?,
        })
    }
}

impl Config {
    pub fn preset_for_stage(&self, stage: crate::pipeline::Stage) -> &Preset {
        use crate::pipeline::Stage;
        match stage {
            Stage::OneGrounder | Stage::OneGrounderFull => {
                return self
                    .stage_presets
                    .one_grounder
                    .as_ref()
                    .unwrap_or_else(|| self.preset_for_stage(Stage::Grounder));
            }
            Stage::Grounder => &self.stage_presets.grounder,
            Stage::ThreatAnalyzer => {
                return self.stage_presets.threat_analyzer.as_ref()
                    .unwrap_or_else(|| self.preset_for_stage(Stage::Proposer));
            }
            Stage::PinAnalyzer => &self.stage_presets.pin_analyzer,
            Stage::Proposer => &self.stage_presets.proposer,
            Stage::Verifier => &self.stage_presets.verifier,
            Stage::Selector => &self.stage_presets.selector,
        }
        .as_ref()
        .unwrap_or(&self.preset)
    }
    pub fn load_for_variant(variant: capablanca_chess_plus::Variant) -> Result<Self> {
        if variant != capablanca_chess_plus::Variant::Classic {
            let profile = crate::profile::cli_profile(std::env::args().skip(1))?;
            return Self::load_with_profile(profile.as_deref());
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
        let pipeline = if self.classic.is_some() {
            "grounder/proposer(inspect_line + final SAN) + engine legality gate"
        } else {
            "grounder/proposer/verifier/selector + legality gate"
        };
        if let Some(c) = &self.classic {
            format!(
                "{pipeline} | classic chess | history: {} | output: SAN",
                if c.include_history {
                    "full SAN"
                } else {
                    "none"
                }
            )
        } else {
            format!("{pipeline} | {}", self.representation.summary())
        }
    }
    /// Relative template paths are relative to the env file. Process env wins.
    pub fn load() -> Result<Self> {
        Self::load_for_variant(capablanca_chess_plus::Variant::Classic)
    }
    pub fn load_with_profile(profile: Option<&str>) -> Result<Self> {
        let path = std::env::var_os("LLM_MATCH_ENV")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/.env")));
        Self::from_env_file_with_profile(&path, profile)
    }

    pub fn from_env_file(path: &Path) -> Result<Self> {
        Self::from_env_mode(path, None, true)
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
        if let Ok(mode) = get("LLM_PIPELINE") {
            ensure!(
                mode == "four-stage",
                "only four-stage is supported; remove obsolete LLM_PIPELINE={mode}"
            );
        }
        let pipeline = crate::pipeline::Templates::load_for_variant(base, get, classic_mode)?;
        let classic = if classic_mode {
            let env_profile = get("LLM_CLASSIC_PROFILE").ok();

            Some(crate::ClassicConfig::load(
                &base.join(
                    get("LLM_CLASSIC_PROFILES_FILE")
                        .unwrap_or_else(|_| "classic-profiles.toml".into()),
                ),
                profile_override.or(env_profile.as_deref()),
            )?)
        } else {
            None
        };
        let (profile_name, representation, rules_templates) = if let Some(c) = &classic {
            (
                format!("classic-chess/{}", c.name),
                crate::Representation::default(),
                Default::default(),
            )
        } else {
            crate::compat::config::load(base, get, profile_override)?
        };
        let context = read_json(&base.join(get("LLM_CONTEXT_TEMPLATE")?))?;
        let reasoning = read_json(&base.join(get("LLM_REASONING_TEMPLATE")?))?;
        let stage_max_attempts: usize = get("LLM_STAGE_MAX_ATTEMPTS")
            .unwrap_or_else(|_| "3".into())
            .parse()?;
        ensure!(stage_max_attempts > 0, "stage attempts must be positive");
        let preset = Preset::from_value(read_json(&base.join(get("LLM_PRESET")?))?)?;
        let stage_presets = StagePresets::load(base, get)?;
        let timeout_seconds = get("LLM_REQUEST_TIMEOUT_SECONDS")?.parse()?;
        let max_attempts = get("LLM_MAX_ATTEMPTS")?.parse()?;
        ensure!(
            timeout_seconds > 0 && max_attempts > 0,
            "timeout and attempts must be positive"
        );
        Ok(Self {
            pipeline,
            stage_max_attempts,
            classic,
            profile_name,
            representation,
            rules_templates,
            endpoint: normalize_endpoint(&get("LLM_KOBOLD_URL")?)?,
            context,
            reasoning,
            preset,
            stage_presets,
            timeout_seconds,
            max_attempts,
            image_token_reserve: get("LLM_IMAGE_TOKEN_RESERVE")
                .unwrap_or_else(|_| "4096".into())
                .parse()?,
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

#[cfg(test)]
mod stage_preset_tests {
    use super::*;
    #[test]
    fn optional_overrides_load_relative_to_env_and_missing_ones_fall_back() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let loaded = StagePresets::load(root, |key| {
            if key == "LLM_GROUNDER_PRESET" {
                Ok("presets/grounder.json".into())
            } else {
                anyhow::bail!("unset")
            }
        })
        .unwrap();
        assert!(loaded.grounder.is_some());
        assert!(loaded.proposer.is_none());
        let mut c = Config::load_classic_with_profile(Some("no-history")).unwrap();
        c.stage_presets = loaded;
        assert!(std::ptr::eq(
            c.preset_for_stage(crate::pipeline::Stage::Proposer),
            &c.preset
        ));
        assert!(!std::ptr::eq(
            c.preset_for_stage(crate::pipeline::Stage::Grounder),
            &c.preset
        ));
        assert!(StagePresets::load(root, |_| Ok("presets/missing-file.json".into())).is_err());
        assert!(StagePresets::load(root, |_| Ok(String::new())).is_err());
    }
}
