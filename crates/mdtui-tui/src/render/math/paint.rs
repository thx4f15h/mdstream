//! Indexed object rectangle traversal. Cells carry text handles, never owned
//! copies of an entire glyph/command or an unbounded rendered object bitmap.
use super::input::Input;
use super::layout::{IndexKind, Layout, Primitive};
use super::storage::{Paged, StorageWork};
use super::syntax::Ink;
use mdtui3::{Budget, Span, WorkLimit};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: usize,
    pub y: usize,
    pub width: usize,
    pub height: usize,
}
impl Rect {
    fn hit(self, x: usize, y: usize, w: usize, h: usize) -> bool {
        x < self.x.saturating_add(self.width)
            && self.x < x.saturating_add(w)
            && y < self.y.saturating_add(self.height)
            && self.y < y.saturating_add(h)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MathCell {
    pub x: usize,
    pub y: usize,
    pub width: u8,
    pub ink: super::MathInk,
    pub source: Span,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaintStep {
    Progress,
    Cell(MathCell),
    Complete,
    BudgetExhausted,
}
#[derive(Clone, Copy)]
struct Visit {
    id: usize,
    x: usize,
    y: usize,
    entered: bool,
    next: Option<usize>,
    primitive: usize,
    offset: usize,
    paid_bytes: usize,
}
#[derive(Clone, Copy, Debug, Default)]
pub struct PaintWork {
    pub steps: usize,
    pub seeks: usize,
    pub output_bytes: usize,
    pub cells: usize,
    pub ancestry: usize,
    pub validation_steps: usize,
}
pub struct Paint {
    pub(super) version: u64,
    root: usize,
    rect: Rect,
    stack: Paged<Visit>,
    started: bool,
    key: super::MathKey,
    pub work: PaintWork,
}
fn pay(b: &mut Budget, bytes: usize) -> bool {
    b.charge(WorkLimit {
        queries: 1,
        objects: 1,
        bytes,
        ..WorkLimit::default()
    })
}
fn ink_len(ink: Ink, store: Option<&super::shaping::SharedGlyphs>) -> usize {
    match ink {
        Ink::Glyph(id) => {
            let data = store.unwrap().borrow();
            let g = data.glyph(id);
            g.end - g.start
        }
        Ink::Glyphs(_) => 0,
        Ink::Payload(s) => s.len(),
        Ink::Static(s) => s.len(),
        Ink::Space(n) => n,
    }
}
impl Paint {
    pub(super) fn new(layout: &Layout, rect: Rect, key: super::MathKey) -> Self {
        Self {
            version: layout.version,
            root: layout.root.unwrap(),
            rect,
            stack: Paged::default(),
            started: false,
            key,
            work: PaintWork::default(),
        }
    }
    pub fn storage(&self) -> StorageWork {
        self.stack.stats()
    }
    pub(super) fn step(
        &mut self,
        l: &Layout,
        input: &Input,
        glyphs: Option<&super::shaping::SharedGlyphs>,
        b: &mut Budget,
    ) -> PaintStep {
        if !self.started {
            if !pay(b, 0) {
                return PaintStep::BudgetExhausted;
            }
            self.started = true;
            self.stack.push(Visit {
                id: self.root,
                x: 0,
                y: 0,
                entered: false,
                next: None,
                primitive: 0,
                offset: 0,
                paid_bytes: 0,
            });
            self.work.steps += 1;
            return PaintStep::Progress;
        }
        let Some(mut v) = self.stack.last().copied() else {
            return PaintStep::Complete;
        };
        let bx = l.boxes[v.id];
        if !v.entered {
            if !pay(b, 0) {
                return PaintStep::BudgetExhausted;
            }
            self.work.steps += 1;
            self.work.ancestry += 1;
            if !self.rect.hit(
                v.x,
                v.y,
                bx.metrics.width.saturating_add(1),
                bx.metrics.height,
            ) {
                self.stack.pop();
                return PaintStep::Progress;
            }
            let coord = if bx.index == IndexKind::Y {
                self.rect.y.saturating_sub(v.y)
            } else {
                self.rect.x.saturating_sub(v.x)
            };
            v.next = l.first_link(v.id, coord);
            v.entered = true;
            self.work.seeks += usize::from(bx.index != IndexKind::Small);
            *self.stack.last_mut().unwrap() = v;
            return PaintStep::Progress;
        }
        if let Some(link) = v.next {
            if !pay(b, 0) {
                return PaintStep::BudgetExhausted;
            }
            self.work.steps += 1;
            let edge = l.links[link];
            let p = l.placed[edge.child];
            let child = l.boxes[p.child];
            let x = v.x + p.x;
            let y = v.y
                + p.y
                    .unwrap_or_else(|| bx.metrics.baseline - child.metrics.baseline);
            self.stack.last_mut().unwrap().next = if bx.index == IndexKind::X
                && x >= self.rect.x.saturating_add(self.rect.width)
                || bx.index == IndexKind::Y && y >= self.rect.y.saturating_add(self.rect.height)
            {
                None
            } else {
                l.next_link(v.id, link)
            };
            if child.has_ink
                && self.rect.hit(
                    x,
                    y,
                    child.metrics.width.saturating_add(1),
                    child.metrics.height,
                )
            {
                self.stack.push(Visit {
                    id: p.child,
                    x,
                    y,
                    entered: false,
                    next: None,
                    primitive: 0,
                    offset: 0,
                    paid_bytes: 0,
                });
            }
            return PaintStep::Progress;
        }
        if let Some(p) = bx.primitives.get(v.primitive).copied().flatten() {
            let (px, py, pw, ph) = match p {
                Primitive::Text {
                    x, y, width, ink, ..
                } => (
                    x,
                    y,
                    if let Ink::Glyphs(node) = ink {
                        if glyphs.unwrap().borrow().has_glyphs(node) {
                            width.saturating_add(1)
                        } else {
                            0
                        }
                    } else {
                        width
                    },
                    1,
                ),
                Primitive::Rule { x, y, length, .. } => (x, y, length, 1),
                Primitive::Fence {
                    x, y, height, ch, ..
                } => (x, y, usize::from(ch != '.'), height),
            };
            if pw == 0 || !self.rect.hit(v.x + px, v.y + py, pw, ph) {
                if !pay(b, 0) {
                    return PaintStep::BudgetExhausted;
                }
                self.work.steps += 1;
                let last = self.stack.last_mut().unwrap();
                last.primitive += 1;
                last.offset = 0;
                last.paid_bytes = 0;
                return PaintStep::Progress;
            }
            let (mut x, y, mut ink, mut width, source, finished) = match p {
                Primitive::Text {
                    ink,
                    width,
                    source,
                    x,
                    y,
                } => {
                    if let Ink::Glyphs(node) = ink {
                        let data = glyphs.unwrap().borrow();
                        let id = if v.offset == 0 {
                            data.first(node, self.rect.x.saturating_sub(v.x + x))
                        } else {
                            Some(v.offset - 1)
                        };
                        let Some(id) = id else {
                            if !pay(b, 0) {
                                return PaintStep::BudgetExhausted;
                            }
                            self.work.steps += 1;
                            self.stack.last_mut().unwrap().primitive += 1;
                            self.stack.last_mut().unwrap().offset = 0;
                            return PaintStep::Progress;
                        };
                        let g = data.glyph(id);
                        (
                            v.x + x + g.x,
                            v.y + y,
                            Ink::Glyph(id),
                            g.width,
                            g.source,
                            data.next(node, id).is_none(),
                        )
                    } else {
                        let ascii = match ink {
                            Ink::Glyphs(_) | Ink::Glyph(_) => unreachable!(),
                            Ink::Payload(_) => true,
                            Ink::Static(s) => s.is_ascii(),
                            Ink::Space(_) => true,
                        };
                        if ascii && width > 1 {
                            let at = v.offset;
                            let unit = match ink {
                                Ink::Glyphs(_) | Ink::Glyph(_) => unreachable!(),
                                Ink::Payload(s) => {
                                    Ink::Payload(Span::new(s.start + at, s.start + at + 1))
                                }
                                Ink::Static(s) => Ink::Static(&s[at..at + 1]),
                                Ink::Space(_) => Ink::Space(1),
                            };
                            (v.x + x + at, v.y + y, unit, 1, source, at + 1 >= width)
                        } else {
                            (v.x + x, v.y + y, ink, width, source, true)
                        }
                    }
                }
                Primitive::Rule {
                    length,
                    source,
                    x,
                    y,
                } => (
                    v.x + x + v.offset,
                    v.y + y,
                    Ink::Static("─"),
                    1,
                    source,
                    v.offset + 1 >= length,
                ),
                Primitive::Fence {
                    ch,
                    height,
                    source,
                    x,
                    y,
                } => {
                    let i = v.offset;
                    let text = match (ch, height, i) {
                        ('.', _, _) => "",
                        ('(', 1, _) => "(",
                        (')', 1, _) => ")",
                        ('[', 1, _) => "[",
                        (']', 1, _) => "]",
                        ('(', _, 0) => "⎛",
                        (')', _, 0) => "⎞",
                        ('(', h, i) if i + 1 == h => "⎝",
                        (')', h, i) if i + 1 == h => "⎠",
                        ('(', _, _) => "⎜",
                        (')', _, _) => "⎟",
                        ('[', _, 0) => "┌",
                        (']', _, 0) => "┐",
                        ('[', h, i) if i + 1 == h => "└",
                        (']', h, i) if i + 1 == h => "┘",
                        _ => "│",
                    };
                    (
                        v.x + x,
                        v.y + y + i,
                        Ink::Static(text),
                        1,
                        source,
                        i + 1 >= height,
                    )
                }
            };
            if v.offset == 0
                && !matches!(
                    p,
                    Primitive::Text {
                        ink: Ink::Glyphs(_),
                        ..
                    }
                )
            {
                let skip = if matches!(p, Primitive::Fence { .. }) {
                    self.rect.y.saturating_sub(y)
                } else {
                    self.rect.x.saturating_sub(x)
                };
                if skip > 0
                    && matches!(
                        p,
                        Primitive::Fence { .. }
                            | Primitive::Rule { .. }
                            | Primitive::Text { width: 2.., .. }
                    )
                {
                    if !pay(b, 0) {
                        return PaintStep::BudgetExhausted;
                    }
                    self.work.steps += 1;
                    self.stack.last_mut().unwrap().offset = skip;
                    return PaintStep::Progress;
                }
            }
            let next_glyph = if let Ink::Glyph(id) = ink {
                let node = match p {
                    Primitive::Text {
                        ink: Ink::Glyphs(node),
                        ..
                    } => node,
                    _ => unreachable!(),
                };
                glyphs.unwrap().borrow().next(node, id)
            } else {
                None
            };
            if width > self.rect.width
                && self.rect.width > 0
                && x <= self.rect.x
                && x + width > self.rect.x
            {
                ink = Ink::Static("…");
                width = 1;
                x = self.rect.x;
            }
            let visible = self.rect.hit(x, y, width.max(1), 1)
                && x >= self.rect.x
                && x + width <= self.rect.x + self.rect.width;
            let bytes = if visible && !matches!(ink, Ink::Glyph(_)) {
                ink_len(ink, glyphs)
            } else {
                0
            };
            let pending = bytes.saturating_sub(v.paid_bytes);
            let admitted = pending.min(b.remaining().bytes);
            if pending > 0 && admitted == 0 || !pay(b, admitted) {
                return PaintStep::BudgetExhausted;
            }
            self.work.steps += 1;
            self.work.output_bytes += admitted;
            if admitted < pending {
                self.stack.last_mut().unwrap().paid_bytes += admitted;
                return PaintStep::Progress;
            }
            let last = self.stack.last_mut().unwrap();
            last.paid_bytes = 0;
            if finished
                || x >= self.rect.x + self.rect.width
                    && matches!(p, Primitive::Text { .. } | Primitive::Rule { .. })
                || y >= self.rect.y + self.rect.height && matches!(p, Primitive::Fence { .. })
            {
                last.primitive += 1;
                last.offset = 0
            } else if let Some(id) = next_glyph {
                last.offset = id + 1;
            } else {
                last.offset += 1
            }
            if visible {
                self.work.cells += 1;
                let _ = input;
                return PaintStep::Cell(MathCell {
                    x,
                    y,
                    width: width as u8,
                    ink: super::MathInk { ink, key: self.key },
                    source,
                });
            }
            return PaintStep::Progress;
        }
        if !pay(b, 0) {
            return PaintStep::BudgetExhausted;
        }
        self.work.steps += 1;
        self.stack.pop();
        PaintStep::Progress
    }
    pub fn retire_step(&mut self, b: &mut Budget) -> bool {
        if self.stack.is_empty() {
            return true;
        }
        if !pay(b, 0) {
            return false;
        }
        self.stack.pop();
        self.stack.is_empty()
    }
}
