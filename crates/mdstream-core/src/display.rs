use crate::{
    BlockId, Budget, Document, DocumentId, LayoutError, LinkId, QueryScratch, RenderRun, Runs,
    Span, Style, TILE_BYTES, WorkLimit,
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
    /// Confirmed pair counts, identical to the underlying semantic run even
    /// for entity/code/tab replacements. These do not encode nesting order.
    pub emphasis_depth: usize,
    pub strong_depth: usize,
    pub style_pair: Option<crate::StylePairId>,
    pub link: Option<LinkId>,
}

#[derive(Default)]
pub(crate) struct DisplayIndex {
    pub(crate) shape: crate::DisplayShapeEffects,
    entries: Vec<Entry>,
    // Source-ordered single-byte replacements, independent of the later code
    // ranges that may enclose them. No rewriting an open code span's body.
    spaces: Vec<(Span, u8)>,
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
    pub(crate) fn entries_len(&self) -> usize {
        self.entries.len()
    }
    pub(crate) fn truncate_entries(&mut self, len: usize) -> u64 {
        self.shape
            .emit(crate::DisplayShapeEffectKind::TruncateReplacements { entries: len });
        let removed = self.entries.len() - len;
        self.entries.truncate(len);
        removed as u64
    }

    fn push(&mut self, source: Span, kind: Kind) {
        if source.is_empty() {
            return;
        }
        debug_assert!(
            self.entries
                .last()
                .is_none_or(|e| e.source.end <= source.start)
        );
        let replacement = match &kind {
            Kind::Entity(text) => {
                let mut utf8 = [0; 8];
                utf8[..text.len()].copy_from_slice(text.as_bytes());
                crate::DisplayShapeReplacement::Entity {
                    utf8,
                    len: text.len() as u8,
                }
            }
            Kind::Code { leading, trailing } => crate::DisplayShapeReplacement::Code {
                leading: *leading,
                trailing: *trailing,
            },
        };
        self.shape.emit(crate::DisplayShapeEffectKind::Replacement {
            source,
            replacement,
        });
        self.entries.push(Entry { source, kind });
    }

    pub fn entity(&mut self, source: Span, text: String) {
        self.push(source, Kind::Entity(text));
    }

    /// A partially consumed container tab, possibly inside a future code span.
    pub fn spaces(&mut self, source: Span, columns: u8) {
        debug_assert!((1..=3).contains(&columns));
        debug_assert!(self.spaces.last().is_none_or(|s| s.0.end <= source.start));
        self.shape.emit(crate::DisplayShapeEffectKind::Replacement {
            source,
            replacement: crate::DisplayShapeReplacement::Spaces { columns },
        });
        self.spaces.push((source, columns));
    }

    pub fn space_mappings(&self) -> &[(Span, u8)] {
        &self.spaces
    }

