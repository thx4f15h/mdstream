use crate::{
    BlockId, Budget, Document, DocumentId, LayoutError, LinkId, RenderRun, Runs, Span, Style,
    TILE_BYTES, WorkLimit,
};

/// A fragment of display text with its original source mapping. Hidden syntax
/// is returned too; omit HIDDEN runs when copying or displaying visible text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DisplayRun<'a> {
    /// Source consumed by this query, clipped to its requested range.
    pub source: Span,
    /// Full source of a replacement, even when a query/tile cuts through it.
    /// None means text borrows exactly `source` with a byte-for-byte mapping.
    pub replacement: Option<Span>,
    pub text: &'a str,
    pub block: Option<BlockId>,
    pub style: Style,
    pub link: Option<LinkId>,
}

#[derive(Default)]
pub(crate) struct DisplayIndex {
    entries: Vec<Entry>,
}

struct Entry {
    source: Span,
    kind: Kind,
}

enum Kind {
    Entity(String),
    Code { leading: Span, trailing: Span },
}

impl DisplayIndex {
    fn push(&mut self, source: Span, kind: Kind) {
        if source.is_empty() {
            return;
        }
        debug_assert!(
            self.entries
                .last()
                .is_none_or(|e| e.source.end <= source.start)
        );
        self.entries.push(Entry { source, kind });
    }

    pub fn entity(&mut self, source: Span, text: String) {
        self.push(source, Kind::Entity(text));
    }

    /// Only inspect the endpoints: the inline scanner maintains all-spaces as
    /// bytes arrive, so closing a huge span never rereads its body.
    pub fn code(&mut self, source: Span, text: &str, all_spaces: bool) {
        if source.is_empty() {
            return;
        }
        let bytes = text.as_bytes();
        let trim =
            !all_spaces && code_space(bytes[source.start]) && code_space(bytes[source.end - 1]);
        let leading_end = source.start
            + if trim {
                1 + usize::from(bytes[source.start..source.end].starts_with(b"\r\n"))
            } else {
                0
            };
        let trailing_start = source.end
            - if trim {
                1 + usize::from(bytes[source.start..source.end].ends_with(b"\r\n"))
            } else {
                0
            };
        self.push(
            source,
            Kind::Code {
                leading: Span::new(source.start, leading_end),
                trailing: Span::new(trailing_start, source.end),
            },
        );
    }
}

pub(crate) fn code_space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\r' | b'\n')
}

/// Lazy range reader. Ordinary text borrows the source; replacements borrow
/// small stored values or static whitespace. No paragraph-sized copy is made.
pub struct DisplayRuns<'a> {
    document: &'a Document,
    runs: Runs<'a>,
    pending: Option<RenderRun<'a>>,
    entry: usize,
}

impl Document {
    pub fn display_runs(&self, tile: crate::TileId) -> Option<DisplayRuns<'_>> {
        self.display_runs_in(self.tile(tile)?)
    }

    /// O(log n + requested source bytes). Entity text is emitted only by the
    /// fragment containing '&'; subsequent fragments are empty continuations.
    /// Code CRLF emits its space at LF (the parser already hides CR).
    pub fn display_runs_in(&self, source: Span) -> Option<DisplayRuns<'_>> {
        Some(DisplayRuns {
            document: self,
            runs: self.runs_in(source)?,
            pending: None,
            entry: self
                .display
                .entries
                .partition_point(|e| e.source.end <= source.start),
        })
    }

    /// A revision-bound reader for a finite requested range. Advance it using
    /// the frame's shared Budget; it never initiates an unsolicited EOF sweep.
    pub fn display_cursor(&self, source: Span) -> Option<DisplayCursor> {
        self.text(source)?;
        Some(DisplayCursor {
            document: self.id(),
            revision: self.revision(),
            position: source.start,
            end: source.end,
        })
    }
}

impl DisplayRuns<'_> {
    pub fn annotation_visits(&self) -> u64 {
        self.runs.annotation_visits()
    }
}

