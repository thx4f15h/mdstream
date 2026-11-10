//! Small batch oracle: a category truth table, reverse opener search, and final
//! range painting. No streaming state, indexed stack, or scoring projection.
#[derive(Clone, Copy)]
enum Class {
    White,
    Punct,
    Other,
}

#[path = "unicode_categories.rs"]
mod unicode;

fn class(ch: Option<char>) -> Class {
    match ch.map(|c| unicode::classes()[c as usize]) {
        None | Some(1) => Class::White,
        Some(2) => Class::Punct,
        _ => Class::Other,
    }
}

pub fn marks(source: &str) -> Vec<(bool, bool, bool)> {
    use Class::*;
    let bytes = source.as_bytes();
    let mut result = vec![(false, false, false); bytes.len()];
    let mut stack: Vec<(u8, usize, usize)> = Vec::new();
    let mut pairs = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'\\' && bytes.get(at + 1).is_some_and(u8::is_ascii_punctuation) {
            result[at].0 = true;
            at += 2;
            continue;
        }
        let marker = bytes[at];
        if marker != b'*' && marker != b'_' {
            at += 1;
            continue;
        }
        let start = at;
        while bytes.get(at) == Some(&marker) {
            at += 1;
        }
        let len = at - start;
        if len > 2 {
            continue;
        }
        let before = class(source[..start].chars().next_back());
        let after = class(source[at..].chars().next());
        let (left, right) = match (before, after) {
            (White, White) => (false, false),
            (White, Punct | Other) | (Punct, Other) => (true, false),
            (Punct | Other, White) | (Other, Punct) => (false, true),
            (Punct, Punct) | (Other, Other) => (true, true),
        };
        let (open, close) = if marker == b'_' {
            (
                left && (!right || matches!(before, Punct)),
                right && (!left || matches!(after, Punct)),
            )
        } else {
            (left, right)
        };
        if let Some(index) = stack
            .iter()
            .rposition(|&(m, n, _)| close && m == marker && n == len)
        {
            pairs.push((stack[index].2, start, len));
            stack.truncate(index);
        } else if open {
            stack.push((marker, len, start));
        }
    }
    for (open, close, len) in pairs {
        for mark in &mut result[open..open + len] {
            mark.0 = true;
        }
        for mark in &mut result[close..close + len] {
            mark.0 = true;
        }
        for mark in &mut result[open + len..close] {
            if len == 1 {
                mark.1 = true;
            } else {
                mark.2 = true;
            }
        }
    }
    result
}
