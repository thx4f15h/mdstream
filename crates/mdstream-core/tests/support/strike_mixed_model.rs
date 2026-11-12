//! Independent bounded, deliberately quadratic batch delimiter oracle.
//! No Document, annotation tree, classified event stream or expected HTML is read.
//! Only plain inline witnesses (ASCII flanking for emphasis) are accepted here;
//! opaque objects, Unicode categories and block scopes have separate fixed tests.
use mdstream::{InlineStyleKind, Span, StylePairId};

pub fn pairs(source: &str) -> Vec<StylePairId> {
    let bytes = source.as_bytes();
    let mut stack: Vec<(u8, Span, usize, bool)> = Vec::new();
    let mut pairs = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'\\' && bytes.get(at + 1).is_some_and(u8::is_ascii_punctuation) {
            at += 2;
            continue;
        }
        let marker = bytes[at];
        if !matches!(marker, b'*' | b'_' | b'~') {
            at += 1;
            continue;
        }
        let start = at;
        while bytes.get(at) == Some(&marker) {
            at += 1;
        }
        let len = at - start;
        if marker == b'~' && len > 2 {
            continue;
        }
        let before = start.checked_sub(1).map(|i| bytes[i]);
        let after = bytes.get(at).copied();
        let white = |c: Option<u8>| c.is_none_or(|c| c.is_ascii_whitespace());
        let punct = |c: Option<u8>| c.is_some_and(|c| c.is_ascii_punctuation());
        let left = !white(after) && (!punct(after) || white(before) || punct(before));
        let right = !white(before) && (!punct(before) || white(after) || punct(after));
        let (open, close) = match marker {
            b'~' => (!white(after), !white(before)),
            b'_' => (
                left && (!right || punct(before)),
                right && (!left || punct(after)),
            ),
            _ => (left, right),
        };
        let mut remaining = Span::new(start, at);
        while close && !remaining.is_empty() {
            let found = stack.iter().rposition(|&(m, span, original, can_close)| {
                m == marker
                    && if marker == b'~' {
                        span.len() == len
                    } else {
                        !((open || can_close)
                            && (original + len).is_multiple_of(3)
                            && (!original.is_multiple_of(3) || !len.is_multiple_of(3)))
                    }
            });
            let Some(i) = found else { break };
            stack.truncate(i + 1);
            let span = &mut stack[i].1;
            let used = if marker == b'~' {
                len
            } else {
                2.min(span.len()).min(remaining.len())
            };
            pairs.push(StylePairId {
                open: Span::new(span.end - used, span.end),
                close: Span::new(remaining.start, remaining.start + used),
                kind: if marker == b'~' {
                    InlineStyleKind::Strike
                } else if used == 2 {
                    InlineStyleKind::Strong
                } else {
                    InlineStyleKind::Emphasis
                },
            });
            span.end -= used;
            if span.is_empty() {
                stack.pop();
            }
            remaining.start += used;
        }
        if open && !remaining.is_empty() {
            stack.push((marker, remaining, len, close));
        }
    }
    pairs.sort_by_key(|p| p.open.start);
    pairs
}
