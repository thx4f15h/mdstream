use crate::{
    Budget, Document, LayoutError, LinkTarget, Span, SymbolId, TileId, TileKey, WorkLimit,
};

/// Direct semantic dependencies. Incoming layout state still needs a measurement
/// and checkpoint ticket; this key alone is not a layout dependency guarantee.
#[derive(Clone, Debug)]
pub struct DependencyKey {
    tile: TileId,
    key: TileKey,
    symbols: Vec<(SymbolId, u64)>,
    footnotes: Vec<(crate::FootnoteId, u64)>,
    finished: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CacheValidity {
    Valid,
    Changed,
    BudgetExhausted,
}

impl DependencyKey {
    /// Capture atomically within this call. If exhausted, no partial key is
    /// published; use DependencyCapture to preserve progress across frames.
    pub fn capture(
        document: &Document,
        tile: TileId,
        budget: &mut Budget,
    ) -> Result<Option<Self>, LayoutError> {
        let mut task = DependencyCapture::new(document, tile)?;
        match task.advance(document, budget)? {
            DependencyCaptureStep::Captured(key) => Ok(Some(key)),
            DependencyCaptureStep::BudgetExhausted => Ok(None),
            DependencyCaptureStep::Complete => unreachable!(),
        }
    }

    pub fn validate(
        &self,
        document: &Document,
        budget: &mut Budget,
    ) -> Result<CacheValidity, LayoutError> {
        if self.key.document != document.id() {
            return Err(LayoutError::WrongDocument);
        }
        if !budget.charge(WorkLimit {
            queries: 1,
            dependencies: self.symbols.len() + self.footnotes.len(),
            ..Default::default()
        }) {
            return Ok(CacheValidity::BudgetExhausted);
        }
        let Some(key) = document.tile_key(self.tile) else {
            return Ok(CacheValidity::Changed);
        };
        Ok(
            if key.source != self.key.source
                || key.syntax_revision != self.key.syntax_revision
                || self.footnotes.iter().any(|(id, version)| {
                    let note = document.footnote(*id).unwrap();
                    note.revision != *version
                        || (note.definition.is_none() && self.finished != document.is_finished())
                })
                || self
                    .symbols
                    .iter()
                    .any(|(id, version)| document.references()[id.0].revision != *version)
            {
                CacheValidity::Changed
            } else {
                CacheValidity::Valid
            },
        )
    }
}

/// A completed atomic key, a paid pause, or an already consumed result.
#[derive(Debug)]
pub enum DependencyCaptureStep {
    Captured(DependencyKey),
    BudgetExhausted,
    Complete,
}

/// Resumable dependency capture, including nested label content and remote
/// target guards. No label text is scanned or reparsed. A single conservative
/// label envelope is queried once, so deep ancestors never repeatedly traverse
/// the same descendants. Constructors allocate no document-sized buffers.
pub struct DependencyCapture {
    document: crate::DocumentId,
    revision: u64,
    tile: TileId,
    key: Option<TileKey>,
    phase: u8,
    window: Option<crate::LinkWindow>,
    current: Option<crate::LinkId>,
    activity: Option<crate::LinkActivity>,
    resolution: crate::link_semantics::Resolution,
    seen: std::collections::BTreeSet<crate::LinkId>,
    expanded: Option<Span>,
    foot_source: Span,
    foot_at: usize,
    footnotes: Vec<(crate::FootnoteId, u64)>,
}
impl DependencyCapture {
    pub fn new(document: &Document, tile: TileId) -> Result<Self, LayoutError> {
        if tile.0 >= document.tile_count() {
            return Err(LayoutError::InvalidRange);
        }
        Ok(Self {
            document: document.id(),
            revision: document.revision(),
            tile,
            key: None,
            phase: 0,
            window: None,
            current: None,
            activity: None,
            resolution: crate::link_semantics::Resolution::capturing(),
            seen: Default::default(),
            expanded: None,
            foot_source: Span::new(0, 0),
            foot_at: 0,
            footnotes: Vec::new(),
        })
    }
    pub fn memo_entries(&self) -> usize {
        self.seen.len() + self.resolution.memo_entries()
    }
    /// Cancel a resumable capture (including after Captured) and pay for its
    /// BTree memo/seen nodes before destruction. Copy-only buffers release once.
    pub fn retire(&mut self, budget: &mut Budget) -> crate::Completion {
        self.phase = 6;
        self.activity = None;
        self.window = None;
        if self.resolution.retire(budget) == crate::Completion::BudgetExhausted {
            return crate::Completion::BudgetExhausted;
        }
        while !self.seen.is_empty() {
            if !budget.charge(WorkLimit {
                queries: 1,
                objects: 1,
                ..Default::default()
            }) {
                return crate::Completion::BudgetExhausted;
            }
            self.seen.pop_first();
            self.resolution.work.retired_entries += 1;
        }
        crate::Completion::Covered
    }
    pub fn object_count(&self) -> usize {
        self.seen.len()
    }
    pub fn link_resolution_work(&self) -> crate::LinkResolutionWork {
        self.resolution.work
    }
    pub fn advance(
        &mut self,
        document: &Document,
        budget: &mut Budget,
    ) -> Result<DependencyCaptureStep, LayoutError> {
        if self.document != document.id() {
            return Err(LayoutError::WrongDocument);
        }
        if self.revision != document.revision() {
            return Err(LayoutError::StaleContinuation);
        }
        loop {
            match self.phase {
                0 => {
                    if !budget.charge(WorkLimit {
                        queries: 1,
                        ..Default::default()
                    }) {
                        return Ok(DependencyCaptureStep::BudgetExhausted);
                    }
                    let key = document.tile_key(self.tile).unwrap();
                    self.foot_source = key.source;
                    self.foot_at = document
                        .footnote_references()
                        .partition_point(|r| r.source.end <= key.source.start);
                    let mut window = document.link_window(key.source);
                    window.seed(document);
                    self.window = Some(window);
                    self.key = Some(key);
                    self.phase = 1;
                }
                1 | 2 => {
                    if let Some(id) = self.current {
                        if let Some(activity) = &mut self.activity {
                            if activity
                                .advance_in(document, &mut self.resolution, budget, 0)?
                                .is_none()
                            {
                                return Ok(DependencyCaptureStep::BudgetExhausted);
                            }
                            self.activity = None;
                            self.current = None;
                            continue;
                        }
                        if !budget.charge(WorkLimit {
                            dependencies: 1,
                            ..Default::default()
                        }) {
                            return Ok(DependencyCaptureStep::BudgetExhausted);
                        }
                        if let LinkTarget::Reference(symbol) = document.links()[id.0] {
                            self.resolution
                                .dependencies
                                .as_mut()
                                .unwrap()
                                .push((symbol, document.references()[symbol.0].revision));
                        }
                        if self.phase == 1 && document.link_order.nested(id) {
                            let label = document.link_info(id).unwrap().label;
                            self.expanded = Some(self.expanded.map_or(label, |old| {
                                Span::new(old.start.min(label.start), old.end.max(label.end))
                            }));
                        }
                        if !document.link_order.gate(id).unconditional() {
                            self.activity = document.link_activity(id);
                        } else {
                            self.current = None;
                        }
                        continue;
                    }
                    match self.window.as_mut().unwrap().advance(document, budget)? {
                        crate::LinkStep::Object(id, _) => {
                            if self.seen.insert(id) {
                                self.current = Some(id);
                            }
                        }
                        crate::LinkStep::BudgetExhausted => {
                            return Ok(DependencyCaptureStep::BudgetExhausted);
                        }
                        crate::LinkStep::Complete => {
                            let source = self.key.as_ref().unwrap().source;
                            if self.phase == 1
                                && self.expanded.is_some_and(|span| {
                                    span.start < source.start || source.end < span.end
                                })
                            {
                                self.phase = 4;
                            } else {
                                self.phase = 3;
                            }
                        }
                    }
                }
                4 => {
                    if !budget.charge(WorkLimit {
                        queries: 1,
                        ..Default::default()
                    }) {
                        return Ok(DependencyCaptureStep::BudgetExhausted);
                    }
                    let label = self.expanded.unwrap();
                    let source = self.key.as_ref().unwrap().source;
                    self.foot_source =
                        Span::new(label.start.min(source.start), label.end.max(source.end));
                    self.foot_at = document
                        .footnote_references()
                        .partition_point(|r| r.source.end <= self.foot_source.start);
                    let mut window = document.link_window(self.foot_source);
                    window.seed(document);
                    self.window = Some(window);
                    self.phase = 2;
                }
                3 => {
                    let entries = document.footnote_references();
                    if self.foot_at >= entries.len()
                        || entries[self.foot_at].source.start >= self.foot_source.end
                    {
                        self.phase = 5;
                        continue;
                    }
                    if !budget.charge(WorkLimit {
                        objects: 1,
                        dependencies: 1,
                        ..Default::default()
                    }) {
                        return Ok(DependencyCaptureStep::BudgetExhausted);
                    }
                    let reference = &entries[self.foot_at];
                    self.footnotes.push((
                        reference.symbol,
                        document.footnote(reference.symbol).unwrap().revision,
                    ));
                    self.foot_at += 1;
                }
                5 => {
                    self.phase = 6;
                    return Ok(DependencyCaptureStep::Captured(DependencyKey {
                        tile: self.tile,
                        key: self.key.take().unwrap(),
                        symbols: std::mem::take(self.resolution.dependencies.as_mut().unwrap()),
                        footnotes: std::mem::take(&mut self.footnotes),
                        finished: document.is_finished(),
                    }));
                }
                _ => return Ok(DependencyCaptureStep::Complete),
            }
        }
    }
}

/// Explicit bounded source copy for background jobs. This is source, not a
/// serialized semantic AST. Publish asynchronous heights using a measurement ticket.
#[derive(Clone, Debug)]
pub struct SourceSnapshot {
    pub document: crate::DocumentId,
    pub revision: u64,
    pub source: Span,
    pub text: String,
}

impl Document {
    pub fn snapshot(
        &self,
        source: Span,
        max_bytes: usize,
        budget: &mut Budget,
    ) -> Result<Option<SourceSnapshot>, LayoutError> {
        let text = self.text(source).ok_or(LayoutError::InvalidRange)?;
        if source.len() > max_bytes
            || !budget.charge(WorkLimit {
                bytes: source.len(),
                objects: 1,
                queries: 1,
                ..Default::default()
            })
        {
            return Ok(None);
        }
        Ok(Some(SourceSnapshot {
            document: self.id(),
            revision: self.revision(),
            source,
            text: text.to_owned(),
        }))
    }
}

#[cfg(test)]
mod reference_boundary_tests {
    use super::*;

