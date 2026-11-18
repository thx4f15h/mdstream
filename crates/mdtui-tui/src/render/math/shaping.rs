//! Post-TeX scalar stream using the accepted shared GCB/width policy. Groups
//! and ignored whitespace do not split a flow; 2D operands/cells do.
use super::{
    input::Input,
    storage::{Paged, StorageWork, retire_map},
    syntax::{Ink, Kind, Syntax},
};
use crate::render::shape_unicode::{GcbState, Properties, WidthStream};
use mdtui3::{Budget, LayoutError, Span, WorkLimit};
use std::{
    cell::{Cell, RefCell},
    collections::BTreeMap,
    rc::Rc,
};

#[derive(Clone, Copy, Debug)]
pub(super) struct Glyph {
    pub start: usize,
    pub end: usize,
    pub source: Span,
    pub owner: usize,
    pub x: usize,
    pub width: usize,
    pub kind: crate::render::text::GraphemeKind,
    pub first: char,
    pub whitespace: bool,
}
#[derive(Clone, Copy, Default)]
struct NodeGlyphs {
    first: Option<usize>,
    end: usize,
    width: usize,
}
#[derive(Default)]
pub(super) struct GlyphStore {
    bytes: Paged<u8>,
    glyphs: Paged<Glyph>,
    nodes: Paged<NodeGlyphs>,
    index: BTreeMap<(usize, usize, usize), usize>,
    pub active: Rc<Cell<bool>>,
}
impl GlyphStore {
    pub fn glyph(&self, id: usize) -> Glyph {
        self.glyphs[id]
    }
    pub fn byte(&self, id: usize) -> u8 {
        self.bytes[id]
    }
    pub fn first(&self, node: usize, x: usize) -> Option<usize> {
        if x > 0
            && let Some((_, &id)) = self.index.range((node, 0, 0)..(node, x, 0)).next_back()
        {
            let g = self.glyphs[id];
            if g.x + g.width > x {
                return Some(id);
            }
        }
        self.index
            .range((node, x, 0)..=(node, usize::MAX, usize::MAX))
            .next()
            .map(|(_, id)| *id)
    }
    pub fn next(&self, node: usize, id: usize) -> Option<usize> {
        let next = id + 1;
        (next < self.nodes[node].end && self.glyphs[next].owner == node).then_some(next)
    }
    pub fn has_glyphs(&self, node: usize) -> bool {
        self.nodes[node].first.is_some()
    }
    pub fn storage(&self) -> [StorageWork; 3] {
        [self.bytes.stats(), self.glyphs.stats(), self.nodes.stats()]
    }
    pub fn index_entries(&self) -> usize {
        self.index.len()
    }
    pub fn index_payload_bytes(&self) -> usize {
        self.index.len() * std::mem::size_of::<((usize, usize, usize), usize)>()
    }
    pub fn retire_one(&mut self) -> bool {
        if retire_map(&mut self.index) || self.nodes.pop().is_some() || self.glyphs.pop().is_some()
        {
            return false;
        }
        self.bytes.pop();
        self.bytes.is_empty()
    }
}
pub(super) type SharedGlyphs = Rc<RefCell<GlyphStore>>;
#[derive(Clone, Copy, Debug, Default)]
pub struct ShapingWork {
    pub scalars: usize,
    pub bytes: usize,
    pub nodes: usize,
    pub joins: usize,
    pub steps: usize,
}
#[derive(Clone, Copy)]
struct Text {
    node: usize,
    ink: Ink,
    offset: usize,
    paid: usize,
}
pub(super) struct Shaper {
    pub store: Option<SharedGlyphs>,
    pub live: Rc<Cell<bool>>,
    next: usize,
    current: Option<Text>,
    flow: Option<usize>,
    gcb: GcbState,
    width: WidthStream,
    last: Option<usize>,
    pub work: ShapingWork,
}
#[derive(Clone, Copy, Debug)]
pub(super) enum ShapeStep {
    Progress,
    Touched { node: usize, width_changed: bool },
    Ready,
    BudgetExhausted,
}
impl Shaper {
    pub fn new() -> Self {
        Self {
            store: None,
            live: Rc::new(Cell::new(true)),
            next: 0,
            current: None,
            flow: None,
            gcb: GcbState::default(),
            width: WidthStream::default(),
            last: None,
            work: ShapingWork::default(),
        }
    }
    pub fn caught_up(&self, syntax: &Syntax) -> bool {
        self.next == syntax.nodes.len()
    }
    pub fn step(&mut self, syntax: &mut Syntax, input: &Input, b: &mut Budget) -> ShapeStep {
        if self.caught_up(syntax) {
            return ShapeStep::Ready;
        }
        if self.current.is_none() {
            if !pay(b, 0) {
                return ShapeStep::BudgetExhausted;
            }
            if self.store.is_none() {
                self.store = Some(Rc::new(RefCell::new(GlyphStore {
                    active: self.live.clone(),
                    ..GlyphStore::default()
                })));
            }
            self.store
                .as_ref()
                .unwrap()
                .borrow_mut()
                .nodes
                .push(NodeGlyphs::default());
            self.work.steps += 1;
            self.work.nodes += 1;
            let n = syntax.nodes[self.next];
            if let Kind::Text { ink, .. } = n.kind {
                if self.flow != Some(n.flow) {
                    self.flow = Some(n.flow);
                    self.gcb = GcbState::default();
                    self.width = WidthStream::default();
                    self.last = None;
                }
                self.current = Some(Text {
                    node: self.next,
                    ink,
                    offset: 0,
                    paid: 0,
                });
            } else {
                syntax.nodes[self.next].needs_shape = false;
                self.next += 1;
            }
            return ShapeStep::Progress;
        }
        let mut text = self.current.unwrap();
        let len = match text.ink {
            Ink::Payload(s) => s.len(),
            Ink::Static(s) => s.len(),
            Ink::Space(n) => n,
            _ => unreachable!(),
        };
        if text.offset == len {
            if !pay(b, 0) {
                return ShapeStep::BudgetExhausted;
            }
            let shaped = self.store.as_ref().unwrap().borrow().nodes[text.node];
            syntax.nodes[text.node].kind = Kind::Text {
                ink: if shaped.first.is_some() {
                    Ink::Glyphs(text.node)
                } else {
                    Ink::Space(0)
                },
                width: Some(shaped.width),
            };
            syntax.nodes[text.node].needs_shape = false;
            self.current = None;
            self.next += 1;
            self.work.steps += 1;
            return ShapeStep::Progress;
        }
        let mut utf8 = [0; 4];
        let ch = match text.ink {
            Ink::Payload(s) => {
                let first = input.byte(s.start + text.offset);
                let n = if first < 128 {
                    1
                } else if first < 0xe0 {
                    2
                } else if first < 0xf0 {
                    3
                } else {
                    4
                };
                for (i, slot) in utf8.iter_mut().enumerate().take(n) {
                    *slot = input.byte(s.start + text.offset + i)
                }
                std::str::from_utf8(&utf8[..n])
                    .unwrap()
                    .chars()
                    .next()
                    .unwrap()
            }
            Ink::Static(s) => s[text.offset..].chars().next().unwrap(),
            Ink::Space(_) => ' ',
            _ => unreachable!(),
        };
        let size = ch.len_utf8();
        let needed = size - text.paid;
        let admitted = needed.min(b.remaining().bytes);
        if admitted == 0 || !pay(b, admitted) {
            return ShapeStep::BudgetExhausted;
        }
        text.paid += admitted;
        self.work.steps += 1;
        self.work.bytes += admitted;
        if text.paid < size {
            self.current = Some(text);
            return ShapeStep::Progress;
        }
        text.paid = 0;
        text.offset += size;
        self.current = Some(text);
        self.work.scalars += 1;
        // Same accepted display filtering as ordinary 021 source atoms.
        if ch.is_control() && ch != '\t' {
            self.gcb = GcbState::default();
            self.width = WidthStream::default();
            self.last = None;
            return ShapeStep::Progress;
        }
        let ch = if ch == '\t' { ' ' } else { ch };
        let break_before = self.gcb.push(Properties::of(ch));
        let display = ch.encode_utf8(&mut utf8);
        self.width.push(display);
        let glyph_width = self.width.pending_width();
        let store = self.store.as_ref().unwrap();
        let mut store = store.borrow_mut();
        let start = store.bytes.len();
        for byte in display.bytes() {
            store.bytes.push(byte);
        }
        let end = store.bytes.len();
        let source = syntax.nodes[text.node].source;
        let previous = if break_before { None } else { self.last };
        if let Some(id) = previous {
            let mut glyph = store.glyphs[id];
            let changed = glyph.width != glyph_width;
            let owner = glyph.owner;
            store.nodes[owner].width = store.nodes[owner].width + glyph_width - glyph.width;
            glyph.width = glyph_width;
            glyph.kind = self.width.pending_kind();
            glyph.end = end;
            glyph.source.end = glyph.source.end.max(source.end);
            glyph.whitespace &= ch.is_whitespace();
            store.glyphs[id] = glyph;
            let node = &mut syntax.nodes[owner];
            node.source.end = node.source.end.max(source.end);
            if let Kind::Text { width, .. } = &mut node.kind {
                *width = Some(store.nodes[owner].width);
            }
            self.work.joins += 1;
            ShapeStep::Touched {
                node: owner,
                width_changed: changed,
            }
        } else {
            let id = store.glyphs.len();
            let old = store.nodes[text.node];
            store.glyphs.push(Glyph {
                start,
                end,
                source,
                owner: text.node,
                x: old.width,
                width: glyph_width,
                kind: self.width.pending_kind(),
                first: ch,
                whitespace: ch.is_whitespace(),
            });
            store.index.insert((text.node, old.width, id), id);
            store.nodes[text.node] = NodeGlyphs {
                first: old.first.or(Some(id)),
                end: id + 1,
                width: old.width + glyph_width,
            };
            self.last = Some(id);
            ShapeStep::Progress
        }
    }

