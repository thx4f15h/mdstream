//! Paid safe body-prefix cuts over the canonical display and native indexes.
//! Manual source folds deliberately retain their separate fixed-Span contract.
use super::{objects::SharedLinks, shape::ShapeWindow};
use mdtui3::{
    Budget, Document, DocumentId, LayoutError, LinkActivity, LinkDisposition, LinkWindow,
    MathWindow, Span, WorkLimit,
};

fn paid(budget: &mut Budget) -> bool {
    budget.charge(WorkLimit {
        queries: 1,
        objects: 1,
        ..Default::default()
    })
}

fn bind_owner(owner: &mut Option<SharedLinks>, links: &SharedLinks) -> Result<(), LayoutError> {
    if owner
        .as_ref()
        .is_some_and(|original| !original.same_owner(links))
    {
        return Err(LayoutError::WrongDocument);
    }
    if owner.is_none() {
        *owner = Some(links.clone());
    }
    Ok(())
}

/// Round a proposed source cut backwards to complete display EGC/replacement
/// and active link/image/math object boundaries. Stores metadata, never text.
/// Construction and zero budget do not prepare or read the paragraph.
pub struct SafePrefixCut {
    owner: Option<SharedLinks>,
    document: DocumentId,
    revision: u64,
    source: Span,
    cut: usize,
    round: usize,
    phase: u8,
    glyphs: Option<ShapeWindow>,
    objects: Option<LinkWindow>,
    activity: Option<(Span, LinkActivity)>,
    math: Option<MathWindow>,
    done: bool,
    dependency_end: usize,
    previous_count: Option<super::shape::ShapeQuery>,
    previous: Option<super::shape::ShapeCluster>,
}
impl SafePrefixCut {
    pub fn new(doc: &Document, source: Span, cut: usize) -> Result<Self, LayoutError> {
        if doc.text(source).is_none()
            || cut < source.start
            || cut > source.end
            || !doc.source().is_char_boundary(cut)
        {
            return Err(LayoutError::InvalidRange);
        }
        Ok(Self {
            owner: None,
            document: doc.id(),
            revision: doc.revision(),
            source,
            cut,
            round: cut,
            phase: 0,
            glyphs: None,
            objects: None,
            activity: None,
            math: None,
            done: cut == source.start || cut == source.end,
            dependency_end: cut,
            previous_count: None,
            previous: None,
        })
    }
    fn dependency_end(&self) -> usize {
        self.dependency_end
    }
    pub fn advance(
        &mut self,
        doc: &Document,
        links: &SharedLinks,
        budget: &mut Budget,
    ) -> Result<Option<usize>, LayoutError> {
        if doc.id() != self.document {
            return Err(LayoutError::WrongDocument);
        }
        if doc.revision() != self.revision {
            return Err(LayoutError::StaleContinuation);
        }
        // Validate the supplied resolver even for cached/empty answers.
        bind_owner(&mut self.owner, links)?;
        if !links.prepare(doc, budget)? {
            return Ok(None);
        }
        if self.done {
            return Ok(Some(self.cut));
        }
        if !links.prepare_shape_indexed(doc, budget)? {
            return Ok(None);
        }
        let shape = links.shape().ok_or(LayoutError::InvalidRange)?;
        loop {
            match self.phase {
                0 => {
                    if !paid(budget) {
                        return Ok(None);
                    }
                    self.round = self.cut;
                    // The actual renderer's physical block is the GCB barrier.
                    let mut start = self.cut - 1;
                    while !doc.source().is_char_boundary(start) {
                        start -= 1;
                    }
                    let fragment = doc
                        .block_at(start)
                        .and_then(|id| doc.block(id))
                        .map_or(self.source, |b| b.source);
                    let mut fragment = Span::new(
                        fragment.start.max(self.source.start),
                        fragment.end.min(self.source.end),
                    );
                    // Native objects reset the actual scanner's EGC context;
                    // raw TeX delimiters must not absorb a following combiner.
                    if let Some(meta) = doc.math_object_before(self.cut)
                        && meta.source.end <= self.cut
                    {
                        fragment.start = fragment.start.max(meta.source.end);
                    }
                    if let Some(meta) = doc.math_object_at_or_after(self.cut)
                        && meta.source.start >= self.cut
                    {
                        fragment.end = fragment.end.min(meta.source.start);
                    }
                    start = start.max(fragment.start);
                    self.glyphs = Some(ShapeWindow::new(
                        &shape,
                        Span::new(start, self.cut),
                        fragment,
                    )?);
                    self.objects = Some(doc.link_window(Span::new(start, self.cut)));
                    self.math = Some(doc.math_window(Span::new(start, self.cut))?);
                    self.phase = 1;
                }
                1 => match self
                    .glyphs
                    .as_mut()
                    .unwrap()
                    .advance(&shape, doc, links, budget)?
                {
                    super::shape::ShapeWindowStep::BudgetExhausted => return Ok(None),
                    super::shape::ShapeWindowStep::Glyph(g) => {
                        self.dependency_end = self.dependency_end.max(g.source.end);
                        if g.source.start < self.round && g.source.end > self.round {
                            self.cut = self.cut.min(g.source.start);
                        }
                    }
                    super::shape::ShapeWindowStep::Complete => {
                        self.glyphs = None;
                        self.phase = 2;
                    }
                },
                2 => {
                    if let Some((source, activity)) = &mut self.activity {
                        let Some(disposition) =
                            links.with_resolver(|r| activity.advance_with(doc, r, budget))?
                        else {
                            return Ok(None);
                        };
                        if disposition == LinkDisposition::Active {
                            self.cut = self.cut.min(source.start);
                        }
                        self.activity = None;
                        continue;
                    }
                    match self.objects.as_mut().unwrap().advance(doc, budget)? {
                        mdtui3::LinkStep::BudgetExhausted => return Ok(None),
                        mdtui3::LinkStep::Object(id, info) => {
                            self.dependency_end = self.dependency_end.max(info.source.end);
                            if info.source.start < self.round && info.source.end > self.round {
                                self.activity = Some((
                                    info.source,
                                    doc.link_activity(id).ok_or(LayoutError::InvalidRange)?,
                                ));
                            }
                        }
                        mdtui3::LinkStep::Complete => {
                            self.objects = None;
                            self.phase = 3;
                        }
                    }
                }
                3 => match self.math.as_mut().unwrap().advance(doc, budget)? {
                    mdtui3::MathWindowStep::BudgetExhausted => return Ok(None),
                    mdtui3::MathWindowStep::Object(meta) => {
                        self.dependency_end = self.dependency_end.max(meta.source.end);
                        if meta.source.start < self.round && meta.source.end > self.round {
                            self.cut = self.cut.min(meta.source.start);
                        }
                    }
                    mdtui3::MathWindowStep::Complete => {
                        self.math = None;
                        self.phase = 4;
                    }
                },
                4 => {
                    if !paid(budget) {
                        return Ok(None);
                    }
                    if self.cut == self.round || self.cut <= self.source.start {
                        self.cut = self.cut.max(self.source.start);
                        self.done = true;
                        return Ok(Some(self.cut));
                    }
                    self.previous_count = Some(super::shape::ShapeQuery::graphemes(
                        &shape,
                        Span::new(self.source.start, self.cut),
                    ));
                    self.phase = 5;
                }
                5 => {
                    let Some(count) = self
                        .previous_count
                        .as_mut()
                        .unwrap()
                        .advance(&shape, doc, links, budget)?
                    else {
                        return Ok(None);
                    };
                    self.previous_count = None;
                    if count == 0 {
                        self.cut = self.source.start;
                        self.phase = 4;
                    } else {
                        self.previous = Some(super::shape::ShapeCluster::new(
                            &shape,
                            self.source,
                            count - 1,
                        ));
                        self.phase = 6;
                    }
                }
                _ => {
                    let Some(glyph) = self
                        .previous
                        .as_mut()
                        .unwrap()
                        .advance(&shape, doc, links, budget)?
                    else {
                        return Ok(None);
                    };
                    self.cut = self.cut.min(glyph.source.end);
                    self.previous = None;
                    self.phase = 0;
                }
            }
        }
    }
}

