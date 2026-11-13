//! Fixed metadata shared by screen cells and output. Contiguous text is for
//! static/small producers; indexed clusters retain source/property references.
use super::shape::{DisplayPosition, ShapeReadStep, ShapeReader, SharedShape};
use mdtui3::{Budget, LayoutError, Span, WorkLimit};
use std::{cell::RefCell, fmt, rc::Rc};
pub(crate) const INLINE: usize = 32;
#[derive(Clone)]
struct Origin {
    shape: SharedShape,
    source: Span,
    epoch: usize,
    version: u64,
    clock: u64,
    cut: (DisplayPosition, DisplayPosition),
}
#[derive(Clone)]
enum Storage {
    Small {
        utf8: [u8; INLINE],
        len: u8,
        origin: Option<Rc<Origin>>,
    },
    Shared(Rc<str>),
    Joined {
        pieces: Rc<RefCell<Vec<TextPayload>>>,
        end: usize,
    },
    Indexed {
        shape: SharedShape,
        source: Span,
        epoch: usize,
        version: u64,
        clock: u64,
        cut: (DisplayPosition, DisplayPosition),
    },
}
#[derive(Clone)]
pub struct TextPayload {
    storage: Storage,
    len: usize,
    width: usize,
    safe: bool,
    kind: Option<super::text::GraphemeKind>,
    first: Option<char>,
    whitespace: bool,
    ascii_whitespace: bool,
}
impl TextPayload {
    fn from_text(text: &str) -> Self {
        let storage = if text.len() <= INLINE {
            let mut utf8 = [0; INLINE];
            utf8[..text.len()].copy_from_slice(text.as_bytes());
            Storage::Small {
                utf8,
                len: text.len() as u8,
                origin: None,
            }
        } else {
            Storage::Shared(Rc::from(text))
        };
        Self {
            storage,
            len: text.len(),
            width: super::text::display_width(text),
            safe: text.chars().all(|ch| !ch.is_control() || ch == '\t'),
            kind: None,
            first: text.chars().next(),
            whitespace: text.chars().all(char::is_whitespace),
            ascii_whitespace: text.chars().all(|ch| ch.is_ascii_whitespace()),
        }
    }
    #[allow(clippy::too_many_arguments)] // One fixed cluster descriptor; no payload copy.
    pub(crate) fn indexed(
        shape: SharedShape,
        source: Span,
        cut: (DisplayPosition, DisplayPosition),
        len: usize,
        width: usize,
        kind: super::text::GraphemeKind,
        first: char,
        whitespace: bool,
        inline: Option<&[u8]>,
    ) -> Self {
        let (epoch, version, clock) = shape.payload_snapshot(source);
        let storage = if let Some(bytes) = inline {
            debug_assert_eq!(bytes.len(), len);
            let mut utf8 = [0; INLINE];
            utf8[..bytes.len()].copy_from_slice(bytes);
            Storage::Small {
                utf8,
                len: bytes.len() as u8,
                origin: Some(Rc::new(Origin {
                    shape,
                    source,
                    epoch,
                    version,
                    clock,
                    cut,
                })),
            }
        } else {
            Storage::Indexed {
                shape,
                source,
                epoch,
                version,
                clock,
                cut,
            }
        };
        Self {
            storage,
            len,
            width,
            safe: true,
            kind: Some(kind),
            first: Some(first),
            whitespace,
            ascii_whitespace: first.is_ascii_whitespace()
                && (len == first.len_utf8() || first == '\r' && len == 2),
        }
    }
    pub fn is_ascii_whitespace(&self) -> bool {
        self.ascii_whitespace
    }
    pub fn first_char(&self) -> Option<char> {
        self.first
    }
    pub fn is_whitespace(&self) -> bool {
        self.whitespace
    }
    pub fn cluster_kind(&self) -> Option<super::text::GraphemeKind> {
        self.kind
    }
    pub fn len(&self) -> usize {
        self.len
    }
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
    pub fn width(&self) -> usize {
        self.width
    }
    pub fn is_safe(&self) -> bool {
        self.safe
    }
    pub fn is_indexed(&self) -> bool {
        matches!(
            self.storage,
            Storage::Indexed { .. }
                | Storage::Small {
                    origin: Some(_),
                    ..
                }
        )
    }
    pub(crate) fn contains_whole_source_atoms(&self) -> bool {
        // A replacement (entity, image fallback or tab) may supply several
        // separate clusters with the same source. Only complete atom cuts can
        // stand in for the visible projection of a source selection.
        self.origin()
            .is_some_and(|origin| origin.cut.0.part == 0 && origin.cut.1.part == 0)
    }
    pub fn contiguous(&self) -> Option<&str> {
        match &self.storage {
            Storage::Small { utf8, len, .. } => {
                Some(std::str::from_utf8(&utf8[..usize::from(*len)]).unwrap())
            }
            Storage::Shared(text) => Some(text),
            Storage::Indexed { .. } | Storage::Joined { .. } => None,
        }
    }
    /// A proven ordinary-append continuation of an already acknowledged cut.
    /// No payload bytes are inspected; semantic changes reject continuation.
    fn origin(&self) -> Option<Origin> {
        match &self.storage {
            Storage::Indexed {
                shape,
                source,
                epoch,
                version,
                clock,
                cut,
            } => Some(Origin {
                shape: shape.clone(),
                source: *source,
                epoch: *epoch,
                version: *version,
                clock: *clock,
                cut: *cut,
            }),
            Storage::Small {
                origin: Some(origin),
                ..
            } => Some((**origin).clone()),
            _ => None,
        }
    }
    pub(crate) fn ends_at_source(&self, end: usize) -> bool {
        self.origin()
            .is_some_and(|origin| origin.source.end == end && origin.cut.1.part == 0)
    }
    pub(crate) fn suffix_after(&self, old: &Self) -> Option<Self> {
        let next = self.origin()?;
        let previous = old.origin()?;
        if !Rc::ptr_eq(&next.shape.0, &previous.shape.0)
            || next.epoch != previous.epoch
            || next.cut.0 != previous.cut.0
            || next.cut.1 < previous.cut.1
            || self.len < old.len
        {
            return None;
        }
        let (current_epoch, current_version, _) = next.shape.payload_snapshot(previous.source);
        if current_epoch != previous.epoch || current_version != previous.version {
            return None;
        }
        let (epoch, version, clock) = next.shape.payload_snapshot(next.source);
        Some(Self {
            storage: Storage::Indexed {
                shape: next.shape,
                source: next.source,
                epoch,
                version,
                clock,
                cut: (previous.cut.1, next.cut.1),
            },
            len: self.len - old.len,
            width: 0,
            safe: self.safe,
            kind: Some(super::text::GraphemeKind::ZeroWidth),
            first: None,
            whitespace: false,
            ascii_whitespace: false,
        })
    }
    pub fn reader(&self) -> PayloadReader {
        let indexed = match &self.storage {
            Storage::Indexed {
                shape,
                source,
                cut,
                epoch,
                version,
                clock,
            } => Some(
                ShapeReader::new(shape.clone(), *source)
                    .with_cut(cut.0, cut.1)
                    .with_snapshot(*epoch, *version, *clock),
            ),
            _ => None,
        };
        let children = if let Storage::Joined { pieces, end } = &self.storage {
            Some(JoinedReader {
                pieces: pieces.clone(),
                end: *end,
                next: 0,
                current: None,
            })
        } else {
            None
        };
        PayloadReader {
            payload: self.clone(),
            indexed,
            position: 0,
            children,
            origin: if let Storage::Small {
                origin: Some(origin),
                ..
            } = &self.storage
            {
                Some((**origin).clone())
            } else {
                None
            },
        }
    }
    /// Explicit unbounded collection for static rendering and assertions. Frame
    /// publication/diff/copy length checks never call this convenience method.
    pub fn collect_text(&self) -> String {
        let mut text = String::with_capacity(self.len);
        let mut reader = self.reader();
        loop {
            match reader
                .next(&mut Budget::new(WorkLimit::unlimited()))
                .expect("current prepared payload")
            {
                PayloadStep::Bytes { utf8, len } => {
                    text.push_str(std::str::from_utf8(&utf8[..usize::from(len)]).unwrap())
                }
                PayloadStep::Complete => break,
                PayloadStep::BudgetExhausted => unreachable!(),
            }
        }
        text
    }
    pub fn chars(&self) -> PayloadChars {
        PayloadChars {
            reader: self.reader(),
            pending: [char::from_u32(0).unwrap(); 8],
            at: 0,
            end: 0,
        }
    }
    pub fn contains(&self, ch: char) -> bool {
        self.chars().any(|c| c == ch)
    }
    pub(crate) fn append_adjacent(&mut self, other: &Self) {
        if let (Some(a), Some(b)) = (self.origin(), other.origin())
            && Rc::ptr_eq(&a.shape.0, &b.shape.0)
            && a.cut.1 == b.cut.0
            && a.epoch == b.epoch
        {
            let source = Span::new(a.source.start, b.source.end);
            let (epoch, version, clock) = a.shape.payload_snapshot(source);
            self.storage = Storage::Indexed {
                shape: a.shape,
                source,
                epoch,
                version,
                clock,
                cut: (a.cut.0, b.cut.1),
            };
            self.len += other.len;
            self.width += other.width;
            self.whitespace &= other.whitespace;
            self.ascii_whitespace &= other.ascii_whitespace;
            return;
        }
        self.append(other);
    }
    /// Explicit concatenation for static producers. Growing indexed zero-width
    /// runs use append_adjacent, which retains one finite source cut.
    pub fn append(&mut self, other: &TextPayload) {
        if other.is_empty() {
            return;
        }
        let mut addition = if let Storage::Joined { pieces, end } = &other.storage {
            pieces.borrow()[..*end].to_vec()
        } else {
            vec![other.clone()]
        };
        let (pieces, mut end) = if let Storage::Joined { pieces, end } = &self.storage {
            if *end == pieces.borrow().len() {
                (pieces.clone(), *end)
            } else {
                let copy = pieces.borrow()[..*end].to_vec();
                (Rc::new(RefCell::new(copy)), *end)
            }
        } else {
            (Rc::new(RefCell::new(vec![self.clone()])), 1)
        };
        end += addition.len();
        pieces.borrow_mut().append(&mut addition);
        self.storage = Storage::Joined { pieces, end };
        self.len += other.len;
        self.safe &= other.safe;
        self.whitespace &= other.whitespace;
        self.ascii_whitespace &= other.ascii_whitespace;
        self.first = self.first.or(other.first);
    }
    pub fn push_str(&mut self, text: &str) {
        self.append(&Self::from(text));
    }
}
impl Default for TextPayload {
    fn default() -> Self {
        Self::from("")
    }
}
impl From<&str> for TextPayload {
    fn from(s: &str) -> Self {
        Self::from_text(s)
    }
}
impl From<&mut str> for TextPayload {
    fn from(s: &mut str) -> Self {
        Self::from_text(s)
    }
}
impl From<String> for TextPayload {
    fn from(s: String) -> Self {
        Self::from_text(&s)
    }
}
impl From<&String> for TextPayload {
    fn from(s: &String) -> Self {
        Self::from_text(s)
    }
}
impl From<&TextPayload> for TextPayload {
    fn from(s: &TextPayload) -> Self {
        s.clone()
    }
}
impl fmt::Debug for TextPayload {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(s) = self.contiguous().filter(|s| s.len() <= INLINE) {
            s.fmt(f)
        } else {
            f.debug_struct("TextPayload")
                .field("bytes", &self.len)
                .field("width", &self.width)
                .field("indexed", &self.is_indexed())
                .finish()
        }
    }
}
impl PartialEq for TextPayload {
    fn eq(&self, other: &Self) -> bool {
        if self.len != other.len || self.width != other.width || self.safe != other.safe {
            return false;
        }
        match (&self.storage, &other.storage) {
            (Storage::Small { utf8: a, len, .. }, Storage::Small { utf8: b, .. }) => {
                a[..usize::from(*len)] == b[..usize::from(*len)]
                    || *len == 1 && matches!(a[0], b' ' | b'\t') && matches!(b[0], b' ' | b'\t')
            }
            (Storage::Shared(a), Storage::Shared(b)) => Rc::ptr_eq(a, b),
            (
                Storage::Indexed {
                    shape: a,
                    source: sa,
                    epoch: ea,
                    version: va,
                    cut: ca,
                    ..
                },
                Storage::Indexed {
                    shape: b,
                    source: sb,
                    epoch: eb,
                    version: vb,
                    cut: cb,
                    ..
                },
            ) => Rc::ptr_eq(&a.0, &b.0) && sa == sb && ea == eb && ca == cb && va == vb,
            (Storage::Joined { pieces: a, end: ea }, Storage::Joined { pieces: b, end: eb }) => {
                Rc::ptr_eq(a, b) && ea == eb
            }
            _ => false,
        }
    }
}
impl Eq for TextPayload {}
impl PartialEq<str> for TextPayload {
    fn eq(&self, s: &str) -> bool {
        self.len == s.len() && self.chars().eq(s.chars())
    }
}
impl PartialEq<&str> for TextPayload {
    fn eq(&self, s: &&str) -> bool {
        self == *s
    }
}
impl PartialEq<String> for TextPayload {
    fn eq(&self, s: &String) -> bool {
        self == s.as_str()
    }
}
impl fmt::Display for TextPayload {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for ch in self.chars() {
            fmt::Write::write_char(f, ch)?;
        }
        Ok(())
    }
}

