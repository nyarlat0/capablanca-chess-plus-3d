use std::process::Command;

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
