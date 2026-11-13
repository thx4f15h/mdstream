use mdtui3::LinkId;

use crate::screen::{ContentArea, Screen};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LinkHit {
    pub row: u16,
    pub x0: u16,
    pub x1: u16,
    pub id: LinkId,
}

impl LinkHit {
    #[must_use]
    pub const fn width(self) -> u16 {
        self.x1.saturating_sub(self.x0)
    }
}

/// Screen-local link runs in row-major order. The table never retains document
/// link objects or source text outside the body rectangle of the current screen.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LinkTable {
    hits: Vec<LinkHit>,
    ids: Vec<LinkId>,
}

impl LinkTable {
    #[must_use]
    pub fn from_screen(screen: &Screen, body: ContentArea) -> Self {
        let left = body.x.min(screen.cols());
        let right = body.right().min(screen.cols());
        if left >= right {
            return Self::default();
        }

        let mut table = Self::default();
        for row in 0..screen.rows() {
            let mut open: Option<LinkHit> = None;
            for column in left..right {
                if let Some(zero) = screen.reading_zero_cell(row, column)
                    && let Some(id) = zero.link_id()
                {
                    if !table.ids.contains(&id) {
                        table.ids.push(id);
                    }
                    table.hits.push(LinkHit {
                        row,
                        x0: column,
                        x1: column,
                        id,
                    });
                }
                let Some(cell) = screen.reading_cell(row, column) else {
                    break;
                };
                if cell.is_continuation() {
                    continue;
                }
                let link = cell.link_id();
                if let Some(hit) = open {
                    if link == Some(hit.id) && column == hit.x1 {
                        open = Some(LinkHit {
                            x1: hit.x1.saturating_add(u16::from(cell.width())).min(right),
                            ..hit
                        });
                        continue;
                    }
                    table.hits.push(hit);
                    open = None;
                }
                if let Some(id) = link {
                    if !table.ids.contains(&id) {
                        table.ids.push(id);
                    }
                    open = Some(LinkHit {
                        row,
                        x0: column,
                        x1: column.saturating_add(u16::from(cell.width())).min(right),
                        id,
                    });
                }
            }
            if let Some(hit) = open {
                table.hits.push(hit);
            }
        }
        table
    }

    #[must_use]
    pub fn hits(&self) -> &[LinkHit] {
        &self.hits
    }

    #[must_use]
    pub fn links(&self) -> &[LinkId] {
        &self.ids
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.ids.len()
    }

    #[must_use]
    pub fn contains(&self, id: LinkId) -> bool {
        self.ids.contains(&id)
    }

    pub fn hits_for(&self, id: LinkId) -> impl Iterator<Item = &LinkHit> {
        self.hits.iter().filter(move |hit| hit.id == id)
    }

    #[must_use]
    pub fn first(&self) -> Option<LinkId> {
        self.ids.first().copied()
    }

    #[must_use]
    pub fn last(&self) -> Option<LinkId> {
        self.ids.last().copied()
    }

    #[must_use]
    pub fn next(&self, current: Option<LinkId>) -> Option<LinkId> {
        let Some(current) = current else {
            return self.first();
        };
        let Some(index) = self.ids.iter().position(|id| *id == current) else {
            return self.first();
        };
        self.ids.get((index + 1) % self.ids.len()).copied()
    }

    #[must_use]
    pub fn previous(&self, current: Option<LinkId>) -> Option<LinkId> {
        let Some(current) = current else {
            return self.last();
        };
        let Some(index) = self.ids.iter().position(|id| *id == current) else {
            return self.last();
        };
        self.ids
            .get((index + self.ids.len() - 1) % self.ids.len())
            .copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::screen::{Cell, StyleRole};

    fn cell(text: &str, width: u8, link: Option<LinkId>) -> Cell {
        Cell::new(text, StyleRole::LINK, width, link, None).unwrap()
    }

    #[test]
    fn hits_group_runs_skip_continuations_and_keep_unique_link_order() {
        let mut screen = Screen::new(3, 20, ContentArea::new(0, 20)).unwrap();
        let first = LinkId(4);
        let second = LinkId(7);
        screen.write_cell(0, 1, cell("中", 2, Some(first))).unwrap();
        screen.write_cell(0, 3, cell("x", 1, Some(first))).unwrap();
        screen.write_cell(0, 5, cell("a", 1, Some(second))).unwrap();
        screen.write_cell(1, 0, cell("y", 1, Some(first))).unwrap();
        screen.swap_buffers();

        let table = LinkTable::from_screen(&screen, ContentArea::new(0, 20));
        assert_eq!(table.links(), &[first, second]);
        assert_eq!(
            table.hits(),
            &[
                LinkHit {
                    row: 0,
                    x0: 1,
                    x1: 4,
                    id: first,
                },
                LinkHit {
                    row: 0,
                    x0: 5,
                    x1: 6,
                    id: second,
                },
                LinkHit {
                    row: 1,
                    x0: 0,
                    x1: 1,
                    id: first,
                },
            ]
        );
        assert_eq!(table.hits_for(first).count(), 2);
        assert_eq!(table.next(Some(first)), Some(second));
        assert_eq!(table.next(Some(second)), Some(first));
        assert_eq!(table.next(None), Some(first));
        assert_eq!(table.next(Some(LinkId(99))), Some(first));
        assert_eq!(table.previous(None), Some(second));
        assert_eq!(table.previous(Some(first)), Some(second));
        assert_eq!(table.previous(Some(LinkId(99))), Some(second));
        assert_eq!(table.first(), Some(first));
        assert_eq!(table.last(), Some(second));
    }

    #[test]
    fn only_body_rectangle_links_are_visible() {
        let mut screen = Screen::new(2, 20, ContentArea::new(0, 20)).unwrap();
        let sidebar = LinkId(1);
        let body = LinkId(2);
        screen
            .write_cell(0, 1, cell("s", 1, Some(sidebar)))
            .unwrap();
        screen.write_cell(0, 6, cell("b", 1, Some(body))).unwrap();
        screen
            .write_cell(0, 11, cell("o", 1, Some(LinkId(3))))
            .unwrap();
        screen.swap_buffers();

        let table = LinkTable::from_screen(&screen, ContentArea::new(5, 5));
        assert_eq!(table.links(), &[body]);
        assert_eq!(table.hits()[0].x0, 6);
        assert_eq!(table.hits()[0].x1, 7);
    }
}
