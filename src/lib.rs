//! Append-only Markdown semantics and lazy viewport data structures.
//!
//! Append valid UTF-8 chunks with [`Document::append`], inspect only visible
//! [`TileId`]s, and seal EOF with [`Document::finish`]. See the repository README
//! for the deliberately relaxed dialect and amortized complexity contract.
//!
//! ```
//! use mdtui3::{Document, TileId, Style};
//! let mut doc = Document::new();
//! doc.append("Hello **world**\n").unwrap();
//! doc.finish();
//! let runs: Vec<_> = doc.runs(TileId(0)).unwrap().collect();
//! assert!(runs.iter().any(|r| r.text == "world" && r.style.contains(Style::STRONG)));
//! ```

#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

mod anchors;
mod annotations;
mod autolink;
mod block;
mod budget;
mod cache;
mod checkpoints;
mod container_folds;
mod containers;
mod destination;
mod display;
mod display_search;
mod entity;
mod excerpt;
mod fold_batch;
mod fold_navigation;
mod folding;
mod frame;
mod heading_search;
mod headings;
mod history;
mod inline;
mod layout;
mod model;
mod navigation;
mod references;
mod search;
mod table;
mod table_preview;
mod utf8;

pub use anchors::{
    AnchorSlug, AnchorTarget, FragmentLookup, FragmentStep, HeadingAnchor, HeadingAnchors,
};
pub use autolink::MAX_AUTOLINK_BYTES;
pub use block::{BLOCK_LOOKAHEAD, MAX_FENCE_LENGTH};
pub use budget::{Budget, Completion, WorkLimit};
pub use cache::{CacheValidity, DependencyKey, SourceSnapshot};
pub use checkpoints::{CheckpointIndex, CheckpointToken};
pub use containers::{Container, ContainerAncestors, ContainerId, ContainerKind, ListMarker};
pub use destination::{MAX_DESTINATION_BYTES, MAX_LINK_SUFFIX_BYTES, MAX_LINK_TITLE_BYTES};
pub use display::{DisplayCursor, DisplayRun, DisplayRuns, DisplayStep};
pub use display_search::{DisplayMatch, DisplaySearch};
pub use excerpt::{DisplayExcerpt, ExcerptEnd, ExcerptLimits, ExcerptStep};
pub use fold_batch::{FoldBatch, FoldBatchAction, FoldBatchProgress};
pub use fold_navigation::{
    FoldAnchor, FoldAnchorTarget, FoldNavigation, FoldNavigationPolicy, FoldNavigationProgress,
};
pub use folding::{
    FoldCandidate, FoldCandidates, FoldChange, FoldDefaults, FoldId, FoldLookup, FoldPlaceholder,
    FoldPreviewProgress, FoldView,
};
pub use frame::{FoldLayoutSink, LayoutPass, LayoutSink};
pub use heading_search::{HeadingMatch, HeadingSearch};
pub use headings::{Heading, HeadingIndex};
pub use history::{Bookmark, HistoryDirection, HistoryNavigation, NavigationHistory};
pub use layout::{
    Anchor, HeightIndex, HeightState, LayoutError, Measurement, ViewportCursor, ViewportPage,
    VisibleTile,
};
pub use model::*;
pub use navigation::{NavAction, Navigator, ScrollTarget};
pub use references::MAX_LABEL_BYTES;
pub use search::{Selection, SourceSearch, TaskState};
pub use table::{Alignment, Column, Table, TableCell, TableRow};
pub use table_preview::{TablePreview, TablePreviewEnd, TablePreviewLimits, TablePreviewStep};
pub use utf8::Utf8Document;

/// Validate both consumers before changing either. Replaying an update preserves
/// measurements and checkpoints; missed updates require an explicit rebuild.
pub fn apply_layout_update<S>(
    document: &Document,
    update: &Update,
    heights: &mut HeightIndex,
    checkpoints: &mut CheckpointIndex<S>,
) -> Result<(), LayoutError> {
    apply_layout_update_with_policy(
        document,
        update,
        heights,
        checkpoints,
        LayoutPolicy::default(),
    )
}

pub fn apply_layout_update_with_policy<S>(
    document: &Document,
    update: &Update,
    heights: &mut HeightIndex,
    checkpoints: &mut CheckpointIndex<S>,
    policy: LayoutPolicy,
) -> Result<(), LayoutError> {
    heights.validate_update(document, update)?;
    checkpoints.validate_update(document, update)?;
    heights.apply_with_policy(document, update, policy)?;
    checkpoints.apply_with_policy(document, update, policy)
}

