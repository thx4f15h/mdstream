//! Qualification of an undecided physical line, without publishing containers.
//! This is deliberately a superset of structural grammar: the ordinary column,
//! scope and precedence classifier makes the single final decision. A rejected
//! byte hands the accumulated raw source to that classifier exactly once.
use crate::{
    Span, Work,
    destination::{Destination, Progress},
};

pub(crate) enum Continuation {
    Target,
    Title,
    Body(Destination),
    Label(usize),
}

enum State {
    Prefix,
    Bullet(u8),
    Digits(u8, u32),
    Separator,
    Rule(u8),
    Equals(bool),
    Dollars(u8),
    Math,
    Atx(u8),
    Blank,
    Label { start: usize, escaped: bool },
    Colon { footnote: bool, task: bool },
    Footnote,
    TaskGap,
    Target(Tail),
    Dead,
}

enum Extra {
    Waiting(Continuation),
    Label { remaining: usize, escaped: bool },
    Colon,
    Tail(Tail),
    Dead,
}

/// Fixed-size grammar probe. Whitespace outside an open title is delayed as a
/// span, matching definition_target's trim; it is read once if later internal.
struct Tail {
    parser: Option<Destination>,
    whitespace: Option<Span>,
    trim: bool,
}

impl Tail {
    fn new(parser: Option<Destination>) -> Self {
        Self {
            parser,
            whitespace: None,
            trim: true,
        }
    }

    fn byte(&mut self, source: &str, at: usize, work: &mut Work) -> bool {
        let byte = source.as_bytes()[at];
        if self.trim
            && byte.is_ascii_whitespace()
            && self.parser.as_ref().is_none_or(|p| !p.open_title())
        {
            if self.parser.is_some() {
                self.whitespace = Some(Span::new(self.whitespace.map_or(at, |s| s.start), at + 1));
            }
            return true;
        }
        let parser = self.parser.get_or_insert_with(|| Destination::new(at));
        if let Some(space) = self.whitespace.take() {
            for pos in space.range() {
                work.structural_prefix_bytes += 1;
                if !matches!(parser.byte(source.as_bytes()[pos], pos), Progress::Pending) {
                    return false;
                }
            }
        }
        matches!(parser.byte(byte, at), Progress::Pending)
    }
}

pub(crate) struct Scanner {
    start: usize,
    state: State,
    extra: Extra,
    carriage: Option<usize>,
    task_eligible: bool,
    prose: bool,
    explicit: bool,
    continuation: bool,
    root: bool,
    column: u8,
    root_indent: u8,
}

impl Scanner {
    pub fn new(
        start: usize,
        continuation: Option<Continuation>,
        prose: bool,
        root: bool,
        work: &mut Work,
    ) -> Self {
        work.structural_prefix_steps += 1;
        Self {
            start,
            state: State::Prefix,
            continuation: continuation.is_some(),
            extra: continuation.map_or(Extra::Dead, Extra::Waiting),
            carriage: None,
            task_eligible: false,
            prose,
            explicit: false,
            root,
            column: 0,
            root_indent: 0,
        }
    }

    pub fn take_definition_rejection(&mut self) -> bool {
        let rejected = self.continuation && matches!(self.extra, Extra::Dead);
        if rejected {
            self.continuation = false;
        }
        rejected
    }

    pub fn feed(&mut self, source: &str, span: Span, work: &mut Work) -> bool {
        for at in span.range() {
            work.structural_prefix_bytes += 1;
            if source.as_bytes()[at] == b'\n' {
                return true;
            }
            if let Some(cr) = self.carriage.take() {
                self.extra_byte(source, cr, work);
                self.byte(source, cr, work);
            }
            if source.as_bytes()[at] == b'\r' {
                self.carriage = Some(at);
                continue;
            }
            self.extra_byte(source, at, work);
            self.byte(source, at, work);
            if matches!(self.state, State::Dead) && matches!(self.extra, Extra::Dead) {
                return false;
            }
        }
        true
    }

