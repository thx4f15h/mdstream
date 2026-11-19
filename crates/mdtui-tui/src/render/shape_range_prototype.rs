//! Isolated, single-range read-only experiment using the production SharedShape.
//! It neither enables the cold frame path nor supplies incremental rollback
//! state. Only the requested range is a valid query domain after completion;
//! overlapping/repeated adoption and future input are deliberately unsupported.

use super::*;
struct RangeSeed {
    owner: SharedShape,
    document: DocumentId,
    revision: u64,
    source: Span,
    raw_ready: bool,
    core: mdtui3::DisplayShapeRangeSeed,
    html_started: bool,
    html_after: Option<usize>,
    html_done: bool,
    html_pending: Option<Span>,
}
impl RangeSeed {
    fn new(shape: &SharedShape, document: &Document, source: Span) -> Self {
        Self {
            owner: shape.clone(),
            document: document.id(),
            revision: document.revision(),
            source,
            raw_ready: false,
            core: document.display_shape_range_seed(source).unwrap(),
            html_started: false,
            html_after: None,
            html_done: false,
            html_pending: None,
        }
    }
    fn advance(
        &mut self,
        shape: &SharedShape,
        document: &Document,
        b: &mut Budget,
    ) -> Result<bool, LayoutError> {
        if !shape.same_index(&self.owner) || document.id() != self.document {
            return Err(LayoutError::WrongDocument);
        }
        if document.revision() != self.revision {
            return Err(LayoutError::StaleContinuation);
        }
        shape.check_document(document)?;
        if shape.applied_revision() != self.revision {
            return Err(LayoutError::StaleContinuation);
        }
        if !self.raw_ready {
            if !paid(b) {
                return Ok(false);
            }
            let native = document.display_source();
            let mut index = shape.0.borrow_mut();
            index.source_scalars = native.len();
            index.end = native.bytes();
            index.positions[0] = index.end;
            index.work.source_indexed_bytes = index.end;
            index.html_scan = native.html_state();
            index.native_source = Some(native);
            index.capacity = index.source_scalars.max(LEAF).next_power_of_two();
            let root = index.root;
            index.nodes[root].hi = index.capacity;
            let count = index.source_scalars;
            index.nodes[root].raw = index.source_summary(0, count, false);
            index.nodes[root].code = index.source_summary(0, count, true);
            self.raw_ready = true;
        }
        while !self.html_done {
            if let Some(span) = self.html_pending {
                let Ok(qualified) = document.display_shape_html_qualified(span.start, b)? else {
                    return Ok(false);
                };
                if qualified {
                    let mut index = shape.0.borrow_mut();
                    index.html.insert(span.start, span);
                    index.fact(
                        Span::new(
                            span.start.max(self.source.start),
                            span.end.min(self.source.end),
                        ),
                        SemanticFact::Html(span.start),
                    )?;
                }
                self.html_pending = None;
                continue;
            }
            if !paid(b) {
                return Ok(false);
            }
            let native = document.display_source();
            let span = if !self.html_started {
                self.html_started = true;
                native.first_html_in(self.source)
            } else {
                native.next_html(self.html_after)
            };
            if let Some(span) = span.filter(|s| s.start < self.source.end) {
                self.html_after = Some(span.start);
                self.html_pending = Some(span);
            } else {
                self.html_done = true;
            }
        }
        loop {
            let before = self.core.work();
            let item = self.core.advance(document, b)?;
            let after = self.core.work();
            {
                let mut index = shape.0.borrow_mut();
                index.work.seed_steps += after.steps - before.steps;
                index.work.seed_annotation_nodes +=
                    after.annotation_nodes - before.annotation_nodes;
            }
            match item {
                Ok(Some(item)) => adopt_item(&mut shape.0.borrow_mut(), item)?,
                Ok(None) => return Ok(true),
                Err(_) => return Ok(false),
            }
        }
    }
}
fn micro() -> Budget {
    Budget::new(WorkLimit {
        queries: 1,
        objects: 1,
        bytes: 6,
        tiles: 1,
        recovery: 1,
        dependencies: 1,
    })
}
fn all() -> Budget {
    Budget::new(WorkLimit::unlimited())
}
fn count(shape: &SharedShape, document: &Document, source: Span) -> usize {
    let links = SharedLinks::new(document);
    links.set_shape(shape.clone()).unwrap();
    let mut q = ShapeQuery::graphemes(shape, source);
    q.advance(shape, document, &links, &mut all())
        .unwrap()
        .unwrap()
}
#[test]
fn semantic_window_uses_current_owner_and_native_ranges() {
    for tail in [
        "# hi\n",
        "**hello**\n",
        "[x](missing)\n",
        "a &amp; ` b `\n",
        "<span>text</span>\n",
        "👩‍💻 e\u{301}\n",
    ] {
        let mut d = Document::new();
        d.enable_display_shape_effects().unwrap();
        let prefix = "old **history**\n\n".repeat(2048);
        d.append(&prefix).unwrap();
        d.append(tail).unwrap();
        d.finish();
        let source = Span::new(prefix.len(), d.source().len());
        let shape = SharedShape::from_current(&d);
        let mut seed = RangeSeed::new(&shape, &d, source);
        assert!(
            !seed
                .advance(&shape, &d, &mut Budget::new(WorkLimit::default()))
                .unwrap()
        );
        assert_eq!(shape.source_len(), 0);
        let foreign = SharedShape::from_current(&d);
        assert_eq!(
            seed.advance(&foreign, &d, &mut micro()),
            Err(LayoutError::WrongDocument)
        );
        let mut done = false;
        let mut frames = 0;
        for _ in 0..4096 {
            frames += 1;
            if seed.advance(&shape, &d, &mut micro()).unwrap() {
                done = true;
                break;
            }
        }
        assert!(done, "{tail:?}");
        let oracle = SharedShape::from_current(&d);
        let mut full = ShapeSeed::indexed(&d);
        assert!(full.advance(&oracle, &d, &mut all()).unwrap());
        let actual = count(&shape, &d, source);
        let expected = count(&oracle, &d, source);
        eprintln!(
            "RANGE043 tail={tail:?} frames={frames} actual={actual} expected={expected} work={:?}",
            shape.work()
        );
        assert_eq!(actual, expected, "{tail:?}");
        assert_eq!(shape.work().source_decode_bytes, 0);
        assert!(shape.work().seed_annotation_nodes < 128);
    }
}
fn glyphs(
    shape: &SharedShape,
    document: &Document,
    source: Span,
) -> Vec<(String, Span, crate::screen::StyleRole)> {
    let links = SharedLinks::new(document);
    links.set_shape(shape.clone()).unwrap();
    let mut window = ShapeWindow::new(shape, source, source).unwrap();
    let mut result = Vec::new();
    loop {
        match window.advance(shape, document, &links, &mut all()).unwrap() {
            ShapeWindowStep::Glyph(g) => result.push((g.text.collect_text(), g.source, g.style)),
            ShapeWindowStep::Complete => return result,
            ShapeWindowStep::BudgetExhausted => panic!("unlimited"),
        }
    }
}
#[test]
fn interior_semantic_ranges_match_complete_owner() {
    for tail in [
        "**abcdefgh**\n",
        "[abcdefgh](https://e.test)\n",
        "![abcdefgh](image)\n",
        "`abcdefgh`\n",
        "<span title='abcdefgh'>text</span>\n",
        "`<span title='abcdefgh'>x</span>`\n",
        "```\n<span title='abcdefgh'>x</span>\n```\n",
        "abc[^abcdefgh]def\n\n[^abcdefgh]: note\n",
    ] {
        let mut d = Document::new();
        d.enable_display_shape_effects().unwrap();
        let prefix = "old **history**\n\n".repeat(128);
        d.append(&prefix).unwrap();
        d.append(tail).unwrap();
        d.finish();
        let start = prefix.len() + tail.find("abcdefgh").unwrap() + 2;
        let source = Span::new(start, start + 4);
        let oracle = SharedShape::from_current(&d);
        let mut full = ShapeSeed::indexed(&d);
        assert!(full.advance(&oracle, &d, &mut all()).unwrap());
        let shape = SharedShape::from_current(&d);
        let mut seed = RangeSeed::new(&shape, &d, source);
        let mut done = false;
        for _ in 0..1024 {
            if seed.advance(&shape, &d, &mut micro()).unwrap() {
                done = true;
                break;
            }
        }
        assert!(done, "{tail:?}");
        let actual = glyphs(&shape, &d, source);
        let expected = glyphs(&oracle, &d, source);
        eprintln!(
            "INTERIOR043 tail={tail:?} actual={actual:?} expected={expected:?} nodes={}",
            shape.work().seed_annotation_nodes
        );
        assert_eq!(actual, expected, "{tail:?}");
    }
}
#[test]
fn range_cursor_rejects_revision_change_before_native_reads() {
    let mut d = Document::new();
    d.enable_display_shape_effects().unwrap();
    d.append("candidate").unwrap();
    let shape = SharedShape::from_current(&d);
    let mut seed = RangeSeed::new(&shape, &d, Span::new(0, d.source().len()));
    d.append("\n===\n").unwrap();
    let before = shape.work();
    assert_eq!(
        seed.advance(&shape, &d, &mut micro()),
        Err(LayoutError::StaleContinuation)
    );
    assert_eq!(shape.work(), before);
}
fn adopt_item(
    index: &mut ShapeIndex,
    item: mdtui3::DisplayShapeSeedItem,
) -> Result<(), LayoutError> {
    match item {
        mdtui3::DisplayShapeSeedItem::AnnotationCheckpoint => {
            unreachable!("a read-only range supplies no rollback baseline")
        }
        mdtui3::DisplayShapeSeedItem::Mark {
            source,
            mark,
            baseline,
        } => {
            let lo = index.lower_bound(source.start);
            let hi = index.lower_bound(source.end).min(index.atom_count());
            let root = index.root;
            assert!(!baseline, "current range only");
            if mark != blank() {
                index.mark_in(root, lo, hi, mark);
            }
        }
        mdtui3::DisplayShapeSeedItem::Replacement {
            source,
            replacement,
        } => index.replace(source, replacement)?,
        mdtui3::DisplayShapeSeedItem::Candidate {
            source,
            conditional_scope,
        } => {
            index.work.scope_dependency_reads += 1;
            if let Some(scope) = conditional_scope {
                index.conditional_scopes.insert(scope.start, scope);
            }
            index.fact(source, SemanticFact::Candidate)?
        }
        mdtui3::DisplayShapeSeedItem::Link { id, info, active } => {
            index.put_object(id, info, active)?
        }
        mdtui3::DisplayShapeSeedItem::FootnoteReference { id, source } => {
            index.fact(source, SemanticFact::Footnote(Some(id)))?;
            index.footnotes.insert(id, source);
        }
    }
    Ok(())
}
