//! Deferred, disjoint raw text candidates. Growth only extends a source span;
//! sealing validates that span in a fixed number of linear passes, then releases
//! each rejected/plain byte to
//! the ordinary inline machine at most once, with candidate creation disabled.
//! No candidate length limit, copied body, growing-prefix scan or rollback IDs.
use crate::Span;

#[derive(Clone, Copy, Debug)]
pub(super) enum Kind {
    Local,
    Email,
    Web {
        www: bool,
    },
    Protocol {
        xmpp: bool,
        resource: bool,
        domain: bool,
    },
}

#[derive(Debug)]
pub(super) struct Candidate {
    pub span: Span,
    pub address: usize,
    pub at: Option<usize>,
    pub kind: Kind,
    slashes: u8,
    pub before: Option<u8>,
    pub before_scalar: Option<char>,
}

pub(super) fn local(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || b".-_+".contains(&byte)
}

fn boundary(ch: Option<char>) -> bool {
    ch.is_none_or(|ch| ch.is_ascii_whitespace() || matches!(ch, '*' | '_' | '~' | '('))
}

impl Candidate {
    pub fn new(pos: usize, before: Option<u8>, before_scalar: Option<char>) -> Self {
        Self {
            span: Span::new(pos, pos),
            address: pos,
            at: None,
            kind: Kind::Local,
            slashes: 0,
            before,
            before_scalar,
        }
    }

    // Prefix lookbehind is bounded by eight ASCII bytes. Even a million-byte
    // local part never causes a search/copy when ':' or '.' arrives.
    fn prefix(&self, source: &str, end: usize, prefix: &str) -> Option<usize> {
        let start = end.checked_sub(prefix.len())?;
        if start < self.span.start || &source.as_bytes()[start..end] != prefix.as_bytes() {
            return None;
        }
        let before = if start == self.span.start {
            self.before_scalar
        } else {
            Some(char::from(source.as_bytes()[start - 1]))
        };
        let valid = if matches!(prefix, "mailto" | "xmpp") {
            before.is_none_or(|ch| !ch.is_ascii_alphanumeric())
        } else {
            boundary(before)
        };
        valid.then_some(start)
    }

    pub fn byte(&mut self, byte: u8, pos: usize, source: &str) -> bool {
        let accept = match self.kind {
            Kind::Local => {
                if byte == b'@' {
                    self.kind = Kind::Email;
                    self.at = Some(pos);
                    true
                } else if byte == b':' {
                    let mut found = false;
                    for (prefix, kind) in [
                        ("http", Kind::Web { www: false }),
                        ("https", Kind::Web { www: false }),
                        (
                            "mailto",
                            Kind::Protocol {
                                xmpp: false,
                                resource: false,
                                domain: false,
                            },
                        ),
                        (
                            "xmpp",
                            Kind::Protocol {
                                xmpp: true,
                                resource: false,
                                domain: false,
                            },
                        ),
                    ] {
                        if let Some(start) = self.prefix(source, pos, prefix) {
                            self.address = start;
                            self.kind = kind;
                            self.slashes = if matches!(kind, Kind::Web { .. }) {
                                2
                            } else {
                                0
                            };
                            found = true;
                            break;
                        }
                    }
                    found
                } else {
                    local(byte)
                }
            }
            Kind::Email => {
                if byte == b'@' {
                    // The prior domain is now the possible local part. Keep
                    // its start in fixed state; never rediscover it by replay.
                    self.address = self.at.unwrap() + 1;
                    self.at = Some(pos);
                    true
                } else {
                    byte.is_ascii_alphanumeric() || b".-_".contains(&byte)
                }
            }
            Kind::Web { .. } => {
                if self.slashes != 0 {
                    if byte != b'/' {
                        return false;
                    }
                    self.slashes -= 1;
                    true
                } else {
                    byte > b' ' && byte != 0x7f && !b"<[]`$\\".contains(&byte)
                }
            }
            Kind::Protocol {
                xmpp,
                resource,
                domain,
            } => {
                if resource {
                    byte.is_ascii_alphanumeric() || b"@.".contains(&byte)
                } else if byte == b'@' && !domain {
                    self.at = Some(pos);
                    self.kind = Kind::Protocol {
                        xmpp,
                        resource: false,
                        domain: true,
                    };
                    true
                } else if byte == b'@' {
                    self.address = self.at.unwrap() + 1;
                    self.at = Some(pos);
                    self.kind = Kind::Email;
                    true
                } else if byte == b'/' && xmpp && domain {
                    self.kind = Kind::Protocol {
                        xmpp,
                        resource: true,
                        domain,
                    };
                    true
                } else if domain {
                    byte.is_ascii_alphanumeric() || b".-_".contains(&byte)
                } else {
                    local(byte)
                }
            }
        };
        if accept {
            self.span.end = pos + 1;
            if matches!(self.kind, Kind::Local)
                && byte == b'.'
                && let Some(start) = self.prefix(source, pos + 1, "www.")
            {
                self.address = start;
                self.kind = Kind::Web { www: true };
            }
        }
        accept
    }

