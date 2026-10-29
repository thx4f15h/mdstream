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

mod annotations;
mod block;
mod checkpoints;
mod headings;
mod inline;
mod layout;
mod model;
mod navigation;
mod references;
mod table;

pub use block::{BLOCK_LOOKAHEAD, MAX_FENCE_LENGTH};
pub use checkpoints::CheckpointIndex;
pub use headings::{Heading, HeadingIndex};
pub use inline::MAX_DESTINATION_BYTES;
pub use layout::{Anchor, HeightIndex, LayoutError, Measurement, VisibleTile};
pub use model::*;
pub use navigation::{NavAction, Navigator, ScrollTarget};
pub use references::MAX_LABEL_BYTES;
pub use table::{Alignment, Column, Table, TableCell, TableRow};

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
    source: String,
    tiles: Vec<Span>,
    blocks: Vec<Block>,
    links: Vec<LinkTarget>,
    link_info: Vec<LinkInfo>,
    headings: HeadingIndex,
    tables: Vec<Table>,
    table_header: Option<table::HeaderCandidate>,
    table_row: Option<table::RowParser>,
    references: References,
    annotations: Annotations,
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
    pub fn source(&self) -> &str {
        &self.source
    }
    pub fn blocks(&self) -> &[Block] {
        &self.blocks
    }
    pub fn block(&self, id: BlockId) -> Option<&Block> {
        self.blocks.get(id.0)
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
            LinkTarget::Direct(span) => *span,
            LinkTarget::Reference(symbol) => self.references.entries.get(symbol.0)?.destination?,
        };
        self.text(span)
    }

    pub fn tile_key(&self, id: TileId) -> Option<TileKey> {
        let source = self.tile(id)?;
        Some(TileKey {
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
        if self.finished {
            return Err(Finished);
        }
        let old_end = self.source.len();
        let mut update = self.begin_update(old_end);
        if text.is_empty() {
            return Ok(update);
        }
        self.revision += 1;
        update.revision = self.revision;
        self.source.push_str(text);
        self.extend_tiles(old_end);
        update.appended.end = self.source.len();
        update.dirty.push(update.appended);
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
                self.extend_line(line, Span::new(start, cursor), &mut update);
                if self.source.as_bytes()[cursor - 1] == b'\n' {
                    self.end_line(line, &mut update);
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
                    self.commit_line(Span::new(self.line_start, cursor), newline, &mut update);
                    if newline {
                        self.line_start = cursor;
                    }
                }
            }
        }
        self.complete_update(&mut update);
        Ok(update)
    }

    /// Seal EOF once. Unmatched constructs become literal without reparsing.
    /// Further calls are no-ops; append after EOF returns [`Finished`].
    pub fn finish(&mut self) -> Update {
        let mut update = self.begin_update(self.source.len());
        if self.finished {
            return update;
        }
        self.revision += 1;
        update.revision = self.revision;
        if let Some(line) = self.streaming.take() {
            self.end_line(line, &mut update);
        } else if self.line_start < self.source.len() {
            self.commit_line(
                Span::new(self.line_start, self.source.len()),
                true,
                &mut update,
            );
        }
        self.flush_table_header(&mut update);
        self.close_current(&mut update);
        self.line_start = self.source.len();
        self.fence = None;
        self.finished = true;
        self.complete_update(&mut update);
        update
    }

    fn begin_update(&self, start: usize) -> Update {
        Update {
            revision: self.revision,
            appended: Span::new(start, start),
            new_blocks: self.blocks.len()..self.blocks.len(),
            new_tiles: self.tiles.len()..self.tiles.len(),
            dirty: Vec::new(),
            references_changed: Vec::new(),
            new_headings: self.headings.entries().len()..self.headings.entries().len(),
            new_tables: self.tables.len()..self.tables.len(),
            tables_changed: Vec::new(),
        }
    }

    fn complete_update(&self, update: &mut Update) {
        update.new_blocks.end = self.blocks.len();
        update.new_tiles.end = self.tiles.len();
        update.new_headings.end = self.headings.entries().len();
        update.new_tables.end = self.tables.len();
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
        self.current = None;
    }

    fn new_block(&mut self, span: Span, kind: BlockKind, update: &mut Update) -> BlockId {
        self.close_current(update);
        let id = BlockId(self.blocks.len());
        self.blocks.push(Block { source: span, kind });
        self.current = Some(id);
        id
    }

    fn commit_line(&mut self, span: Span, complete: bool, update: &mut Update) {
        self.work.classified_bytes += span.len() as u64;
        if let Some(header) = self.table_header.take() {
            if complete && let Some(columns) = table::delimiter(&self.source, span, header.columns)
            {
                let id = TableId(self.tables.len());
                let block = header.block.unwrap_or_else(|| {
                    self.new_block(header.source, BlockKind::Table { id }, update)
                });
                self.blocks[block.0].kind = BlockKind::Table { id };
                self.blocks[block.0].source.end = span.end;
                self.tables
                    .push(Table::new(block, header.source, span, columns));
                self.start_table_row(id, header.source, true, update);
                self.tables[id.0].source.end = span.end;
                self.mark(
                    Span::new(header.source.start, span.end),
                    Style::default(),
                    update,
                );
                self.mark(span, Style::HIDDEN, update);
                return;
            }
            self.feed_header(header, update);
        }
        let paragraph = self
            .current
            .is_some_and(|id| self.blocks[id.0].kind == BlockKind::Paragraph);
        let mut classification =
            block::classify(&self.source, span, complete, self.fence, paragraph);
        if let Some(block) = self.current
            && let BlockKind::Table { id } = self.blocks[block.0].kind
        {
            if matches!(classification, Line::Text) {
                self.blocks[block.0].source.end = span.end;
                self.mark(span, Style::default(), update);
                self.start_table_row(id, span, complete, update);
                return;
            }
            self.close_current(update);
        }
        if complete
            && matches!(classification, Line::Text)
            && let Some(cells) = table::split(&self.source, span)
        {
            let block = if paragraph {
                None
            } else {
                Some(self.new_block(span, BlockKind::Paragraph, update))
            };
            self.table_header = Some(table::HeaderCandidate {
                block,
                source: span,
                columns: cells.len(),
            });
            self.mark(span, Style::default(), update);
            let end = block::body_end(&self.source, span);
            if end + 2 == span.end {
                self.mark(Span::new(end, end + 1), Style::HIDDEN, update);
            }
            return;
        }
        if let Line::Definition { label, destination } = classification {
            if let Some(symbol) = self
                .references
                .intern(&self.source[label.range()], &mut self.work)
            {
                self.new_block(span, BlockKind::Definition { symbol }, update);
                self.mark(span, Style::HIDDEN, update);
                if self.references.define(symbol, destination, self.revision) {
                    update.references_changed.push(symbol);
                }
                self.current = None;
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
                let id = self.current.unwrap();
                let content = Span::new(
                    self.blocks[id.0].source.start,
                    block::body_end(&self.source, self.blocks[id.0].source),
                );
                self.blocks[id.0].kind = BlockKind::Heading { level };
                self.blocks[id.0].source.end = span.end;
                let whole = self.blocks[id.0].source;
                self.headings.push(Heading {
                    block: id,
                    level,
                    source: whole,
                    content,
                });
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
            Line::Quote { depth, prefix } => {
                standalone = true;
                payload.start = prefix;
                self.new_block(span, BlockKind::Quote { depth }, update)
            }
            Line::List {
                indent,
                number,
                prefix,
                task,
            } => {
                standalone = true;
                payload.start = prefix;
                self.new_block(
                    span,
                    BlockKind::ListItem {
                        indent,
                        number,
                        task,
                    },
                    update,
                )
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
        if let BlockKind::Heading { level } = self.blocks[id.0].kind {
            self.headings.push(Heading {
                block: id,
                level,
                source: span,
                content: Span::new(payload.start, payload.end.min(body_end)),
            });
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
        self.blocks[line.block.0].source.end = span.end;
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
        self.feed(header.source, update);
    }

    fn start_table_row(&mut self, id: TableId, span: Span, complete: bool, update: &mut Update) {
        self.table_row = Some(table::RowParser::new(
            id,
            &mut self.tables[id.0],
            span.start,
        ));
        self.feed_table_row(span, update);
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
