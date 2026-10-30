use crate::{BlockId, ContainerId, HeadingId, Span, Work};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Heading {
    pub block: BlockId,
    /// Exact section scope. None is the document root.
    pub container: Option<ContainerId>,
    pub level: u8,
    pub source: Span,
    /// Title source without heading markers; query inline runs lazily for a TOC.
    pub content: Span,
}

/// Source-ordered heading arena plus six level-specific indexes. Title text is
/// never copied or flattened, including arbitrarily large Setext headings.
#[derive(Default)]
pub struct HeadingIndex {
    entries: Vec<Heading>,
    levels: [Vec<HeadingId>; 6],
    root: Scope,
    scopes: Vec<Scope>,
    contexts: Vec<(usize, Option<ContainerId>)>,
    pub(crate) parsed_end: usize,
}

#[derive(Default)]
struct Scope {
    levels: [Vec<HeadingId>; 6],
    inherited: Option<HeadingId>,
    end: Option<usize>,
}

impl HeadingIndex {
    /// Current section in the classified container context, falling back to the
    /// section inherited at container entry. O(log n), without walking ancestors.
    /// Pending source retains the last classified context until its line commits.
    pub fn current(&self, offset: usize) -> Option<HeadingId> {
        let scope = self
            .contexts
            .partition_point(|(start, _)| *start <= offset)
            .checked_sub(1)
            .and_then(|i| self.contexts[i].1);
        self.current_in(scope, offset)
    }

    fn scope(&self, container: Option<ContainerId>) -> &Scope {
        container.map_or(&self.root, |id| &self.scopes[id.0])
    }

    fn scope_mut(&mut self, container: Option<ContainerId>) -> &mut Scope {
        match container {
            Some(id) => &mut self.scopes[id.0],
            None => &mut self.root,
        }
    }

    fn current_in(&self, container: Option<ContainerId>, offset: usize) -> Option<HeadingId> {
        let scope = self.scope(container);
        scope
            .levels
            .iter()
            .filter_map(|ids| {
                ids.partition_point(|id| self.entries[id.0].source.start <= offset)
                    .checked_sub(1)
                    .map(|i| ids[i])
            })
            .max()
            .or(scope.inherited)
    }

    pub(crate) fn root_current(&self, offset: usize) -> Option<HeadingId> {
        self.current_in(None, offset)
    }

    /// Nearest shallower heading in the same exact container scope.
    pub fn parent(&self, id: HeadingId) -> Option<HeadingId> {
        let h = self.get(id)?;
        self.before(&self.scope(h.container).levels, h.source.start, h.level - 1)
    }

    pub fn first_child(&self, id: HeadingId) -> Option<HeadingId> {
        let h = self.get(id)?;
        let next = self.after(&self.scope(h.container).levels, h.source.start, 6)?;
        (self.entries[next.0].level > h.level).then_some(next)
    }

    /// Next same-or-shallower heading in the same scope. None can also mean
    /// the section ended at its container boundary; use section for byte ranges.
    pub fn section_end(&self, id: HeadingId) -> Option<HeadingId> {
        let h = self.get(id)?;
        self.after(&self.scope(h.container).levels, h.source.start, h.level)
    }

    /// Root tails use source_end. Container tails use the shared classified end
    /// while open, then their fixed boundary. Nested headings cannot close them.
    pub fn section(&self, id: HeadingId, source_end: usize) -> Option<Span> {
        let h = self.get(id)?;
        let end = self.section_end(id).map_or_else(
            || {
                if h.container.is_some() {
                    self.scope(h.container).end.unwrap_or(self.parsed_end)
                } else {
                    source_end
                }
            },
            |end| self.entries[end.0].source.start,
        );
        Some(Span::new(h.source.start, end))
    }

    pub fn next_at_level(&self, offset: usize, level: u8) -> Option<HeadingId> {
        let ids = self.at_level(level);
        ids.get(ids.partition_point(|id| self.entries[id.0].source.start <= offset))
            .copied()
    }

    pub fn previous_at_level(&self, offset: usize, level: u8) -> Option<HeadingId> {
        let ids = self.at_level(level);
        ids.partition_point(|id| self.entries[id.0].source.start < offset)
            .checked_sub(1)
            .map(|i| ids[i])
    }

