use std::env;
use std::io::{self, Write};
use std::process::ExitCode;

use anyhow::Context;

use geohueo::app;

fn main() -> ExitCode {
    let mut stdout = io::stdout().lock();
    let mut stderr = io::stderr().lock();
    let result = app::execute(env::args_os().skip(1), &mut stdout, &mut stderr).and_then(|code| {
        stdout.flush().context("could not flush stdout")?;
        Ok(code)
    });

    match result {
        Ok(code) => ExitCode::from(code),
        // A consumer such as `head` may close the pipe before we finish writing.
        Err(error) if app::is_broken_pipe(&error) => ExitCode::SUCCESS,
        Err(error) => {
            let _ = writeln!(stderr, "geohueo: {error:#}");
            ExitCode::FAILURE
        }
    }
}
