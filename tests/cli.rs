use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT_FILE: AtomicUsize = AtomicUsize::new(0);

struct TempConfig(PathBuf);

impl TempConfig {
    fn new(text: &str) -> Self {
        let base = if Path::new("/tmp/opencode").is_dir() {
            PathBuf::from("/tmp/opencode")
        } else {
            std::env::temp_dir()
        };
        let path = base.join(format!(
            "geohueo-test-{}-{}.toml",
            std::process::id(),
            NEXT_FILE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::write(&path, text).unwrap();
        Self(path)
    }
}

impl Drop for TempConfig {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn isolated_command() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_geohueo"));
    command.env_remove("GEOHUEO_CONFIG");
    command.env_remove("GEOHUEO_PROFILE");
    command.env(
        "XDG_CONFIG_HOME",
        std::env::temp_dir().join(format!("geohueo-test-no-config-{}", std::process::id())),
    );
    command.env(
        "APPDATA",
        std::env::temp_dir().join(format!("geohueo-test-no-config-{}", std::process::id())),
    );
    command
}

#[test]
fn help_succeeds_without_a_lookup() {
    let output = isolated_command().arg("--help").output().unwrap();

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let help = String::from_utf8(output.stdout).unwrap();
    assert!(help.starts_with(concat!("geohueo ", env!("CARGO_PKG_VERSION"), "\n")));
    for option in [
        "Usage: geohueo",
        "--ipv4",
        "--ipv6",
        "--timeout",
        "--profile",
    ] {
        assert!(help.contains(option));
    }
}

#[test]
fn version_succeeds_without_a_lookup() {
    let output = isolated_command().arg("--version").output().unwrap();

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        concat!("geohueo ", env!("CARGO_PKG_VERSION"), "\n")
    );
}

#[test]
fn unknown_option_returns_usage_error() {
    let output = isolated_command().arg("--unknown").output().unwrap();

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    let diagnostic = String::from_utf8(output.stderr).unwrap();
    assert!(diagnostic.starts_with("geohueo: unknown option"));
    assert!(diagnostic.contains("Try 'geohueo --help'"));
}

#[test]
fn invalid_timeout_returns_usage_error_without_a_lookup() {
    for args in [
        vec!["--timeout"],
        vec!["--timeout", "0"],
        vec!["--timeout=-1"],
        vec!["--timeout=NaN"],
        vec!["--timeout=18446744073709551615"],
    ] {
        let output = isolated_command().args(args).output().unwrap();

        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
        assert!(
            String::from_utf8(output.stderr)
                .unwrap()
                .contains("--timeout")
        );
    }
}

#[test]
fn conflicting_address_families_return_usage_error_without_a_lookup() {
    let output = isolated_command().args(["-4", "-6"]).output().unwrap();

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("cannot be used together")
    );
}

#[test]
fn resolution_failures_do_not_stop_remaining_targets() {
    // Malformed targets are rejected locally without DNS or API requests.
    let output = isolated_command()
        .args(["--json", "-4", "--timeout=5", "[invalid]", "[also-invalid]"])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8(output.stderr)
            .unwrap()
            .matches("cannot resolve")
            .count(),
        2
    );
}

