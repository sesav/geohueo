use std::env;
use std::ffi::OsString;
use std::io::{self, Write};
use std::net::IpAddr;

use anyhow::{Context, Result};

use crate::cli::{self, Args, Command};
use crate::config::{Adapter, Config, SelectedProfile};
use crate::error::{ErrorCode, LookupError};
use crate::model::LookupTarget;
use crate::output::Output;
use crate::provider::GeoProvider;
use crate::provider::configurable;
use crate::provider::geoiplookup::Client;
use crate::provider::maxmind;
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
    let provider_label = profile.provider_label();

    match profile.adapter {
        Adapter::Geoiplookup => {
            let client = Client::new(profile.timeout);
            // The legacy JSON contract has no profile field, so keep it in its
            // original mode. Text output and normalized JSON can identify it.
            let name = (args.normalized_json || !args.json).then_some(profile.name.as_str());
            run_with_provider(args, &client, name, Some(&provider_label), stdout, stderr)
        }
        Adapter::Maxmind(settings) => {
            let client = maxmind::Client::new(settings, profile.timeout);
            run_with_provider(
                args,
                &client,
                Some(&profile.name),
                Some(&provider_label),
                stdout,
                stderr,
            )
        }
        Adapter::RestJson(settings) => {
            let client = configurable::Client::new(*settings, profile.timeout);
            run_with_provider(
                args,
                &client,
                Some(&profile.name),
                Some(&provider_label),
                stdout,
                stderr,
            )
        }
    }
}

