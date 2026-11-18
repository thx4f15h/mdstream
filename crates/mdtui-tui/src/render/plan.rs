use std::collections::HashMap;
use std::fmt;

use mdtui3::{
    Block, BlockId, BlockKind, Budget, ContainerId, ContainerKind, Document, DocumentId, Span,
    TileId, WorkLimit,
};

use crate::render::code::{
    CodeError, MAX_INFO_SOURCE_BYTES, code_label_prefix, format_info_prefix,
};
use crate::render::context::ContainerContext;
use crate::render::thematic::{ThematicError, is_thematic_break_block};
use crate::render::visibility::is_hidden_block;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupClass {
    ParagraphLike,
    List,
    Other,
}

#[must_use]
pub const fn blank_lines_between(previous: GroupClass, next: GroupClass) -> u16 {
    match (previous, next) {
        (GroupClass::ParagraphLike, GroupClass::List)
        | (GroupClass::List, GroupClass::ParagraphLike)
        | (GroupClass::List, GroupClass::List) => 0,
        _ => 1,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GroupKey {
    Block(BlockId),
    Container(ContainerId),
}

pub(crate) fn group_identity(
    document: &Document,
    block_id: BlockId,
    block: &Block,
) -> Option<(GroupKey, GroupClass)> {
    let Some(container) = block.container else {
        return Some((GroupKey::Block(block_id), class_for_kind(&block.kind)));
    };
    let root = outermost_container(document, container)?;
    let class = match document.container(root)?.kind {
        ContainerKind::List { .. } => GroupClass::List,
        _ => GroupClass::Other,
    };
    Some((GroupKey::Container(root), class))
}

fn outermost_container(document: &Document, container: ContainerId) -> Option<ContainerId> {
    let budget = document.container_count().saturating_add(1);
    let mut current = container;
    for _ in 0..budget {
        let node = document.container(current)?;
        match node.parent {
            Some(parent) => current = parent,
            None => return Some(current),
        }
    }
    None
}

fn class_for_kind(kind: &BlockKind) -> GroupClass {
    match kind {
        BlockKind::Paragraph | BlockKind::Code { .. } | BlockKind::ThematicBreak => {
            GroupClass::ParagraphLike
        }
        _ => GroupClass::Other,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanError {
    InvalidRange,
    InvalidContainer(BlockId),
    Code(CodeError),
    Thematic(ThematicError),
}

impl fmt::Display for PlanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRange => formatter.write_str("plan range is not valid UTF-8 source"),
            Self::InvalidContainer(block) => {
                write!(formatter, "block {} has an invalid container", block.0)
            }
            Self::Code(error) => error.fmt(formatter),
            Self::Thematic(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for PlanError {}

impl From<CodeError> for PlanError {
    fn from(error: CodeError) -> Self {
        Self::Code(error)
    }
}

impl From<ThematicError> for PlanError {
    fn from(error: ThematicError) -> Self {
        Self::Thematic(error)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedBlock {
    pub id: BlockId,
    pub source: Span,
    pub kind: BlockKind,
    pub group: Option<usize>,
    pub group_start: bool,
    pub group_class: Option<GroupClass>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedGroup {
    pub index: usize,
    pub class: GroupClass,
    pub start: BlockId,
    pub blocks: Vec<BlockId>,
    pub spacing_before: u16,
    pub first_tile: TileId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanEventKind {
    Spacing { blank_lines: u16 },
    CodeLabel { label: String },
    FoldLabel { label: String, summary: Vec<String> },
    ThematicBreak,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanEvent {
    pub block: Option<BlockId>,
    pub source: Span,
    pub display_source: Option<Span>,
    pub kind: PlanEventKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TileSegmentKind {
    Drawable,
    TuiOwned,
    ZeroRow,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TileSegment {
    pub tile: TileId,
    pub source: Span,
    pub kind: TileSegmentKind,
    pub drawable_at: Option<usize>,
    pub blocks: Vec<BlockId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockPlan {
    document: DocumentId,
    pub revision: u64,
    pub visible: Span,
    pub tile_start: TileId,
    pub tile_end: TileId,
    pub blocks: Vec<PlannedBlock>,
    pub groups: Vec<PlannedGroup>,
    pub events: Vec<PlanEvent>,
    pub code_info_source_bytes: usize,
    pub unbudgeted_drawable_queries: usize,
    info_prefixes: Vec<InfoPrefix>,
    pub segments: Vec<TileSegment>,
    pub(crate) projected: bool,
    pub(crate) flow_owners: HashMap<BlockId, BlockId>,
    pub(crate) flow_sources: HashMap<BlockId, Span>,
    pub(crate) flow_ids: HashMap<BlockId, BlockId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct InfoPrefix {
    event: usize,
    source: Span,
    position: usize,
    text: String,
    truncated: bool,
}

impl BlockPlan {
    pub(crate) fn clip_info_prefixes(&mut self, contexts: &HashMap<usize, Span>) {
        for prefix in &mut self.info_prefixes {
            let end = contexts
                .values()
                .find(|context| {
                    context.start <= prefix.source.start && prefix.source.start < context.end
                })
                .map_or(prefix.source.start, |context| {
                    prefix.source.end.min(context.end)
                });
            if end < prefix.source.end {
                prefix.truncated = true;
                prefix.source.end = end;
            }
            if prefix.source.is_empty() {
                self.events[prefix.event].kind = PlanEventKind::CodeLabel {
                    label: "[code]".to_owned(),
                };
            }
        }
    }

    pub(crate) fn advance_info(&mut self, document: &Document, budget: &mut Budget) -> bool {
        for prefix in &mut self.info_prefixes {
            while prefix.position < prefix.source.end {
                let mut end = prefix
                    .position
                    .saturating_add(budget.remaining().bytes.min(mdtui3::TILE_BYTES))
                    .min(prefix.source.end);
                while end > prefix.position && !document.source().is_char_boundary(end) {
                    end -= 1;
                }
                if end == prefix.position
                    || !budget.charge(WorkLimit {
                        queries: 1,
                        objects: 1,
                        bytes: end - prefix.position,
                        ..WorkLimit::default()
                    })
                {
                    return false;
                }
                prefix
                    .text
                    .push_str(&document.source()[prefix.position..end]);
                self.code_info_source_bytes += end - prefix.position;
                prefix.position = end;
            }
            if !prefix.text.is_empty() {
                self.events[prefix.event].kind = format_info_prefix(&prefix.text, prefix.truncated)
                    .map_or(PlanEventKind::Spacing { blank_lines: 0 }, |label| {
                        PlanEventKind::CodeLabel { label }
                    });
                prefix.text.clear();
            }
        }
        true
    }

    #[must_use]
    pub fn matches(&self, document: &Document) -> bool {
        self.document == document.id() && self.revision == document.revision()
    }

    #[must_use]
    pub fn segment(&self, tile: TileId) -> Option<&TileSegment> {
        self.segments
            .binary_search_by_key(&tile, |segment| segment.tile)
            .ok()
            .and_then(|index| self.segments.get(index))
    }

    pub fn events_for_tile(&self, tile: TileId) -> impl Iterator<Item = &PlanEvent> + '_ {
        self.segment(tile).into_iter().flat_map(move |segment| {
            self.events.iter().filter(move |event| {
                event.source.start >= segment.source.start
                    && event.source.start < segment.source.end
            })
        })
    }

    #[must_use]
    pub fn total_blank_rows(&self) -> u16 {
        self.events
            .iter()
            .filter_map(|event| match event.kind {
                PlanEventKind::Spacing { blank_lines } => Some(blank_lines),
                _ => None,
            })
            .fold(0_u16, u16::saturating_add)
    }
}

pub fn plan_visible(
    document: &Document,
    visible: Span,
    overscan_tiles: usize,
) -> Result<BlockPlan, PlanError> {
    if document.text(visible).is_none() {
        return Err(PlanError::InvalidRange);
    }
    let visible_tiles = document.tiles_overlapping(visible);
    let start = visible_tiles.start.saturating_sub(overscan_tiles);
    let end = visible_tiles
        .end
        .saturating_add(overscan_tiles)
        .min(document.tile_count());
    plan_tiles(document, visible, start..end, None, false, None, false)
}

pub(crate) fn plan_with_contexts(
    document: &Document,
    visible: Span,
    contexts: &HashMap<ContainerId, ContainerContext>,
    promoted_code: Option<BlockId>,
) -> Result<BlockPlan, PlanError> {
    let tiles = document.tiles_overlapping(visible);
    plan_tiles(
        document,
        visible,
        tiles.start..tiles.end,
        Some(contexts),
        true,
        promoted_code,
        false,
    )
}

pub(crate) fn empty_plan(
    document: &Document,
    visible: Span,
    start: usize,
    end: usize,
) -> BlockPlan {
    let tile_start = TileId(start);
    let tile_end = TileId(end);
    BlockPlan {
        document: document.id(),
        revision: document.revision(),
        visible,
        tile_start,
        tile_end,
        blocks: Vec::new(),
        groups: Vec::new(),
        events: Vec::new(),
        code_info_source_bytes: 0,
        unbudgeted_drawable_queries: 0,
        info_prefixes: Vec::new(),
        projected: false,
        flow_owners: HashMap::new(),
        flow_sources: HashMap::new(),
        flow_ids: HashMap::new(),
        segments: (tile_start.0..tile_end.0)
            .map(|index| {
                let tile = TileId(index);
                let source = document.tile(tile).expect("planned tile exists");
                let drawable_at = Some(source.start);
                TileSegment {
                    tile,
                    source,
                    kind: TileSegmentKind::ZeroRow,
                    drawable_at,
                    blocks: Vec::new(),
                }
            })
            .collect(),
    }
}

fn plan_tiles(
    document: &Document,
    visible: Span,
    tiles: std::ops::Range<usize>,
    contexts: Option<&HashMap<ContainerId, ContainerContext>>,
    defer_info: bool,
    promoted_code: Option<BlockId>,
    precise: bool,
) -> Result<BlockPlan, PlanError> {
    let tile_start = TileId(tiles.start);
    let tile_end = TileId(tiles.end);
    let mut plan = empty_plan(document, visible, tile_start.0, tile_end.0);
    if !defer_info {
        for segment in &mut plan.segments {
            segment.drawable_at = document.next_drawable(segment.source.start);
            plan.unbudgeted_drawable_queries += 1;
        }
    }
    if tile_start.0 == tile_end.0 {
        return Ok(plan);
    }

    let source_start = document
        .tile(tile_start)
        .ok_or(PlanError::InvalidRange)?
        .start
        .max(if precise { visible.start } else { 0 });
    let source_end = document
        .tile(TileId(tile_end.0 - 1))
        .ok_or(PlanError::InvalidRange)?
        .end
        .min(if precise { visible.end } else { usize::MAX });
    let blocks = document.blocks();
    let block_start = blocks.partition_point(|block| block.source.end <= source_start);
    let block_end = blocks.partition_point(|block| block.source.start < source_end);

    let mut last_group_key = None;
    let mut last_group_class = None;
    for (index, block) in blocks[block_start..block_end].iter().enumerate() {
        let block_id = BlockId(block_start + index);
        let (group, group_start, group_class) =
            if is_hidden_block(block) || block.kind == BlockKind::Blank {
                (None, false, None)
            } else {
                let (key, class) = if let Some(contexts) = contexts {
                    match block.container {
                        Some(container) => {
                            let context = contexts
                                .get(&container)
                                .ok_or(PlanError::InvalidContainer(block_id))?;
                            let class = if matches!(context.root_kind, ContainerKind::List { .. }) {
                                GroupClass::List
                            } else {
                                GroupClass::Other
                            };
                            (GroupKey::Container(context.root), class)
                        }
                        None => (GroupKey::Block(block_id), class_for_kind(&block.kind)),
                    }
                } else {
                    group_identity(document, block_id, block)
                        .ok_or(PlanError::InvalidContainer(block_id))?
                };
                if last_group_key == Some(key) {
                    let group = plan.groups.len() - 1;
                    plan.groups[group].blocks.push(block_id);
                    (Some(group), false, Some(class))
                } else {
                    let spacing_before =
                        last_group_class.map_or(0, |previous| blank_lines_between(previous, class));
                    let group_start = if plan.groups.is_empty() {
                        group_starts_in_range(document, key, source_start)
                    } else {
                        true
                    };
                    let group = plan.groups.len();
                    plan.groups.push(PlannedGroup {
                        index: group,
                        class,
                        start: block_id,
                        blocks: vec![block_id],
                        spacing_before,
                        first_tile: document.tile_at(block.source.start).unwrap_or(tile_start),
                    });
                    last_group_key = Some(key);
                    last_group_class = Some(class);
                    (Some(group), group_start, Some(class))
                }
            };
        plan.blocks.push(PlannedBlock {
            id: block_id,
            source: block.source,
            kind: block.kind.clone(),
            group,
            group_start,
            group_class,
        });

        if group_start {
            let group = group.expect("visible group");
            let spacing = plan.groups[group].spacing_before;
            if spacing > 0 {
                plan.events.push(PlanEvent {
                    block: Some(block_id),
                    source: Span::new(block.source.start, block.source.start),
                    display_source: None,
                    kind: PlanEventKind::Spacing {
                        blank_lines: spacing,
                    },
                });
            }
        }
        if matches!(block.kind, BlockKind::Code { .. })
            && ((block.source.start >= source_start && block.source.start < source_end)
                || promoted_code == Some(block_id))
        {
            if defer_info {
                if let BlockKind::Code { info: Some(info) } = block.kind {
                    let mut end = info
                        .start
                        .saturating_add(MAX_INFO_SOURCE_BYTES)
                        .min(info.end);
                    while !document.source().is_char_boundary(end) {
                        end -= 1;
                    }
                    if end > info.start {
                        let event = plan.events.len();
                        plan.events.push(PlanEvent {
                            block: Some(block_id),
                            source: if promoted_code == Some(block_id) {
                                Span::new(source_start, source_start)
                            } else {
                                block.source
                            },
                            display_source: Some(block.source),
                            kind: PlanEventKind::CodeLabel {
                                label: "[…]".to_owned(),
                            },
                        });
                        plan.info_prefixes.push(InfoPrefix {
                            event,
                            source: Span::new(info.start, end),
                            position: info.start,
                            text: String::new(),
                            truncated: end < info.end,
                        });
                    }
                }
            } else {
                let (label, bytes) = code_label_prefix(document, block)?;
                plan.code_info_source_bytes = plan.code_info_source_bytes.saturating_add(bytes);
                if let Some(label) = label {
                    plan.events.push(PlanEvent {
                        block: Some(block_id),
                        source: block.source,
                        display_source: None,
                        kind: PlanEventKind::CodeLabel { label },
                    });
                }
            }
        }
        if block.kind == BlockKind::ThematicBreak
            || (!defer_info && is_thematic_break_block(document, block)?)
        {
            plan.events.push(PlanEvent {
                block: Some(block_id),
                source: block.source,
                display_source: None,
                kind: PlanEventKind::ThematicBreak,
            });
        }
    }

    for block in &plan.blocks {
        let overlap_start = block.source.start.max(source_start);
        let overlap_end = block.source.end.min(source_end);
        if overlap_start >= overlap_end {
            continue;
        }
        let block_tiles = document.tiles_overlapping(Span::new(overlap_start, overlap_end));
        for index in block_tiles.start.max(tile_start.0)..block_tiles.end.min(tile_end.0) {
            plan.segments[index - tile_start.0].blocks.push(block.id);
        }
    }

    for segment in &mut plan.segments {
        let drawable = segment
            .drawable_at
            .is_some_and(|position| position < segment.source.end);
        if drawable {
            segment.kind = TileSegmentKind::Drawable;
            continue;
        }
        let has_rows = plan.events.iter().any(|event| {
            event.source.start >= segment.source.start && event.source.start < segment.source.end
        });
        segment.kind = if has_rows {
            TileSegmentKind::TuiOwned
        } else {
            TileSegmentKind::ZeroRow
        };
    }

    Ok(plan)
}

fn group_starts_in_range(document: &Document, key: GroupKey, source_start: usize) -> bool {
    match key {
        GroupKey::Block(block) => document
            .block(block)
            .is_some_and(|block| block.source.start >= source_start),
        GroupKey::Container(container) => document
            .container(container)
            .is_some_and(|container| container.source.start >= source_start),
    }
}

/// Plan only projected text. The envelope is identity metadata, never a request
/// to visit the tiles or blocks between disconnected visible fragments.
pub(crate) fn projected_plan(
    document: &Document,
    visible: Span,
    sources: &[Span],
    placeholders: &[mdtui3::FoldPlaceholder],
    contexts: &HashMap<ContainerId, ContainerContext>,
) -> Result<BlockPlan, PlanError> {
    let mut result = empty_plan(document, visible, 0, 0);
    result.projected = true;
    let mut tiles = std::collections::BTreeMap::new();
    let mut blocks = std::collections::BTreeMap::new();
    for source in sources {
        let range = document.tiles_overlapping(*source);
        let mut part = plan_tiles(
            document,
            *source,
            range.start..range.end,
            Some(contexts),
            true,
            None,
            true,
        )?;
        let event_offset = result.events.len();
        for prefix in &mut part.info_prefixes {
            prefix.event += event_offset;
        }
        result.info_prefixes.extend(part.info_prefixes);
        result.events.extend(part.events);
        for block in part.blocks {
            blocks.entry(block.id).or_insert(block);
        }
        for segment in part.segments {
            tiles.entry(segment.tile).or_insert(segment);
        }
    }
    for placeholder in placeholders {
        let tile = placeholder.owner;
        tiles.entry(tile).or_insert_with(|| TileSegment {
            tile,
            source: document.tile(tile).expect("projection owner"),
            kind: TileSegmentKind::TuiOwned,
            drawable_at: None,
            blocks: Vec::new(),
        });
        let source = placeholder.candidate.hidden;
        if inline_placeholder(
            document,
            placeholder,
            &blocks.keys().copied().collect::<Vec<_>>(),
        ) {
            continue;
        }
        result.events.push(PlanEvent {
            block: match placeholder.candidate.id {
                mdtui3::FoldId::Heading(id) => document.headings().get(id).map(|h| h.block),
                _ => document.block_at(source.start),
            },
            source: Span::new(source.start, source.start),
            display_source: Some(source),
            kind: PlanEventKind::FoldLabel {
                label: placeholder_label(placeholder.candidate),
                summary: Vec::new(),
            },
        });
    }

    result.blocks = blocks.into_values().collect();
    result.segments = tiles.into_values().collect();
    result.tile_start = result.segments.first().map_or(TileId(0), |s| s.tile);
    result.tile_end = result
        .segments
        .last()
        .map_or(result.tile_start, |s| TileId(s.tile.0 + 1));
    Ok(result)
}

pub(crate) fn inline_placeholder(
    document: &Document,
    placeholder: &mdtui3::FoldPlaceholder,
    blocks: &[BlockId],
) -> bool {
    matches!(placeholder.candidate.id, mdtui3::FoldId::Source(_))
        && (document
            .block_at(placeholder.candidate.hidden.start)
            .is_none()
            && document.pending().start <= placeholder.candidate.hidden.start
            && placeholder.candidate.hidden.start < document.pending().end
            || document
                .block_at(placeholder.candidate.hidden.start)
                .is_some_and(|id| {
                    blocks.contains(&id)
                        && document.block(id).is_some_and(|b| {
                            matches!(
                                b.kind,
                                BlockKind::Paragraph
                                    | BlockKind::Quote { .. }
                                    | BlockKind::Code { .. }
                                    | BlockKind::ListItem { .. }
                            ) || match b.kind {
                                BlockKind::Table { id } => {
                                    document.table(id).is_some_and(|table| {
                                        table
                                            .row_at(placeholder.candidate.hidden.start)
                                            .and_then(|row| table.row(row))
                                            .is_some_and(|row| {
                                                row.cells.iter().any(|cell| {
                                                    cell.source.start
                                                        <= placeholder.candidate.hidden.start
                                                        && placeholder.candidate.hidden.start
                                                            < cell.source.end
                                                })
                                            })
                                    })
                                }
                                _ => false,
                            }
                        })
                }))
}

pub(crate) fn placeholder_label(candidate: mdtui3::FoldCandidate) -> String {
    let (kind, id) = match candidate.id {
        mdtui3::FoldId::Heading(id) => ("heading", id.0),
        mdtui3::FoldId::Code(id) => ("code", id.0),
        mdtui3::FoldId::Math(id) => ("math", id.0),
        mdtui3::FoldId::Container(id) => ("container", id.0),
        mdtui3::FoldId::Table(id) => ("table", id.0),
        mdtui3::FoldId::TableRows(range) => ("rows", range.start),
        mdtui3::FoldId::Source(id) => ("source", id.index()),
    };
    format!(
        "[+ {kind} {id} {}..{}]",
        candidate.source.start, candidate.source.end
    )
}
