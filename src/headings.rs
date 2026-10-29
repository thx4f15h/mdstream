use crate::{BlockId, HeadingId, Span};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Heading {
    pub block: BlockId,
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
}

impl HeadingIndex {
    /// Innermost current section, including its heading's first byte.
    pub fn current(&self, offset: usize) -> Option<HeadingId> {
        self.entries
            .partition_point(|h| h.source.start <= offset)
            .checked_sub(1)
            .map(HeadingId)
    }

    pub fn parent(&self, id: HeadingId) -> Option<HeadingId> {
        let heading = self.get(id)?;
        self.previous(heading.source.start, heading.level - 1)
    }

    pub fn first_child(&self, id: HeadingId) -> Option<HeadingId> {
        let next = HeadingId(id.0.checked_add(1)?);
        (self.get(next)?.level > self.get(id)?.level).then_some(next)
    }

    /// First heading outside this section. None means an open tail at current EOF.
    pub fn section_end(&self, id: HeadingId) -> Option<HeadingId> {
        let heading = self.get(id)?;
        self.next(heading.source.start, heading.level)
    }

    pub fn section(&self, id: HeadingId, source_end: usize) -> Option<Span> {
        Some(Span::new(
            self.get(id)?.source.start,
            self.section_end(id)
                .map_or(source_end, |end| self.entries[end.0].source.start),
        ))
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
        let h = self.get(id)?;
        // A deeper heading is a child. The first non-descendant is the only candidate.
        let next = self.next(h.source.start, h.level)?;
        (self.parent(next) == parent).then_some(next)
    }

    pub fn previous_sibling(&self, id: HeadingId) -> Option<HeadingId> {
        let parent = self.parent(id);
        let lower = parent.map_or(0, |p| self.entries[p.0].level);
        let before = self.get(id)?.source.start;
        // At most six candidates: for each level, its last heading before us.
        // Its ancestors are bounded by the six Markdown heading levels.
        let mut candidate = self.previous(before, 6)?;
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
        self.levels
            .iter()
            .take(usize::from(max_level.min(6)))
            .filter_map(|ids| {
                let i = ids.partition_point(|id| self.entries[id.0].source.start <= offset);
                ids.get(i).copied()
            })
            .min()
    }

    pub fn previous(&self, offset: usize, max_level: u8) -> Option<HeadingId> {
        self.levels
            .iter()
            .take(usize::from(max_level.min(6)))
            .filter_map(|ids| {
                let i = ids.partition_point(|id| self.entries[id.0].source.start < offset);
                i.checked_sub(1).map(|i| ids[i])
            })
            .max()
    }

    pub(crate) fn push(&mut self, heading: Heading) -> HeadingId {
        let id = HeadingId(self.entries.len());
        self.levels[usize::from(heading.level - 1)].push(id);
        self.entries.push(heading);
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
