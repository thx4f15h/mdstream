use mdtui3::{
    ArenaStats, BlockId, Budget, DisplayCursor, DisplayRun, DisplayStep, Document, DocumentId,
    LayoutError, LinkId, Span, Style, TileId,
};

use crate::render::edges::{EdgeProbe, MAX_EDGE_OVERSCAN};
use crate::render::input::{TokenKind, image_tokens, style_role, token_kind};
use crate::screen::StyleRole;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplaySegment {
    pub tile: TileId,
    pub source: Span,
    pub replacement: Option<Span>,
    pub text: String,
    pub block: Option<BlockId>,
    pub link: Option<LinkId>,
    pub style: StyleRole,
    pub kind: TokenKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanStatus {
    BudgetExhausted,
    Complete,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DisplayScanStats {
    pub segments: usize,
    pub display_bytes: usize,
    pub complete_tiles: usize,
    pub scanned_bytes: usize,
    pub cursor_steps: u64,
    pub budget_exhausted: u64,
    pub cursor_scratch: ArenaStats,
    pub edge_source_bytes: usize,
    pub edge_queries: usize,
    pub omitted_edge_clusters: usize,
    pub display_source_work: usize,
    pub context_limited_edges: usize,
}

struct Selection {
    ranges: Vec<(Span, bool)>,
    cuts: Vec<usize>,
    rows: std::collections::HashMap<mdtui3::TableId, std::ops::Range<usize>>,
    next: usize,
    child: Option<Box<DisplayScanner>>,
    source_work: usize,
    skipped: Vec<(TileId, usize, u64)>,
    index_work: mdtui3::LinkWindowWork,
}

pub struct DisplayScanner {
    selection: Option<Selection>,
    table_mode: bool,
    footnote_index: Option<usize>,
    hidden_footnotes: Vec<Span>,
    links: crate::render::objects::SharedLinks,
    lease: Option<crate::render::objects::LinkLease>,
    objects: Option<crate::render::objects::ObjectWindow>,
    document: DocumentId,
    revision: u64,
    requested: Span,
    cursor: DisplayCursor,
    start_probe: Option<EdgeProbe>,
    end_probe: Option<EdgeProbe>,
    context_end: usize,
    resolved_start: Option<usize>,
    resolved_end: Option<usize>,
    edge_source_bytes: usize,
    edge_queries: usize,
    omit_start: bool,
    omit_end: bool,
    limited_start: bool,
    limited_end: bool,
    segments: Vec<DisplaySegment>,
    layout_segments: Vec<DisplaySegment>,
    display_source_work: usize,
    complete_tiles: Vec<TileId>,
    tile_range: std::ops::Range<usize>,
    next_tile: usize,
    scanned_through: usize,
    display_bytes: usize,
    cursor_steps: u64,
    budget_exhausted: u64,
    complete: bool,
    source_complete: bool,
    cursor_initialized: bool,
}

impl DisplayScanner {
    pub fn new(document: &Document, requested: Span) -> Option<Self> {
        let cursor = document.display_cursor(requested)?;
        let tile_range = document.tiles_overlapping(requested);
        let mut context_start = requested.start.saturating_sub(MAX_EDGE_OVERSCAN);
        while !document.source().is_char_boundary(context_start) {
            context_start += 1;
        }
        let mut context_end = requested
            .end
            .saturating_add(MAX_EDGE_OVERSCAN)
            .min(document.source().len());
        while !document.source().is_char_boundary(context_end) {
            context_end -= 1;
        }
        if requested.is_empty() {
            context_start = requested.start;
            context_end = requested.end;
        }
        Some(Self {
            selection: None,
            table_mode: false,
            footnote_index: None,
            hidden_footnotes: Vec::new(),
            links: crate::render::objects::SharedLinks::new(document),
            lease: None,
            objects: None,
            document: document.id(),
            revision: document.revision(),
            requested,
            cursor,
            start_probe: (!requested.is_empty() && context_start > 0)
                .then(|| EdgeProbe::new(context_start, document.source().len(), true)),
            end_probe: None,
            context_end,
            resolved_start: (context_start == 0).then_some(0),
            resolved_end: (context_end == document.source().len()).then_some(context_end),
            edge_source_bytes: 0,
            edge_queries: 0,
            omit_start: false,
            omit_end: false,
            limited_start: false,
            limited_end: false,
            segments: Vec::new(),
            layout_segments: Vec::new(),
            display_source_work: 0,
            complete_tiles: Vec::new(),
            next_tile: tile_range.start,
            tile_range,
            scanned_through: requested.start,
            display_bytes: 0,
            cursor_steps: 0,
            budget_exhausted: 0,
            complete: false,
            source_complete: false,
            cursor_initialized: requested.is_empty(),
        })
    }

    pub(crate) fn exact(
        document: &Document,
        source: Span,
        links: crate::render::objects::SharedLinks,
    ) -> Option<Self> {
        let mut scanner = Self::new(document, source)?;
        scanner.links = links;
        scanner.start_probe = None;
        scanner.resolved_start = Some(source.start);
        scanner.resolved_end = Some(source.end);
        scanner.context_end = source.end;
        scanner.cursor_initialized = true;
        Some(scanner)
    }

    pub(crate) fn select(&mut self, window: super::table_window::TableWindow) {
        assert!(!self.complete && self.segments.is_empty());
        self.selection = Some(Selection {
            ranges: window.ranges,
            cuts: window.cuts,
            rows: window.rows,
            next: 0,
            child: None,
            source_work: 0,
            skipped: window.skipped,
            index_work: mdtui3::LinkWindowWork::default(),
        });
    }

    pub(crate) fn selected_rows(&self, id: mdtui3::TableId) -> Option<std::ops::Range<usize>> {
        self.selection.as_ref()?.rows.get(&id).cloned()
    }

    pub(crate) fn cell_cut(&self, at: usize) -> bool {
        self.selection
            .as_ref()
            .is_some_and(|s| s.cuts.binary_search(&at).is_ok())
    }

    pub(crate) fn skipped_rows(&self) -> &[(TileId, usize, u64)] {
        self.selection.as_ref().map_or(&[], |s| &s.skipped)
    }

    fn advance_selected(
        &mut self,
        document: &Document,
        budget: &mut Budget,
    ) -> Result<ScanStatus, LayoutError> {
        let selection = self.selection.as_mut().unwrap();
        loop {
            if selection.child.is_none() {
                let Some(&(source, table)) = selection.ranges.get(selection.next) else {
                    break;
                };
                let mut child = if table {
                    Self::exact(document, source, self.links.clone())
                } else {
                    Self::new(document, source)
                }
                .ok_or(LayoutError::InvalidRange)?;
                child.links = self.links.clone();
                child.table_mode = table;
                selection.child = Some(Box::new(child));
            }
            let child = selection.child.as_mut().unwrap();
            let result = child.advance(document, budget)?;
            if result != ScanStatus::Complete {
                return Ok(result);
            }
            let mut child = selection.child.take().unwrap();
            selection.source_work += child.stats().scanned_bytes;
            let work = child.object_index_work();
            selection.index_work.index_visits += work.index_visits;
            selection.index_work.sort_steps += work.sort_steps;
            selection.index_work.objects += work.objects;
            self.display_source_work += child.display_source_work;
            self.edge_source_bytes += child.stats().edge_source_bytes;
            self.edge_queries += child.stats().edge_queries;
            self.cursor_steps += child.cursor_steps;
            self.budget_exhausted += child.budget_exhausted;
            self.display_bytes += child.display_bytes;
            self.layout_segments.extend(child.segments.iter().cloned());
            self.segments.append(&mut child.segments);
            selection.next += 1;
        }
        self.scanned_through = self.requested.end;
        self.refresh_complete_tiles(document)?;
        self.complete = true;
        Ok(ScanStatus::Complete)
    }

    pub fn object_index_work(&self) -> mdtui3::LinkWindowWork {
        if let Some(selection) = &self.selection {
            let mut work = selection.index_work;
            if let Some(child) = &selection.child {
                let pending = child.object_index_work();
                work.index_visits += pending.index_visits;
                work.sort_steps += pending.sort_steps;
                work.objects += pending.objects;
            }
            return work;
        }
        self.objects
            .as_ref()
            .map_or(mdtui3::LinkWindowWork::default(), |objects| {
                objects.index_work()
            })
    }

    pub fn shared_links(&self) -> crate::render::objects::SharedLinks {
        self.links.clone()
    }

    pub fn set_links(&mut self, links: crate::render::objects::SharedLinks) {
        assert!(
            self.objects.is_none(),
            "install shared links before advancing"
        );
        self.links = links;
    }

    #[must_use]
    pub fn matches(&self, document: &Document) -> bool {
        self.document == document.id() && self.revision == document.revision()
    }

    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    #[must_use]
    pub const fn requested(&self) -> Span {
        self.requested
    }

    #[must_use]
    pub const fn scanned_through(&self) -> usize {
        self.scanned_through
    }

    #[must_use]
    pub fn complete_tiles(&self) -> &[TileId] {
        &self.complete_tiles
    }

    #[must_use]
    pub fn segments(&self) -> &[DisplaySegment] {
        &self.segments
    }

    pub fn segments_for_tile(&self, tile: TileId) -> impl Iterator<Item = &DisplaySegment> + '_ {
        self.segments
            .iter()
            .filter(move |segment| segment.tile == tile)
    }

    #[must_use]
    pub const fn is_complete(&self) -> bool {
        self.complete
    }

    #[must_use]
    pub fn stats(&self) -> DisplayScanStats {
        DisplayScanStats {
            segments: self.segments.len(),
            display_bytes: self.display_bytes,
            complete_tiles: self.complete_tiles.len(),
            scanned_bytes: self.selection.as_ref().map_or_else(
                || {
                    self.scanned_through
                        .min(self.requested.end)
                        .saturating_sub(self.requested.start)
                },
                |s| s.source_work + s.child.as_ref().map_or(0, |c| c.stats().scanned_bytes),
            ),
            cursor_steps: self.cursor_steps,
            budget_exhausted: self.budget_exhausted,
            cursor_scratch: self.cursor.scratch_stats(),
            edge_source_bytes: self.edge_source_bytes
                + self.start_probe.as_ref().map_or(0, |probe| probe.bytes)
                + self.end_probe.as_ref().map_or(0, |probe| probe.bytes),
            edge_queries: self.edge_queries
                + self.start_probe.as_ref().map_or(0, |probe| probe.queries)
                + self.end_probe.as_ref().map_or(0, |probe| probe.queries),
            omitted_edge_clusters: usize::from(self.omit_start) + usize::from(self.omit_end),
            display_source_work: self.display_source_work,
            context_limited_edges: usize::from(self.limited_start) + usize::from(self.limited_end),
        }
    }

    pub(crate) fn omitted_edges(&self) -> (bool, bool) {
        (self.omit_start, self.omit_end)
    }

    pub(crate) fn limited_context(&self) -> (bool, bool) {
        (self.limited_start, self.limited_end)
    }

    pub(crate) fn layout_segments(&self) -> &[DisplaySegment] {
        &self.layout_segments
    }

    pub(crate) fn layout_source(&self) -> Span {
        Span::new(
            self.resolved_start.unwrap_or(self.requested.start),
            self.resolved_end.unwrap_or(self.requested.end),
        )
    }

    pub fn advance(
        &mut self,
        document: &Document,
        budget: &mut Budget,
    ) -> Result<ScanStatus, LayoutError> {
        if self.document != document.id() {
            return Err(LayoutError::WrongDocument);
        }
        if self.revision != document.revision() {
            return Err(LayoutError::StaleContinuation);
        }
        if self.complete {
            return Ok(ScanStatus::Complete);
        }
        if self.selection.is_some() {
            return self.advance_selected(document, budget);
        }

        if !self.links.prepare(document, budget)? {
            return Ok(ScanStatus::BudgetExhausted);
        }
        if self.lease.is_none() {
            self.lease = Some(self.links.hold());
        }
        if !self.cursor_initialized {
            if let Some(probe) = &mut self.start_probe {
                let Some((start, omitted)) = probe.advance(document.source(), budget) else {
                    return Ok(ScanStatus::BudgetExhausted);
                };
                self.resolved_start = Some(start);
                self.omit_start = omitted;
                self.limited_start = probe.context_limited;
                self.edge_source_bytes += probe.bytes;
                self.edge_queries += probe.queries;
                self.start_probe = None;
            }
            let start = self.resolved_start.unwrap_or(self.requested.start);
            if self.resolved_end.is_none() && self.end_probe.is_none() {
                // The resolved opening is a boundary. Rebase the end probe to
                // it so RI parity can be recovered inside this bounded window
                // without walking the earlier document prefix a second time.
                self.end_probe = Some(
                    EdgeProbe::new(
                        self.context_end - start,
                        document.source().len() - start,
                        false,
                    )
                    .with_context_limit(self.context_end - start),
                );
            }
            if let Some(probe) = &mut self.end_probe {
                let Some((end, omitted)) = probe.advance(&document.source()[start..], budget)
                else {
                    return Ok(ScanStatus::BudgetExhausted);
                };
                self.resolved_end = Some(start + end);
                self.omit_end = omitted;
                self.limited_end = probe.context_limited;
                self.edge_source_bytes += probe.bytes;
                self.edge_queries += probe.queries;
                self.end_probe = None;
            }
            let source = Span::new(
                self.resolved_start.unwrap_or(self.requested.start),
                self.resolved_end.unwrap_or(self.requested.end),
            );
            self.cursor = document
                .display_cursor(source)
                .ok_or(LayoutError::InvalidRange)?;
            self.scanned_through = source.start;
            self.cursor_initialized = true;
        }
        if self.objects.is_none() {
            self.objects = Some(
                crate::render::objects::ObjectWindow::new(
                    document,
                    self.layout_source(),
                    self.links.clone(),
                )
                .ok_or(LayoutError::InvalidRange)?,
            );
        }
        if !self
            .objects
            .as_mut()
            .expect("object window")
            .advance(document, budget)?
        {
            return Ok(ScanStatus::BudgetExhausted);
        }
        if self.source_complete {
            return self.finish_empty(document, budget);
        }
        loop {
            let before = budget.used().bytes;
            let step = self.links.with_resolver(|resolver| {
                self.cursor.next_with_resolver(document, resolver, budget)
            })?;
            self.display_source_work += budget.used().bytes.saturating_sub(before);
            match step {
                DisplayStep::Run(run) => {
                    self.cursor_steps = self.cursor_steps.saturating_add(1);
                    self.scanned_through = self.cursor.source_position();
                    self.ingest_run(document, run)?;
                    self.refresh_complete_tiles(document)?;
                }
                DisplayStep::BudgetExhausted => {
                    self.budget_exhausted = self.budget_exhausted.saturating_add(1);
                    return Ok(ScanStatus::BudgetExhausted);
                }
                DisplayStep::Complete => {
                    self.scanned_through = self.cursor.source_position();
                    self.refresh_complete_tiles(document)?;
                    self.source_complete = true;
                    return self.finish_empty(document, budget);
                }
            }
        }
    }

    fn finish_empty(
        &mut self,
        document: &Document,
        budget: &mut Budget,
    ) -> Result<ScanStatus, LayoutError> {
        let segments = &self.layout_segments;
        let objects = self.objects.as_mut().expect("prepared objects");
        if !objects.finish_empty(budget, |label| {
            let at = segments.partition_point(|segment| segment.source.end <= label.start);
            segments
                .get(at)
                .is_some_and(|segment| segment.source.start < label.end && !segment.text.is_empty())
        }) {
            return Ok(ScanStatus::BudgetExhausted);
        }
        let fallback: Vec<_> = objects
            .empty_fallbacks
            .iter()
            .map(|object| DisplaySegment {
                tile: document.tile_at(object.source.start).expect("image source"),
                source: object.source,
                replacement: Some(object.source),
                text: "[image]".to_owned(),
                block: document.block_at(object.source.start),
                link: Some(object.hit),
                style: StyleRole::LINK.with(StyleRole::IMAGE),
                kind: TokenKind::Text,
            })
            .collect();
        objects.empty_fallbacks.clear();
        let logical: Vec<_> = fallback
            .iter()
            .filter(|segment| {
                segment.source.start >= self.requested.start
                    && segment.source.start < self.requested.end
            })
            .cloned()
            .collect();
        self.display_bytes += logical
            .iter()
            .map(|segment| segment.text.len())
            .sum::<usize>();
        self.segments = crate::render::input::merge_source(
            std::mem::take(&mut self.segments),
            logical,
            |segment| segment.source.start,
        );
        self.layout_segments = crate::render::input::merge_source(
            std::mem::take(&mut self.layout_segments),
            fallback,
            |segment| segment.source.start,
        );
        if self.table_mode {
            if self.footnote_index.is_none() {
                if !budget.charge(mdtui3::WorkLimit {
                    queries: 1,
                    objects: 1,
                    ..Default::default()
                }) {
                    return Ok(ScanStatus::BudgetExhausted);
                }
                self.footnote_index = Some(
                    document
                        .footnote_references()
                        .partition_point(|r| r.source.end <= self.requested.start),
                );
            }
            while let Some(reference) = document
                .footnote_references()
                .get(self.footnote_index.unwrap())
                .filter(|r| r.source.start < self.requested.end)
            {
                let Ok(Some(resolution)) = document.resolve_footnote(
                    mdtui3::FootnoteReferenceId(self.footnote_index.unwrap()),
                    budget,
                ) else {
                    return Ok(ScanStatus::BudgetExhausted);
                };
                if matches!(resolution.target, mdtui3::FootnoteTarget::Defined(_)) {
                    self.hidden_footnotes.push(reference.source);
                }
                *self.footnote_index.as_mut().unwrap() += 1;
            }
            let tokens = self
                .layout_segments
                .iter()
                .map(|s| super::input::InputToken {
                    source: s.source,
                    replacement: s.replacement,
                    text: s.text.clone(),
                    block: s.block,
                    link: s.link,
                    style: s.style,
                    kind: s.kind,
                })
                .collect();
            let input = super::visibility::visible_tokens(
                self.requested,
                tokens,
                std::mem::take(&mut self.hidden_footnotes),
            );
            self.layout_segments = input
                .into_tokens()
                .into_iter()
                .map(|s| DisplaySegment {
                    tile: document.tile_at(s.source.start).expect("prepared cell"),
                    source: s.source,
                    replacement: s.replacement,
                    text: s.text,
                    block: s.block,
                    link: s.link,
                    style: s.style,
                    kind: s.kind,
                })
                .collect();
            self.segments = self.layout_segments.clone();
        }
        self.complete = true;
        self.lease = None;
        Ok(ScanStatus::Complete)
    }

    fn ingest_run(
        &mut self,
        document: &Document,
        mut run: DisplayRun<'_>,
    ) -> Result<(), LayoutError> {
        let objects = self.objects.as_ref().expect("prepared objects");
        for token in image_tokens(objects, &run).collect::<Vec<_>>() {
            let tile = document
                .tile_at(token.source.start)
                .ok_or(LayoutError::InvalidRange)?;
            self.push_segment(DisplaySegment {
                tile,
                source: token.source,
                replacement: token.replacement,
                text: token.text,
                block: token.block,
                link: token.link,
                style: token.style,
                kind: token.kind,
            });
        }
        if run.style.contains(Style::HIDDEN) || run.text.is_empty() {
            return Ok(());
        }
        let presentation = run
            .link
            .map(|id| self.objects.as_ref().unwrap().presentation(id))
            .unwrap_or_default();
        run.link = presentation.hit;
        let mut style = style_role(run.style, run.link.is_some());
        if presentation.image.is_some() {
            style = style.with(StyleRole::IMAGE);
        } else if presentation.disposition == Some(mdtui3::LinkDisposition::Unresolved)
            || self.objects.as_ref().unwrap().is_literal_image(run.source)
        {
            style = style.with(StyleRole::LITERAL_LINK);
        }
        let kind = token_kind(run.style);

        if let Some(replacement) = run.replacement {
            let tile = document
                .tile_at(run.source.start)
                .ok_or(LayoutError::InvalidRange)?;
            self.push_segment(DisplaySegment {
                tile,
                source: run.source,
                replacement: Some(replacement),
                text: run.text.to_owned(),
                block: run.block,
                link: run.link,
                style,
                kind,
            });
            return Ok(());
        }

        if run.text.len() != run.source.len() {
            return Err(LayoutError::InvalidRange);
        }
        let mut position = run.source.start;
        while position < run.source.end {
            let tile = document
                .tile_at(position)
                .ok_or(LayoutError::InvalidRange)?;
            let tile_span = document.tile(tile).ok_or(LayoutError::InvalidRange)?;
            let end = run.source.end.min(tile_span.end);
            let start_offset = position - run.source.start;
            let end_offset = end - run.source.start;
            if end <= position
                || !run.text.is_char_boundary(start_offset)
                || !run.text.is_char_boundary(end_offset)
            {
                return Err(LayoutError::InvalidRange);
            }
            self.push_segment(DisplaySegment {
                tile,
                source: Span::new(position, end),
                replacement: None,
                text: run.text[start_offset..end_offset].to_owned(),
                block: run.block,
                link: run.link,
                style,
                kind,
            });
            position = end;
        }
        Ok(())
    }

    fn push_segment(&mut self, segment: DisplaySegment) {
        self.layout_segments.push(segment.clone());
        if segment.source.end <= self.requested.start || segment.source.start >= self.requested.end
        {
            return;
        }
        let mut segment = segment;
        if segment.replacement.is_some() {
            if segment.source.start < self.requested.start {
                return;
            }
            segment.source.end = segment.source.end.min(self.requested.end);
        } else {
            let lo = segment.source.start.max(self.requested.start);
            let hi = segment.source.end.min(self.requested.end);
            let Some(text) = segment
                .text
                .get(lo - segment.source.start..hi - segment.source.start)
            else {
                return;
            };
            segment.text = text.to_owned();
            segment.source = Span::new(lo, hi);
        }
        self.display_bytes = self.display_bytes.saturating_add(segment.text.len());
        self.segments.push(segment);
    }

    fn refresh_complete_tiles(&mut self, document: &Document) -> Result<(), LayoutError> {
        while self.next_tile < self.tile_range.end {
            let tile = TileId(self.next_tile);
            let tile_span = document.tile(tile).ok_or(LayoutError::InvalidRange)?;
            if tile_span.start < self.requested.start {
                self.next_tile += 1;
                continue;
            }
            if tile_span.end > self.requested.end || self.scanned_through < tile_span.end {
                break;
            }
            self.complete_tiles.push(tile);
            self.next_tile += 1;
        }
        Ok(())
    }
}
