use std::ffi::{OsStr, OsString};
use std::fmt;
use std::path::PathBuf;

pub const EXIT_SUCCESS: u8 = 0;
pub const EXIT_USAGE: u8 = 2;
pub const EXIT_IO: u8 = 3;
pub const EXIT_NOT_TTY: u8 = 4;
pub const MAX_DIMENSION: u16 = 1_000;

pub const HELP: &str = "\
mdtui-tui - terminal Markdown reader

Usage:
  mdtui-tui [OPTIONS] <FILE>
  mdtui-tui --help

Options:
      --once             Read and validate FILE once; ANSI rendering is not implemented yet
      --width <N>        Fixed width for --once, 1..=1000 (TUI mode uses terminal size)
      --height <N>       Fixed height for --once, 1..=1000 (TUI mode uses terminal size)
      --open-links       Allow explicit external-link opening (reserved; not implemented yet)
      --no-links         Disable OSC 8 only; keep link styling, internal anchors, and in-app link focus
  -h, --help             Print this help

TUI keys:
  q                      Quit
  Ctrl-C                 Quit

Exit codes:
  0 success
  2 invalid arguments
  3 file or terminal I/O error
  4 TUI mode requires an interactive terminal

--width/--height are rejected in TUI mode. --once does not emit reference ANSI yet.
";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkMode {
    Default,
    OpenExternal,
    NoOsc8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    pub once: bool,
    pub width: Option<u16>,
    pub height: Option<u16>,
    pub link_mode: LinkMode,
    pub file: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Help,
    Run(Options),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    message: String,
}

impl ParseError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ParseError {}

pub fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Outcome, ParseError> {
    let args: Vec<OsString> = args.into_iter().collect();
    if help_requested(&args) {
        return Ok(Outcome::Help);
    }

    let mut once = false;
    let mut width = None;
    let mut height = None;
    let mut open_links = false;
    let mut no_links = false;
    let mut files = Vec::new();
    let mut options_ended = false;
    let mut index = 0;

    while index < args.len() {
        let argument = &args[index];
        if !options_ended {
            match argument.to_str() {
                Some("--") => {
                    options_ended = true;
                    index += 1;
                    continue;
                }
                Some("--once") => once = true,
                Some("--width") => {
                    if width.is_some() {
                        return Err(ParseError::new(
                            "option '--width' may only be specified once",
                        ));
                    }
                    let value = take_value(&args, &mut index, "--width")?;
                    width = Some(parse_dimension("--width", &value)?);
                    continue;
                }
                Some("--height") => {
                    if height.is_some() {
                        return Err(ParseError::new(
                            "option '--height' may only be specified once",
                        ));
                    }
                    let value = take_value(&args, &mut index, "--height")?;
                    height = Some(parse_dimension("--height", &value)?);
                    continue;
                }
                Some("--open-links") => open_links = true,
                Some("--no-links") => no_links = true,
                Some(value) if value.starts_with('-') && value != "-" => {
                    return Err(ParseError::new(format!("unknown option: {value}")));
                }
                _ => files.push(argument.clone()),
            }
        } else {
            files.push(argument.clone());
        }
        index += 1;
    }

    if open_links && no_links {
        return Err(ParseError::new(
            "options '--open-links' and '--no-links' cannot be used together",
        ));
    }
    if !once && (width.is_some() || height.is_some()) {
        return Err(ParseError::new(
            "options '--width' and '--height' are only valid with '--once'",
        ));
    }
    let file = match files.as_slice() {
        [file] => PathBuf::from(file),
        _ => {
            return Err(ParseError::new(
                "expected exactly one FILE argument (use --help for usage)",
            ));
        }
    };
    let link_mode = if open_links {
        LinkMode::OpenExternal
    } else if no_links {
        LinkMode::NoOsc8
    } else {
        LinkMode::Default
    };

    Ok(Outcome::Run(Options {
        once,
        width,
        height,
        link_mode,
        file,
    }))
}

fn help_requested(args: &[OsString]) -> bool {
    let mut options_ended = false;
    for argument in args {
        match argument.to_str() {
            Some("--") if !options_ended => options_ended = true,
            Some("--help" | "-h") if !options_ended => return true,
            _ => {}
        }
    }
    false
}

fn take_value(args: &[OsString], index: &mut usize, option: &str) -> Result<OsString, ParseError> {
    let value = args
        .get(*index + 1)
        .ok_or_else(|| ParseError::new(format!("option '{option}' requires a value")))?;
    *index += 2;
    Ok(value.clone())
}

