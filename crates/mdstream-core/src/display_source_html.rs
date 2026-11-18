//! Input-side continuation of the TUI's inline HTML visibility recognizer.
//! It retains punctuation state and source offsets, never a tag's text. Code,
//! escape and autolink qualification is decided from current core annotations.
#[derive(Default, Clone, PartialEq, Eq)]
pub struct HtmlShape {
    start: usize,
    mode: u8,
    prefix: [u8; 9],
    len: usize,
    quote: u8,
    tail: [u8; 3],
}
impl HtmlShape {
    /// Repetition is a finite-state transition too. Once the same punctuation
    /// state recurs, skip the remaining identical transitions and preserve the
    /// latest opener's offset. No source bytes or tag contents are rescanned.
    pub fn push_repeated(
        &mut self,
        at: usize,
        ch: char,
        count: usize,
        steps: &mut usize,
    ) -> Vec<crate::Span> {
        let mut spans = Vec::new();
        let mut done = 0;
        while done < count {
            let before = self.clone();
            if let Some(span) = self.push(at + done * ch.len_utf8(), ch) {
                spans.push(span);
            }
            *steps += 1;
            done += 1;
            let new_start = self.start;
            self.start = before.start;
            let stable = *self == before;
            self.start = new_start;
            if stable {
                if new_start != before.start {
                    self.start += (count - done) * ch.len_utf8();
                }
                break;
            }
        }
        spans
    }
    pub fn push(&mut self, at: usize, ch: char) -> Option<crate::Span> {
        let byte = if ch.is_ascii() { ch as u8 } else { 0 };
        if self.mode == 0 {
            if byte == b'<' {
                self.start = at;
                self.mode = 1;
                self.len = 1;
                self.prefix[0] = b'<';
                self.quote = 0;
                self.tail = [0; 3];
            }
            return None;
        }
        if self.len < 9 {
            self.prefix[self.len] = byte;
            self.len += 1;
        }
        if self.mode == 1 && self.len == 2 {
            self.mode = if byte.is_ascii_alphabetic() {
                3
            } else {
                match byte {
                    b'/' => 2,
                    b'!' | b'?' => 4,
                    _ => 0,
                }
            };
            if self.mode == 0 && byte == b'<' {
                self.start = at;
                self.mode = 1;
                self.len = 1;
            }
            return None;
        }
        if self.mode == 2 {
            self.mode = if byte.is_ascii_alphabetic() { 3 } else { 0 };
            return None;
        }
        if self.mode == 4 {
            if self.len == 4 && &self.prefix[..4] == b"<!--" {
                self.mode = 5;
            } else if self.len == 9 && &self.prefix[..9] == b"<![CDATA[" {
                self.mode = 6;
            }
        }
        self.tail.rotate_left(1);
        self.tail[2] = byte;
        let closed = if self.mode == 5 {
            self.tail == *b"-->"
        } else if self.mode == 6 {
            self.tail == *b"]]>"
        } else if matches!(self.mode, 3 | 4) {
            if self.quote == byte && self.quote != 0 {
                self.quote = 0;
                false
            } else if self.quote == 0 && matches!(byte, b'\'' | b'"') {
                self.quote = byte;
                false
            } else {
                self.quote == 0 && byte == b'>'
            }
        } else {
            false
        };
        if closed {
            self.mode = 0;
            Some(crate::Span::new(self.start, at + ch.len_utf8()))
        } else {
            None
        }
    }
    pub fn open(&self, end: usize) -> Option<crate::Span> {
        matches!(self.mode, 4..=6).then_some(crate::Span::new(self.start, end))
    }
}
