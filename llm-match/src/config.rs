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
    pub pipeline_mode: crate::pipeline::two_stage::PipelineMode,
    pub two_stage: crate::pipeline::two_stage::Settings,
    pub classic: Option<crate::ClassicConfig>,
    pub pipeline: crate::pipeline::Templates,
    pub stage_max_attempts: usize,
    pub profile_name: String,
    pub representation: crate::Representation,
    pub rules_templates: std::collections::BTreeMap<String, String>,
    pub endpoint: String,
    pub context: ContextTemplate,
    pub instruct: InstructTemplate,
    pub reasoning: ReasoningTemplate,
    pub system: SystemPromptTemplate,
    pub preset: Preset,
    pub stage_presets: StagePresets,
    pub timeout_seconds: u64,
    pub max_attempts: usize,
}

/// Missing stage overrides use the legacy shared preset.
#[derive(Clone, Default)]
pub struct StagePresets {
    pub analyst: Option<Preset>,
    pub grounder: Option<Preset>,
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
            analyst: read("LLM_ANALYST_PRESET")?,
            grounder: read("LLM_GROUNDER_PRESET")?,
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
            Stage::Analyst => &self.stage_presets.analyst,
            Stage::Grounder => &self.stage_presets.grounder,
            Stage::Proposer => &self.stage_presets.proposer,
            Stage::Verifier => &self.stage_presets.verifier,
            Stage::Selector => &self.stage_presets.selector,
        }
        .as_ref()
        .unwrap_or(&self.preset)
    }
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
        let pipeline = match self.pipeline_mode {
            crate::pipeline::two_stage::PipelineMode::SingleStage => {
                "single-stage proposer + first legal move"
            }
            crate::pipeline::two_stage::PipelineMode::TwoStage => {
                "two-stage pipeline + legality gate"
            }
            crate::pipeline::two_stage::PipelineMode::FourStage => {
                "4-stage analyst/proposer/verifier/selector + legality gate"
            }
            crate::pipeline::two_stage::PipelineMode::Legacy => {
                "legacy 5-stage pipeline + legality gate"
            }
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
        let pipeline_mode = get("LLM_PIPELINE")
            .unwrap_or_else(|_| "single-stage".into())
            .parse()?;
        let two_stage = crate::pipeline::two_stage::Settings::load(base, &get)?;
        let pipeline = crate::pipeline::Templates::load(base, &get)?;
        let classic = if classic_mode {
            let env_profile = get("LLM_CLASSIC_PROFILE").ok();

            Some(crate::ClassicConfig::load(
                &base.join(
                    get("LLM_CLASSIC_PROFILES_FILE")
                        .unwrap_or_else(|_| "classic-profiles.toml".into()),
                ),
                profile_override.or(env_profile.as_deref()),
                pipeline.analyst.clone(),
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
        let system = pipeline.analyst.clone();
        let stage_max_attempts: usize = get("LLM_STAGE_MAX_ATTEMPTS")
            .unwrap_or_else(|_| "3".into())
            .parse()?;
        ensure!(stage_max_attempts > 0, "stage attempts must be positive");
        let preset = Preset::from_value(read_json(&base.join(get("LLM_PRESET")?))?)?;
        let stage_presets = StagePresets::load(base, &get)?;
        let timeout_seconds = get("LLM_REQUEST_TIMEOUT_SECONDS")?.parse()?;
        let max_attempts = get("LLM_MAX_ATTEMPTS")?.parse()?;
        ensure!(
            timeout_seconds > 0 && max_attempts > 0,
            "timeout and attempts must be positive"
        );
        Ok(Self {
            pipeline_mode,
            two_stage,
            pipeline,
            stage_max_attempts,
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
            stage_presets,
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

#[cfg(test)]
mod stage_preset_tests {
    use super::*;
    #[test]
    fn optional_overrides_load_relative_to_env_and_missing_ones_fall_back() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let loaded = StagePresets::load(root, |key| {
            if key == "LLM_ANALYST_PRESET" {
                Ok("presets/analyst.json".into())
            } else {
                anyhow::bail!("unset")
            }
        })
        .unwrap();
        assert!(loaded.analyst.is_some());
        assert!(loaded.proposer.is_none());
        let mut c = Config::load_classic_with_profile(Some("no-history")).unwrap();
        c.stage_presets = loaded;
        assert!(std::ptr::eq(
            c.preset_for_stage(crate::pipeline::Stage::Proposer),
            &c.preset
        ));
        assert!(!std::ptr::eq(
            c.preset_for_stage(crate::pipeline::Stage::Analyst),
            &c.preset
        ));
        assert!(StagePresets::load(root, |_| Ok("presets/missing-file.json".into())).is_err());
        assert!(StagePresets::load(root, |_| Ok(String::new())).is_err());
    }
}
