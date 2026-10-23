mod api;
mod cli;
mod output;
mod target;

use std::env;
use std::ffi::OsString;
use std::io::{self, Write};
use std::net::IpAddr;
use std::process::ExitCode;

use api::Client;
use cli::{Args, Command};
use output::Output;
use target::{no_data_message, resolve};

fn main() -> ExitCode {
    let mut stdout = io::stdout().lock();
    let mut stderr = io::stderr().lock();
    let result = execute(env::args_os().skip(1), &mut stdout, &mut stderr)
        .and_then(|code| stdout.flush().map(|()| code));

    match result {
        Ok(code) => ExitCode::from(code),
        // A consumer such as `head` may close the pipe before we finish writing.
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => ExitCode::SUCCESS,
        Err(error) => {
            let _ = writeln!(stderr, "geolkp: {error}");
            ExitCode::FAILURE
        }
    }
}

fn execute(
    args: impl IntoIterator<Item = OsString>,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> io::Result<u8> {
    let command = match cli::parse_args(args) {
        Ok(command) => command,
        Err(message) => {
            writeln!(stderr, "geolkp: {message}")?;
            writeln!(stderr, "Try 'geolkp --help' for more information.")?;
            return Ok(2);
        }
    };

    match command {
        Command::Help => {
            write!(stdout, "{}", cli::HELP)?;
            Ok(0)
        }
        Command::Version => {
            writeln!(stdout, "geolkp {}", env!("CARGO_PKG_VERSION"))?;
            Ok(0)
        }
        Command::Run(args) => run(args, stdout, stderr),
    }
}

fn run(args: Args, stdout: &mut impl Write, stderr: &mut impl Write) -> io::Result<u8> {
    let client = Client::new(args.timeout);
    let mut output = Output::new(stdout, args.json);
    let mut failed = false;

    if args.targets.is_empty() {
        let success = lookup(&client, None, "current IP", &mut output, stderr)?;
        return Ok(u8::from(!success));
    }

    for input in &args.targets {
        let target = match resolve(input, args.family) {
            Ok(target) => target,
            Err(message) => {
                writeln!(stderr, "geolkp: {message}")?;
                failed = true;
                continue;
            }
        };

        if !lookup(
            &client,
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
    client: &Client,
    ip: Option<IpAddr>,
    label: &str,
    output: &mut Output<impl Write>,
    stderr: &mut impl Write,
) -> io::Result<bool> {
    let info = match client.lookup(ip) {
        Ok(info) => info,
        Err(error) => {
            writeln!(stderr, "geolkp: {label}: lookup failed: {error}")?;
            return Ok(false);
        }
    };

    let label = if ip.is_none() && !info.ip.is_empty() {
        &info.ip
    } else {
        label
    };
    output.emit(&info, label)?;

    if !info.success {
        writeln!(
            stderr,
            "geolkp: {label}: {}",
            no_data_message(ip.or_else(|| info.ip.parse().ok()))
        )?;
    }

    Ok(info.success)
}