    /// Siblings share a parent, but may have different levels when levels are skipped.
    pub fn next_sibling(&self, id: HeadingId) -> Option<HeadingId> {
        let parent = self.parent(id);
        // A deeper heading is a child. The first non-descendant is the only candidate.
        let next = self.section_end(id)?;
        (self.parent(next) == parent).then_some(next)
    }

    pub fn previous_sibling(&self, id: HeadingId) -> Option<HeadingId> {
        let parent = self.parent(id);
        let lower = parent.map_or(0, |p| self.entries[p.0].level);
        let h = self.get(id)?;
        let before = h.source.start;
        // At most six candidates: for each level, its last heading before us.
        // Its ancestors are bounded by the six Markdown heading levels.
        let mut candidate = self.before(&self.scope(h.container).levels, before, 6)?;
        while self.get(candidate)?.level > lower {
            if self.parent(candidate) == parent {
                return Some(candidate);
            }
            candidate = self.parent(candidate)?;
        }
        None
    }

    pub fn entries(&self) -> &[Heading] {
        &self.entries
    }
    pub fn get(&self, id: HeadingId) -> Option<&Heading> {
        self.entries.get(id.0)
    }
    pub fn at_level(&self, level: u8) -> &[HeadingId] {
        self.levels
            .get(level.wrapping_sub(1) as usize)
            .map_or(&[], Vec::as_slice)
    }

    /// Strictly after/before a source offset. Max level 2 selects h1 and h2.
    pub fn next(&self, offset: usize, max_level: u8) -> Option<HeadingId> {
        self.after(&self.levels, offset, max_level)
    }

    fn after(
        &self,
        levels: &[Vec<HeadingId>; 6],
        offset: usize,
        max_level: u8,
    ) -> Option<HeadingId> {
        levels
            .iter()
            .take(usize::from(max_level.min(6)))
            .filter_map(|ids| {
                let i = ids.partition_point(|id| self.entries[id.0].source.start <= offset);
                ids.get(i).copied()
            })
            .min()
    }

    pub fn previous(&self, offset: usize, max_level: u8) -> Option<HeadingId> {
        self.before(&self.levels, offset, max_level)
    }

    fn before(
        &self,
        levels: &[Vec<HeadingId>; 6],
        offset: usize,
        max_level: u8,
    ) -> Option<HeadingId> {
        levels
            .iter()
            .take(usize::from(max_level.min(6)))
            .filter_map(|ids| {
                let i = ids.partition_point(|id| self.entries[id.0].source.start < offset);
                i.checked_sub(1).map(|i| ids[i])
            })
            .max()
    }

    pub(crate) fn open_scope(&mut self, parent: Option<ContainerId>, work: &mut Work) {
        // Open events are in source order; only the six last IDs are needed.
        // Capture the outer section once, even when that scope has no heading.
        let scope = self.scope(parent);
        let inherited = scope
            .levels
            .iter()
            .filter_map(|ids| ids.last().copied())
            .max()
            .or(scope.inherited);
        self.scopes.push(Scope {
            inherited,
            ..Scope::default()
        });
        work.heading_steps += 1;
    }

    pub(crate) fn close_scope(&mut self, id: ContainerId, end: usize, work: &mut Work) {
        self.scopes[id.0].end = Some(end);
        work.heading_steps += 1;
    }

    pub(crate) fn context(
        &mut self,
        container: Option<ContainerId>,
        start: usize,
        work: &mut Work,
    ) {
        work.heading_steps += 1;
        if self.contexts.last().and_then(|(_, scope)| *scope) != container {
            self.contexts.push((start, container));
        }
    }

    pub(crate) fn push(&mut self, heading: Heading, work: &mut Work) -> HeadingId {
        let id = HeadingId(self.entries.len());
        self.levels[usize::from(heading.level - 1)].push(id);
        self.scope_mut(heading.container).levels[usize::from(heading.level - 1)].push(id);
        self.entries.push(heading);
        work.heading_steps += 1;
        id
    }

    pub(crate) fn extend(&mut self, block: BlockId, end: usize, content_end: usize) {
        if let Some(heading) = self.entries.last_mut()
            && heading.block == block
        {
            heading.source.end = end;
            heading.content.end = content_end;
        }
    }
}
