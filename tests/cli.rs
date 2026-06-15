use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;

#[test]
fn init_and_list_work() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("openfusion.toml");

    Command::cargo_bin("openfusion")
        .unwrap()
        .args(["init", "--no-probe", "--output", config.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("Wrote"));

    Command::cargo_bin("openfusion")
        .unwrap()
        .args(["list", "--config", config.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("Runners"))
        .stdout(predicate::str::contains("Strategies"));
}

#[test]
fn process_runner_consensus_works_with_weighted_candidates() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("openfusion.toml");
    fs::write(
        &config,
        r#"
[[runners]]
name = "a"
kind = "process"
command = "printf"
args = ["candidate-a: safe answer"]
weight = 0.2

[[runners]]
name = "b"
kind = "process"
command = "printf"
args = ["candidate-b: better answer"]
weight = 1.0

[[runners]]
name = "judge"
kind = "process"
command = "printf"
args = ["final-from-judge"]
weight = 1.0

[[strategies]]
name = "consensus"
type = "consensus"
runners = ["a", "b"]
judge = "judge"
"#,
    )
    .unwrap();

    Command::cargo_bin("openfusion")
        .unwrap()
        .args([
            "ask",
            "--config",
            config.to_str().unwrap(),
            "--strategy",
            "consensus",
            "hello",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("final-from-judge"));
}

#[test]
fn ask_emits_progress_to_stderr_without_corrupting_stdout() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("openfusion.toml");
    fs::write(
        &config,
        r#"
[[runners]]
name = "candidate"
kind = "process"
command = "printf"
args = ["candidate answer"]
weight = 1.0

[[runners]]
name = "judge"
kind = "process"
command = "printf"
args = ["final answer"]
weight = 1.0

[[strategies]]
name = "consensus"
type = "consensus"
runners = ["candidate"]
judge = "judge"
"#,
    )
    .unwrap();

    Command::cargo_bin("openfusion")
        .unwrap()
        .args([
            "ask",
            "--config",
            config.to_str().unwrap(),
            "--strategy",
            "consensus",
            "hello",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("final answer"))
        .stderr(predicate::str::contains("openfusion:"))
        .stderr(predicate::str::contains("calling candidate"))
        .stderr(predicate::str::contains("judging with judge"));
}

#[test]
fn quiet_suppresses_progress_and_non_tty_default_json_stdout_stays_parseable() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("openfusion.toml");
    fs::write(
        &config,
        r#"
[[runners]]
name = "candidate"
kind = "process"
command = "printf"
args = ["candidate answer"]
weight = 1.0

[[strategies]]
name = "fallback"
type = "fallback"
runners = ["candidate"]
"#,
    )
    .unwrap();

    let assert = Command::cargo_bin("openfusion")
        .unwrap()
        .args([
            "ask",
            "--config",
            config.to_str().unwrap(),
            "--strategy",
            "fallback",
            "--quiet",
            "hello",
        ])
        .assert()
        .success()
        .stderr(predicate::str::is_empty());
    let out = assert.get_output().stdout.clone();
    let json: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(json["final"], "candidate answer");
}

#[test]
fn explicit_markdown_overrides_non_tty_json_default() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("openfusion.toml");
    fs::write(
        &config,
        r#"
[[runners]]
name = "candidate"
kind = "process"
command = "printf"
args = ["candidate answer"]
weight = 1.0

[[strategies]]
name = "fallback"
type = "fallback"
runners = ["candidate"]
"#,
    )
    .unwrap();

    Command::cargo_bin("openfusion")
        .unwrap()
        .args([
            "ask",
            "--config",
            config.to_str().unwrap(),
            "--strategy",
            "fallback",
            "--format",
            "markdown",
            "--quiet",
            "hello",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("## OpenFusion verdict"))
        .stdout(predicate::str::contains("candidate answer"))
        .stderr(predicate::str::is_empty());
}

