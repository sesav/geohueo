mod api;
mod cli;
mod output;
mod target;

use std::env;
use std::ffi::OsString;
use std::io::{self, Write};
use std::net::IpAddr;
use std::process::ExitCode;

use anyhow::{Context, Result};

use api::Client;
use cli::{Args, Command};
use output::Output;
use target::{no_data_message, resolve};

fn main() -> ExitCode {
    let mut stdout = io::stdout().lock();
    let mut stderr = io::stderr().lock();
    let result = execute(env::args_os().skip(1), &mut stdout, &mut stderr).and_then(|code| {
        stdout.flush().context("could not flush stdout")?;
        Ok(code)
    });

    match result {
        Ok(code) => ExitCode::from(code),
        // A consumer such as `head` may close the pipe before we finish writing.
        Err(error) if is_broken_pipe(&error) => ExitCode::SUCCESS,
        Err(error) => {
            let _ = writeln!(stderr, "geolkp: {error:#}");
            ExitCode::FAILURE
        }
    }
}

fn is_broken_pipe(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<io::Error>()
        .is_some_and(|error| error.kind() == io::ErrorKind::BrokenPipe)
}

fn execute(
    args: impl IntoIterator<Item = OsString>,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> Result<u8> {
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
            write!(stdout, "{}", cli::HELP).context("could not write help to stdout")?;
            Ok(0)
        }
        Command::Version => {
            writeln!(stdout, "geolkp {}", env!("CARGO_PKG_VERSION"))
                .context("could not write version to stdout")?;
            Ok(0)
        }
        Command::Run(args) => run(args, stdout, stderr),
    }
}

fn run(args: Args, stdout: &mut impl Write, stderr: &mut impl Write) -> Result<u8> {
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
) -> Result<bool> {
    let info = match client.lookup(ip) {
        Ok(info) => info,
        Err(error) => {
            writeln!(stderr, "geolkp: {label}: lookup failed: {error:#}")?;
            return Ok(false);
        }
    };

    let label = if ip.is_none() && !info.ip.is_empty() {
        &info.ip
    } else {
        label
    };
    output
        .emit(&info, label)
        .with_context(|| format!("could not write result for {label} to stdout"))?;

    if !info.success {
        writeln!(
            stderr,
            "geolkp: {label}: {}",
            no_data_message(ip.or_else(|| info.ip.parse().ok()))
        )?;
    }

    Ok(info.success)
}

#[cfg(test)]
mod tests {
    use super::*;

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
