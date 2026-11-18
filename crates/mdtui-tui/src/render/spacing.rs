//! Reading-layout boundaries. Source syntax and logical paragraphs belong to
//! the core; this pass only consumes indexed metadata of projected objects.
use super::plan::{BlockPlan, GroupClass, PlanEvent, PlanEventKind, blank_lines_between};
use mdtui3::{
    BlockId, BlockKind, Budget, ContainerId, ContainerKind, Document, ListTightness, Span,
    WorkLimit,
};
use std::collections::{BTreeMap, HashMap};

#[derive(Clone, Copy)]
struct Object {
    block: BlockId,
    source: Span,
    container: Option<ContainerId>,
    paragraph: bool,
    class: GroupClass,
}
impl Object {
    fn block(doc: &Document, id: BlockId) -> Self {
        let block = doc.block(id).expect("planned block");
        let paragraph = doc.paragraph_at(block.source.start);
        Self {
            block: paragraph.map_or(id, |p| p.block),
            source: paragraph.map_or(block.source, |p| p.source),
            container: block.container,
            paragraph: paragraph.is_some(),
            class: match block.kind {
                BlockKind::Paragraph | BlockKind::Code { .. } | BlockKind::ThematicBreak => {
                    GroupClass::ParagraphLike
                }
                _ => GroupClass::Other,
            },
        }
    }
}

struct Boundary {
    next: Object,
    left: Option<ContainerId>,
    right: Option<ContainerId>,
    left_class: GroupClass,
    right_class: GroupClass,
    compact: bool,
    item_parent: Option<ContainerId>,
    left_node: Option<mdtui3::Container>,
    right_node: Option<mdtui3::Container>,
}
impl Boundary {
    fn new(previous: Object, next: Object) -> Self {
        Self {
            next,
            left: previous.container,
            right: next.container,
            left_class: previous.class,
            right_class: next.class,
            compact: previous.paragraph && next.paragraph,
            item_parent: None,
            left_node: None,
            right_node: None,
        }
    }
    fn advance(&mut self, doc: &Document, budget: &mut Budget, reads: &mut usize) -> Option<u16> {
        loop {
            if let Some(parent) = self.item_parent {
                if !pay(budget) {
                    return None;
                }
                *reads += 1;
                let list = doc.container(parent).expect("item parent");
                return Some(u16::from(
                    !self.compact || list.list_tightness != Some(ListTightness::Tight),
                ));
            }
            if self.left == self.right {
                let node = if let Some(id) = self.left {
                    if !pay(budget) {
                        return None;
                    }
                    *reads += 1;
                    Some(doc.container(id).expect("common ancestor"))
                } else {
                    None
                };
                return Some(match node {
                    None => blank_lines_between(self.left_class, self.right_class),
                    Some(c) => match c.kind {
                        ContainerKind::List { .. } => u16::from(
                            !self.compact || c.list_tightness != Some(ListTightness::Tight),
                        ),
                        ContainerKind::Item { .. } => {
                            self.item_parent = c.parent;
                            continue;
                        }
                        ContainerKind::Quote => 1,
                    },
                });
            }
            if let Some(id) = self.left
                && self.left_node.is_none()
            {
                if !pay(budget) {
                    return None;
                }
                *reads += 1;
                self.left_node = doc.container(id);
            }
            if let Some(id) = self.right
                && self.right_node.is_none()
            {
                if !pay(budget) {
                    return None;
                }
                *reads += 1;
                self.right_node = doc.container(id);
            }
            let take_left =
                self.left_node.map_or(0, |c| c.depth) >= self.right_node.map_or(0, |c| c.depth);
            let (node, cursor, class) = if take_left {
                (
                    self.left_node.take().expect("nonempty path"),
                    &mut self.left,
                    &mut self.left_class,
                )
            } else {
                (
                    self.right_node.take().expect("nonempty path"),
                    &mut self.right,
                    &mut self.right_class,
                )
            };
            if matches!(node.kind, ContainerKind::Quote) {
                self.compact = false;
            }
            if node.parent.is_none() {
                *class = if matches!(node.kind, ContainerKind::List { .. }) {
                    GroupClass::List
                } else {
                    GroupClass::Other
                };
            }
            *cursor = node.parent;
        }
    }
}

