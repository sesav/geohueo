use std::path::Path;
use std::process::Command;

fn cli(config: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_geohueo"));
    command.args(["--config", config.to_str().unwrap()]);
    command.env_remove("GEOHUEO_CONFIG");
    command.env_remove("GEOHUEO_PROFILE");
    command.env_remove("MAXMIND_ACCOUNT_ID");
    command.env_remove("MAXMIND_LICENSE_KEY");
    command.env_remove("CUSTOM_GEO_TOKEN");
    command.env_remove("CUSTOM_GEO_API_KEY");
    command.env_remove("CUSTOM_GEO_USERNAME");
    command.env_remove("CUSTOM_GEO_PASSWORD");
    command
}

#[test]
fn shipped_profile_examples_validate_without_network_and_load_only_selected_secrets() {
    let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/profiles");
    for (file, name, secrets) in [
        (
            "maxmind-geoip.toml",
            "maxmind",
            [
                ("MAXMIND_ACCOUNT_ID", "123456"),
                ("MAXMIND_LICENSE_KEY", "example-key"),
            ],
        ),
        (
            "maxmind-geolite.toml",
            "geolite",
            [
                ("MAXMIND_ACCOUNT_ID", "123456"),
                ("MAXMIND_LICENSE_KEY", "example-key"),
            ],
        ),
        (
            "maxmind-geolite-city.toml",
            "geolite",
            [
                ("MAXMIND_ACCOUNT_ID", "123456"),
                ("MAXMIND_LICENSE_KEY", "example-key"),
            ],
        ),
        (
            "rest-path.toml",
            "custom",
            [("CUSTOM_GEO_TOKEN", "example-token"), ("", "")],
        ),
        (
            "rest-query.toml",
            "query-api",
            [("CUSTOM_GEO_API_KEY", "example-key"), ("", "")],
        ),
        (
            "rest-basic.toml",
            "basic-api",
            [
                ("CUSTOM_GEO_USERNAME", "example-user"),
                ("CUSTOM_GEO_PASSWORD", "example-password"),
            ],
        ),
    ] {
        let config = examples.join(file);
        // This checks every field of every profile without requiring any credential.
        let alternative = cli(&config)
            .args(["--profile", "geoiplookup", "--check-config"])
            .output()
            .unwrap();
        assert!(
            alternative.status.success(),
            "{file}: {}",
            String::from_utf8_lossy(&alternative.stderr)
        );

        let missing = cli(&config).arg("--check-config").output().unwrap();
        assert_eq!(missing.status.code(), Some(2), "{file}");
        assert!(missing.stdout.is_empty());

        let mut selected = cli(&config);
        selected.arg("--check-config");
        for (env_name, value) in secrets {
            if !env_name.is_empty() {
                selected.env(env_name, value);
            }
        }
        let valid = selected.output().unwrap();
        assert!(
            valid.status.success(),
            "{file}: {}",
            String::from_utf8_lossy(&valid.stderr)
        );
        assert_eq!(
            String::from_utf8(valid.stdout).unwrap(),
            format!("configuration is valid (profile: {name})\n")
        );
        assert!(valid.stderr.is_empty());
    }
}
