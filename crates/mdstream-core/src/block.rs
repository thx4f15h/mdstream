use crate::Span;

/// General block classification prefix bound (+3 to finish UTF-8). A long
/// table header/separator or fence can defer a raw suffix without copying it.
pub const BLOCK_LOOKAHEAD: usize = 1024;
/// Historical bounded-classifier threshold, retained for source compatibility.
/// This is **not** a fence length limit: opening/closing runs have no grammar
/// maximum. Do not use this value to validate fences or enumerate all lengths.
#[deprecated(note = "historical classification threshold; fences have no maximum length")]
pub const MAX_FENCE_LENGTH: usize = BLOCK_LOOKAHEAD - 1;

#[derive(Clone, Copy)]
pub(crate) struct Fence {
    pub byte: u8,
    pub length: usize,
    /// Indent relative to the matched container prefix; math keeps its literal body.
    pub indent: u8,
    /// At most three spaces may continue beyond the classification window.
    pub remaining_indent: u8,
    /// Physical column modulo four at the next unconsumed body byte.
    pub indent_column: u8,
}

pub(crate) enum Line {
    Text,
    Blank,
    FootnoteDefinition {
        label: Span,
        content: Span,
    },
    Definition {
        label: Span,
        destination: Span,
        title: Option<Span>,
    },
    DefinitionTitle {
        label: Span,
        parser: crate::destination::Destination,
    },
    DefinitionStart {
        label: Span,
    },
    DefinitionLabel {
        label: Span,
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
        virtual_spaces: u8,
    },
    Heading {
        level: u8,
        prefix: usize,
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

// Only bounded physical prefix bytes, shared by ordinary commit and its probe.
pub(crate) fn prefix_column(source: &str, prefix: Span, work: &mut crate::Work) -> u8 {
    let mut column = 0;
    for &byte in &source.as_bytes()[prefix.range()] {
        work.classified_bytes += 1;
        column = if byte == b'\t' { 0 } else { (column + 1) % 4 };
    }
    column
}

pub(crate) fn classify(
    source: &str,
    span: Span,
    complete: bool,
    fence: Option<Fence>,
    paragraph: bool,
) -> Line {
    classify_indented(source, span, complete, fence, paragraph, 0, 0)
}

/// Unconsumed columns of a container-prefix tab precede this source slice.
/// They affect indentation only; all returned offsets remain source bytes.
/// `column` is the physical column modulo four at span.start, after any tab.
pub(crate) fn classify_indented(
    source: &str,
    span: Span,
    complete: bool,
    fence: Option<Fence>,
    paragraph: bool,
    virtual_spaces: u8,
    column: u8,
) -> Line {
    debug_assert!(virtual_spaces <= 3);
    if let Some(fence) = fence {
        return classify_fenced(source, span, complete, fence, virtual_spaces, column);
    }
    let end = body_end(source, span);
    let body = &source.as_bytes()[span.start..end];
    let spaces = body.iter().take_while(|&&b| b == b' ').count();
    let text = &body[spaces..];
    if complete && body.iter().all(u8::is_ascii_whitespace) {
        return Line::Blank;
    }
    if spaces + usize::from(virtual_spaces) >= 4 {
        return Line::Indented {
            prefix: span.start + 4 - usize::from(virtual_spaces),
            virtual_spaces: 0,
        };
    }
    if text.is_empty() {
        return Line::Text;
    }
    // Every structural leaf uses the same physical columns. Consume exactly
    // four columns for code, including a partial terminal tab; subsequent tabs
    // are literal payload. At most four raw indentation bytes are inspected.
    let mut fence_start = 0;
    let mut fence_indent = virtual_spaces;
    let mut fence_column = column;
    for &byte in body {
        let width = match byte {
            b' ' => 1,
            b'\t' => 4 - fence_column,
            _ => break,
        };
        fence_indent += width;
        if fence_indent > 3 {
            return Line::Indented {
                prefix: span.start + fence_start + 1,
                virtual_spaces: fence_indent - 4,
            };
        }
        fence_column = (fence_column + width) % 4;
        fence_start += 1;
    }
    let fence_text = &body[fence_start..];
    // Independent math blocks require a complete, bounded delimiter line.
    // Their body stays literal; opening indentation is not stripped from it.
    if complete
        && fence_indent <= 3
        && fence_text.starts_with(b"$$")
        && fence_text[2..].iter().all(|b| matches!(b, b' ' | b'\t'))
    {
        return Line::FenceOpen {
            fence: Fence {
                byte: b'$',
                length: 2,
                indent: 0,
                remaining_indent: 0,
                indent_column: 0,
            },
            info: None,
        };
    }
    if fence_indent <= 3
        && fence_text.starts_with(b"[^")
        && let Some(close) = fence_text.iter().position(|&b| b == b']')
        && fence_text.get(close + 1) == Some(&b':')
        && crate::footnotes::valid_label(
            &source[span.start + fence_start + 2..span.start + fence_start + close],
        )
    {
        let mut start = span.start + fence_start + close + 2;
        if source
            .as_bytes()
            .get(start)
            .is_some_and(|b| matches!(b, b' ' | b'\t'))
            && start < end
        {
            start += 1;
        }
        return Line::FootnoteDefinition {
            label: Span::new(
                span.start + fence_start + 2,
                span.start + fence_start + close,
            ),
            content: Span::new(start, end),
        };
    }
    // A reference definition can start a block, but cannot interrupt prose in
    // this exact scope. Reuse the paragraph eligibility already needed for
    // Setext; rejected candidates flow through the ordinary inline parser.
    if complete
        && !paragraph
        && fence_indent <= 3
        && let Some(definition) = definition(source, span.start + fence_start, end)
    {
        return definition;
    }
    if complete
        && paragraph
        && fence_indent <= 3
        && let Some(level) = setext_level(fence_text)
    {
        return Line::Setext(level);
    }
    if complete && fence_indent <= 3 && matches!(fence_text.first(), Some(b'*' | b'-' | b'_')) {
        let mut count = 0;
        if fence_text.iter().all(|&b| {
            if b == fence_text[0] {
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
    if fence_indent <= 3 && matches!(fence_text.first(), Some(b'`' | b'~')) {
        let length = fence_text
            .iter()
            .take_while(|&&b| b == fence_text[0])
            .count();
        if length >= 3
            // A run cut by the window may continue in the next append. In
            // particular, a container prefix can leave fewer than MAX bytes.
            && (complete || length < fence_text.len())
            && (fence_text[0] != b'`' || !fence_text[length..].contains(&b'`'))
        {
            let rest = &source[span.start + fence_start + length..end];
            let info = rest.trim_ascii();
            let offset = rest.len() - rest.trim_ascii_start().len();
            let info_start = span.start + fence_start + length + offset;
            return Line::FenceOpen {
                fence: Fence {
                    byte: fence_text[0],
                    length,
                    indent: fence_indent,
                    remaining_indent: 0,
                    indent_column: 0,
                },
                info: (!info.is_empty()).then(|| Span::new(info_start, info_start + info.len())),
            };
        }
    }
    let text = fence_text;
    if fence_indent <= 3 && text.first() == Some(&b'#') {
        let level = text.iter().take_while(|&&b| b == b'#').count();
        if (1..=6).contains(&level)
            && ((level == text.len() && complete)
                || text.get(level).is_some_and(|b| matches!(b, b' ' | b'\t')))
        {
            let mut prefix = span.start + fence_start + level;
            while prefix < end && matches!(source.as_bytes()[prefix], b' ' | b'\t') {
                prefix += 1;
            }
            return Line::Heading {
                level: level as u8,
                prefix,
            };
        }
    }
    Line::Text
}

/// Single physical-line candidate. The raw source remains provisional until
/// LF/EOF; only endpoints and the current run length survive an append.
pub(crate) struct FenceProbe {
    fence: Fence,
    minimum: usize,
    closing: bool,
    run: bool,
    info: Option<Span>,
}

impl FenceProbe {
    pub fn new(
        source: &str,
        span: Span,
        active: Option<Fence>,
        mut indent: u8,
        mut column: u8,
        work: &mut crate::Work,
    ) -> Option<Self> {
        if active.is_some_and(|f| f.byte == b'$') {
            return None;
        }
        let mut start = span.start;
        for &byte in &source.as_bytes()[span.range()] {
            work.fence_candidate_bytes += 1;
            let width = match byte {
                b' ' => 1,
                b'\t' => 4 - column,
                _ => break,
            };
            indent += width;
            if indent > 3 {
                return None;
            }
            column = (column + width) % 4;
            start += 1;
        }
        let byte = *source.as_bytes().get(start).filter(|_| start < span.end)?;
        if !matches!(byte, b'`' | b'~') || active.is_some_and(|f| f.byte != byte) {
            return None;
        }
        let mut probe = Self {
            fence: Fence {
                byte,
                length: 0,
                indent,
                remaining_indent: 0,
                indent_column: 0,
            },
            minimum: active.map_or(3, |f| f.length),
            closing: active.is_some(),
            run: true,
            info: None,
        };
        if !probe.feed(source, Span::new(start, span.end), work) {
            return None;
        }
        Some(probe)
    }

    pub fn feed(&mut self, source: &str, span: Span, work: &mut crate::Work) -> bool {
        for at in span.range() {
            work.fence_candidate_bytes += 1;
            let byte = source.as_bytes()[at];
            if byte == b'\n' {
                continue;
            }
            if self.run && byte == self.fence.byte {
                self.fence.length += 1;
                continue;
            }
            self.run = false;
            if self.fence.length < self.minimum {
                return false;
            }
            if self.closing {
                if !byte.is_ascii_whitespace() {
                    return false;
                }
            } else {
                if self.fence.byte == b'`' && byte == b'`' {
                    return false;
                }
                if !byte.is_ascii_whitespace() {
                    self.info = Some(Span::new(self.info.map_or(at, |s| s.start), at + 1));
                }
            }
        }
        true
    }

    pub fn finish(self) -> Option<Line> {
        (self.fence.length >= self.minimum).then_some(if self.closing {
            Line::FenceClose
        } else {
            Line::FenceOpen {
                fence: self.fence,
                info: self.info,
            }
        })
    }
}

/// Match closers before stripping body indentation. `column` is the physical
/// column modulo four after the matched prefix (including a partly used tab).
pub(crate) fn classify_fenced(
    source: &str,
    span: Span,
    complete: bool,
    fence: Fence,
    virtual_spaces: u8,
    mut column: u8,
) -> Line {
    if !complete {
        return Line::Code;
    }
    let body = &source.as_bytes()[span.start..body_end(source, span)];
    let mut indent = virtual_spaces;
    let mut start = 0;
    for &byte in body {
        let width = match byte {
            b' ' => 1,
            b'\t' => 4 - column,
            _ => break,
        };
        indent += width;
        if indent > 3 {
            return Line::Code;
        }
        column = (column + width) % 4;
        start += 1;
    }
    let text = &body[start..];
    let run = text.iter().take_while(|&&b| b == fence.byte).count();
    if (if fence.byte == b'$' {
        run == 2
    } else {
        run >= fence.length
    }) && text[run..].iter().all(|b| {
        if fence.byte == b'$' {
            matches!(b, b' ' | b'\t')
        } else {
            b.is_ascii_whitespace()
        }
    }) {
        Line::FenceClose
    } else {
        Line::Code
    }
}

/// A possible ATX suffix is only spaces/tabs, or a whitespace-separated hash
/// run followed by spaces/tabs. Keep its endpoints, never a copy of the line.
/// `content.end` advances monotonically: rejected suffixes enter inline parsing
/// once, while a suffix confirmed by LF/EOF never enters it at all.
pub(crate) struct Atx {
    pub content: Span,
    hashes_end: Option<usize>,
    cr: Option<usize>,
}

impl Atx {
    pub fn new(start: usize) -> Self {
        Self {
            content: Span::new(start, start),
            hashes_end: None,
            cr: None,
        }
    }

    pub fn feed(&mut self, source: &str, span: Span, work: &mut crate::Work) {
        for pos in span.range() {
            let byte = source.as_bytes()[pos];
            work.atx_bytes += 1;
            if let Some(cr) = self.cr.take()
                && byte != b'\n'
            {
                self.byte(b'\r', cr);
            }
            match byte {
                b'\r' => self.cr = Some(pos),
                b'\n' => (),
                _ => self.byte(byte, pos),
            }
        }
    }

    pub fn finish(&mut self) {
        if let Some(cr) = self.cr.take() {
            self.byte(b'\r', cr);
        }
    }

    fn byte(&mut self, byte: u8, pos: usize) {
        match byte {
            b' ' | b'\t' => {
                if self.content.start == pos {
                    self.content = Span::new(pos + 1, pos + 1);
                }
            }
            b'#' if self.hashes_end.is_some()
                || self.content.end < pos
                || self.content.start == pos =>
            {
                if let Some(end) = self.hashes_end
                    && end < pos
                {
                    // In "a ### ###", the first run is content, and only the
                    // whitespace immediately before the second run is pending.
                    self.content.end = end;
                }
                self.hashes_end = Some(pos + 1);
            }
            _ => {
                self.content.end = pos + 1;
                self.hashes_end = None;
            }
        }
    }
}

// Called only on new bounded physical label bytes. A real line ending resets
// backslash parity; it is retained in the raw key, never in container prefixes.
pub(crate) fn unescaped_bracket(text: &[u8]) -> Option<usize> {
    let mut escaped = false;
    text.iter().position(|&byte| {
        let bracket = matches!(byte, b'[' | b']') && !escaped;
        escaped = byte == b'\\' && !escaped;
        bracket
    })
}

fn definition(source: &str, start: usize, end: usize) -> Option<Line> {
    let text = &source.as_bytes()[start..end];
    if text.first() != Some(&b'[') || text.get(1) == Some(&b'^') {
        return None;
    }
    // Find the first unescaped bracket after the opener in this bounded line. Keep
    // the raw label spelling for reference matching and source offsets.
    let Some(bracket) = unescaped_bracket(&text[1..]) else {
        return (text.len() - 1 <= crate::references::MAX_LABEL_BYTES).then_some(
            Line::DefinitionLabel {
                label: Span::new(start + 1, end),
            },
        );
    };
    let close = bracket + 1;
    if text[close] != b']' || close <= 1 || text.get(close + 1) != Some(&b':') {
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
    let label = Span::new(start + 1, start + close);
    if a == b {
        return (text[close + 2..].iter().all(|b| matches!(b, b' ' | b'\t'))
            && label.len() <= crate::references::MAX_LABEL_BYTES
            && source[label.range()].chars().any(|c| !c.is_whitespace()))
        .then_some(Line::DefinitionStart { label });
    }
    if let Some((destination, title)) = definition_target(source, Span::new(a, b)) {
        return Some(Line::Definition {
            label,
            destination,
            title,
        });
    }
    (label.len() <= crate::MAX_LABEL_BYTES
        && source[label.range()].chars().any(|c| !c.is_whitespace()))
    .then(|| definition_open_title(source, Span::new(a, end), false))
    .flatten()
    .map(|parser| Line::DefinitionTitle { label, parser })
}

/// A complete bounded physical target line; all offsets borrow original source.
/// Leading indentation on this continuation is horizontal whitespace, including
/// four or more columns. Independent title lines use definition_title instead.
pub(crate) fn definition_target(source: &str, span: Span) -> Option<(Span, Option<Span>)> {
    let mut a = span.start;
    let mut b = body_end(source, span);
    while a < b && matches!(source.as_bytes()[a], b' ' | b'\t') {
        a += 1;
    }
    while a < b && matches!(source.as_bytes()[b - 1], b' ' | b'\t') {
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
    Some((destination, title))
}

// An unclosed title parser, never a partially published destination. The
// caller decides whether the scope permits retaining its physical sources.
pub(crate) fn definition_open_title(
    source: &str,
    span: Span,
    standalone: bool,
) -> Option<crate::destination::Destination> {
    let end = body_end(source, span);
    let bytes = source.as_bytes();
    let start = (span.start..end).find(|&pos| !matches!(bytes[pos], b' ' | b'\t'))?;
    let mut parser = if standalone {
        if !matches!(bytes[start], b'\'' | b'"' | b'(') {
            return None;
        }
        crate::destination::Destination::title(start, bytes[start])
    } else {
        crate::destination::Destination::new(start)
    };
    for (pos, &byte) in bytes
        .iter()
        .enumerate()
        .take(end)
        .skip(start + usize::from(standalone))
    {
        if !matches!(
            parser.byte(byte, pos),
            crate::destination::Progress::Pending
        ) {
            return None;
        }
    }
    parser.open_title().then_some(parser)
}

/// Exactly one complete physical title line, retaining its original spelling.
pub(crate) fn definition_title(source: &str, span: Span) -> Option<Span> {
    let end = body_end(source, span);
    let bytes = source.as_bytes();
    let start = (span.start..end).find(|&pos| !matches!(bytes[pos], b' ' | b'\t'))?;
    if !matches!(bytes[start], b'\'' | b'"' | b'(') {
        return None;
    }
    let mut parser = crate::destination::Destination::title(start, bytes[start]);
    for (offset, &byte) in bytes[start + 1..end].iter().enumerate() {
        if !matches!(
            parser.byte(byte, start + 1 + offset),
            crate::destination::Progress::Pending
        ) {
            return None;
        }
    }
    match parser.finish_definition() {
        crate::destination::Progress::Complete { title, .. } => title,
        _ => None,
    }
}

/// Caller has already removed up to three indentation columns and the physical LF.
pub(crate) fn setext_level(text: &[u8]) -> Option<u8> {
    let trimmed = text.trim_ascii_end();
    (matches!(trimmed.first(), Some(b'=' | b'-')) && trimmed.iter().all(|b| *b == trimmed[0]))
        .then(|| if trimmed[0] == b'=' { 1 } else { 2 })
}