#[derive(Default)]
pub(crate) struct WindowSpacing {
    block: usize,
    event: usize,
    objects: BTreeMap<usize, Object>,
    flow: HashMap<BlockId, BlockId>,
    ordered: Vec<Object>,
    ready: bool,
    at: usize,
    boundary: Option<Boundary>,
    previous: Option<Object>,
    predecessor: Option<Option<BlockId>>,
    pub metadata_steps: usize,
    pub container_reads: usize,
}
impl WindowSpacing {
    pub fn advance(
        &mut self,
        doc: &Document,
        plan: &mut BlockPlan,
        budget: &mut Budget,
        top_only: bool,
    ) -> bool {
        if top_only {
            let Some(next) = plan.blocks.iter().find(|b| b.group.is_some()) else {
                return true;
            };
            if !next.group_start {
                return true;
            }
            if self.boundary.is_none() {
                if self.predecessor.is_none() {
                    let previous = match doc.previous_content_block(next.source.start, budget) {
                        Ok(previous) => previous,
                        Err(_) => return false,
                    };
                    self.metadata_steps += 1;
                    self.predecessor = Some(previous);
                }
                let Some(previous) = self.predecessor.unwrap() else {
                    return true;
                };
                if !pay(budget) {
                    return false;
                }
                self.metadata_steps += 1;
                let next = Object {
                    block: next.id,
                    source: next.source,
                    container: None,
                    paragraph: matches!(next.kind, BlockKind::Paragraph),
                    class: next.group_class.unwrap(),
                };
                self.boundary = Some(Boundary::new(Object::block(doc, previous), next));
            }
            let boundary = self.boundary.as_mut().unwrap();
            let Some(rows) = boundary.advance(doc, budget, &mut self.container_reads) else {
                return false;
            };
            if rows > 0 {
                plan.events.push(PlanEvent {
                    block: Some(next.id),
                    source: Span::new(next.source.start, next.source.start),
                    display_source: None,
                    kind: PlanEventKind::Spacing { blank_lines: rows },
                });
            }
            return true;
        }
        while self.block < plan.blocks.len() {
            if matches!(
                plan.blocks[self.block].kind,
                BlockKind::Blank
                    | BlockKind::Definition { .. }
                    | BlockKind::FootnoteDefinition { .. }
            ) {
                self.block += 1;
                continue;
            }
            if !budget.charge(WorkLimit {
                queries: 1,
                objects: 1,
                ..Default::default()
            }) {
                return false;
            }
            self.metadata_steps += 1;
            let planned = &plan.blocks[self.block];
            self.block += 1;
            let obj = Object::block(doc, planned.id);
            let owner = *self.flow.entry(obj.block).or_insert(planned.id);
            plan.flow_owners.insert(planned.id, owner);
            plan.flow_sources.insert(owner, obj.source);
            plan.flow_ids.insert(owner, obj.block);
            // Only the actual beginning owns a separator. A cropped continuation
            // or resumed fragment must not invent a boundary at its local start.
            self.objects.entry(obj.source.start).or_insert(obj);
        }
        while !self.ready && self.event < plan.events.len() {
            if !matches!(
                plan.events[self.event].kind,
                PlanEventKind::FoldLabel { .. }
            ) {
                self.event += 1;
                continue;
            }
            if !budget.charge(WorkLimit {
                queries: 1,
                objects: 1,
                ..Default::default()
            }) {
                return false;
            }
            self.metadata_steps += 1;
            let event = &plan.events[self.event];
            self.event += 1;
            if let Some(id) = event.block {
                let mut obj = Object::block(doc, id);
                obj.source = event.display_source.unwrap_or(event.source);
                // The PH is the visible replacement of its hidden interval.
                self.objects.entry(obj.source.start).or_insert(obj);
            }
        }
        if !self.ready {
            plan.events
                .retain(|e| !matches!(e.kind, PlanEventKind::Spacing { .. }));
            self.ordered = self.objects.values().copied().collect();
            self.ready = true;
        }
        while let Some(next) = self.ordered.get(self.at).copied() {
            if self.boundary.is_none() {
                if self.predecessor.is_none() {
                    let previous = match doc.previous_content_block(next.source.start, budget) {
                        Ok(previous) => previous,
                        Err(_) => return false,
                    };
                    self.metadata_steps += 1;
                    self.predecessor = Some(previous);
                }
                let Some(previous) = self.predecessor.unwrap() else {
                    self.predecessor = None;
                    self.previous = Some(next);
                    self.at += 1;
                    continue;
                };
                if !pay(budget) {
                    return false;
                }
                self.metadata_steps += 1;
                let mut previous = Object::block(doc, previous);
                self.predecessor = None;
                if let Some(projected) = self.previous
                    && projected.source.end >= previous.source.end
                {
                    previous = projected;
                }
                if previous.block == next.block
                    || doc
                        .tile_at(next.source.start)
                        .and_then(|tile| plan.segment(tile))
                        .is_none()
                {
                    self.previous = Some(next);
                    self.at += 1;
                    continue;
                }
                self.boundary = Some(Boundary::new(previous, next));
            }
            let boundary = self.boundary.as_mut().unwrap();
            let Some(rows) = boundary.advance(doc, budget, &mut self.container_reads) else {
                return false;
            };
            if rows > 0 {
                let at = boundary.next.source.start;
                plan.events.push(PlanEvent {
                    block: Some(boundary.next.block),
                    source: Span::new(at, at),
                    display_source: None,
                    kind: PlanEventKind::Spacing { blank_lines: rows },
                });
            }
            self.boundary = None;
            self.previous = Some(next);
            self.at += 1;
        }
        true
    }
}

fn pay(budget: &mut Budget) -> bool {
    budget.charge(WorkLimit {
        queries: 1,
        objects: 1,
        ..Default::default()
    })
}