    // Only sealing calls this. All scans are over this disjoint candidate,
    // never over an old paragraph, a prior candidate or historical use sites.
    pub fn seal(&self, source: &str, bytes: &mut u64) -> Option<(Span, bool)> {
        let raw = &source[self.address..self.span.end];
        *bytes += raw.len() as u64;
        match self.kind {
            Kind::Local => None,
            Kind::Email => {
                email(raw).map(|end| (Span::new(self.address, self.address + end), false))
            }
            Kind::Protocol { xmpp, .. } => {
                let prefix = if xmpp { 5 } else { 7 };
                let body = &raw[prefix..];
                let (address, resource) = body
                    .split_once('/')
                    .map_or((body, None), |(a, r)| (a, Some(r)));
                let end = email(address)?;
                if end != address.len() && resource.is_some() {
                    return None;
                }
                let end = if let Some(resource) = resource {
                    let trimmed = resource.trim_end_matches('.');
                    if trimmed.is_empty() {
                        return None;
                    }
                    prefix + address.len() + 1 + trimmed.len()
                } else {
                    prefix + end
                };
                Some((Span::new(self.address, self.address + end), false))
            }
            Kind::Web { www } => {
                let body_start = if www {
                    0
                } else if raw.starts_with("https://") {
                    8
                } else if raw.starts_with("http://") {
                    7
                } else {
                    return None;
                };
                let end = web(raw, body_start, bytes)?;
                Some((Span::new(self.address, self.address + end), www))
            }
        }
    }
}

// One forward validation pass. Coalesced plain runs and suffix tokens retain
// only possible trim boundaries; each token is pushed and removed at most once.
// Entity-shaped suffixes collapse their own tokens, never re-reading raw bytes.
fn web(raw: &str, body_start: usize, bytes: &mut u64) -> Option<usize> {
    #[derive(Clone, Copy, PartialEq)]
    enum Token {
        Plain,
        Amp,
        Punctuation,
        Close,
        Entity,
    }
    struct End {
        at: usize,
        valid: bool,
        token: Token,
    }
    let mut ends: Vec<End> = Vec::new();
    let mut amp: Option<(usize, usize)> = None;
    let mut opens = 0usize;
    let mut closes = 0usize;
    let mut domain = Domain::default();
    for (at, ch) in raw.char_indices() {
        *bytes += ch.len_utf8() as u64;
        if at >= body_start {
            domain.push(ch);
        }
        let valid = domain.valid();
        opens += usize::from(ch == '(');
        closes += usize::from(ch == ')');
        let token = if ch == '&' {
            amp = Some((ends.len(), at));
            Token::Amp
        } else if ch == ';' && amp.is_some_and(|(_, start)| at > start + 1) {
            let (index, _) = amp.take().unwrap();
            ends.truncate(index);
            Token::Entity
        } else {
            if !ch.is_ascii_alphanumeric() {
                amp = None;
            }
            match ch {
                '?' | '!' | '.' | ',' | ':' | '*' | '_' | '~' => Token::Punctuation,
                ')' => Token::Close,
                _ => Token::Plain,
            }
        };
        let end = End {
            at: at + ch.len_utf8(),
            valid,
            token,
        };
        if token == Token::Plain && ends.last().is_some_and(|e| e.token == Token::Plain) {
            *ends.last_mut().unwrap() = end;
        } else {
            ends.push(end);
        }
    }
    // Host validity precedes path trimming. A terminal underscore remains a
    // host character even when it could also be an emphasis delimiter.
    if domain.terminal_underscore {
        return None;
    }
    let mut excess = closes.saturating_sub(opens);
    while let Some(last) = ends.last() {
        *bytes += 1;
        match last.token {
            Token::Punctuation | Token::Entity => (),
            Token::Close if excess != 0 => excess -= 1,
            _ => break,
        }
        ends.pop();
    }
    let last = ends.last()?;
    (last.valid && last.at > body_start).then_some(last.at)
}

#[derive(Default)]
struct Domain {
    periods: usize,
    nonempty: bool,
    empty_segment: bool,
    underscore: bool,
    previous_underscore: bool,
    terminal_underscore: bool,
    closed: bool,
}
impl Domain {
    fn valid(&self) -> bool {
        self.periods >= 1
            && self.nonempty
            && !self.empty_segment
            && !self.underscore
            && !self.previous_underscore
    }
    fn push(&mut self, ch: char) {
        if self.closed {
            return;
        }
        if ch == '.' {
            self.empty_segment |= !self.nonempty;
            self.periods += 1;
            self.previous_underscore = self.underscore;
            self.underscore = false;
            self.nonempty = false;
        } else if ch.is_alphanumeric() || matches!(ch, '_' | '-') {
            self.nonempty = true;
            self.underscore |= ch == '_';
            self.terminal_underscore = self.underscore || self.previous_underscore;
        } else {
            self.closed = true;
        }
    }
}

pub(super) fn email(raw: &str) -> Option<usize> {
    let mut at_sign = false;
    let mut local_bytes = 0;
    let mut end = None;
    let mut pending_dots = 0;
    let mut period = false;
    let mut last = 0;
    for (at, byte) in raw.bytes().enumerate() {
        if !at_sign {
            if byte == b'@' {
                at_sign = true;
            } else if local(byte) {
                local_bytes += 1;
            } else {
                return None;
            }
        } else if byte == b'.' {
            pending_dots += 1;
        } else if byte.is_ascii_alphanumeric() || b"-_".contains(&byte) {
            if pending_dots != 0 {
                if end.is_none() || pending_dots != 1 {
                    return None;
                }
                period = true;
                pending_dots = 0;
            }
            last = byte;
            end = Some(at + 1);
        } else {
            return None;
        }
    }
    (at_sign && local_bytes != 0 && period && !b"-_".contains(&last)).then_some(end?)
}
