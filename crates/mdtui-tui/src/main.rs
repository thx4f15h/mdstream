mod cli;
#[allow(dead_code)]
mod screen;
mod terminal;
#[allow(dead_code)]
mod theme;

use std::fs::File;
use std::io::{self, Write};
use std::path::Path;
use std::process::ExitCode;

use cli::{EXIT_IO, EXIT_NOT_TTY, EXIT_SUCCESS, EXIT_USAGE, Options, Outcome};
use crossterm::tty::IsTty;

fn main() -> ExitCode {
    match cli::parse(std::env::args_os().skip(1)) {
        Ok(Outcome::Help) => match write_help() {
            Ok(()) => ExitCode::from(EXIT_SUCCESS),
            Err(error) => {
                eprintln!("{}", error.message);
                ExitCode::from(error.code)
            }
        },
        Ok(Outcome::Run(options)) => match run(options) {
            Ok(()) => ExitCode::from(EXIT_SUCCESS),
            Err(error) => {
                eprintln!("{}", error.message);
                ExitCode::from(error.code)
            }
        },
        Err(error) => {
            eprintln!("error: {error}");
            eprintln!("Try 'mdtui-tui --help' for more information.");
            ExitCode::from(EXIT_USAGE)
        }
    }
}

fn write_help() -> Result<(), AppError> {
    io::stdout()
        .lock()
        .write_all(cli::HELP.as_bytes())
        .map_err(|error| AppError::io(format!("cannot write help: {error}")))
}

fn run(options: Options) -> Result<(), AppError> {
    let Options {
        once,
        width,
        height,
        link_mode,
        file,
    } = options;

    if once {
        let input_path = file.display().to_string();
        let mut input = open_regular_file(&file)?;
        io::copy(&mut input, &mut io::sink())
            .map_err(|error| AppError::io(format!("cannot read '{input_path}': {error}")))?;
        return Ok(());
    }

    debug_assert!(width.is_none() && height.is_none());
    let _file = open_regular_file(&file)?;
    reject_non_tty()?;
    terminal::run(link_mode).map_err(|error| AppError::io(format!("terminal error: {error}")))
}

fn open_regular_file(path: &Path) -> Result<File, AppError> {
    let file = File::open(path)
        .map_err(|error| AppError::io(format!("cannot open '{}': {error}", path.display())))?;
    let metadata = file
        .metadata()
        .map_err(|error| AppError::io(format!("cannot inspect '{}': {error}", path.display())))?;
    if !metadata.is_file() {
        return Err(AppError::io(format!(
            "not a regular file: '{}'",
            path.display()
        )));
    }
    Ok(file)
}

fn reject_non_tty() -> Result<(), AppError> {
    if io::stdin().is_tty() && io::stdout().is_tty() {
        Ok(())
    } else {
        Err(AppError::new(
            EXIT_NOT_TTY,
            "TUI mode requires an interactive terminal; use --once for non-TTY use",
        ))
    }
}

#[derive(Debug)]
struct AppError {
    code: u8,
    message: String,
}

impl AppError {
    fn new(code: u8, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    fn io(message: impl Into<String>) -> Self {
        Self::new(EXIT_IO, message)
    }
}