#[test]
fn explicit_normalized_json_has_one_error_per_target_without_changing_legacy_json() {
    let normalized = isolated_command()
        .args(["--output", "normalized-json", "[invalid]", "[also-invalid]"])
        .output()
        .unwrap();
    assert_eq!(normalized.status.code(), Some(1));
    let lines: Vec<serde_json::Value> = String::from_utf8(normalized.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0]["profile"], "geoiplookup");
    assert_eq!(lines[0]["target"], "[invalid]");
    assert_eq!(lines[0]["schema_version"], 1);
    assert_eq!(lines[0]["success"], false);
    assert!(
        lines[0]["error"]
            .as_str()
            .unwrap()
            .contains("cannot resolve")
    );
    assert!(lines[0].get("data").is_none());
    assert_eq!(lines[1]["target"], "[also-invalid]");

    let legacy = isolated_command()
        .args(["--json", "[invalid]"])
        .output()
        .unwrap();
    assert_eq!(legacy.status.code(), Some(1));
    assert!(legacy.stdout.is_empty());
    let conflicting = isolated_command()
        .args(["--output=normalized-json", "--json", "[invalid]"])
        .output()
        .unwrap();
    assert_eq!(conflicting.status.code(), Some(2));
    assert!(conflicting.stdout.is_empty());

    let alias = TempConfig::new(
        "schema_version = 1\ndefault_profile = 'personal'\n[profiles.personal]\nadapter = 'geoiplookup'\n",
    );
    let renamed = isolated_command()
        .args([
            "--config",
            alias.0.to_str().unwrap(),
            "--output=normalized-json",
            "[invalid]",
        ])
        .output()
        .unwrap();
    assert_eq!(renamed.status.code(), Some(1));
    let line: serde_json::Value = serde_json::from_slice(&renamed.stdout).unwrap();
    assert_eq!(line["profile"], "personal");
}

#[cfg(unix)]
#[test]
fn non_utf8_argument_returns_usage_error_not_a_panic() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    let output = isolated_command()
        .arg(OsString::from_vec(vec![0xff]))
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("not valid UTF-8")
    );
}

#[test]
fn checks_config_and_lists_profiles_without_a_lookup() {
    let file = TempConfig::new(
        "schema_version = 1\ndefault_profile = 'work'\n\
         [profiles.work]\nadapter = 'geoiplookup'\ntimeout_seconds = 3\n",
    );
    let check = isolated_command()
        .args(["--config", file.0.to_str().unwrap(), "--check-config"])
        .output()
        .unwrap();
    assert!(check.status.success());
    assert_eq!(
        String::from_utf8(check.stdout).unwrap(),
        "configuration is valid (profile: work)\n"
    );
    assert!(check.stderr.is_empty());

    let list = isolated_command()
        .args(["--config", file.0.to_str().unwrap(), "--list-profiles"])
        .output()
        .unwrap();
    assert!(list.status.success());
    assert_eq!(
        String::from_utf8(list.stdout).unwrap(),
        "geoiplookup\nwork\n"
    );
    assert!(list.stderr.is_empty());
}

#[test]
fn cli_config_and_profile_override_environment_without_a_lookup() {
    let ignored = TempConfig::new("schema_version = 7\n");
    let selected = TempConfig::new(
        "schema_version = 1\ndefault_profile = 'geoiplookup'\n\
         [profiles.personal]\nadapter = 'geoiplookup'\n",
    );
    let output = isolated_command()
        .args([
            "--config",
            selected.0.to_str().unwrap(),
            "--profile=personal",
            "--check-config",
        ])
        .env("GEOHUEO_CONFIG", &ignored.0)
        .env("GEOHUEO_PROFILE", "geoiplookup")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "configuration is valid (profile: personal)\n"
    );
    assert!(output.stderr.is_empty());

    let via_env = isolated_command()
        .args(["--check-config"])
        .env("GEOHUEO_CONFIG", &selected.0)
        .env("GEOHUEO_PROFILE", "personal")
        .output()
        .unwrap();
    assert!(via_env.status.success());
    assert_eq!(
        String::from_utf8(via_env.stdout).unwrap(),
        "configuration is valid (profile: personal)\n"
    );
}

#[test]
fn config_errors_are_usage_errors_and_help_bypasses_config() {
    let missing = TempConfig::new("schema_version = 1\n");
    fs::remove_file(&missing.0).unwrap();
    for args in [
        vec!["--config", missing.0.to_str().unwrap(), "--check-config"],
        vec!["--profile", "not-configured", "--check-config"],
    ] {
        let output = isolated_command().args(args).output().unwrap();
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
        assert!(
            String::from_utf8(output.stderr)
                .unwrap()
                .starts_with("geohueo: ")
        );
    }

    let env_missing = isolated_command()
        .arg("--check-config")
        .env("GEOHUEO_CONFIG", &missing.0)
        .output()
        .unwrap();
    assert_eq!(env_missing.status.code(), Some(2));
    assert!(env_missing.stdout.is_empty());

    let output = isolated_command()
        .args(["--config", missing.0.to_str().unwrap(), "--help"])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(
        String::from_utf8(output.stdout)
            .unwrap()
            .contains("Usage: geohueo")
    );
    assert!(output.stderr.is_empty());
}

