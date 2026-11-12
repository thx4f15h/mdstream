#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HistoryPosition {
    pub source: usize,
    pub row_offset: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HistoryState {
    entries: Vec<HistoryPosition>,
    current: usize,
}

impl HistoryState {
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
        self.entries.get(self.current).copied()
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
        if *current == position {
            return false;
        }
        *current = position;
        true
    }

    pub fn record_jump(&mut self, from: Option<HistoryPosition>, to: HistoryPosition) {
        if self.entries.is_empty() {
            self.entries.push(to);
            self.current = 0;
            return;
        }
        if let Some(from) = from {
            self.entries[self.current] = from;
        }
        self.entries.truncate(self.current + 1);
        if self.entries.get(self.current) == Some(&to) {
            return;
        }
        self.entries.push(to);
        self.current = self.entries.len() - 1;
    }

    #[must_use]
    pub fn peek_back(&self) -> Option<HistoryPosition> {
        self.current
            .checked_sub(1)
            .and_then(|index| self.entries.get(index))
            .copied()
    }

    #[must_use]
    pub fn peek_forward(&self) -> Option<HistoryPosition> {
        self.entries.get(self.current + 1).copied()
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
