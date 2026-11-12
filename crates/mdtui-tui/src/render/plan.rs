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
    ThematicBreak,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanEvent {
    pub block: BlockId,
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
            .get(tile.0.checked_sub(self.tile_start.0)?)
            .filter(|segment| segment.tile == tile)
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
    plan_tiles(document, visible, start, end, None, false, None)
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
        tiles.start,
        tiles.end,
        Some(contexts),
        true,
        promoted_code,
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
    tile_start: usize,
    tile_end: usize,
    contexts: Option<&HashMap<ContainerId, ContainerContext>>,
    defer_info: bool,
    promoted_code: Option<BlockId>,
) -> Result<BlockPlan, PlanError> {
    let tile_start = TileId(tile_start);
    let tile_end = TileId(tile_end);
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
        .start;
    let source_end = document
        .tile(TileId(tile_end.0 - 1))
        .ok_or(PlanError::InvalidRange)?
        .end;
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
                    block: block_id,
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
                            block: block_id,
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
                        block: block_id,
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
                block: block_id,
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
