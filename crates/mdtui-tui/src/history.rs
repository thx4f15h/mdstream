#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HistoryPosition {
    pub source: usize,
    pub row_offset: u64,
}

/// A navigation address whose document/body cannot be mistaken for root bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SurfacePosition {
    Root {
        document: mdtui3::DocumentId,
        position: HistoryPosition,
    },
    Body {
        source: mdtui3::BodySpan,
        row_offset: u64,
    },
}
impl SurfacePosition {
    pub fn document(self) -> mdtui3::DocumentId {
        match self {
            Self::Root { document, .. } => document,
            Self::Body { source, .. } => source.body.document(),
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Entry {
    Root(HistoryPosition),
    Body {
        source: mdtui3::BodySpan,
        row_offset: u64,
    },
}
impl Entry {
    fn from_surface(position: SurfacePosition) -> Self {
        match position {
            SurfacePosition::Root { position, .. } => Self::Root(position),
            SurfacePosition::Body { source, row_offset } => Self::Body { source, row_offset },
        }
    }
    fn surface(self, document: mdtui3::DocumentId) -> SurfacePosition {
        match self {
            Self::Root(position) => SurfacePosition::Root { document, position },
            Self::Body { source, row_offset } => SurfacePosition::Body { source, row_offset },
        }
    }
    fn root(self) -> Option<HistoryPosition> {
        match self {
            Self::Root(p) => Some(p),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HistoryState {
    entries: Vec<Entry>,
    current: usize,
    document: Option<mdtui3::DocumentId>,
}

impl HistoryState {
    pub fn bind(&mut self, document: mdtui3::DocumentId) -> Result<(), mdtui3::LayoutError> {
        if self.document.is_some_and(|old| old != document) {
            return Err(mdtui3::LayoutError::WrongDocument);
        }
        self.document = Some(document);
        Ok(())
    }
    pub fn current_surface(&self) -> Option<SurfacePosition> {
        Some(self.entries.get(self.current)?.surface(self.document?))
    }
    pub fn peek_back_surface(&self) -> Option<SurfacePosition> {
        Some(
            self.entries
                .get(self.current.checked_sub(1)?)?
                .surface(self.document?),
        )
    }
    pub fn peek_forward_surface(&self) -> Option<SurfacePosition> {
        Some(self.entries.get(self.current + 1)?.surface(self.document?))
    }
    pub fn commit_back_surface(&mut self) -> Option<SurfacePosition> {
        let p = self.peek_back_surface()?;
        self.current -= 1;
        Some(p)
    }
    pub fn commit_forward_surface(&mut self) -> Option<SurfacePosition> {
        let p = self.peek_forward_surface()?;
        self.current += 1;
        Some(p)
    }
    pub fn update_surface(
        &mut self,
        position: SurfacePosition,
    ) -> Result<bool, mdtui3::LayoutError> {
        if self.document != Some(position.document()) {
            return Err(mdtui3::LayoutError::WrongDocument);
        }
        let Some(entry) = self.entries.get_mut(self.current) else {
            return Ok(false);
        };
        let next = Entry::from_surface(position);
        let changed = *entry != next;
        *entry = next;
        Ok(changed)
    }
    pub fn record_surface_jump(
        &mut self,
        from: Option<SurfacePosition>,
        to: SurfacePosition,
    ) -> Result<(), mdtui3::LayoutError> {
        if from.is_some_and(|p| p.document() != to.document())
            || self.document.is_some_and(|d| d != to.document())
        {
            return Err(mdtui3::LayoutError::WrongDocument);
        }
        if from == Some(to) {
            return Ok(());
        }
        self.document = Some(to.document());
        let to = Entry::from_surface(to);
        let from = from.map(Entry::from_surface);
        if self.entries.is_empty() {
            if let Some(from) = from {
                self.entries.push(from);
            } else {
                self.entries.push(to);
                self.current = 0;
                return Ok(());
            }
        }
        if let Some(from) = from {
            self.entries[self.current] = from;
        }
        self.entries.truncate(self.current + 1);
        if self.entries[self.current] != to {
            self.entries.push(to);
            self.current = self.entries.len() - 1;
        }
        Ok(())
    }
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn current(&self) -> Option<HistoryPosition> {
        self.entries
            .get(self.current)
            .copied()
            .and_then(Entry::root)
    }

    #[must_use]
    pub fn last_source(&self) -> Option<usize> {
        self.current().map(|position| position.source)
    }

    #[must_use]
    pub fn can_go_back(&self) -> bool {
        self.current > 0
    }

    #[must_use]
    pub fn can_go_forward(&self) -> bool {
        self.current + 1 < self.entries.len()
    }

    pub fn update_current(&mut self, position: Option<HistoryPosition>) -> bool {
        let (Some(position), Some(current)) = (position, self.entries.get_mut(self.current)) else {
            return false;
        };
        if current.root().is_none() || *current == Entry::Root(position) {
            return false;
        }
        *current = Entry::Root(position);
        true
    }

    pub fn record_jump(&mut self, from: Option<HistoryPosition>, to: HistoryPosition) {
        if self.entries.is_empty() {
            self.entries.push(Entry::Root(to));
            self.current = 0;
            return;
        }
        if let Some(from) = from {
            self.entries[self.current] = Entry::Root(from);
        }
        self.entries.truncate(self.current + 1);
        if self.entries.get(self.current) == Some(&Entry::Root(to)) {
            return;
        }
        self.entries.push(Entry::Root(to));
        self.current = self.entries.len() - 1;
    }

    #[must_use]
    pub fn peek_back(&self) -> Option<HistoryPosition> {
        self.current
            .checked_sub(1)
            .and_then(|index| self.entries.get(index))
            .copied()
            .and_then(Entry::root)
    }

    #[must_use]
    pub fn peek_forward(&self) -> Option<HistoryPosition> {
        self.entries
            .get(self.current + 1)
            .copied()
            .and_then(Entry::root)
    }

    pub fn commit_back(&mut self) -> Option<HistoryPosition> {
        let target = self.peek_back()?;
        self.current -= 1;
        Some(target)
    }

    pub fn commit_forward(&mut self) -> Option<HistoryPosition> {
        let target = self.peek_forward()?;
        self.current += 1;
        Some(target)
    }

    pub fn back(&mut self) -> Option<HistoryPosition> {
        self.commit_back()
    }

    pub fn forward(&mut self) -> Option<HistoryPosition> {
        self.commit_forward()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn position(source: usize, row_offset: u64) -> HistoryPosition {
        HistoryPosition { source, row_offset }
    }

    #[test]
    fn empty_history_has_no_current_or_movement() {
        let mut history = HistoryState::default();
        assert!(history.is_empty());
        assert_eq!(history.current(), None);
        assert_eq!(history.back(), None);
        assert_eq!(history.forward(), None);
        assert!(!history.update_current(Some(position(1, 0))));
    }

    #[test]
    fn scrolling_updates_current_without_adding_an_entry() {
        let mut history = HistoryState::default();
        history.record_jump(None, position(10, 0));
        assert!(history.update_current(Some(position(10, 3))));
        assert_eq!(history.len(), 1);
        assert_eq!(history.current(), Some(position(10, 3)));
        assert!(!history.update_current(Some(position(10, 3))));
    }

    #[test]
    fn jumps_back_and_forward_keep_the_expected_branch() {
        let mut history = HistoryState::default();
        history.record_jump(None, position(10, 0));
        history.record_jump(Some(position(10, 2)), position(20, 1));
        assert_eq!(history.len(), 2);
        assert_eq!(history.current(), Some(position(20, 1)));
        assert_eq!(history.back(), Some(position(10, 2)));
        assert_eq!(history.forward(), Some(position(20, 1)));

        assert_eq!(history.back(), Some(position(10, 2)));
        history.record_jump(Some(position(10, 2)), position(30, 0));
        assert_eq!(history.len(), 2);
        assert_eq!(history.current(), Some(position(30, 0)));
        assert!(!history.can_go_forward());
        assert_eq!(history.forward(), None);
    }

    #[test]
    fn recording_the_current_position_does_not_duplicate_it() {
        let mut history = HistoryState::default();
        history.record_jump(None, position(10, 0));
        history.record_jump(Some(position(10, 0)), position(10, 0));
        assert_eq!(history.len(), 1);
        assert_eq!(history.current(), Some(position(10, 0)));
    }
}
