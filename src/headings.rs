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
