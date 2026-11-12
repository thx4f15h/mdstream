//! Append-only Markdown semantics and lazy viewport data structures.
//!
//! Append valid UTF-8 chunks with [`Document::append`], inspect only visible
//! [`TileId`]s, and seal EOF with [`Document::finish`]. See the repository README
//! for the deliberately relaxed dialect and amortized complexity contract.
//!
//! ```
//! use mdstream::{Document, TileId, Style};
//! let mut doc = Document::new();
//! doc.append("Hello **world**\n").unwrap();
//! doc.finish();
//! let runs: Vec<_> = doc.runs(TileId(0)).unwrap().collect();
//! assert!(runs.iter().any(|r| r.text == "world" && r.style.contains(Style::STRONG)));
//! ```

#![forbid(unsafe_code)]
#![doc = include_str!("../../../README.md")]

mod anchors;
mod annotations;
mod autolink;
mod block;
mod budget;
mod cache;
mod checkpoints;
mod code_info;
mod containers;
mod deferred;
mod destination;
mod display;
mod display_search;
mod entity;
mod excerpt;
mod fold_batch;
mod fold_intervals;
mod fold_navigation;
mod fold_ranges;
mod folding;
mod footnotes;
mod frame;
mod heading_search;
mod headings;
mod history;
#[cfg(test)]
mod index_boundaries;
mod inline;
mod layout;
mod link_destination;
mod link_title;
mod math;
mod model;
mod navigation;
mod paragraph_folds;
mod references;
mod search;
mod source_fold_restore;
mod source_folds;
mod table;
mod table_preview;
mod utf8;

pub use anchors::{
    AnchorSlug, AnchorTarget, FragmentLookup, FragmentStep, HeadingAnchor, HeadingAnchors,
};
pub use annotations::QueryScratch;
pub use autolink::MAX_AUTOLINK_BYTES;
pub use block::{BLOCK_LOOKAHEAD, MAX_FENCE_LENGTH};
pub use budget::{Budget, Completion, WorkLimit};
pub use cache::{CacheValidity, DependencyKey, SourceSnapshot};
pub use checkpoints::{
    CheckpointCompaction, CheckpointCompactionStats, CheckpointIndex, CheckpointToken,
};
pub use code_info::{CodeInfo, CodeInfoChunk};
pub use containers::{
    Container, ContainerAncestors, ContainerId, ContainerKind, ListMarker, ListTightness,
};
pub use destination::{MAX_DESTINATION_BYTES, MAX_LINK_SUFFIX_BYTES, MAX_LINK_TITLE_BYTES};
pub use display::{DisplayCursor, DisplayRun, DisplayRuns, DisplayRunsWithScratch, DisplayStep};
pub use display_search::{DisplayMatch, DisplaySearch};
pub use excerpt::{DisplayExcerpt, ExcerptEnd, ExcerptLimits, ExcerptStep};
pub use fold_batch::{FoldBatch, FoldBatchAction, FoldBatchProgress};
pub use fold_navigation::{
    FoldAnchor, FoldAnchorTarget, FoldNavigation, FoldNavigationPolicy, FoldNavigationProgress,
};
pub use folding::{
    ContainerFoldFilter, FoldCandidate, FoldCandidates, FoldChange, FoldDefaults, FoldId,
    FoldLookup, FoldPlaceholder, FoldPreviewProgress, FoldView, TableRowRange,
};
pub use footnotes::{
    Footnote, FootnoteChunk, FootnoteContent, FootnoteDefinition, FootnoteId, FootnoteReference,
    FootnoteReferenceId, FootnoteReferences, FootnoteResolution, FootnoteTarget,
    MAX_FOOTNOTE_LABEL_BYTES,
};
pub use frame::{FoldLayoutSink, LayoutPass, LayoutSink};
pub use heading_search::{HeadingMatch, HeadingSearch};
pub use headings::{Heading, HeadingIndex};
pub use history::{Bookmark, HistoryDirection, HistoryNavigation, NavigationHistory};
pub use layout::{
    Anchor, HeightIndex, HeightState, LayoutError, Measurement, ViewportCursor, ViewportPage,
    VisibleTile,
};
pub use link_destination::{LinkDestination, LinkDestinationChunk};
pub use link_title::{LinkTitle, LinkTitleChunk};
pub use math::{Math, MathChunk, MathContent, MathId, MathKind, MathObjects};
pub use model::*;
pub use navigation::{NavAction, Navigator, ScrollTarget};
pub use paragraph_folds::{
    ParagraphAutoFold, ParagraphAutoFoldProgress, ParagraphFoldCandidate, ParagraphFoldCandidates,
    ParagraphFoldPolicy,
};
pub use references::MAX_LABEL_BYTES;
pub use search::{Selection, SourceSearch, TaskState};
pub use source_fold_restore::{SourceFoldHost, SourceFoldRestore, SourceFoldSnapshot};
pub use source_folds::{
    SourceFoldCandidates, SourceFoldId, SourceFoldProgress, SourceFoldRegistration,
};
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
use deferred::{DeferredFragment, DeferredSources, SourceOwner};
use inline::{Effects, Eligibility, Inline, Publishing, RootEligibility};
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

// No new inline/link object is published for these undecided bounded sources.
// The first ordinary block preserves paragraph/lazy eligibility through replay.
enum DefinitionLabel {
    PublishedTitle {
        owner: Option<SymbolId>,
    },
    Target(Span),
    Multiline {
        raw: String,
        closed: bool,
        columns: Option<usize>,
    },
}

struct DefinitionCandidate {
    block: BlockId,
    label: DefinitionLabel,
    sources: DeferredSources,
    title: Option<DefinitionTitleBody>,
}

struct DefinitionTitleBody {
    parser: destination::Destination,
    columns: Option<usize>,
    gaps: Vec<Span>,
}

// The target is already published. Only its first definition owns the symbol;
// duplicates may consume a title without changing any reference metadata.
struct DefinitionTitleCandidate {
    block: BlockId,
    owner: Option<SymbolId>,
}