#[test]
fn runner_repetition_expands_for_best_of_n_json() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("openfusion.toml");
    fs::write(
        &config,
        r#"
[[runners]]
name = "sample"
kind = "process"
command = "printf"
args = ["sample-answer"]
weight = 1.0

[[runners]]
name = "judge"
kind = "process"
command = "printf"
args = ["judged"]
weight = 1.0

[[strategies]]
name = "best-of-n"
type = "best_of_n"
runners = ["sample*3"]
judge = "judge"
"#,
    )
    .unwrap();

    let out = Command::cargo_bin("openfusion")
        .unwrap()
        .args([
            "ask",
            "--config",
            config.to_str().unwrap(),
            "--strategy",
            "best-of-n",
            "--format",
            "json",
            "hello",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(json["candidates"].as_array().unwrap().len(), 3);
    assert_eq!(json["final"], "judged");
}

#[test]
fn keys_list_accepts_explicit_config_path() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("keys.toml");
    fs::write(
        &config,
        r#"
[[runners]]
name = "custom-api"
kind = "openai_compat"
base_url = "https://example.invalid/v1"
model = "x"
api_key_ref = "my-custom-key"
api_key_env = "MY_CUSTOM_KEY"

[[strategies]]
name = "fallback"
type = "fallback"
runners = ["custom-api"]
"#,
    )
    .unwrap();

    Command::cargo_bin("openfusion")
        .unwrap()
        .args(["keys", "list", "--config", config.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("my-custom-key"));
}

#[test]
fn skill_install_dry_run_prints_skills_command() {
    Command::cargo_bin("openfusion")
        .unwrap()
        .args(["skills", "install", "--dry-run"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "npx skills add nachoiacovino/openfusion",
        ));
}

#[test]
fn skill_install_dry_run_forwards_global_agent_and_skill_flags() {
    Command::cargo_bin("openfusion")
        .unwrap()
        .args([
            "skills",
            "install",
            "--dry-run",
            "--global",
            "--agent",
            "claude-code",
            "--skill",
            "openfusion-consensus",
            "--yes",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "npx skills add nachoiacovino/openfusion --global --agent claude-code --skill openfusion-consensus --yes",
        ));
}

#[test]
fn race_returns_first_successful_candidate() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("openfusion.toml");
    fs::write(
        &config,
        r#"
max_jobs = 2

[[runners]]
name = "slow"
kind = "process"
command = "sh"
args = ["-c", "sleep 1; printf slow"]
weight = 1.0

[[runners]]
name = "fast"
kind = "process"
command = "printf"
args = ["fast"]
weight = 1.0

[[strategies]]
name = "race"
type = "race"
runners = ["slow", "fast"]
"#,
    )
    .unwrap();

    Command::cargo_bin("openfusion")
        .unwrap()
        .args([
            "ask",
            "--config",
            config.to_str().unwrap(),
            "--strategy",
            "race",
            "hello",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("fast"));
}

#[test]
fn ci_gate_outputs_schema_and_passes_without_fail_marker() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("openfusion.toml");
    let file = dir.path().join("subject.txt");
    fs::write(&file, "safe").unwrap();
    fs::write(
        &config,
        r#"
[[runners]]
name = "candidate"
kind = "process"
command = "printf"
args = ["looks okay"]
weight = 1.0

[[runners]]
name = "judge"
kind = "process"
command = "printf"
args = ["STATUS: PASS\nNo blockers."]
weight = 1.0

[[strategies]]
name = "review"
type = "review"
runners = ["candidate"]
judge = "judge"
"#,
    )
    .unwrap();

    let out = Command::cargo_bin("openfusion")
        .unwrap()
        .args([
            "ci-gate",
            "--config",
            config.to_str().unwrap(),
            file.to_str().unwrap(),
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(json["schema_version"], "openfusion.ci_gate.v1");
    assert_eq!(json["status"], "pass");
    assert_eq!(json["exit_code"], 0);
}

#[test]
fn ci_gate_fails_on_fail_marker() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("openfusion.toml");
    let file = dir.path().join("subject.txt");
    fs::write(&file, "unsafe").unwrap();
    fs::write(
        &config,
        r#"
[[runners]]
name = "candidate"
kind = "process"
command = "printf"
args = ["has blocker"]
weight = 1.0

[[runners]]
name = "judge"
kind = "process"
command = "printf"
args = ["STATUS: FAIL\nBlocker found."]
weight = 1.0

[[strategies]]
name = "review"
type = "review"
runners = ["candidate"]
judge = "judge"
"#,
    )
    .unwrap();

    Command::cargo_bin("openfusion")
        .unwrap()
        .args([
            "ci-gate",
            "--config",
            config.to_str().unwrap(),
            file.to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stdout(predicate::str::contains("openfusion.ci_gate.v1"))
        .stdout(predicate::str::contains("\"status\": \"fail\""));
}

#[test]
fn process_runner_receives_reasoning_env_and_extra_json() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("openfusion.toml");
    fs::write(
        &config,
        r#"
[[runners]]
name = "envcheck"
kind = "process"
command = "sh"
args = ["-c", "printf '%s|%s|%s' \"$OPENFUSION_REASONING_EFFORT\" \"$OPENFUSION_REASONING_BUDGET_TOKENS\" \"$OPENFUSION_EXTRA_BODY_JSON\""]
weight = 1.0
reasoning_effort = "high"
reasoning_budget_tokens = 4096
extra_body = { top_p = 0.9, seed = 42 }

[[strategies]]
name = "race"
type = "race"
runners = ["envcheck"]
"#,
    )
    .unwrap();

    Command::cargo_bin("openfusion")
        .unwrap()
        .args([
            "ask",
            "--config",
            config.to_str().unwrap(),
            "--strategy",
            "race",
            "hello",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("high|4096|"))
        .stdout(predicate::str::contains("top_p"))
        .stdout(predicate::str::contains("seed"));
}

#[test]
fn list_shows_reasoning_profile() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("openfusion.toml");
    fs::write(
        &config,
        r#"
[[runners]]
name = "deep"
kind = "process"
command = "printf"
args = ["ok"]
weight = 1.0
reasoning_effort = "medium"
reasoning_budget_tokens = 1234
reasoning_include = true
reasoning_extra = { vendor_knob = "yes" }
extra_body = { top_p = 0.8 }

[[strategies]]
name = "race"
type = "race"
runners = ["deep"]
"#,
    )
    .unwrap();

    Command::cargo_bin("openfusion")
        .unwrap()
        .args(["list", "--config", config.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("effort=medium"))
        .stdout(predicate::str::contains("budget=1234"))
        .stdout(predicate::str::contains("include=true"))
        .stdout(predicate::str::contains("reasoning_extra"))
        .stdout(predicate::str::contains("extra_body"));
}

#[test]
fn init_local_writes_project_override() {
    let dir = tempfile::tempdir().unwrap();
    Command::cargo_bin("openfusion")
        .unwrap()
        .current_dir(dir.path())
        .args(["init", "--local", "--no-probe"])
        .assert()
        .success()
        .stdout(predicate::str::contains("openfusion.toml"));
    assert!(dir.path().join("openfusion.toml").exists());
}

#[test]
fn openrouter_key_ref_is_optional_with_env_fallback_for_doctor() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("openfusion.toml");
    fs::write(
        &config,
        r#"
[[runners]]
name = "or"
kind = "openrouter"
model = "fake/model"
api_key_ref = "definitely-missing-test-key"
api_key_env = "OPENFUSION_TEST_KEY"

[[strategies]]
name = "race"
type = "race"
runners = ["or"]
"#,
    )
    .unwrap();

    Command::cargo_bin("openfusion")
        .unwrap()
        .env("OPENFUSION_TEST_KEY", "test-secret")
        .args(["doctor", "--config", config.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("env:OPENFUSION_TEST_KEY"));
}

#[test]
fn anthropic_key_ref_is_optional_with_env_fallback_for_doctor() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("openfusion.toml");
    fs::write(
        &config,
        r#"
[[runners]]
name = "claude-api"
kind = "anthropic"
model = "claude-sonnet-4-5"
api_key_ref = "missing-test-key"
api_key_env = "OPENFUSION_TEST_ANTHROPIC_KEY"

[[strategies]]
name = "race"
type = "race"
runners = ["claude-api"]
"#,
    )
    .unwrap();

    Command::cargo_bin("openfusion")
        .unwrap()
        .env("OPENFUSION_TEST_ANTHROPIC_KEY", "test-key")
        .args(["doctor", "--config", config.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("claude-api"))
        .stdout(predicate::str::contains(
            "env:OPENFUSION_TEST_ANTHROPIC_KEY",
        ));
}
