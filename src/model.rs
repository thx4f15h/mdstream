use std::ops::Range;

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
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reference {
    pub label: String,
    pub destination: Option<Span>,
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
}

/// Consume updates in order. Ranges invalidate caches without listing old tiles.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Update {
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
}

/// A tile cache key. Reference definitions invalidate lazily through a shared epoch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TileKey {
    pub source: Span,
    pub syntax_revision: u64,
    pub reference_revision: u64,
}
