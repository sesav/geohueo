use std::ffi::OsString;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::target::AddressFamily;

pub const HELP: &str = concat!(
    "geohueo ",
    env!("CARGO_PKG_VERSION"),
    "\n",
    "IP geolocation via configurable providers\n",
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
    "      --config PATH  Load configuration from PATH\n",
    "      --profile NAME Use the named provider profile\n",
    "      --check-config  Validate configuration without a lookup\n",
    "      --list-profiles List configured provider profiles\n",
    "  -j, --json         Print JSON Lines (legacy format for geoiplookup.io)\n",
    "      --output normalized-json  Print versioned, normalized JSON Lines for any profile\n",
    "  -h, --help         Print help\n",
    "  -V, --version      Print version\n",
    "  --                 Stop parsing options\n",
    "\n",
    "Default data: https://geoiplookup.io/api\n",
);

#[derive(Debug, PartialEq, Eq)]
pub struct Args {
    pub json: bool,
    pub normalized_json: bool,
    pub family: AddressFamily,
    pub timeout: Option<Duration>,
    pub config: Option<PathBuf>,
    pub profile: Option<String>,
    pub targets: Vec<String>,
}

impl Default for Args {
    fn default() -> Self {
        Self {
            json: false,
            normalized_json: false,
            family: AddressFamily::Any,
            timeout: None,
            config: None,
            profile: None,
            targets: Vec::new(),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    Run(Args),
    CheckConfig(Args),
    ListProfiles(Args),
    Help,
    Version,
}

pub fn parse_args(args: impl IntoIterator<Item = OsString>) -> Result<Command, String> {
    let mut parsed = Args::default();
    let mut positional_only = false;
    let mut command = None;
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
                    if parsed.normalized_json {
                        return Err(
                            "options '--json' and '--output' cannot be used together".to_string()
                        );
                    }
                    parsed.json = true;
                    continue;
                }
                "--output" => {
                    let value = args
                        .next()
                        .transpose()?
                        .ok_or("option '--output' requires a format")?;
                    set_output(&mut parsed, &value)?;
                    continue;
                }
                value if value.starts_with("--output=") => {
                    set_output(&mut parsed, &value["--output=".len()..])?;
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
                    parsed.timeout = Some(parse_timeout(&value)?);
                    continue;
                }
                value if value.starts_with("--timeout=") => {
                    parsed.timeout = Some(parse_timeout(&value["--timeout=".len()..])?);
                    continue;
                }
                "--config" => {
                    let value = args
                        .next()
                        .transpose()?
                        .ok_or("option '--config' requires a path")?;
                    parsed.config = Some(parse_path(&value)?);
                    continue;
                }
                value if value.starts_with("--config=") => {
                    parsed.config = Some(parse_path(&value["--config=".len()..])?);
                    continue;
                }
                "--profile" => {
                    let value = args
                        .next()
                        .transpose()?
                        .ok_or("option '--profile' requires a name")?;
                    parsed.profile = Some(parse_profile(&value)?);
                    continue;
                }
                value if value.starts_with("--profile=") => {
                    parsed.profile = Some(parse_profile(&value["--profile=".len()..])?);
                    continue;
                }
                "--check-config" => {
                    set_command(&mut command, "--check-config")?;
                    continue;
                }
                "--list-profiles" => {
                    set_command(&mut command, "--list-profiles")?;
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

    match command {
        None => Ok(Command::Run(parsed)),
        Some(option) => {
            if !parsed.targets.is_empty() {
                return Err(format!("option '{option}' cannot be used with targets"));
            }
            if parsed.json || parsed.normalized_json || parsed.family != AddressFamily::Any {
                return Err(format!(
                    "option '{option}' cannot be used with output or DNS options"
                ));
            }
            if option == "--list-profiles" && (parsed.profile.is_some() || parsed.timeout.is_some())
            {
                return Err(
                    "option '--list-profiles' cannot be used with '--profile' or '--timeout'"
                        .to_string(),
                );
            }
            if option == "--check-config" {
                Ok(Command::CheckConfig(parsed))
            } else {
                Ok(Command::ListProfiles(parsed))
            }
        }
    }
}

fn set_output(args: &mut Args, value: &str) -> Result<(), String> {
    if value != "normalized-json" {
        return Err(format!(
            "invalid value for '--output': '{value}' (expected normalized-json)"
        ));
    }
    if args.json {
        return Err("options '--json' and '--output' cannot be used together".to_string());
    }
    args.normalized_json = true;
    Ok(())
}

fn set_command(current: &mut Option<&'static str>, option: &'static str) -> Result<(), String> {
    if current.is_some_and(|current| current != option) {
        return Err(
            "options '--check-config' and '--list-profiles' cannot be used together".to_string(),
        );
    }
    *current = Some(option);
    Ok(())
}

fn parse_path(value: &str) -> Result<PathBuf, String> {
    if value.is_empty() {
        return Err("option '--config' requires a non-empty path".to_string());
    }
    Ok(PathBuf::from(value))
}

fn parse_profile(value: &str) -> Result<String, String> {
    if value.is_empty() || value.trim() != value || value.chars().any(char::is_control) {
        return Err("option '--profile' requires a valid non-empty name".to_string());
    }
    Ok(value.to_string())
}

fn parse_timeout(value: &str) -> Result<Duration, String> {
    let invalid = || {
        format!(
            "invalid value for '--timeout': '{value}' (expected a positive whole number of seconds within the supported range)"
        )
    };
    let seconds = value.parse::<u64>().map_err(|_| invalid())?;
    timeout_from_seconds(seconds).ok_or_else(invalid)
}

pub(crate) fn timeout_from_seconds(seconds: u64) -> Option<Duration> {
    let timeout = Duration::from_secs(seconds);
    (!timeout.is_zero() && Instant::now().checked_add(timeout).is_some()).then_some(timeout)
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
                    timeout: Some(Duration::from_secs(5)),
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
                normalized_json: false,
                family: AddressFamily::Ipv6,
                timeout: Some(Duration::from_secs(1)),
                config: None,
                profile: None,
                targets: vec!["8.8.8.8".to_string(), "example.com".to_string()],
            })
        );
    }

    #[test]
    fn normalized_json_output_is_explicit_and_conflicts_with_legacy_flag() {
        for option in [
            vec!["--output", "normalized-json"],
            vec!["--output=normalized-json"],
        ] {
            assert_eq!(
                parse(&option).unwrap(),
                Command::Run(Args {
                    normalized_json: true,
                    ..Args::default()
                })
            );
        }
        for options in [
            vec!["--output"],
            vec!["--output=other"],
            vec!["--json", "--output=normalized-json"],
            vec!["--output=normalized-json", "-j"],
            vec!["--check-config", "--output=normalized-json"],
        ] {
            assert!(parse(&options).is_err(), "accepted {options:?}");
        }
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

    #[test]
    fn parses_configuration_options_in_both_forms() {
        assert_eq!(
            parse(&["--config=./settings.toml", "--profile", "personal"]).unwrap(),
            Command::Run(Args {
                config: Some(PathBuf::from("./settings.toml")),
                profile: Some("personal".to_string()),
                ..Args::default()
            })
        );
        assert_eq!(
            parse(&["--config", "./settings.toml", "--profile=personal"]),
            parse(&["--config=./settings.toml", "--profile", "personal"])
        );
        assert_eq!(
            parse(&["--check-config", "--profile=personal"]).unwrap(),
            Command::CheckConfig(Args {
                profile: Some("personal".to_string()),
                ..Args::default()
            })
        );
        assert_eq!(
            parse(&["--list-profiles"]).unwrap(),
            Command::ListProfiles(Args::default())
        );
    }

    #[test]
    fn rejects_invalid_configuration_options() {
        for args in [
            vec!["--config"],
            vec!["--config="],
            vec!["--profile"],
            vec!["--profile="],
            vec!["--check-config", "8.8.8.8"],
            vec!["--list-profiles", "--", "8.8.8.8"],
            vec!["--check-config", "--list-profiles"],
            vec!["--check-config", "--json"],
            vec!["--list-profiles", "--profile", "personal"],
        ] {
            assert!(parse(&args).is_err(), "unexpectedly accepted {args:?}");
        }
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