    fn extra_byte(&mut self, source: &str, at: usize, work: &mut Work) {
        work.structural_prefix_steps += 1;
        let byte = source.as_bytes()[at];
        let extra = std::mem::replace(&mut self.extra, Extra::Dead);
        self.extra = match extra {
            Extra::Waiting(mode) if matches!(byte, b' ' | b'\t' | b'>') => Extra::Waiting(mode),
            Extra::Waiting(mode) => {
                let tail = match mode {
                    Continuation::Target => Tail::new(None),
                    Continuation::Title if matches!(byte, b'\'' | b'"' | b'(') => {
                        let mut tail = Tail::new(Some(Destination::title(at, byte)));
                        tail.trim = false;
                        self.extra = Extra::Tail(tail);
                        return;
                    }
                    Continuation::Title => return,
                    Continuation::Body(mut parser) => {
                        parser.title_prefix(Span::new(self.start, at));
                        let mut tail = Tail::new(Some(parser));
                        tail.trim = false;
                        tail
                    }
                    Continuation::Label(remaining) => {
                        self.extra = Extra::Label {
                            remaining,
                            escaped: false,
                        };
                        self.extra_byte(source, at, work);
                        return;
                    }
                };
                self.extra = Extra::Tail(tail);
                self.extra_byte(source, at, work);
                return;
            }
            Extra::Label { remaining, escaped } => {
                if !escaped && byte == b']' {
                    Extra::Colon
                } else if remaining == 0 || (!escaped && byte == b'[') {
                    Extra::Dead
                } else {
                    Extra::Label {
                        remaining: remaining - 1,
                        escaped: byte == b'\\' && !escaped,
                    }
                }
            }
            Extra::Colon if byte == b':' => Extra::Tail(Tail::new(None)),
            Extra::Tail(mut tail) => {
                if tail.byte(source, at, work) {
                    Extra::Tail(tail)
                } else {
                    Extra::Dead
                }
            }
            _ => Extra::Dead,
        };
    }

    fn byte(&mut self, source: &str, at: usize, work: &mut Work) {
        work.structural_prefix_steps += 1;
        let byte = source.as_bytes()[at];
        if self.root && !self.explicit && matches!(self.state, State::Prefix) {
            let width = match byte {
                b' ' => 1,
                b'\t' => 4 - self.column,
                _ => 0,
            };
            self.root_indent = self.root_indent.saturating_add(width);
            if width == 0 && !byte.is_ascii_whitespace() && self.root_indent >= 4 {
                self.state = State::Dead;
            }
        }
        self.column = if byte == b'\t' {
            0
        } else {
            (self.column + 1) % 4
        };
        let state = std::mem::replace(&mut self.state, State::Dead);
        self.state = match state {
            State::Prefix => match byte {
                b' ' | b'\t' => State::Prefix,
                b'>' => {
                    self.task_eligible = false;
                    self.explicit = true;
                    State::Prefix
                }
                b'-' | b'*' | b'+' => State::Bullet(byte),
                b'0'..=b'9' => State::Digits(1, u32::from(byte - b'0')),
                b'_' => State::Rule(byte),
                b'=' => State::Equals(false),
                b'$' => State::Dollars(1),
                b'#' => State::Atx(1),
                b'[' if !self.prose || self.explicit => State::Label {
                    start: at + 1,
                    escaped: false,
                },
                b if b.is_ascii_whitespace() => State::Blank,
                _ => State::Dead,
            },
            State::Bullet(marker) if byte == marker && marker != b'+' => State::Rule(marker),
            State::Bullet(_) | State::Separator if matches!(byte, b' ' | b'\t') => {
                self.task_eligible = true;
                self.explicit = true;
                State::Prefix
            }
            State::Digits(count, number) if byte.is_ascii_digit() && count < 9 => {
                State::Digits(count + 1, number * 10 + u32::from(byte - b'0'))
            }
            State::Digits(_, number) if matches!(byte, b'.' | b')') => {
                if self.root && self.prose && !self.explicit && number != 1 {
                    State::Dead
                } else {
                    State::Separator
                }
            }
            State::Rule(marker) if byte == marker || matches!(byte, b' ' | b'\t') => {
                State::Rule(marker)
            }
            State::Equals(false) if byte == b'=' => State::Equals(false),
            State::Equals(_) if byte.is_ascii_whitespace() => State::Equals(true),
            State::Dollars(1) if byte == b'$' => State::Dollars(2),
            State::Dollars(2) | State::Math if matches!(byte, b' ' | b'\t' | b'\r') => State::Math,
            State::Atx(count) if byte == b'#' && count < 6 => State::Atx(count + 1),
            State::Blank if byte.is_ascii_whitespace() => State::Blank,
            State::Label { start, escaped } => {
                if !escaped && byte == b']' {
                    State::Colon {
                        footnote: source.as_bytes().get(start) == Some(&b'^'),
                        task: self.task_eligible
                            && at - start == 1
                            && matches!(source.as_bytes()[start], b' ' | b'x' | b'X'),
                    }
                } else if at - start > crate::MAX_LABEL_BYTES || (!escaped && byte == b'[') {
                    State::Dead
                } else {
                    State::Label {
                        start,
                        escaped: byte == b'\\' && !escaped,
                    }
                }
            }
            State::Colon { footnote, .. } if byte == b':' => {
                if footnote {
                    State::Footnote
                } else {
                    State::Target(Tail::new(None))
                }
            }
            State::Colon { task: true, .. } | State::TaskGap if matches!(byte, b' ' | b'\t') => {
                State::TaskGap
            }
            State::Target(mut tail) => {
                if tail.byte(source, at, work) {
                    State::Target(tail)
                } else {
                    State::Dead
                }
            }
            _ => State::Dead,
        };
    }
}