pub enum PayloadStep {
    Bytes { utf8: [u8; 8], len: u8 },
    BudgetExhausted,
    Complete,
}
struct JoinedReader {
    pieces: Rc<RefCell<Vec<TextPayload>>>,
    end: usize,
    next: usize,
    current: Option<Box<PayloadReader>>,
}
pub struct PayloadReader {
    origin: Option<Origin>,
    payload: TextPayload,
    indexed: Option<ShapeReader>,
    position: usize,
    children: Option<JoinedReader>,
}
impl PayloadReader {
    pub fn next(&mut self, budget: &mut Budget) -> Result<PayloadStep, LayoutError> {
        if let Some(origin) = self.origin.as_mut() {
            if origin.epoch != 0 && origin.shape.binding_epoch() != origin.epoch {
                return Err(LayoutError::StaleContinuation);
            }
            if origin.shape.semantic_clock() != origin.clock {
                if !budget.charge(WorkLimit {
                    queries: 1,
                    objects: 1,
                    ..Default::default()
                }) {
                    return Ok(PayloadStep::BudgetExhausted);
                }
                let (epoch, version, clock) = origin.shape.payload_snapshot(origin.source);
                if epoch != origin.epoch || version != origin.version {
                    return Err(LayoutError::StaleContinuation);
                }
                origin.clock = clock;
            }
        }
        if let Some(children) = &mut self.children {
            loop {
                if children.current.is_none() {
                    if children.next == children.end {
                        return Ok(PayloadStep::Complete);
                    }
                    if !budget.charge(WorkLimit {
                        queries: 1,
                        objects: 1,
                        ..Default::default()
                    }) {
                        return Ok(PayloadStep::BudgetExhausted);
                    }
                    children.current =
                        Some(Box::new(children.pieces.borrow()[children.next].reader()));
                    children.next += 1;
                }
                match children.current.as_mut().unwrap().next(budget)? {
                    PayloadStep::Complete => children.current = None,
                    step => return Ok(step),
                }
            }
        }
        if let Some(reader) = &mut self.indexed {
            return Ok(match reader.next(budget)? {
                ShapeReadStep::Atom(atom) => PayloadStep::Bytes {
                    utf8: atom.utf8,
                    len: atom.len,
                },
                ShapeReadStep::BudgetExhausted => PayloadStep::BudgetExhausted,
                ShapeReadStep::Complete => PayloadStep::Complete,
            });
        }
        if self.position == self.payload.len {
            return Ok(PayloadStep::Complete);
        }
        if !budget.charge(WorkLimit {
            queries: 1,
            objects: 1,
            ..WorkLimit::default()
        }) {
            return Ok(PayloadStep::BudgetExhausted);
        }
        let text = self.payload.contiguous().unwrap();
        let ch = text[self.position..].chars().next().unwrap();
        let mut utf8 = [0; 8];
        let len = ch.encode_utf8(&mut utf8[..4]).len() as u8;
        self.position += usize::from(len);
        Ok(PayloadStep::Bytes { utf8, len })
    }
}
pub struct PayloadChars {
    reader: PayloadReader,
    pending: [char; 8],
    at: usize,
    end: usize,
}
impl Iterator for PayloadChars {
    type Item = char;
    fn next(&mut self) -> Option<char> {
        loop {
            if self.at < self.end {
                let ch = self.pending[self.at];
                self.at += 1;
                return Some(ch);
            }
            match self
                .reader
                .next(&mut Budget::new(WorkLimit::unlimited()))
                .expect("current prepared payload")
            {
                PayloadStep::Complete => return None,
                PayloadStep::Bytes { utf8, len } => {
                    self.at = 0;
                    self.end = 0;
                    for ch in std::str::from_utf8(&utf8[..usize::from(len)])
                        .unwrap()
                        .chars()
                    {
                        self.pending[self.end] = ch;
                        self.end += 1;
                    }
                }
                PayloadStep::BudgetExhausted => unreachable!(),
            }
        }
    }
}

