use std::ffi::OsString;

pub const HELP: &str = concat!(
    "geolkp ",
    env!("CARGO_PKG_VERSION"),
    "\n",
    "IP geolocation via geoiplookup.io\n",
    "\n",
    "Usage: geolkp [OPTIONS] [HOST...]\n",
    "\n",
    "Arguments:\n",
    "  HOST...  IPv4/IPv6 address or hostname to look up.\n",
    "           If omitted, the public IP of this machine is looked up.\n",
    "\n",
    "Options:\n",
    "  -j, --json     Print one JSON object per line\n",
    "  -h, --help     Print help\n",
    "  -V, --version  Print version\n",
    "  --            Stop parsing options\n",
    "\n",
    "Data: https://geoiplookup.io/api\n",
);

#[derive(Debug, PartialEq, Eq)]
pub struct Args {
    pub json: bool,
    pub targets: Vec<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    Run(Args),
    Help,
    Version,
}

pub fn parse_args(args: impl IntoIterator<Item = OsString>) -> Result<Command, String> {
    let mut json = false;
    let mut targets = Vec::new();
    let mut positional_only = false;

    for arg in args {
        let arg = arg
            .into_string()
            .map_err(|arg| format!("argument is not valid UTF-8: {arg:?}"))?;

        if !positional_only {
            match arg.as_str() {
                "-h" | "--help" => return Ok(Command::Help),
                "-V" | "--version" => return Ok(Command::Version),
                "-j" | "--json" => {
                    json = true;
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

        targets.push(arg);
    }

    Ok(Command::Run(Args { json, targets }))
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
            })
        );
    }

    #[test]
    fn defaults_to_current_ip() {
        assert_eq!(
            parse(&[]).unwrap(),
            Command::Run(Args {
                json: false,
                targets: Vec::new(),
            })
        );
    }

    #[test]
    fn double_dash_stops_option_parsing() {
        assert_eq!(
            parse(&["--", "--json"]).unwrap(),
            Command::Run(Args {
                json: false,
                targets: vec!["--json".to_string()],
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
    fn rejects_unknown_option() {
        assert!(parse(&["--wat"]).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn rejects_non_utf8_argument_without_panicking() {
        use std::os::unix::ffi::OsStringExt;

        let error = parse_args([OsString::from_vec(vec![0xff])]).unwrap_err();
        assert!(error.contains("not valid UTF-8"));
    }
}
