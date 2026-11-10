//! Deliberately small batch oracle for maximal tilde runs and ASCII escapes.
//! Uses reverse searches and final range painting, not the core's indexed
//! streaming stack/tree. Only used on bounded counterexamples, never in scoring.
pub fn marks(source: &str) -> Vec<(bool, bool)> {
    let bytes = source.as_bytes();
    let mut marks = vec![(false, false); bytes.len()];
    let mut openers: Vec<(usize, usize)> = Vec::new();
    let mut pairs = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'\\' && bytes.get(at + 1).is_some_and(u8::is_ascii_punctuation) {
            marks[at].0 = true;
            at += 2;
            continue;
        }
        if bytes[at] != b'~' {
            at += 1;
            continue;
        }
        let start = at;
        while bytes.get(at) == Some(&b'~') {
            at += 1;
        }
        let len = at - start;
        if len > 2 {
            continue;
        }
        let left = start > 0 && !bytes[start - 1].is_ascii_whitespace();
        let right = bytes.get(at).is_some_and(|b| !b.is_ascii_whitespace());
        if let Some(index) = openers.iter().rposition(|&(_, n)| left && n == len) {
            pairs.push((openers[index].0, start, len));
            // A crossing consumes every intervening opener, including the
            // other length. A closer is never reopened on the same run.
            openers.truncate(index);
        } else if right {
            openers.push((start, len));
        }
    }
    for (open, close, len) in pairs {
        for mark in &mut marks[open..open + len] {
            mark.0 = true;
        }
        for mark in &mut marks[close..close + len] {
            mark.0 = true;
        }
        for mark in &mut marks[open + len..close] {
            mark.1 = true;
        }
    }
    marks
}