/// LF-delimited canonical display lines (the DisplayExcerpt line convention).
/// Unlike a summary this selects original body source; no display byte limit or
/// arbitrary two-line cap is substituted for the requested logical line count.
pub struct LogicalPrefix {
    owner: Option<SharedLinks>,
    document: DocumentId,
    revision: u64,
    source: Span,
    cursor: mdtui3::DisplayCursor,
    remaining: usize,
    safe: Option<SafePrefixCut>,
    result: Option<usize>,
    source_work: usize,
}
impl LogicalPrefix {
    pub fn new(doc: &Document, source: Span, lines: usize) -> Result<Self, LayoutError> {
        Ok(Self {
            owner: None,
            document: doc.id(),
            revision: doc.revision(),
            source,
            cursor: doc
                .display_cursor(source)
                .ok_or(LayoutError::InvalidRange)?,
            remaining: lines,
            safe: None,
            result: (lines == 0).then_some(source.start),
            source_work: 0,
        })
    }
    pub(crate) fn dependency_end(&self) -> usize {
        self.cursor
            .source_position()
            .max(self.safe.as_ref().map_or(0, |s| s.dependency_end()))
            .min(self.source.end)
    }
    pub(crate) fn extend_append(
        &mut self,
        doc: &Document,
        source: Span,
    ) -> Result<(), LayoutError> {
        if doc.id() != self.document {
            return Err(LayoutError::WrongDocument);
        }
        if source.start != self.source.start
            || source.end < self.source.end
            || doc.text(source).is_none()
        {
            return Err(LayoutError::InvalidRange);
        }
        let old = self.source.end;
        self.revision = doc.revision();
        self.source = source;
        self.result = None;
        if self.remaining == 0 {
            self.safe = Some(SafePrefixCut::new(doc, source, old)?);
        } else {
            self.safe = None;
            self.cursor = doc
                .display_cursor(Span::new(old, source.end))
                .ok_or(LayoutError::InvalidRange)?;
        }
        Ok(())
    }
    pub fn source_work(&self) -> usize {
        self.source_work
    }
    pub fn advance(
        &mut self,
        doc: &Document,
        links: &SharedLinks,
        budget: &mut Budget,
    ) -> Result<Option<usize>, LayoutError> {
        if doc.id() != self.document {
            return Err(LayoutError::WrongDocument);
        }
        if doc.revision() != self.revision {
            return Err(LayoutError::StaleContinuation);
        }
        bind_owner(&mut self.owner, links)?;
        if !links.prepare(doc, budget)? {
            return Ok(None);
        }
        if let Some(cut) = self.result {
            return Ok(Some(cut));
        }
        loop {
            if let Some(safe) = &mut self.safe {
                self.result = safe.advance(doc, links, budget)?;
                return Ok(self.result);
            }
            let before = budget.used().bytes;
            let step = links.with_resolver(|r| self.cursor.next_with_resolver(doc, r, budget))?;
            self.source_work += budget.used().bytes - before;
            match step {
                mdtui3::DisplayStep::BudgetExhausted => return Ok(None),
                mdtui3::DisplayStep::Complete => {
                    self.result = Some(self.source.end);
                    return Ok(self.result);
                }
                mdtui3::DisplayStep::Run(run) => {
                    if run.style.contains(mdtui3::Style::HIDDEN) {
                        continue;
                    }
                    for (at, ch) in run.text.char_indices() {
                        if ch == '\n' {
                            self.remaining -= 1;
                            if self.remaining == 0 {
                                let cut =
                                    run.replacement.map_or(run.source.start + at + 1, |s| s.end);
                                self.safe = Some(SafePrefixCut::new(doc, self.source, cut)?);
                                break;
                            }
                        }
                    }
                }
            }
        }
    }
}

