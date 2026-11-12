use std::fs::File;
use std::io::{self, Read, Write};
use std::path::Path;
use std::process::ExitCode;

use crossterm::tty::IsTty;
use mdtui_tui::cli::{
    DEFAULT_HEIGHT, DEFAULT_WIDTH, EXIT_IO, EXIT_NOT_TTY, EXIT_RENDER, EXIT_SUCCESS, EXIT_USAGE,
    Options, Outcome,
};
use mdtui_tui::render::document::StaticRenderError;
use mdtui_tui::static_render;
use mdtui_tui::terminal;
use mdtui3::{Document, Utf8Document};

fn main() -> ExitCode {
    match mdtui_tui::cli::parse(std::env::args_os().skip(1)) {
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
        .write_all(mdtui_tui::cli::HELP.as_bytes())
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
        let width = width.unwrap_or(DEFAULT_WIDTH);
        let height = height.unwrap_or(DEFAULT_HEIGHT);
        let document = load_document(&file)?;
        let output = static_render::render_once_ansi(&document, width, height, link_mode)
            .map_err(|error| once_render_error(error, height))?;
        io::stdout()
            .lock()
            .write_all(&output)
            .map_err(|error| AppError::io(format!("cannot write --once output: {error}")))?;
        return Ok(());
    }

    debug_assert!(width.is_none() && height.is_none());
    let file = open_input_file(&file)?;
    reject_non_tty()?;
    terminal::run(link_mode, file).map_err(|error| AppError::io(format!("terminal error: {error}")))
}

fn load_document(path: &Path) -> Result<Document, AppError> {
    let input_path = path.display().to_string();
    let mut input = open_input_file(path)?;
    let mut bytes = Vec::new();
    input
        .read_to_end(&mut bytes)
        .map_err(|error| AppError::io(format!("cannot read '{input_path}': {error}")))?;
    let mut decoded = Utf8Document::new();
    decoded
        .append(&bytes)
        .map_err(|_| AppError::new(EXIT_RENDER, "--once input is already finished"))?;
    decoded.finish();
    Ok(decoded.into_document())
}

fn once_render_error(error: StaticRenderError, height: u16) -> AppError {
    if error.is_row_overflow() {
        AppError::new(
            EXIT_RENDER,
            format!("--once content exceeds height {height}; increase --height"),
        )
    } else {
        AppError::new(EXIT_RENDER, format!("--once static render failed: {error}"))
    }
}

fn open_input_file(path: &Path) -> Result<File, AppError> {
    let file = File::open(path)
        .map_err(|error| AppError::io(format!("cannot open '{}': {error}", path.display())))?;
    let metadata = file
        .metadata()
        .map_err(|error| AppError::io(format!("cannot inspect '{}': {error}", path.display())))?;
    let supported = metadata.is_file();
    #[cfg(unix)]
    let supported = supported || {
        use std::os::unix::fs::FileTypeExt;
        metadata.file_type().is_fifo()
    };
    if !supported {
        return Err(AppError::io(format!(
            "not a regular file or FIFO: '{}'",
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