use annotations::Annotations;
use block::{Fence, Line};
use inline::{Effects, Inline};
use references::References;
use std::fmt;

/// Tiles never split a UTF-8 scalar. They are source/layout checkpoints, not AST
/// subtrees or mandatory line breaks. One enormous paragraph still has small tiles.
pub const TILE_BYTES: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Finished;

impl fmt::Display for Finished {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("document is already finished")
    }
}
impl std::error::Error for Finished {}

#[derive(Clone, Copy)]
struct StreamingLine {
    block: BlockId,
    inline: bool,
    standalone: bool,
    hidden: bool,
    table: bool,
}

#[derive(Default)]
pub struct Document {
    id: DocumentId,
    source: String,
    tiles: Vec<Span>,
    blocks: Vec<Block>,
    containers: containers::Containers,
    links: Vec<LinkTarget>,
    link_info: Vec<LinkInfo>,
    headings: HeadingIndex,
    tables: Vec<Table>,
    table_header: Option<table::HeaderCandidate>,
    // One physical inline payload can become a container Setext heading on the
    // next explicit line. Its text is never copied or reparsed on confirmation.
    container_title: Option<(BlockId, Span)>,
    table_row: Option<table::RowParser>,
    references: References,
    annotations: Annotations,
    display: display::DisplayIndex,
    inline: Inline,
    current: Option<BlockId>,
    fence: Option<Fence>,
    line_start: usize,
    streaming: Option<StreamingLine>,
    finished: bool,
    revision: u64,
    work: Work,
}