/// A body-prefix selected by the same flow/word placement state used by the
/// production sink. Canonical scanner quanta and individual placement steps
/// resume through Budget; no terminal grid or whole body is buffered.
pub struct DisplayPrefix {
    stream: Box<super::prefix_flow::PrefixFlow>,
    owner: Option<SharedLinks>,
    document: DocumentId,
    revision: u64,
    source: Span,
    safe: Option<SafePrefixCut>,
    result: Option<usize>,
    source_work: usize,
    prepared_cells: usize,
    probes: usize,
}
impl DisplayPrefix {
    pub fn new(doc: &Document, source: Span, rows: usize, width: u16) -> Result<Self, LayoutError> {
        if width == 0 || doc.text(source).is_none() {
            return Err(LayoutError::InvalidRange);
        }
        Ok(Self {
            stream: Box::new(super::prefix_flow::PrefixFlow::new(
                doc, source, width, rows,
            )),
            owner: None,
            document: doc.id(),
            revision: doc.revision(),
            source,
            safe: None,
            result: (rows == 0).then_some(source.start),
            source_work: 0,
            prepared_cells: 0,
            probes: 0,
        })
    }
    pub(crate) fn can_extend(&self) -> bool {
        self.stream.can_extend()
    }
    pub(crate) fn extend_append(
        &mut self,
        doc: &Document,
        source: Span,
    ) -> Result<(), LayoutError> {
        if doc.id() != self.document {
            return Err(LayoutError::WrongDocument);
        }
        self.stream.extend_append(source)?;
        self.source = source;
        self.revision = doc.revision();
        self.result = None;
        self.safe = None;
        Ok(())
    }
    pub(crate) fn dependency_end(&self) -> usize {
        self.stream
            .dependency_end()
            .max(self.safe.as_ref().map_or(0, |s| s.dependency_end()))
            .min(self.source.end)
    }
    pub fn source_work(&self) -> usize {
        self.source_work
    }
    pub fn prepared_cells(&self) -> usize {
        self.prepared_cells
    }
    pub fn probes(&self) -> usize {
        self.probes
    }
    pub fn advance(
        &mut self,
        doc: &Document,
        links: &SharedLinks,
        budget: &mut Budget,
    ) -> Result<Option<usize>, super::first_frame::FirstFrameError> {
        if doc.id() != self.document {
            return Err(LayoutError::WrongDocument.into());
        }
        if doc.revision() != self.revision {
            return Err(LayoutError::StaleContinuation.into());
        }
        bind_owner(&mut self.owner, links)?;
        if !links.prepare(doc, budget)? {
            return Ok(None);
        }
        if let Some(cut) = self.result {
            return Ok(Some(cut));
        }
        if !links.prepare_shape_indexed(doc, budget)? {
            return Ok(None);
        }
        loop {
            if let Some(safe) = &mut self.safe {
                self.result = safe.advance(doc, links, budget)?;
                return Ok(self.result);
            }
            let before = budget.used().bytes;
            let result = self.stream.advance(doc, links, budget)?;
            self.source_work += budget.used().bytes - before;
            self.prepared_cells = self.stream.prepared_atoms;
            self.probes = self.stream.quanta;
            let Some(cut) = result else {
                return Ok(None);
            };
            if cut == self.source.end {
                self.result = Some(cut);
                return Ok(self.result);
            }
            self.safe = Some(SafePrefixCut::new(doc, self.source, cut)?);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrefixLines {
    Logical(usize),
    Display(usize),
}
