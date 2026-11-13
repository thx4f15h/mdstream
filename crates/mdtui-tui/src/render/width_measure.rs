//! Resumable, non-buffering table-cell display measurement. The core cursor owns
//! the source position; only one paid run and the unfinished grapheme survive a
//! frame. Object resolution uses the frame's sole shared memo.
use mdtui3::{
    BlockId, Budget, DisplayCursor, DisplayRun, DisplayStep, Document, FootnoteReferenceId,
    FootnoteTarget, LayoutError, LinkId, Span, Style,
};

use super::input::{InputToken, image_tokens, style_role, token_kind};
use super::objects::{LinkLease, ObjectWindow, SharedLinks};
use super::shape_unicode::WidthStream;

pub(crate) struct WidthMeasure {
    cursor: DisplayCursor,
    pub source: Span,
    links: SharedLinks,
    lease: Option<LinkLease>,
    pending: Option<PendingRun>,
    width: WidthStream,
    image: Option<(LinkId, usize, bool)>,
    complete: bool,
    pub source_work: usize,
    memo_window: usize,
    indexed: Option<Box<super::shape::ShapeWindow>>,
    indexed_width: usize,
}

struct PendingRun {
    source: Span,
    replacement: Option<Span>,
    text: String,
    block: Option<BlockId>,
    link: Option<LinkId>,
    style: Style,
    style_pair: Option<mdtui3::StylePairId>,
    emphasis_depth: usize,
    strong_depth: usize,
    objects: ObjectWindow,
    footnote: Option<usize>,
    hidden: Vec<Span>,
}

impl WidthMeasure {
    pub fn new(document: &Document, source: Span, links: SharedLinks) -> Option<Self> {
        Some(Self {
            cursor: document.display_cursor(source)?,
            source,
            links,
            lease: None,
            pending: None,
            width: WidthStream::default(),
            image: None,
            complete: false,
            source_work: 0,
            memo_window: source.start,
            indexed: None,
            indexed_width: 0,
        })
    }

    /// The caller has proved the complete measured prefix unchanged. A pending
    /// paid run and its object recovery stay owned; no old prefix is replayed.
    pub fn rebind(&mut self, document: &Document) -> Option<()> {
        let position = self.cursor.source_position();
        self.cursor = document.display_cursor(Span::new(position, self.source.end))?;
        self.memo_window = position;
        Some(())
    }

