//! Paid consumption of the production paragraph placement state. The canonical
//! scanner publishes complete EGC/native atoms in fixed source quanta; no
//! paragraph text or terminal grid is buffered here.
use super::{
    display::{DisplayScanner, ScanStatus},
    flow::FlowCursor,
    objects::SharedLinks,
    text::GraphemeKind,
};
use mdtui3::{Budget, Document, LayoutError, Span, WorkLimit};

#[derive(Clone, Copy)]
struct PreviousAtom {
    flow: FlowCursor,
    started: bool,
    pending_cut: Option<usize>,
    admitted_end: usize,
    source: Span,
}

pub(crate) struct PrefixFlow {
    contexts: super::context::WindowContexts,
    geometry_ready: bool,
    list: Option<super::prefix_list::PrefixList>,
    native: bool,
    list_layout: bool,
    glyph_cut: Option<usize>,
    previous: Option<PreviousAtom>,
    source: Span,
    next: usize,
    end: usize,
    width: u16,
    rows: usize,
    flow: FlowCursor,
    scanner: Option<Box<DisplayScanner>>,
    index: usize,
    fragment: usize,
    last_source_end: usize,
    started: bool,
    pending_cut: Option<usize>,
    admitted_end: usize,
    dependency_end: usize,
    pub prepared_atoms: usize,
    pub quanta: usize,
    pub skipped_spaces: usize,
}
impl PrefixFlow {
    pub fn new(doc: &Document, source: Span, width: u16, rows: usize) -> Self {
        Self {
            contexts: super::context::WindowContexts::fragments(
                doc,
                &[Span::new(
                    source.start,
                    source.start.saturating_add(1).min(source.end),
                )],
            ),
            geometry_ready: false,
            list: None,
            native: false,
            list_layout: false,
            glyph_cut: None,
            previous: None,
            source,
            next: source.start,
            end: source.start,
            width,
            rows,
            flow: FlowCursor::new(usize::from(width), usize::from(width)),
            scanner: None,
            index: 0,
            fragment: 0,
            last_source_end: source.start,
            started: false,
            pending_cut: None,
            admitted_end: source.start,
            dependency_end: source.start,
            prepared_atoms: 0,
            quanta: 0,
            skipped_spaces: 0,
        }
    }
    pub fn can_extend(&self) -> bool {
        !self.list_layout
    }
    pub fn dependency_end(&self) -> usize {
        self.dependency_end.min(self.source.end)
    }
    pub fn extend_append(&mut self, source: Span) -> Result<(), LayoutError> {
        if source.start != self.source.start
            || source.end < self.source.end
            || self.scanner.is_some()
        {
            return Err(LayoutError::InvalidRange);
        }
        self.source = source;
        Ok(())
    }
    pub fn advance(
        &mut self,
        doc: &Document,
        links: &SharedLinks,
        budget: &mut Budget,
    ) -> Result<Option<usize>, super::first_frame::FirstFrameError> {
        if !self.geometry_ready {
            if !self.contexts.advance(doc, budget) {
                return Ok(None);
            }
            if !budget.charge(WorkLimit {
                queries: 1,
                objects: 1,
                ..Default::default()
            }) {
                return Ok(None);
            }
            let block = doc.block_at(self.source.start).and_then(|id| doc.block(id));
            let mut first = 0;
            let mut continued = 0;
            if let Some(block) = block {
                let container = block.container.and_then(|id| doc.container(id));
                let list = matches!(block.kind, mdtui3::BlockKind::ListItem { .. })
                    || container
                        .is_some_and(|c| matches!(c.kind, mdtui3::ContainerKind::Item { .. }));
                if list {
                    let mut marker = block.clone();
                    let continuation = !matches!(block.kind, mdtui3::BlockKind::ListItem { .. });
                    if let Some(c) = container
                        && let mdtui3::ContainerKind::Item { number, task } = c.kind
                    {
                        marker.kind = mdtui3::BlockKind::ListItem {
                            indent: 0,
                            number,
                            task,
                        };
                    }
                    let indent = block
                        .container
                        .and_then(|id| self.contexts.entries.get(&id))
                        .map_or(0, |c| c.indent);
                    let visual = super::list::build_list_visual_with_indent(&marker, indent)
                        .map_err(|e| super::sink::SinkError::Render(e.to_string()))?;
                    first = if continuation {
                        visual.continuation_indent
                    } else {
                        visual.body_indent
                    };
                    continued = visual.continuation_indent;
                    self.list_layout = true;
                    self.list = Some(super::prefix_list::PrefixList::new(
                        usize::from(self.width).saturating_sub(first),
                        usize::from(self.width).saturating_sub(continued),
                        self.rows,
                        self.source.start,
                    ));
                } else if matches!(block.kind, mdtui3::BlockKind::Quote { .. })
                    || container.is_some_and(|c| matches!(c.kind, mdtui3::ContainerKind::Quote))
                {
                    first = usize::from(self.width).min(2);
                    continued = first;
                }
            }
            self.flow = FlowCursor::new(
                usize::from(self.width).saturating_sub(first),
                usize::from(self.width).saturating_sub(continued),
            );
            self.geometry_ready = true;
        }
        loop {
            if let Some(list) = &mut self.list {
                if !list.advance(budget, &mut self.prepared_atoms) {
                    return Ok(None);
                }
                if let Some(cut) = list.cut() {
                    return Ok(Some(cut));
                }
            }
            if self.scanner.is_none() {
                if self.next >= self.source.end {
                    if let Some(list) = &mut self.list {
                        list.finish();
                        if !list.advance(budget, &mut self.prepared_atoms) {
                            return Ok(None);
                        }
                        if let Some(cut) = list.cut() {
                            return Ok(Some(cut));
                        }
                    }
                    return Ok(Some(self.source.end));
                }
                if !budget.charge(WorkLimit {
                    queries: 1,
                    objects: 1,
                    ..Default::default()
                }) {
                    return Ok(None);
                }
                self.end = self.next.saturating_add(256).min(self.source.end);
                while !doc.source().is_char_boundary(self.end) {
                    self.end -= 1;
                }
                let mut scanner =
                    DisplayScanner::exact(doc, Span::new(self.next, self.end), links.clone())
                        .ok_or(LayoutError::InvalidRange)?;
                scanner.force_indexed();
                scanner.set_math_rect(super::math::Rect {
                    x: 0,
                    y: 0,
                    width: usize::from(self.width),
                    height: self.rows.saturating_add(1),
                });
                self.scanner = Some(Box::new(scanner));
                self.index = 0;
                self.last_source_end = self.end;
                self.quanta += 1;
            }
            let scanner = self.scanner.as_mut().unwrap();
            if scanner.advance(doc, budget)? == ScanStatus::BudgetExhausted {
                return Ok(None);
            }
            let mut jump = None;
            while let Some(atom) = scanner.layout_segments().get(self.index) {
                if let Some(list) = &mut self.list {
                    if !list.advance(budget, &mut self.prepared_atoms) {
                        return Ok(None);
                    }
                    if let Some(cut) = list.cut() {
                        return Ok(Some(cut));
                    }
                    if self.native {
                        self.list = None;
                    }
                }
                if !budget.charge(WorkLimit {
                    queries: 1,
                    objects: 1,
                    ..Default::default()
                }) {
                    return Ok(None);
                }
                self.prepared_atoms += 1;
                self.last_source_end = self.last_source_end.max(atom.source.end);
                self.dependency_end = self.dependency_end.max(atom.source.end).max(self.end);
                let native = scanner
                    .math_at_source(atom.source.start)
                    .filter(|view| view.meta.source == atom.source);
                let mut first_char = atom.text.first_char().unwrap_or(' ');
                let mut list_whitespace = atom.text.is_whitespace();
                let (width, ascent, descent, whitespace) = if let Some(view) = &native {
                    self.index += 1;
                    (
                        view.metrics.width,
                        view.metrics.baseline,
                        view.metrics
                            .height
                            .saturating_sub(view.metrics.baseline + 1),
                        false,
                    )
                } else {
                    let (kind, width, whitespace) = if let Some(kind) = atom.text.cluster_kind() {
                        self.index += 1;
                        (kind, atom.text.width(), atom.text.is_ascii_whitespace())
                    } else {
                        // Existing finite generated placeholders (e.g. [image])
                        // retain one object source envelope, but real drawing
                        // still places their canonical display glyphs.
                        let text = atom.text.contiguous().ok_or(LayoutError::InvalidRange)?;
                        let glyph = super::text::graphemes(&text[self.fragment..])
                            .next()
                            .ok_or(LayoutError::InvalidRange)?;
                        self.fragment += glyph.text.len();
                        first_char = glyph.text.chars().next().unwrap_or(' ');
                        list_whitespace = glyph.text.chars().all(char::is_whitespace);
                        if self.fragment == text.len() {
                            self.fragment = 0;
                            self.index += 1;
                        }
                        (
                            glyph.kind,
                            usize::from(glyph.width),
                            glyph.text.chars().all(|ch| ch.is_ascii_whitespace()),
                        )
                    };
                    if kind == GraphemeKind::Control {
                        continue;
                    }
                    (width, 0, 0, whitespace)
                };
                if atom.source.start < self.next
                    && atom.source.end > self.next
                    && let Some(old) = self.previous.filter(|old| old.source.end == self.next)
                {
                    self.flow = old.flow;
                    self.started = old.started;
                    self.pending_cut = old.pending_cut;
                    self.admitted_end = old.admitted_end;
                }
                let previous = PreviousAtom {
                    flow: self.flow,
                    started: self.started,
                    pending_cut: self.pending_cut,
                    admitted_end: self.admitted_end,
                    source: atom.source,
                };
                self.previous = Some(previous);
                if let Some(list) = &mut self.list {
                    if native.is_some() {
                        // Finish the preceding plain word before switching to
                        // the same native glyph-flow policy as the real sink.
                        list.finish();
                        self.index -= 1;
                        self.native = true;
                        continue;
                    }
                    list.feed(super::prefix_list::WordAtom {
                        source: atom.source,
                        width,
                        whitespace: list_whitespace,
                        first: first_char,
                        preserved: atom.style.contains(crate::screen::StyleRole::IMAGE)
                            || atom.style.contains(crate::screen::StyleRole::LITERAL_LINK),
                    });
                }
                if whitespace && !self.started {
                    continue;
                }
                self.flow.push(width, ascent, descent, native.is_some())?;
                if whitespace {
                    if self.flow.height() > self.rows && self.pending_cut.is_none() {
                        self.pending_cut = Some(atom.source.start.min(self.admitted_end));
                    }
                } else {
                    if let Some(cut) = self.pending_cut {
                        if self.list.is_none() {
                            return Ok(Some(self.glyph_cut.unwrap_or(cut)));
                        }
                        self.glyph_cut.get_or_insert(cut);
                    }
                    if self.flow.height() > self.rows {
                        let cut = atom.source.start.min(self.admitted_end);
                        if self.list.is_none() {
                            return Ok(Some(self.glyph_cut.unwrap_or(cut)));
                        }
                        self.glyph_cut.get_or_insert(cut);
                    }
                    self.started = true;
                }
                self.admitted_end = self.admitted_end.max(atom.source.end);
                if whitespace
                    && (self.pending_cut.is_some() || !self.started)
                    && atom.source.len() == 1
                    && let Some((' ', run)) = doc.display_source().run_at_source(atom.source.start)
                {
                    let last = run.end.min(self.source.end).saturating_sub(1);
                    if last > self.end {
                        self.skipped_spaces += last - atom.source.end;
                        self.dependency_end = self.dependency_end.max(last + 1);
                        jump = Some(last);
                        break;
                    }
                }
            }
            self.next = jump.unwrap_or_else(|| {
                self.last_source_end.max(
                    scanner
                        .following_source()
                        .unwrap_or(self.last_source_end)
                        .min(self.source.end),
                )
            });
            self.scanner = None;
        }
    }
}