// Only the active logical sequence owns this key; ended scopes cannot revive it.
struct ContainerEligibility {
    first: BlockId,
    scope: ContainerId,
    observer: Eligibility,
    prefix_source: Option<usize>,
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
    link_order: std::collections::BTreeMap<usize, LinkId>,
    // Parallel stable LinkId slots; candidates move their structural gaps here.
    direct_title_gaps: Vec<Vec<Span>>,
    maths: Vec<Math>,
    math_gaps: Vec<Span>,
    headings: HeadingIndex,
    tables: Vec<Table>,
    table_header: Option<table::HeaderCandidate>,
    streaming_header: Option<table::StreamingHeader>,
    table_delimiter: Option<table::DelimiterCandidate>,
    definition_candidate: Option<DefinitionCandidate>,
    definition_title_candidate: Option<DefinitionTitleCandidate>,
    // First leaf and content envelope of the current container text sequence.
    // Setext confirmation never merges leaves or reparses their inline content.
    container_title: Option<(BlockId, Span)>,
    // Final paragraph ends keyed by the first physical leaf. The active
    // sequence reuses container_title/current; ordinary growth writes no index.
    container_paragraph_ends: std::collections::BTreeMap<BlockId, usize>,
    table_row: Option<table::RowParser>,
    references: References,
    footnotes: footnotes::Footnotes,
    annotations: Annotations,
    display: display::DisplayIndex,
    inline: Inline,
    root_eligibility: Option<RootEligibility>,
    container_eligibility: Option<ContainerEligibility>,
    atx: Option<block::Atx>,
    // Inline continuity is independent of the physical leaf-block identity.
    container_inline: Option<ContainerId>,
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
    /// Stable allocation order. Use `links_in` for source order: an email
    /// waiting on an emphasis opener may be confirmed after a later object.
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
            .link_order
            .range(..=span.start)
            .next_back()
            .filter(|(_, id)| self.link_info[id.0].source.end > span.start)
            .map_or(span.start, |(&at, _)| at);
        let end = if span.is_empty() {
            start
        } else {
            span.end.max(start)
        };
        self.link_order
            .range(start..end)
            .map(|(_, &id)| (id, &self.link_info[id.0]))
    }

    pub(crate) fn next_link(&self, id: LinkId) -> Option<LinkId> {
        let start = self.link_info(id)?.source.start;
        self.link_order
            .range((std::ops::Bound::Excluded(start), std::ops::Bound::Unbounded))
            .next()
            .map(|(_, &id)| id)
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
    /// text. It is classified at newline, EOF, or the lookahead limit. A long
    /// separator probe retains this bounded prefix while its raw tail grows.
    /// An eligible long header has already been classified; its deferred payload
    /// (after a late first pipe, or the whole early-pipe header) is kept separately
    /// and does not enlarge this pending prefix.
    pub fn pending(&self) -> Span {
        if let Some(delimiter) = &self.table_delimiter {
            delimiter.prefix
        } else if self.streaming.is_some() || self.streaming_header.is_some() {
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
        self.annotations.next_visible_resolving(
            offset,
            self.source.len(),
            &self.references.entries,
            &self.links,
        )
    }

    /// First text OR drawable object. Unlike next_visible, this cannot skip an
    /// empty-alt image. A query inside an object's source keeps that object alive.
    pub fn next_drawable(&self, offset: usize) -> Option<usize> {
        if !self.source.is_char_boundary(offset) || offset >= self.source.len() {
            return None;
        }
        let text = self.next_visible(offset);
        let object = self
            .links_in(Span::new(offset, self.source.len()))
            .next()
            .map(|(_, info)| info.source.start.max(offset));
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
        self.text(self.link_destination_span(id)?)
    }

    fn link_destination_span(&self, id: LinkId) -> Option<Span> {
        Some(match self.links.get(id.0)? {
            LinkTarget::Direct(span) | LinkTarget::Email(span) | LinkTarget::Www(span) => *span,
            LinkTarget::Reference(symbol) => self.references.entries.get(symbol.0)?.destination?,
        })
    }

    /// Resolve inline or late-defined reference titles without copying text.
    /// Returns the original source envelope, including internal container
    /// prefixes. [`Document::decoded_link_title`] omits those structural gaps.
    pub fn link_title(&self, id: LinkId) -> Option<&str> {
        self.text(self.link_title_span(id)?)
    }

    fn link_title_span(&self, id: LinkId) -> Option<Span> {
        match self.links.get(id.0)? {
            LinkTarget::Direct(_) | LinkTarget::Email(_) | LinkTarget::Www(_) => {
                self.link_info.get(id.0)?.title
            }
            LinkTarget::Reference(symbol) => self.references.entries.get(symbol.0)?.title,
        }
    }

    /// Explicit/URI targets borrow source. Email/www autolinks allocate a URL
    /// with the implied mailto:/http:// prefix; raw destinations remain borrowed.
    pub fn link_url(&self, id: LinkId) -> Option<std::borrow::Cow<'_, str>> {
        let destination = self.link_destination(id)?;
        Some(if matches!(self.links.get(id.0)?, LinkTarget::Email(_)) {
            std::borrow::Cow::Owned(format!("mailto:{destination}"))
        } else if matches!(self.links.get(id.0)?, LinkTarget::Www(_)) {
            std::borrow::Cow::Owned(format!("http://{destination}"))
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
            footnote_revision: self.footnotes.revision,
        })
    }

    /// Borrow a lazy iterator; constructing it never parses or renders a block.
    pub fn runs(&self, id: TileId) -> Option<Runs<'_>> {
        self.runs_in(self.tile(id)?)
    }

    pub fn runs_in(&self, span: Span) -> Option<Runs<'_>> {
        self.text(span)?;
        Some(self.runs_with_segments(span, self.annotations.segments(span)))
    }

    /// Like `runs`, retaining the traversal allocation in caller storage.
    pub fn runs_with_scratch<'doc, 'scratch>(
        &'doc self,
        id: TileId,
        scratch: &'scratch mut QueryScratch,
    ) -> Option<RunsWithScratch<'doc, 'scratch>> {
        self.runs_in_with_scratch(self.tile(id)?, scratch)
    }

    /// Like `runs_in`, with reusable O(log n) traversal storage. Invalid ranges
    /// leave scratch unchanged. Drop the iterator before borrowing scratch
    /// again; yielded runs remain usable for the document's lifetime.
    pub fn runs_in_with_scratch<'doc, 'scratch>(
        &'doc self,
        span: Span,
        scratch: &'scratch mut QueryScratch,
    ) -> Option<RunsWithScratch<'doc, 'scratch>> {
        self.text(span)?;
        let marks = self
            .annotations
            .segments_with_scratch(span, std::mem::take(scratch));
        Some(RunsWithScratch {
            runs: self.runs_with_segments(span, marks),
            scratch,
        })
    }

    fn runs_with_segments<'a>(&'a self, span: Span, marks: annotations::Segments<'a>) -> Runs<'a> {
        Runs {
            document: self,
            cursor: span.start,
            end: span.end,
            block: self
                .blocks
                .partition_point(|block| block.source.end <= span.start),
            marks,
            pending: None,
        }
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
            if let Some(mut delimiter) = self.table_delimiter.take() {
                let start = cursor;
                while cursor < end {
                    let newline = self.source.as_bytes()[cursor] == b'\n';
                    cursor += 1;
                    self.work.input_bytes += 1;
                    if newline {
                        break;
                    }
                }
                let complete = self.source.as_bytes()[cursor - 1] == b'\n';
                if !delimiter.feed(&self.source[start..cursor], &mut self.work) {
                    self.reject_long_delimiter(delimiter.prefix, cursor, complete, update);
                } else if complete {
                    self.finish_long_delimiter(delimiter, cursor, update);
                } else {
                    self.table_delimiter = Some(delimiter);
                }
            } else if let Some(mut candidate) = self.streaming_header.take() {
                let start = cursor;
                while cursor < end {
                    let newline = self.source.as_bytes()[cursor] == b'\n';
                    cursor += 1;
                    self.work.input_bytes += 1;
                    if newline {
                        break;
                    }
                }
                let eager = !candidate.summary.pipe;
                let first_pipe = candidate
                    .summary
                    .feed(&self.source[start..cursor], &mut self.work);
                if eager {
                    let parsed_end = first_pipe.map_or(cursor, |offset| start + offset);
                    let span = Span::new(start, parsed_end);
                    match candidate.header.mode {
                        table::HeaderMode::Ordinary => {
                            self.feed(span, update);
                            candidate.header.parsed_end = parsed_end;
                        }
                        table::HeaderMode::Deferred(key) => self.observe_header(key, span, 0),
                    }
                }
                candidate.header.source.end = cursor;
                candidate.header.content.end = cursor;
                if let Some(block) = candidate.header.block {
                    self.blocks[block.0].source.end = cursor;
                } else if candidate.header.paragraph_end.is_some() {
                    self.blocks[self.current.unwrap().0].source.end = cursor;
                }
                self.containers.end = cursor;
                self.headings.parsed_end = cursor;
                if self.header_crossed() {
                    self.reject_streaming_header(candidate.header, update);
                } else if self.source.as_bytes()[cursor - 1] == b'\n' {
                    self.finish_header(candidate, update);
                    self.line_start = cursor;
                } else {
                    self.streaming_header = Some(candidate);
                }
            } else if let Some(line) = self.streaming {
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
                    let span = Span::new(self.line_start, cursor);
                    if !newline && self.probe_long_delimiter(span, update) {
                        continue;
                    }
                    self.commit_line(span, newline, true, update);
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
        if let Some(delimiter) = self.table_delimiter.take() {
            self.finish_long_delimiter(delimiter, self.source.len(), update);
        } else if let Some(candidate) = self.streaming_header.take() {
            self.finish_header(candidate, update);
        } else if let Some(line) = self.streaming.take() {
            self.end_line(line, update);
        } else if self.line_start < self.source.len() {
            self.commit_line(
                Span::new(self.line_start, self.source.len()),
                true,
                true,
                update,
            );
        }
        self.flush_definition_candidate(update);
        self.seal_definition_title(update);
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
        if let Some(first) = self.footnotes.references.first() {
            self.footnotes.revision = self.revision;
            update.change(
                Span::new(first.source.start, self.source.len()),
                ChangeKind::Structure,
            );
        }
        self.complete_update(update);
    }

    fn begin_update(&self, start: usize, update: &mut Update) {
        let mut dirty = std::mem::take(&mut update.dirty);
        let mut footnotes_changed = std::mem::take(&mut update.footnotes_changed);
        let mut references_changed = std::mem::take(&mut update.references_changed);
        let mut tables_changed = std::mem::take(&mut update.tables_changed);
        let mut rows_changed = std::mem::take(&mut update.rows_changed);
        let mut changes = std::mem::take(&mut update.changes);
        let mut closed_containers = std::mem::take(&mut update.closed_containers);
        let mut loosened_lists = std::mem::take(&mut update.loosened_lists);
        dirty.clear();
        references_changed.clear();
        footnotes_changed.clear();
        tables_changed.clear();
        rows_changed.clear();
        changes.clear();
        closed_containers.clear();
        loosened_lists.clear();
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
            new_maths: self.maths.len()..self.maths.len(),
            new_footnotes: self.footnotes.entries.len()..self.footnotes.entries.len(),
            new_footnote_references: self.footnotes.references.len()
                ..self.footnotes.references.len(),
            footnotes_changed,
            new_containers: self.containers.len()..self.containers.len(),
            closed_containers,
            loosened_lists,
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
        update.new_maths.end = self.maths.len();
        update.new_footnotes.end = self.footnotes.entries.len();
        update.new_footnote_references.end = self.footnotes.references.len();
        update.new_containers.end = self.containers.len();
        update.container_end = self.containers.end;
        if update.base_revision != update.revision {
            update.changed_blocks = update
                .changed_blocks
                .start
                .min(update.new_blocks.start.saturating_sub(1))
                ..self.blocks.len();
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
        Effects::publishing(
            &self.source,
            &mut self.work,
            Publishing {
                annotations: &mut self.annotations,
                display: &mut self.display,
                references: &mut self.references,
                links: &mut self.links,
                link_info: &mut self.link_info,
                link_order: &mut self.link_order,
                direct_title_gaps: &mut self.direct_title_gaps,
                maths: &mut self.maths,
                math_gaps: &mut self.math_gaps,
                footnotes: &mut self.footnotes,
                update,
            },
        )
    }

    fn mark(&mut self, span: Span, style: Style, update: &mut Update) {
        self.effects(update).mark(span, style, None);
    }

    fn feed(&mut self, mut span: Span, update: &mut Update) {
        let root_paragraph = self.current.is_some_and(|id| {
            self.blocks[id.0].kind == BlockKind::Paragraph && self.blocks[id.0].container.is_none()
        });
        let mut indent = 0;
        // Strip only the bounded ASCII indentation of classified root prose.
        // A physical line enters here once, including deferred/failed table
        // headers; streaming suffixes never revisit its prefix. Containers,
        // code blocks, tabs and the four-space interruption dialect are separate.
        if root_paragraph && (span.start == 0 || self.source.as_bytes()[span.start - 1] == b'\n') {
            let spaces = self.source.as_bytes()[span.range()]
                .iter()
                .take(3)
                .take_while(|&&byte| byte == b' ')
                .count();
            // Mixed space/tab indentation retains the existing tab dialect.
            if self.source.as_bytes().get(span.start + spaces) != Some(&b'\t') {
                self.work.inline_bytes += spaces as u64;
                indent = spaces;
                self.mark(
                    Span::new(span.start, span.start + spaces),
                    Style::HIDDEN,
                    update,
                );
                span.start += spaces;
            }
        }
        if root_paragraph && (!span.is_empty() || indent > 0) {
            let key = table::ParagraphKey {
                first: self.current.unwrap(),
                scope: None,
            };
            self.inline.root_paragraph(key);
            let observer = self
                .root_eligibility
                .get_or_insert_with(RootEligibility::default);
            observer.bind_key(key);
            observer.feed(&self.source, span, indent, &mut self.work);
        }
        if let Some(container) = &mut self.container_eligibility {
            container
                .observer
                .feed(&self.source, span, 0, &mut self.work);
        }
        let mut inline = std::mem::take(&mut self.inline);
        inline.feed(span, &mut self.effects(update));
        self.inline = inline;
        self.check_container_eligibility();
    }

    fn check_container_eligibility(&self) {
        if let Some(container) = &self.container_eligibility {
            debug_assert!(self.root_eligibility.is_none());
            debug_assert_eq!(self.container_inline, Some(container.scope));
            debug_assert_eq!(
                self.blocks[container.first.0].container,
                Some(container.scope)
            );
            #[cfg(test)]
            if !self.inline.root_separator_seen() {
                container.observer.assert_same(&self.inline);
            }
        }
    }

    // Producers normalize their payload first. The tab's entire semantic group
    // is a separate event; no offset gap is interpreted as a structural prefix.
    fn feed_virtual(&mut self, at: usize, columns: u8, update: &mut Update) {
        if let Some(container) = &mut self.container_eligibility {
            container
                .observer
                .virtual_spaces(&self.source, at, columns, &mut self.work);
        }
        let mut inline = std::mem::take(&mut self.inline);
        inline.virtual_spaces(at, columns, &mut self.effects(update));
        self.inline = inline;
        self.check_container_eligibility();
    }

    fn assert_deferred_key(&self, key: table::ParagraphKey) {
        assert_eq!(
            self.inline.paragraph_key(),
            Some(key),
            "deferred publisher key"
        );
        let observer_key = if key.scope.is_some() {
            let active = self
                .container_eligibility
                .as_ref()
                .expect("deferred container observer");
            assert_eq!(
                table::ParagraphKey {
                    first: active.first,
                    scope: Some(active.scope)
                },
                key
            );
            active.observer.key()
        } else {
            self.root_eligibility
                .as_ref()
                .expect("deferred root observer")
                .key()
        };
        assert_eq!(observer_key, Some(key), "deferred observer key");
    }

    fn observe_header(&mut self, key: table::ParagraphKey, mut span: Span, spaces: u8) {
        self.assert_deferred_key(key);
        if key.scope.is_none() {
            assert_eq!(spaces, 0);
            self.observe_root(span);
        } else {
            let observer = &mut self.container_eligibility.as_mut().unwrap().observer;
            if spaces > 0 {
                observer.virtual_spaces(&self.source, span.start, spaces, &mut self.work);
                span.start += 1;
            }
            observer.feed(&self.source, span, 0, &mut self.work);
        }
    }

    // Only producers know which bytes were structural. A retained header may
    // release its already consumed prefix, so notify each physical source once.
    fn container_prefix(&mut self, source: Span, raw_start: usize) {
        if let Some(active) = &mut self.container_eligibility {
            if active.prefix_source == Some(source.start) {
                return;
            }
            debug_assert!(active.prefix_source.is_none_or(|old| old < source.start));
            active.prefix_source = Some(source.start);
            assert_eq!(
                self.inline.paragraph_key(),
                active.observer.key(),
                "prefix belongs to active ordinary sequence"
            );
            self.inline
                .component_prefix(Span::new(source.start, raw_start), &mut self.work);
        }
    }

    // Observe a deferred eager prefix with the same bounded indentation rule
    // as feed(), without taking the real parser's source consumption right.
    fn observe_root(&mut self, mut span: Span) {
        let mut indent = 0;
        if span.start == 0 || self.source.as_bytes()[span.start - 1] == b'\n' {
            let spaces = self.source.as_bytes()[span.range()]
                .iter()
                .take(3)
                .take_while(|&&byte| byte == b' ')
                .count();
            if self.source.as_bytes().get(span.start + spaces) != Some(&b'\t') {
                indent = spaces;
                span.start += spaces;
            }
        }
        self.root_eligibility
            .as_mut()
            .expect("deferred root header has an observer")
            .feed(&self.source, span, indent, &mut self.work);
    }

    fn isolate_header(&mut self, mode: table::HeaderMode) -> Option<inline::LineBreak> {
        if let Some(root) = &mut self.root_eligibility {
            root.isolate_line(&mut self.work);
        }
        if let Some(container) = &mut self.container_eligibility {
            container.observer.isolate_line(&mut self.work);
        }
        let line_break = match mode {
            table::HeaderMode::Ordinary => self.inline.isolate_line(),
            table::HeaderMode::Deferred(key) => {
                self.assert_deferred_key(key);
                None
            }
        };
        self.check_container_eligibility();
        line_break
    }

    fn can_isolate_header(&self) -> bool {
        self.check_container_eligibility();
        self.root_eligibility.as_ref().map_or_else(
            || {
                self.container_eligibility.as_ref().map_or_else(
                    || self.inline.can_isolate_line(),
                    |active| active.observer.can_isolate_line(),
                )
            },
            RootEligibility::can_isolate_line,
        )
    }

    fn header_crossed(&self) -> bool {
        self.check_container_eligibility();
        self.root_eligibility.as_ref().map_or_else(
            || {
                self.container_eligibility.as_ref().map_or_else(
                    || self.inline.header_crossed(),
                    |active| active.observer.header_crossed(),
                )
            },
            RootEligibility::header_crossed,
        )
    }

    fn cancel_header(&mut self, update: &mut Update) {
        if let Some(root) = &mut self.root_eligibility {
            root.cancel_header(&self.source, &mut self.work);
        }
        if let Some(container) = &mut self.container_eligibility {
            container
                .observer
                .cancel_header(&self.source, &mut self.work);
        }
        let mut inline = std::mem::take(&mut self.inline);
        inline.cancel_header(&mut self.effects(update));
        self.inline = inline;
        self.check_container_eligibility();
    }

    fn current_paragraph_tail(&self) -> Option<Span> {
        let id = self.current?;
        // Container leaves can be sealed while their logical paragraph stays
        // open. Its first leaf also detects a multi-line Setext promotion.
        let first = if self.blocks[id.0].container.is_some() {
            self.container_title?.0
        } else {
            id
        };
        if !matches!(
            self.blocks[first.0].kind,
            BlockKind::Paragraph | BlockKind::Quote { .. } | BlockKind::ListItem { .. }
        ) {
            return None;
        }
        self.inline
            .paragraph_tail(block::body_end(&self.source, self.blocks[id.0].source))
    }

    fn close_current(&mut self, update: &mut Update) {
        #[cfg(test)]
        let diverged = self.inline.root_separator_seen();
        let container = self.container_eligibility.take();
        if let Some(mut root) = self.root_eligibility.take() {
            root.finish(&self.source, &mut self.work);
        }
        if let Some(tail) = self.current_paragraph_tail() {
            self.mark(tail, Style::HIDDEN, update);
        }
        if let Some(id) = self.current {
            self.close_container_paragraph(self.blocks[id.0].source.end);
        }
        self.container_inline = None;
        self.container_title = None;
        let mut inline = std::mem::take(&mut self.inline);
        inline.finish(&mut self.effects(update));
        self.inline = inline;
        if let Some(mut container) = container {
            container.observer.finish(&self.source, &mut self.work);
            #[cfg(test)]
            if !diverged {
                container.observer.assert_same(&self.inline);
            }
        }
        if let Some(id) = self.current.take() {
            if matches!(self.blocks[id.0].kind, BlockKind::Math { .. }) {
                // Sealing can change an object consumer's layout even at EOF.
                // One range mutation, never a walk of its body or prefix gaps.
                self.mark(self.blocks[id.0].source, Style::default(), update);
            }
            if let BlockKind::FootnoteDefinition { symbol } = self.blocks[id.0].kind {
                self.footnotes
                    .seal(symbol, id, self.source.len(), update, &mut self.work);
            }
            self.blocks[id.0].state = SyntaxState::Sealed;
        }
    }

    fn new_block(&mut self, span: Span, kind: BlockKind, update: &mut Update) -> BlockId {
        self.close_current(update);
        self.push_block(span, kind, update)
    }

    fn new_container_text(&mut self, span: Span, kind: BlockKind, update: &mut Update) -> BlockId {
        if self.container_inline.is_some() && self.container_inline == self.containers.inner() {
            self.blocks[self.current.take().unwrap().0].state = SyntaxState::Sealed;
        } else {
            self.close_current(update);
        }
        self.container_inline = self.containers.inner();
        let first = self.push_block(span, kind, update);
        if self.container_eligibility.is_none() {
            self.container_eligibility = Some(ContainerEligibility {
                first,
                scope: self.container_inline.expect("ordinary container scope"),
                observer: Eligibility::container(),
                prefix_source: None,
            });
            let active = self.container_eligibility.as_mut().unwrap();
            let key = table::ParagraphKey {
                first,
                scope: Some(active.scope),
            };
            active.observer.bind_key(key);
            self.inline.container_paragraph(key);
        }
        self.check_container_eligibility();
        first
    }

    fn extend_container_title(&mut self, block: BlockId, content: Span) {
        if let Some((_, title)) = &mut self.container_title {
            title.end = content.end;
        } else {
            self.container_title = Some((block, content));
        }
    }

    fn push_block(&mut self, span: Span, kind: BlockKind, update: &mut Update) -> BlockId {
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

    fn probe_long_delimiter(&mut self, span: Span, update: &mut Update) -> bool {
        if self.definition_candidate.as_ref().is_some_and(|candidate| {
            candidate.title.is_some()
                || matches!(candidate.label, DefinitionLabel::Multiline { .. })
        }) {
            // The growing trigger hit its physical window. Replay before the
            // ordinary long-delimiter probe so it sees the last real header.
            self.flush_definition_candidate(update);
        }
        let Some(header) = &self.table_header else {
            return false;
        };
        let Some(start) =
            self.containers
                .delimiter_prefix(&self.source, span, header.container, &mut self.work)
        else {
            return false;
        };
        let mut delimiter = table::DelimiterCandidate::new(span, header.columns);
        if !delimiter.feed(&self.source[start..span.end], &mut self.work) {
            return false;
        }
        self.table_delimiter = Some(delimiter);
        true
    }

    fn finish_long_delimiter(
        &mut self,
        delimiter: table::DelimiterCandidate,
        end: usize,
        update: &mut Update,
    ) {
        let prefix = delimiter.prefix;
        if let Some(columns) = delimiter.finish() {
            let span = Span::new(prefix.start, end);
            self.containers.end = end;
            self.headings.parsed_end = end;
            let loosened_start = update.loosened_lists.len();
            self.containers
                .observe_payload(false, span.start, &mut self.work, update);
            for index in loosened_start..update.loosened_lists.len() {
                let list = self.containers.get(update.loosened_lists[index]).unwrap();
                self.mark(list.source, Style::default(), update);
            }
            let header = self.table_header.take().unwrap();
            self.confirm_table(header, span, columns, update);
            self.line_start = end;
        } else {
            self.reject_long_delimiter(prefix, end, true, update);
        }
    }

    fn reject_long_delimiter(
        &mut self,
        prefix: Span,
        end: usize,
        complete: bool,
        update: &mut Update,
    ) {
        // Reclassify only the original bounded prefix, using the same incomplete
        // line rules as before this extension. Consume the delayed suffix once;
        // it cannot become another header/probe or trigger a full-line retry.
        self.commit_line(prefix, false, false, update);
        let line = self.streaming.unwrap();
        if end > prefix.end {
            self.extend_line(line, Span::new(prefix.end, end), update);
        }
        if complete {
            self.end_line(line, update);
            self.streaming = None;
            self.line_start = end;
        }
    }

    fn flush_definition_candidate(&mut self, update: &mut Update) {
        if let Some(candidate) = self.definition_candidate.take() {
            if matches!(candidate.label, DefinitionLabel::PublishedTitle { .. }) {
                self.blocks[candidate.block.0].state = SyntaxState::Sealed;
                update.changed_blocks.start = update.changed_blocks.start.min(candidate.block.0);
                for fragment in candidate.sources.into_fragments() {
                    self.commit_line_mode(fragment.source, true, true, false, update);
                }
            } else {
                self.replay_deferred(candidate.sources, update);
            }
        }
    }

    // The candidate has already been taken. No fragment can reacquire definition
    // eligibility, and the triggering line is still owned by commit_line's caller.
    fn replay_deferred(&mut self, sources: DeferredSources, update: &mut Update) {
        let parsed_end = self.headings.parsed_end;
        let container_end = self.containers.end;
        let first = sources.block;
        debug_assert_eq!(self.blocks[first.0].container, sources.scope);
        let mut fragments = sources.into_fragments();
        let fragment = fragments.next().unwrap();
        if let Some(mut header) = self.table_header.take() {
            debug_assert_eq!(header.block, Some(first));
            debug_assert_eq!(header.source, fragment.source);
            debug_assert_eq!(header.parsed_end, header.content.start);
            debug_assert!(header.owner.is_none());
            // Restore the ordinary header's sole consumption right. Its next
            // line must classify with the same pending-header context as prose,
            // including tab/code/lazy priority; later replay may confirm it.
            header.owner = Some(fragment.owner);
            self.table_header = Some(header);
        } else {
            self.feed_fragment(fragment, update);
            if self.blocks[first.0].container.is_some() {
                self.end_container_text(first);
            }
        }
        self.mark(self.blocks[first.0].source, Style::default(), update);
        for fragment in fragments {
            // Reuse actual header/Setext/container/inline qualification. The
            // recorded raw span is for a future successful label consumer;
            // ordinary replay classifies each full physical source once.
            self.commit_line_mode(fragment.source, true, true, false, update);
        }
        self.containers.end = self.containers.end.max(container_end);
        self.headings.parsed_end = self.headings.parsed_end.max(parsed_end);
    }

    fn feed_fragment(&mut self, fragment: DeferredFragment, update: &mut Update) {
        self.container_prefix(fragment.source, fragment.raw.start);
        if fragment.virtual_spaces > 0 {
            self.feed_virtual(fragment.raw.start - 1, fragment.virtual_spaces, update);
        }
        self.feed(fragment.raw, update);
    }

    fn seal_definition_title(&mut self, update: &mut Update) -> Option<DefinitionTitleCandidate> {
        let candidate = self.definition_title_candidate.take()?;
        self.blocks[candidate.block.0].state = SyntaxState::Sealed;
        update.changed_blocks.start = update.changed_blocks.start.min(candidate.block.0);
        Some(candidate)
    }

    fn resolve_definition_title(
        &mut self,
        span: Span,
        complete: bool,
        update: &mut Update,
    ) -> bool {
        let Some(candidate) = self.seal_definition_title(update) else {
            return false;
        };
        let block = candidate.block;
        let prefix = complete
            .then(|| {
                self.containers.definition_prefix(
                    &self.source,
                    span,
                    self.blocks[block.0].container,
                    &mut self.work,
                )
            })
            .flatten();
        let title = prefix.and_then(|(start, _, _)| {
            let content = Span::new(start, span.end);
            self.work.classified_bytes += content.len() as u64;
            block::definition_title(&self.source, content)
        });
        let Some(title) = title else {
            if let Some((start, raw, virtual_spaces)) = prefix
                && let Some(parser) =
                    block::definition_open_title(&self.source, Span::new(start, span.end), true)
                && let Some(title) = self.definition_title_body(span, raw, parser)
            {
                self.blocks[block.0].state = SyntaxState::Growing;
                self.definition_candidate = Some(DefinitionCandidate {
                    block,
                    label: DefinitionLabel::PublishedTitle {
                        owner: candidate.owner,
                    },
                    sources: DeferredSources::new(
                        block,
                        self.blocks[block.0].container,
                        DeferredFragment {
                            source: span,
                            raw,
                            virtual_spaces,
                            owner: SourceOwner::new(),
                        },
                    ),
                    title: Some(title),
                });
                return true;
            }
            return false;
        };
        let loosened_start = update.loosened_lists.len();
        self.containers
            .observe_payload(false, span.start, &mut self.work, update);
        for index in loosened_start..update.loosened_lists.len() {
            let list = self.containers.get(update.loosened_lists[index]).unwrap();
            self.mark(list.source, Style::default(), update);
        }
        self.blocks[block.0].source.end = span.end;
        update.change(self.blocks[block.0].source, ChangeKind::Structure);
        self.mark(span, Style::HIDDEN, update);
        if let Some(symbol) = candidate.owner {
            self.references
                .define_title(symbol, title, Vec::new(), self.revision);
            update.references_changed.push(symbol);
            if let Some(start) = self.references.entries[symbol.0].first_use {
                update.change(Span::new(start, self.source.len()), ChangeKind::Reference);
            }
        }
        true
    }

    fn finish_definition(
        &mut self,
        block: BlockId,
        symbol: SymbolId,
        destination: Span,
        title: Option<Span>,
        gaps: Vec<Span>,
        update: &mut Update,
    ) {
        let winner = self
            .references
            .define(symbol, destination, title, gaps, self.revision);
        if winner {
            update.references_changed.push(symbol);
            if let Some(start) = self.references.entries[symbol.0].first_use {
                update.change(Span::new(start, self.source.len()), ChangeKind::Reference);
                update.change(Span::new(start, self.source.len()), ChangeKind::Visibility);
            }
        }
        self.close_current(update);
        if title.is_none() {
            // This growing definition has no paragraph/inline eligibility.
            self.blocks[block.0].state = SyntaxState::Growing;
            self.definition_title_candidate = Some(DefinitionTitleCandidate {
                block,
                owner: winner.then_some(symbol),
            });
        }
    }

    // Called only on a new bounded physical source. Earlier parser state and
    // summaries are retained; neither title bytes nor old ancestors are reread.
    fn definition_title_body(
        &mut self,
        span: Span,
        raw: Span,
        mut parser: destination::Destination,
    ) -> Option<DefinitionTitleBody> {
        for pos in block::body_end(&self.source, span)..span.end {
            self.work.classified_bytes += 1;
            if !matches!(
                parser.definition_title_byte(self.source.as_bytes()[pos], pos),
                destination::Progress::Pending
            ) {
                return None;
            }
        }
        let mut summary = table::HeaderSummary::default();
        summary.feed(&self.source[raw.range()], &mut self.work);
        Some(DefinitionTitleBody {
            parser,
            columns: summary.columns(),
            gaps: Vec::new(),
        })
    }

    fn retain_definition_title(
        &mut self,
        span: Span,
        raw: Span,
        virtual_spaces: u8,
        parser: destination::Destination,
        gaps: Vec<Span>,
    ) -> bool {
        let Some(mut title) = self.definition_title_body(span, raw, parser) else {
            return false;
        };
        let candidate = self.definition_candidate.as_mut().unwrap();
        if candidate
            .sources
            .push(DeferredFragment {
                source: span,
                raw,
                virtual_spaces,
                owner: SourceOwner::new(),
            })
            .is_err()
        {
            return false;
        }
        title.gaps = gaps;
        candidate.title = Some(title);
        true
    }

    fn resolve_definition_title_body(
        &mut self,
        span: Span,
        complete: bool,
        update: &mut Update,
    ) -> bool {
        let candidate = self.definition_candidate.as_ref().unwrap();
        let columns = candidate.title.as_ref().unwrap().columns;
        let scope = candidate.sources.scope;
        let continuation = complete
            .then(|| {
                self.containers.continuation_source(
                    &self.source,
                    span,
                    scope,
                    false,
                    columns,
                    &mut self.work,
                )
            })
            .flatten();
        let Some((_, raw, virtual_spaces)) = continuation else {
            self.flush_definition_candidate(update);
            return false;
        };
        let mut title = self
            .definition_candidate
            .as_mut()
            .unwrap()
            .title
            .take()
            .unwrap();
        let prefix = Span::new(span.start, raw.start);
        // Qualification already used residual tab columns. The consumed tab
        // belongs to this real prefix gap; its virtual columns are not title
        // bytes. Keep any real indentation after raw.start unchanged.
        title.parser.title_prefix(prefix);
        if !prefix.is_empty() {
            title.gaps.push(prefix);
        }
        let end = block::body_end(&self.source, span);
        for pos in raw.start..end {
            self.work.classified_bytes += 1;
            if !matches!(
                title
                    .parser
                    .definition_title_byte(self.source.as_bytes()[pos], pos),
                destination::Progress::Pending
            ) {
                self.flush_definition_candidate(update);
                return false;
            }
        }
        if let destination::Progress::Complete {
            destination,
            title: title_span,
        } = title.parser.finish_definition()
        {
            self.definition_candidate.as_mut().unwrap().title = Some(title);
            self.confirm_definition_candidate(span, destination, title_span, false, update);
            return true;
        }
        if self.retain_definition_title(span, raw, virtual_spaces, title.parser, title.gaps) {
            return true;
        }
        self.flush_definition_candidate(update);
        false
    }

    fn resolve_multiline_label(&mut self, span: Span, complete: bool, update: &mut Update) -> bool {
        let candidate = self.definition_candidate.as_ref().unwrap();
        let DefinitionLabel::Multiline {
            closed, columns, ..
        } = candidate.label
        else {
            unreachable!()
        };
        let scope = candidate.sources.scope;
        if !complete {
            self.flush_definition_candidate(update);
            return false;
        }
        if closed {
            // Check the real table path before the target-only indentation
            // exception. Partial-tab leaves reject four residual columns.
            let delimiter = columns.is_some_and(|columns| {
                self.containers
                    .reference_table_prefix(&self.source, span, scope, &mut self.work)
                    .is_some_and(|start| {
                        let content = Span::new(start, span.end);
                        self.work.classified_bytes += content.len() as u64;
                        table::delimiter(&self.source, content, columns).is_some()
                    })
            });
            if delimiter {
                self.flush_definition_candidate(update);
                return false;
            }
            return self.resolve_definition_candidate(span, complete, false, update);
        }
        // A previous pipe line is an ordinary pending header, including its
        // different tab/code qualification. Only the new physical prefix is read.
        let ordinary = self.containers.continuation_source(
            &self.source,
            span,
            scope,
            false,
            columns,
            &mut self.work,
        );
        let Some((_, raw, virtual_spaces)) = ordinary else {
            self.flush_definition_candidate(update);
            return false;
        };
        self.work.classified_bytes += raw.len() as u64;
        let bracket = block::unescaped_bracket(&self.source.as_bytes()[raw.range()]);
        let end = bracket.map_or(raw.end, |offset| raw.start + offset);
        let candidate = self.definition_candidate.as_mut().unwrap();
        let DefinitionLabel::Multiline {
            raw: label,
            closed,
            columns,
        } = &mut candidate.label
        else {
            unreachable!()
        };
        let valid = label.len() + end - raw.start <= MAX_LABEL_BYTES
            && bracket.is_none_or(|_| {
                self.source.as_bytes()[end] == b']'
                    && end + 1 < block::body_end(&self.source, raw)
                    && self.source.as_bytes()[end + 1] == b':'
            });
        if !valid {
            self.flush_definition_candidate(update);
            return false;
        }
        label.push_str(&self.source[raw.start..end]);
        let mut target = None;
        if bracket.is_some() {
            *closed = true;
            if !label.chars().any(|c| !c.is_whitespace()) {
                self.flush_definition_candidate(update);
                return false;
            }
            let suffix = Span::new(end + 2, block::body_end(&self.source, raw));
            if !self.source.as_bytes()[suffix.range()]
                .iter()
                .all(|b| matches!(b, b' ' | b'\t'))
            {
                target = block::definition_target(&self.source, suffix);
                if target.is_none() {
                    if let Some(parser) = block::definition_open_title(&self.source, suffix, false)
                        && self.retain_definition_title(
                            span,
                            raw,
                            virtual_spaces,
                            parser,
                            Vec::new(),
                        )
                    {
                        return true;
                    }
                    self.flush_definition_candidate(update);
                    return false;
                }
            }
        }
        if let Some((destination, title)) = target {
            self.confirm_definition_candidate(span, destination, title, false, update);
            return true;
        }
        let mut summary = table::HeaderSummary::default();
        summary.feed(&self.source[raw.range()], &mut self.work);
        *columns = summary.columns();
        let fragment = DeferredFragment {
            source: span,
            raw,
            virtual_spaces,
            owner: SourceOwner::new(),
        };
        if candidate.sources.push(fragment).is_err() {
            self.flush_definition_candidate(update);
            return false;
        }
        true
    }

    fn resolve_definition_candidate(
        &mut self,
        span: Span,
        complete: bool,
        payload_observed: bool,
        update: &mut Update,
    ) -> bool {
        let Some(candidate) = &self.definition_candidate else {
            return false;
        };
        let block = candidate.block;
        let prefix = complete
            .then(|| {
                self.containers.definition_prefix(
                    &self.source,
                    span,
                    self.blocks[block.0].container,
                    &mut self.work,
                )
            })
            .flatten();
        let target = prefix.and_then(|(start, _, _)| {
            let content = Span::new(start, span.end);
            self.work.classified_bytes += content.len() as u64;
            block::definition_target(&self.source, content)
        });
        let Some((destination, title)) = target else {
            if let Some((start, raw, virtual_spaces)) = prefix {
                let content = Span::new(start, span.end);
                self.work.classified_bytes += content.len() as u64;
                if let Some(parser) = block::definition_open_title(&self.source, content, false)
                    && self.retain_definition_title(span, raw, virtual_spaces, parser, Vec::new())
                {
                    return true;
                }
            }
            self.flush_definition_candidate(update);
            return false;
        };
        self.confirm_definition_candidate(span, destination, title, payload_observed, update);
        true
    }

    fn confirm_definition_candidate(
        &mut self,
        span: Span,
        destination: Span,
        title: Option<Span>,
        payload_observed: bool,
        update: &mut Update,
    ) {
        let candidate = self.definition_candidate.take().unwrap();
        self.table_header = None;
        let block = candidate.block;
        let retained = candidate.sources.confirm();
        let gaps = candidate.title.map_or_else(Vec::new, |body| body.gaps);
        if let DefinitionLabel::PublishedTitle { owner } = candidate.label {
            self.blocks[block.0].source.end = span.end;
            self.blocks[block.0].state = SyntaxState::Sealed;
            update.changed_blocks.start = update.changed_blocks.start.min(block.0);
            update.change(self.blocks[block.0].source, ChangeKind::Structure);
            self.mark(Span::new(retained.start, span.end), Style::HIDDEN, update);
            if let Some(symbol) = owner {
                self.references
                    .define_title(symbol, title.unwrap(), gaps, self.revision);
                update.references_changed.push(symbol);
                if let Some(start) = self.references.entries[symbol.0].first_use {
                    update.change(Span::new(start, self.source.len()), ChangeKind::Reference);
                }
            }
            return;
        }
        debug_assert_eq!(retained.start, self.blocks[block.0].source.start);
        let label = match &candidate.label {
            DefinitionLabel::PublishedTitle { .. } => unreachable!(),
            DefinitionLabel::Target(label) => &self.source[label.range()],
            DefinitionLabel::Multiline { raw, closed, .. } => {
                debug_assert!(closed);
                raw.as_str()
            }
        };
        let symbol = self
            .references
            .intern(label, &mut self.work)
            .expect("candidate raw label was validated before confirmation");
        // A container header has already observed this line in commit_line.
        if !payload_observed {
            let loosened_start = update.loosened_lists.len();
            self.containers
                .observe_payload(false, span.start, &mut self.work, update);
            for index in loosened_start..update.loosened_lists.len() {
                let list = self.containers.get(update.loosened_lists[index]).unwrap();
                self.mark(list.source, Style::default(), update);
            }
        }
        self.blocks[block.0].kind = BlockKind::Definition { symbol };
        self.blocks[block.0].source.end = span.end;
        update.changed_blocks.start = update.changed_blocks.start.min(block.0);
        let whole = self.blocks[block.0].source;
        update.change(whole, ChangeKind::Structure);
        self.mark(whole, Style::HIDDEN, update);
        self.finish_definition(block, symbol, destination, title, gaps, update);
    }

    fn commit_line(
        &mut self,
        span: Span,
        complete: bool,
        allow_long_header: bool,
        update: &mut Update,
    ) {
        self.commit_line_mode(span, complete, allow_long_header, true, update);
    }

    fn commit_line_mode(
        &mut self,
        span: Span,
        complete: bool,
        allow_long_header: bool,
        definitions: bool,
        update: &mut Update,
    ) {
        self.work.classified_bytes += span.len() as u64;
        self.containers.end = span.end;
        self.headings.parsed_end = span.end;
        if self
            .definition_candidate
            .as_ref()
            .is_some_and(|c| c.title.is_some())
            && self.resolve_definition_title_body(span, complete, update)
        {
            return;
        }
        if self.resolve_definition_title(span, complete, update) {
            return;
        }
        if self
            .definition_candidate
            .as_ref()
            .is_some_and(|candidate| matches!(candidate.label, DefinitionLabel::Multiline { .. }))
            && self.resolve_multiline_label(span, complete, update)
        {
            return;
        }
        // Joint definition/header candidates wait for the ordinary table
        // confirmation below, including its exact path and partial-leaf rules.
        if self.table_header.is_none()
            && self.resolve_definition_candidate(span, complete, false, update)
        {
            return;
        }
        if self.continue_footnote(span, complete, update) {
            return;
        }
        let mut header = self.table_header.take();
        if header.as_ref().is_some_and(|h| h.container.is_none()) {
            let root_header = header.take().unwrap();
            if complete
                && let Some(columns) = table::delimiter(&self.source, span, root_header.columns)
            {
                self.confirm_table(root_header, span, columns, update);
                return;
            }
            self.table_header = Some(root_header);
            if self.resolve_definition_candidate(span, complete, false, update) {
                return;
            }
            if let Some(root_header) = self.table_header.take() {
                self.feed_header(root_header, update);
            }
        }
        let previous_title = self.container_title.or_else(|| {
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
        let fence_scope = self
            .fence
            .and_then(|_| self.blocks[self.current.unwrap().0].container);
        let mut classification = block::classify(
            &self.source,
            span,
            complete,
            self.fence.filter(|_| fence_scope.is_none()),
            paragraph,
        );
        let mut content = span;
        let mut container_line = None;
        let mut container_blank = false;
        let mut virtual_spaces = 0;
        let mut fence_column = 0;
        let mut partial_leaf = false;
        let mut container_code = false;
        let mut lazy = false;
        let mut table_text = matches!(classification, Line::Text);
        if self.fence.is_none() || fence_scope.is_some() {
            // Root fences remain opaque. Scoped fences first match their
            // explicit path, allowing code blanks to omit list indentation.
            // Other missing prefixes or siblings seal the old block.
            let root_markers = matches!(
                classification,
                Line::Indented { .. }
                    | Line::Text
                    | Line::Blank
                    | Line::DefinitionTitle { .. }
                    | Line::DefinitionStart { .. }
                    | Line::DefinitionLabel { .. }
            );
            let new_start = self.containers.len();
            let closed_start = update.closed_containers.len();
            let loosened_start = update.loosened_lists.len();
            let fence_line = self.fence.and_then(|_| {
                fence_scope.and_then(|_| {
                    self.containers.partial_prefix(
                        &self.source,
                        span,
                        None,
                        false,
                        false,
                        &mut self.work,
                    )
                })
            });
            let partial_line = if self.fence.is_none() && self.containers.inner().is_some() {
                self.containers.partial_prefix(
                    &self.source,
                    span,
                    Some(complete),
                    header.is_none() && self.container_inline == self.containers.inner(),
                    previous_title.is_some_and(|(id, _)| {
                        self.blocks[id.0].container == self.containers.inner()
                    }),
                    &mut self.work,
                )
            } else {
                None
            };
            let partial_container = partial_line.as_ref().is_some_and(|line| line.new_container);
            let partial_line = partial_line.filter(|line| !line.new_container);
            let partial = partial_line.is_some();
            let line = fence_line.or(partial_line).or_else(|| {
                self.containers.line(
                    &self.source,
                    span,
                    containers::ContainerOptions {
                        partial_container,
                        complete,
                        root_markers,
                        root_paragraph: paragraph,
                        paragraph: previous_title.and_then(|(id, _)| self.blocks[id.0].container),
                        lazy_paragraph: self.container_inline,
                        table_header: header.as_ref().map(|h| (h.container.unwrap(), h.columns)),
                        fence: fence_scope,
                        code_fence: self.fence.is_some_and(|fence| fence.byte != b'$'),
                    },
                    &mut self.work,
                    update,
                )
            });
            self.sync_heading_scopes(new_start, closed_start, update);
            self.headings
                .context(self.containers.inner(), span.start, &mut self.work);
            if fence_scope.is_some()
                && (fence_scope != self.containers.inner()
                    || line.as_ref().is_some_and(|line| line.detached_blank))
            {
                self.fence = None;
                self.close_current(update);
            }
            if let Some(line) = line {
                lazy = line.lazy;
                container_code = line.indented;
                virtual_spaces = line.virtual_spaces;
                let same_scope = previous_title
                    .is_some_and(|(id, _)| self.blocks[id.0].container == self.containers.inner());
                let inner = Span::new(line.prefix, span.end);
                content = inner;
                let inner_classification = if line.indented {
                    Line::Indented {
                        prefix: line.prefix,
                    }
                } else if lazy {
                    Line::Text
                } else {
                    self.work.classified_bytes += inner.len() as u64;
                    // Plain leaves do not need physical columns. A possible
                    // tab-indented delimiter inspects at most four leading bytes;
                    // active code fences also need the column for body indent.
                    let tab_indent = self.source.as_bytes()[inner.range()]
                        .iter()
                        .take(4)
                        .take_while(|&&b| matches!(b, b' ' | b'\t'))
                        .any(|&b| b == b'\t');
                    if self.fence.is_some_and(|fence| fence.byte != b'$') || tab_indent {
                        // Only this bounded physical prefix, never old text or
                        // ancestor nodes. Reuse the result for body de-indent.
                        fence_column = block::prefix_column(
                            &self.source,
                            Span::new(span.start, inner.start),
                            &mut self.work,
                        );
                    }
                    if let Some(fence) = self.fence {
                        block::classify_fenced(
                            &self.source,
                            inner,
                            complete,
                            fence,
                            virtual_spaces,
                            fence_column,
                        )
                    } else {
                        block::classify_indented(
                            &self.source,
                            inner,
                            complete,
                            None,
                            same_scope,
                            virtual_spaces,
                            fence_column,
                        )
                    }
                };
                partial_leaf =
                    line.indented || (partial && !matches!(inner_classification, Line::Text));
                container_blank = matches!(inner_classification, Line::Blank);
                self.containers.observe_payload(
                    container_blank,
                    span.start,
                    &mut self.work,
                    update,
                );
                // A complete partial-tab path has the same table eligibility
                // as a byte-aligned path. Header fallback already preserves its
                // virtual spaces; confirmed cells trim leading whitespace.
                table_text = !lazy && matches!(inner_classification, Line::Text);
                classification = match inner_classification {
                    leaf @ (Line::Heading { .. }
                    | Line::Setext(_)
                    | Line::FenceOpen { .. }
                    | Line::FenceClose
                    | Line::Code
                    | Line::Indented { .. }
                    | Line::Rule
                    | Line::Definition { .. }
                    | Line::DefinitionTitle { .. }
                    | Line::DefinitionStart { .. }
                    | Line::DefinitionLabel { .. }
                    | Line::FootnoteDefinition { .. }) => leaf,
                    _ => Line::Container {
                        prefix: line.prefix,
                        kind: line.kind.clone(),
                    },
                };
                container_line = Some(line);
            }
            // A list's spacing is shared metadata. Invalidate its old source
            // interval once without touching any old item, block or tile.
            for index in loosened_start..update.loosened_lists.len() {
                let list = self.containers.get(update.loosened_lists[index]).unwrap();
                self.mark(list.source, Style::default(), update);
            }
        }
        if let Some(header) = header {
            if header.container == self.containers.inner()
                && !lazy
                // Four columns (including virtual ones) start literal code,
                // never a separator for the pending header in this extension.
                && !partial_leaf
                && complete
                && let Some(columns) = table::delimiter(&self.source, content, header.columns)
            {
                self.confirm_table(header, span, columns, update);
                return;
            }
            self.table_header = Some(header);
            if self.resolve_definition_candidate(span, complete, true, update) {
                return;
            }
            if let Some(header) = self.table_header.take() {
                self.feed_header(header, update);
            }
        }
        if let Some(block) = self.current
            && let BlockKind::Table { id } = self.blocks[block.0].kind
        {
            if table_text && self.blocks[block.0].container == self.containers.inner() {
                self.blocks[block.0].source.end = span.end;
                self.mark(span, Style::default(), update);
                self.start_table_row(id, span, content.start, complete, None, update);
                return;
            }
            self.close_current(update);
        }
        let mut candidate_title = None;
        if let Line::DefinitionTitle { label, parser } = classification {
            if definitions {
                candidate_title = self.definition_title_body(span, content, parser);
            }
            classification = if candidate_title.is_some() {
                Line::DefinitionStart { label }
            } else {
                container_line
                    .as_ref()
                    .map_or(Line::Text, |line| Line::Container {
                        prefix: line.prefix,
                        kind: line.kind.clone(),
                    })
            };
            table_text = true;
        }
        let mut candidate_label = match classification {
            Line::DefinitionStart { label } | Line::DefinitionLabel { label } => {
                let open = matches!(classification, Line::DefinitionLabel { .. });
                table_text = true;
                classification =
                    container_line
                        .as_ref()
                        .map_or(Line::Text, |line| Line::Container {
                            prefix: line.prefix,
                            kind: line.kind.clone(),
                        });
                if !definitions || (open && span.end - label.start > MAX_LABEL_BYTES) {
                    None
                } else if open {
                    Some(DefinitionLabel::Multiline {
                        raw: self.source[label.start..span.end].to_owned(),
                        closed: false,
                        columns: None,
                    })
                } else {
                    Some(DefinitionLabel::Target(label))
                }
            }
            _ => None,
        };
        let mut summary = table::HeaderSummary::default();
        if table_text && (complete || allow_long_header) {
            summary.feed(&self.source[content.range()], &mut self.work);
        }
        if let Some(DefinitionLabel::Multiline { columns, .. }) = &mut candidate_label {
            *columns = summary.columns();
        }
        // Emphasis openers and separable close-only pairs may be retained while
        // probing. Other inherited pairs cancel with their original semantics.
        // Inherited code/math and labels (including inert explicit tails) stay
        // isolated through an inert suffix after closure. Labels retain local effects; their link
        // objects and reference side effects wait for the header decision.
        let inherited = self
            .containers
            .inner()
            .map_or(paragraph, |scope| self.container_inline == Some(scope));
        // Only this branch retains the current root paragraph and its observer.
        // A new container closes both, even if the old root accepted a separator.
        let root_continuation = paragraph && self.containers.inner().is_none();
        let eager = table_text
            && !complete
            && allow_long_header
            && !summary.pipe
            && (!inherited || self.can_isolate_header());
        if eager || (summary.pipe && (!complete || summary.columns().is_some())) {
            let continuation_key = if root_continuation {
                self.root_eligibility.as_ref().and_then(Eligibility::key)
            } else if matches!(classification, Line::Container { .. }) && inherited {
                self.container_eligibility
                    .as_ref()
                    .and_then(|active| active.observer.key())
            } else {
                None
            };
            let mode = if let Some(key) = continuation_key
                && self.inline.root_separator_seen()
                && self.inline.paragraph_key() == Some(key)
            {
                self.assert_deferred_key(key);
                table::HeaderMode::Deferred(key)
            } else {
                table::HeaderMode::Ordinary
            };
            // An eager header moves the inline state into its first cell on
            // confirmation. Save the old paragraph's raw tail before replacing
            // its container leaf or consuming header bytes. Early container
            // headers also replace the leaf; failure leaves the tail provisional.
            let paragraph_tail = if inherited {
                self.current_paragraph_tail()
            } else {
                None
            };
            let leading_break = if eager && inherited {
                self.isolate_header(mode)
            } else {
                None
            };
            let block = if root_continuation {
                None
            } else {
                let kind = match classification {
                    Line::Container { kind, .. } => kind,
                    _ => BlockKind::Paragraph,
                };
                Some(if self.containers.inner().is_some() {
                    self.new_container_text(span, kind, update)
                } else {
                    self.new_block(span, kind, update)
                })
            };
            let paragraph_end = if eager && block.is_none() {
                let end = &mut self.blocks[self.current.unwrap().0].source.end;
                Some(std::mem::replace(end, span.end))
            } else {
                None
            };
            // Keep a split quote separator in the candidate's source envelope.
            // Table confirmation trims it as leading cell whitespace; paragraph
            // fallback must instead consume its semantic columns exactly once.
            if virtual_spaces > 0 {
                content.start -= 1;
                self.display
                    .spaces(Span::new(content.start, content.start + 1), virtual_spaces);
            }
            let mut header = table::HeaderCandidate {
                mode,
                block,
                source: span,
                content,
                container: self.containers.inner(),
                columns: summary.columns().unwrap_or(0),
                parsed_end: if eager && mode == table::HeaderMode::Ordinary {
                    content.end
                } else {
                    content.start
                },
                leading_break,
                paragraph_end,
                paragraph_tail,
                virtual_spaces,
                owner: Some(SourceOwner::new()),
            };
            self.mark(span, Style::default(), update);
            self.mark(Span::new(span.start, content.start), Style::HIDDEN, update);
            let end = block::body_end(&self.source, span);
            if header.container.is_some() {
                self.extend_container_title(block.unwrap(), Span::new(content.start, end));
            }
            if end + 2 == span.end {
                self.mark(Span::new(end, end + 1), Style::HIDDEN, update);
            }
            if eager {
                match mode {
                    table::HeaderMode::Ordinary => {
                        self.container_prefix(
                            span,
                            content.start + usize::from(virtual_spaces > 0),
                        );
                        self.feed_header_content(content, virtual_spaces, update);
                    }
                    table::HeaderMode::Deferred(key) => {
                        self.observe_header(key, content, virtual_spaces)
                    }
                }
            }
            if let Some(label) = candidate_label {
                self.definition_candidate = Some(DefinitionCandidate {
                    block: block.unwrap(),
                    label,
                    title: candidate_title,
                    sources: DeferredSources::new(
                        block.unwrap(),
                        header.container,
                        DeferredFragment::from_payload(
                            span,
                            header.content,
                            header.virtual_spaces,
                            header.owner.take().unwrap(),
                        ),
                    ),
                });
            }
            if self.header_crossed() {
                self.reject_streaming_header(header, update);
            } else if complete {
                self.table_header = Some(header);
            } else {
                self.streaming_header = Some(table::StreamingHeader { header, summary });
            }
            return;
        }
        if let Line::FootnoteDefinition { label, content } = classification {
            let symbol = self
                .footnotes
                .intern(&self.source[label.range()], &mut self.work);
            let id = self.new_block(span, BlockKind::FootnoteDefinition { symbol }, update);
            self.footnotes.define(
                symbol,
                FootnoteDefinition {
                    block: id,
                    content,
                    state: SyntaxState::Growing,
                },
                span,
                update,
                &mut self.work,
            );
            self.mark(span, Style::HIDDEN, update);
            let line = StreamingLine {
                block: id,
                inline: false,
                standalone: false,
                hidden: true,
                table: false,
            };
            if complete {
                self.end_line(line, update);
            } else {
                self.streaming = Some(line);
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
                let block = self.new_block(span, BlockKind::Definition { symbol }, update);
                self.mark(span, Style::HIDDEN, update);
                self.finish_definition(block, symbol, destination, title, Vec::new(), update);
                return;
            }
            // An invalid normalized label is ordinary text in its original
            // container, with the same prefix hiding and inline continuity.
            classification = container_line.map_or(Line::Text, |line| Line::Container {
                prefix: line.prefix,
                kind: line.kind,
            });
        }
        let body_end = block::body_end(&self.source, span);
        let mut payload = span;
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
                let trimmed = self
                    .inline
                    .setext_content()
                    .unwrap_or(Span::new(content.end, content.end));
                debug_assert!(content.start <= trimmed.start && trimmed.end <= content.end);
                self.mark(
                    Span::new(content.start, trimmed.start),
                    Style::HIDDEN,
                    update,
                );
                self.mark(Span::new(trimmed.end, content.end), Style::HIDDEN, update);
                self.blocks[id.0].kind = BlockKind::Heading { level };
                // Keep all source-ordered physical leaves and their IDs. Only
                // the first leaf becomes the opener; the last owns the underline.
                self.blocks[self.current.unwrap().0].source.end = span.end;
                let whole = Span::new(self.blocks[id.0].source.start, span.end);
                update.changed_blocks.start = update.changed_blocks.start.min(id.0);
                self.headings.push(
                    Heading {
                        block: id,
                        container: self.blocks[id.0].container,
                        level,
                        source: whole,
                        content: trimmed,
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
                let kind = if fence.byte == b'$' {
                    BlockKind::Math {
                        content: Span::new(span.end, span.end),
                        closed: false,
                    }
                } else {
                    BlockKind::Code { info }
                };
                let id = self.new_block(span, kind, update);
                self.fence = Some(fence);
                id
            }
            Line::FenceClose => {
                let id = self.current.unwrap();
                self.blocks[id.0].source.end = span.end;
                if let BlockKind::Math { closed, .. } = &mut self.blocks[id.0].kind {
                    *closed = true;
                }
                self.mark(span, Style::HIDDEN, update);
                self.fence = None;
                self.close_current(update);
                return;
            }
            Line::Code => {
                inline = false;
                let fence = self.fence.as_mut().unwrap();
                fence.remaining_indent = fence.indent;
                // Carry the classification column through streaming suffixes;
                // never revisit the prefix or ancestors after the window fills.
                fence.indent_column = fence_column;
                payload.start = self.strip_fence_indent(content, &mut virtual_spaces);
                if virtual_spaces > 0 {
                    payload.start -= 1;
                    self.display
                        .spaces(Span::new(payload.start, payload.start + 1), virtual_spaces);
                }
                let id = self.current.unwrap();
                self.blocks[id.0].source.end = span.end;
                if let BlockKind::Math { content: body, .. } = &mut self.blocks[id.0].kind {
                    body.end = span.end;
                    if payload.start > span.start {
                        self.math_gaps.push(Span::new(span.start, payload.start));
                    }
                    self.mark(self.blocks[id.0].source, Style::default(), update);
                }
                id
            }
            Line::Indented { prefix } => {
                inline = false;
                payload.start = prefix;
                if container_code && virtual_spaces > 0 {
                    payload.start -= 1;
                    self.display
                        .spaces(Span::new(payload.start, prefix), virtual_spaces);
                }
                virtual_spaces = 0;
                if let Some(id) = self.current
                    && matches!(self.blocks[id.0].kind, BlockKind::Code { .. })
                    && self.blocks[id.0].container == self.containers.inner()
                {
                    self.blocks[id.0].source.end = span.end;
                    id
                } else {
                    self.new_block(span, BlockKind::Code { info: None }, update)
                }
            }
            Line::Heading { level, prefix } => {
                standalone = true;
                payload.start = prefix;
                virtual_spaces = 0;
                let id = self.new_block(span, BlockKind::Heading { level }, update);
                self.atx = Some(block::Atx::new(prefix));
                id
            }
            Line::Container { prefix, kind } => {
                payload.start = prefix;
                if virtual_spaces > 0 {
                    payload.start -= 1;
                    self.display
                        .spaces(Span::new(payload.start, prefix), virtual_spaces);
                }
                let blank = container_blank;
                inline = !blank;
                container_text = !blank;
                standalone = blank;
                // Empty container leaves keep their scope and physical source.
                // Task items still publish their interactive marker on the leaf;
                // plain item numbers and markers remain in the container arena.
                let kind = if blank
                    && matches!(
                        kind,
                        BlockKind::Paragraph
                            | BlockKind::Quote { .. }
                            | BlockKind::ListItem { task: None, .. }
                    ) {
                    BlockKind::Blank
                } else {
                    kind
                };
                if blank {
                    self.new_block(span, kind, update)
                } else {
                    self.new_container_text(span, kind, update)
                }
            }
            Line::Rule => {
                standalone = true;
                inline = false;
                payload.start = content.start;
                self.new_block(span, BlockKind::ThematicBreak, update)
            }
            Line::Definition { .. }
            | Line::DefinitionTitle { .. }
            | Line::DefinitionStart { .. }
            | Line::DefinitionLabel { .. }
            | Line::FootnoteDefinition { .. } => unreachable!(),
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
        if body_end + 2 == span.end {
            self.mark(Span::new(body_end, body_end + 1), Style::HIDDEN, update);
        }
        if let Some(label) = candidate_label {
            self.definition_candidate = Some(DefinitionCandidate {
                block: id,
                label,
                title: candidate_title,
                sources: DeferredSources::new(
                    id,
                    self.blocks[id.0].container,
                    DeferredFragment::from_payload(
                        span,
                        payload,
                        virtual_spaces,
                        SourceOwner::new(),
                    ),
                ),
            });
            // Nothing has entered Inline yet. A failed candidate consumes this
            // source once before classifying its successor; success never does.
            inline = false;
        }
        if inline {
            self.container_prefix(span, payload.start + usize::from(virtual_spaces > 0));
            let mut fed = payload;
            if virtual_spaces > 0 {
                self.feed_virtual(payload.start, virtual_spaces, update);
                fed.start += 1;
            }
            if self.atx.is_some() {
                self.feed_atx(fed, update);
            } else {
                self.feed(fed, update);
            }
        }
        if container_text {
            self.extend_container_title(id, Span::new(payload.start, body_end));
        }
        if let BlockKind::Heading { level } = self.blocks[id.0].kind {
            self.headings.push(
                Heading {
                    block: id,
                    container: self.blocks[id.0].container,
                    level,
                    source: span,
                    content: Span::new(
                        self.atx
                            .as_ref()
                            .map_or(payload.start, |atx| atx.content.start),
                        body_end,
                    ),
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

    /// Consume only this line's outstanding indent, never old body or prefixes.
    /// Split only tabs needed for indentation; later tabs remain literal.
    fn strip_fence_indent(&mut self, span: Span, virtual_spaces: &mut u8) -> usize {
        let fence = self.fence.as_mut().unwrap();
        let removed = (*virtual_spaces).min(fence.remaining_indent);
        *virtual_spaces -= removed;
        fence.remaining_indent -= removed;
        let mut start = span.start;
        while fence.remaining_indent > 0 && start < span.end {
            self.work.classified_bytes += 1;
            let width = match self.source.as_bytes()[start] {
                b' ' => 1,
                b'\t' => 4 - fence.indent_column,
                _ => {
                    fence.remaining_indent = 0;
                    break;
                }
            };
            start += 1;
            fence.indent_column = (fence.indent_column + width) % 4;
            let removed = width.min(fence.remaining_indent);
            fence.remaining_indent -= removed;
            *virtual_spaces = width - removed;
        }
        start
    }

    fn extend_line(&mut self, line: StreamingLine, span: Span, update: &mut Update) {
        if !line.hidden && self.fence.is_some_and(|fence| fence.remaining_indent > 0) {
            let mut virtual_spaces = 0;
            let mut start = self.strip_fence_indent(span, &mut virtual_spaces);
            if virtual_spaces > 0 {
                start -= 1;
                self.display
                    .spaces(Span::new(start, start + 1), virtual_spaces);
            }
            self.mark(Span::new(span.start, start), Style::HIDDEN, update);
        }
        self.containers.end = span.end;
        self.headings.parsed_end = span.end;
        self.blocks[line.block.0].source.end = span.end;
        if let BlockKind::Math { content, .. } = &mut self.blocks[line.block.0].kind {
            content.end = span.end;
            self.mark(self.blocks[line.block.0].source, Style::default(), update);
        }
        if let BlockKind::FootnoteDefinition { symbol } = self.blocks[line.block.0].kind {
            self.footnotes.extend(
                symbol,
                line.block,
                block::body_end(&self.source, self.blocks[line.block.0].source),
                span,
                update,
                &mut self.work,
            );
        }
        if let Some((_, content)) = &mut self.container_title {
            content.end = block::body_end(&self.source, self.blocks[line.block.0].source);
        }
        if line.table {
            self.feed_table_row(span, update);
            return;
        }
        if line.hidden {
            self.mark(span, Style::HIDDEN, update);
        }
        if line.inline {
            if self.atx.is_some() {
                self.feed_atx(span, update);
                self.headings.extend(
                    line.block,
                    span.end,
                    Span::new(
                        self.atx.as_ref().unwrap().content.start,
                        block::body_end(&self.source, self.blocks[line.block.0].source),
                    ),
                );
            } else {
                self.feed(span, update);
            }
        }
        if self.source.as_bytes()[span.end - 1] == b'\n'
            && span.end >= 2
            && self.source.as_bytes()[span.end - 2] == b'\r'
        {
            self.mark(Span::new(span.end - 2, span.end - 1), Style::HIDDEN, update);
        }
    }

    fn end_line(&mut self, line: StreamingLine, update: &mut Update) {
        if let Some(mut atx) = self.atx.take() {
            let old_end = atx.content.end;
            atx.finish();
            self.feed(Span::new(old_end, atx.content.end), update);
            let source = self.blocks[line.block.0].source;
            let body_end = block::body_end(&self.source, source);
            self.mark(Span::new(atx.content.end, body_end), Style::HIDDEN, update);
            self.feed(Span::new(body_end, source.end), update);
            self.headings.extend(line.block, source.end, atx.content);
        }
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
        } else if line.inline && self.container_inline.is_some() {
            self.end_container_text(line.block);
        }
    }

    fn feed_atx(&mut self, span: Span, update: &mut Update) {
        let mut atx = self.atx.take().unwrap();
        let previous = atx.content;
        atx.feed(&self.source, span, &mut self.work);
        self.mark(
            Span::new(previous.start, atx.content.start),
            Style::HIDDEN,
            update,
        );
        self.feed(
            Span::new(previous.end.max(atx.content.start), atx.content.end),
            update,
        );
        self.atx = Some(atx);
    }

    fn end_container_text(&mut self, block: BlockId) {
        self.blocks[block.0].state = SyntaxState::Sealed;
    }

    fn flush_table_header(&mut self, update: &mut Update) {
        if let Some(header) = self.table_header.take() {
            self.feed_header(header, update);
        }
    }

    fn reject_streaming_header(&mut self, header: table::HeaderCandidate, update: &mut Update) {
        // A cross-line pair/object could not remain isolated.
        // Resume text once, including any suffix after a pipe in this chunk.
        self.cancel_header(update);
        if let Some(line_break) = header.leading_break {
            line_break.commit(&mut self.effects(update));
        }
        let block = self.current.unwrap();
        if header.container.is_some() {
            self.extend_container_title(block, header.content);
        }
        let line = StreamingLine {
            block,
            inline: true,
            standalone: false,
            hidden: false,
            table: false,
        };
        if let table::HeaderMode::Deferred(key) = header.mode {
            self.assert_deferred_key(key);
        }
        let _owner = header
            .owner
            .expect("rejected header owns its unconsumed source");
        self.container_prefix(
            header.source,
            header.content.start + usize::from(header.virtual_spaces > 0),
        );
        let mut remaining = Span::new(header.parsed_end, header.content.end);
        if header.parsed_end == header.content.start && header.virtual_spaces > 0 {
            self.feed_virtual(remaining.start, header.virtual_spaces, update);
            remaining.start += 1;
        }
        self.extend_line(line, remaining, update);
        if self.source.as_bytes()[header.source.end - 1] == b'\n' {
            self.end_line(line, update);
            self.line_start = header.source.end;
        } else {
            self.streaming = Some(line);
        }
    }

    fn finish_header(&mut self, mut candidate: table::StreamingHeader, update: &mut Update) {
        let end = block::body_end(&self.source, candidate.header.source);
        if candidate.header.container.is_some() {
            self.extend_container_title(
                candidate.header.block.unwrap(),
                Span::new(candidate.header.content.start, end),
            );
        }
        if end + 2 == candidate.header.source.end {
            self.mark(Span::new(end, end + 1), Style::HIDDEN, update);
        }
        if let Some(columns) = candidate.summary.columns() {
            candidate.header.columns = columns;
            self.table_header = Some(candidate.header);
        } else if !candidate.summary.pipe && candidate.header.mode == table::HeaderMode::Ordinary {
            // No pipe arrived: this was ordinary streaming text all along.
            // Do not add a whole-line annotation or revisit its semantics.
            self.cancel_header(update);
            if let Some(line_break) = candidate.header.leading_break {
                line_break.commit(&mut self.effects(update));
            }
            if candidate.header.container.is_some() {
                let block = candidate.header.block.unwrap();
                self.end_container_text(block);
            }
        } else {
            self.feed_header(candidate.header, update);
        }
    }

    fn feed_header(&mut self, mut header: table::HeaderCandidate, update: &mut Update) {
        if let Some(candidate) = self.definition_candidate.take() {
            debug_assert_eq!(header.block, Some(candidate.block));
            self.table_header = Some(header);
            self.replay_deferred(candidate.sources, update);
            self.flush_table_header(update);
        } else {
            let owner = header
                .owner
                .take()
                .expect("header owns its unconsumed source");
            self.feed_owned_header(header, owner, update);
        }
    }

    fn feed_owned_header(
        &mut self,
        header: table::HeaderCandidate,
        _owner: SourceOwner,
        update: &mut Update,
    ) {
        if let table::HeaderMode::Deferred(key) = header.mode {
            self.assert_deferred_key(key);
        }
        self.cancel_header(update);
        if let Some(line_break) = header.leading_break {
            line_break.commit(&mut self.effects(update));
        }
        if header.block.is_none() {
            self.blocks[self.current.unwrap().0].source.end = header.source.end;
        }
        self.mark(header.source, Style::default(), update);
        self.container_prefix(
            header.source,
            header.content.start + usize::from(header.virtual_spaces > 0),
        );
        self.feed_header_content(
            Span::new(header.parsed_end, header.content.end),
            if header.parsed_end == header.content.start {
                header.virtual_spaces
            } else {
                0
            },
            update,
        );
        if header.container.is_some() {
            self.end_container_text(header.block.unwrap());
        }
    }

    fn feed_header_content(&mut self, mut content: Span, spaces: u8, update: &mut Update) {
        if spaces > 0 {
            self.feed_virtual(content.start, spaces, update);
            content.start += 1;
        }
        self.feed(content, update);
    }

    fn confirm_table(
        &mut self,
        header: table::HeaderCandidate,
        delimiter: Span,
        columns: Vec<Alignment>,
        update: &mut Update,
    ) {
        if let table::HeaderMode::Deferred(key) = header.mode {
            self.assert_deferred_key(key);
        }
        if let Some(candidate) = self.definition_candidate.take() {
            debug_assert_eq!(header.block, Some(candidate.block));
            debug_assert!(header.owner.is_none());
            candidate.sources.confirm();
        } else {
            // The header's own right is retired by table confirmation.
            debug_assert!(header.owner.is_some());
        }
        let id = TableId(self.tables.len());
        let first_cell = if header.mode == table::HeaderMode::Ordinary
            && header.parsed_end > header.content.start
        {
            let mut inline = std::mem::take(&mut self.inline);
            let reuse = inline.confirm_header(&mut self.effects(update));
            if let Some(mut root) = self.root_eligibility.take() {
                let observed_reuse = root.confirm_header(&self.source, &mut self.work);
                debug_assert_eq!(reuse, observed_reuse);
            }
            if let Some(mut container) = self.container_eligibility.take() {
                let observed_reuse = container
                    .observer
                    .confirm_header(&self.source, &mut self.work);
                debug_assert_eq!(reuse, observed_reuse);
                #[cfg(test)]
                container.observer.assert_same(&inline);
            }
            if reuse {
                Some(inline)
            } else {
                self.inline = inline;
                None
            }
        } else {
            None
        };
        if let Some(end) = header.paragraph_end {
            self.blocks[self.current.unwrap().0].source.end = end;
        }
        if let Some(tail) = header.paragraph_tail {
            self.mark(tail, Style::HIDDEN, update);
        }
        let block = header
            .block
            .unwrap_or_else(|| self.new_block(header.source, BlockKind::Table { id }, update));
        if header.container.is_some() {
            // A late header's first-cell state was moved above. An early header
            // discards its inherited paragraph state before parsing the cells.
            self.close_container_paragraph(header.source.start);
            self.close_current(update);
            self.current = Some(block);
            self.blocks[block.0].state = SyntaxState::Growing;
        }
        self.blocks[block.0].kind = BlockKind::Table { id };
        self.blocks[block.0].source.end = delimiter.end;
        self.tables
            .push(Table::new(block, header.source, delimiter, columns));
        // RowParser still scans the header once for cell boundaries, whitespace
        // and escaped pipes (also inside code), skipping reusable semantics.
        // An inherited code/math prefix had no cell semantics; parse those here once.
        self.start_table_row(
            id,
            header.source,
            header.content.start,
            true,
            first_cell.map(|inline| (inline, header.parsed_end)),
            update,
        );
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
        parsed: Option<(Inline, usize)>,
        update: &mut Update,
    ) {
        let mut parser =
            table::RowParser::new(id, &mut self.tables[id.0], span.start, content_start);
        if let Some((inline, parsed_end)) = parsed {
            parser.resume_header(inline, parsed_end);
        }
        self.table_row = Some(parser);
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
    /// Number of confirmed emphasis pairs covering this source interval.
    /// Zero for unconfirmed candidates; not a complete nesting tree or order.
    pub emphasis_depth: usize,
    /// Number of confirmed strong pairs covering this source interval.
    pub strong_depth: usize,
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

/// A semantic iterator that returns its traversal storage on drop. Text borrows
/// only `'doc`; the exclusive scratch borrow ends when this iterator is dropped.
pub struct RunsWithScratch<'doc, 'scratch> {
    runs: Runs<'doc>,
    scratch: &'scratch mut QueryScratch,
}

impl RunsWithScratch<'_, '_> {
    pub fn annotation_visits(&self) -> u64 {
        self.runs.annotation_visits()
    }

    /// Pending frames, retained capacity and this query's allocation calls.
    pub fn scratch_stats(&self) -> ArenaStats {
        self.runs.scratch_stats()
    }
}

impl<'doc> Iterator for RunsWithScratch<'doc, '_> {
    type Item = RenderRun<'doc>;

    fn next(&mut self) -> Option<Self::Item> {
        self.runs.next()
    }
}

impl Drop for RunsWithScratch<'_, '_> {
    fn drop(&mut self) {
        self.runs.marks.recycle_into(self.scratch);
    }
}

impl Runs<'_> {
    /// Tree nodes traversed by this query; independent of parser mutation work.
    pub fn annotation_visits(&self) -> u64 {
        self.marks.visits
    }

    /// Pending annotation stack frames, retained Vec capacity bytes and actual
    /// allocation/growth calls for this iterator. Capacity never shrinks during
    /// iteration, so its final value is also this stack's peak. Excludes the
    /// document, allocator overhead and inline iterator state. Each new query
    /// owns a fresh stack unless constructed with caller scratch; this method
    /// does not allocate or traverse the tree.
    pub fn scratch_stats(&self) -> ArenaStats {
        self.marks.scratch_stats()
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
        let (span, mut first) = self.pending.take().or_else(|| self.marks.next())?;
        first.style =
            first.effective_style(&self.document.references.entries, &self.document.links);
        let mut end = span.end.min(limit);
        self.pending = (span.end > end).then_some((span, first));
        while end < limit {
            let (span, mut mark) = self.marks.next()?;
            mark.style =
                mark.effective_style(&self.document.references.entries, &self.document.links);
            if mark.style != first.style
                || mark.link != first.link
                || mark.emphasis_depth != first.emphasis_depth
                || mark.strong_depth != first.strong_depth
            {
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
        if block.is_some_and(|id| matches!(self.document.blocks[id.0].kind, BlockKind::Math { .. }))
        {
            style = style.union(Style::MATH);
        }
        Some(RenderRun {
            source: Span::new(start, end),
            text: &self.document.source[start..end],
            block,
            style,
            emphasis_depth: first.emphasis_depth,
            strong_depth: first.strong_depth,
            link: first.link,
        })
    }
}
