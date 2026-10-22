use std::process::{Command, Output};

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_llm-match"))
        .env_remove("LLM_PIPELINE")
        .args(args)
        .output()
        .unwrap()
}
fn success(output: Output) -> String {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn classic_is_default_and_san_replay_matches_uci() {
    let san = success(run(&[
        "--moves-format",
        "san",
        "--moves",
        "e4 e5 Nf3 Nc6",
        "--print-prompt",
    ]));
    let uci = success(run(&["--moves", "e2e4 e7e5 g1f3 b8c6", "--print-prompt"]));
    assert_eq!(san, uci);
    assert!(san.contains("r1bqkbnr/pppp1ppp/2n5/4p3/4P3/5N2/PPPP1PPP/RNBQKB1R"));
    assert!(
        !run(&[
            "--moves-format",
            "san",
            "--moves",
            "e4 e5 Qh9",
            "--print-prompt"
        ])
        .status
        .success()
    );
}
#[test]
fn cli_initial_fen_and_replay() {
    let text = success(run(&[
        "--fen",
        "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR b KQkq - 0 17",
        "--moves-format",
        "san",
        "--moves",
        "e5",
        "--print-prompt",
    ]));
    assert!(text.contains("rnbqkbnr/pppp1ppp/8/4p3/8/8/PPPPPPPP/RNBQKBNR w KQkq e6 0 18"));
    assert!(!run(&["--fen", "bad", "--print-prompt"]).status.success());
}
#[test]
fn deleted_modes_and_unknown_stages_fail_before_network() {
    for mode in ["single-stage", "two-stage", "legacy"] {
        let output = Command::new(env!("CARGO_BIN_EXE_llm-match"))
            .env("LLM_PIPELINE", mode)
            .arg("--print-prompt")
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("only four-stage"));
    }
    assert!(
        !run(&["--stage", "analyst", "--print-prompt"])
            .status
            .success()
    );
    assert!(
        !run(&["--stage", "proposer", "--print-prompt"])
            .status
            .success()
    );
}
#[test]
fn debug_grounder_preview_matches_full_pipeline_first_request() {
    let normal = success(run(&["--print-prompt"]));
    let debug = success(run(&["--stage", "grounder", "--print-prompt"]));
    assert_eq!(normal, debug);
}
#[test]
fn fairy_configuration_is_explicit_and_separate() {
    let text = success(run(&[
        "--variant",
        "Gothic",
        "--llm-profile",
        "numeric-no-history",
        "--print-prompt",
    ]));
    assert!(text.contains("width: 10"));
    assert!(!text.contains("ASCII BOARD"));
    let output = Command::new(env!("CARGO_BIN_EXE_llm-match"))
        .env_remove("LLM_PIPELINE")
        .env("LLM_PROFILE", "nonexistent")
        .arg("--print-prompt")
        .output()
        .unwrap();
    assert!(output.status.success()); // unrelated fairy profile cannot break Classic
    assert!(
        !run(&[
            "--variant",
            "Gothic",
            "--llm-profile",
            "nonexistent",
            "--print-prompt"
        ])
        .status
        .success()
    );
}

#[test]
fn stage_files_are_loaded_and_routed_without_running_preceding_stages() {
    use std::{
        fs,
        time::{SystemTime, UNIX_EPOCH},
    };
    struct Temp(std::path::PathBuf);
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    let dir = Temp(std::env::temp_dir().join(format!("llm-stage-inputs-{}-{}",
        std::process::id(),SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos())));
    fs::create_dir(&dir.0).unwrap();
    let facts = dir.0.join("facts.txt");
    let candidate = dir.0.join("candidate.json");
    let accepted = dir.0.join("accepted.json");
    fs::write(&facts, "SUPPLIED_STATIC_FACTS").unwrap();
    fs::write(
        &candidate,
        r#"{"id":"c1","move":"e4","from":"e2","to":"e4","rationale":"PRIVATE_IDEA"}"#,
    )
    .unwrap();
    fs::write(
        &accepted,
        r#"[{"id":"c1","move":"e4","from":"e2","to":"e4","tactical_result":"SUPPLIED_OUTCOME"}]"#,
    )
    .unwrap();
    let facts = facts.to_str().unwrap();
    let candidate = candidate.to_str().unwrap();
    let accepted = accepted.to_str().unwrap();
    let p = success(run(&[
        "--stage",
        "proposer",
        "--analysis-file",
        facts,
        "--print-prompt",
    ]));
    assert!(p.contains("SUPPLIED_STATIC_FACTS"));
    let v = success(run(&[
        "--stage",
        "verifier",
        "--analysis-file",
        facts,
        "--candidate-file",
        candidate,
        "--print-prompt",
    ]));
    assert!(v.contains("SUPPLIED_STATIC_FACTS"));
    assert!(!v.contains("PRIVATE_IDEA"));
    let s = success(run(&[
        "--stage",
        "selector",
        "--accepted-candidates-file",
        accepted,
        "--print-prompt",
    ]));
    assert!(s.contains("SUPPLIED_OUTCOME"));
    assert!(!s.contains("SUPPLIED_STATIC_FACTS"));
    assert!(
        !run(&["--analysis-file", facts, "--print-prompt"])
            .status
            .success()
    );
    assert!(
        !run(&[
            "--stage",
            "grounder",
            "--analysis-file",
            facts,
            "--print-prompt"
        ])
        .status
        .success()
    );
    assert!(
        !run(&[
            "--stage",
            "verifier",
            "--analysis-file",
            facts,
            "--candidate-file",
            facts,
            "--print-prompt"
        ])
        .status
        .success()
    );
}
