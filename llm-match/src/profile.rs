use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StateFormat {
    #[default]
    PiecesAscii,
    Numeric,
    NumericAscii,
}
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HistoryMode {
    None,
    #[default]
    Full,
    LastN,
}
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HistoryFormat {
    #[default]
    Semantic,
    Numeric,
}
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OutputFormat {
    Uci,
    NumericJson,
    #[default]
    Semantic,
    Numeric,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Representation {
    pub state_format: StateFormat,
    pub history_mode: HistoryMode,
    pub history_format: HistoryFormat,
    pub output_format: OutputFormat,
    pub history_count: Option<usize>,
}
impl Representation {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.history_mode != HistoryMode::LastN || self.history_count.is_some_and(|n| n > 0),
            "last_n requires a positive history_count"
        );
        if self.history_mode != HistoryMode::None {
            let expected = match self.history_format {
                HistoryFormat::Semantic => OutputFormat::Semantic,
                HistoryFormat::Numeric => OutputFormat::Numeric,
            };
            ensure!(
                self.output_format == expected,
                "history_format={} requires output_format={} when history is enabled (use history_mode=none for independent output)",
                self.history_format.name(),
                expected.name()
            );
        }
        Ok(())
    }
    pub fn history_start(&self, len: usize) -> usize {
        match self.history_mode {
            HistoryMode::None => len,
            HistoryMode::Full => 0,
            HistoryMode::LastN => len.saturating_sub(self.history_count.unwrap_or(0)),
        }
    }
    pub fn summary(&self) -> String {
        format!(
            "state: {} | history: {}{} ({}) | output: {}",
            self.state_format.name(),
            self.history_mode.name(),
            self.history_count
                .map(|n| format!("/{n}"))
                .unwrap_or_default(),
            self.history_format.name(),
            self.output_format.name()
        )
    }
}
macro_rules! names { ($t:ty, $($v:ident => $s:literal),+) => { impl $t { pub const fn name(self) -> &'static str { match self { $(Self::$v => $s),+ } } } }; }
names!(StateFormat, PiecesAscii => "pieces_ascii", Numeric => "numeric", NumericAscii => "numeric_ascii");
names!(HistoryMode, None => "none", Full => "full", LastN => "last_n");
names!(HistoryFormat, Semantic => "semantic", Numeric => "numeric");
names!(OutputFormat, Uci => "uci", NumericJson => "numeric_json", Semantic => "semantic", Numeric => "numeric");

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileFile {
    pub active_profile: String,
    pub profiles: BTreeMap<String, Representation>,
}
impl ProfileFile {
    pub fn parse(text: &str) -> Result<Self> {
        let file: Self = toml::from_str(text).context("invalid LLM profile TOML")?;
        for (name, profile) in &file.profiles {
            profile
                .validate()
                .with_context(|| format!("profile {name}"))?;
        }
        ensure!(
            file.profiles.contains_key(&file.active_profile),
            "unknown active_profile {}",
            file.active_profile
        );
        Ok(file)
    }
    pub fn select(&self, override_name: Option<&str>) -> Result<(String, Representation)> {
        let name = override_name.unwrap_or(&self.active_profile);
        Ok((
            name.into(),
            self.profiles
                .get(name)
                .with_context(|| {
                    format!(
                        "unknown LLM profile {name}; available: {}",
                        self.profiles.keys().cloned().collect::<Vec<_>>().join(", ")
                    )
                })?
                .clone(),
        ))
    }
}

pub fn cli_profile(args: impl IntoIterator<Item = String>) -> Result<Option<String>> {
    let mut args = args.into_iter();
    let mut selected = None;
    while let Some(arg) = args.next() {
        if arg == "--llm-profile" {
            let value = args.next().context("--llm-profile requires a name")?;
            ensure!(
                !value.is_empty() && !value.starts_with('-'),
                "--llm-profile requires a name"
            );
            selected = Some(value);
        } else if let Some(value) = arg.strip_prefix("--llm-profile=") {
            ensure!(!value.is_empty(), "--llm-profile requires a name");
            selected = Some(value.into());
        }
    }
    Ok(selected)
}
