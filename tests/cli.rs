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
    for option in ["Usage: geohueo", "--ipv4", "--ipv6", "--timeout"] {
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
fn rejects_unknown_adapter_without_network_access() {
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