fn run_with_provider(
    args: Args,
    provider: &dyn GeoProvider,
    profile: Option<&str>,
    provider_label: Option<&str>,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> Result<u8> {
    let mut output = match (args.normalized_json, profile) {
        (true, name) => Output::for_profile(stdout, true, name.unwrap_or("geoiplookup")),
        (false, Some(name)) => Output::for_profile(stdout, args.json, name),
        (false, None) => Output::new(stdout, args.json),
    };
    if let Some(label) = provider_label {
        output.set_provider(label);
    }
    output.write_header()?;
    let mut failed = false;
    let mut unsupported = false;

    if args.targets.is_empty() {
        return Ok(lookup(
            provider,
            None,
            "current IP",
            "current IP",
            &mut output,
            stderr,
        )?
        .exit_code());
    }

    for input in &args.targets {
        let target = match resolve(input, args.family) {
            Ok(target) => target,
            Err(message) => {
                output.emit_error(input, &LookupError::new(ErrorCode::Dns, &message))?;
                writeln!(stderr, "geohueo: {message}")?;
                failed = true;
                continue;
            }
        };

        match lookup(
            provider,
            Some(target.ip()),
            &target.label(),
            input,
            &mut output,
            stderr,
        )? {
            LookupStatus::Success => {}
            LookupStatus::Failure => failed = true,
            LookupStatus::Unsupported => unsupported = true,
        }
    }

    Ok(if unsupported { 2 } else { u8::from(failed) })
}

enum LookupStatus {
    Success,
    Failure,
    Unsupported,
}

impl LookupStatus {
    fn exit_code(self) -> u8 {
        match self {
            Self::Success => 0,
            Self::Failure => 1,
            Self::Unsupported => 2,
        }
    }
}

fn lookup(
    provider: &dyn GeoProvider,
    ip: Option<IpAddr>,
    label: &str,
    target: &str,
    output: &mut Output<impl Write>,
    stderr: &mut impl Write,
) -> Result<LookupStatus> {
    let lookup_target = ip.map_or(LookupTarget::SelfIp, LookupTarget::Ip);
    if !provider.capabilities().supports(lookup_target) {
        let message = "target is not supported by this profile";
        output.emit_error(
            target,
            &LookupError::new(ErrorCode::UnsupportedCapability, message),
        )?;
        writeln!(stderr, "geohueo: {label}: {message}")?;
        return Ok(LookupStatus::Unsupported);
    }
    let info = match provider.lookup(lookup_target) {
        Ok(info) => info,
        Err(error) => {
            output.emit_error(
                target,
                &LookupError::from_error(&error, ErrorCode::Provider),
            )?;
            writeln!(stderr, "geohueo: {label}: lookup failed: {error:#}")?;
            return Ok(LookupStatus::Failure);
        }
    };

    let display_label = if ip.is_none() {
        info.record.ip.as_deref().unwrap_or(label)
    } else {
        label
    };
    let resolved_ip = ip.map(|ip| ip.to_string());
    output
        .emit(
            &info,
            resolved_ip.as_deref().unwrap_or(display_label),
            target,
        )
        .with_context(|| format!("could not write result for {display_label} to stdout"))?;

    if !info.success {
        writeln!(
            stderr,
            "geohueo: {display_label}: {}",
            no_data_message(
                ip.or_else(|| info.record.ip.as_deref().and_then(|ip| ip.parse().ok()))
            )
        )?;
    }

    Ok(if info.success {
        LookupStatus::Success
    } else {
        LookupStatus::Failure
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{MaxMindProduct, MaxMindService, MaxMindSettings};
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

        let code =
            run_with_provider(args, &provider, None, None, &mut stdout, &mut stderr).unwrap();

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
            run_with_provider(
                Args::default(),
                &provider,
                None,
                None,
                &mut stdout,
                &mut stderr
            )
            .unwrap(),
            0
        );
        assert!(stderr.is_empty());
        assert!(
            String::from_utf8(stdout)
                .unwrap()
                .starts_with("  IP:              9.9.9.9\n")
        );
        assert_eq!(*provider.targets.borrow(), [LookupTarget::SelfIp]);
    }

    #[test]
    fn text_uses_resolved_ip_when_provider_omits_ip() {
        struct WithoutIp;
        impl GeoProvider for WithoutIp {
            fn capabilities(&self) -> Capabilities {
                Capabilities {
                    ipv4: true,
                    ipv6: true,
                    self_ip: true,
                }
            }
            fn lookup(&self, _: LookupTarget) -> Result<LookupResult> {
                Ok(LookupResult::new(
                    GeoRecord {
                        country_code: Some("US".into()),
                        ..GeoRecord::default()
                    },
                    true,
                ))
            }
        }
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let code = run_with_provider(
            Args {
                targets: vec!["localhost".into(), "8.8.8.8".into()],
                ..Args::default()
            },
            &WithoutIp,
            Some("custom"),
            Some("rest-json fixture"),
            &mut stdout,
            &mut stderr,
        )
        .unwrap();
        assert_eq!(code, 0);
        assert!(stderr.is_empty());
        let text = String::from_utf8(stdout).unwrap();
        let addresses: Vec<IpAddr> = text
            .lines()
            .filter_map(|line| line.trim().strip_prefix("IP:"))
            .map(|ip| ip.trim().parse().unwrap())
            .collect();
        assert_eq!(addresses.len(), 2);
        assert!(addresses[0].is_loopback());
        assert_eq!(addresses[1], "8.8.8.8".parse::<IpAddr>().unwrap());
        assert!(text.contains("Target:          localhost"));
    }

    #[test]
    fn normalized_json_keeps_original_hostname_input_as_target() {
        let provider = StubProvider {
            targets: RefCell::new(Vec::new()),
        };
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();

        let code = run_with_provider(
            Args {
                normalized_json: true,
                targets: vec!["localhost".to_string()],
                ..Args::default()
            },
            &provider,
            Some("test"),
            Some("rest-json test"),
            &mut stdout,
            &mut stderr,
        )
        .unwrap();

        assert_eq!(code, 0);
        assert!(stderr.is_empty());
        let line: serde_json::Value = serde_json::from_slice(&stdout).unwrap();
        assert_eq!(line["profile"], "test");
        assert_eq!(line["provider"], "rest-json test");
        assert_eq!(line["target"], "localhost");
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
                None,
                None,
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
                None,
                None,
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
            None,
            None,
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

    #[test]
    fn maxmind_json_lines_include_successes_dns_and_api_errors() {
        let server = TestServer::new(vec![
            Response::HttpWithType {
                status: 404,
                content_type: "application/vnd.maxmind.com-error+json",
                body: r#"{"code":"IP_ADDRESS_NOT_FOUND","error":"not found"}"#,
            },
            Response::Http {
                status: 200,
                body: r#"{"traits":{"ip_address":"1.1.1.1","network":"1.1.1.0/24"},"country":{"iso_code":"AU"}}"#,
            },
        ]);
        let client = maxmind::Client::for_test(
            MaxMindSettings {
                service: MaxMindService::Geoip,
                product: MaxMindProduct::Country,
                locales: vec!["en".to_string()],
                account_id: "123456".to_string(),
                license_key: "test-license".to_string(),
            },
            server.url(),
        );
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let code = run_with_provider(
            Args {
                json: true,
                targets: vec![
                    "[invalid]".to_string(),
                    "8.8.8.8".to_string(),
                    "1.1.1.1".to_string(),
                ],
                ..Args::default()
            },
            &client,
            Some("maxmind"),
            Some("maxmind geoip country"),
            &mut stdout,
            &mut stderr,
        )
        .unwrap();

        assert_eq!(code, 1);
        let lines: Vec<serde_json::Value> = String::from_utf8(stdout)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0]["target"], "[invalid]");
        assert_eq!(lines[0]["success"], false);
        assert_eq!(lines[0]["error"]["code"], "dns");
        assert_eq!(lines[1]["error"]["code"], "no_data");
        assert_eq!(lines[1]["error"]["http_status"], 404);
        assert_eq!(lines[1]["target"], "8.8.8.8");
        assert!(
            lines[1]["error"]["message"]
                .as_str()
                .unwrap()
                .contains("no geolocation")
        );
        assert_eq!(lines[2]["schema_version"], 1);
        assert_eq!(lines[2]["profile"], "maxmind");
        assert_eq!(lines[2]["provider"], "maxmind geoip country");
        assert_eq!(lines[2]["data"]["country_code"], "AU");
        assert_eq!(lines[2]["success"], true);
        assert_eq!(
            server.requests(),
            [
                "GET /geoip/v2.1/country/8.8.8.8 HTTP/1.1",
                "GET /geoip/v2.1/country/1.1.1.1 HTTP/1.1"
            ]
        );
        assert!(!String::from_utf8(stderr).unwrap().contains("test-license"));
    }

    #[test]
    fn maxmind_self_lookup_uses_original_target_in_json_and_resolved_ip_in_text() {
        let make_client = || {
            let server = TestServer::new(vec![Response::Http {
                status: 200,
                body: r#"{"traits":{"ip_address":"8.8.8.8","network":"8.8.8.0/24"},"country":{"iso_code":"US"}}"#,
            }]);
            let client = maxmind::Client::for_test(
                MaxMindSettings {
                    service: MaxMindService::Geoip,
                    product: MaxMindProduct::Country,
                    locales: vec!["en".to_string()],
                    account_id: "123456".to_string(),
                    license_key: "test-license".to_string(),
                },
                server.url(),
            );
            (server, client)
        };
        let (server, client) = make_client();
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        assert_eq!(
            run_with_provider(
                Args {
                    json: true,
                    ..Args::default()
                },
                &client,
                Some("mm"),
                Some("maxmind geoip country"),
                &mut stdout,
                &mut stderr
            )
            .unwrap(),
            0
        );
        let json: serde_json::Value = serde_json::from_slice(&stdout).unwrap();
        assert_eq!(json["target"], "current IP");
        assert_eq!(json["profile"], "mm");
        assert_eq!(json["provider"], "maxmind geoip country");
        assert_eq!(json["data"]["ip"], "8.8.8.8");
        assert_eq!(server.requests(), ["GET /geoip/v2.1/country/me HTTP/1.1"]);
        assert!(stderr.is_empty());

        let (_server, client) = make_client();
        stdout.clear();
        assert_eq!(
            run_with_provider(
                Args::default(),
                &client,
                Some("mm"),
                Some("maxmind geoip country"),
                &mut stdout,
                &mut stderr
            )
            .unwrap(),
            0
        );
        assert!(
            String::from_utf8(stdout)
                .unwrap()
                .starts_with("  Provider:        maxmind geoip country\n  Profile:         mm\n  IP:              8.8.8.8\n")
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
