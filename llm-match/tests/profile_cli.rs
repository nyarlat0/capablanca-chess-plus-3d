use std::process::Command;

#[test]
fn classic_system_templates_can_be_overridden_independently_from_env_file_and_process() {
    use std::{
        collections::BTreeMap,
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };
    struct Temp(PathBuf);
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    let dir = Temp(std::env::temp_dir().join(format!(
            "classic-sysprompt-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        )));
    fs::create_dir(&dir.0).unwrap();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    for (file, marker, history) in [
        ("with.json", "WITH_ENV_FILE", true),
        ("without.json", "WITHOUT_ENV_FILE", false),
        ("process.json", "WITH_PROCESS_ENV", true),
    ] {
        fs::write(dir.0.join(file),serde_json::json!({"name":marker,"content":marker,
            "post_history":format!("{{{{fen}}}}\n{{{{ascii}}}}\n{{{{wrong-move}}}}\n{}",if history {"{{classic-san-history}}"} else {""})}).to_string()).unwrap();
    }
    for (file, content) in [
        ("plain.json", "Choose a move."),
        ("empty.json", ""),
        ("optional-history.json", "{{classic-san-history}}"),
    ] {
        fs::write(
            dir.0.join(file),
            serde_json::json!({
                "name": "Experiment", "content": content, "post_history": ""
            })
            .to_string(),
        )
        .unwrap();
    }
    let mut env: BTreeMap<String, String> = dotenvy::from_path_iter(root.join(".env"))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    for value in env.values_mut() {
        if value.starts_with("templates/") || value.starts_with("presets/") {
            *value = root.join(&*value).to_string_lossy().into_owned();
        }
    }
    env.insert(
        "LLM_CLASSIC_PROFILES_FILE".into(),
        root.join("classic-profiles.toml")
            .to_string_lossy()
            .into_owned(),
    );
    env.insert(
        "LLM_CLASSIC_SYSTEM_TEMPLATE_WITH_HISTORY".into(),
        "with.json".into(),
    );
    env.insert(
        "LLM_CLASSIC_SYSTEM_TEMPLATE_NO_HISTORY".into(),
        "without.json".into(),
    );
    fs::write(
        dir.0.join(".env"),
        env.iter()
            .map(|(k, v)| format!("{k}={v}\n"))
            .collect::<String>(),
    )
    .unwrap();
    let run = |profile: &str, override_path: Option<&str>| {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_llm-match"));
        for key in env.keys() {
            cmd.env_remove(key);
        }
        cmd.env("LLM_MATCH_ENV", dir.0.join(".env"));
        if let Some(path) = override_path {
            cmd.env("LLM_CLASSIC_SYSTEM_TEMPLATE_WITH_HISTORY", path);
        }
        cmd.args([
            "--variant",
            "Classic",
            "--llm-classic-profile",
            profile,
            "--print-prompt",
        ])
        .output()
        .unwrap()
    };
    for (profile, marker) in [
        ("with-history", "WITH_ENV_FILE"),
        ("no-history", "WITHOUT_ENV_FILE"),
    ] {
        let result = run(profile, None);
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(String::from_utf8_lossy(&result.stdout).contains(marker));
    }
    let result = run("with-history", Some("process.json"));
    assert!(result.status.success());
    assert!(String::from_utf8_lossy(&result.stdout).contains("WITH_PROCESS_ENV"));
    assert!(
        run("no-history", Some("missing-unused.json"))
            .status
            .success()
    );
    let result = run("with-history", Some("missing-active.json"));
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("missing-active.json"));
    // No mandatory directives or content restrictions, in either profile.
    assert!(run("with-history", Some("without.json")).status.success());
    for file in ["plain.json", "empty.json", "optional-history.json"] {
        assert!(run("with-history", Some(file)).status.success());
        fs::copy(dir.0.join(file), dir.0.join("without.json")).unwrap();
        assert!(run("no-history", None).status.success());
    }
}

#[test]
fn classic_cli_and_environment_are_separate_from_fairy_profiles() {
    let exe = env!("CARGO_BIN_EXE_llm-match");
    for (profile, history) in [("with-history", true), ("no-history", false)] {
        let run = Command::new(exe)
            .env("LLM_PROFILE", "intentionally-not-a-fairy-profile")
            .env("LLM_CLASSIC_PROFILE", "invalid-overridden-by-cli")
            .args([
                "--variant",
                "Classic",
                "--llm-classic-profile",
                profile,
                "--moves",
                "e2e4 e7e5 g1f3 b8c6",
                "--print-prompt",
            ])
            .output()
            .unwrap();
        assert!(
            run.status.success(),
            "{}",
            String::from_utf8_lossy(&run.stderr)
        );
        let text = String::from_utf8(run.stdout).unwrap();
        let _ = history;
        assert!(
            text.contains("FEN: r1bqkbnr/pppp1ppp/2n5/4p3/4P3/5N2/PPPP1PPP/RNBQKB1R w KQkq - 2 3")
        );
    }
    let run = Command::new(exe)
        .env("LLM_CLASSIC_PROFILE", "no-history")
        .args(["--variant", "Classic", "--moves", "e2e4", "--print-prompt"])
        .output()
        .unwrap();
    assert!(run.status.success());
    assert!(!String::from_utf8(run.stdout).unwrap().contains("1. e4"));
}

#[test]
fn cli_profile_overrides_environment_and_unknown_profile_fails_without_network() {
    let exe = env!("CARGO_BIN_EXE_llm-match");
    let run = Command::new(exe)
        .env("LLM_PROFILE", "numeric-no-history")
        .args([
            "--llm-profile",
            "classic",
            "--print-prompt",
            "--moves",
            "e2e4 e7e5",
        ])
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    let prompt = String::from_utf8(run.stdout).unwrap();
    assert!(prompt.contains("ASCII BOARD"));
    assert!(prompt.contains("Black Pawn e7-e5"));
    assert!(prompt.contains("White Pawn e2-e4"));
    let run = Command::new(exe)
        .env("LLM_PROFILE", "numeric-no-history")
        .args(["--print-prompt"])
        .output()
        .unwrap();
    assert!(run.status.success());
    assert!(
        !String::from_utf8(run.stdout)
            .unwrap()
            .contains("ASCII BOARD")
    );
    let run = Command::new(exe)
        .args(["--llm-profile", "does-not-exist", "--print-prompt"])
        .output()
        .unwrap();
    assert!(!run.status.success());
    assert!(String::from_utf8_lossy(&run.stderr).contains("unknown LLM profile"));
}
