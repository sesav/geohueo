use std::env;
use std::ffi::OsString;
use std::io::{self, Write};
use std::net::IpAddr;

use anyhow::{Context, Result};

use crate::cli::{self, Args, Command};
use crate::config::{Adapter, Config, SelectedProfile};
use crate::error::{ErrorCode, LookupError};
use crate::model::{LookupResult, LookupTarget};
use crate::output::Output;
use crate::provider::GeoProvider;
use crate::provider::configurable;
use crate::provider::geoiplookup::Client;
use crate::provider::maxmind;
use crate::target::{AddressFamily, no_data_message, resolve_for_provider};

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
            run_with_provider(
                args,
                &client,
                Some(&profile.name),
                Some(&provider_label),
                stdout,
                stderr,
            )
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
    let mut output = match profile {
        Some(name) => Output::for_profile(stdout, args.json, name),
        None if args.json => Output::for_profile(stdout, true, "geoiplookup"),
        None => Output::new(stdout, false),
    };
    if let Some(label) = provider_label {
        output.set_provider(label);
    }
    output.write_header()?;
    emit_outcome(
        lookup(args.target, provider, args.family),
        &mut output,
        stderr,
    )
}

struct Outcome {
    target: String,
    resolved_ip: Option<IpAddr>,
    result: std::result::Result<LookupResult, LookupError>,
}

fn lookup(input: Option<String>, provider: &dyn GeoProvider, family: AddressFamily) -> Outcome {
    let mut outcome = Outcome {
        target: input.clone().unwrap_or_else(|| "current IP".into()),
        resolved_ip: None,
        result: Err(LookupError::new(
            ErrorCode::UnsupportedCapability,
            "target is not supported by this profile",
        )),
    };
    let target = if let Some(input) = input {
        match resolve_for_provider(&input, family, provider.capabilities()) {
            Ok(target) => {
                outcome.resolved_ip = Some(target.ip());
                LookupTarget::Ip(target.ip())
            }
            Err(message) => {
                outcome.result = Err(LookupError::new(ErrorCode::Dns, message));
                return outcome;
            }
        }
    } else {
        LookupTarget::SelfIp
    };
    if !provider.capabilities().supports(target) {
        return outcome;
    }
    outcome.result = provider
        .lookup(target)
        .map_err(|error| LookupError::from_error(&error, ErrorCode::Provider));
    if outcome.resolved_ip.is_none() {
        if let Ok(result) = &outcome.result {
            outcome.resolved_ip = result.record.ip.as_deref().and_then(|ip| ip.parse().ok());
        }
    }
    outcome
}

