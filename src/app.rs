use std::env;
use std::ffi::OsString;
use std::io::{self, Write};
use std::net::IpAddr;

use anyhow::{Context, Result};

use crate::cli::{self, Args, Command};
use crate::config::{Adapter, Config, SelectedProfile};
use crate::model::LookupTarget;
use crate::output::Output;
use crate::provider::GeoProvider;
use crate::provider::geoiplookup::Client;
use crate::target::{no_data_message, resolve};

pub fn is_broken_pipe(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<io::Error>()
        .is_some_and(|error| error.kind() == io::ErrorKind::BrokenPipe)
}

pub fn execute(
    args: impl IntoIterator<Item = OsString>,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> Result<u8> {
    let command = match cli::parse_args(args) {
        Ok(command) => command,
        Err(message) => {
            writeln!(stderr, "geohueo: {message}")?;
            writeln!(stderr, "Try 'geohueo --help' for more information.")?;
            return Ok(2);
        }
    };

    enum Action {
        Run,
        CheckConfig,
        ListProfiles,
    }

    let (args, action) = match command {
        Command::Help => {
            write!(stdout, "{}", cli::HELP).context("could not write help to stdout")?;
            return Ok(0);
        }
        Command::Version => {
            writeln!(stdout, "geohueo {}", env!("CARGO_PKG_VERSION"))
                .context("could not write version to stdout")?;
            return Ok(0);
        }
        Command::Run(args) => (args, Action::Run),
        Command::CheckConfig(args) => (args, Action::CheckConfig),
        Command::ListProfiles(args) => (args, Action::ListProfiles),
    };

    let config = match Config::load(args.config.as_deref(), env::var_os("GEOHUEO_CONFIG")) {
        Ok(config) => config,
        Err(error) => {
            writeln!(stderr, "geohueo: {error:#}")?;
            return Ok(2);
        }
    };
    if matches!(action, Action::ListProfiles) {
        for name in config.profile_names() {
            writeln!(stdout, "{name}")?;
        }
        return Ok(0);
    }
    let profile = match config.select(
        args.profile.as_deref(),
        env::var_os("GEOHUEO_PROFILE"),
        args.timeout,
    ) {
        Ok(profile) => profile,
        Err(error) => {
            writeln!(stderr, "geohueo: {error:#}")?;
            return Ok(2);
        }
    };
    if matches!(action, Action::CheckConfig) {
        writeln!(stdout, "configuration is valid (profile: {})", profile.name)?;
        return Ok(0);
    }
    run(args, profile, stdout, stderr)
}

fn run(
    args: Args,
    profile: SelectedProfile,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> Result<u8> {
    match profile.adapter {
        Adapter::Geoiplookup => {
            let client = Client::new(profile.timeout);
            run_with_provider(args, &client, stdout, stderr)
        }
    }
}

fn run_with_provider(
    args: Args,
    provider: &dyn GeoProvider,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> Result<u8> {
    let mut output = Output::new(stdout, args.json);
    let mut failed = false;

    if args.targets.is_empty() {
        let success = lookup(provider, None, "current IP", &mut output, stderr)?;
        return Ok(u8::from(!success));
    }

    for input in &args.targets {
        let target = match resolve(input, args.family) {
            Ok(target) => target,
            Err(message) => {
                writeln!(stderr, "geohueo: {message}")?;
                failed = true;
                continue;
            }
        };

        if !lookup(
            provider,
            Some(target.ip()),
            &target.label(),
            &mut output,
            stderr,
        )? {
            failed = true;
        }
    }

    Ok(u8::from(failed))
}

fn lookup(
    provider: &dyn GeoProvider,
    ip: Option<IpAddr>,
    label: &str,
    output: &mut Output<impl Write>,
    stderr: &mut impl Write,
) -> Result<bool> {
    let target = ip.map_or(LookupTarget::SelfIp, LookupTarget::Ip);
    let info = match provider.lookup(target) {
        Ok(info) => info,
        Err(error) => {
            writeln!(stderr, "geohueo: {label}: lookup failed: {error:#}")?;
            return Ok(false);
        }
    };

    let label = if ip.is_none() {
        info.record.ip.as_deref().unwrap_or(label)
    } else {
        label
    };
    output
        .emit(&info, label)
        .with_context(|| format!("could not write result for {label} to stdout"))?;

    if !info.success {
        writeln!(
            stderr,
            "geohueo: {label}: {}",
            no_data_message(
                ip.or_else(|| info.record.ip.as_deref().and_then(|ip| ip.parse().ok()))
            )
        )?;
    }

    Ok(info.success)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{GeoRecord, LookupResult};
    use crate::provider::Capabilities;
    use crate::test_support::{Response, TestServer, unavailable_client};
    use std::cell::RefCell;

    struct StubProvider {
        targets: RefCell<Vec<LookupTarget>>,
    }

    impl GeoProvider for StubProvider {
        fn capabilities(&self) -> Capabilities {
            Capabilities {
                ipv4: true,
                ipv6: true,
                self_ip: true,
            }
        }

        fn lookup(&self, target: LookupTarget) -> Result<LookupResult> {
            self.targets.borrow_mut().push(target);
            let ip = match target {
                LookupTarget::SelfIp => "9.9.9.9".to_string(),
                LookupTarget::Ip(ip) => ip.to_string(),
            };
            Ok(LookupResult::new(
                GeoRecord {
                    ip: Some(ip),
                    country_code: Some("US".to_string()),
                    ..GeoRecord::default()
                },
                true,
            ))
        }
    }

    #[test]
    fn app_can_run_with_a_provider_without_an_http_client() {
        let provider = StubProvider {
            targets: RefCell::new(Vec::new()),
        };
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let args = Args {
            targets: vec!["8.8.8.8".to_string(), "2001:4860:4860::8888".to_string()],
            ..Args::default()
        };

        let code = run_with_provider(args, &provider, &mut stdout, &mut stderr).unwrap();

        assert_eq!(code, 0);
        assert!(stderr.is_empty());
        assert!(
            String::from_utf8(stdout)
                .unwrap()
                .contains("Country:         US")
        );
        assert_eq!(
            *provider.targets.borrow(),
            [
                LookupTarget::Ip("8.8.8.8".parse().unwrap()),
                LookupTarget::Ip("2001:4860:4860::8888".parse().unwrap()),
            ]
        );
    }

    #[test]
    fn app_uses_self_target_for_empty_input() {
        let provider = StubProvider {
            targets: RefCell::new(Vec::new()),
        };
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();

        assert_eq!(
            run_with_provider(Args::default(), &provider, &mut stdout, &mut stderr).unwrap(),
            0
        );
        assert!(stderr.is_empty());
        assert!(String::from_utf8(stdout).unwrap().starts_with("9.9.9.9\n"));
        assert_eq!(*provider.targets.borrow(), [LookupTarget::SelfIp]);
    }

    #[test]
    fn unavailable_api_returns_failure_without_stdout_in_both_formats() {
        for json in [false, true] {
            let server = TestServer::new(vec![Response::Http {
                status: 503,
                body: r#"{"error":"unavailable"}"#,
            }]);
            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            let code = run_with_provider(
                Args {
                    json,
                    ..Args::default()
                },
                &server.client(),
                &mut stdout,
                &mut stderr,
            )
            .unwrap();

            assert_eq!(code, 1);
            assert!(stdout.is_empty());
            let diagnostic = String::from_utf8(stderr).unwrap();
            assert!(diagnostic.contains("current IP: lookup failed"));
            assert!(diagnostic.contains("request to"));
            assert!(diagnostic.contains("503"));
            assert_eq!(server.requests(), ["GET / HTTP/1.1"]);
        }
    }

    #[test]
    fn refused_connection_returns_failure_and_target_diagnostic() {
        for json in [false, true] {
            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            let code = run_with_provider(
                Args {
                    json,
                    targets: vec!["8.8.8.8".to_string()],
                    ..Args::default()
                },
                &unavailable_client(),
                &mut stdout,
                &mut stderr,
            )
            .unwrap();

            assert_eq!(code, 1);
            assert!(stdout.is_empty());
            let diagnostic = String::from_utf8(stderr).unwrap();
            assert!(diagnostic.contains("8.8.8.8: lookup failed"));
            assert!(diagnostic.contains("request to"));
        }
    }

    #[test]
    fn api_failure_does_not_stop_remaining_targets_or_reset_failure_status() {
        let server = TestServer::new(vec![
            Response::Http {
                status: 503,
                body: r#"{"error":"unavailable"}"#,
            },
            Response::Http {
                status: 200,
                body: r#"{"ip":"1.1.1.1","success":true}"#,
            },
        ]);
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let code = run_with_provider(
            Args {
                json: true,
                targets: vec!["8.8.8.8".to_string(), "1.1.1.1".to_string()],
                ..Args::default()
            },
            &server.client(),
            &mut stdout,
            &mut stderr,
        )
        .unwrap();

        assert_eq!(code, 1);
        let output = String::from_utf8(stdout).unwrap();
        assert_eq!(output.lines().count(), 1);
        let info: serde_json::Value = serde_json::from_str(output.trim()).unwrap();
        assert_eq!(info["ip"], "1.1.1.1");
        assert_eq!(info["success"], true);
        let diagnostic = String::from_utf8(stderr).unwrap();
        assert!(diagnostic.contains("8.8.8.8: lookup failed"));
        assert!(!diagnostic.contains("1.1.1.1: lookup failed"));
        assert_eq!(
            server.requests(),
            ["GET /8.8.8.8 HTTP/1.1", "GET /1.1.1.1 HTTP/1.1"]
        );
    }

    struct FailingWriter(io::ErrorKind);

    impl Write for FailingWriter {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::Error::new(self.0, "test output failure"))
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn preserves_output_error_kind_and_cause_through_context() {
        for kind in [io::ErrorKind::BrokenPipe, io::ErrorKind::PermissionDenied] {
            for flag in ["--help", "--version"] {
                let mut stderr = Vec::new();
                let error = execute(
                    [OsString::from(flag)],
                    &mut FailingWriter(kind),
                    &mut stderr,
                )
                .unwrap_err();

                assert_eq!(error.downcast_ref::<io::Error>().unwrap().kind(), kind);
                assert_eq!(is_broken_pipe(&error), kind == io::ErrorKind::BrokenPipe);
                assert!(error.to_string().contains("could not write"));
                assert!(format!("{error:#}").contains("test output failure"));
                assert!(stderr.is_empty());
            }
        }
    }

    #[test]
    fn non_io_errors_are_not_broken_pipes() {
        let error = anyhow::anyhow!("test failure").context("operation failed");
        assert!(!is_broken_pipe(&error));
    }
}
