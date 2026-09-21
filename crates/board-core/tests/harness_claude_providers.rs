//! `[claude]` launch options for the built-in claude harness: settings
//! providers, extra env, and the fail-closed rules around them.

use std::collections::BTreeMap;

use board_core::config::{ClaudeConfig, Config, RootConfig};
use board_core::harness::{build_invocation, HarnessError, SessionPlan};
use board_core::prompt::EffectiveSettings;
use board_core::protocol::Effort;

const UUID: &str = "11111111-1111-4111-8111-111111111111";

fn settings(model: Option<&str>) -> EffectiveSettings {
    EffectiveSettings {
        harness: "claude".into(),
        model: model.map(str::to_string),
        effort: Some(Effort::High),
        permission_mode: Some("bypassPermissions".into()),
        system_prompt: None,
        fresh_session: false,
        timeout_minutes: None,
    }
}

fn config_with(providers: &[(&str, &str)], env: &[(&str, &str)]) -> Config {
    let pairs = |items: &[(&str, &str)]| -> BTreeMap<String, String> {
        items
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    };
    Config {
        claude: ClaudeConfig {
            providers: pairs(providers),
            env: pairs(env),
            name_sessions: false,
        },
        ..Config::default()
    }
}

fn build(
    config: &Config,
    model: Option<&str>,
) -> Result<board_core::harness::HarnessInvocation, HarnessError> {
    build_invocation(
        "claude",
        config,
        &settings(model),
        &SessionPlan::Mint,
        Some(UUID),
        "task",
    )
}

#[test]
fn without_providers_the_launch_is_unchanged() {
    let invocation = build(&Config::default(), Some("opus")).unwrap();
    assert_eq!(
        invocation.argv,
        [
            "claude",
            "--model",
            "opus",
            "--effort",
            "high",
            "--permission-mode",
            "bypassPermissions",
            "--allowedTools",
            "Bash(board:*)",
            "--session-id",
            UUID,
        ]
    );
    assert!(invocation.env.is_empty());

    // A slash in the model is not provider syntax until providers exist.
    let passthrough = build(&Config::default(), Some("vendor/model")).unwrap();
    assert_eq!(passthrough.argv[1..3], ["--model", "vendor/model"]);
    // No model at all stays valid too.
    let bare = build(&Config::default(), None).unwrap();
    assert_eq!(bare.argv[..2], ["claude", "--effort"]);
}

#[test]
fn a_provider_adds_its_settings_file_right_after_the_executable() {
    let file = tempfile::NamedTempFile::new().unwrap();
    let path = file.path().to_str().unwrap().to_string();
    let config = config_with(&[("DeepSeek", &path)], &[]);

    let invocation = build(&config, Some("DeepSeek/opus")).unwrap();
    assert_eq!(
        invocation.argv[..5],
        ["claude", "--settings", path.as_str(), "--model", "opus"]
    );
    // The rest of the launch keeps its established order, session flags last.
    assert_eq!(
        invocation.argv[5..],
        [
            "--effort",
            "high",
            "--permission-mode",
            "bypassPermissions",
            "--allowedTools",
            "Bash(board:*)",
            "--session-id",
            UUID,
        ]
    );
    // Only the first slash splits, so the model may contain more of them.
    let nested = build(&config, Some("DeepSeek/vendor/model")).unwrap();
    assert_eq!(nested.argv[3..5], ["--model", "vendor/model"]);
}

#[test]
fn an_empty_settings_value_means_the_logged_in_account() {
    let config = config_with(&[("official", "")], &[]);
    let invocation = build(&config, Some("official/opus")).unwrap();
    assert_eq!(invocation.argv[..3], ["claude", "--model", "opus"]);
    assert!(!invocation.argv.iter().any(|a| a == "--settings"));
}

#[test]
fn configured_providers_make_the_prefix_mandatory() {
    let file = tempfile::NamedTempFile::new().unwrap();
    let path = file.path().to_str().unwrap();
    let config = config_with(&[("DeepSeek", path), ("official", "")], &[]);
    let required = HarnessError::ClaudeProviderRequired("DeepSeek, official".into());

    for model in [None, Some("opus"), Some("/opus"), Some("DeepSeek/")] {
        assert_eq!(build(&config, model).unwrap_err(), required, "{model:?}");
    }
    assert!(required.to_string().contains("DeepSeek, official"));
}

#[test]
fn an_unlisted_provider_is_rejected() {
    let config = config_with(&[("official", "")], &[]);
    assert_eq!(
        build(&config, Some("nope/opus")).unwrap_err(),
        HarnessError::UnknownClaudeProvider("nope".into())
    );
}

#[test]
fn a_provider_settings_file_must_exist_at_an_absolute_path() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("gone.json");
    let missing = missing.to_str().unwrap();
    let directory = dir.path().to_str().unwrap();

    for bad in [missing, directory, "relative/settings.json"] {
        let config = config_with(&[("p", bad)], &[]);
        assert_eq!(
            build(&config, Some("p/opus")).unwrap_err(),
            HarnessError::ClaudeProviderSettingsMissing(bad.into()),
            "{bad}"
        );
    }
}

#[test]
fn claude_env_rides_the_invocation_and_board_keys_are_reserved() {
    let config = config_with(&[], &[("ZED", "last"), ("CLAUDE_ROLE", "card")]);
    let invocation = build(&config, Some("opus")).unwrap();
    assert_eq!(
        invocation.env,
        [
            ("CLAUDE_ROLE".to_string(), "card".to_string()),
            ("ZED".to_string(), "last".to_string()),
        ]
    );

    let reserved = config_with(&[], &[("BOARD_CARD_ID", "7")]);
    assert_eq!(
        build(&reserved, Some("opus")).unwrap_err(),
        HarnessError::ClaudeReservedEnv("BOARD_CARD_ID".into())
    );
}

#[test]
fn the_claude_table_does_not_leak_into_other_harnesses() {
    let config = config_with(&[("official", "")], &[("CLAUDE_ROLE", "card")]);
    let mut pi = settings(Some("opus"));
    pi.harness = "pi".into();
    pi.permission_mode = None;
    let invocation =
        build_invocation("pi", &config, &pi, &SessionPlan::Mint, Some(UUID), "task").unwrap();
    assert!(invocation.env.is_empty());
    assert!(invocation.argv.iter().any(|a| a == "opus"));
}

#[test]
fn the_claude_table_parses_from_toml_and_defaults_when_absent() {
    let parsed = RootConfig::from_toml(
        r#"
[claude]
name_sessions = true

[claude.providers]
"公司Model" = "/abs/_Model.json"
official = ""

[claude.env]
CLAUDE_ROLE = "card"
"#,
    )
    .unwrap()
    .board
    .claude;
    assert!(parsed.name_sessions);
    assert_eq!(parsed.providers["公司Model"], "/abs/_Model.json");
    assert_eq!(parsed.providers["official"], "");
    assert_eq!(parsed.env["CLAUDE_ROLE"], "card");

    let absent = RootConfig::from_toml("max_concurrent = 2").unwrap();
    assert_eq!(absent.board.claude, ClaudeConfig::default());
    // Defaults are not written back out, so an untouched config round-trips.
    assert!(!toml::to_string(&absent.board).unwrap().contains("claude"));
}
