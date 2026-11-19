//! Native image geometry, independent of URI selection and codec ownership.
//! Entries are current source objects, never revision-keyed history. The
//! formal selected-target producer supplies/withdraws these records separately.
use super::{cache::ResourceId, worker::Geometry};
use crate::render::{
    objects::SharedLinks,
    shape::{ShapeQuery, ShapeSelect},
};
use mdtui3::{BodyKey, Budget, Document, DocumentId, LayoutError, LinkId, Span, WorkLimit};
use std::{cell::RefCell, collections::BTreeMap, ops::Bound, rc::Rc};
pub const MAX_OBJECTS: usize = 256;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Surface {
    Root(DocumentId),
    Note(BodyKey),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ImageKey {
    pub surface: Surface,
    pub object: LinkId,
    pub source_start: usize,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Band {
    pub key: ImageKey,
    pub source: Span,
    pub label: Span,
    pub hit: LinkId,
    pub anchor: usize,
    pub resource: ResourceId,
    pub geometry: Geometry,
    pub generation: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImageFocus {
    pub key: ImageKey,
    pub row: u32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BandLine {
    pub band: Band,
    pub column: u16,
    pub row: u32,
    pub width: u16,
}
#[derive(Clone)]
pub struct SharedBands(Rc<RefCell<Bands>>);
struct Bands {
    native: DocumentId,
    surface: Surface,
    version: u64,
    entries: BTreeMap<(usize, LinkId), Band>,
    ids: BTreeMap<LinkId, usize>,
}
impl SharedBands {
    pub fn new(d: &Document) -> Self {
        Self(Rc::new(RefCell::new(Bands {
            native: d.id(),
            surface: Surface::Root(d.id()),
            version: 0,
            entries: BTreeMap::new(),
            ids: BTreeMap::new(),
        })))
    }
    /// A native note renderer keeps its BodyKey while its local DocumentId
    /// remains private to with_renderer. Bind before publishing any entries.
    pub fn bind_note(&self, body: BodyKey) -> Result<(), LayoutError> {
        let mut s = self.0.borrow_mut();
        if !s.entries.is_empty() {
            return Err(LayoutError::OutOfOrderUpdate);
        }
        s.surface = Surface::Note(body);
        Ok(())
    }
    pub fn surface(&self) -> Surface {
        self.0.borrow().surface
    }
    pub fn version(&self) -> u64 {
        self.0.borrow().version
    }
    pub fn len(&self) -> usize {
        self.0.borrow().entries.len()
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub fn capacity_bytes(&self) -> usize {
        std::mem::size_of::<Bands>()
            + self.len()
                * (std::mem::size_of::<Band>()
                    + std::mem::size_of::<(usize, LinkId)>()
                    + 6 * std::mem::size_of::<usize>())
    }
    pub fn publish(&self, d: &Document, band: Band, b: &mut Budget) -> Result<bool, LayoutError> {
        if !b.charge(WorkLimit {
            queries: 1,
            objects: 1,
            ..Default::default()
        }) {
            return Ok(false);
        }
        let mut s = self.0.borrow_mut();
        if d.id() != s.native || band.key.surface != s.surface {
            return Err(LayoutError::WrongDocument);
        }
        if band.key.source_start != band.source.start
            || band.source.end > d.source().len()
            || band.anchor < band.source.start
            || band.anchor >= band.source.end
            || band.geometry.columns == 0
            || band.geometry.rows == 0
            || band.geometry.rows > 1000
            || band.geometry.columns > 1000
        {
            return Err(LayoutError::InvalidRange);
        }
        let info = d
            .link_info(band.key.object)
            .ok_or(LayoutError::InvalidRange)?;
        if info.kind != mdtui3::LinkKind::Image
            || info.source != band.source
            || info.label != band.label
        {
            return Err(LayoutError::StaleContinuation);
        }
        if band.anchor != band.source.start
            && !(band.label.start <= band.anchor && band.anchor < band.label.end)
        {
            return Err(LayoutError::InvalidRange);
        }
        let key = (band.anchor, band.key.object);
        if s.entries.get(&key) == Some(&band) {
            return Ok(true);
        }
        if !s.ids.contains_key(&band.key.object) && s.entries.len() == MAX_OBJECTS {
            return Err(LayoutError::InvalidRange);
        }
        if let Some(old) = s.ids.insert(band.key.object, band.anchor) {
            s.entries.remove(&(old, band.key.object));
        }
        s.entries.insert(key, band);
        s.version = s
            .version
            .checked_add(1)
            .expect("image geometry generation exhausted");
        Ok(true)
    }
    pub fn withdraw(&self, expected: Band, b: &mut Budget) -> bool {
        if !b.charge(WorkLimit {
            queries: 1,
            objects: 1,
            ..Default::default()
        }) {
            return false;
        }
        let mut s = self.0.borrow_mut();
        let key = (expected.anchor, expected.key.object);
        // A paused old owner must not withdraw a replacement at the same
        // native coordinates. Geometry/resource/generation are all bound.
        if s.entries.get(&key) == Some(&expected) {
            s.entries.remove(&key);
            s.ids.remove(&expected.key.object);
            s.version = s
                .version
                .checked_add(1)
                .expect("image geometry generation exhausted");
        }
        true
    }
    pub fn query(&self, source: Span, cut: usize) -> BandQuery {
        BandQuery {
            owner: self.clone(),
            version: self.version(),
            source,
            cut,
            after: None,
            pending: None,
            rows: Vec::new(),
            done: false,
        }
    }
}
pub struct BandQuery {
    owner: SharedBands,
    version: u64,
    source: Span,
    cut: usize,
    after: Option<(usize, LinkId)>,
    pending: Option<Band>,
    rows: Vec<Band>,
    done: bool,
}
impl BandQuery {
    pub fn advance(
        &mut self,
        d: &Document,
        view: Option<&mdtui3::FoldView<crate::render::checkpoint::RenderCheckpoint>>,
        b: &mut Budget,
    ) -> Result<bool, LayoutError> {
        if self.owner.0.borrow().native != d.id() {
            return Err(LayoutError::WrongDocument);
        }
        if self.version != self.owner.version() {
            return Err(LayoutError::StaleContinuation);
        }
        if self.done {
            return Ok(true);
        }
        loop {
            if self.pending.is_none() {
                if !b.charge(WorkLimit {
                    queries: 1,
                    objects: 1,
                    ..Default::default()
                }) {
                    return Ok(false);
                }
                let next = {
                    let s = self.owner.0.borrow();
                    let start = self.after.map_or(
                        Bound::Included((self.source.start, LinkId(0))),
                        Bound::Excluded,
                    );
                    s.entries
                        .range((start, Bound::Unbounded))
                        .next()
                        .map(|(k, v)| (*k, *v))
                };
                let Some((key, band)) = next.filter(|(k, _)| k.0 < self.source.end) else {
                    self.done = true;
                    return Ok(true);
                };
                self.after = Some(key);
                if band.source.end > self.cut {
                    continue;
                }
                self.pending = Some(band);
            }
            let band = self.pending.unwrap();
            if let Some(view) = view {
                match view.visible_fragment(d, band.source.start, b) {
                    Ok(None) => return Ok(false),
                    Ok(Some(span))
                        if span.start <= band.source.start && span.end >= band.source.end => {}
                    Ok(Some(_)) | Err(LayoutError::InvalidRange) => {
                        self.pending = None;
                        continue;
                    }
                    Err(e) => return Err(e),
                }
            } else if !b.charge(WorkLimit {
                queries: 1,
                objects: 1,
                ..Default::default()
            }) {
                return Ok(false);
            }
            self.rows.push(band);
            self.pending = None;
        }
    }
    pub fn take(&mut self) -> Option<Vec<Band>> {
        self.done.then(|| std::mem::take(&mut self.rows))
    }
    pub fn retire(&mut self, b: &mut Budget) -> bool {
        while !self.rows.is_empty() {
            if !b.charge(WorkLimit {
                queries: 1,
                objects: 1,
                ..Default::default()
            }) {
                return false;
            }
            self.rows.pop();
        }
        true
    }
}
/// Select the final displayed grapheme through cached native shape summaries.
/// No scan of the entire alt or the source gap is needed for a long caption.
pub struct AnchorQuery {
    source: Span,
    label: Span,
    count: Option<ShapeQuery>,
    select: Option<ShapeSelect>,
    answer: Option<usize>,
}
impl AnchorQuery {
    pub fn new(source: Span, label: Span) -> Self {
        Self {
            source,
            label,
            count: None,
            select: None,
            answer: None,
        }
    }
    pub fn advance(
        &mut self,
        d: &Document,
        links: &SharedLinks,
        b: &mut Budget,
    ) -> Result<Option<usize>, LayoutError> {
        if self.answer.is_some() {
            return Ok(self.answer);
        }
        let shape = links.shape().ok_or(LayoutError::OutOfOrderUpdate)?;
        if self.select.is_none() {
            let q = self
                .count
                .get_or_insert_with(|| ShapeQuery::graphemes(&shape, self.label));
            let Some(n) = q.advance(&shape, d, links, b)? else {
                return Ok(None);
            };
            if n == 0 {
                self.answer = Some(self.source.start);
                return Ok(self.answer);
            }
            self.select = Some(ShapeSelect::new(&shape, self.label, n - 1));
            self.count = None;
        }
        let select = self.select.as_mut().unwrap();
        if select.advance(&shape, d, links, b)?.is_none() {
            return Ok(None);
        }
        self.answer = Some(
            select
                .selected_source()
                .ok_or(LayoutError::InvalidRange)?
                .start,
        );
        Ok(self.answer)
    }
}
