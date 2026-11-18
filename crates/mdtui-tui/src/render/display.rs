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
    pub text: super::payload::TextPayload,
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
    contexts: std::collections::HashMap<usize, Span>,
    cuts: Vec<usize>,
    rows: std::collections::HashMap<mdtui3::TableId, std::ops::Range<usize>>,
    row_ranges: std::collections::HashMap<mdtui3::TableId, Vec<std::ops::Range<usize>>>,
    next: usize,
    child: Option<Box<DisplayScanner>>,
    source_work: usize,
    skipped: Vec<(TileId, usize, u64)>,
    index_work: mdtui3::LinkWindowWork,
    folded: bool,
}

struct IndexedScan {
    next: usize,
    end: usize,
    window: Option<super::shape::ShapeWindow>,
}

pub struct DisplayScanner {
    // A fold cut is a semantic boundary, unlike an ordinary tile/window cut.
    // Both the GCB query and payload reader must use this same fragment.
    fold_fragment: Option<Span>,
    placeholders: Vec<mdtui3::FoldPlaceholder>,
    indexed: Option<Box<IndexedScan>>,
    zero_ranges: Vec<std::ops::Range<usize>>,
    tail: Option<Box<super::shape::ShapeWindow>>,
    tail_original: Option<Span>,
    tail_glyphs: Vec<DisplaySegment>,
    tail_ready: bool,
    cluster_floor: Option<usize>,
    source_cut: Option<usize>,
    selection: Option<Selection>,
    table_mode: bool,
    table_context: Option<Span>,
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
        Self::new_inner(document, requested, None, false)
    }
    fn new_inner(
        document: &Document,
        requested: Span,
        links: Option<crate::render::objects::SharedLinks>,
        exact: bool,
    ) -> Option<Self> {
        let cursor = document.display_cursor(requested)?;
        let tile_range = document.tiles_overlapping(requested);
        let mut context_start = if exact {
            requested.start
        } else {
            requested.start.saturating_sub(MAX_EDGE_OVERSCAN)
        };
        while !document.source().is_char_boundary(context_start) {
            context_start += 1;
        }
        let mut context_end = if exact {
            requested.end
        } else {
            requested
                .end
                .saturating_add(MAX_EDGE_OVERSCAN)
                .min(document.source().len())
        };
        while !document.source().is_char_boundary(context_end) {
            context_end -= 1;
        }
        if requested.is_empty() {
            context_start = requested.start;
            context_end = requested.end;
        }
        Some(Self {
            fold_fragment: None,
            placeholders: Vec::new(),
            indexed: None,
            zero_ranges: Vec::new(),
            tail: None,
            tail_original: None,
            tail_glyphs: Vec::new(),
            tail_ready: false,
            cluster_floor: None,
            source_cut: None,
            selection: None,
            table_mode: false,
            table_context: None,
            footnote_index: None,
            hidden_footnotes: Vec::new(),
            links: links.unwrap_or_else(|| crate::render::objects::SharedLinks::new(document)),
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
        let mut scanner = Self::new_inner(document, source, Some(links), true)?;
        scanner.start_probe = None;
        scanner.resolved_start = Some(source.start);
        scanner.resolved_end = Some(source.end);
        scanner.context_end = source.end;
        scanner.cursor_initialized = true;
        Some(scanner)
    }

    pub(crate) fn fold_fragment(
        document: &Document,
        source: Span,
        fragment: Span,
        links: crate::render::objects::SharedLinks,
    ) -> Option<Self> {
        let mut scanner = Self::exact(document, source, links)?;
        scanner.fold_fragment = Some(fragment);
        scanner.source_cut = Some(fragment.end);
        Some(scanner)
    }

    pub(crate) fn select(&mut self, window: super::table_window::TableWindow) {
        assert!(!self.complete && self.segments.is_empty());
        self.selection = Some(Selection {
            ranges: window.ranges,
            contexts: window.contexts,
            cuts: window.cuts,
            rows: window.rows,
            row_ranges: window.row_ranges,
            next: 0,
            child: None,
            source_work: 0,
            skipped: window.skipped,
            index_work: mdtui3::LinkWindowWork::default(),
            folded: false,
        });
    }

    pub(crate) fn projected(
        document: &Document,
        requested: Span,
        ranges: &[Span],
        contexts: &std::collections::HashMap<usize, Span>,
        links: crate::render::objects::SharedLinks,
    ) -> Option<Self> {
        let mut result = Self::exact(document, requested, links)?;
        result.selection = Some(Selection {
            ranges: ranges.iter().map(|s| (*s, false)).collect(),
            contexts: contexts.clone(),
            cuts: Vec::new(),
            rows: Default::default(),
            row_ranges: Default::default(),
            next: 0,
            child: None,
            source_work: 0,
            skipped: Vec::new(),
            index_work: Default::default(),
            folded: true,
        });
        Some(result)
    }

    pub(crate) fn inline_placeholders(&self) -> &[mdtui3::FoldPlaceholder] {
        &self.placeholders
    }
    pub(crate) fn insert_placeholder(
        &mut self,
        document: &Document,
        placeholder: mdtui3::FoldPlaceholder,
        label: String,
    ) {
        self.placeholders.push(placeholder);
        let source = placeholder.candidate.hidden;
        let segment = DisplaySegment {
            tile: placeholder.owner,
            source,
            replacement: Some(source),
            text: label.into(),
            block: document.block_at(source.start),
            link: None,
            style: StyleRole::META,
            kind: TokenKind::Text,
        };
        let at = self
            .segments
            .partition_point(|s| s.source.start < source.start);
        self.segments.insert(at, segment.clone());
        let at = self
            .layout_segments
            .partition_point(|s| s.source.start < source.start);
        self.layout_segments.insert(at, segment);
    }

    pub(crate) fn select_projected(&mut self, mut window: super::table_window::TableWindow) {
        let selection = self.selection.as_mut().expect("projected scanner");
        // Ordinary ranges still use the visible fold context; cell ranges use
        // their already intersected cell/fold context, never a raw row envelope.
        for (source, _) in &window.ranges {
            if let std::collections::hash_map::Entry::Vacant(entry) =
                window.contexts.entry(source.start)
                && let Some((_, context)) = selection
                    .contexts
                    .iter()
                    .find(|(_, context)| context.start <= source.start && source.end <= context.end)
            {
                entry.insert(*context);
            }
        }
        selection.ranges = window.ranges;
        selection.contexts = window.contexts;
        selection.rows = window.rows;
        selection.row_ranges = window.row_ranges;
        selection.cuts = window.cuts;
        selection.skipped = window.skipped;
    }
    pub(crate) fn selected_row_ranges(
        &self,
        id: mdtui3::TableId,
    ) -> Option<Vec<std::ops::Range<usize>>> {
        let selection = self.selection.as_ref()?;
        if !selection.folded {
            return None;
        }
        let mut ranges = selection.row_ranges.get(&id)?.clone();
        ranges.sort_by_key(|r| r.start);
        let mut result: Vec<std::ops::Range<usize>> = Vec::new();
        for range in ranges {
            if let Some(last) = result.last_mut().filter(|last| range.start <= last.end) {
                last.end = last.end.max(range.end);
            } else {
                result.push(range);
            }
        }
        Some(result)
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
                let mut child = if selection.folded {
                    Self::fold_fragment(
                        document,
                        source,
                        selection
                            .contexts
                            .get(&source.start)
                            .copied()
                            .unwrap_or(source),
                        self.links.clone(),
                    )
                } else if table {
                    Self::exact(document, source, self.links.clone())
                } else {
                    Self::new(document, source)
                }
                .ok_or(LayoutError::InvalidRange)?;
                child.links = self.links.clone();
                child.table_mode = table;
                child.source_cut = self.source_cut;
                child.table_context = selection.contexts.get(&source.start).copied();
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
            self.zero_ranges.append(&mut child.zero_ranges);
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

    pub(crate) fn source_cut(&self) -> Option<usize> {
        self.source_cut
    }
    pub(crate) fn set_source_cut(&mut self, cut: Option<usize>) {
        self.source_cut = cut;
    }
    pub(crate) fn cluster_floor(&self) -> Option<usize> {
        self.cluster_floor
    }
    pub(crate) fn set_cluster_floor(&mut self, source: usize) {
        self.cluster_floor = Some(source);
    }
    pub(crate) fn zero_ranges(&self) -> &[std::ops::Range<usize>] {
        &self.zero_ranges
    }
    pub fn initialize(
        &mut self,
        document: &Document,
        budget: &mut Budget,
    ) -> Result<bool, LayoutError> {
        self.links.prepare_shape(document, budget)
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
                        .saturating_sub(self.requested.start.max(self.cluster_floor.unwrap_or(0)))
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

        if !self.links.prepare_shape(document, budget)? {
            return Ok(ScanStatus::BudgetExhausted);
        }
        if !self.links.prepare(document, budget)? {
            return Ok(ScanStatus::BudgetExhausted);
        }
        if self.lease.is_none() {
            self.lease = Some(self.links.hold());
        }
        let shape_context = if let Some(fragment) = self.fold_fragment {
            fragment
        } else if self.table_mode {
            self.table_context.unwrap_or(self.requested)
        } else {
            Span::new(
                document
                    .block_at(self.requested.start)
                    .and_then(|id| document.block(id))
                    .map_or(self.requested.start, |b| b.source.start),
                document
                    .block_at(self.requested.end.saturating_sub(1))
                    .and_then(|id| document.block(id))
                    .map_or(self.requested.end, |b| b.source.end),
            )
        };
        if let Some(shape) = self.links.shape()
            && (self.source_cut.is_some_and(|cut| shape_context.end > cut)
                || shape.requires_shape(shape_context))
        {
            return self.advance_indexed(document, &shape, budget);
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

    fn advance_indexed(
        &mut self,
        document: &Document,
        shape: &super::shape::SharedShape,
        budget: &mut Budget,
    ) -> Result<ScanStatus, LayoutError> {
        let scan = self.indexed.get_or_insert_with(|| {
            Box::new(IndexedScan {
                next: self.cluster_floor.unwrap_or(self.requested.start),
                end: self.requested.end,
                window: None,
            })
        });
        let view_end = self
            .requested
            .end
            .min(self.source_cut.unwrap_or(usize::MAX));
        while scan.next < view_end {
            if scan.window.is_none() {
                if !budget.charge(mdtui3::WorkLimit {
                    queries: 1,
                    objects: 1,
                    ..Default::default()
                }) {
                    return Ok(ScanStatus::BudgetExhausted);
                }
                // Block and selected table-cell boundaries are explicit GCB
                // barriers. Source windows inside a block retain its context.
                let mut fragment = if let Some(fragment) = self.fold_fragment {
                    let block = document
                        .block_at(scan.next)
                        .and_then(|id| document.block(id))
                        .map_or(fragment, |b| b.source);
                    Span::new(fragment.start.max(block.start), fragment.end.min(block.end))
                } else if self.table_mode {
                    self.table_context.unwrap_or(self.requested)
                } else if let Some(block) = document
                    .block_at(scan.next)
                    .and_then(|id| document.block(id))
                {
                    block.source
                } else {
                    Span::new(
                        document.pending().start.min(scan.next),
                        document.source().len(),
                    )
                };
                fragment.end = fragment.end.min(self.source_cut.unwrap_or(usize::MAX));
                scan.end = fragment.end.min(view_end);
                if scan.end <= scan.next {
                    return Err(LayoutError::InvalidRange);
                }
                scan.window = Some(super::shape::ShapeWindow::new(
                    shape,
                    Span::new(scan.next, scan.end),
                    fragment,
                )?);
            }
            match scan
                .window
                .as_mut()
                .unwrap()
                .advance(shape, document, &self.links, budget)?
            {
                super::shape::ShapeWindowStep::BudgetExhausted => {
                    return Ok(ScanStatus::BudgetExhausted);
                }
                super::shape::ShapeWindowStep::Complete => {
                    scan.next = scan.end;
                    scan.window = None;
                }
                super::shape::ShapeWindowStep::Glyph(glyph) => {
                    // A decoded atom (entity/object fallback) can map a wider
                    // raw span than the requested fragment. A manual fold cut
                    // must not publish that cross-boundary payload.
                    if self.fold_fragment.is_some_and(|fragment| {
                        glyph.source.start < fragment.start || glyph.source.end > fragment.end
                    }) {
                        continue;
                    }
                    let first = document
                        .tile_at(glyph.source.start)
                        .ok_or(LayoutError::InvalidRange)?;
                    let last = document
                        .tile_at(glyph.source.end.saturating_sub(1))
                        .ok_or(LayoutError::InvalidRange)?;
                    let end =
                        last.0 + usize::from(document.tile(last).unwrap().end <= glyph.source.end);
                    if first.0 + 1 < end {
                        self.zero_ranges.push(first.0 + 1..end);
                    }
                    let segment = DisplaySegment {
                        tile: document
                            .tile_at(glyph.source.start)
                            .ok_or(LayoutError::InvalidRange)?,
                        source: glyph.source,
                        replacement: Some(glyph.source),
                        text: glyph.text,
                        block: glyph.block,
                        link: glyph.link,
                        style: glyph.style,
                        kind: TokenKind::Text,
                    };
                    self.display_bytes = self.display_bytes.saturating_add(segment.text.len());
                    self.segments.push(segment.clone());
                    self.layout_segments.push(segment);
                    self.cursor_steps += 1;
                }
            }
        }
        self.resolved_start = Some(
            self.layout_segments
                .first()
                .map_or(self.requested.start, |s| s.source.start),
        );
        self.resolved_end = Some(
            self.layout_segments
                .last()
                .map_or(self.requested.end, |s| s.source.end)
                .max(self.requested.end),
        );
        self.scanned_through = self.requested.end;
        self.refresh_complete_tiles(document)?;
        self.source_complete = true;
        self.complete = true;
        self.lease = None;
        Ok(ScanStatus::Complete)
    }

    fn finish_ascii_tail(
        &mut self,
        document: &Document,
        budget: &mut Budget,
    ) -> Result<bool, LayoutError> {
        if self.tail_ready {
            return Ok(true);
        }
        if self
            .fold_fragment
            .is_some_and(|s| s.end < document.source().len())
            || self
                .source_cut
                .is_some_and(|cut| cut < document.source().len())
            || document.is_finished()
            || self.table_mode
            || document.source().ends_with(['\r', '\n'])
        {
            self.tail_ready = true;
            return Ok(true);
        }
        let shape = self.links.shape().ok_or(LayoutError::InvalidRange)?;
        if self.tail.is_none() {
            let Some(last) = self.layout_segments.last() else {
                self.tail_ready = true;
                return Ok(true);
            };
            let fragment = self.fold_fragment.unwrap_or_else(|| {
                last.block.and_then(|id| document.block(id)).map_or(
                    Span::new(document.pending().start, document.source().len()),
                    |block| block.source,
                )
            });
            if fragment.end != document.source().len() {
                self.tail_ready = true;
                return Ok(true);
            }
            let source = if last.replacement.is_none() && last.text.len() == last.source.len() {
                Span::new(last.source.end - 1, last.source.end)
            } else {
                last.source
            };
            self.tail_original = Some(source);
            self.tail = Some(Box::new(super::shape::ShapeWindow::new(
                &shape, source, fragment,
            )?));
        }
        loop {
            match self
                .tail
                .as_mut()
                .unwrap()
                .advance(&shape, document, &self.links, budget)?
            {
                super::shape::ShapeWindowStep::BudgetExhausted => return Ok(false),
                super::shape::ShapeWindowStep::Glyph(glyph) => {
                    self.tail_glyphs.push(DisplaySegment {
                        tile: document
                            .tile_at(glyph.source.start)
                            .ok_or(LayoutError::InvalidRange)?,
                        source: glyph.source,
                        replacement: Some(glyph.source),
                        text: glyph.text,
                        block: glyph.block,
                        link: glyph.link,
                        style: glyph.style,
                        kind: TokenKind::Text,
                    })
                }
                super::shape::ShapeWindowStep::Complete => break,
            }
        }
        if let Some(first) = self.tail_glyphs.first() {
            let source = Span::new(first.source.start, self.tail_original.unwrap().end);
            fn trim(segments: &mut Vec<DisplaySegment>, source: Span) {
                segments.retain_mut(|segment| {
                    if segment.source.end <= source.start || source.end <= segment.source.start {
                        return true;
                    }
                    if segment.replacement.is_some() {
                        return false;
                    }
                    if segment.source.start < source.start {
                        let text = segment.text.contiguous().expect("bounded ASCII cursor run");
                        segment.text = text[..source.start - segment.source.start].into();
                        segment.source.end = source.start;
                        return true;
                    }
                    false
                });
            }
            trim(&mut self.layout_segments, source);
            trim(&mut self.segments, source);
            for glyph in self.tail_glyphs.drain(..) {
                self.layout_segments.push(glyph.clone());
                if glyph.source.end > self.requested.start
                    && glyph.source.start < self.requested.end
                {
                    self.segments.push(glyph);
                }
            }
            self.display_bytes = self.segments.iter().map(|segment| segment.text.len()).sum();
        }
        self.tail_ready = true;
        self.tail = None;
        Ok(true)
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
                text: "[image]".into(),
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
                let mut references = document
                    .footnote_references_in(self.requested)
                    .ok_or(LayoutError::InvalidRange)?;
                let Ok(first) = references.advance(budget) else {
                    return Ok(ScanStatus::BudgetExhausted);
                };
                self.footnote_index = Some(first.map_or(usize::MAX, |(id, _)| id.0));
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
                self.footnote_index = Some(
                    document
                        .next_footnote_reference(mdtui3::FootnoteReferenceId(
                            self.footnote_index.unwrap(),
                        ))
                        .map_or(usize::MAX, |id| id.0),
                );
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
        if !self.finish_ascii_tail(document, budget)? {
            return Ok(ScanStatus::BudgetExhausted);
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
                text: run.text.into(),
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
                text: run.text[start_offset..end_offset].into(),
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
                .contiguous()
                .expect("cursor token is contiguous")
                .get(lo - segment.source.start..hi - segment.source.start)
            else {
                return;
            };
            segment.text = text.into();
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
