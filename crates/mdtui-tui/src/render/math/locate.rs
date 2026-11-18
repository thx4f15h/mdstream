//! Source-order primitive seek followed by paid parent-edge projection.
use super::layout::{IndexKind, Layout, Primitive};
use mdtui3::{Budget, Span, WorkLimit};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MathPosition {
    pub row: usize,
    pub column: usize,
    pub source: Span,
    pub cell_source: Span,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LocateStep {
    Progress,
    Position(MathPosition),
    BudgetExhausted,
}
pub(super) struct Locator {
    goal: Span,
    owner: Option<usize>,
    x: usize,
    y: usize,
    cell_source: Span,
    done: Option<MathPosition>,
    pub steps: usize,
}
impl Locator {
    pub fn new(goal: Span) -> Self {
        Self {
            goal,
            owner: None,
            x: 0,
            y: 0,
            cell_source: goal,
            done: None,
            steps: 0,
        }
    }
    pub fn step(&mut self, l: &Layout, b: &mut Budget) -> LocateStep {
        if !b.charge(WorkLimit {
            queries: 1,
            objects: 1,
            ..WorkLimit::default()
        }) {
            return LocateStep::BudgetExhausted;
        }
        self.steps += 1;
        if let Some(position) = self.done {
            return LocateStep::Position(position);
        }
        let root = l.root.unwrap();
        if l.boxes[root].index == IndexKind::Small {
            let position = MathPosition {
                row: 0,
                column: 0,
                source: self.goal,
                cell_source: Span::new(0, 0),
            };
            self.done = Some(position);
            return LocateStep::Position(position);
        }
        if let Some(owner) = self.owner {
            if owner == root {
                let position = MathPosition {
                    row: self.y,
                    column: self.x,
                    source: self.goal,
                    cell_source: self.cell_source,
                };
                self.done = Some(position);
                return LocateStep::Position(position);
            }
            if let Some(&(parent, edge)) = l.parents.get(&owner)
                && l.links[edge].attached
            {
                let p = l.placed[l.links[edge].child];
                self.x += p.x;
                self.y += p.y.unwrap_or_else(|| {
                    l.boxes[parent].metrics.baseline - l.boxes[owner].metrics.baseline
                });
                self.owner = Some(parent);
                return LocateStep::Progress;
            }
            // A syntactically incomplete atom has not joined the visible prefix yet.
            let position = MathPosition {
                row: 0,
                column: 0,
                source: self.goal,
                cell_source: self.goal,
            };
            self.done = Some(position);
            return LocateStep::Position(position);
        }
        if let Some((&(_, owner, at), &source)) = l
            .source_index
            .range(..=(self.goal.start, usize::MAX, usize::MAX))
            .next_back()
        {
            let primitive = l.boxes[owner].primitives[at].unwrap();
            let (x, y) = match primitive {
                Primitive::Text { x, y, .. }
                | Primitive::Rule { x, y, .. }
                | Primitive::Fence { x, y, .. } => (x, y),
            };
            self.x = x;
            self.y = y;
            self.cell_source = source;
            self.owner = Some(owner);
            LocateStep::Progress
        } else {
            let position = MathPosition {
                row: 0,
                column: 0,
                source: self.goal,
                cell_source: self.goal,
            };
            self.done = Some(position);
            LocateStep::Position(position)
        }
    }
}
