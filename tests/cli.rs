use std::process::Command;

#[test]
fn help_succeeds_without_a_lookup() {
    let output = Command::new(env!("CARGO_BIN_EXE_geohueo"))
        .arg("--help")
        .output()
        .unwrap();

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
    let output = Command::new(env!("CARGO_BIN_EXE_geohueo"))
        .arg("--version")
        .output()
        .unwrap();

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        concat!("geohueo ", env!("CARGO_PKG_VERSION"), "\n")
    );
}

#[test]
fn unknown_option_returns_usage_error() {
    let output = Command::new(env!("CARGO_BIN_EXE_geohueo"))
        .arg("--unknown")
        .output()
        .unwrap();

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
        let output = Command::new(env!("CARGO_BIN_EXE_geohueo"))
            .args(args)
            .output()
            .unwrap();

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
    let output = Command::new(env!("CARGO_BIN_EXE_geohueo"))
        .args(["-4", "-6"])
        .output()
        .unwrap();

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
    let output = Command::new(env!("CARGO_BIN_EXE_geohueo"))
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

    let output = Command::new(env!("CARGO_BIN_EXE_geohueo"))
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
