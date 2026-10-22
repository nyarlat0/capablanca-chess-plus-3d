//! Files used only by explicit fairy-variant configurations.
use anyhow::{Context, Result};
use std::{collections::BTreeMap, path::Path};

pub(crate) fn load(
    base: &Path,
    get: impl Fn(&str) -> Result<String>,
    selected: Option<&str>,
) -> Result<(String, crate::Representation, BTreeMap<String, String>)> {
    let profiles_path = base.join(
        get("LLM_PROFILES_FILE")
            .unwrap_or_else(|_| "compat/fairy/representation-profiles.toml".into()),
    );
    let profiles = crate::ProfileFile::parse(
        &std::fs::read_to_string(&profiles_path)
            .with_context(|| format!("cannot read {}", profiles_path.display()))?,
    )?;
    let env_profile = get("LLM_PROFILE").ok();
    let (name, representation) = profiles.select(selected.or(env_profile.as_deref()))?;
    let rules_dir = base.join(get("LLM_RULES_DIR").unwrap_or_else(|_| "compat/fairy/rules".into()));
    let mut rules = BTreeMap::new();
    for variant in capablanca_chess_plus::Variant::ALL {
        if variant == capablanca_chess_plus::Variant::Classic {
            continue;
        }
        let key = super::rules::variant_key(variant);
        let path = rules_dir.join(format!("{key}.txt"));
        rules.insert(
            key.into(),
            std::fs::read_to_string(&path)
                .with_context(|| format!("cannot read rules {}", path.display()))?,
        );
    }
    Ok((name, representation, rules))
}
