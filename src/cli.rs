use std::ffi::OsString;
use std::time::{Duration, Instant};

use crate::provider::geoiplookup::DEFAULT_TIMEOUT;
use crate::target::AddressFamily;

pub const HELP: &str = concat!(
    "geohueo ",
    env!("CARGO_PKG_VERSION"),
    "\n",
    "IP geolocation via geoiplookup.io\n",
    "\n",
    "Usage: geohueo [OPTIONS] [HOST...]\n",
    "\n",
    "Arguments:\n",
    "  HOST...  IPv4/IPv6 address or hostname to look up.\n",
    "           If omitted, the public IP of this machine is looked up.\n",
    "\n",
    "Options:\n",
    "  -4, --ipv4         Resolve hostnames to IPv4 only\n",
    "  -6, --ipv6         Resolve hostnames to IPv6 only\n",
    "      --timeout SEC  HTTP timeout in positive whole seconds (default: 10)\n",
    "  -j, --json         Print one JSON object per line\n",
    "  -h, --help         Print help\n",
    "  -V, --version      Print version\n",
    "  --                 Stop parsing options\n",
    "\n",
    "Data: https://geoiplookup.io/api\n",
);

#[derive(Debug, PartialEq, Eq)]
pub struct Args {
    pub json: bool,
    pub family: AddressFamily,
    pub timeout: Duration,
    pub targets: Vec<String>,
}