    pub fn capacity_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.cursor.scratch_stats().capacity_bytes
            + self.pending.as_ref().map_or(0, |run| {
                run.text.capacity()
                    + run.hidden.capacity() * std::mem::size_of::<Span>()
                    + run.objects.scratch_capacity_bytes()
            })
    }

    pub fn advance(
        &mut self,
        document: &Document,
        budget: &mut Budget,
    ) -> Result<Option<usize>, LayoutError> {
        if self.source.len() > mdtui3::TILE_BYTES && !self.links.prepare_shape(document, budget)? {
            return Ok(None);
        }
        if self.source.len() > mdtui3::TILE_BYTES
            && let Some(shape) = self.links.shape()
            && shape.requires_shape(self.source)
        {
            let window = self.indexed.get_or_insert_with(|| {
                Box::new(
                    super::shape::ShapeWindow::new(&shape, self.source, self.source)
                        .expect("valid measured fragment"),
                )
            });
            loop {
                match window.advance(&shape, document, &self.links, budget)? {
                    super::shape::ShapeWindowStep::BudgetExhausted => return Ok(None),
                    super::shape::ShapeWindowStep::Glyph(glyph) => {
                        self.indexed_width += usize::from(glyph.width)
                    }
                    super::shape::ShapeWindowStep::Complete => {
                        self.complete = true;
                        return Ok(Some(self.indexed_width));
                    }
                }
            }
        }
        // This also rejects a stale reader after source has been fully read.
        if !self.links.prepare(document, budget)? {
            return Ok(None);
        }
        if self.complete {
            return Ok(Some(self.width.width()));
        }
        if self.lease.is_none() {
            self.lease = Some(self.links.hold());
        }
        loop {
            if self.pending.is_none() {
                if !self.links.prepare(document, budget)? {
                    return Ok(None);
                }
                if self.lease.is_none() {
                    self.lease = Some(self.links.hold());
                }
                let before = budget.used().bytes;
                let step = self.links.with_resolver(|resolver| {
                    self.cursor.next_with_resolver(document, resolver, budget)
                })?;
                self.source_work += budget.used().bytes.saturating_sub(before);
                match step {
                    DisplayStep::BudgetExhausted => return Ok(None),
                    DisplayStep::Complete => {
                        self.finish_image();
                        self.complete = true;
                        self.lease = None;
                        self.links.request_retirement();
                        return Ok(Some(self.width.width()));
                    }
                    DisplayStep::Run(run) => {
                        self.pending = Some(PendingRun {
                            source: run.source,
                            replacement: run.replacement,
                            text: run.text.to_owned(),
                            block: run.block,
                            link: run.link,
                            style: run.style,
                            style_pair: run.style_pair,
                            emphasis_depth: run.emphasis_depth,
                            strong_depth: run.strong_depth,
                            objects: ObjectWindow::new(document, run.source, self.links.clone())
                                .ok_or(LayoutError::InvalidRange)?,
                            footnote: None,
                            hidden: Vec::new(),
                        });
                    }
                }
            }
            let pending = self.pending.as_mut().unwrap();
            if !pending.objects.advance(document, budget)? {
                return Ok(None);
            }
            if pending.footnote.is_none() {
                let mut references = document
                    .footnote_references_in(pending.source)
                    .ok_or(LayoutError::InvalidRange)?;
                let Ok(first) = references.advance(budget) else {
                    return Ok(None);
                };
                pending.footnote = Some(first.map_or(usize::MAX, |(id, _)| id.0));
            }
            let refs = document.footnote_references();
            while let Some(reference) = refs
                .get(pending.footnote.unwrap())
                .filter(|r| r.source.start < pending.source.end)
            {
                let Ok(Some(resolution)) = document
                    .resolve_footnote(FootnoteReferenceId(pending.footnote.unwrap()), budget)
                else {
                    return Ok(None);
                };
                if matches!(resolution.target, FootnoteTarget::Defined(_)) {
                    pending.hidden.push(reference.source);
                }
                // Resolution paid for this occurrence and the fixed successor
                // read. Raw arena order can change after a header withdrawal.
                pending.footnote = Some(
                    document
                        .next_footnote_reference(FootnoteReferenceId(pending.footnote.unwrap()))
                        .map_or(usize::MAX, |id| id.0),
                );
            }
            let pending = self.pending.take().unwrap();
            if self
                .image
                .is_some_and(|(_, end, _)| end <= pending.source.start)
            {
                self.finish_image();
            }
            if let Some(&(id, source, _)) = pending.objects.outer_images.first()
                && self.image.is_none()
            {
                self.image = Some((id, source.end, false));
            }
            let run = DisplayRun {
                source: pending.source,
                replacement: pending.replacement,
                text: &pending.text,
                block: pending.block,
                link: pending.link,
                style: pending.style,
                style_pair: pending.style_pair,
                emphasis_depth: pending.emphasis_depth,
                strong_depth: pending.strong_depth,
            };
            let mut tokens: Vec<_> = image_tokens(&pending.objects, &run).collect();
            if !pending.style.contains(Style::HIDDEN) && !pending.text.is_empty() {
                tokens.push(InputToken {
                    source: pending.source,
                    replacement: pending.replacement,
                    text: pending.text.into(),
                    block: pending.block,
                    link: pending.link,
                    style: style_role(pending.style, pending.link.is_some()),
                    kind: token_kind(pending.style),
                });
            }
            let input = super::visibility::visible_tokens(pending.source, tokens, pending.hidden);
            for token in input.tokens() {
                if let Some((_, _, has_text)) = &mut self.image {
                    *has_text |= !token.text.is_empty();
                }
                self.push(token.text.contiguous().expect("paid cursor run"));
            }
            if self
                .cursor
                .source_position()
                .saturating_sub(self.memo_window)
                >= mdtui3::TILE_BYTES
                && self.links.stats().style_memo_entries == 0
            {
                // A conditional scope can cross every tile in this cell. Keep
                // its shared memo through the measurement, avoiding repeated
                // recovery of the same source events at each tile boundary.
                self.memo_window = self.cursor.source_position();
                self.lease = None;
                self.links.request_retirement();
            }
        }
    }

    fn finish_image(&mut self) {
        if self.image.take().is_some_and(|(_, _, has_text)| !has_text) {
            self.push("[image]");
        }
    }

    fn push(&mut self, text: &str) {
        self.width.push(text);
    }
}