fn parse_dimension(option: &str, value: &OsStr) -> Result<u16, ParseError> {
    let text = value
        .to_str()
        .ok_or_else(|| ParseError::new(format!("invalid value for {option}: not UTF-8")))?;
    let parsed = text.parse::<u32>().map_err(|_| {
        ParseError::new(format!(
            "invalid value for {option}: '{text}' (expected integer 1..={MAX_DIMENSION})"
        ))
    })?;
    if !(1..=u32::from(MAX_DIMENSION)).contains(&parsed) {
        return Err(ParseError::new(format!(
            "invalid value for {option}: '{text}' (expected integer 1..={MAX_DIMENSION})"
        )));
    }
    Ok(parsed as u16)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_args(args: &[&str]) -> Result<Outcome, ParseError> {
        parse(args.iter().map(OsString::from))
    }

    fn run_options(args: &[&str]) -> Options {
        match parse_args(args).expect("arguments must parse") {
            Outcome::Run(options) => options,
            Outcome::Help => panic!("expected run options, got help"),
        }
    }

    #[test]
    fn help_is_detected_without_a_file() {
        assert_eq!(parse_args(&["--help"]).unwrap(), Outcome::Help);
        assert_eq!(parse_args(&["-h"]).unwrap(), Outcome::Help);
        assert_eq!(
            parse_args(&["--width", "0", "--help"]).unwrap(),
            Outcome::Help
        );
        assert!(HELP.contains("Disable OSC 8 only"));
        assert!(HELP.contains("3 file or terminal I/O error"));
        assert!(HELP.contains("4 TUI mode requires an interactive terminal"));
        assert!(HELP.contains("--once does not emit reference ANSI yet"));
    }

    #[test]
    fn parses_once_options_and_file() {
        let options = run_options(&[
            "--once",
            "--width",
            "80",
            "--height",
            "24",
            "--open-links",
            "doc.md",
        ]);
        assert!(options.once);
        assert_eq!(options.width, Some(80));
        assert_eq!(options.height, Some(24));
        assert_eq!(options.link_mode, LinkMode::OpenExternal);
        assert_eq!(options.file, PathBuf::from("doc.md"));
    }

    #[test]
    fn parses_no_links_semantics_as_a_model() {
        let options = run_options(&["--once", "--no-links", "doc.md"]);
        assert_eq!(options.link_mode, LinkMode::NoOsc8);
    }

    #[test]
    fn rejects_conflicting_link_options() {
        let error = parse_args(&["--once", "--open-links", "--no-links", "doc.md"]).unwrap_err();
        assert!(error.to_string().contains("cannot be used together"));
    }

    #[test]
    fn accepts_dimension_boundaries() {
        let low = run_options(&["--once", "--width", "1", "--height", "1", "doc.md"]);
        assert_eq!((low.width, low.height), (Some(1), Some(1)));
        let high = run_options(&["--once", "--width", "1000", "--height", "1000", "doc.md"]);
        assert_eq!((high.width, high.height), (Some(1000), Some(1000)));
    }

    #[test]
    fn rejects_invalid_dimensions() {
        for value in ["0", "1001", "65536", "-1", "abc", ""] {
            let error = parse_args(&["--once", "--width", value, "doc.md"]).unwrap_err();
            assert!(error.to_string().contains("expected integer 1..=1000"));
        }
        let error = parse_args(&["--once", "--height", "0", "doc.md"]).unwrap_err();
        assert!(error.to_string().contains("expected integer 1..=1000"));
    }

    #[test]
    fn rejects_tui_geometry_and_missing_geometry_values() {
        let error = parse_args(&["--width", "80", "doc.md"]).unwrap_err();
        assert!(error.to_string().contains("only valid with '--once'"));
        let error = parse_args(&["--once", "--width"]).unwrap_err();
        assert!(error.to_string().contains("requires a value"));
        let error = parse_args(&["--once", "--height"]).unwrap_err();
        assert!(error.to_string().contains("requires a value"));
    }

    #[test]
    fn rejects_duplicate_dimensions() {
        let error =
            parse_args(&["--once", "--width", "80", "--width", "81", "doc.md"]).unwrap_err();
        assert!(error.to_string().contains("may only be specified once"));
        let error =
            parse_args(&["--once", "--height", "24", "--height", "25", "doc.md"]).unwrap_err();
        assert!(error.to_string().contains("may only be specified once"));
    }

    #[test]
    fn requires_exactly_one_file() {
        let error = parse_args(&["--once"]).unwrap_err();
        assert!(error.to_string().contains("expected exactly one FILE"));
        let error = parse_args(&["--once", "one.md", "two.md"]).unwrap_err();
        assert!(error.to_string().contains("expected exactly one FILE"));
    }

    #[test]
    fn rejects_unknown_options() {
        let error = parse_args(&["--once", "--bogus", "doc.md"]).unwrap_err();
        assert!(error.to_string().contains("unknown option: --bogus"));
    }

    #[test]
    fn supports_double_dash_for_dash_prefixed_files() {
        let options = run_options(&["--once", "--", "--literal.md"]);
        assert_eq!(options.file, PathBuf::from("--literal.md"));
    }
}