    pub fn retire_one(&mut self) -> bool {
        let Some(store) = self.store.as_ref() else {
            return true;
        };
        if Rc::strong_count(store) > 1 {
            return false;
        }
        store.borrow_mut().retire_one()
    }
}
fn pay(b: &mut Budget, bytes: usize) -> bool {
    b.charge(WorkLimit {
        queries: 1,
        objects: 1,
        bytes,
        ..WorkLimit::default()
    })
}

/// Fixed descriptor used by ordinary Cell/diff/wire/clipboard readers. The
/// indexed bytes remain paged and are never collected into an unfinished String.
#[derive(Clone)]
pub(crate) struct MathPayload {
    store: SharedGlyphs,
    glyph: usize,
    start: usize,
    end: usize,
    source_end: usize,
    whole_source_atom: bool,
}
impl MathPayload {
    pub(super) fn new(store: SharedGlyphs, glyph: usize) -> Self {
        let g = store.borrow().glyph(glyph);
        let node = store.borrow().nodes[g.owner];
        let whole_source_atom = node.first == Some(glyph) && node.end == glyph + 1;
        Self {
            whole_source_atom,
            store,
            glyph,
            start: g.start,
            end: g.end,
            source_end: g.source.end,
        }
    }
    pub fn contains_whole_source_atom(&self) -> bool {
        self.whole_source_atom
    }
    pub fn source_end(&self) -> usize {
        self.source_end
    }
    pub fn len(&self) -> usize {
        self.end - self.start
    }
    pub fn next(
        &self,
        position: usize,
        b: &mut Budget,
    ) -> Result<Option<([u8; 8], u8)>, LayoutError> {
        let store = self.store.borrow();
        if !store.active.get() {
            return Err(LayoutError::StaleContinuation);
        }
        if self.start + position == self.end {
            return Ok(None);
        }
        if !pay(b, 0) {
            return Ok(Some(([0; 8], 0)));
        }
        let at = self.start + position;
        let byte = store.byte(at);
        let n = if byte < 128 {
            1
        } else if byte < 0xe0 {
            2
        } else if byte < 0xf0 {
            3
        } else {
            4
        };
        let mut out = [0; 8];
        for (i, slot) in out.iter_mut().enumerate().take(n) {
            *slot = store.byte(at + i)
        }
        Ok(Some((out, n as u8)))
    }
    pub fn same(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.store, &other.store)
            && self.glyph == other.glyph
            && self.start == other.start
            && self.end == other.end
    }
    pub fn suffix(&self, old: &Self) -> Option<Self> {
        (Rc::ptr_eq(&self.store, &old.store)
            && self.glyph == old.glyph
            && self.start == old.start
            && self.end >= old.end)
            .then(|| Self {
                store: self.store.clone(),
                glyph: self.glyph,
                start: old.end,
                end: self.end,
                source_end: self.source_end,
                whole_source_atom: false,
            })
    }
}
