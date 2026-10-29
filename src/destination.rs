use crate::Span;

pub const MAX_DESTINATION_BYTES: usize = 4096;
pub const MAX_LINK_TITLE_BYTES: usize = 4096;
/// Includes whitespace and the closing parenthesis, excludes the opening one.
pub const MAX_LINK_SUFFIX_BYTES: usize = MAX_DESTINATION_BYTES + MAX_LINK_TITLE_BYTES + 64;

#[derive(Clone, Copy)]
enum Stage {
    Before,
    Bare(usize),
    Angle,
    AfterDestination,
    Title(u8),
    AfterTitle,
}

pub(crate) enum Progress {
    Pending,
    Complete {
        destination: Span,
        title: Option<Span>,
    },
    Invalid,
}

/// One pass over a destination and optional title, starting after `(` or `:`.
/// Spans retain escapes verbatim. Only single-line suffixes are accepted.
pub(crate) struct Destination {
    pub start: usize,
    stage: Stage,
    destination: Span,
    title: Option<Span>,
    escaped: bool,
    separated: bool,
}

impl Destination {
    pub fn new(start: usize) -> Self {
        Self {
            start,
            stage: Stage::Before,
            destination: Span::new(start, start),
            title: None,
            escaped: false,
            separated: false,
        }
    }

    pub fn byte(&mut self, byte: u8, pos: usize) -> Progress {
        // Also bound whitespace and unfinished candidates, without rescanning.
        if pos - self.start >= MAX_LINK_SUFFIX_BYTES || matches!(byte, b'\n' | b'\r') {
            return Progress::Invalid;
        }
        if matches!(self.stage, Stage::Before) {
            if matches!(byte, b' ' | b'\t') {
                self.destination = Span::new(pos + 1, pos + 1);
                return Progress::Pending;
            }
            if byte == b'<' {
                self.destination = Span::new(pos + 1, pos + 1);
                self.stage = Stage::Angle;
                return Progress::Pending;
            }
            self.destination = Span::new(pos, pos);
            self.stage = Stage::Bare(0);
        }
        match self.stage {
            Stage::Bare(mut depth) => {
                if !self.escaped {
                    match byte {
                        b')' if depth == 0 => return self.complete(),
                        b')' => depth -= 1,
                        b'(' => depth += 1,
                        b' ' | b'\t' if depth == 0 => {
                            self.stage = Stage::AfterDestination;
                            self.separated = true;
                            return Progress::Pending;
                        }
                        b'<' => return Progress::Invalid,
                        _ => {}
                    }
                }
                if byte <= b' ' || byte == 127 {
                    return Progress::Invalid;
                }
                self.stage = Stage::Bare(depth);
                self.destination.end = pos + 1;
            }
            Stage::Angle => {
                if !self.escaped {
                    if byte == b'>' {
                        self.stage = Stage::AfterDestination;
                        return Progress::Pending;
                    }
                    if byte == b'<' {
                        return Progress::Invalid;
                    }
                }
                self.destination.end = pos + 1;
            }
            Stage::AfterDestination => {
                if byte == b')' {
                    return self.complete();
                }
                if matches!(byte, b' ' | b'\t') {
                    self.separated = true;
                    return Progress::Pending;
                }
                if self.separated && matches!(byte, b'\'' | b'"' | b'(') {
                    self.title = Some(Span::new(pos + 1, pos + 1));
                    self.stage = Stage::Title(if byte == b'(' { b')' } else { byte });
                    return Progress::Pending;
                }
                return Progress::Invalid;
            }
            Stage::Title(close) => {
                if !self.escaped {
                    if byte == close {
                        self.stage = Stage::AfterTitle;
                        return Progress::Pending;
                    }
                    if close == b')' && byte == b'(' {
                        return Progress::Invalid;
                    }
                }
                self.title.as_mut().unwrap().end = pos + 1;
            }
            Stage::AfterTitle => {
                return match byte {
                    b')' => self.complete(),
                    b' ' | b'\t' => Progress::Pending,
                    _ => Progress::Invalid,
                };
            }
            Stage::Before => unreachable!(),
        }
        self.escaped = byte == b'\\' && !self.escaped;
        if self.destination.len() > MAX_DESTINATION_BYTES
            || self
                .title
                .is_some_and(|span| span.len() > MAX_LINK_TITLE_BYTES)
        {
            Progress::Invalid
        } else {
            Progress::Pending
        }
    }

    fn complete(&self) -> Progress {
        Progress::Complete {
            destination: self.destination,
            title: self.title,
        }
    }

    pub fn finish_definition(&self) -> Progress {
        match self.stage {
            Stage::Bare(0) | Stage::AfterDestination | Stage::AfterTitle => self.complete(),
            _ => Progress::Invalid,
        }
    }
}