#[cfg(test)]
mod ordered_scope_tests {
    use super::*;
    use mdtui3::WorkLimit;

    #[test]
    fn cross_tile_conditional_measurement_recovers_once_and_retires_with_shared_budget() {
        let n = 8 * mdtui3::TILE_BYTES + 17;
        let source = format!("**{} [x** z]", "a".repeat(n));
        let mut d = Document::new();
        d.append(&format!("{source}\n\n")).unwrap();
        let span = Span::new(0, source.len());
        let links = SharedLinks::new(&d);
        let micro = WorkLimit {
            queries: 1,
            objects: 1,
            bytes: 6,
            ..WorkLimit::default()
        };
        let mut measure = WidthMeasure::new(&d, span, links.clone()).unwrap();
        let before = links.stats();
        assert_eq!(
            measure
                .advance(&d, &mut Budget::new(WorkLimit::default()))
                .unwrap(),
            None
        );
        assert_eq!(links.stats(), before);
        let mut result = None;
        for _ in 0..400_000 {
            let mut b = Budget::new(micro);
            result = measure.advance(&d, &mut b).unwrap();
            assert!(b.used().queries <= 1 && b.used().objects <= 1 && b.used().bytes <= 6);
            if result.is_some() {
                break;
            }
        }
        assert_eq!(result, Some(n + 6));
        assert_eq!(
            links.stats().styles.events,
            4,
            "one scope, not one recovery per tile"
        );
        assert!(measure.source_work <= 4 * source.len());
        assert!(links.stats().style_memo_entries > 0);
        let before = links.stats();
        d.append("[x** z]: /u\n").unwrap();
        assert!(
            !links
                .prepare(&d, &mut Budget::new(WorkLimit::default()))
                .unwrap()
        );
        assert_eq!(links.stats(), before);
        let mut paid = 0;
        loop {
            let mut b = Budget::new(micro);
            let ready = links.prepare(&d, &mut b).unwrap();
            paid += b.used().objects;
            assert!(b.used().objects <= 1);
            if ready {
                break;
            }
        }
        assert_eq!(paid, before.memo_entries + before.presentation_entries);
        assert_eq!(links.stats().memo_entries, 0);
        let mut changed = WidthMeasure::new(&d, span, links.clone()).unwrap();
        assert_eq!(
            changed
                .advance(&d, &mut Budget::new(WorkLimit::unlimited()))
                .unwrap(),
            Some(n + 8)
        );
        println!(
            "cross-tile width: n={n} source_work={} events={} retired={paid}",
            measure.source_work, before.styles.events
        );
    }
}

#[cfg(test)]
mod withdrawn_footnote_tests {
    use super::*;
    use mdtui3::WorkLimit;
    #[test]
    fn confirmed_header_measurement_skips_withdrawn_footnote_occurrences() {
        let text = format!("end$ [^n] {}", "x".repeat(1100));
        let mut d = Document::new();
        d.append(&format!("$old\n{text}|b\n-|-\nx|y\n\n[^n]: note\n"))
            .unwrap();
        d.finish();
        assert_eq!(d.tables().len(), 1);
        assert!(!d.footnote_references().is_empty());
        assert!(d.footnote_reference_state(FootnoteReferenceId(0)).is_none());
        let span = d.tables()[0].cell(0, 0).unwrap().source;
        let links = SharedLinks::new(&d);
        let mut measure = WidthMeasure::new(&d, span, links).unwrap();
        assert_eq!(
            measure
                .advance(&d, &mut Budget::new(WorkLimit::unlimited()))
                .unwrap(),
            Some(text.len())
        );
    }
}

