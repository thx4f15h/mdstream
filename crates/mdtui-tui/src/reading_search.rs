//! Preserve native DisplaySearch bytes outside mathematical objects, replacing
//! only current visible math runs with their intrinsic row-major projection.
use crate::render::math::{MathFocus, MathTextCursor, MathTextStep};
use crate::render::{
    objects::SharedLinks,
    payload::{PayloadReader, PayloadStep},
};
use mdtui3::{
    Budget, Completion, DisplayCursor, DisplayMatch, DisplayStep, Document, DocumentId,
    LayoutError, MathObjectId, MathWindow, MathWindowStep, Span, Style, TILE_BYTES, WorkLimit,
};
use std::collections::VecDeque;
pub(crate) struct RenderedSearch {
    document: DocumentId,
    revision: u64,
    cursor: DisplayCursor,
    math_window: Option<MathWindow>,
    native_math: bool,
    math: Option<MathTextCursor>,
    math_row: usize,
    block_end: Option<Span>,
    reader: Option<PayloadReader>,
    needle: Vec<u8>,
    prefix: Vec<usize>,
    matched: usize,
    serial: usize,
    spans: VecDeque<(Span, Option<MathFocus>)>,
    source: Span,
    focus: Option<MathFocus>,
    hit_focus: Option<MathFocus>,
    pending_hit: Option<DisplayMatch>,
    pending: [u8; TILE_BYTES],
    pending_source: Span,
    atomic_source: bool,
    at: usize,
    end: usize,
}
impl RenderedSearch {
    pub fn new(d: &Document, query: &str, _width: u16) -> Self {
        let needle = query.as_bytes().to_vec();
        let mut prefix = vec![0; needle.len()];
        let mut k = 0;
        for i in 1..needle.len() {
            while k > 0 && needle[k] != needle[i] {
                k = prefix[k - 1];
            }
            if needle[k] == needle[i] {
                k += 1;
            }
            prefix[i] = k;
        }
        Self {
            document: d.id(),
            revision: d.revision(),
            cursor: d.display_cursor(Span::new(0, d.source().len())).unwrap(),
            math_window: None,
            native_math: false,
            math: None,
            math_row: 0,
            block_end: None,
            reader: None,
            needle,
            prefix,
            matched: 0,
            serial: 0,
            spans: VecDeque::new(),
            source: Span::new(0, 0),
            focus: None,
            hit_focus: None,
            pending_hit: None,
            pending: [0; TILE_BYTES],
            pending_source: Span::new(0, 0),
            atomic_source: false,
            at: 0,
            end: 0,
        }
    }
    fn byte(&mut self, byte: u8) -> Option<DisplayMatch> {
        self.serial += 1;
        if self.spans.len() == self.needle.len() {
            self.spans.pop_front();
        }
        self.spans.push_back((self.source, self.focus));
        while self.matched > 0 && self.needle[self.matched] != byte {
            self.matched = self.prefix[self.matched - 1];
        }
        if self.needle[self.matched] == byte {
            self.matched += 1;
        }
        if self.matched != self.needle.len() {
            return None;
        }
        self.matched = self.prefix[self.matched - 1];
        let start = self.spans.iter().map(|s| s.0.start).min().unwrap();
        let end = self.spans.iter().map(|s| s.0.end).max().unwrap();
        self.hit_focus = self.spans.front().and_then(|s| s.1).filter(|f| {
            self.spans
                .iter()
                .all(|s| s.1.is_some_and(|other| other.same_object(*f)))
        });
        Some(DisplayMatch {
            source: Span::new(start, end),
            display: self.serial - self.needle.len()..self.serial,
        })
    }
    pub(crate) fn hit_focus(&self) -> Option<MathFocus> {
        self.hit_focus
    }
    pub fn next(
        &mut self,
        d: &Document,
        links: &SharedLinks,
        b: &mut Budget,
    ) -> Result<(Completion, Option<DisplayMatch>), LayoutError> {
        if d.id() != self.document {
            return Err(LayoutError::WrongDocument);
        }
        if d.revision() != self.revision {
            return Err(LayoutError::StaleContinuation);
        }
        loop {
            if self.pending_hit.is_some() {
                if !b.charge(WorkLimit {
                    objects: 1,
                    ..Default::default()
                }) {
                    return Ok((Completion::BudgetExhausted, None));
                }
                return Ok((Completion::Covered, self.pending_hit.take()));
            }
            if self.native_math {
                if self.math_window.is_none() {
                    self.math_window = Some(d.math_window(self.pending_source)?);
                }
                match self.math_window.as_mut().unwrap().advance(d, b)? {
                    MathWindowStep::BudgetExhausted => {
                        return Ok((Completion::BudgetExhausted, None));
                    }
                    MathWindowStep::Object(meta)
                        if meta.content.start <= self.pending_source.start
                            && self.pending_source.start < meta.content.end =>
                    {
                        // The native run already proves canonical visibility. Skip
                        // raw TeX by its indexed object boundary, never by rereading
                        // Markdown or traversing its historical allocation arena.
                        self.math = Some(MathTextCursor::new(d, links.math(), meta)?);
                        self.math_row = 0;
                        self.cursor = d
                            .display_cursor(Span::new(meta.source.end, d.source().len()))
                            .ok_or(LayoutError::InvalidRange)?;
                        self.block_end =
                            matches!(meta.id, MathObjectId::Block(_)).then_some(meta.source);
                        self.at = 0;
                        self.end = 0;
                    }
                    _ => {} // e.g. a canonical whole-image replacement containing an empty object
                }
                self.native_math = false;
                self.math_window = None;
                continue;
            }
            if self.at < self.end {
                if !b.charge(WorkLimit {
                    bytes: 1,
                    ..Default::default()
                }) {
                    return Ok((Completion::BudgetExhausted, None));
                }
                self.source = if self.atomic_source {
                    self.pending_source
                } else {
                    Span::new(
                        self.pending_source.start + self.at,
                        self.pending_source.start + self.at + 1,
                    )
                };
                let byte = self.pending[self.at];
                self.at += 1;
                if let Some(hit) = self.byte(byte) {
                    self.pending_hit = Some(hit);
                }
                continue;
            }
            if let Some(reader) = &mut self.reader {
                match reader.next(b)? {
                    PayloadStep::Bytes { utf8, len } => {
                        self.pending[..len as usize].copy_from_slice(&utf8[..len as usize]);
                        self.at = 0;
                        self.end = len as usize;
                    }
                    PayloadStep::BudgetExhausted => return Ok((Completion::BudgetExhausted, None)),
                    PayloadStep::Complete => self.reader = None,
                }
                continue;
            }
            if let Some(math) = &mut self.math {
                self.atomic_source = true;
                match math.advance(d, b)? {
                    MathTextStep::BudgetExhausted => {
                        return Ok((Completion::BudgetExhausted, None));
                    }
                    MathTextStep::Complete => {
                        self.math = None;
                        if let Some(source) = self.block_end.take() {
                            self.pending_source = source;
                            self.focus = None;
                            self.pending[0] = b'\n';
                            self.at = 0;
                            self.end = 1;
                        }
                    }
                    MathTextStep::Cell { cell, focus } => {
                        self.pending_source = cell.source;
                        self.focus = Some(focus);
                        self.reader = Some(cell.text.reader());
                    }
                    MathTextStep::Space(source) => {
                        self.pending_source = source;
                        self.focus = None;
                        self.pending[0] = b' ';
                        self.at = 0;
                        self.end = 1;
                    }
                    MathTextStep::Newline(source) => {
                        self.math_row += 1;
                        if self.math_row >= math.metrics().unwrap().height {
                            continue;
                        }
                        self.pending_source = source;
                        self.focus = None;
                        self.pending[0] = b'\n';
                        self.at = 0;
                        self.end = 1;
                    }
                }
                continue;
            }
            let step =
                links.with_resolver(|resolver| self.cursor.next_with_resolver(d, resolver, b))?;
            match step {
                DisplayStep::BudgetExhausted => return Ok((Completion::BudgetExhausted, None)),
                DisplayStep::Complete => return Ok((Completion::EndOfDocument, None)),
                DisplayStep::Run(run) => {
                    if run.style.contains(Style::HIDDEN) || run.text.is_empty() {
                        continue;
                    }
                    self.pending[..run.text.len()].copy_from_slice(run.text.as_bytes());
                    self.at = 0;
                    self.end = run.text.len();
                    self.focus = None;
                    self.pending_source = run.replacement.unwrap_or(run.source);
                    self.atomic_source = run.replacement.is_some();
                    // Block math need not carry the inline MATH style bit.
                    // The current native source index is the object authority.
                    self.native_math = true;
                }
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn find(source: &str, query: &str) -> Option<(Document, DisplayMatch)> {
        let mut d = Document::new();
        d.append(source).unwrap();
        d.finish();
        let links = SharedLinks::new(&d);
        let mut search = RenderedSearch::new(&d, query, 20);
        let mut zero = Budget::new(WorkLimit::default());
        assert_eq!(
            search.next(&d, &links, &mut zero).unwrap(),
            (Completion::BudgetExhausted, None)
        );
        assert_eq!(zero.used(), WorkLimit::default());
        for _ in 0..source.len() * 300 + 10000 {
            let mut b = Budget::new(WorkLimit {
                queries: 1,
                objects: 1,
                bytes: 6,
                tiles: 1,
                recovery: 1,
                dependencies: 1,
            });
            let (status, hit) = search.next(&d, &links, &mut b).unwrap();
            assert!(b.used().queries <= 1 && b.used().objects <= 1 && b.used().bytes <= 6);
            if let Some(hit) = hit {
                return Some((d, hit));
            }
            if status == Completion::EndOfDocument {
                return None;
            }
        }
        panic!("rendered search stalled");
    }
    #[test]
    fn searches_rendered_math_and_plain_suffix_with_atomic_root_sources() {
        let source = "\\[\n\\frac{\\alpha{}́}{β}\n\\]\n\nplain tail\n";
        let (d, hit) = find(source, "ά").unwrap();
        assert_eq!(d.text(hit.source), Some("\\alpha{}́"));
        let (d, hit) = find(source, "β").unwrap();
        assert_eq!(d.text(hit.source), Some("β"));
        let (d, hit) = find(source, "tail").unwrap();
        assert_eq!(d.text(hit.source), Some("tail"));
        assert!(find(source, "frac").is_none());
    }
    #[test]
    fn intrinsic_math_search_finds_a_symbol_outside_the_current_horizontal_clip() {
        let source = format!("\\(\\frac{{\\alpha}}{{{}}}\\)\n", "x".repeat(80));
        let (d, hit) = find(&source, "α").unwrap();
        assert_eq!(d.text(hit.source), Some("\\alpha"));
    }
    #[test]
    fn searches_beyond_the_first_object_row_page() {
        let source = format!(
            "\\[\n\\begin{{matrix}}{}\\end{{matrix}}\n\\]\n",
            (0..40)
                .map(|i| format!("r{i}"))
                .collect::<Vec<_>>()
                .join("\\\\")
        );
        let (d, hit) = find(&source, "r35").unwrap();
        assert_eq!(d.text(hit.source), Some("r35"));
    }
    #[test]
    fn mathematical_search_deduplication_retires_completed_rows_under_the_shared_budget() {
        let mut d = Document::new();
        d.append(&"\\(x\\)\n\n".repeat(256)).unwrap();
        d.finish();
        let links = SharedLinks::new(&d);
        let mut search = RenderedSearch::new(&d, "NOT_PRESENT", 20);
        let mut peak = 0;
        let mut completed = false;
        for _ in 0..100000 {
            let mut b = Budget::new(WorkLimit {
                queries: 128,
                objects: 128,
                bytes: 768,
                tiles: 8,
                ..Default::default()
            });
            let (status, hit) = search.next(&d, &links, &mut b).unwrap();
            assert!(hit.is_none());
            assert!(b.used().queries <= 128 && b.used().objects <= 128 && b.used().bytes <= 768);
            peak = peak.max(
                usize::from(search.math.is_some()) + usize::from(search.math_window.is_some()),
            );
            if status == Completion::EndOfDocument {
                completed = true;
                break;
            }
        }
        assert!(completed);
        println!(
            "search object IDs: 256 expressions, peak={peak}, retained={}",
            usize::from(search.math.is_some()) + usize::from(search.math_window.is_some())
        );
        assert!(
            peak <= 32,
            "only overlapping output rows may retain deduplication IDs: {peak}"
        );
        assert!(
            search.math.is_none() && search.math_window.is_none(),
            "completed pages retire their IDs before cancellation"
        );
    }
    #[test]
    fn unrelated_math_preserves_native_plain_text_clipped_code_entities_and_empty_image_search() {
        for prefix in [
            format!("# {}HIDDEN_HIT\n\n", "x".repeat(80)),
            format!("word {}HIDDEN_HIT\n\n", "x".repeat(80)),
            format!("```\n{}HIDDEN_HIT\n```\n\n", "x".repeat(80)),
            "| a | HIDDEN_HIT |\n| - | - |\n| x | y |\n\n".to_owned(),
        ] {
            let expected = prefix.find("HIDDEN_HIT").unwrap();
            for suffix in ["", "\\(a\\)\n"] {
                let (d, hit) = find(&format!("{prefix}{suffix}"), "HIDDEN_HIT").unwrap();
                assert_eq!(hit.source, Span::new(expected, expected + 10));
                assert_eq!(d.text(hit.source), Some("HIDDEN_HIT"));
            }
        }
        let (d, hit) = find("A &amp; B\n\n\\(x\\)\n", "&").unwrap();
        assert_eq!(d.text(hit.source), Some("&amp;"));
        assert!(
            find("![](img)\n\n\\(x\\)\n", "[image]").is_none(),
            "terminal-only placeholder is absent from native logical search"
        );
        let (d, hit) = find("ab\\(x\\)cd\n", "bxc").unwrap();
        assert_eq!(d.text(hit.source), Some("b\\(x\\)c"));
    }
    #[test]
    fn native_math_interception_keeps_multibyte_run_boundaries_and_literal_code() {
        for source in ["界 \\(x\\) tail\n", "\\(界\\)\n", "😀 \\(界\\) end\n"] {
            let (d, hit) = find(source, "界").unwrap();
            assert_eq!(d.text(hit.source), Some("界"));
        }
        let source = "`\\(\\frac{a}{b}\\)` \\(\\alpha\\)\n";
        let (d, hit) = find(source, "frac").unwrap();
        assert_eq!(d.text(hit.source), Some("frac"));
        let (d, hit) = find(source, "α").unwrap();
        assert_eq!(d.text(hit.source), Some("\\alpha"));
    }
}
