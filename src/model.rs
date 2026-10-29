use std::ops::Range;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Process-local identity. IDs are never reused, including after an instance is dropped.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DocumentId(usize);

impl Default for DocumentId {
    fn default() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(1);
        Self(
            NEXT.try_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
                .expect("document identity space exhausted"),
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SyntaxState {
    Provisional,
    Growing,
    /// Local syntax is closed; reference destinations may still resolve later.
    Sealed,
}

/// A half-open UTF-8 byte range in the original, append-only source.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

impl Span {
    pub const fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }

    pub fn len(self) -> usize {
        self.end - self.start
    }

    pub fn is_empty(self) -> bool {
        self.start == self.end
    }

    pub fn range(self) -> Range<usize> {
        self.start..self.end
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BlockId(pub usize);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TileId(pub usize);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LinkId(pub usize);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SymbolId(pub usize);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TableId(pub usize);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct HeadingId(pub usize);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TaskMarker {
    pub checked: bool,
    pub source: Span,
}

/// Containers are deliberately flattened; quote and list markers describe one line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BlockKind {
    Paragraph,
    Heading {
        level: u8,
    },
    Quote {
        depth: usize,
    },
    ListItem {
        indent: usize,
        number: Option<u32>,
        task: Option<TaskMarker>,
    },
    Table {
        id: TableId,
    },
    Code {
        info: Option<Span>,
    },
    ThematicBreak,
    Blank,
    Definition {
        symbol: SymbolId,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Block {
    pub source: Span,
    pub kind: BlockKind,
    pub state: SyntaxState,
}

/// Independent flags. Hidden syntax stays addressable in the source.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Style(pub(crate) u8);

impl Style {
    pub const EMPHASIS: Self = Self(1);
    pub const STRONG: Self = Self(2);
    pub const STRIKE: Self = Self(4);
    pub const CODE: Self = Self(8);
    pub const HIDDEN: Self = Self(16);
    /// An explicit line break within continuing inline content (on the LF).
    pub const HARD_BREAK: Self = Self(32);
    /// An ordinary line break within continuing inline content (on the LF).
    pub const SOFT_BREAK: Self = Self(64);

    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkTarget {
    Direct(Span),
    /// Source address without the implied mailto: prefix. Use Document::link_url
    /// when a ready-to-open URL is needed.
    Email(Span),
    Reference(SymbolId),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkKind {
    Link,
    Image,
}

/// Images share destination resolution with links, while preserving their kind
/// and alt-text range. No image download or decoding is performed by the core.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LinkInfo {
    pub kind: LinkKind,
    pub label: Span,
    pub source: Span,
    /// Inline title without quotes; reference titles are resolved by Document.
    pub title: Option<Span>,
    pub state: SyntaxState,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reference {
    pub label: String,
    pub destination: Option<Span>,
    pub title: Option<Span>,
    pub revision: u64,
    pub first_use: Option<usize>,
}

/// Counters of actual parser work, useful for deterministic scaling tests.
/// Tree visits count mutation traversals; view queries do not mutate statistics.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Work {
    pub input_bytes: u64,
    pub classified_bytes: u64,
    pub inline_bytes: u64,
    pub delimiter_steps: u64,
    pub label_bytes: u64,
    pub tree_visits: u64,
    pub table_bytes: u64,
    pub autolink_bytes: u64,
    /// Bounded entity candidate bytes examined at semicolon; feed bytes are
    /// already included in inline_bytes.
    pub entity_bytes: u64,
}

/// Actual Vec arena capacity and allocation/reallocation calls. Allocator headers,
/// fragmentation and heap storage owned by user payloads are excluded.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ArenaStats {
    pub nodes: usize,
    pub capacity_bytes: usize,
    pub allocation_calls: u64,
}

/// Consume updates in order. Ranges invalidate caches without listing old tiles.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Update {
    pub document: Option<DocumentId>,
    pub base_revision: u64,
    pub revision: u64,
    pub appended: Span,
    pub new_blocks: Range<usize>,
    pub new_tiles: Range<usize>,
    pub dirty: Vec<Span>,
    pub references_changed: Vec<SymbolId>,
    pub new_headings: Range<usize>,
    pub new_tables: Range<usize>,
    /// Column width hints changed; invalidate layout of this table lazily.
    pub tables_changed: Vec<TableId>,
    pub new_links: Range<usize>,
    /// Conservative contiguous object deltas, including new objects. The last
    /// existing object can be reported even when its value did not change.
    pub changed_blocks: Range<usize>,
    pub changed_headings: Range<usize>,
    pub changed_links: Range<usize>,
    pub rows_changed: Vec<RowChange>,
    pub changes: Vec<Change>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChangeKind {
    Text,
    Visibility,
    Style,
    Structure,
    Reference,
    TableWidth,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Change {
    pub source: Span,
    pub kind: ChangeKind,
}

/// Facts come from the parser; a renderer decides whether these facts resize
/// content. Changing policy itself requires resetting the view's layout state.
#[derive(Clone, Copy, Debug)]
pub struct LayoutPolicy {
    pub style_affects_size: bool,
    pub references_affect_size: bool,
    pub table_width_affects_size: bool,
}

impl Default for LayoutPolicy {
    fn default() -> Self {
        Self {
            style_affects_size: true,
            references_affect_size: true,
            table_width_affects_size: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RowChange {
    pub table: TableId,
    pub rows: Range<usize>,
}

impl Update {
    pub(crate) fn layout_start(&self, policy: LayoutPolicy) -> Option<usize> {
        self.changes
            .iter()
            .filter(|change| match change.kind {
                ChangeKind::Style => policy.style_affects_size,
                ChangeKind::Reference => policy.references_affect_size,
                ChangeKind::TableWidth => policy.table_width_affects_size,
                _ => true,
            })
            .map(|change| change.source.start)
            .min()
    }
    pub(crate) fn validate(
        &self,
        document: &crate::Document,
        bound: Option<DocumentId>,
        revision: Option<u64>,
    ) -> Result<bool, crate::LayoutError> {
        if self.document != Some(document.id()) || bound.is_some_and(|id| id != document.id()) {
            return Err(crate::LayoutError::WrongDocument);
        }
        let previous = revision.unwrap_or(0);
        if self.revision != document.revision() || self.base_revision > self.revision {
            return Err(crate::LayoutError::OutOfOrderUpdate);
        }
        if previous == self.revision {
            return Ok(false);
        }
        if self.base_revision != previous {
            return Err(crate::LayoutError::OutOfOrderUpdate);
        }
        Ok(true)
    }
    pub(crate) fn change(&mut self, source: Span, kind: ChangeKind) {
        if let Some(last) = self.changes.last_mut()
            && last.kind == kind
            && source.start <= last.source.end
            && last.source.start <= source.end
        {
            last.source.start = last.source.start.min(source.start);
            last.source.end = last.source.end.max(source.end);
        } else if !source.is_empty() {
            self.changes.push(Change { source, kind });
        }
    }

    pub(crate) fn row_changed(&mut self, table: TableId, row: usize) {
        if let Some(last) = self.rows_changed.last_mut()
            && last.table == table
        {
            last.rows.start = last.rows.start.min(row);
            last.rows.end = last.rows.end.max(row + 1);
        } else {
            self.rows_changed.push(RowChange {
                table,
                rows: row..row + 1,
            });
        }
    }

    /// Move only incoming events; accumulated events are never cloned. Empty
    /// updates and repetition of the exact same interval are no-ops. Other
    /// overlaps and gaps are rejected.
    pub fn merge(&mut self, mut next: Self) -> Result<(), crate::LayoutError> {
        if self.document.is_none() {
            *self = next;
            return Ok(());
        }
        if self.document != next.document {
            return Err(crate::LayoutError::WrongDocument);
        }
        if self.base_revision == next.base_revision && self.revision == next.revision {
            return Ok(());
        }
        if self.revision != next.base_revision {
            return Err(crate::LayoutError::OutOfOrderUpdate);
        }
        if next.base_revision == next.revision {
            return Ok(());
        }
        self.revision = next.revision;
        self.appended.end = next.appended.end;
        self.new_blocks.end = next.new_blocks.end;
        self.new_tiles.end = next.new_tiles.end;
        self.new_links.end = next.new_links.end;
        self.new_headings.end = next.new_headings.end;
        self.new_tables.end = next.new_tables.end;
        for (old, new) in [
            (&mut self.changed_blocks, next.changed_blocks),
            (&mut self.changed_links, next.changed_links),
            (&mut self.changed_headings, next.changed_headings),
        ] {
            if !new.is_empty() {
                old.start = if old.start == old.end {
                    new.start
                } else {
                    old.start.min(new.start)
                };
                old.end = new.end;
            }
        }
        self.dirty.append(&mut next.dirty);
        self.references_changed.append(&mut next.references_changed);
        self.tables_changed.append(&mut next.tables_changed);
        self.rows_changed.append(&mut next.rows_changed);
        self.changes.append(&mut next.changes);
        Ok(())
    }
}

/// A tile cache key. Reference definitions invalidate lazily through a shared epoch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TileKey {
    pub document: DocumentId,
    pub source: Span,
    pub syntax_revision: u64,
    pub reference_revision: u64,
}