#[test]
fn rejects_incomplete_maxmind_profile_without_network_access() {
    let file = TempConfig::new(
        "schema_version = 1\ndefault_profile = 'maxmind'\n\
         [profiles.maxmind]\nadapter = 'maxmind'\n",
    );
    let output = isolated_command()
        .args(["--config", file.0.to_str().unwrap(), "8.8.8.8"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("maxmind")
    );
}

#[test]
fn maxmind_credentials_are_required_only_for_selected_profile_and_not_printed() {
    let file = TempConfig::new(
        "schema_version = 1\ndefault_profile = 'maxmind'\n\
         [profiles.maxmind]\nadapter = 'maxmind'\nservice = 'geoip'\nproduct = 'city'\n\
         [profiles.maxmind.auth]\ntype = 'basic'\nusername_env = 'GEOHUEO_TEST_MM_ACCOUNT'\npassword_env = 'GEOHUEO_TEST_MM_KEY'\n",
    );
    let args = ["--config", file.0.to_str().unwrap(), "--check-config"];
    let missing = isolated_command()
        .args(args)
        .env_remove("GEOHUEO_TEST_MM_ACCOUNT")
        .env_remove("GEOHUEO_TEST_MM_KEY")
        .output()
        .unwrap();
    assert_eq!(missing.status.code(), Some(2));
    assert!(missing.stdout.is_empty());
    assert!(
        String::from_utf8(missing.stderr)
            .unwrap()
            .contains("GEOHUEO_TEST_MM_ACCOUNT")
    );

    let with_keys = isolated_command()
        .args(args)
        .env("GEOHUEO_TEST_MM_ACCOUNT", "123456")
        .env("GEOHUEO_TEST_MM_KEY", "my-test-license")
        .output()
        .unwrap();
    assert!(with_keys.status.success());
    assert_eq!(
        String::from_utf8(with_keys.stdout).unwrap(),
        "configuration is valid (profile: maxmind)\n"
    );
    assert!(with_keys.stderr.is_empty());

    let other_profile = isolated_command()
        .args([
            "--config",
            file.0.to_str().unwrap(),
            "--profile",
            "geoiplookup",
            "--check-config",
        ])
        .env_remove("GEOHUEO_TEST_MM_ACCOUNT")
        .env_remove("GEOHUEO_TEST_MM_KEY")
        .output()
        .unwrap();
    assert!(other_profile.status.success());

    let run = isolated_command()
        .args(["--config", file.0.to_str().unwrap(), "--json", "[invalid]"])
        .env("GEOHUEO_TEST_MM_ACCOUNT", "123456")
        .env("GEOHUEO_TEST_MM_KEY", "my-test-license")
        .output()
        .unwrap();
    assert_eq!(run.status.code(), Some(1));
    let result: serde_json::Value = serde_json::from_slice(&run.stdout).unwrap();
    assert_eq!(result["profile"], "maxmind");
    assert_eq!(result["target"], "[invalid]");
    assert_eq!(result["success"], false);
    assert!(
        !String::from_utf8(run.stderr)
            .unwrap()
            .contains("my-test-license")
    );

    let bad_account = isolated_command()
        .args(args)
        .env("GEOHUEO_TEST_MM_ACCOUNT", "not-an-id")
        .env("GEOHUEO_TEST_MM_KEY", "my-test-license")
        .output()
        .unwrap();
    assert_eq!(bad_account.status.code(), Some(2));
    let stderr = String::from_utf8(bad_account.stderr).unwrap();
    assert!(!stderr.contains("not-an-id"));
    assert!(!stderr.contains("my-test-license"));
}

#[test]
fn rest_json_cli_checks_config_credentials_and_capabilities_without_network() {
    let file = TempConfig::new(
        "schema_version = 1\ndefault_profile = 'custom'\n\
         [profiles.custom]\nadapter = 'rest-json'\nbase_url = 'https://geo.example.invalid'\nsupports_ipv4 = true\nsupports_ipv6 = false\n\
         [profiles.custom.auth]\ntype = 'api_key'\nlocation = 'query'\nname = 'token'\nvalue_env = 'GEOHUEO_TEST_REST_KEY'\n\
         [profiles.custom.request]\nmethod = 'GET'\npath = '/lookup/{ip}'\n\
         [profiles.custom.response]\nrequired_fields = ['ip']\n\
         [profiles.custom.response.fields]\nip = '/ip'\n",
    );
    let args = ["--config", file.0.to_str().unwrap(), "--check-config"];
    let missing = isolated_command()
        .args(args)
        .env_remove("GEOHUEO_TEST_REST_KEY")
        .output()
        .unwrap();
    assert_eq!(missing.status.code(), Some(2));
    assert!(
        String::from_utf8(missing.stderr)
            .unwrap()
            .contains("GEOHUEO_TEST_REST_KEY")
    );
    let selected = isolated_command()
        .args(args)
        .env("GEOHUEO_TEST_REST_KEY", "test/secret?value")
        .output()
        .unwrap();
    assert!(selected.status.success());
    assert_eq!(
        String::from_utf8(selected.stdout).unwrap(),
        "configuration is valid (profile: custom)\n"
    );
    assert!(selected.stderr.is_empty());
    let unselected = isolated_command()
        .args([
            "--config",
            file.0.to_str().unwrap(),
            "--profile",
            "geoiplookup",
            "--check-config",
        ])
        .env_remove("GEOHUEO_TEST_REST_KEY")
        .output()
        .unwrap();
    assert!(unselected.status.success());

    // Unsupported targets are rejected before the HTTPS lookup, even with a
    // non-resolving test origin and a secret-bearing query authentication mode.
    let unsupported = isolated_command()
        .args([
            "--config",
            file.0.to_str().unwrap(),
            "--json",
            "2001:db8::1",
        ])
        .env("GEOHUEO_TEST_REST_KEY", "test/secret?value")
        .output()
        .unwrap();
    assert_eq!(unsupported.status.code(), Some(2));
    let line: serde_json::Value = serde_json::from_slice(&unsupported.stdout).unwrap();
    assert_eq!(line["profile"], "custom");
    assert_eq!(line["success"], false);
    assert!(line["error"].as_str().unwrap().contains("not supported"));
    assert!(
        !String::from_utf8(unsupported.stderr)
            .unwrap()
            .contains("test/secret?value")
    );

    let no_self = isolated_command()
        .args(["--config", file.0.to_str().unwrap(), "--json"])
        .env("GEOHUEO_TEST_REST_KEY", "test/secret?value")
        .output()
        .unwrap();
    assert_eq!(no_self.status.code(), Some(2));
    let line: serde_json::Value = serde_json::from_slice(&no_self.stdout).unwrap();
    assert_eq!(line["target"], "current IP");
    assert_eq!(line["success"], false);
}

#[test]
fn default_profile_is_available_without_a_config_file() {
    let output = isolated_command().arg("--list-profiles").output().unwrap();
    assert!(output.status.success());
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "geoiplookup\n");
    assert!(output.stderr.is_empty());
}

#[test]
fn configured_alias_uses_the_existing_lookup_path() {
    let file = TempConfig::new(
        "schema_version = 1\ndefault_profile = 'personal'\n\
         [profiles.personal]\nadapter = 'geoiplookup'\n",
    );
    // Invalid targets are rejected before DNS or HTTP. This exercises the Run path.
    let output = isolated_command()
        .args(["--config", file.0.to_str().unwrap(), "[invalid]"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("cannot resolve")
    );
}