fn emit_outcome(
    outcome: Outcome,
    output: &mut Output<impl Write>,
    stderr: &mut impl Write,
) -> Result<u8> {
    let Outcome {
        target,
        resolved_ip,
        result,
    } = outcome;
    let label = match resolved_ip {
        Some(ip) if target != "current IP" && target != ip.to_string() => {
            format!("{target} ({ip})")
        }
        Some(ip) => ip.to_string(),
        None => target.clone(),
    };
    let info = match result {
        Ok(info) => info,
        Err(error) => {
            output.emit_error(&target, resolved_ip, &error)?;
            if error.code == ErrorCode::Dns && resolved_ip.is_none() && target != "current IP" {
                writeln!(stderr, "geohueo: {error}")?;
            } else if error.code == ErrorCode::UnsupportedCapability {
                writeln!(stderr, "geohueo: {label}: {error}")?;
            } else {
                writeln!(stderr, "geohueo: {label}: lookup failed: {error}")?;
            }
            return Ok(if error.code == ErrorCode::UnsupportedCapability {
                2
            } else {
                1
            });
        }
    };

    output
        .emit(&info, &target, resolved_ip)
        .with_context(|| format!("could not write result for {label} to stdout"))?;

    if !info.success {
        writeln!(
            stderr,
            "geohueo: {label}: {}",
            no_data_message(
                resolved_ip.or_else(|| info.record.ip.as_deref().and_then(|ip| ip.parse().ok()))
            )
        )?;
    }

    Ok(u8::from(!info.success))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{MaxMindProduct, MaxMindService, MaxMindSettings};
    use crate::model::GeoRecord;
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
        for input in ["8.8.8.8", "2001:4860:4860::8888"] {
            let provider = StubProvider {
                targets: RefCell::new(Vec::new()),
            };
            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            let args = Args {
                target: Some(input.into()),
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
                [LookupTarget::Ip(input.parse().unwrap())]
            );
        }
    }

    #[test]
    fn app_uses_self_target_when_no_host_is_given() {
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
        for input in ["localhost", "8.8.8.8"] {
            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            let code = run_with_provider(
                Args {
                    target: Some(input.into()),
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
            let ip: IpAddr = text
                .lines()
                .find_map(|line| line.trim().strip_prefix("IP:"))
                .unwrap()
                .trim()
                .parse()
                .unwrap();
            if input == "localhost" {
                assert!(ip.is_loopback());
                assert!(text.contains("Target:          localhost"));
            } else {
                assert_eq!(ip, "8.8.8.8".parse::<IpAddr>().unwrap());
            }
        }
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
                json: true,
                target: Some("localhost".to_string()),
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
        assert!(
            line["resolved_ip"]
                .as_str()
                .unwrap()
                .parse::<IpAddr>()
                .unwrap()
                .is_loopback()
        );
    }

    #[test]
    fn hostname_selects_supported_ipv4_and_keeps_it_when_api_omits_ip_or_fails() {
        use crate::error::LookupError;

        struct Ipv4Provider {
            fail: bool,
        }
        impl GeoProvider for Ipv4Provider {
            fn capabilities(&self) -> Capabilities {
                Capabilities {
                    ipv4: true,
                    ipv6: false,
                    self_ip: false,
                }
            }
            fn lookup(&self, target: LookupTarget) -> Result<LookupResult> {
                assert_eq!(target, LookupTarget::Ip("127.0.0.1".parse().unwrap()));
                if self.fail {
                    return Err(LookupError::http(503, "unavailable").into());
                }
                Ok(LookupResult::new(GeoRecord::default(), true))
            }
        }
        for fail in [false, true] {
            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            let code = run_with_provider(
                Args {
                    json: true,
                    target: Some("localhost".into()),
                    ..Args::default()
                },
                &Ipv4Provider { fail },
                Some("fixture"),
                None,
                &mut stdout,
                &mut stderr,
            )
            .unwrap();
            assert_eq!(code, u8::from(fail));
            let line: serde_json::Value = serde_json::from_slice(&stdout).unwrap();
            assert_eq!(line["target"], "localhost");
            assert_eq!(line["resolved_ip"], "127.0.0.1");
            if fail {
                assert_eq!(line["error"]["http_status"], 503);
            } else {
                assert!(line["data"]["ip"].is_null());
                assert!(stderr.is_empty());
            }
        }
    }

    #[test]
    fn http_errors_are_reported_without_retrying() {
        for status in [429, 503] {
            let server = TestServer::new(vec![
                Response::Http { status, body: "{}" },
                Response::Http {
                    status: 200,
                    body: r#"{"success":true}"#,
                },
            ]);
            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            let code = run_with_provider(
                Args {
                    json: true,
                    target: Some("8.8.8.8".into()),
                    ..Args::default()
                },
                &server.client(),
                Some("geoiplookup"),
                None,
                &mut stdout,
                &mut stderr,
            )
            .unwrap();
            assert_eq!(code, 1);
            let result: serde_json::Value = serde_json::from_slice(&stdout).unwrap();
            assert_eq!(result["resolved_ip"], "8.8.8.8");
            assert_eq!(result["success"], false);
            assert_eq!(result["error"]["http_status"], status);
            assert!(result.get("data").is_none());
            assert_eq!(server.requests(), ["GET /8.8.8.8 HTTP/1.1"]);
        }
    }

    #[test]
    fn unsupported_ip_keeps_resolved_metadata_without_a_provider_request() {
        struct Ipv4Only;
        impl GeoProvider for Ipv4Only {
            fn capabilities(&self) -> Capabilities {
                Capabilities {
                    ipv4: true,
                    ipv6: false,
                    self_ip: false,
                }
            }
            fn lookup(&self, _: LookupTarget) -> Result<LookupResult> {
                panic!("unsupported IP must not reach the provider")
            }
        }
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let code = run_with_provider(
            Args {
                json: true,
                target: Some("2001:db8::1".into()),
                ..Args::default()
            },
            &Ipv4Only,
            Some("fixture"),
            None,
            &mut stdout,
            &mut stderr,
        )
        .unwrap();
        assert_eq!(code, 2);
        let result: serde_json::Value = serde_json::from_slice(&stdout).unwrap();
        assert_eq!(result["target"], "2001:db8::1");
        assert_eq!(result["resolved_ip"], "2001:db8::1");
        assert_eq!(result["error"]["code"], "unsupported_capability");
    }

    #[test]
    fn unavailable_api_returns_failure_and_a_json_error_when_requested() {
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
            if json {
                let line: serde_json::Value = serde_json::from_slice(&stdout).unwrap();
                assert_eq!(line["target"], "current IP");
                assert!(line["resolved_ip"].is_null());
                assert_eq!(line["success"], false);
                assert_eq!(line["error"]["http_status"], 503);
            } else {
                assert!(stdout.is_empty());
            }
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
                    target: Some("8.8.8.8".to_string()),
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
            if json {
                let line: serde_json::Value = serde_json::from_slice(&stdout).unwrap();
                assert_eq!(line["target"], "8.8.8.8");
                assert_eq!(line["resolved_ip"], "8.8.8.8");
                assert_eq!(line["success"], false);
                assert!(line.get("data").is_none());
            } else {
                assert!(stdout.is_empty());
            }
            let diagnostic = String::from_utf8(stderr).unwrap();
            assert!(diagnostic.contains("8.8.8.8: lookup failed"));
            assert!(diagnostic.contains("request to"));
        }
    }

    #[test]
    fn maxmind_json_covers_success_dns_and_api_errors() {
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
        let mut stderr = Vec::new();
        let mut lines = Vec::<serde_json::Value>::new();
        for input in ["[invalid]", "8.8.8.8", "1.1.1.1"] {
            let mut stdout = Vec::new();
            let code = run_with_provider(
                Args {
                    json: true,
                    target: Some(input.into()),
                    ..Args::default()
                },
                &client,
                Some("maxmind"),
                Some("maxmind geoip country"),
                &mut stdout,
                &mut stderr,
            )
            .unwrap();
            assert_eq!(code, u8::from(input != "1.1.1.1"));
            lines.push(serde_json::from_slice(&stdout).unwrap());
        }
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0]["target"], "[invalid]");
        assert_eq!(lines[0]["success"], false);
        assert_eq!(lines[0]["error"]["code"], "dns");
        assert!(lines[0]["resolved_ip"].is_null());
        assert_eq!(lines[1]["error"]["code"], "no_data");
        assert_eq!(lines[1]["error"]["http_status"], 404);
        assert_eq!(lines[1]["target"], "8.8.8.8");
        assert_eq!(lines[1]["resolved_ip"], "8.8.8.8");
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
        assert_eq!(lines[2]["resolved_ip"], "1.1.1.1");
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
        assert_eq!(json["resolved_ip"], "8.8.8.8");
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