    /// Only inspect the endpoints: the inline scanner maintains all-spaces as
    /// bytes arrive, so closing a huge span never rereads its body.
    pub fn code(
        &mut self,
        source: Span,
        text: &str,
        all_spaces: bool,
        last: Span,
        last_space: bool,
    ) {
        if source.is_empty() {
            return;
        }
        let bytes = text.as_bytes();
        // A failed outer candidate may publish this code after later virtual
        // spaces. Use its semantic tail summary, not the latest space mapping.
        let trim = !all_spaces && code_space(bytes[source.start]) && last_space;
        let leading_end = source.start
            + if trim {
                1 + usize::from(bytes[source.start..source.end].starts_with(b"\r\n"))
            } else {
                0
            };
        self.push(
            source,
            Kind::Code {
                leading: Span::new(source.start, leading_end),
                // Any gap after `last` is an already-hidden container prefix.
                trailing: Span::new(if trim { last.start } else { source.end }, source.end),
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
    space: usize,
}

/// Display iterator with the same source borrowing as `DisplayRuns`, returning
/// caller-owned traversal capacity on drop, including after early termination.
pub struct DisplayRunsWithScratch<'doc, 'scratch> {
    runs: DisplayRuns<'doc>,
    scratch: &'scratch mut QueryScratch,
}

impl DisplayRunsWithScratch<'_, '_> {
    pub fn annotation_visits(&self) -> u64 {
        self.runs.annotation_visits()
    }

    pub fn scratch_stats(&self) -> crate::ArenaStats {
        self.runs.scratch_stats()
    }
}

impl<'doc> Iterator for DisplayRunsWithScratch<'doc, '_> {
    type Item = DisplayRun<'doc>;

    fn next(&mut self) -> Option<Self::Item> {
        self.runs.next()
    }
}

impl Drop for DisplayRunsWithScratch<'_, '_> {
    fn drop(&mut self) {
        self.runs.runs.marks.recycle_into(self.scratch);
    }
}

impl Document {
    pub fn display_runs(&self, tile: crate::TileId) -> Option<DisplayRuns<'_>> {
        self.display_runs_in(self.tile(tile)?)
    }

    /// O(log n + requested source bytes). Entity text is emitted only by the
    /// fragment containing '&'; subsequent fragments are empty continuations.
    /// Code CRLF emits its space at LF (the parser already hides CR).
    pub fn display_runs_in(&self, source: Span) -> Option<DisplayRuns<'_>> {
        Some(self.display_with_runs(source, self.runs_in(source)?))
    }

    pub fn display_runs_with_scratch<'doc, 'scratch>(
        &'doc self,
        tile: crate::TileId,
        scratch: &'scratch mut QueryScratch,
    ) -> Option<DisplayRunsWithScratch<'doc, 'scratch>> {
        self.display_runs_in_with_scratch(self.tile(tile)?, scratch)
    }

    /// Same mapping and complexity as `display_runs_in`, retaining stack
    /// capacity in `scratch`. Invalid ranges leave the buffer unchanged.
    pub fn display_runs_in_with_scratch<'doc, 'scratch>(
        &'doc self,
        source: Span,
        scratch: &'scratch mut QueryScratch,
    ) -> Option<DisplayRunsWithScratch<'doc, 'scratch>> {
        self.text(source)?;
        let marks = self
            .annotations
            .segments_with_scratch(source, std::mem::take(scratch));
        Some(DisplayRunsWithScratch {
            runs: self.display_with_runs(source, self.runs_with_segments(source, marks)),
            scratch,
        })
    }

    fn display_with_runs<'a>(&'a self, source: Span, runs: Runs<'a>) -> DisplayRuns<'a> {
        DisplayRuns {
            document: self,
            runs,
            pending: None,
            entry: self
                .display
                .entries
                .partition_point(|e| e.source.end <= source.start),
            space: self
                .display
                .spaces
                .partition_point(|s| s.0.end <= source.start),
        }
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
            scratch: QueryScratch::default(),
            resolver: crate::LinkResolver::new(self),
            pending: None,
        })
    }
}

