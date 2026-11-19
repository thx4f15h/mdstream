//! Pure per-view wishes, without renderer, heights, checkpoints or source text.
use super::table_row_folds::TableRowWish;
use super::*;

#[derive(Clone, Copy)]
pub(super) struct Counts([usize; 4]);
impl Counts {
    pub(super) fn new(d: &Document) -> Self {
        Self([
            d.headings().entries().len(),
            d.blocks().len(),
            d.tables().len(),
            d.container_count(),
        ])
    }
}

/// Lightweight wishes for one logical view. Source registration IDs remain
/// with this transferred view; no old layout ticket or checkpoint is retained.
/// Capture and restoration walk only explicit wishes and newly indexed syntax.
/// Stable row IDs, extents, choices and the private new-only row policy transfer;
/// physical indexes/groups are rebuilt against the new owner and current input.
pub struct FoldViewIntent {
    document: DocumentId,
    defaults: FoldDefaults,
    collapsed: BTreeSet<FoldId>,
    sources: BTreeMap<SourceFoldId, SourceCandidate>,
    rows: BTreeMap<usize, TableRowRange>,
    stable_rows: BTreeMap<TableRowFoldId, TableRowWish>,
    row_policy: Option<crate::TableRowPolicy>,
    policy_next: usize,
    stable_ready: bool,
    excluded: BTreeSet<BlockId>,
    tasks: BTreeMap<ContainerId, bool>,
    tails: BTreeSet<SourceFoldId>,
    preview: Option<(FoldId, bool)>,
    phase: u8,
    preview_prepared: bool,
    known: Counts,
    initialized: bool,
    stale_sources: usize,
    target: Option<DocumentId>,
    retiring: bool,
}
pub struct FoldIntentCapture {
    owner: DocumentId,
    revision: u64,
    version: u64,
    phase: u8,
    fold: Option<FoldId>,
    source: Option<SourceFoldId>,
    row: Option<usize>,
    stable_row: Option<TableRowFoldId>,
    block: Option<BlockId>,
    task: Option<ContainerId>,
    tail: Option<SourceFoldId>,
    result: FoldViewIntent,
}
fn paid(b: &mut Budget) -> bool {
    b.charge(WorkLimit {
        queries: 1,
        objects: 1,
        ..Default::default()
    })
}
fn after<T: Copy>(key: Option<T>) -> Bound<T> {
    key.map_or(Bound::Unbounded, Bound::Excluded)
}
impl<S> FoldView<S> {
    pub fn capture_intent(&self) -> FoldIntentCapture {
        FoldIntentCapture {
            owner: self.heights.version().0,
            revision: self.revision,
            version: self.version,
            phase: 0,
            fold: None,
            source: None,
            row: None,
            stable_row: None,
            block: None,
            task: None,
            tail: None,
            result: FoldViewIntent {
                document: self.document,
                defaults: self.defaults,
                collapsed: BTreeSet::new(),
                sources: BTreeMap::new(),
                rows: BTreeMap::new(),
                stable_rows: BTreeMap::new(),
                row_policy: self.table_row_policy(),
                policy_next: self.known.0[2],
                stable_ready: false,
                excluded: BTreeSet::new(),
                tasks: BTreeMap::new(),
                tails: BTreeSet::new(),
                preview: self.preview,
                phase: 0,
                preview_prepared: false,
                known: self.known,
                initialized: false,
                stale_sources: 0,
                target: None,
                retiring: false,
            },
        }
    }
}
impl FoldIntentCapture {
    pub fn advance<S>(
        &mut self,
        view: &FoldView<S>,
        budget: &mut Budget,
    ) -> Result<Completion, LayoutError> {
        if view.document != self.result.document {
            return Err(LayoutError::WrongDocument);
        }
        if view.heights.version().0 != self.owner {
            return Err(LayoutError::InvalidRange);
        }
        if self.result.retiring {
            return Err(LayoutError::StaleContinuation);
        }
        if view.revision != self.revision || view.version != self.version {
            return Err(LayoutError::StaleContinuation);
        }
        while self.phase < 7 {
            if !paid(budget) {
                return Ok(Completion::BudgetExhausted);
            }
            match self.phase {
                0 => {
                    if let Some((&id, _)) = view
                        .state
                        .collapsed
                        .range((after(self.fold), Bound::Unbounded))
                        .next()
                    {
                        self.fold = Some(id);
                        self.result.collapsed.insert(id);
                    } else {
                        self.phase += 1;
                    }
                }
                1 => {
                    if let Some((&id, &entry)) = view
                        .state
                        .source_candidates
                        .range((after(self.source), Bound::Unbounded))
                        .next()
                    {
                        self.source = Some(id);
                        self.result.sources.insert(id, entry);
                    } else {
                        self.phase += 1;
                    }
                }
                2 => {
                    if let Some((&id, &entry)) = view
                        .state
                        .row_ranges
                        .range((after(self.row), Bound::Unbounded))
                        .next()
                    {
                        self.row = Some(id);
                        self.result.rows.insert(id, entry);
                    } else {
                        self.phase += 1;
                    }
                }
                3 => {
                    if let Some(&id) = view
                        .paragraph_auto_excluded
                        .range((after(self.block), Bound::Unbounded))
                        .next()
                    {
                        self.block = Some(id);
                        self.result.excluded.insert(id);
                    } else {
                        self.phase += 1;
                    }
                }
                4 => {
                    if let Some((&id, &entry)) = view
                        .task_overrides
                        .range((after(self.task), Bound::Unbounded))
                        .next()
                    {
                        self.task = Some(id);
                        self.result.tasks.insert(id, entry);
                    } else {
                        self.phase += 1;
                    }
                }
                5 => {
                    if let Some(&id) = view
                        .state
                        .open_source_tails
                        .range((after(self.tail), Bound::Unbounded))
                        .next()
                    {
                        self.tail = Some(id);
                        self.result.tails.insert(id);
                    } else {
                        self.phase += 1;
                    }
                }
                _ => {
                    if let Some((id, wish)) = view.state.table_rows.next_wish(self.stable_row) {
                        self.stable_row = Some(id);
                        self.result.stable_rows.insert(id, wish);
                    } else {
                        self.phase += 1;
                    }
                }
            }
        }
        Ok(Completion::Covered)
    }
    pub fn finish(self) -> Result<FoldViewIntent, Box<Self>> {
        if self.phase != 7 || self.result.retiring {
            return Err(Box::new(self));
        }
        Ok(self.result)
    }
    pub fn retire(&mut self, budget: &mut Budget) -> Completion {
        self.result.retire(budget)
    }
}
impl FoldViewIntent {
    pub fn is_empty(&self) -> bool {
        self.records() == 0 && self.defaults == FoldDefaults::default() && self.row_policy.is_none()
    }
    pub fn document(&self) -> DocumentId {
        self.document
    }
    pub fn stale_sources(&self) -> usize {
        self.stale_sources
    }
    /// Explicit diagnostic record count, excluding map node slack/allocator RSS.
    pub fn records(&self) -> usize {
        self.collapsed.len()
            + self.sources.len()
            + self.rows.len()
            + self.stable_rows.len()
            + self.excluded.len()
            + self.tasks.len()
            + self.tails.len()
            + usize::from(self.preview.is_some())
    }
    pub fn retire(&mut self, budget: &mut Budget) -> Completion {
        self.retiring = true;
        while self.records() != 0 {
            if !paid(budget) {
                return Completion::BudgetExhausted;
            }
            let _ = self.collapsed.pop_first().is_some()
                || self.sources.pop_first().is_some()
                || self.rows.pop_first().is_some()
                || self.stable_rows.pop_first().is_some()
                || self.excluded.pop_first().is_some()
                || self.tasks.pop_first().is_some()
                || self.tails.pop_first().is_some()
                || self.preview.take().is_some();
        }
        Completion::Covered
    }
    /// The caller keeps this restore exclusive until Covered. Actual input
    /// Updates may be applied to the same view between steps, including EOF;
    /// user choice/default/preview mutations wait until restoration completes.
    /// A second view, even of the same document/version, is rejected.
    pub fn restore<S>(
        &mut self,
        doc: &Document,
        view: &mut FoldView<S>,
        budget: &mut Budget,
    ) -> Result<Completion, LayoutError> {
        if doc.id() != self.document || view.document != self.document {
            return Err(LayoutError::WrongDocument);
        }
        if self.retiring {
            return Err(LayoutError::StaleContinuation);
        }
        view.check(doc)?;
        let identity = view.heights.version().0;
        if self.target.is_some_and(|target| target != identity) {
            return Err(LayoutError::InvalidRange);
        }
        if !self.initialized {
            if view.has_collapsed()
                || view.source_fold_count() != 0
                || view.table_row_range_count() != 0
                || !view.state.table_rows.is_empty()
                || view.table_row_policy().is_some()
            {
                return Err(LayoutError::InvalidRange);
            }
            if !paid(budget) {
                return Ok(Completion::BudgetExhausted);
            }
            view.set_defaults(self.defaults);
            view.restore_row_policy(self.row_policy);
            self.initialized = true;
            self.target = Some(identity);
        }
        if !self.stable_ready {
            while !self.stable_rows.is_empty() {
                if !paid(budget) {
                    return Ok(Completion::BudgetExhausted);
                }
                let (id, wish) = self.stable_rows.pop_first().unwrap();
                if !view.restore_row_wish(doc, id, wish)? {
                    self.stale_sources += 1;
                }
            }
            self.stable_ready = true;
        }
        if let Some(policy) = self.row_policy {
            while self.policy_next < doc.tables().len() {
                if !paid(budget) {
                    return Ok(Completion::BudgetExhausted);
                }
                let table = TableId(self.policy_next);
                self.policy_next += 1;
                view.apply_new_row_policy(doc, table, policy)?;
            }
        }
        while self.phase < 10 {
            if self.phase == 9 {
                if let Some((id, choice)) = self.preview {
                    if view.candidate(doc, id).is_none() {
                        self.preview = None;
                        continue;
                    }
                    if !self.preview_prepared {
                        if !paid(budget) {
                            return Ok(Completion::BudgetExhausted);
                        }
                        view.set_collapsed(doc, id, choice)?;
                        self.preview_prepared = true;
                    }
                    if view.set_preview(doc, Some(id), budget)?.completion
                        == Completion::BudgetExhausted
                    {
                        return Ok(Completion::BudgetExhausted);
                    }
                    self.preview = None;
                }
                self.phase += 1;
                continue;
            }
            if !paid(budget) {
                return Ok(Completion::BudgetExhausted);
            }
            match self.phase {
                0 => {
                    let Some((id, mut entry)) = self.sources.pop_first() else {
                        self.phase += 1;
                        continue;
                    };
                    if let Some(block) = entry.paragraph {
                        let Some((source, open)) = doc.open_paragraph_source(block) else {
                            self.stale_sources += 1;
                            continue;
                        };
                        entry.candidate.source = source;
                        entry.candidate.hidden.start = entry.candidate.hidden.start.min(source.end);
                        entry.candidate.hidden.end = source.end;
                        entry.candidate.summary =
                            Some(Span::new(source.start, entry.candidate.hidden.start));
                        entry.candidate.open = open;
                    } else if self.tails.contains(&id) {
                        entry.candidate.source.end = doc.source().len();
                        entry.candidate.hidden.end = doc.source().len();
                        entry.candidate.open = !doc.is_input_sealed();
                    }
                    if entry.candidate.source.end > doc.source().len() {
                        self.stale_sources += 1;
                        continue;
                    }
                    let key = FoldState::source_key(entry);
                    view.state.source_ranges.insert(key);
                    if !entry.flexible {
                        view.state.strict_source_ranges.insert(key);
                    }
                    if entry.candidate.open && entry.paragraph.is_some() {
                        view.state.open_paragraph = Some(id);
                    }
                    if self.tails.remove(&id) {
                        view.state.open_source_tails.insert(id);
                    }
                    view.state.source_candidates.insert(id, entry);
                    view.version += 1;
                }
                1 => {
                    if let Some((at, rows)) = self.rows.pop_first() {
                        if doc.fold_candidate(FoldId::TableRows(rows)).is_some() {
                            view.state.row_ranges.insert(at, rows);
                        }
                    } else {
                        self.phase += 1;
                    }
                }
                2 => {
                    if let Some(id) = self.excluded.pop_first() {
                        if doc.open_paragraph_source(id).is_some() {
                            view.paragraph_auto_excluded.insert(id);
                        }
                    } else {
                        self.phase += 1;
                    }
                }
                3 => {
                    if let Some((id, value)) = self.tasks.pop_first() {
                        if matches!(
                            doc.container(id).map(|c| c.kind),
                            Some(ContainerKind::Item { task: Some(_), .. })
                        ) {
                            view.set_task_checked(doc, id, value)?;
                        }
                    } else {
                        self.phase += 1;
                    }
                }
                4 => {
                    if let Some(id) = self.collapsed.pop_first() {
                        if view.candidate(doc, id).is_some() {
                            view.set_collapsed(doc, id, true)?;
                        }
                    } else {
                        self.phase += 1;
                    }
                }
                5..=8 => {
                    let domain = usize::from(self.phase - 5);
                    let now = Counts::new(doc).0[domain];
                    let at = self.known.0[domain];
                    if at >= now {
                        self.phase += 1;
                        continue;
                    }
                    self.known.0[domain] += 1;
                    if let Some(candidate) = default_candidate(doc, self.defaults, domain, at) {
                        view.set_collapsed(doc, candidate.id, true)?;
                    }
                }
                _ => unreachable!(),
            }
        }
        // Tails whose paragraphs ceased to exist are discarded as paid records.
        while !self.tails.is_empty() {
            if !paid(budget) {
                return Ok(Completion::BudgetExhausted);
            }
            self.tails.pop_first();
        }
        Ok(Completion::Covered)
    }
}
impl std::fmt::Debug for FoldIntentCapture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FoldIntentCapture")
            .field("phase", &self.phase)
            .field("records", &self.result.records())
            .finish()
    }
}
fn default_candidate(
    d: &Document,
    defaults: FoldDefaults,
    domain: usize,
    at: usize,
) -> Option<FoldCandidate> {
    let id = match domain {
        0 => {
            let heading = d.headings().get(HeadingId(at))?;
            if !defaults.heading_levels[usize::from(heading.level - 1)] {
                return None;
            }
            FoldId::Heading(HeadingId(at))
        }
        1 => match d.blocks()[at].kind {
            BlockKind::Code { .. } if defaults.code => FoldId::Code(BlockId(at)),
            BlockKind::Math { .. } if defaults.math => FoldId::Math(BlockId(at)),
            _ => return None,
        },
        2 => {
            if !defaults.table {
                return None;
            }
            FoldId::Table(TableId(at))
        }
        _ => {
            let c = d.container(ContainerId(at))?;
            let checked = match c.kind {
                ContainerKind::Item { task, .. } => task.map(|t| t.checked),
                _ => None,
            };
            if !defaults
                .container_depth
                .is_some_and(|depth| c.depth >= depth)
                && !defaults.containers.matches(c.kind, checked)
            {
                return None;
            }
            FoldId::Container(ContainerId(at))
        }
    };
    d.fold_candidate(id)
}