impl Default for Args {
    fn default() -> Self {
        Self {
            json: false,
            family: AddressFamily::Any,
            timeout: DEFAULT_TIMEOUT,
            targets: Vec::new(),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    Run(Args),
    Help,
    Version,
}

pub fn parse_args(args: impl IntoIterator<Item = OsString>) -> Result<Command, String> {
    let mut parsed = Args::default();
    let mut positional_only = false;
    let mut args = args.into_iter().map(|arg| {
        arg.into_string()
            .map_err(|arg| format!("argument is not valid UTF-8: {arg:?}"))
    });

    while let Some(arg) = args.next() {
        let arg = arg?;

        if !positional_only {
            match arg.as_str() {
                "-h" | "--help" => return Ok(Command::Help),
                "-V" | "--version" => return Ok(Command::Version),
                "-j" | "--json" => {
                    parsed.json = true;
                    continue;
                }
                "-4" | "--ipv4" | "-6" | "--ipv6" => {
                    let family = match arg.as_str() {
                        "-4" | "--ipv4" => AddressFamily::Ipv4,
                        _ => AddressFamily::Ipv6,
                    };
                    if parsed.family != AddressFamily::Any && parsed.family != family {
                        return Err(
                            "options '--ipv4' and '--ipv6' cannot be used together".to_string()
                        );
                    }
                    parsed.family = family;
                    continue;
                }
                "--timeout" => {
                    let value = args
                        .next()
                        .transpose()?
                        .ok_or("option '--timeout' requires a value in seconds")?;
                    parsed.timeout = parse_timeout(&value)?;
                    continue;
                }
                value if value.starts_with("--timeout=") => {
                    parsed.timeout = parse_timeout(&value["--timeout=".len()..])?;
                    continue;
                }
                "--" => {
                    positional_only = true;
                    continue;
                }
                value if value.starts_with('-') => {
                    return Err(format!("unknown option '{value}'"));
                }
                _ => {}
            }
        }

        parsed.targets.push(arg);
    }

    Ok(Command::Run(parsed))
}

fn parse_timeout(value: &str) -> Result<Duration, String> {
    let invalid = || {
        format!(
            "invalid value for '--timeout': '{value}' (expected a positive whole number of seconds within the supported range)"
        )
    };
    let seconds = value.parse::<u64>().map_err(|_| invalid())?;
    let timeout = Duration::from_secs(seconds);
    if timeout.is_zero() || Instant::now().checked_add(timeout).is_none() {
        return Err(invalid());
    }

    Ok(timeout)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Command, String> {
        parse_args(args.iter().map(OsString::from))
    }

    #[test]
    fn parses_flags_and_targets() {
        assert_eq!(
            parse(&["--json", "8.8.8.8", "example.com."]).unwrap(),
            Command::Run(Args {
                json: true,
                targets: vec!["8.8.8.8".to_string(), "example.com.".to_string()],
                ..Args::default()
            })
        );
    }

    #[test]
    fn defaults_to_current_ip() {
        assert_eq!(parse(&[]).unwrap(), Command::Run(Args::default()));
    }

    #[test]
    fn double_dash_stops_option_parsing() {
        assert_eq!(
            parse(&["--", "--json", "-6", "--timeout=1"]).unwrap(),
            Command::Run(Args {
                targets: vec![
                    "--json".to_string(),
                    "-6".to_string(),
                    "--timeout=1".to_string(),
                ],
                ..Args::default()
            })
        );
    }

    #[test]
    fn parses_short_flags() {
        assert_eq!(parse(&["-h"]).unwrap(), Command::Help);
        assert_eq!(parse(&["-V"]).unwrap(), Command::Version);
        assert_eq!(parse(&["-j"]), parse(&["--json"]));
    }

    #[test]
    fn parses_address_family_flags() {
        for (flag, family) in [
            ("-4", AddressFamily::Ipv4),
            ("--ipv4", AddressFamily::Ipv4),
            ("-6", AddressFamily::Ipv6),
            ("--ipv6", AddressFamily::Ipv6),
        ] {
            assert_eq!(
                parse(&[flag, "example.com"]).unwrap(),
                Command::Run(Args {
                    family,
                    targets: vec!["example.com".to_string()],
                    ..Args::default()
                })
            );
        }
        assert_eq!(parse(&["-4", "--ipv4"]), parse(&["-4"]));
        assert_eq!(parse(&["-6", "--ipv6"]), parse(&["-6"]));
    }

    #[test]
    fn rejects_conflicting_address_families() {
        for flags in [["-4", "-6"], ["--ipv6", "--ipv4"], ["-4", "--ipv6"]] {
            assert!(
                parse(&flags)
                    .unwrap_err()
                    .contains("cannot be used together")
            );
        }
    }

    #[test]
    fn parses_timeout_in_both_forms() {
        for args in [vec!["--timeout", "5"], vec!["--timeout=5"]] {
            assert_eq!(
                parse(&args).unwrap(),
                Command::Run(Args {
                    timeout: Duration::from_secs(5),
                    ..Args::default()
                })
            );
        }
        assert_eq!(
            parse(&["--timeout=1", "--timeout", "5"]),
            parse(&["--timeout=5"])
        );
    }

    #[test]
    fn parses_new_options_between_targets() {
        assert_eq!(
            parse(&["8.8.8.8", "--timeout", "1", "-6", "example.com", "--json"]).unwrap(),
            Command::Run(Args {
                json: true,
                family: AddressFamily::Ipv6,
                timeout: Duration::from_secs(1),
                targets: vec!["8.8.8.8".to_string(), "example.com".to_string()],
            })
        );
    }

    #[test]
    fn rejects_missing_or_invalid_timeout() {
        assert!(
            parse(&["--timeout"])
                .unwrap_err()
                .contains("requires a value")
        );
        for value in [
            "",
            "0",
            "-1",
            "1.5",
            "NaN",
            "inf",
            "abc",
            "--json",
            "18446744073709551615",
            "18446744073709551616",
        ] {
            for args in [
                vec!["--timeout".to_string(), value.to_string()],
                vec![format!("--timeout={value}")],
            ] {
                assert!(
                    parse_args(args.into_iter().map(OsString::from))
                        .unwrap_err()
                        .contains("--timeout")
                );
            }
        }
    }

    #[test]
    fn rejects_unknown_option() {
        assert!(parse(&["--wat"]).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn rejects_non_utf8_argument_without_panicking() {
        use std::os::unix::ffi::OsStringExt;

        let error = parse_args([OsString::from_vec(vec![0xff])]).unwrap_err();
        assert!(error.contains("not valid UTF-8"));
        let error =
            parse_args([OsString::from("--timeout"), OsString::from_vec(vec![0xff])]).unwrap_err();
        assert!(error.contains("not valid UTF-8"));
    }
}