impl DisplayRuns<'_> {
    pub fn annotation_visits(&self) -> u64 {
        self.runs.annotation_visits()
    }

    /// Pending annotation frames, capacity and per-query allocation calls.
    pub fn scratch_stats(&self) -> crate::ArenaStats {
        self.runs.scratch_stats()
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
            .spaces
            .get(self.space)
            .is_some_and(|s| s.0.end <= start)
        {
            self.space += 1;
        }
        let mut virtual_columns = None;
        if let Some(&(source, columns)) = self.document.display.spaces.get(self.space) {
            if start < source.start {
                end = end.min(source.start);
            } else {
                end = end.min(source.end);
                virtual_columns = Some(columns);
            }
        }
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
                            if let Some(columns) = &mut virtual_columns {
                                // Code normalization trims one semantic space,
                                // not every column owned by the source tab.
                                *columns -= 1;
                            }
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
        if let Some(columns) = virtual_columns {
            replacement = Some(Span::new(start, start + 1));
            substitute = Some(&"   "[..usize::from(columns)]);
        }
        let output = DisplayRun {
            source: Span::new(start, end),
            replacement,
            text: substitute.unwrap_or(&self.document.source()[start..end]),
            block: run.block,
            style: run.style,
            emphasis_depth: run.emphasis_depth,
            strong_depth: run.strong_depth,
            style_pair: run.style_pair,
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
    scratch: QueryScratch,
    resolver: crate::LinkResolver,
    pending: Option<PreparedDisplay>,
}

#[derive(Clone, Copy)]
enum PreparedText {
    Source,
    Entity(usize),
    Static(&'static str),
}
struct PreparedDisplay {
    source: Span,
    replacement: Option<Span>,
    text: PreparedText,
    block: Option<BlockId>,
    style: Style,
    emphasis_depth: usize,
    strong_depth: usize,
    style_pair: Option<crate::StylePairId>,
    task: crate::link_semantics::MarkTask,
}
impl PreparedDisplay {
    fn new(document: &Document, run: DisplayRun<'_>, mark: crate::annotations::Mark) -> Self {
        let text = if run.replacement.is_none() {
            PreparedText::Source
        } else {
            match run.text {
                "" => PreparedText::Static(""),
                " " => PreparedText::Static(" "),
                "  " => PreparedText::Static("  "),
                "   " => PreparedText::Static("   "),
                _ => {
                    let at = document
                        .display
                        .entries
                        .partition_point(|entry| entry.source.end <= run.source.start);
                    assert!(matches!(document.display.entries[at].kind, Kind::Entity(_)));
                    PreparedText::Entity(at)
                }
            }
        };
        Self {
            source: run.source,
            replacement: run.replacement,
            text,
            block: run.block,
            style: run.style,
            emphasis_depth: run.emphasis_depth,
            strong_depth: run.strong_depth,
            style_pair: run.style_pair,
            task: crate::link_semantics::MarkTask::new(mark),
        }
    }
    fn emit<'a>(&mut self, document: &'a Document, end: usize) -> DisplayRun<'a> {
        let source = Span::new(self.source.start, end);
        let text = match self.text {
            PreparedText::Source => &document.source()[source.range()],
            PreparedText::Static(text) => text,
            PreparedText::Entity(at) => match &document.display.entries[at].kind {
                Kind::Entity(text) => text.as_str(),
                _ => unreachable!(),
            },
        };
        if matches!(self.text, PreparedText::Entity(_)) {
            self.text = PreparedText::Static("");
        }
        self.source.start = end;
        let (hidden, link) = self.task.result.unwrap();
        DisplayRun {
            source,
            replacement: self.replacement,
            text,
            block: self.block,
            style: if hidden {
                self.style.union(Style::HIDDEN)
            } else {
                self.style
            },
            emphasis_depth: self.emphasis_depth,
            strong_depth: self.strong_depth,
            style_pair: self.style_pair,
            link,
        }
    }
}

