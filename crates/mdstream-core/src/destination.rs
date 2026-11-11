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
/// Spans retain escapes verbatim. Inline suffixes remain single-line; only
/// definition titles opt into line endings and known structural prefix gaps.
pub(crate) struct Destination {
    pub start: usize,
    stage: Stage,
    destination: Span,
    title: Option<Span>,
    escaped: bool,
    separated: bool,
    // Only definition title continuations can exclude structural source bytes.
    title_prefix_bytes: usize,
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
            title_prefix_bytes: 0,
        }
    }

    /// A standalone definition title, after its validated opening delimiter.
    pub fn title(start: usize, open: u8) -> Self {
        Self {
            stage: Stage::Title(if open == b'(' { b')' } else { open }),
            title: Some(Span::new(start + 1, start + 1)),
            ..Self::new(start)
        }
    }

    pub fn byte(&mut self, byte: u8, pos: usize) -> Progress {
        // Also bound whitespace and unfinished candidates, without rescanning.
        if pos - self.start - self.title_prefix_bytes >= MAX_LINK_SUFFIX_BYTES
            || matches!(byte, b'\n' | b'\r')
        {
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
                .is_some_and(|span| span.len() - self.title_prefix_bytes > MAX_LINK_TITLE_BYTES)
        {
            Progress::Invalid
        } else {
            Progress::Pending
        }
    }

    pub fn open_title(&self) -> bool {
        matches!(self.stage, Stage::Title(_))
    }

    // Only a definition body opts in. Inline suffixes and destinations
    // retain byte()'s physical-line rejection and the same raw byte limits.
    pub fn definition_title_byte(&mut self, byte: u8, pos: usize) -> Progress {
        if matches!(byte, b'\n' | b'\r') && self.open_title() {
            if pos - self.start - self.title_prefix_bytes >= MAX_LINK_SUFFIX_BYTES {
                return Progress::Invalid;
            }
            let title = self.title.as_mut().unwrap();
            title.end = pos + 1;
            self.escaped = false;
            return if title.len() - self.title_prefix_bytes <= MAX_LINK_TITLE_BYTES {
                Progress::Pending
            } else {
                Progress::Invalid
            };
        }
        self.byte(byte, pos)
    }

    // Called once per qualified continuation's real prefix. Keep the raw
    // envelope, including the prefix of a delimiter-only closing line, while
    // title and suffix limits count only real syntax bytes.
    pub fn title_prefix(&mut self, prefix: Span) {
        debug_assert!(self.open_title());
        let title = self.title.as_mut().unwrap();
        debug_assert_eq!(title.end, prefix.start);
        title.end = prefix.end;
        self.title_prefix_bytes += prefix.len();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn structural_title_gaps_do_not_spend_title_or_suffix_byte_limits() {
        for prefix_len in [0, 1, 1_000_000] {
            // Exercise the independently bounded suffix, with both its target
            // and title at their limits; physical windows remain a caller rule.
            let logical = format!(
                "{} 'a\n{}'",
                "d".repeat(MAX_DESTINATION_BYTES),
                "x".repeat(MAX_LINK_TITLE_BYTES - 2)
            );
            let split = MAX_DESTINATION_BYTES + 4;
            let mut parser = Destination::new(0);
            for (pos, byte) in logical.bytes().enumerate() {
                if pos == split {
                    parser.title_prefix(Span::new(pos, pos + prefix_len));
                }
                let physical = pos + if pos >= split { prefix_len } else { 0 };
                assert!(matches!(
                    parser.definition_title_byte(byte, physical),
                    Progress::Pending
                ));
            }
            let Progress::Complete {
                title: Some(title), ..
            } = parser.finish_definition()
            else {
                panic!("complete")
            };
            assert_eq!(title.len(), MAX_LINK_TITLE_BYTES + prefix_len);
            for pos in logical.len()..MAX_LINK_SUFFIX_BYTES {
                assert!(matches!(
                    parser.definition_title_byte(b' ', pos + prefix_len),
                    Progress::Pending
                ));
            }
            assert!(matches!(
                parser.definition_title_byte(b' ', MAX_LINK_SUFFIX_BYTES + prefix_len),
                Progress::Invalid
            ));

            let mut parser = Destination::title(0, b'\'');
            assert!(matches!(
                parser.definition_title_byte(b'\n', 1),
                Progress::Pending
            ));
            parser.title_prefix(Span::new(2, 2 + prefix_len));
            for pos in 2..=MAX_LINK_TITLE_BYTES {
                assert!(matches!(
                    parser.definition_title_byte(b'x', pos + prefix_len),
                    Progress::Pending
                ));
            }
            assert!(matches!(
                parser.definition_title_byte(b'x', MAX_LINK_TITLE_BYTES + 1 + prefix_len),
                Progress::Invalid
            ));
        }
    }

    #[test]
    fn definition_body_opt_in_keeps_destination_title_and_suffix_byte_limits() {
        for byte in *b"\n\r" {
            assert!(matches!(
                Destination::new(0).definition_title_byte(byte, 0),
                Progress::Invalid
            ));
            assert!(matches!(
                Destination::title(0, b'\'').byte(byte, 1),
                Progress::Invalid
            ));
        }
        for size in [MAX_DESTINATION_BYTES, MAX_DESTINATION_BYTES + 1] {
            let mut parser = Destination::new(0);
            let mut valid = true;
            for pos in 0..size {
                valid &= matches!(parser.definition_title_byte(b'x', pos), Progress::Pending);
            }
            assert_eq!(valid, size == MAX_DESTINATION_BYTES);
        }
        let mut parser = Destination::title(0, b'\'');
        for pos in 1..=MAX_LINK_TITLE_BYTES {
            assert!(matches!(
                parser.definition_title_byte(b'\n', pos),
                Progress::Pending
            ));
        }
        assert!(matches!(
            parser.definition_title_byte(b'x', MAX_LINK_TITLE_BYTES + 1),
            Progress::Invalid
        ));
        // A short, closed title with a long whitespace suffix must still stop
        // at the suffix bound independently of its title's size and newlines.
        let mut parser = Destination::title(0, b'\'');
        assert!(matches!(
            parser.definition_title_byte(b'\n', 1),
            Progress::Pending
        ));
        assert!(matches!(
            parser.definition_title_byte(b'\'', 2),
            Progress::Pending
        ));
        for pos in 3..MAX_LINK_SUFFIX_BYTES {
            assert!(matches!(
                parser.definition_title_byte(b' ', pos),
                Progress::Pending
            ));
        }
        assert!(matches!(
            parser.finish_definition(),
            Progress::Complete { .. }
        ));
        assert!(matches!(
            parser.definition_title_byte(b' ', MAX_LINK_SUFFIX_BYTES),
            Progress::Invalid
        ));
    }
}
