use std::process::Command;

#[test]
fn san_replay_matches_uci_and_template_history_is_profile_independent() {
    let run = |format: &str, moves: &str, profile: &str| {
        Command::new(env!("CARGO_BIN_EXE_llm-match"))
            .env("LLM_PIPELINE", "single-stage")
            .args([
                "--variant",
                "Classic",
                "--moves-format",
                format,
                "--moves",
                moves,
                "--llm-classic-profile",
                profile,
                "--print-prompt",
            ])
            .output()
            .unwrap()
    };
    let san = run("san", "e4 e5 Nf3 Nc6", "with-history");
    assert!(
        san.status.success(),
        "{}",
        String::from_utf8_lossy(&san.stderr)
    );
    let uci = run("uci", "e2e4 e7e5 g1f3 b8c6", "with-history");
    assert!(uci.status.success());
    assert_eq!(san.stdout, uci.stdout);
    assert!(String::from_utf8_lossy(&san.stdout).contains("1. e4 e5 2. Nf3 Nc6"));
    let hidden = run("san", "e4 e5 Nf3 Nc6", "no-history");
    assert!(hidden.status.success());
    assert!(String::from_utf8_lossy(&hidden.stdout).contains("1. e4 e5 2. Nf3 Nc6"));
    assert!(!run("san", "e4 e5 Qh9", "with-history").status.success());
    assert!(!run("invalid", "e4", "with-history").status.success());
}

#[test]
fn stage_templates_can_be_overridden_from_env_file_and_process() {
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
    env.insert("LLM_PIPELINE".into(), "legacy".into());
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
    env.insert("LLM_ANALYST_TEMPLATE".into(), "with.json".into());
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
            cmd.env("LLM_ANALYST_TEMPLATE", path);
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
        ("no-history", "WITH_ENV_FILE"),
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
        !run("no-history", Some("missing-unused.json"))
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
        assert!(run("no-history", Some(file)).status.success());
    }
}

#[test]
fn classic_cli_and_environment_are_separate_from_fairy_profiles() {
    let exe = env!("CARGO_BIN_EXE_llm-match");
    for profile in ["with-history", "no-history"] {
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
    assert!(String::from_utf8(run.stdout).unwrap().contains("1. e4"));
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
    assert!(prompt.contains("a b c d e f g h i j"));
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
#[test]
fn cli_initial_fen_and_replay() {
    let exe = env!("CARGO_BIN_EXE_llm-match");
    let fen = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR b KQkq - 0 17";
    let output = Command::new(exe)
        .args([
            "--variant",
            "Classic",
            "--fen",
            fen,
            "--moves",
            "e7e5",
            "--print-prompt",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout)
            .contains("rnbqkbnr/pppp1ppp/8/4p3/8/8/PPPPPPPP/RNBQKBNR w KQkq e6 0 18")
    );
    for args in [vec!["--fen"], vec!["--fen", "bad", "--print-prompt"]] {
        assert!(
            !Command::new(exe)
                .args(args)
                .output()
                .unwrap()
                .status
                .success()
        );
    }
    let state = llm_match::Match::from_fen(
        capablanca_chess_plus::Variant::Classic,
        capablanca_chess_plus::Color::Black,
        fen,
    )
    .unwrap();
    assert!(state.uci_history().is_empty());
    assert_eq!(state.game().position().to_fen(), fen);
}