impl DisplayCursor {
    /// Retained traversal capacity and cumulative allocations across steps.
    /// No traversal or allocation occurs here; pending frames are always zero.
    pub fn scratch_stats(&self) -> crate::ArenaStats {
        self.scratch.stats()
    }

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
        self.resolver.check(document)?;
        let mut resolution = self.resolver.resolution.take();
        let result = self.next_in(document, &mut resolution, budget);
        self.resolver.resolution = resolution;
        result
    }
    /// Share conditional target/parent memoization with layout and hit queries.
    pub(crate) fn validate(&self, document: &Document) -> Result<(), LayoutError> {
        if self.document != document.id() {
            return Err(LayoutError::WrongDocument);
        }
        if self.revision != document.revision() {
            return Err(LayoutError::StaleContinuation);
        }
        self.resolver.check(document)
    }
    pub fn next_with_resolver<'a>(
        &mut self,
        document: &'a Document,
        resolver: &mut crate::LinkResolver,
        budget: &mut Budget,
    ) -> Result<DisplayStep<'a>, LayoutError> {
        self.validate(document)?;
        resolver.check(document)?;
        self.next_in(document, &mut resolver.resolution, budget)
    }
    pub fn take_link_resolver(&mut self) -> crate::LinkResolver {
        let empty = self.resolver.empty_like();
        std::mem::replace(&mut self.resolver, empty)
    }
    pub fn swap_link_resolver(
        &mut self,
        resolver: &mut crate::LinkResolver,
    ) -> Result<(), LayoutError> {
        resolver.check_binding(self.document, self.revision)?;
        std::mem::swap(&mut self.resolver, resolver);
        Ok(())
    }
    pub fn retire(&mut self, budget: &mut Budget) -> crate::Completion {
        self.resolver.retire(budget)
    }
    pub fn link_resolution_work(&self) -> crate::LinkResolutionWork {
        self.resolver.work()
    }
    fn next_in<'a>(
        &mut self,
        document: &'a Document,
        resolution: &mut crate::link_semantics::Resolution,
        budget: &mut Budget,
    ) -> Result<DisplayStep<'a>, LayoutError> {
        if self.document != document.id() {
            return Err(LayoutError::WrongDocument);
        }
        if self.revision != document.revision() {
            return Err(LayoutError::StaleContinuation);
        }
        loop {
            if self.position == self.end {
                return Ok(DisplayStep::Complete);
            }
            if let Some(pending) = document.indented_code_pending()
                && pending.source.start <= self.position
                && self.position < pending.source.end
            {
                if !budget.charge(WorkLimit {
                    queries: 1,
                    objects: 1,
                    ..Default::default()
                }) {
                    return Ok(DisplayStep::BudgetExhausted);
                }
                let source = Span::new(self.position, self.end.min(pending.source.end));
                self.position = source.end;
                self.pending = None;
                return Ok(DisplayStep::Run(DisplayRun {
                    source,
                    replacement: None,
                    text: "",
                    block: None,
                    style: Style::HIDDEN,
                    emphasis_depth: 0,
                    strong_depth: 0,
                    style_pair: None,
                    link: None,
                }));
            }
            if let Some(pending) = &mut self.pending {
                if !pending.task.advance(document, resolution, budget, 0) {
                    return Ok(DisplayStep::BudgetExhausted);
                }
                let window = budget.remaining().bytes.saturating_sub(2).min(TILE_BYTES);
                let mut end = self.position.saturating_add(window).min(pending.source.end);
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
                let run = pending.emit(document, end);
                self.position = end;
                if pending.source.is_empty() {
                    self.pending = None;
                }
                return Ok(DisplayStep::Run(run));
            }
            if !resolution.prepare_styles(document, self.position, budget)? {
                return Ok(DisplayStep::BudgetExhausted);
            }
            let style_snapshot = resolution
                .styles
                .snapshot(document, self.position, self.end);
            let window = budget.remaining().bytes.saturating_sub(2).min(TILE_BYTES);
            let mut charged_end = self.position.saturating_add(window).min(self.end);
            while !document.source().is_char_boundary(charged_end) {
                charged_end -= 1;
            }
            let mut end = if style_snapshot.authoritative {
                charged_end.min(style_snapshot.next)
            } else {
                charged_end
            };
            while !document.source().is_char_boundary(end) {
                end -= 1;
            }
            if end == self.position
                || !budget.charge(WorkLimit {
                    queries: 1,
                    objects: 1,
                    bytes: charged_end - self.position + 2,
                    ..Default::default()
                })
            {
                return Ok(DisplayStep::BudgetExhausted);
            }
            let (run, mark) = {
                let mut runs = document
                    .display_runs_in_with_scratch(Span::new(self.position, end), &mut self.scratch)
                    .unwrap();
                runs.runs.runs.raw = true;
                runs.runs.runs.style_override =
                    style_snapshot.authoritative.then_some(style_snapshot);
                let run = runs.next().unwrap();
                let mark = runs.runs.runs.last_mark;
                (run, mark)
            };
            if crate::link_semantics::Resolution::needs(document, mark) {
                self.pending = Some(PreparedDisplay::new(document, run, mark));
            } else {
                let mut run = run;
                run.style = run
                    .style
                    .union(mark.effective_style(&document.references, &document.links));
                self.position = run.source.end;
                return Ok(DisplayStep::Run(run));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{Document, Span, Style};

    #[test]
    fn atomic_suffix_recovery_preserves_entry_order_and_first_cell_effect_guards() {
        for source in [
            "&amp; [a](u '&amp;'&amp;) `x` &amp;",
            "> &amp; [a](u 'x\r\n>\t&amp;'&amp;)\n",
            "[a](u 'x\n&amp;' | b\n-|-\n",
            "[a](u 'x\n&amp;' | b\nwrong\n",
            "> [a](u 'x\n> &amp;' | b\n> -|-\n",
            "> [a](u 'x\n> &amp;' | b\n> wrong\n",
            "[old\nend](u 't') | &amp;\n-|-\n",
            "[old\nend](u '&amp;') | b\n-|-\n",
            "`old\nend` [a](u '&amp;'&amp;) | b\nwrong\n",
            "[a](u '&amp;'x) | &amp;\n-|-\n",
        ] {
            let mut d = Document::new();
            for ch in source.chars() {
                d.append(ch.encode_utf8(&mut [0; 4])).unwrap();
                assert!(
                    d.display
                        .entries
                        .windows(2)
                        .all(|w| w[0].source.end <= w[1].source.start)
                );
            }
            d.finish();
            let actual: Vec<_> = d
                .display
                .entries
                .iter()
                .filter_map(|e| matches!(e.kind, super::Kind::Entity(_)).then_some(e.source))
                .collect();
            let expected: Vec<_> = source
                .match_indices("&amp;")
                .map(|(at, _)| Span::new(at, at + 5))
                .collect();
            assert_eq!(actual, expected, "{source:?}");
            assert!(
                d.display
                    .entries
                    .windows(2)
                    .all(|w| w[0].source.end <= w[1].source.start)
            );
            // Independent CRLF/softbreak/virtual indices retain their own source.
            for (span, _) in d.display.space_mappings() {
                assert_eq!(d.text(*span), Some("\t"));
            }
        }
    }

    #[test]
    fn partial_tabs_store_one_record_per_line_and_code_closure_never_rewrites_them() {
        let mut doc = Document::new();
        doc.append("- ` a\n").unwrap();
        for _ in 0..4096 {
            doc.append("\tb\n").unwrap();
        }
        assert_eq!(doc.display.spaces.len(), 4096);
        assert!(doc.display.entries.is_empty());
        doc.append(&format!("\t{}", "x".repeat(1_000_000))).unwrap();
        let capacity = doc.display.spaces.capacity();
        let before = doc.work();
        for _ in 0..1000 {
            doc.append("x").unwrap();
        }
        assert_eq!(doc.display.spaces.len(), 4097);
        assert_eq!(doc.display.spaces.capacity(), capacity);
        assert_eq!(doc.work().container_steps, before.container_steps);
        assert_eq!(doc.work().inline_bytes - before.inline_bytes, 1000);
        let records = doc.display.spaces.clone();
        let before = doc.work();
        doc.append(" `\n").unwrap();
        assert_eq!(doc.display.entries.len(), 1);
        assert_eq!(doc.display.spaces, records);
        assert_eq!(doc.work().inline_bytes - before.inline_bytes, 3);
        assert!(doc.work().tree_visits - before.tree_visits < 2000);
        let tab = doc.source().find('\t').unwrap();
        let run = doc
            .display_runs_in(Span::new(tab, tab + 1))
            .unwrap()
            .next()
            .unwrap();
        assert_eq!(run.text, "  ");
        assert!(run.style.contains(Style::CODE));
    }
}
