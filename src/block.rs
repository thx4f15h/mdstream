use crate::Span;

/// At most this many bytes (+3 to finish UTF-8) await block classification.
pub const BLOCK_LOOKAHEAD: usize = 1024;
pub const MAX_FENCE_LENGTH: usize = 32;

#[derive(Clone, Copy)]
pub(crate) struct Fence {
    pub byte: u8,
    pub length: usize,
}

pub(crate) enum Line {
    Text,
    Blank,
    Definition {
        label: Span,
        destination: Span,
        title: Option<Span>,
    },
    Setext(u8),
    FenceOpen {
        fence: Fence,
        info: Option<Span>,
    },
    FenceClose,
    Code,
    Indented {
        prefix: usize,
    },
    Heading {
        level: u8,
        prefix: usize,
        body_end: usize,
    },
    Container {
        prefix: usize,
        kind: crate::BlockKind,
    },
    Rule,
}

pub(crate) fn body_end(source: &str, span: Span) -> usize {
    let bytes = source.as_bytes();
    let mut end = span.end;
    if end > span.start && bytes[end - 1] == b'\n' {
        end -= 1;
        if end > span.start && bytes[end - 1] == b'\r' {
            end -= 1;
        }
    }
    end
}

pub(crate) fn classify(
    source: &str,
    span: Span,
    complete: bool,
    fence: Option<Fence>,
    paragraph: bool,
) -> Line {
    let end = body_end(source, span);
    let body = &source.as_bytes()[span.start..end];
    let spaces = body.iter().take_while(|&&b| b == b' ').count();
    let text = &body[spaces..];
    if let Some(fence) = fence {
        let run = text.iter().take_while(|&&b| b == fence.byte).count();
        return if complete
            && spaces <= 3
            && run >= fence.length
            && text[run..].iter().all(u8::is_ascii_whitespace)
        {
            Line::FenceClose
        } else {
            Line::Code
        };
    }
    if complete && body.iter().all(u8::is_ascii_whitespace) {
        return Line::Blank;
    }
    if spaces >= 4 {
        return Line::Indented {
            prefix: span.start + 4,
        };
    }
    if text.is_empty() {
        return Line::Text;
    }
    if complete {
        if paragraph && let Some(level) = setext_level(text) {
            return Line::Setext(level);
        }
        if let Some(definition) = definition(source, span.start + spaces, end) {
            return definition;
        }
        if matches!(text[0], b'*' | b'-' | b'_') {
            let mut count = 0;
            if text.iter().all(|&b| {
                if b == text[0] {
                    count += 1;
                    true
                } else {
                    b == b' ' || b == b'\t'
                }
            }) && count >= 3
            {
                return Line::Rule;
            }
        }
    }
    if matches!(text[0], b'`' | b'~') {
        let length = text.iter().take_while(|&&b| b == text[0]).count();
        if (3..=MAX_FENCE_LENGTH).contains(&length)
            && (text[0] != b'`' || !text[length..].contains(&b'`'))
        {
            let rest = &source[span.start + spaces + length..end];
            let info = rest.trim_ascii();
            let offset = rest.len() - rest.trim_ascii_start().len();
            let info_start = span.start + spaces + length + offset;
            return Line::FenceOpen {
                fence: Fence {
                    byte: text[0],
                    length,
                },
                info: (!info.is_empty()).then(|| Span::new(info_start, info_start + info.len())),
            };
        }
    }
    if text[0] == b'#' {
        let level = text.iter().take_while(|&&b| b == b'#').count();
        if (1..=6).contains(&level)
            && ((level == text.len() && complete)
                || text.get(level).is_some_and(u8::is_ascii_whitespace))
        {
            let mut prefix = span.start + spaces + level;
            while prefix < end && source.as_bytes()[prefix].is_ascii_whitespace() {
                prefix += 1;
            }
            let mut body_end = end;
            if complete {
                let trimmed = source[prefix..end].trim_ascii_end();
                let hashes = trimmed
                    .as_bytes()
                    .iter()
                    .rev()
                    .take_while(|&&b| b == b'#')
                    .count();
                let stop = prefix + trimmed.len() - hashes;
                if hashes > 0
                    && (stop == prefix || source.as_bytes()[stop - 1].is_ascii_whitespace())
                {
                    body_end = stop;
                }
            }
            return Line::Heading {
                level: level as u8,
                prefix,
                body_end,
            };
        }
    }
    Line::Text
}

fn definition(source: &str, start: usize, end: usize) -> Option<Line> {
    let text = &source.as_bytes()[start..end];
    if text.first() != Some(&b'[') {
        return None;
    }
    let close = text.iter().position(|&b| b == b']')?;
    if close <= 1 || text.get(close + 1) != Some(&b':') {
        return None;
    }
    let mut a = start + close + 2;
    let mut b = end;
    while a < b && source.as_bytes()[a].is_ascii_whitespace() {
        a += 1;
    }
    while a < b && source.as_bytes()[b - 1].is_ascii_whitespace() {
        b -= 1;
    }
    if a == b {
        return None;
    }
    let mut parser = crate::destination::Destination::new(a);
    for pos in a..b {
        if !matches!(
            parser.byte(source.as_bytes()[pos], pos),
            crate::destination::Progress::Pending
        ) {
            return None;
        }
    }
    let crate::destination::Progress::Complete { destination, title } = parser.finish_definition()
    else {
        return None;
    };
    Some(Line::Definition {
        label: Span::new(start + 1, start + close),
        destination,
        title,
    })
}

/// Caller has already removed up to three leading spaces and the physical LF.
pub(crate) fn setext_level(text: &[u8]) -> Option<u8> {
    let trimmed = text.trim_ascii_end();
    (matches!(trimmed.first(), Some(b'=' | b'-')) && trimmed.iter().all(|b| *b == trimmed[0]))
        .then(|| if trimmed[0] == b'=' { 1 } else { 2 })
}