impl Document {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn id(&self) -> DocumentId {
        self.id
    }
    pub fn source(&self) -> &str {
        &self.source
    }
    pub fn blocks(&self) -> &[Block] {
        &self.blocks
    }
    pub fn block(&self, id: BlockId) -> Option<&Block> {
        self.blocks.get(id.0)
    }
    /// O(1) current-revision snapshot; ID belongs to this document.
    pub fn container(&self, id: ContainerId) -> Option<Container> {
        self.containers.get(id)
    }
    pub fn container_count(&self) -> usize {
        self.containers.len()
    }
    pub fn container_ancestors(&self, id: ContainerId) -> Option<ContainerAncestors<'_>> {
        self.containers.ancestors(id)
    }
    /// Arena plus active-stack capacity; excludes source and Update buffers.
    pub fn container_arena_stats(&self) -> ArenaStats {
        self.containers.stats()
    }
    pub fn tile_count(&self) -> usize {
        self.tiles.len()
    }
    pub fn tile(&self, id: TileId) -> Option<Span> {
        self.tiles.get(id.0).copied()
    }
    pub fn references(&self) -> &[Reference] {
        &self.references.entries
    }
    pub fn links(&self) -> &[LinkTarget] {
        &self.links
    }
    pub fn link_info(&self, id: LinkId) -> Option<&LinkInfo> {
        self.link_info.get(id.0)
    }
    /// Locate link/image objects overlapping a viewport in O(log links + results).
    /// Includes empty-alt images, which have no visible label run to hit-test.
    pub fn links_in(&self, span: Span) -> impl Iterator<Item = (LinkId, &LinkInfo)> {
        let start = self
            .link_info
            .partition_point(|info| info.source.end <= span.start);
        let end = if span.is_empty() {
            start
        } else {
            self.link_info
                .partition_point(|info| info.source.start < span.end)
        };
        self.link_info[start..end.max(start)]
            .iter()
            .enumerate()
            .map(move |(i, info)| (LinkId(start + i), info))
    }
    pub fn headings(&self) -> &HeadingIndex {
        &self.headings
    }
    pub fn tables(&self) -> &[Table] {
        &self.tables
    }
    pub fn table(&self, id: TableId) -> Option<&Table> {
        self.tables.get(id.0)
    }
    pub fn table_at(&self, offset: usize) -> Option<TableId> {
        match self.block(self.block_at(offset)?)?.kind {
            BlockKind::Table { id } => Some(id),
            _ => None,
        }
    }
    pub fn is_finished(&self) -> bool {
        self.finished
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn work(&self) -> Work {
        Work {
            tree_visits: self.annotations.visits,
            ..self.work
        }
    }
    pub fn annotation_arena_stats(&self) -> ArenaStats {
        self.annotations.arena_stats()
    }

    pub fn annotation_node_count(&self) -> usize {
        self.annotations.node_count()
    }

    /// A bounded provisional line prefix, already included in the tiles as raw
    /// text. It is classified at newline, EOF, or the lookahead limit.
    pub fn pending(&self) -> Span {
        if self.streaming.is_some() {
            Span::new(self.source.len(), self.source.len())
        } else {
            Span::new(self.line_start, self.source.len())
        }
    }

    pub fn text(&self, span: Span) -> Option<&str> {
        self.source.get(span.range())
    }

    pub fn block_at(&self, offset: usize) -> Option<BlockId> {
        let i = self
            .blocks
            .partition_point(|block| block.source.end <= offset);
        self.blocks
            .get(i)
            .filter(|block| block.source.start <= offset)
            .map(|_| BlockId(i))
    }

    pub fn tile_at(&self, offset: usize) -> Option<TileId> {
        let i = self.tiles.partition_point(|span| span.end <= offset);
        self.tiles
            .get(i)
            .filter(|span| span.start <= offset)
            .map(|_| TileId(i))
    }

    /// First non-hidden source scalar at or after a UTF-8 boundary, in O(log n).
    /// This skips even enormous, fully hidden ranges without reading their bytes.
    /// Invalid boundaries or EOF return None. Provisional raw text is visible.
    pub fn next_visible(&self, offset: usize) -> Option<usize> {
        if !self.source.is_char_boundary(offset) {
            return None;
        }
        self.annotations.next_visible(offset, self.source.len())
    }

    /// First text OR drawable object. Unlike next_visible, this cannot skip an
    /// empty-alt image. A query inside an object's source keeps that object alive.
    pub fn next_drawable(&self, offset: usize) -> Option<usize> {
        if !self.source.is_char_boundary(offset) || offset >= self.source.len() {
            return None;
        }
        let text = self.next_visible(offset);
        let i = self
            .link_info
            .partition_point(|info| info.source.end <= offset);
        let object = self
            .link_info
            .get(i)
            .map(|info| info.source.start.max(offset));
        match (text, object) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    /// Find tiles intersecting a source range in O(log tiles), with no scan.
    pub fn tiles_overlapping(&self, span: Span) -> std::ops::Range<usize> {
        let start = self.tiles.partition_point(|tile| tile.end <= span.start);
        let end = if span.is_empty() {
            start
        } else {
            self.tiles.partition_point(|tile| tile.start < span.end)
        };
        start..end.max(start)
    }

    pub fn link_destination(&self, id: LinkId) -> Option<&str> {
        let span = match self.links.get(id.0)? {
            LinkTarget::Direct(span) | LinkTarget::Email(span) => *span,
            LinkTarget::Reference(symbol) => self.references.entries.get(symbol.0)?.destination?,
        };
        self.text(span)
    }

    /// Resolve inline or late-defined reference titles without copying text.
    pub fn link_title(&self, id: LinkId) -> Option<&str> {
        let span = match self.links.get(id.0)? {
            LinkTarget::Direct(_) | LinkTarget::Email(_) => self.link_info.get(id.0)?.title?,
            LinkTarget::Reference(symbol) => self.references.entries.get(symbol.0)?.title?,
        };
        self.text(span)
    }

    /// URI targets borrow source; email autolinks allocate only the mailto: URL.
    pub fn link_url(&self, id: LinkId) -> Option<std::borrow::Cow<'_, str>> {
        let destination = self.link_destination(id)?;
        Some(if matches!(self.links.get(id.0)?, LinkTarget::Email(_)) {
            std::borrow::Cow::Owned(format!("mailto:{destination}"))
        } else {
            std::borrow::Cow::Borrowed(destination)
        })
    }

    pub fn tile_key(&self, id: TileId) -> Option<TileKey> {
        let source = self.tile(id)?;
        Some(TileKey {
            document: self.id,
            source,
            syntax_revision: self.annotations.newest(source),
            reference_revision: self.references.revision,
        })
    }

    /// Borrow a lazy iterator; constructing it never parses or renders a block.
    pub fn runs(&self, id: TileId) -> Option<Runs<'_>> {
        self.runs_in(self.tile(id)?)
    }

    pub fn runs_in(&self, span: Span) -> Option<Runs<'_>> {
        self.text(span)?;
        Some(Runs {
            document: self,
            cursor: span.start,
            end: span.end,
            block: self
                .blocks
                .partition_point(|block| block.source.end <= span.start),
            marks: self.annotations.segments(span),
            pending: None,
        })
    }

    pub fn append(&mut self, text: &str) -> Result<Update, Finished> {
        let mut update = Update::default();
        self.append_into(text, &mut update)?;
        Ok(update)
    }

    /// Reuse event vector capacities. On Finished, the buffer is unchanged.
    pub fn append_into(&mut self, text: &str, update: &mut Update) -> Result<(), Finished> {
        if self.finished {
            return Err(Finished);
        }
        let old_end = self.source.len();
        self.begin_update(old_end, update);
        if text.is_empty() {
            return Ok(());
        }
        self.revision += 1;
        update.revision = self.revision;
        self.source.push_str(text);
        self.extend_tiles(old_end);
        update.appended.end = self.source.len();
        update.dirty.push(update.appended);
        update.change(update.appended, ChangeKind::Text);
        let end = self.source.len();
        let mut cursor = old_end;
        while cursor < end {
            if let Some(line) = self.streaming {
                let start = cursor;
                while cursor < end {
                    let newline = self.source.as_bytes()[cursor] == b'\n';
                    cursor += 1;
                    self.work.input_bytes += 1;
                    if newline {
                        break;
                    }
                }
                self.extend_line(line, Span::new(start, cursor), update);
                if self.source.as_bytes()[cursor - 1] == b'\n' {
                    self.end_line(line, update);
                    self.line_start = cursor;
                    self.streaming = None;
                }
            } else {
                let newline = self.source.as_bytes()[cursor] == b'\n';
                cursor += 1;
                self.work.input_bytes += 1;
                let limit = cursor - self.line_start >= BLOCK_LOOKAHEAD
                    && self.source.is_char_boundary(cursor);
                if newline || limit {
                    self.commit_line(Span::new(self.line_start, cursor), newline, update);
                    if newline {
                        self.line_start = cursor;
                    }
                }
            }
        }
        self.complete_update(update);
        Ok(())
    }

    /// Seal EOF once. Unmatched constructs become literal without reparsing.
    /// Further calls are no-ops; append after EOF returns [`Finished`].
    pub fn finish(&mut self) -> Update {
        let mut update = Update::default();
        self.finish_into(&mut update);
        update
    }

    pub fn finish_into(&mut self, update: &mut Update) {
        self.begin_update(self.source.len(), update);
        if self.finished {
            return;
        }
        self.revision += 1;
        update.revision = self.revision;
        if let Some(line) = self.streaming.take() {
            self.end_line(line, update);
        } else if self.line_start < self.source.len() {
            self.commit_line(Span::new(self.line_start, self.source.len()), true, update);
        }
        self.flush_table_header(update);
        self.close_current(update);
        let closed_start = update.closed_containers.len();
        self.containers
            .close_to(0, self.source.len(), &mut self.work, update);
        self.sync_heading_scopes(self.containers.len(), closed_start, update);
        self.headings
            .context(None, self.source.len(), &mut self.work);
        self.line_start = self.source.len();
        self.fence = None;
        self.finished = true;
        self.complete_update(update);
    }

    fn begin_update(&self, start: usize, update: &mut Update) {
        let mut dirty = std::mem::take(&mut update.dirty);
        let mut references_changed = std::mem::take(&mut update.references_changed);
        let mut tables_changed = std::mem::take(&mut update.tables_changed);
        let mut rows_changed = std::mem::take(&mut update.rows_changed);
        let mut changes = std::mem::take(&mut update.changes);
        let mut closed_containers = std::mem::take(&mut update.closed_containers);
        dirty.clear();
        references_changed.clear();
        tables_changed.clear();
        rows_changed.clear();
        changes.clear();
        closed_containers.clear();
        *update = Update {
            document: Some(self.id),
            base_revision: self.revision,
            revision: self.revision,
            appended: Span::new(start, start),
            new_blocks: self.blocks.len()..self.blocks.len(),
            new_tiles: self.tiles.len()..self.tiles.len(),
            dirty,
            references_changed,
            new_headings: self.headings.entries().len()..self.headings.entries().len(),
            new_tables: self.tables.len()..self.tables.len(),
            tables_changed,
            rows_changed,
            changes,
            new_links: self.links.len()..self.links.len(),
            new_containers: self.containers.len()..self.containers.len(),
            closed_containers,
            container_end: self.containers.end,
            changed_blocks: self.blocks.len()..self.blocks.len(),
            changed_links: self.links.len()..self.links.len(),
            changed_headings: self.headings.entries().len()..self.headings.entries().len(),
        };
    }

    fn complete_update(&self, update: &mut Update) {
        update.new_blocks.end = self.blocks.len();
        update.new_tiles.end = self.tiles.len();
        update.new_headings.end = self.headings.entries().len();
        update.new_tables.end = self.tables.len();
        update.new_links.end = self.links.len();
        update.new_containers.end = self.containers.len();
        update.container_end = self.containers.end;
        if update.base_revision != update.revision {
            update.changed_blocks = update.new_blocks.start.saturating_sub(1)..self.blocks.len();
            update.changed_links = update.new_links.start.saturating_sub(1)..self.links.len();
            update.changed_headings =
                update.new_headings.start.saturating_sub(1)..self.headings.entries().len();
        }
    }

    fn extend_tiles(&mut self, start: usize) {
        let mut cursor = start;
        while cursor < self.source.len() {
            if let Some(tile) = self.tiles.last_mut() {
                let mut end = tile.start.saturating_add(TILE_BYTES).min(self.source.len());
                while !self.source.is_char_boundary(end) {
                    end -= 1;
                }
                if end > cursor {
                    tile.end = end;
                    cursor = end;
                    continue;
                }
            }
            let mut end = cursor.saturating_add(TILE_BYTES).min(self.source.len());
            while !self.source.is_char_boundary(end) {
                end -= 1;
            }
            self.tiles.push(Span::new(cursor, end));
            cursor = end;
        }
    }

    fn effects<'a>(&'a mut self, update: &'a mut Update) -> Effects<'a> {
        Effects {
            source: &self.source,
            annotations: &mut self.annotations,
            display: &mut self.display,
            references: &mut self.references,
            links: &mut self.links,
            link_info: &mut self.link_info,
            update,
            work: &mut self.work,
        }
    }

    fn mark(&mut self, span: Span, style: Style, update: &mut Update) {
        self.effects(update).mark(span, style, None);
    }

    fn feed(&mut self, span: Span, update: &mut Update) {
        let mut inline = std::mem::take(&mut self.inline);
        inline.feed(span, &mut self.effects(update));
        self.inline = inline;
    }

    fn close_current(&mut self, update: &mut Update) {
        let mut inline = std::mem::take(&mut self.inline);
        inline.finish(&mut self.effects(update));
        self.inline = inline;
        if let Some(id) = self.current.take() {
            self.blocks[id.0].state = SyntaxState::Sealed;
        }
    }

    fn new_block(&mut self, span: Span, kind: BlockKind, update: &mut Update) -> BlockId {
        self.close_current(update);
        let id = BlockId(self.blocks.len());
        self.blocks.push(Block {
            source: span,
            kind,
            state: SyntaxState::Growing,
            container: self.containers.inner(),
        });
        update.change(span, ChangeKind::Structure);
        self.current = Some(id);
        id
    }

    fn sync_heading_scopes(&mut self, new_start: usize, closed_start: usize, update: &Update) {
        for &id in &update.closed_containers[closed_start..] {
            let end = self.containers.get(id).unwrap().source.end;
            self.headings.close_scope(id, end, &mut self.work);
        }
        for i in new_start..self.containers.len() {
            let container = self.containers.get(ContainerId(i)).unwrap();
            self.headings.open_scope(container.parent, &mut self.work);
        }
    }

    fn commit_line(&mut self, span: Span, complete: bool, update: &mut Update) {
        self.work.classified_bytes += span.len() as u64;
        self.containers.end = span.end;
        self.headings.parsed_end = span.end;
        let mut header = self.table_header.take();
        if header.as_ref().is_some_and(|h| h.container.is_none()) {
            let root_header = header.take().unwrap();
            if complete
                && let Some(columns) = table::delimiter(&self.source, span, root_header.columns)
            {
                self.confirm_table(root_header, span, columns, update);
                return;
            }
            self.feed_header(root_header, update);
        }
        let previous_title = self.container_title.take().or_else(|| {
            header.as_ref().map(|h| {
                (
                    h.block.unwrap(),
                    Span::new(h.content.start, block::body_end(&self.source, h.content)),
                )
            })
        });
        let paragraph = self.current.is_some_and(|id| {
            self.blocks[id.0].kind == BlockKind::Paragraph && self.blocks[id.0].container.is_none()
        });
        let mut classification =
            block::classify(&self.source, span, complete, self.fence, paragraph);
        let mut content = span;
        let mut table_text = matches!(classification, Line::Text);
        if self.fence.is_none() {
            // Preserve root-level rule/Setext precedence. The inner payload
            // recognizes headings/tables; other container syntax stays literal.
            let root_markers = matches!(
                classification,
                Line::Indented { .. } | Line::Text | Line::Blank
            );
            let new_start = self.containers.len();
            let closed_start = update.closed_containers.len();
            let line = self.containers.line(
                &self.source,
                span,
                containers::ContainerOptions {
                    complete,
                    root_markers,
                    paragraph: previous_title.and_then(|(id, _)| self.blocks[id.0].container),
                    table_header: header.as_ref().map(|h| (h.container.unwrap(), h.columns)),
                },
                &mut self.work,
                update,
            );
            self.sync_heading_scopes(new_start, closed_start, update);
            self.headings
                .context(self.containers.inner(), span.start, &mut self.work);
            if let Some(line) = line {
                let same_scope = previous_title
                    .is_some_and(|(id, _)| self.blocks[id.0].container == self.containers.inner());
                let inner = Span::new(line.prefix, span.end);
                content = inner;
                self.work.classified_bytes += inner.len() as u64;
                let inner_classification =
                    block::classify(&self.source, inner, complete, None, same_scope);
                table_text = matches!(inner_classification, Line::Text);
                classification = match inner_classification {
                    heading @ (Line::Heading { .. } | Line::Setext(_)) => heading,
                    _ => Line::Container {
                        prefix: line.prefix,
                        kind: line.kind,
                    },
                };
            }
        }
        if let Some(header) = header {
            if header.container == self.containers.inner()
                && complete
                && let Some(columns) = table::delimiter(&self.source, content, header.columns)
            {
                self.confirm_table(header, span, columns, update);
                return;
            }
            self.feed_header(header, update);
        }
        if let Some(block) = self.current
            && let BlockKind::Table { id } = self.blocks[block.0].kind
        {
            if table_text && self.blocks[block.0].container == self.containers.inner() {
                self.blocks[block.0].source.end = span.end;
                self.mark(span, Style::default(), update);
                self.start_table_row(id, span, content.start, complete, update);
                return;
            }
            self.close_current(update);
        }
        if complete
            && table_text
            && let Some(cells) = table::split(&self.source, content)
        {
            let block = if paragraph && self.containers.inner().is_none() {
                None
            } else {
                let kind = match classification {
                    Line::Container { kind, .. } => kind,
                    _ => BlockKind::Paragraph,
                };
                Some(self.new_block(span, kind, update))
            };
            self.table_header = Some(table::HeaderCandidate {
                block,
                source: span,
                content,
                container: self.containers.inner(),
                columns: cells.len(),
            });
            self.mark(span, Style::default(), update);
            self.mark(Span::new(span.start, content.start), Style::HIDDEN, update);
            let end = block::body_end(&self.source, span);
            if end + 2 == span.end {
                self.mark(Span::new(end, end + 1), Style::HIDDEN, update);
            }
            return;
        }
        if let Line::Definition {
            label,
            destination,
            title,
        } = classification
        {
            if let Some(symbol) = self
                .references
                .intern(&self.source[label.range()], &mut self.work)
            {
                self.new_block(span, BlockKind::Definition { symbol }, update);
                self.mark(span, Style::HIDDEN, update);
                if self
                    .references
                    .define(symbol, destination, title, self.revision)
                {
                    update.references_changed.push(symbol);
                    if let Some(start) = self.references.entries[symbol.0].first_use {
                        update.change(Span::new(start, self.source.len()), ChangeKind::Reference);
                    }
                }
                self.close_current(update);
                return;
            }
            classification = Line::Text;
        }
        let body_end = block::body_end(&self.source, span);
        let mut payload = span;
        let mut suffix = None;
        let mut inline = true;
        let mut standalone = false;
        let mut hidden = false;
        let mut container_text = false;
        let id = match classification {
            Line::Text => {
                if paragraph {
                    let id = self.current.unwrap();
                    self.blocks[id.0].source.end = span.end;
                    id
                } else {
                    self.new_block(span, BlockKind::Paragraph, update)
                }
            }
            Line::Blank => {
                inline = false;
                standalone = true;
                self.new_block(span, BlockKind::Blank, update)
            }
            Line::Setext(level) => {
                let (id, content) = if self.containers.inner().is_some() {
                    previous_title.unwrap()
                } else {
                    let id = self.current.unwrap();
                    (
                        id,
                        Span::new(
                            self.blocks[id.0].source.start,
                            block::body_end(&self.source, self.blocks[id.0].source),
                        ),
                    )
                };
                self.blocks[id.0].kind = BlockKind::Heading { level };
                self.blocks[id.0].source.end = span.end;
                let whole = self.blocks[id.0].source;
                self.headings.push(
                    Heading {
                        block: id,
                        container: self.blocks[id.0].container,
                        level,
                        source: whole,
                        content,
                    },
                    &mut self.work,
                );
                self.mark(whole, Style::default(), update);
                self.mark(span, Style::HIDDEN, update);
                self.close_current(update);
                return;
            }
            Line::FenceOpen { fence, info } => {
                inline = false;
                hidden = true;
                let id = self.new_block(span, BlockKind::Code { info }, update);
                self.fence = Some(fence);
                id
            }
            Line::FenceClose => {
                let id = self.current.unwrap();
                self.blocks[id.0].source.end = span.end;
                self.mark(span, Style::HIDDEN, update);
                self.fence = None;
                self.close_current(update);
                return;
            }
            Line::Code => {
                inline = false;
                let id = self.current.unwrap();
                self.blocks[id.0].source.end = span.end;
                id
            }
            Line::Indented { prefix } => {
                inline = false;
                payload.start = prefix;
                if let Some(id) = self.current
                    && matches!(self.blocks[id.0].kind, BlockKind::Code { .. })
                {
                    self.blocks[id.0].source.end = span.end;
                    id
                } else {
                    self.new_block(span, BlockKind::Code { info: None }, update)
                }
            }
            Line::Heading {
                level,
                prefix,
                body_end: content_end,
            } => {
                standalone = true;
                payload.start = prefix;
                if content_end < body_end {
                    suffix = Some(Span::new(content_end, body_end));
                    payload.end = content_end;
                }
                self.new_block(span, BlockKind::Heading { level }, update)
            }
            Line::Container { prefix, kind } => {
                standalone = true;
                payload.start = prefix;
                let blank = complete
                    && self.source[prefix..body_end]
                        .bytes()
                        .all(|b| b.is_ascii_whitespace());
                inline = !blank;
                container_text = !blank;
                let kind = if blank && kind == BlockKind::Paragraph {
                    BlockKind::Blank
                } else {
                    kind
                };
                self.new_block(span, kind, update)
            }
            Line::Rule => {
                standalone = true;
                inline = false;
                self.new_block(span, BlockKind::ThematicBreak, update)
            }
            Line::Definition { .. } => unreachable!(),
        };
        // A classification change invalidates provisional raw runs, even without
        // any hidden delimiter (e.g. an indented code block).
        self.mark(
            span,
            if hidden {
                Style::HIDDEN
            } else {
                Style::default()
            },
            update,
        );
        if payload.start > span.start {
            self.mark(Span::new(span.start, payload.start), Style::HIDDEN, update);
        }
        if let Some(suffix) = suffix {
            self.mark(suffix, Style::HIDDEN, update);
        }
        if body_end + 2 == span.end {
            self.mark(Span::new(body_end, body_end + 1), Style::HIDDEN, update);
        }
        if inline {
            self.feed(payload, update);
            if suffix.is_some() {
                self.feed(Span::new(body_end, span.end), update);
            }
        }
        if container_text {
            self.container_title = Some((id, Span::new(payload.start, body_end)));
        }
        if let BlockKind::Heading { level } = self.blocks[id.0].kind {
            self.headings.push(
                Heading {
                    block: id,
                    container: self.blocks[id.0].container,
                    level,
                    source: span,
                    content: Span::new(payload.start, payload.end.min(body_end)),
                },
                &mut self.work,
            );
        }
        let line = StreamingLine {
            block: id,
            inline,
            standalone,
            hidden,
            table: false,
        };
        if complete {
            self.end_line(line, update);
        } else {
            self.streaming = Some(line);
        }
    }

    fn extend_line(&mut self, line: StreamingLine, span: Span, update: &mut Update) {
        self.containers.end = span.end;
        self.headings.parsed_end = span.end;
        self.blocks[line.block.0].source.end = span.end;
        if let Some((id, content)) = &mut self.container_title
            && *id == line.block
        {
            content.end = block::body_end(&self.source, self.blocks[line.block.0].source);
        }
        if line.table {
            self.feed_table_row(span, update);
            return;
        }
        if matches!(self.blocks[line.block.0].kind, BlockKind::Heading { .. }) {
            self.headings.extend(
                line.block,
                span.end,
                block::body_end(&self.source, self.blocks[line.block.0].source),
            );
        }
        if line.hidden {
            self.mark(span, Style::HIDDEN, update);
        }
        if line.inline {
            self.feed(span, update);
        }
        if self.source.as_bytes()[span.end - 1] == b'\n'
            && span.end >= 2
            && self.source.as_bytes()[span.end - 2] == b'\r'
        {
            self.mark(Span::new(span.end - 2, span.end - 1), Style::HIDDEN, update);
        }
    }

    fn end_line(&mut self, line: StreamingLine, update: &mut Update) {
        if line.table {
            let mut parser = self.table_row.take().unwrap();
            let mut tables = std::mem::take(&mut self.tables);
            parser.finish(
                self.blocks[line.block.0].source.end,
                &mut tables[parser.table.0],
                &mut self.effects(update),
            );
            self.tables = tables;
        }
        if line.standalone {
            self.close_current(update);
        }
    }

    fn flush_table_header(&mut self, update: &mut Update) {
        if let Some(header) = self.table_header.take() {
            self.feed_header(header, update);
        }
    }

    fn feed_header(&mut self, header: table::HeaderCandidate, update: &mut Update) {
        if header.block.is_none() {
            self.blocks[self.current.unwrap().0].source.end = header.source.end;
        }
        self.mark(header.source, Style::default(), update);
        self.feed(header.content, update);
        if header.container.is_some() {
            self.close_current(update);
        }
    }

    fn confirm_table(
        &mut self,
        header: table::HeaderCandidate,
        delimiter: Span,
        columns: Vec<Alignment>,
        update: &mut Update,
    ) {
        let id = TableId(self.tables.len());
        let block = header
            .block
            .unwrap_or_else(|| self.new_block(header.source, BlockKind::Table { id }, update));
        self.blocks[block.0].kind = BlockKind::Table { id };
        self.blocks[block.0].source.end = delimiter.end;
        self.tables
            .push(Table::new(block, header.source, delimiter, columns));
        self.start_table_row(id, header.source, header.content.start, true, update);
        self.tables[id.0].source.end = delimiter.end;
        self.mark(
            Span::new(header.source.start, delimiter.end),
            Style::default(),
            update,
        );
        self.mark(delimiter, Style::HIDDEN, update);
    }

    fn start_table_row(
        &mut self,
        id: TableId,
        span: Span,
        content_start: usize,
        complete: bool,
        update: &mut Update,
    ) {
        self.table_row = Some(table::RowParser::new(
            id,
            &mut self.tables[id.0],
            span.start,
            content_start,
        ));
        self.mark(Span::new(span.start, content_start), Style::HIDDEN, update);
        self.feed_table_row(Span::new(content_start, span.end), update);
        let line = StreamingLine {
            block: self.tables[id.0].block,
            inline: false,
            standalone: false,
            hidden: false,
            table: true,
        };
        if complete {
            self.end_line(line, update);
        } else {
            self.streaming = Some(line);
        }
    }

    fn feed_table_row(&mut self, span: Span, update: &mut Update) {
        let mut parser = self.table_row.take().unwrap();
        let mut tables = std::mem::take(&mut self.tables);
        parser.feed(span, &mut tables[parser.table.0], &mut self.effects(update));
        self.tables = tables;
        self.table_row = Some(parser);
    }
}

