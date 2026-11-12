//! Deferred, disjoint raw text candidates. Growth only extends a source span;
//! sealing validates that span in a fixed number of linear passes. Failed Web
//! spans get one indexed recovery pass; their disjoint inner candidates release
//! plain bytes with candidate creation disabled.
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

#[derive(Clone, Copy, Debug)]
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

    pub fn byte(
        &mut self,
        byte: u8,
        pos: usize,
        source: &str,
        recovery: Option<&Recovery>,
    ) -> bool {
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
                            if matches!(kind, Kind::Web { .. })
                                && recovery.is_some_and(|r| {
                                    source.as_bytes().get(pos + 1..pos + 3) != Some(b"//")
                                        || !r.allows(pos + 3)
                                })
                            {
                                return false;
                            }
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
                && recovery.is_none_or(|r| r.allows(start))
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

/// Transient index for one failed Web span. Each entry says whether the host
/// suffix beginning there can seal as a Web address. Recovery consults this
/// before entering Web mode, so rejected nested prefixes never collect or
/// validate overlapping long tails. Accepted inner candidates are disjoint.
#[derive(Debug)]
pub(super) struct Recovery {
    start: usize,
    hosts: Vec<bool>,
}

impl Recovery {
    pub fn new(span: Span, source: &str, bytes: &mut u64) -> Self {
        let raw = &source[span.range()];
        // The tail can reach a host only if everything after it consists of
        // punctuation, excess ')' or entity-shaped tokens. Any '(' is plain
        // and stops trimming, so ignoring ')' balance here is exact for hosts.
        let mut tail = raw.len();
        while tail != 0 {
            *bytes += 1;
            match raw.as_bytes()[tail - 1] {
                b'?' | b'!' | b'.' | b',' | b':' | b'*' | b'_' | b'~' | b')' => {
                    tail -= 1;
                }
                b';' => {
                    let mut amp = tail - 1;
                    while amp != 0 && raw.as_bytes()[amp - 1].is_ascii_alphanumeric() {
                        *bytes += 1;
                        amp -= 1;
                    }
                    if amp == tail - 1 || amp == 0 || raw.as_bytes()[amp - 1] != b'&' {
                        break;
                    }
                    tail = amp - 1;
                }
                _ => break,
            }
        }
        let mut hosts = vec![false; raw.len()];
        *bytes += raw.len() as u64;
        let mut end = raw.len();
        let mut content = false;
        let mut trailing_dot = false;
        let mut periods = 0usize;
        let mut empty = false;
        let mut underscore = false;
        let mut right_dot = false;
        for (at, ch) in raw.char_indices().rev() {
            *bytes += ch.len_utf8() as u64;
            if !ch.is_alphanumeric() && !matches!(ch, '.' | '_' | '-') {
                end = at;
                content = false;
                trailing_dot = false;
                periods = 0;
                empty = false;
                underscore = false;
                right_dot = false;
                continue;
            }
            if !content && ch == '.' {
                trailing_dot = true;
                continue;
            }
            content = true;
            if ch == '.' {
                periods += 1;
                empty |= right_dot;
            } else if ch == '_' && periods < 2 {
                underscore = true;
            }
            right_dot = ch == '.';
            hosts[at] = !right_dot
                && periods != 0
                && !empty
                && !underscore
                && (!trailing_dot || tail <= end);
        }
        Self {
            start: span.start,
            hosts,
        }
    }

    fn allows(&self, body: usize) -> bool {
        body.checked_sub(self.start)
            .and_then(|at| self.hosts.get(at))
            .copied()
            .unwrap_or(false)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovery_index_agrees_with_web_sealing_for_every_suffix() {
        // Every suffix exercises a possible host start, including starts in
        // the last two segments, adjacent dots, entity tails and parentheses.
        // Compare the new reverse index against the existing forward seal.
        let alphabet = b"a._/);&";
        for len in 0..=6u32 {
            for mut value in 0..alphabet.len().pow(len) {
                let mut raw = String::new();
                for _ in 0..len {
                    raw.push(char::from(alphabet[value % alphabet.len()]));
                    value /= alphabet.len();
                }
                check(&raw);
            }
        }
        for host in ["a.b", "a_b.c.d", "a.b...", "例.子", "é._www.c"] {
            for suffix in ["", "??", "((a))", ")&abc;", ")&abc;;", "/?&a;", "_&a;"] {
                check(&format!("{host}{suffix}"));
            }
        }

        fn check(raw: &str) {
            let source = format!("offset {raw}");
            let mut work = 0;
            let recovery = Recovery::new(Span::new(7, source.len()), &source, &mut work);
            assert!(work <= 4 * raw.len() as u64 + 1, "{raw:?}: {work}");
            for (at, _) in raw.char_indices() {
                assert_eq!(
                    recovery.allows(at + 7),
                    web(&raw[at..], 0, &mut 0).is_some(),
                    "{raw:?} at {at}"
                );
            }
            assert!(!recovery.allows(6));
            assert!(!recovery.allows(source.len()));
        }
    }
}