impl<'a> Iterator for DisplayRuns<'a> {
    type Item = DisplayRun<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        let mut run = self.pending.take().or_else(|| self.runs.next())?;
        let start = run.source.start;
        let mut end = run.source.end;
        let mut replacement = None;
        let mut substitute = None;
        while self
            .document
            .display
            .entries
            .get(self.entry)
            .is_some_and(|e| e.source.end <= start)
        {
            self.entry += 1;
        }
        if let Some(entry) = self.document.display.entries.get(self.entry) {
            if start < entry.source.start {
                end = end.min(entry.source.start);
            } else {
                end = end.min(entry.source.end);
                match &entry.kind {
                    Kind::Entity(text) => {
                        replacement = Some(entry.source);
                        substitute = Some(if start == entry.source.start {
                            text.as_str()
                        } else {
                            ""
                        });
                    }
                    Kind::Code { leading, trailing } => {
                        if start < leading.end {
                            end = end.min(leading.end);
                            replacement = Some(*leading);
                            substitute = Some("");
                        } else if start >= trailing.start {
                            replacement = Some(*trailing);
                            substitute = Some("");
                        } else {
                            end = end.min(trailing.start);
                            let bytes = self.document.source().as_bytes();
                            if matches!(bytes[start], b'\r' | b'\n') {
                                end = start + 1;
                                let crlf_before = bytes[start] == b'\n'
                                    && start > entry.source.start
                                    && bytes[start - 1] == b'\r';
                                let crlf_after = bytes[start] == b'\r'
                                    && start + 1 < entry.source.end
                                    && bytes[start + 1] == b'\n';
                                replacement = Some(Span::new(
                                    start - usize::from(crlf_before),
                                    end + usize::from(crlf_after),
                                ));
                                substitute = Some(if crlf_after { "" } else { " " });
                            } else if let Some(offset) = bytes[start..end]
                                .iter()
                                .position(|b| matches!(b, b'\r' | b'\n'))
                            {
                                end = start + offset;
                            }
                        }
                    }
                }
            }
        }
        let output = DisplayRun {
            source: Span::new(start, end),
            replacement,
            text: substitute.unwrap_or(&self.document.source()[start..end]),
            block: run.block,
            style: run.style,
            link: run.link,
        };
        if end < run.source.end {
            run.source.start = end;
            run.text = &self.document.source()[end..run.source.end];
            self.pending = Some(run);
        }
        Some(output)
    }
}

/// Result of one budgeted display read. Empty text can still consume source
/// (hidden syntax, stripped spaces, or a replacement continuation).
#[derive(Debug, PartialEq, Eq)]
pub enum DisplayStep<'a> {
    Run(DisplayRun<'a>),
    Complete,
    BudgetExhausted,
}

pub struct DisplayCursor {
    document: DocumentId,
    revision: u64,
    position: usize,
    end: usize,
}

impl DisplayCursor {
    pub fn source_position(&self) -> usize {
        self.position
    }

    /// Charges a whole inspection window (at most TILE_BYTES + 2), one query
    /// and one object before reading. Bytes conservatively include lookahead
    /// that may not be returned by the first run. Six byte units plus one query
    /// and object suffice for any UTF-8 scalar, including CRLF lookaround.
    /// Replacement output is at most eight bytes.
    pub fn next<'a>(
        &mut self,
        document: &'a Document,
        budget: &mut Budget,
    ) -> Result<DisplayStep<'a>, LayoutError> {
        if self.document != document.id() {
            return Err(LayoutError::WrongDocument);
        }
        if self.revision != document.revision() {
            return Err(LayoutError::StaleContinuation);
        }
        if self.position == self.end {
            return Ok(DisplayStep::Complete);
        }
        let remaining = budget.remaining();
        // Up to two adjacent bytes are inspected for code CRLF mapping.
        let window = remaining.bytes.saturating_sub(2).min(TILE_BYTES);
        let mut end = self.position.saturating_add(window).min(self.end);
        while !document.source().is_char_boundary(end) {
            end -= 1;
        }
        if end == self.position
            || !budget.charge(WorkLimit {
                queries: 1,
                objects: 1,
                bytes: end - self.position + 2,
                ..Default::default()
            })
        {
            return Ok(DisplayStep::BudgetExhausted);
        }
        let run = document
            .display_runs_in(Span::new(self.position, end))
            .unwrap()
            .next()
            .unwrap();
        self.position = run.source.end;
        Ok(DisplayStep::Run(run))
    }
}