/// Borrowed semantic run. Hidden markers are returned too, so a renderer can
/// maintain exact source anchors; omit them when emitting display text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenderRun<'a> {
    pub source: Span,
    pub text: &'a str,
    pub block: Option<BlockId>,
    pub style: Style,
    pub link: Option<LinkId>,
}

pub struct Runs<'a> {
    document: &'a Document,
    cursor: usize,
    end: usize,
    block: usize,
    marks: annotations::Segments<'a>,
    pending: Option<(Span, annotations::Mark)>,
}

impl Runs<'_> {
    /// Tree nodes traversed by this query; independent of parser mutation work.
    pub fn annotation_visits(&self) -> u64 {
        self.marks.visits
    }
}

impl<'a> Iterator for Runs<'a> {
    type Item = RenderRun<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.cursor == self.end {
            return None;
        }
        let start = self.cursor;
        while self
            .document
            .blocks
            .get(self.block)
            .is_some_and(|b| b.source.end <= start)
        {
            self.block += 1;
        }
        let (block, limit) = match self.document.blocks.get(self.block) {
            Some(b) if b.source.start <= start => {
                (Some(BlockId(self.block)), b.source.end.min(self.end))
            }
            Some(b) => (None, b.source.start.min(self.end)),
            None => (None, self.end),
        };
        let (span, first) = self.pending.take().or_else(|| self.marks.next())?;
        let mut end = span.end.min(limit);
        self.pending = (span.end > end).then_some((span, first));
        while end < limit {
            let (span, mark) = self.marks.next()?;
            if mark.style != first.style || mark.link != first.link {
                self.pending = Some((span, mark));
                break;
            }
            end = span.end.min(limit);
            self.pending = (span.end > end).then_some((span, mark));
        }
        self.cursor = end;
        let mut style = first.style;
        if block.is_some_and(|id| matches!(self.document.blocks[id.0].kind, BlockKind::Code { .. }))
        {
            style = style.union(Style::CODE);
        }
        Some(RenderRun {
            source: Span::new(start, end),
            text: &self.document.source[start..end],
            block,
            style,
            link: first.link,
        })
    }
}
