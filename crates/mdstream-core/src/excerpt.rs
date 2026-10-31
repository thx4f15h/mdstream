use crate::{
    Budget, DisplayCursor, DisplayRun, DisplayStep, Document, DocumentId, LayoutError, Span, Style,
};

/// Independent, lifetime limits for a display-text prefix. The frame's Budget
/// separately limits each call's work, including hidden syntax and lookahead.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExcerptLimits {
    /// Maximum source prefix, including hidden markup; rounded down to UTF-8.
    pub source_bytes: usize,
    /// Maximum emitted UTF-8 bytes. A scalar is never split.
    pub display_bytes: usize,
    /// Maximum LF-delimited lines, including the last line's LF if present.
    /// Soft wrapping, graphemes and terminal cell widths belong to the renderer.
    pub lines: usize,
}

/// Why the prefix ended. A limit does not assert more *visible* text exists:
/// the remainder is deliberately not read to find out. SourceEnd takes priority
/// when the requested range ends without clipping display text, even at an
/// exact limit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExcerptEnd {
    SourceEnd,
    SourceLimit,
    DisplayLimit,
    LineLimit,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ExcerptStep<'a> {
    /// Nonempty, non-HIDDEN text, retaining style, link and source mapping.
    Run(DisplayRun<'a>),
    Complete(ExcerptEnd),
    BudgetExhausted,
}

/// Revision-bound, bounded display prefix for summaries and code previews.
/// Does not alter folds or heights and does not apply a view's folding choices.
/// Use a fold candidate's hidden range to explicitly preview its body, or a
/// heading's content range to read its title. No full-text buffer is allocated.
pub struct DisplayExcerpt {
    document: DocumentId,
    revision: u64,
    cursor: DisplayCursor,
    source_end: usize,
    end: usize,
    remaining_bytes: usize,
    remaining_lines: usize,
    complete: Option<ExcerptEnd>,
}

impl DisplayExcerpt {
    /// O(1), without reading content. Invalid ranges/UTF-8 boundaries fail even
    /// with zero limits. Open ranges are snapshots; append/finish invalidates
    /// both this task and previously emitted runs, requiring explicit recreation.
    pub fn new(
        document: &Document,
        source: Span,
        limits: ExcerptLimits,
    ) -> Result<Self, LayoutError> {
        document.text(source).ok_or(LayoutError::InvalidRange)?;
        let mut end = source
            .start
            .saturating_add(limits.source_bytes)
            .min(source.end);
        while !document.source().is_char_boundary(end) {
            end -= 1;
        }
        Ok(Self {
            document: document.id(),
            revision: document.revision(),
            cursor: document
                .display_cursor(Span::new(source.start, end))
                .unwrap(),
            source_end: source.end,
            end,
            remaining_bytes: limits.display_bytes,
            remaining_lines: limits.lines,
            complete: None,
        })
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// End of the source run inspected so far, possibly past the emitted prefix
    /// when a line/display limit clips a run. Not a continuation for the tail.
    pub fn source_position(&self) -> usize {
        self.cursor.source_position()
    }

    /// Reuses DisplayCursor charges: one query/object and at most TILE_BYTES + 2
    /// bytes per inspected window. The extra two cover code CRLF lookaround;
    /// repeated lookahead is charged again. Clipping scans only that paid window
    /// (or a replacement of at most eight bytes). Six bytes plus one query/object
    /// suffice for progress. Hidden runs also pay and can exhaust the frame.
    ///
    /// Emitted text borrows the document. A clipped replacement keeps its full
    /// source mapping, including syntax beyond the source-prefix limit. There
    /// is no lookahead for more visible content and no automatic ellipsis.
    /// Wrong/stale documents fail before any state or budget changes, even once
    /// complete. Repeated completion consumes nothing.
    pub fn next<'a>(
        &mut self,
        document: &'a Document,
        budget: &mut Budget,
    ) -> Result<ExcerptStep<'a>, LayoutError> {
        if self.document != document.id() {
            return Err(LayoutError::WrongDocument);
        }
        if self.revision != document.revision() {
            return Err(LayoutError::StaleContinuation);
        }
        loop {
            if self.complete.is_none() {
                self.complete = if self.cursor.source_position() == self.end {
                    Some(if self.end == self.source_end {
                        ExcerptEnd::SourceEnd
                    } else {
                        ExcerptEnd::SourceLimit
                    })
                } else if self.remaining_lines == 0 {
                    Some(ExcerptEnd::LineLimit)
                } else if self.remaining_bytes == 0 {
                    Some(ExcerptEnd::DisplayLimit)
                } else {
                    None
                };
            }
            if let Some(end) = self.complete {
                return Ok(ExcerptStep::Complete(end));
            }
            let mut run = match self.cursor.next(document, budget)? {
                DisplayStep::Run(run) => run,
                DisplayStep::BudgetExhausted => return Ok(ExcerptStep::BudgetExhausted),
                DisplayStep::Complete => unreachable!("source end checked above"),
            };
            if run.style.contains(Style::HIDDEN) || run.text.is_empty() {
                continue;
            }
            let mut end = 0;
            for ch in run.text.chars() {
                if self.remaining_lines == 0 {
                    self.complete = Some(ExcerptEnd::LineLimit);
                    break;
                }
                if ch.len_utf8() > self.remaining_bytes {
                    self.complete = Some(ExcerptEnd::DisplayLimit);
                    break;
                }
                end += ch.len_utf8();
                self.remaining_bytes -= ch.len_utf8();
                if ch == '\n' {
                    self.remaining_lines -= 1;
                }
            }
            if end == 0 {
                continue;
            }
            run.text = &run.text[..end];
            if run.replacement.is_none() {
                run.source.end = run.source.start + end;
            }
            return Ok(ExcerptStep::Run(run));
        }
    }
}
