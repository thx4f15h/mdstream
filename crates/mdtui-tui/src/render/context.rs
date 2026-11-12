use std::collections::HashMap;

use mdtui3::{Budget, Container, ContainerId, ContainerKind, Document, Span, WorkLimit};

#[derive(Debug, Clone, Copy)]
pub(crate) struct ContainerContext {
    pub root: ContainerId,
    pub root_kind: ContainerKind,
    pub indent: usize,
    total_indent: usize,
}

/// Resumable metadata-only ancestry preparation for the current source window.
/// Shared ancestors are visited once. No source or ancestor path is copied per
/// leaf block, and every node read/unwind consumes the caller's frame budget.
pub(crate) struct WindowContexts {
    targets: Vec<ContainerId>,
    next: usize,
    active: Option<ContainerId>,
    path: Vec<(ContainerId, Container)>,
    unwinding: bool,
    parent: Option<ContainerContext>,
    pub entries: HashMap<ContainerId, ContainerContext>,
    pub reads: usize,
    pub nodes: usize,
}

impl WindowContexts {
    pub fn new(document: &Document, source: Span) -> Self {
        let blocks = document.blocks();
        let start = blocks.partition_point(|block| block.source.end <= source.start);
        let end = blocks.partition_point(|block| block.source.start < source.end);
        let mut targets: Vec<_> = blocks[start..end]
            .iter()
            .filter_map(|block| block.container)
            .collect();
        targets.sort_unstable();
        targets.dedup();
        Self {
            targets,
            next: 0,
            active: None,
            path: Vec::new(),
            unwinding: false,
            parent: None,
            entries: HashMap::new(),
            reads: 0,
            nodes: 0,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.targets.is_empty()
    }

    pub fn advance(&mut self, document: &Document, budget: &mut Budget) -> bool {
        loop {
            if self.unwinding {
                let Some(&(id, node)) = self.path.last() else {
                    self.unwinding = false;
                    self.parent = None;
                    continue;
                };
                if !budget.charge(WorkLimit {
                    objects: 1,
                    ..WorkLimit::default()
                }) {
                    return false;
                }
                self.path.pop();
                let indent = self.parent.map_or(0, |parent| parent.total_indent);
                let own = match node.kind {
                    ContainerKind::Item { number, task } => {
                        if task.is_some() {
                            2
                        } else {
                            number.map_or(2, |n| decimal_digits(n) + 2)
                        }
                    }
                    _ => 0,
                };
                let context = ContainerContext {
                    root: self.parent.map_or(id, |parent| parent.root),
                    root_kind: self.parent.map_or(node.kind, |parent| parent.root_kind),
                    indent,
                    total_indent: indent.saturating_add(own),
                };
                self.entries.insert(id, context);
                self.parent = Some(context);
                self.nodes += 1;
                continue;
            }
            if let Some(id) = self.active {
                if let Some(context) = self.entries.get(&id).copied() {
                    self.parent = Some(context);
                    self.active = None;
                    self.unwinding = true;
                    continue;
                }
                if !budget.charge(WorkLimit {
                    queries: 1,
                    objects: 1,
                    ..WorkLimit::default()
                }) {
                    return false;
                }
                let node = document
                    .container(id)
                    .expect("window container belongs to document");
                self.reads += 1;
                self.path.push((id, node));
                self.active = node.parent;
                if node.parent.is_none() {
                    self.unwinding = true;
                }
                continue;
            }
            let Some(&id) = self.targets.get(self.next) else {
                return true;
            };
            self.next += 1;
            if !self.entries.contains_key(&id) {
                self.active = Some(id);
            }
        }
    }
}

fn decimal_digits(number: u32) -> usize {
    if number == 0 {
        1
    } else {
        number.ilog10() as usize + 1
    }
}
