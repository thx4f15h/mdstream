use std::ffi::OsString;
use std::process::{Command, Stdio};
use std::thread;

use mdtui3::{Budget, Document, LinkId, LinkTarget, MAX_DESTINATION_BYTES, WorkLimit};

pub const MAX_EXTERNAL_URL_BYTES: usize = MAX_DESTINATION_BYTES + 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExternalTargetError {
    Missing,
    Empty,
    TooLong,
    ControlCharacter,
    InternalFragment,
    UnsupportedScheme,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalTarget {
    url: String,
}

impl ExternalTarget {
    pub fn from_document(document: &Document, id: LinkId) -> Result<Self, ExternalTargetError> {
        let destination = document
            .link_destination(id)
            .ok_or(ExternalTargetError::Missing)?;
        if destination.len() > MAX_EXTERNAL_URL_BYTES {
            return Err(ExternalTargetError::TooLong);
        }
        if destination.starts_with('#') {
            return Err(ExternalTargetError::InternalFragment);
        }
        let prefix = match document.links().get(id.0) {
            Some(LinkTarget::Email(_)) => "mailto:",
            Some(LinkTarget::Www(_)) => "http://",
            _ => "",
        };
        if destination.len().saturating_add(prefix.len()) > MAX_EXTERNAL_URL_BYTES {
            return Err(ExternalTargetError::TooLong);
        }
        let mut cursor = document
            .decoded_link_destination(id)
            .ok_or(ExternalTargetError::Missing)?;
        let mut budget = Budget::new(WorkLimit {
            bytes: MAX_EXTERNAL_URL_BYTES * 3 + 8,
            queries: MAX_EXTERNAL_URL_BYTES * 2 + 1,
            objects: MAX_EXTERNAL_URL_BYTES * 2 + 1,
            ..WorkLimit::default()
        });
        let mut url = String::from(prefix);
        while let Some(chunk) = cursor
            .advance(&mut budget)
            .map_err(|_| ExternalTargetError::TooLong)?
        {
            if url.len().saturating_add(chunk.text.len()) > MAX_EXTERNAL_URL_BYTES {
                return Err(ExternalTargetError::TooLong);
            }
            url.push_str(chunk.text);
        }
        Self::from_url(&url)
    }

    pub fn from_url(url: &str) -> Result<Self, ExternalTargetError> {
        if url.is_empty() {
            return Err(ExternalTargetError::Empty);
        }
        if url.len() > MAX_EXTERNAL_URL_BYTES {
            return Err(ExternalTargetError::TooLong);
        }
        if url.chars().any(|ch| ch.is_control() || ch.is_whitespace()) {
            return Err(ExternalTargetError::ControlCharacter);
        }
        if url.starts_with('#') {
            return Err(ExternalTargetError::InternalFragment);
        }
        if !supported_scheme(url) {
            return Err(ExternalTargetError::UnsupportedScheme);
        }
        Ok(Self {
            url: url.to_owned(),
        })
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.url
    }
}

fn supported_scheme(url: &str) -> bool {
    let Some(colon) = url.find(':') else {
        return false;
    };
    let scheme = &url[..colon];
    let rest = &url[colon + 1..];
    (scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https"))
        .then_some(rest.strip_prefix("//"))
        .flatten()
        .is_some_and(|host| !host.is_empty())
        || (scheme.eq_ignore_ascii_case("mailto") && !rest.is_empty())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExternalOpenStatus {
    Opened,
    Rejected,
    Unsupported,
    Failed,
}

impl ExternalOpenStatus {
    #[must_use]
    pub const fn from_target_error(error: ExternalTargetError) -> Self {
        match error {
            ExternalTargetError::UnsupportedScheme => Self::Unsupported,
            ExternalTargetError::Missing
            | ExternalTargetError::Empty
            | ExternalTargetError::TooLong
            | ExternalTargetError::ControlCharacter
            | ExternalTargetError::InternalFragment => Self::Rejected,
        }
    }

    #[must_use]
    pub const fn shows_notice(self) -> bool {
        true
    }

    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::Opened => "已打开外部链接",
            Self::Rejected => "无效的外部链接",
            Self::Unsupported => "不支持打开该链接",
            Self::Failed => "打开外部链接失败",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExternalOpenOutcome {
    pub status: ExternalOpenStatus,
}

impl ExternalOpenOutcome {
    #[must_use]
    pub const fn new(status: ExternalOpenStatus) -> Self {
        Self { status }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalCommand {
    pub program: OsString,
    pub args: Vec<OsString>,
}

/// Build an explicit argv for the platform opener. The URL is always a single
/// argument and is never interpreted by a shell.
pub fn command_for_external(
    target: &ExternalTarget,
) -> Result<ExternalCommand, ExternalOpenStatus> {
    #[cfg(target_os = "linux")]
    {
        Ok(ExternalCommand {
            program: OsString::from("xdg-open"),
            args: vec![OsString::from(target.as_str())],
        })
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = target;
        Err(ExternalOpenStatus::Unsupported)
    }
}

pub trait ExternalOpener {
    fn open(&mut self, target: &ExternalTarget) -> ExternalOpenOutcome;
}

#[derive(Debug, Default)]
pub struct SystemExternalOpener;

impl ExternalOpener for SystemExternalOpener {
    fn open(&mut self, target: &ExternalTarget) -> ExternalOpenOutcome {
        let command = match command_for_external(target) {
            Ok(command) => command,
            Err(status) => return ExternalOpenOutcome::new(status),
        };
        let mut child = match Command::new(&command.program)
            .args(&command.args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(child) => child,
            Err(_) => {
                return ExternalOpenOutcome::new(ExternalOpenStatus::Failed);
            }
        };
        let _ = thread::Builder::new()
            .name("mdtui-external-wait".to_owned())
            .spawn(move || {
                let _ = child.wait();
            });
        ExternalOpenOutcome::new(ExternalOpenStatus::Opened)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(source: &str) -> Document {
        let mut document = Document::new();
        document.append(source).unwrap();
        document.finish();
        document
    }

    #[test]
    fn argv_construction_is_explicit_and_mailto_is_normalized() {
        let document = parse(
            "<user@example.com> and [web](https://example.com/a?b=c) and [http](http://example.com/)\n",
        );
        let email = ExternalTarget::from_document(&document, LinkId(0)).unwrap();
        let web = ExternalTarget::from_document(&document, LinkId(1)).unwrap();
        let http = ExternalTarget::from_document(&document, LinkId(2)).unwrap();
        assert_eq!(email.as_str(), "mailto:user@example.com");
        assert_eq!(web.as_str(), "https://example.com/a?b=c");
        assert_eq!(http.as_str(), "http://example.com/");

        #[cfg(target_os = "linux")]
        for target in [&email, &web, &http] {
            let command = command_for_external(target).unwrap();
            assert_eq!(command.program, OsString::from("xdg-open"));
            assert_eq!(command.args, vec![OsString::from(target.as_str())]);
        }
        #[cfg(target_os = "linux")]
        {
            let shell_metacharacters =
                ExternalTarget::from_url("https://example.com/;$(echo-not-a-shell)").unwrap();
            let command = command_for_external(&shell_metacharacters).unwrap();
            assert_eq!(
                command.args,
                vec![OsString::from("https://example.com/;$(echo-not-a-shell)")]
            );
        }
        #[cfg(not(target_os = "linux"))]
        assert_eq!(
            command_for_external(&web),
            Err(ExternalOpenStatus::Unsupported)
        );
    }

    #[test]
    fn empty_control_internal_and_unknown_schemes_are_rejected() {
        assert_eq!(
            ExternalTarget::from_url(""),
            Err(ExternalTargetError::Empty)
        );
        assert_eq!(
            ExternalTarget::from_url("https://example.com/\u{1b}[31m"),
            Err(ExternalTargetError::ControlCharacter)
        );
        assert_eq!(
            ExternalTarget::from_url("https://example.com/\n"),
            Err(ExternalTargetError::ControlCharacter)
        );
        assert_eq!(
            ExternalTarget::from_url("https://example.com/\u{85}"),
            Err(ExternalTargetError::ControlCharacter)
        );
        assert_eq!(
            ExternalTarget::from_url("#fragment"),
            Err(ExternalTargetError::InternalFragment)
        );
        assert_eq!(
            ExternalTarget::from_url("ftp://example.com/file"),
            Err(ExternalTargetError::UnsupportedScheme)
        );
        assert_eq!(
            ExternalTarget::from_url("https://example.com/a b"),
            Err(ExternalTargetError::ControlCharacter)
        );
    }
}