#[cfg(test)]
mod source_order_footnote_tests {
    use super::*;
    use mdtui3::WorkLimit;
    #[test]
    fn confirmed_header_footnotes_follow_source_order_after_old_code_replay() {
        let text = format!("[^early] ` tail [^later] {}", "x".repeat(1100));
        let mut d = Document::new();
        d.append(&format!(
            "`old\n{text}|b\n-|-\nx|y\n\n[^early]: E\n[^later]: L\n"
        ))
        .unwrap();
        d.finish();
        let span = d.tables()[0].cell(0, 0).unwrap().source;
        let mut refs = d.footnote_references_in(span).unwrap();
        let first = refs
            .advance(&mut Budget::new(WorkLimit::unlimited()))
            .unwrap()
            .unwrap();
        let second = refs
            .advance(&mut Budget::new(WorkLimit::unlimited()))
            .unwrap()
            .unwrap();
        assert!(
            first.0.0 > second.0.0,
            "allocation order differs from active source order"
        );
        assert!(first.1.source.start < second.1.source.start);
        assert_eq!(d.next_footnote_reference(first.0), Some(second.0));
        assert_eq!(d.next_footnote_reference(second.0), None);
        for micro in [false, true] {
            let links = SharedLinks::new(&d);
            let mut measure = WidthMeasure::new(&d, span, links).unwrap();
            assert_eq!(
                measure
                    .advance(&d, &mut Budget::new(WorkLimit::default()))
                    .unwrap(),
                None
            );
            let mut result = None;
            for _ in 0..50_000 {
                let limit = if micro {
                    WorkLimit {
                        queries: 1,
                        objects: 1,
                        bytes: 6,
                        dependencies: 1,
                        ..WorkLimit::default()
                    }
                } else {
                    WorkLimit::unlimited()
                };
                let mut b = Budget::new(limit);
                result = measure.advance(&d, &mut b).unwrap();
                if micro {
                    assert!(
                        b.used().queries <= 1
                            && b.used().objects <= 1
                            && b.used().bytes <= 6
                            && b.used().dependencies <= 1
                    );
                }
                if result.is_some() {
                    break;
                }
            }
            assert_eq!(
                result,
                Some(text.len() - "[^early]".len() - "[^later]".len())
            );
        }
    }
}

#[cfg(test)]
mod giant_cluster_tests {
    use super::*;
    use unicode_width::UnicodeWidthStr;

    #[test]
    fn complete_long_emoji_cluster_uses_locked_width_oracle() {
        let glyph = format!("👩{}‍💻", "\u{301}".repeat(10_000));
        assert_eq!(UnicodeWidthStr::width(glyph.as_str()).min(2), 2);
        let mut document = Document::new();
        document.append(&format!("{glyph}\n")).unwrap();
        document.finish();
        let links = SharedLinks::new(&document);
        let mut measure = WidthMeasure::new(&document, Span::new(0, glyph.len()), links).unwrap();
        assert_eq!(
            measure
                .advance(&document, &mut Budget::new(mdtui3::WorkLimit::unlimited()))
                .unwrap(),
            Some(2)
        );
    }
}

#[cfg(test)]
mod append_recovery_tests {
    use super::*;
    #[test]
    fn conditional_pending_recovery_keeps_paid_events_across_unrelated_appends() {
        let source = format!("**{} [x** z]", "a\u{301}".repeat(200));
        let mut d = Document::new();
        d.enable_display_shape_effects().unwrap();
        d.append(&format!("{source}\n\n")).unwrap();
        let links = SharedLinks::new(&d);
        assert!(
            links
                .prepare_shape(&d, &mut Budget::new(mdtui3::WorkLimit::unlimited()))
                .unwrap()
        );
        let shape = links.shape().unwrap();
        let before = shape.work();
        let mut measure = WidthMeasure::new(&d, Span::new(0, source.len()), links.clone()).unwrap();
        let mut result = None;
        for step in 0..400_000 {
            if step != 0 && step % 32 == 0 {
                let update = d.append("unrelated\n\n").unwrap();
                links.apply_shape_update(&d, &update).unwrap();
                measure.rebind(&d).unwrap();
            }
            let mut b = Budget::new(mdtui3::WorkLimit {
                queries: 1,
                objects: 1,
                bytes: 6,
                ..Default::default()
            });
            result = measure.advance(&d, &mut b).unwrap();
            if result.is_some() {
                break;
            }
        }
        eprintln!(
            "measured={} styles={:?} shape={:?}",
            measure.indexed_width,
            links.stats().styles,
            shape.work()
        );
        assert_eq!(result, Some(206));
        assert_eq!(links.stats().styles.events, 4);
        let after = shape.work();
        assert_eq!(
            after.source_decode_bytes - before.source_decode_bytes,
            d.source().len() - (source.len() + 2)
        );
    }
}
