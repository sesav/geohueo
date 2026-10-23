use std::process::Command;

#[test]
fn help_succeeds_without_a_lookup() {
    let output = Command::new(env!("CARGO_BIN_EXE_geolkp"))
        .arg("--help")
        .output()
        .unwrap();

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert!(String::from_utf8(output.stdout).unwrap().contains("Usage:"));
}

#[test]
fn version_succeeds_without_a_lookup() {
    let output = Command::new(env!("CARGO_BIN_EXE_geolkp"))
        .arg("--version")
        .output()
        .unwrap();

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        concat!("geolkp ", env!("CARGO_PKG_VERSION"), "\n")
    );
}

#[test]
fn unknown_option_returns_usage_error() {
    let output = Command::new(env!("CARGO_BIN_EXE_geolkp"))
        .arg("--unknown")
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("unknown option")
    );
}

#[test]
fn resolution_failures_do_not_stop_remaining_targets() {
    // Malformed targets are rejected locally without DNS or API requests.
    let output = Command::new(env!("CARGO_BIN_EXE_geolkp"))
        .args(["--json", "[invalid]", "[also-invalid]"])
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

    let output = Command::new(env!("CARGO_BIN_EXE_geolkp"))
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