#[cfg(test)]
mod production_join_bounds {
    use super::*;
    use crate::{
        cli::LinkMode,
        render::frame::{FrameGeometry, FrameRuntime},
    };
    #[test]
    fn actual_orphan_growth_and_block_barriers_keep_one_indexed_payload_not_join_history() {
        let geometry = FrameGeometry {
            rows: 8,
            cols: 24,
            content_width: 24,
        };
        let mut runtime = FrameRuntime::new(geometry, LinkMode::NoOsc8).unwrap();
        let mut wire = Vec::new();
        let marks = "\u{301}".repeat(128);
        for batch in 1..=64 {
            let update = runtime.append_bytes(marks.as_bytes()).unwrap().unwrap();
            runtime.apply_update(&update).unwrap();
            let mut ready = false;
            for _ in 0..1000 {
                if runtime
                    .frame_to(&mut wire, &mut Budget::new(WorkLimit::unlimited()))
                    .unwrap()
                    .committed
                {
                    ready = true;
                    break;
                }
            }
            assert!(ready);
            let zero = runtime.screen().front_zero_cell(0, 0).unwrap();
            assert!(matches!(zero.text().storage, Storage::Indexed { .. }));
            assert_eq!(zero.text().len(), batch * marks.len());
            assert_eq!(zero.source(), Some(Span::new(0, batch * marks.len())));
            assert_eq!(
                runtime.screen().front_line_source(0),
                runtime.screen().line_source(0),
                "zero ACK must update the row's source envelope"
            );
            assert_eq!(wire.iter().filter(|&&b| b == 0xcc).count(), batch * 128);
        }
        let expected = marks.repeat(64);
        assert!(
            wire.windows(expected.len())
                .any(|w| w == expected.as_bytes())
        );
        let source = format!("{marks}\n\n{marks}\n\n> {marks}\n\n- {marks}\n");
        let mut runtime =
            FrameRuntime::from_finished_source(&source, geometry, LinkMode::NoOsc8).unwrap();
        for _ in 0..1000 {
            if runtime
                .frame_to(&mut Vec::new(), &mut Budget::new(WorkLimit::unlimited()))
                .unwrap()
                .committed
            {
                break;
            }
        }
        let mut zero_count = 0;
        for row in 0..runtime.screen().rows() {
            for col in 0..runtime.screen().cols() {
                if let Some(zero) = runtime.screen().front_zero_cell(row, col) {
                    zero_count += 1;
                    assert!(matches!(zero.text().storage, Storage::Indexed { .. }));
                    assert_eq!(zero.text().len(), marks.len());
                }
            }
        }
        assert_eq!(
            zero_count, 4,
            "each real block fragment keeps its own orphan owner"
        );
    }
}