    #[test]
    fn marker_and_suffix_only_tiles_pay_before_looking_up_the_current_target() {
        let mut doc = Document::new();
        let label = "b".repeat(999);
        doc.append(&format!(
            "{}[a][{label}]\n\n",
            "x".repeat(crate::TILE_BYTES - 1)
        ))
        .unwrap();
        let id = crate::LinkId(0);
        let info = *doc.link_info(id).unwrap();
        let LinkTarget::Reference(b) = doc.links()[0] else {
            panic!()
        };
        assert_eq!(doc.references()[b.0].label, label);
        let mut keys = Vec::new();
        for tile in [TileId(0), TileId(2), TileId(3)] {
            let source = doc.tile(tile).unwrap();
            assert!(source.end <= info.label.start || info.label.end <= source.start);
            assert_eq!(
                doc.links_in(source).map(|(id, _)| id).collect::<Vec<_>>(),
                [id]
            );
            let key = DependencyKey::capture(&doc, tile, &mut Budget::new(WorkLimit::unlimited()))
                .unwrap()
                .unwrap();
            assert_eq!(key.symbols, [(b, 0)]);
            keys.push(key);
        }
        // Empty references intentionally make an early lookup panic. All three
        // failure charges must return before indexing the winning symbol.
        let references = std::mem::take(&mut doc.references.entries);
        for limit in [
            WorkLimit {
                queries: 0,
                ..WorkLimit::unlimited()
            },
            WorkLimit {
                objects: 0,
                ..WorkLimit::unlimited()
            },
            WorkLimit {
                dependencies: 0,
                ..WorkLimit::unlimited()
            },
        ] {
            assert!(
                DependencyKey::capture(&doc, TileId(2), &mut Budget::new(limit))
                    .unwrap()
                    .is_none()
            );
        }
        doc.references.entries = references;
        doc.append("[a]: /old\n").unwrap();
        assert_eq!(doc.link_destination(id), None);
        for key in &keys {
            assert_eq!(
                key.validate(&doc, &mut Budget::new(WorkLimit::unlimited())),
                Ok(CacheValidity::Valid)
            );
        }
        doc.append(&format!("[{label}]: /new\n")).unwrap();
        assert_eq!(doc.link_destination(id), Some("/new"));
        assert_eq!(doc.link_info(id), Some(&info));
        for key in &keys {
            assert_eq!(
                key.validate(&doc, &mut Budget::new(WorkLimit::unlimited())),
                Ok(CacheValidity::Changed)
            );
        }
    }
}
