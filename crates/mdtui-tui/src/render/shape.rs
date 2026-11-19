//! Incremental source-property and conditional display index. This consumes
//! typed core effects; queries never lex Markdown or decode an old source prefix.
use super::{
    append_store::AppendStore,
    objects::SharedLinks,
    shape_unicode::{GcbState, GcbSummary, Properties, WidthSummary},
};
use mdtui3::{
    Budget, Completion, DisplayShapeDomain, DisplayShapeEffectKind as Effect,
    DisplayShapeMark as Mark, DisplayShapeReplacement as Replacement, Document, DocumentId,
    LayoutError, LinkActivity, LinkDisposition, ResolvedStyleSpan, Span, Style, StyleMarkerStep,
    StyleMarkers, Update, WorkLimit,
};
use std::{cell::RefCell, collections::BTreeMap, rc::Rc};

const LEAF: usize = 64;
fn paid(b: &mut Budget) -> bool {
    b.charge(WorkLimit {
        queries: 1,
        objects: 1,
        ..WorkLimit::default()
    })
}
fn blank() -> Mark {
    Mark {
        style: Style::default(),
        base_hidden: false,
        delimiter_hidden: false,
        emphasis_depth: 0,
        strong_depth: 0,
        link: None,
        hide_owner: None,
        hide_alternative: None,
        object_syntax: false,
        order: 0,
    }
}
fn merge(a: Mark, b: Mark) -> Mark {
    let replace = b.link.is_some()
        && if a.object_syntax && b.object_syntax {
            b.link < a.link || b.link == a.link && b.order >= a.order
        } else {
            b.order >= a.order
        };
    Mark {
        style: a.style.union(b.style),
        base_hidden: a.base_hidden || b.base_hidden,
        delimiter_hidden: a.delimiter_hidden || b.delimiter_hidden,
        emphasis_depth: a.emphasis_depth + b.emphasis_depth,
        strong_depth: a.strong_depth + b.strong_depth,
        link: if replace { b.link } else { a.link },
        hide_owner: a.hide_owner.or(b.hide_owner),
        hide_alternative: a.hide_alternative.or(b.hide_alternative),
        object_syntax: if replace {
            b.object_syntax
        } else {
            a.object_syntax
        },
        order: a.order.max(b.order),
    }
}
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ShapeWork {
    pub source_decode_bytes: usize,
    pub classified_scalars: usize,
    pub source_run_reads: usize,
    pub source_index_nodes: usize,
    pub native_source_capacity_bytes: usize,
    pub source_indexed_bytes: usize,
    pub source_summary_joins: usize,
    pub html_state_steps: usize,
    pub input_node_visits: usize,
    pub property_replays: usize,
    pub width_state_transitions: usize,
    pub gcb_state_transitions: usize,
    pub query_nodes: usize,
    pub query_properties: usize,
    pub reader_node_visits: usize,
    pub reader_leaf_visits: usize,
    pub reader_atom_reads: usize,
    pub reader_properties: usize,
    pub summary_hits: usize,
    pub recovered_guards: usize,
    pub scope_dependency_reads: usize,
    pub index_nodes: usize,
    pub index_chunk_allocations: usize,
    pub index_reserved_bytes: usize,
    pub index_relocated_items: usize,
    pub style_spans_seen: usize,
    pub style_buffer_peak_entries: usize,
    pub style_buffer_peak_bytes: usize,
    pub overlay_node_visits: usize,
    pub overlay_scalar_writes: usize,
    pub seed_steps: usize,
    pub seed_undo_copies: usize,
    pub seed_annotation_nodes: usize,
    pub seed_retired_nodes: usize,
    pub presentation_hits: usize,
    pub presentation_publications: usize,
    pub object_index_visits: u64,
    pub object_sort_steps: u64,
    pub object_candidates: u64,
}
#[derive(Clone, Copy)]
struct RawAtom {
    source: Span,
    ch: char,
    raw: Properties,
    code: Properties,
    crlf: bool,
}
impl RawAtom {
    fn code_source(self) -> Span {
        if self.crlf {
            Span::new(self.source.start - 1, self.source.end)
        } else {
            self.source
        }
    }
    fn displayed(self, code: bool) -> Option<(char, Properties)> {
        if self.ch == '\t' || code && matches!(self.ch, '\r' | '\n') {
            return Some((
                ' ',
                if self.ch == '\t' {
                    Properties::of(' ')
                } else {
                    self.code
                },
            ));
        }
        if self.ch.is_control() && !matches!(self.ch, '\r' | '\n') {
            return None;
        }
        Some((self.ch, self.raw))
    }
}
#[derive(Clone)]
struct Summary {
    gcb: GcbSummary,
    width: WidthSummary,
    bytes: usize,
    scalars: usize,
    ascii: bool,
    first: Option<Span>,
    last: Option<Span>,
}
impl Default for Summary {
    fn default() -> Self {
        Self {
            gcb: GcbSummary::default(),
            width: WidthSummary::default(),
            bytes: 0,
            scalars: 0,
            ascii: true,
            first: None,
            last: None,
        }
    }
}
impl Summary {
    fn push(&mut self, p: Properties, ch: char, source: Span) {
        self.gcb.push(p);
        self.width.push(p);
        self.bytes += ch.len_utf8();
        self.scalars += 1;
        self.ascii &= ch.is_ascii();
        self.first = self.first.or(Some(source));
        self.last = Some(source);
    }
    fn join(a: &Self, b: &Self) -> Self {
        Self {
            gcb: GcbSummary::join(&a.gcb, &b.gcb),
            width: WidthSummary::join(&a.width, &b.width),
            bytes: a.bytes + b.bytes,
            scalars: a.scalars + b.scalars,
            ascii: a.ascii && b.ascii,
            first: a.first.or(b.first),
            last: b.last.or(a.last),
        }
    }
    fn answer(&self, kind: QueryKind, input: u8) -> Answer {
        let (out, value) = match kind {
            QueryKind::Gcb => (
                self.gcb.out[usize::from(input)],
                self.gcb.breaks[usize::from(input)] as i64,
            ),
            QueryKind::Width => (
                self.width.out[usize::from(input)],
                self.width.delta[usize::from(input)],
            ),
        };
        Answer {
            out,
            value,
            bytes: self.bytes,
            scalars: self.scalars,
            first: self.first,
            last: self.last,
        }
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
struct Entity {
    number: Option<usize>,
    chars: [char; 8],
    properties: [Properties; 8],
    scalars: u8,
    source: Span,
}
impl Entity {
    fn character(self, at: usize) -> char {
        if let Some(number) = self.number {
            let power = 10usize.pow(u32::from(self.scalars) - at as u32 - 1);
            ['⁰', '¹', '²', '³', '⁴', '⁵', '⁶', '⁷', '⁸', '⁹'][(number / power) % 10]
        } else {
            self.chars[at]
        }
    }
    fn property(self, at: usize) -> Properties {
        if self.number.is_some() {
            Properties::of(self.character(at))
        } else {
            self.properties[at]
        }
    }
}
#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct Layer {
    code: bool,
    trim: bool,
    entity: Option<Entity>,
    skip: bool,
}
#[derive(Clone, Copy)]
struct Decision {
    epoch: usize,
    semantic: u64,
    hidden: bool,
    context: Context,
    replacement: Option<Entity>,
}
#[derive(Clone, Copy)]
struct StyleTag {
    epoch: usize,
    snapshot: ResolvedStyleSpan,
}
struct Node {
    lo: usize,
    hi: usize,
    parent: Option<usize>,
    children: Option<[usize; 2]>,
    raw: Summary,
    code: Summary,
    mark: Mark,
    saved: Option<Mark>,
    layer: Layer,
    spaces: u8,
    literal: Option<mdtui3::LinkId>,
    literal_lazy: Option<Option<mdtui3::LinkId>>,
    image: Option<mdtui3::LinkId>,
    footnote: Option<mdtui3::FootnoteReferenceId>,
    footnote_lazy: Option<Option<mdtui3::FootnoteReferenceId>>,
    foot_state: Option<(usize, u64, bool)>,
    facts: [bool; 3],
    base_changed: bool,
    descendant_base: bool,
    candidate: bool,
    html: Option<usize>,
    descendant_html: bool,
    permanent: bool,
    descendant_permanent: bool,
    hints: bool,
    descendant_special: bool,
    replacement_descendants: bool,
    unicode_replacement_descendants: bool,
    changed: bool,
    descendant_changes: bool,
    generation: u64,
    semantic: u64,
    semantic_max: u64,
    decision: Option<Decision>,
    cache: Option<Box<Cache>>,
    resolved_style: Option<StyleTag>,
    style_cells: Option<Box<[Option<StyleTag>; LEAF]>>,
}
impl Node {
    fn has_special(&self) -> bool {
        self.literal.is_some() || self.image.is_some() || self.footnote.is_some()
    }
    fn unicode_replacement(&self) -> bool {
        self.layer
            .entity
            .is_some_and(|e| (0..usize::from(e.scalars)).any(|at| !e.character(at).is_ascii()))
    }
    fn has_replacement(&self) -> bool {
        self.layer != Layer::default() || self.spaces != 0
    }
    fn own_changes(&self, context: Context) -> bool {
        self.html.is_some()
            || (if context.literal {
                self.has_replacement()
            } else {
                self.base_changed
            })
            || !context.literal_bound && self.literal.is_some()
            || !context.literal && self.image.is_some()
            || !context.foot_bound && self.footnote.is_some()
    }
    fn subtree_special(&self, context: Context) -> bool {
        !context.literal_bound && self.facts[0]
            || !context.literal && self.facts[1]
            || !context.foot_bound && self.facts[2]
    }
}
#[derive(Clone, Copy)]
enum SemanticFact {
    Literal(Option<mdtui3::LinkId>),
    Image(Option<mdtui3::LinkId>),
    Footnote(Option<mdtui3::FootnoteReferenceId>),
    Candidate,
    Html(usize),
}
struct Cache {
    generation: u64,
    epoch: usize,
    mode: Context,
    gcb: [Option<Answer>; 24],
    width: [Option<Answer>; 132],
}
impl Cache {
    fn new(generation: u64, epoch: usize, mode: Context) -> Self {
        Self {
            generation,
            epoch,
            mode,
            gcb: [None; 24],
            width: [None; 132],
        }
    }
}
#[derive(Clone, Copy)]
struct ScalarPresentation {
    epoch: usize,
    version: u64,
    style: crate::screen::StyleRole,
    link: Option<mdtui3::LinkId>,
}
struct ReplacementUndo {
    nodes: Vec<(usize, Layer)>,
}

/// Shared by the actual input owner, display, geometry and payload readers.
/// This is one current semantic index; no per-revision display snapshots.
#[derive(Clone)]
pub struct SharedShape(pub(crate) Rc<RefCell<ShapeIndex>>);
pub struct ShapeIndex {
    document: DocumentId,
    revision: u64,
    sequence: usize,
    end: usize,
    epoch: usize,
    semantic_clock: u64,
    pending_code: Option<Span>,
    html_scan: super::html_shape::HtmlShape,
    html: BTreeMap<usize, Span>,
    conditional_scopes: BTreeMap<usize, Span>,
    space: Properties,
    atoms: AppendStore<RawAtom>,
    native_source: Option<mdtui3::DisplaySource>,
    source_scalars: usize,
    positions: AppendStore<usize>,
    nodes: AppendStore<Node>,
    root: usize,
    capacity: usize,
    undo: Option<AppendStore<usize>>,
    replacements: BTreeMap<usize, ReplacementUndo>,
    objects: BTreeMap<mdtui3::LinkId, (mdtui3::LinkInfo, bool)>,
    presentations: BTreeMap<usize, ScalarPresentation>,
    footnotes: BTreeMap<mdtui3::FootnoteReferenceId, Span>,
    fallback: Entity,
    work: ShapeWork,
    pub(crate) generation: u64,
}
/// One-time budgeted adoption of current core metadata. Source scalars are
/// classified exactly once; the native checkpoint supplies real rollback state.
pub struct ShapeSeed {
    document: DocumentId,
    owner: Option<SharedShape>,
    revision: u64,
    source_at: usize,
    indexed: bool,
    source_ready: bool,
    html_ready: bool,
    html_after: Option<usize>,
    core: mdtui3::DisplayShapeSeed,
    done: bool,
}
impl ShapeSeed {
    pub(crate) fn retire(
        &mut self,
        shape: &SharedShape,
        budget: &mut Budget,
    ) -> mdtui3::Completion {
        self.done = true;
        let before = self.core.work();
        let completion = self.core.retire(budget);
        let after = self.core.work();
        let mut index = shape.0.borrow_mut();
        index.work.seed_steps += after.steps - before.steps;
        index.work.seed_retired_nodes += after.retired_nodes - before.retired_nodes;
        completion
    }
    pub fn new(document: &Document) -> Self {
        Self {
            document: document.id(),
            owner: None,
            revision: document.revision(),
            source_at: 0,
            indexed: false,
            source_ready: false,
            html_ready: false,
            html_after: None,
            core: document.display_shape_seed(),
            done: false,
        }
    }
    /// Adopt input-maintained source facts without decoding an unrelated prefix.
    /// Native metadata/rollback adoption is identical to the ordinary seed.
    pub(crate) fn indexed(document: &Document) -> Self {
        Self {
            indexed: true,
            ..Self::new(document)
        }
    }
    fn advance_source_index(
        &mut self,
        shape: &SharedShape,
        document: &Document,
        budget: &mut Budget,
    ) -> Result<bool, LayoutError> {
        if !self.source_ready {
            if !paid(budget) {
                return Ok(false);
            }
            let native = document.display_source();
            let mut index = shape.0.borrow_mut();
            index.source_scalars = native.len();
            index.end = native.bytes();
            index.positions[0] = index.end;
            index.work.source_indexed_bytes = index.end;
            index.html_scan = native.html_state();
            index.native_source = Some(native);
            index.capacity = index.source_scalars.max(LEAF).next_power_of_two();
            let root = index.root;
            index.nodes[root].hi = index.capacity;
            let count = index.source_scalars;
            index.nodes[root].raw = index.source_summary(0, count, false);
            index.nodes[root].code = index.source_summary(0, count, true);
            self.source_at = index.end;
            self.source_ready = true;
        }
        while !self.html_ready {
            if !paid(budget) {
                return Ok(false);
            }
            let mut index = shape.0.borrow_mut();
            let native = index.native_source.as_ref().unwrap();
            if let Some(span) = native.next_html(self.html_after) {
                index.html.insert(span.start, span);
                index.fact(span, SemanticFact::Html(span.start))?;
                self.html_after = Some(span.start);
            } else {
                self.html_ready = true;
            }
        }
        Ok(true)
    }
    pub fn advance(
        &mut self,
        shape: &SharedShape,
        document: &Document,
        budget: &mut Budget,
    ) -> Result<bool, LayoutError> {
        if self.document != document.id() {
            return Err(LayoutError::WrongDocument);
        }
        shape.check_document(document)?;
        if self
            .owner
            .as_ref()
            .is_some_and(|owner| !owner.same_index(shape))
        {
            return Err(LayoutError::WrongDocument);
        }
        if self.revision != document.revision() {
            return Err(LayoutError::StaleContinuation);
        }
        if shape.applied_revision() != self.revision {
            return Err(LayoutError::StaleContinuation);
        }
        if self.done {
            return Ok(true);
        }
        self.owner.get_or_insert_with(|| shape.clone());
        if self.indexed && !self.advance_source_index(shape, document, budget)? {
            return Ok(false);
        }
        while self.source_at < document.source().len() {
            let mut end =
                (self.source_at + budget.remaining().bytes.min(256)).min(document.source().len());
            while !document.source().is_char_boundary(end) {
                end -= 1;
            }
            if end == self.source_at
                || !budget.charge(WorkLimit {
                    queries: 1,
                    objects: 1,
                    bytes: end - self.source_at,
                    ..Default::default()
                })
            {
                return Ok(false);
            }
            shape
                .0
                .borrow_mut()
                .append(document.source(), Span::new(self.source_at, end))?;
            self.source_at = end;
        }
        loop {
            let before = self.core.work();
            let advanced = self.core.advance(document, budget)?;
            let after = self.core.work();
            {
                let mut index = shape.0.borrow_mut();
                index.work.seed_steps += after.steps - before.steps;
                index.work.seed_undo_copies += after.undo_copies - before.undo_copies;
                index.work.seed_annotation_nodes +=
                    after.annotation_nodes - before.annotation_nodes;
                index.work.seed_retired_nodes += after.retired_nodes - before.retired_nodes;
            }
            let item = match advanced {
                Ok(Some(item)) => item,
                Ok(None) => {
                    self.done = true;
                    return Ok(true);
                }
                Err(_) => return Ok(false),
            };
            let mut index = shape.0.borrow_mut();
            match item {
                mdtui3::DisplayShapeSeedItem::AnnotationCheckpoint => {
                    index.undo = Some(AppendStore::default())
                }
                mdtui3::DisplayShapeSeedItem::Mark {
                    source,
                    mark,
                    baseline,
                } => {
                    let lo = index.lower_bound(source.start);
                    let hi = index.lower_bound(source.end).min(index.atom_count());
                    let root = index.root;
                    if baseline {
                        index.seed_baseline(root, lo, hi, mark);
                    } else if mark != blank() {
                        index.mark_in(root, lo, hi, mark);
                    }
                }
                mdtui3::DisplayShapeSeedItem::Replacement {
                    source,
                    replacement,
                } => index.replace(source, replacement)?,
                mdtui3::DisplayShapeSeedItem::Candidate {
                    source,
                    conditional_scope,
                } => {
                    index.work.scope_dependency_reads += 1;
                    if let Some(scope) = conditional_scope {
                        index.conditional_scopes.insert(scope.start, scope);
                    }
                    index.fact(source, SemanticFact::Candidate)?
                }
                mdtui3::DisplayShapeSeedItem::Link { id, info, active } => {
                    index.put_object(id, info, active)?
                }
                mdtui3::DisplayShapeSeedItem::FootnoteReference { id, source } => {
                    index.fact(source, SemanticFact::Footnote(Some(id)))?;
                    index.footnotes.insert(id, source);
                }
            }
        }
    }
}

impl SharedShape {
    pub(crate) fn document_id(&self) -> DocumentId {
        self.0.borrow().document
    }
    pub(crate) fn same_index(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
    fn check_document(&self, document: &Document) -> Result<(), LayoutError> {
        if self.document_id() != document.id() {
            return Err(LayoutError::WrongDocument);
        }
        Ok(())
    }
    fn check_context(
        &self,
        original: &Self,
        document: &Document,
        links: &SharedLinks,
    ) -> Result<(), LayoutError> {
        if !self.same_index(original) {
            return Err(LayoutError::WrongDocument);
        }
        self.check_document(document)?;
        links.check_shape_context(document, self)
    }
    pub fn new(document: &Document) -> Result<Self, LayoutError> {
        if !document.source().is_empty() {
            return Err(LayoutError::InvalidRange);
        }
        Ok(Self(Rc::new(RefCell::new(ShapeIndex::new(document)))))
    }
    /// Empty property storage bound to the current native metadata revision.
    /// Complete ShapeSeed::advance before querying or consuming future Updates;
    /// construction neither classifies source nor reconstructs event history.
    pub fn from_current(document: &Document) -> Self {
        let mut index = ShapeIndex::new(document);
        let stats = document.display_shape_stats();
        index.sequence = stats.emitted;
        index.epoch = stats.binding_epoch + 1;
        Self(Rc::new(RefCell::new(index)))
    }
    pub(crate) fn applied_revision(&self) -> u64 {
        self.0.borrow().revision
    }
    pub fn apply(&self, document: &Document, update: &Update) -> Result<(), LayoutError> {
        self.0.borrow_mut().apply(document, update)
    }
    pub(crate) fn invalidate_host_binding(&self) {
        let mut index = self.0.borrow_mut();
        index.epoch += 1;
        index.semantic_clock += 1;
        index.generation += 1;
    }
    /// Retire one actual index/undo record per paid step. The native source
    /// remains owned by the input parser, so releasing this handle is constant.
    pub(crate) fn retire_index(&self, budget: &mut Budget) -> Completion {
        let mut index = self.0.borrow_mut();
        loop {
            if !paid(budget) {
                return Completion::BudgetExhausted;
            }
            if let Some((_, undo)) = index.replacements.first_key_value() {
                if !undo.nodes.is_empty() {
                    index
                        .replacements
                        .first_entry()
                        .unwrap()
                        .get_mut()
                        .nodes
                        .pop();
                } else {
                    index.replacements.pop_first();
                }
            } else if let Some(undo) = &mut index.undo {
                if undo.pop().is_none() {
                    index.undo = None;
                }
            } else if index.nodes.pop().is_some()
                || index.atoms.pop().is_some()
                || index.positions.pop().is_some()
                || index.objects.pop_first().is_some()
                || index.presentations.pop_first().is_some()
                || index.footnotes.pop_first().is_some()
                || index.html.pop_first().is_some()
                || index.conditional_scopes.pop_first().is_some()
            {
            } else {
                index.native_source = None;
                return Completion::Covered;
            }
        }
    }
    pub fn work(&self) -> ShapeWork {
        let index = self.0.borrow();
        let mut work = index.work;
        work.native_source_capacity_bytes = index
            .native_source
            .as_ref()
            .map_or(0, |s| s.stats().capacity_bytes);
        work.index_nodes = index.nodes.len();
        work.index_chunk_allocations = index.atoms.allocations()
            + index.positions.allocations()
            + index.nodes.allocations()
            + index.undo.as_ref().map_or(0, |u| u.allocations());
        work.index_reserved_bytes = index.atoms.capacity() * std::mem::size_of::<RawAtom>()
            + index.positions.capacity() * std::mem::size_of::<usize>()
            + index.nodes.capacity() * std::mem::size_of::<Node>()
            + index
                .undo
                .as_ref()
                .map_or(0, |u| u.capacity() * std::mem::size_of::<usize>());
        work
    }
    pub(crate) fn payload_snapshot(&self, source: Span) -> (usize, u64, u64) {
        let index = self.0.borrow();
        (
            index.payload_epoch(source),
            index.semantic_version(source),
            index.semantic_clock,
        )
    }
    /// The legacy ASCII path has only the bounded CRLF join. Any Unicode or
    /// display replacement uses the complete conditional cluster index.
    pub(crate) fn requires_shape(&self, source: Span) -> bool {
        fn visit(index: &ShapeIndex, id: usize, lo: usize, hi: usize) -> bool {
            let n = &index.nodes[id];
            if hi <= n.lo || n.hi <= lo {
                return false;
            }
            if lo <= n.lo && n.hi <= hi {
                return !n.raw.ascii
                    || n.unicode_replacement()
                    || n.unicode_replacement_descendants;
            }
            if n.unicode_replacement() {
                return true;
            }
            if let Some(children) = n.children {
                children
                    .into_iter()
                    .any(|child| visit(index, child, lo, hi))
            } else {
                !n.raw.ascii
            }
        }
        let index = self.0.borrow();
        let lo = index.lower_bound(source.start);
        let hi = index.lower_bound(source.end);
        visit(&index, index.root, lo, hi)
    }
    pub(crate) fn semantic_clock(&self) -> u64 {
        self.0.borrow().semantic_clock
    }
    pub fn binding_epoch(&self) -> usize {
        self.0.borrow().epoch
    }
    pub fn source_len(&self) -> usize {
        self.0.borrow().end
    }
    pub fn capacity_bytes(&self) -> usize {
        self.0.borrow().capacity_bytes()
    }
}
impl ShapeIndex {
    fn new(document: &Document) -> Self {
        let mut result = Self {
            document: document.id(),
            revision: document.revision(),
            sequence: 0,
            end: 0,
            epoch: 1,
            semantic_clock: 0,
            pending_code: document.indented_code_pending().map(|p| p.source),
            html_scan: Default::default(),
            html: BTreeMap::new(),
            conditional_scopes: BTreeMap::new(),
            space: Properties::of(' '),
            atoms: AppendStore::default(),
            native_source: None,
            source_scalars: 0,
            positions: AppendStore::default(),
            nodes: AppendStore::default(),
            root: 0,
            capacity: LEAF,
            undo: None,
            replacements: BTreeMap::new(),
            objects: BTreeMap::new(),
            presentations: BTreeMap::new(),
            footnotes: BTreeMap::new(),
            fallback: Entity {
                number: None,
                chars: ['[', 'i', 'm', 'a', 'g', 'e', ']', '\0'],
                properties: std::array::from_fn(|i| {
                    Properties::of("[image]\0".chars().nth(i).unwrap())
                }),
                scalars: 7,
                source: Span::new(0, 0),
            },
            work: ShapeWork::default(),
            generation: 0,
        };
        result.positions.push(0);
        result.root = result.node(0, LEAF, None);
        result
    }
    fn source_summary(&mut self, lo: usize, hi: usize, code: bool) -> Summary {
        let mut work = mdtui3::DisplaySourceWork::default();
        let answer = self
            .native_source
            .as_ref()
            .unwrap()
            .summary(lo, hi, code, &mut work);
        self.work.source_index_nodes += work.query_nodes;
        self.work.source_run_reads += work.query_runs;
        self.work.source_summary_joins += work.summary_joins;
        self.work.gcb_state_transitions += work.gcb_state_transitions;
        self.work.width_state_transitions += work.width_state_transitions;
        Summary {
            gcb: answer.gcb,
            width: answer.width,
            bytes: answer.bytes,
            scalars: answer.scalars,
            ascii: answer.ascii,
            first: answer.first,
            last: answer.last,
        }
    }
    fn atom_count(&self) -> usize {
        self.source_scalars + self.atoms.len()
    }
    fn atom(&self, at: usize) -> RawAtom {
        if at < self.source_scalars {
            let atom = self.native_source.as_ref().unwrap().atom(at);
            RawAtom {
                source: atom.source,
                ch: atom.scalar,
                raw: atom.raw,
                code: atom.code,
                crlf: atom.crlf,
            }
        } else {
            self.atoms[at - self.source_scalars]
        }
    }
    fn position(&self, at: usize) -> usize {
        if at < self.source_scalars {
            self.native_source.as_ref().unwrap().position(at)
        } else {
            self.positions[at - self.source_scalars]
        }
    }
    fn lower_bound(&self, byte: usize) -> usize {
        if byte < self.positions[0] {
            self.native_source.as_ref().unwrap().lower_bound(byte)
        } else {
            self.source_scalars + self.positions.partition_point(|&p| p < byte)
        }
    }
    fn node(&mut self, lo: usize, hi: usize, parent: Option<usize>) -> usize {
        let id = self.nodes.len();
        self.nodes.push(Node {
            lo,
            hi,
            parent,
            children: None,
            raw: Summary::default(),
            code: Summary::default(),
            mark: blank(),
            saved: None,
            layer: Layer::default(),
            spaces: 0,
            literal: None,
            literal_lazy: None,
            image: None,
            footnote: None,
            footnote_lazy: None,
            foot_state: None,
            facts: [false; 3],
            base_changed: false,
            descendant_base: false,
            candidate: false,
            html: None,
            descendant_html: false,
            permanent: false,
            descendant_permanent: false,
            hints: false,
            descendant_special: false,
            replacement_descendants: false,
            unicode_replacement_descendants: false,
            changed: false,
            descendant_changes: false,
            generation: 0,
            semantic: 0,
            semantic_max: 0,
            decision: None,
            cache: None,
            resolved_style: None,
            style_cells: None,
        });
        id
    }
    fn bump(&mut self, id: usize) {
        self.nodes[id].generation = self.nodes[id].generation.wrapping_add(1);
    }
    fn refresh(&mut self, id: usize) {
        if let Some([a, b]) = self.nodes[id].children {
            self.work.width_state_transitions += 264;
            self.work.gcb_state_transitions += 48;
            self.nodes[id].raw = Summary::join(&self.nodes[a].raw, &self.nodes[b].raw);
            self.nodes[id].code = Summary::join(&self.nodes[a].code, &self.nodes[b].code);
            self.nodes[id].descendant_changes = self.nodes[a].changed
                || self.nodes[a].descendant_changes
                || self.nodes[b].changed
                || self.nodes[b].descendant_changes;
            self.nodes[id].descendant_special = self.nodes[a].has_special()
                || self.nodes[a].descendant_special
                || self.nodes[b].has_special()
                || self.nodes[b].descendant_special;
            self.nodes[id].replacement_descendants = self.nodes[a].has_replacement()
                || self.nodes[a].replacement_descendants
                || self.nodes[b].has_replacement()
                || self.nodes[b].replacement_descendants;
            self.nodes[id].unicode_replacement_descendants = self.nodes[a].unicode_replacement()
                || self.nodes[a].unicode_replacement_descendants
                || self.nodes[b].unicode_replacement()
                || self.nodes[b].unicode_replacement_descendants;
        }
        let child_max = self.nodes[id].children.map_or(0, |[a, b]| {
            self.nodes[a].semantic_max.max(self.nodes[b].semantic_max)
        });
        self.nodes[id].semantic_max = self.nodes[id].semantic.max(child_max);
        self.refresh_facts(id);
        self.bump(id);
    }
    fn append(&mut self, source: &str, span: Span) -> Result<(), LayoutError> {
        if span.start != self.end {
            return Err(LayoutError::StaleContinuation);
        }
        let old_count = self.atom_count();
        let mut html = Vec::new();
        for (offset, ch) in source[span.range()].char_indices() {
            let start = span.start + offset;
            if let Some(span) = self.html_scan.push(start, ch) {
                html.push(span);
            }
            let raw = Properties::of(ch);
            let code = if matches!(ch, '\r' | '\n') {
                self.space
            } else {
                raw
            };
            let crlf = ch == '\n'
                && if self.source_scalars == 0 {
                    self.atoms.last().is_some_and(|a| a.ch == '\r')
                } else {
                    self.atom_count()
                        .checked_sub(1)
                        .is_some_and(|at| self.atom(at).ch == '\r')
                };
            self.atoms.push(RawAtom {
                crlf,
                source: Span::new(start, start + ch.len_utf8()),
                ch,
                raw,
                code,
            });
            self.positions.push(start + ch.len_utf8());
            self.work.source_decode_bytes += ch.len_utf8();
            self.work.classified_scalars += 1;
        }
        self.finish_append(old_count, span.end);
        if let Some(span) = self.html_scan.open(self.end) {
            html.push(span);
        }
        for source in html {
            self.append_html(source)?;
        }
        Ok(())
    }
    fn append_indexed(&mut self, span: Span) -> Result<(), LayoutError> {
        if span.start != self.end || self.atoms.len() != 0 {
            return Err(LayoutError::StaleContinuation);
        }
        let native = self.native_source.as_ref().unwrap().clone();
        let count = native.lower_bound(span.end);
        if count > native.len() || native.position(count) != span.end {
            return Err(LayoutError::InvalidRange);
        }
        let old_count = self.source_scalars;
        self.source_scalars = count;
        self.positions[0] = span.end;
        self.work.source_indexed_bytes += span.len();
        self.finish_append(old_count, span.end);
        // The input index has already classified these scalars and HTML spans.
        // Include the last old span: an open tag can grow across this append.
        // Merged Updates may expose later native atoms, so clip to this event.
        let mut after = self
            .html
            .last_key_value()
            .and_then(|(&at, _)| at.checked_sub(1));
        while let Some(mut source) = native.next_html(after) {
            if source.start >= self.end {
                break;
            }
            after = Some(source.start);
            source.end = source.end.min(self.end);
            self.append_html(source)?;
        }
        self.html_scan = native.html_state();
        Ok(())
    }
    fn finish_append(&mut self, old_count: usize, end: usize) {
        while self.atom_count() > self.capacity {
            let old = self.root;
            let root = self.node(0, self.capacity * 2, None);
            let right = self.node(self.capacity, self.capacity * 2, Some(root));
            self.nodes[old].parent = Some(root);
            self.nodes[root].children = Some([old, right]);
            self.root = root;
            self.capacity *= 2;
            self.refresh(root);
        }
        if old_count < self.atom_count() {
            self.append_range(self.root, old_count, self.atom_count(), old_count);
        }
        self.end = end;
    }
    fn append_html(&mut self, source: Span) -> Result<(), LayoutError> {
        let old = self.html.insert(source.start, source);
        let begin = old.map_or(source.start, |span| span.end);
        if begin < source.end {
            self.fact(
                Span::new(begin, source.end),
                SemanticFact::Html(source.start),
            )?;
        }
        Ok(())
    }
    fn append_range(&mut self, id: usize, lo: usize, hi: usize, old_count: usize) {
        self.work.input_node_visits += 1;
        if hi <= self.nodes[id].lo || self.nodes[id].hi <= lo {
            return;
        }
        if self.nodes[id].children.is_none() && self.nodes[id].hi - self.nodes[id].lo <= LEAF {
            for at in lo.max(self.nodes[id].lo)..hi.min(self.nodes[id].hi) {
                let a = self.atom(at);
                self.work.width_state_transitions += 264;
                self.work.gcb_state_transitions += 48;
                if let Some((ch, p)) = a.displayed(false) {
                    self.nodes[id].raw.push(p, ch, a.source);
                }
                if let Some((ch, p)) = a.displayed(true) {
                    self.nodes[id].code.push(p, ch, a.code_source());
                }
            }
            self.bump(id);
            return;
        }
        self.split_to(id, old_count);
        for child in self.nodes[id].children.unwrap() {
            self.append_range(child, lo, hi, old_count);
        }
        self.refresh(id);
    }
    fn split(&mut self, id: usize) {
        self.split_to(id, self.atom_count());
    }
    fn split_to(&mut self, id: usize, classified: usize) {
        if self.nodes[id].children.is_some() {
            return;
        }
        let (lo, hi) = (self.nodes[id].lo, self.nodes[id].hi);
        assert!(hi - lo > 1);
        let mid = (lo + hi) / 2;
        let a = self.node(lo, mid, Some(id));
        let b = self.node(mid, hi, Some(id));
        for child in [a, b] {
            let lo = self.nodes[child].lo;
            let hi = self.nodes[child]
                .hi
                .min(classified)
                .min(self.source_scalars);
            if lo < hi {
                self.nodes[child].raw = self.source_summary(lo, hi, false);
                self.nodes[child].code = self.source_summary(lo, hi, true);
            }
            for at in
                self.nodes[child].lo.max(self.source_scalars)..self.nodes[child].hi.min(classified)
            {
                let atom = self.atom(at);
                if let Some((ch, p)) = atom.displayed(false) {
                    self.nodes[child].raw.push(p, ch, atom.source);
                }
                if let Some((ch, p)) = atom.displayed(true) {
                    self.nodes[child].code.push(p, ch, atom.code_source());
                }
                self.work.property_replays += 1;
                self.work.width_state_transitions += 264;
                self.work.gcb_state_transitions += 48;
            }
        }
        if let Some(at) = self.nodes[id].html {
            self.nodes[a].html = Some(at);
            self.nodes[b].html = Some(at);
            self.nodes[id].html = None;
            self.refresh_facts(a);
            self.refresh_facts(b);
        }
        self.nodes[id].children = Some([a, b]);
    }
    fn scalar(&self, byte: usize) -> Result<usize, LayoutError> {
        if self.native_source.is_none() {
            return self
                .positions
                .binary_search(&byte)
                .map_err(|_| LayoutError::InvalidRange);
        }
        {
            let at = self.lower_bound(byte);
            if at <= self.atom_count() && self.position(at) == byte {
                Ok(at)
            } else {
                Err(LayoutError::InvalidRange)
            }
        }
    }
    fn mark(&mut self, source: Span, mark: Mark) -> Result<(), LayoutError> {
        if mark == blank() {
            return Ok(());
        }
        let lo = self.scalar(source.start)?;
        let hi = self.scalar(source.end)?;
        self.mark_in(self.root, lo, hi, mark);
        Ok(())
    }
    fn seed_baseline(&mut self, id: usize, lo: usize, hi: usize, mark: Mark) {
        self.work.input_node_visits += 1;
        if hi <= self.nodes[id].lo || self.nodes[id].hi <= lo || mark == blank() {
            return;
        }
        if lo <= self.nodes[id].lo && self.nodes[id].hi <= hi {
            if self.nodes[id].saved.is_none() {
                self.undo.as_mut().unwrap().push(id);
            }
            self.nodes[id].saved = Some(merge(self.nodes[id].saved.unwrap_or_else(blank), mark));
            return;
        }
        self.split(id);
        for child in self.nodes[id].children.unwrap() {
            self.seed_baseline(child, lo, hi, mark);
        }
    }
    fn mark_in(&mut self, id: usize, lo: usize, hi: usize, mark: Mark) {
        self.work.input_node_visits += 1;
        if hi <= self.nodes[id].lo || self.nodes[id].hi <= lo {
            return;
        }
        if lo <= self.nodes[id].lo && self.nodes[id].hi <= hi {
            if let Some(undo) = &mut self.undo
                && self.nodes[id].saved.is_none()
            {
                self.nodes[id].saved = Some(self.nodes[id].mark);
                undo.push(id);
            }
            let merged = merge(self.nodes[id].mark, mark);
            if self.nodes[id].mark == merged {
                return;
            }
            self.nodes[id].mark = merged;
            self.set_changed(id);
            self.bump(id);
            return;
        }
        self.split(id);
        for child in self.nodes[id].children.unwrap() {
            self.mark_in(child, lo, hi, mark);
        }
        self.refresh(id);
    }
    fn refresh_facts(&mut self, id: usize) {
        let child = self.nodes[id].children;
        let descendants = std::array::from_fn::<_, 3, _>(|i| {
            child.is_some_and(|[a, b]| self.nodes[a].facts[i] || self.nodes[b].facts[i])
        });
        let descendant_base = child.is_some_and(|[a, b]| {
            self.nodes[a].base_changed
                || self.nodes[a].descendant_base
                || self.nodes[b].base_changed
                || self.nodes[b].descendant_base
        });
        let descendant_permanent = child.is_some_and(|[a, b]| {
            self.nodes[a].permanent
                || self.nodes[a].descendant_permanent
                || self.nodes[b].permanent
                || self.nodes[b].descendant_permanent
        });
        let child_hints = child.is_some_and(|[a, b]| self.nodes[a].hints || self.nodes[b].hints);
        let descendant_html = child.is_some_and(|[a, b]| {
            self.nodes[a].html.is_some()
                || self.nodes[a].descendant_html
                || self.nodes[b].html.is_some()
                || self.nodes[b].descendant_html
        });
        let n = &mut self.nodes[id];
        n.descendant_html = descendant_html;
        n.permanent = n.mark.base_hidden
            || n.mark.hide_owner.is_some()
            || n.mark.hide_alternative.is_some()
            || n.mark.style.contains(Style::SOFT_BREAK)
            || n.mark.style.contains(Style::HARD_BREAK)
            || n.has_replacement();
        n.base_changed = n.candidate
            || n.mark.base_hidden
            || n.mark.delimiter_hidden
            || n.mark.hide_owner.is_some()
            || n.mark.hide_alternative.is_some()
            || n.mark.style.contains(Style::SOFT_BREAK)
            || n.mark.style.contains(Style::HARD_BREAK)
            || n.has_replacement();
        n.descendant_base = descendant_base;
        n.descendant_permanent = descendant_permanent;
        n.hints = n.mark.delimiter_hidden || child_hints;
        n.facts = [
            n.literal_lazy
                .map_or(n.literal.is_some() || descendants[0], |v| v.is_some()),
            n.image.is_some() || descendants[1],
            n.footnote_lazy
                .map_or(n.footnote.is_some() || descendants[2], |v| v.is_some()),
        ];
        n.changed = n.base_changed || n.has_special() || n.html.is_some();
        n.descendant_changes = n.descendant_base || n.facts.iter().any(|&v| v);
        n.descendant_special = n.facts.iter().any(|&v| v);
    }
    fn set_changed(&mut self, id: usize) {
        self.semantic_clock += 1;
        self.nodes[id].semantic = self.semantic_clock;
        self.nodes[id].semantic_max = self.nodes[id].semantic_max.max(self.semantic_clock);
        self.refresh_facts(id);
    }
    fn push_fact(&mut self, id: usize, fact: SemanticFact) {
        let Some(children) = self.nodes[id].children else {
            return;
        };
        match fact {
            SemanticFact::Literal(_) => {
                if let Some(value) = self.nodes[id].literal_lazy.take() {
                    self.nodes[id].literal = None;
                    for child in children {
                        self.nodes[child].literal = value;
                        self.nodes[child].literal_lazy = Some(value);
                        self.refresh_facts(child);
                        self.bump(child);
                    }
                }
            }
            SemanticFact::Footnote(_) => {
                if let Some(value) = self.nodes[id].footnote_lazy.take() {
                    self.nodes[id].footnote = None;
                    for child in children {
                        self.nodes[child].footnote = value;
                        self.nodes[child].footnote_lazy = Some(value);
                        self.refresh_facts(child);
                        self.bump(child);
                    }
                }
            }
            SemanticFact::Image(_) | SemanticFact::Candidate | SemanticFact::Html(_) => {}
        }
    }
    fn invalidate_path(&mut self, mut id: usize) {
        self.set_changed(id);
        self.bump(id);
        while let Some(parent) = self.nodes[id].parent {
            self.refresh(parent);
            id = parent;
        }
    }
    fn layer_range(
        &mut self,
        id: usize,
        lo: usize,
        hi: usize,
        layer: Layer,
        undo: &mut Vec<(usize, Layer)>,
    ) {
        self.work.input_node_visits += 1;
        if hi <= self.nodes[id].lo || self.nodes[id].hi <= lo {
            return;
        }
        if lo <= self.nodes[id].lo && self.nodes[id].hi <= hi {
            undo.push((id, self.nodes[id].layer));
            self.nodes[id].layer = layer;
            self.set_changed(id);
            self.bump(id);
            return;
        }
        self.split(id);
        for child in self.nodes[id].children.unwrap() {
            self.layer_range(child, lo, hi, layer, undo);
        }
        self.refresh(id);
    }
    fn replace(&mut self, source: Span, replacement: Replacement) -> Result<(), LayoutError> {
        let lo = self.scalar(source.start)?;
        let hi = self.scalar(source.end)?;
        let mut undo = Vec::new();
        match replacement {
            Replacement::Spaces { columns } => {
                let id = self.atom_node(self.root, lo);
                self.nodes[id].spaces = columns;
                self.invalidate_path(id);
                return Ok(());
            }
            Replacement::Entity { utf8, len } => {
                let mut entity = Entity {
                    number: None,
                    chars: ['\0'; 8],
                    properties: [self.space; 8],
                    scalars: 0,
                    source,
                };
                for (i, ch) in std::str::from_utf8(&utf8[..usize::from(len)])
                    .unwrap()
                    .chars()
                    .enumerate()
                {
                    entity.chars[i] = ch;
                    entity.properties[i] = Properties::of(ch);
                    entity.scalars += 1;
                }

                self.layer_range(
                    self.root,
                    lo + 1,
                    hi,
                    Layer {
                        skip: true,
                        ..Layer::default()
                    },
                    &mut undo,
                );
                let id = self.atom_node(self.root, lo);
                undo.push((id, self.nodes[id].layer));
                self.nodes[id].layer = Layer {
                    entity: Some(entity),
                    ..Layer::default()
                };
                self.invalidate_path(id);
            }
            Replacement::Code { leading, trailing } => {
                self.layer_range(
                    self.root,
                    lo,
                    hi,
                    Layer {
                        code: true,
                        ..Layer::default()
                    },
                    &mut undo,
                );
                for trim in [leading, trailing] {
                    if !trim.is_empty() {
                        let a = self.scalar(trim.start)?;
                        let b = self.scalar(trim.end)?;
                        self.layer_range(
                            self.root,
                            a,
                            b,
                            Layer {
                                code: true,
                                trim: true,
                                ..Layer::default()
                            },
                            &mut undo,
                        );
                    }
                }
            }
        }
        self.replacements
            .insert(self.replacements.len(), ReplacementUndo { nodes: undo });
        Ok(())
    }
    fn atom_node(&mut self, id: usize, at: usize) -> usize {
        if self.nodes[id].hi - self.nodes[id].lo == 1 {
            return id;
        }
        self.split(id);
        let [a, b] = self.nodes[id].children.unwrap();
        let result = self.atom_node(if at < self.nodes[b].lo { a } else { b }, at);
        self.refresh(id);
        result
    }
    fn apply(&mut self, document: &Document, update: &Update) -> Result<(), LayoutError> {
        if document.id() != self.document || update.document != Some(self.document) {
            return Err(LayoutError::WrongDocument);
        }
        if !update.display_shape.enabled() || update.base_revision != self.revision {
            return Err(LayoutError::StaleContinuation);
        }
        let old_end = self.end;
        let mut scopes = std::collections::BTreeSet::new();
        let mut binding_changed = false;
        for event in update.display_shape.events() {
            if event.sequence != self.sequence + 1 {
                return Err(LayoutError::StaleContinuation);
            }
            self.sequence = event.sequence;
            match event.kind {
                Effect::ScopeExtent { scope, .. }
                | Effect::ScopeEvent { scope, .. }
                | Effect::Begin(DisplayShapeDomain::Styles { scope })
                | Effect::Commit(DisplayShapeDomain::Styles { scope })
                | Effect::Rollback(DisplayShapeDomain::Styles { scope }) => {
                    scopes.insert(scope);
                }
                _ => {}
            }
            binding_changed |= match event.kind {
                Effect::ScopeEvent { scope, .. } => scope < old_end,
                Effect::Link { info, .. } => info.source.start < old_end,
                Effect::LinkGate { id, .. }
                | Effect::LinkParent { id, .. }
                | Effect::LinkActive { id, .. }
                | Effect::LinkState { id, .. } => self
                    .objects
                    .get(&id)
                    .is_some_and(|(info, _)| info.source.start < old_end),
                Effect::Reference { id, .. } => document
                    .references()
                    .get(id.0)
                    .is_some_and(|r| r.first_use.is_some_and(|at| at < old_end)),
                Effect::Footnote { .. }
                | Effect::FootnoteReference { .. }
                | Effect::Math { .. }
                | Effect::Rollback(_) => true,
                _ => false,
            };
            match event.kind {
                Effect::SourceAppend(span) => {
                    if self.native_source.is_some() {
                        self.append_indexed(span)?;
                    } else {
                        self.append(document.source(), span)?;
                    }
                }
                Effect::Mark { source, mark } => self.mark(source, mark)?,
                Effect::Begin(DisplayShapeDomain::Annotations) => {
                    assert!(self.undo.is_none());
                    self.undo = Some(AppendStore::default());
                }
                Effect::Commit(DisplayShapeDomain::Annotations) => {
                    for id in self.undo.take().unwrap() {
                        self.nodes[id].saved = None;
                    }
                }
                Effect::Rollback(DisplayShapeDomain::Annotations) => {
                    for id in self.undo.take().unwrap() {
                        self.nodes[id].mark = self.nodes[id].saved.take().unwrap();
                        self.invalidate_path(id);
                    }
                }
                Effect::Replacement {
                    source,
                    replacement,
                } => self.replace(source, replacement)?,
                Effect::TruncateReplacements { entries } => {
                    while self.replacements.len() > entries {
                        for (id, layer) in self
                            .replacements
                            .pop_last()
                            .unwrap()
                            .1
                            .nodes
                            .into_iter()
                            .rev()
                        {
                            self.nodes[id].layer = layer;
                            self.invalidate_path(id);
                        }
                    }
                }
                Effect::ScopeEvent {
                    event: mdtui3::DisplayShapeStyleEvent::Run { source, .. },
                    ..
                } => {
                    self.fact(source, SemanticFact::Candidate)?;
                }
                Effect::Link { id, info, .. } => {
                    self.put_object(id, info, true)?;
                }
                Effect::LinkSource { id, source } => {
                    if let Some(&(mut info, active)) = self.objects.get(&id) {
                        info.source = source;
                        self.put_object(id, info, active)?;
                    }
                }
                Effect::LinkActive { id, active } => {
                    if let Some(&(info, _)) = self.objects.get(&id) {
                        self.put_object(id, info, active)?;
                    }
                }
                Effect::FootnoteReference {
                    id,
                    reference,
                    state,
                } => {
                    if let Some(old) = self.footnotes.remove(&id) {
                        self.fact(old, SemanticFact::Footnote(None))?;
                    }
                    if state.is_some() {
                        self.fact(reference.source, SemanticFact::Footnote(Some(id)))?;
                        self.footnotes.insert(id, reference.source);
                    }
                }
                _ => {}
            }
        }
        for scope in scopes {
            self.work.scope_dependency_reads += 1;
            if let Some(source) = document.display_shape_scope_dependency(scope) {
                self.conditional_scopes.insert(scope, source);
            } else {
                self.conditional_scopes.remove(&scope);
            }
        }
        if self.end != document.source().len() {
            return Err(LayoutError::StaleContinuation);
        }
        let pending = document.indented_code_pending().map(|p| p.source);
        if pending != self.pending_code {
            for source in [self.pending_code, pending].into_iter().flatten() {
                let lo = self.scalar(source.start)?;
                let hi = self.scalar(source.end)?;
                self.touch(self.root, lo, hi);
            }
            self.pending_code = pending;
        }
        self.revision = update.revision;
        // Finish alone only seals the input. Every actual EOF semantic
        // change has its own typed effect; retain an unchanged open EGC cut.
        if binding_changed {
            self.epoch += 1;
            self.semantic_clock += 1;
        }
        self.generation += 1;
        Ok(())
    }
    fn touch(&mut self, id: usize, lo: usize, hi: usize) {
        if hi <= self.nodes[id].lo || self.nodes[id].hi <= lo {
            return;
        }
        if lo <= self.nodes[id].lo && self.nodes[id].hi <= hi {
            self.set_changed(id);
            self.bump(id);
            return;
        }
        self.split(id);
        for child in self.nodes[id].children.unwrap() {
            self.touch(child, lo, hi);
        }
        self.refresh(id);
    }
    fn fact(&mut self, source: Span, fact: SemanticFact) -> Result<(), LayoutError> {
        let lo = self.scalar(source.start)?;
        let hi = self.scalar(source.end)?;
        self.fact_in(self.root, lo, hi, fact);
        Ok(())
    }
    fn fact_in(&mut self, id: usize, lo: usize, hi: usize, fact: SemanticFact) {
        self.work.input_node_visits += 1;
        if hi <= self.nodes[id].lo || self.nodes[id].hi <= lo {
            return;
        }
        if lo <= self.nodes[id].lo && self.nodes[id].hi <= hi {
            match fact {
                SemanticFact::Literal(value) => {
                    self.nodes[id].literal = value;
                    self.nodes[id].literal_lazy = Some(value);
                }
                SemanticFact::Image(value) => self.nodes[id].image = value,
                SemanticFact::Footnote(value) => {
                    self.nodes[id].footnote = value;
                    self.nodes[id].footnote_lazy = Some(value);
                }
                SemanticFact::Candidate => self.nodes[id].candidate = true,
                SemanticFact::Html(at) => self.nodes[id].html = Some(at),
            };
            self.set_changed(id);
            self.bump(id);
            return;
        }
        self.split(id);
        self.push_fact(id, fact);
        for child in self.nodes[id].children.unwrap() {
            self.fact_in(child, lo, hi, fact);
        }
        self.refresh(id);
    }
    fn put_object(
        &mut self,
        id: mdtui3::LinkId,
        info: mdtui3::LinkInfo,
        active: bool,
    ) -> Result<(), LayoutError> {
        fn ranges(info: mdtui3::LinkInfo, active: bool) -> [Span; 2] {
            if !active {
                return [Span::new(0, 0); 2];
            }
            [
                Span::new(
                    info.source.start,
                    info.label.start.clamp(info.source.start, info.source.end),
                ),
                Span::new(
                    info.label.end.clamp(info.source.start, info.source.end),
                    info.source.end,
                ),
            ]
        }
        fn difference(a: Span, b: [Span; 2]) -> Vec<Span> {
            let mut parts = vec![a];
            for cut in b {
                let mut next = Vec::new();
                for part in parts {
                    if cut.end <= part.start || part.end <= cut.start {
                        next.push(part)
                    } else {
                        if part.start < cut.start {
                            next.push(Span::new(part.start, cut.start));
                        }
                        if cut.end < part.end {
                            next.push(Span::new(cut.end, part.end));
                        }
                    }
                }
                parts = next;
            }
            parts
        }
        let old = self.objects.insert(id, (info, active));
        let old_ranges = old.map_or([Span::new(0, 0); 2], |(info, active)| ranges(info, active));
        let new_ranges = ranges(info, active);
        for old in old_ranges {
            for source in difference(old, new_ranges) {
                if !source.is_empty() {
                    self.fact(source, SemanticFact::Literal(None))?;
                }
            }
        }
        for new in new_ranges {
            for source in difference(new, old_ranges) {
                if !source.is_empty() {
                    self.fact(source, SemanticFact::Literal(Some(id)))?;
                }
            }
        }
        let image = |this: &Self, info: mdtui3::LinkInfo, active: bool| -> Option<Span> {
            if !active || info.kind != mdtui3::LinkKind::Image || info.source.is_empty() {
                return None;
            }
            let at = this.scalar(info.source.start).ok()?;
            Some(Span::new(info.source.start, this.position(at + 1)))
        };
        let old_image = old.and_then(|(info, active)| image(self, info, active));
        let new_image = image(self, info, active);
        if old_image != new_image {
            if let Some(source) = old_image {
                self.fact(source, SemanticFact::Image(None))?;
            }
            if let Some(source) = new_image {
                self.fact(source, SemanticFact::Image(Some(id)))?;
            }
        }
        Ok(())
    }
    fn html_qualified(&self, source: Span) -> bool {
        let at = self.lower_bound(source.start);
        let mut id = self.root;
        let mut mark = blank();
        let mut replaced = false;
        loop {
            let n = &self.nodes[id];
            mark = merge(mark, n.mark);
            replaced |= n.has_replacement();
            let Some([a, b]) = n.children else { break };
            id = if at < self.nodes[b].lo { a } else { b };
        }
        !replaced
            && !mark.style.contains(Style::CODE)
            && !mark.base_hidden
            && mark.hide_owner.is_none()
    }
    fn display_atom(&self, at: usize, id: usize, context: Context) -> Entity {
        let n = &self.nodes[id];
        let atom = self.atom(at);
        if let Some((number, source)) = context.foot_number {
            return Entity {
                number: Some(number),
                chars: ['\0'; 8],
                properties: [self.space; 8],
                scalars: if atom.source.start == source.start {
                    number.ilog10() as u8 + 1
                } else {
                    0
                },
                source,
            };
        }
        if self
            .pending_code
            .is_some_and(|p| p.start <= atom.source.start && atom.source.start < p.end)
        {
            return Entity {
                number: None,
                chars: ['\0'; 8],
                properties: [self.space; 8],
                scalars: 0,
                source: atom.source,
            };
        }
        if let Some(decision) = n
            .decision
            .filter(|d| d.epoch == self.epoch && d.semantic == n.semantic)
            && let Some(entity) = decision.replacement
        {
            return entity;
        }
        if let Some(entity) = n.layer.entity {
            return entity;
        }
        let mut result = Entity {
            number: None,
            chars: ['\0'; 8],
            properties: [self.space; 8],
            scalars: 0,
            source: atom.source,
        };
        if n.spaces != 0 {
            result.scalars = n.spaces.saturating_sub(u8::from(context.trim));
            result.chars.fill(' ');
        } else if !context.trim
            && (!atom.ch.is_control() || matches!(atom.ch, '\t' | '\r' | '\n'))
            && (context.literal || !n.mark.style.contains(Style::HARD_BREAK))
        {
            let normalized = atom.ch == '\t'
                || !context.literal && n.mark.style.contains(Style::SOFT_BREAK)
                || context.code && matches!(atom.ch, '\r' | '\n');
            result.scalars = 1;
            result.chars[0] = if normalized { ' ' } else { atom.ch };
            if normalized && atom.ch == '\n' && at > 0 && self.atom(at - 1).ch == '\r' {
                result.source.start = self.atom(at - 1).source.start;
            }
            result.properties[0] = if normalized { self.space } else { atom.raw };
        }
        result
    }
    // An unchanged literal range has no binding dependencies. All conditional
    // semantics remain revisioned by the core binding epoch; local marks and
    // replacements are additionally checked by semantic_version.
    fn payload_epoch(&self, source: Span) -> usize {
        // Literal scalars can still inherit conditional styles from markers
        // outside this EGC. One current scope lookup proves that dependency.
        if self
            .conditional_scopes
            .range(..=source.start)
            .next_back()
            .is_some_and(|(_, scope)| source.start < scope.end)
        {
            return self.epoch;
        }
        fn depends(index: &ShapeIndex, id: usize, lo: usize, hi: usize) -> bool {
            let n = &index.nodes[id];
            if hi <= n.lo || n.hi <= lo {
                return false;
            }
            if n.changed || n.mark.link.is_some() {
                return true;
            }
            if lo <= n.lo && n.hi <= hi {
                return n.descendant_changes;
            }
            n.children.is_some_and(|children| {
                children
                    .into_iter()
                    .any(|child| depends(index, child, lo, hi))
            })
        }
        let lo = self.lower_bound(source.start);
        let hi = self.lower_bound(source.end);
        if depends(self, self.root, lo, hi) {
            self.epoch
        } else {
            0
        }
    }
    fn semantic_version(&self, source: Span) -> u64 {
        let lo = self.lower_bound(source.start);
        let hi = self.lower_bound(source.end);
        fn range(index: &ShapeIndex, id: usize, lo: usize, hi: usize) -> u64 {
            let n = &index.nodes[id];
            if hi <= n.lo || n.hi <= lo {
                return 0;
            }
            if lo <= n.lo && n.hi <= hi {
                return n.semantic_max;
            }
            let mut value = n.semantic;
            if let Some(children) = n.children {
                for child in children {
                    value = value.max(range(index, child, lo, hi));
                }
            }
            value
        }
        range(self, self.root, lo, hi)
    }
    fn descendant_special(&self, id: usize, context: Context) -> bool {
        self.nodes[id].children.is_some_and(|children| {
            children
                .into_iter()
                .any(|child| self.nodes[child].subtree_special(context))
        })
    }
    fn store_style(&mut self, snapshot: ResolvedStyleSpan) -> Result<(), LayoutError> {
        let lo = self.scalar(snapshot.source.start)?;
        let hi = self.scalar(snapshot.source.end)?;
        self.style_in(
            self.root,
            lo,
            hi,
            StyleTag {
                epoch: self.epoch,
                snapshot,
            },
        );
        Ok(())
    }
    fn style_in(&mut self, id: usize, lo: usize, hi: usize, tag: StyleTag) {
        self.work.overlay_node_visits += 1;
        let n = &self.nodes[id];
        if hi <= n.lo || n.hi <= lo {
            return;
        }
        if lo <= n.lo && n.hi <= hi {
            self.nodes[id].resolved_style = Some(tag);
            return;
        }
        if n.children.is_none() && n.hi - n.lo > LEAF {
            self.split(id);
        }
        if let Some(children) = self.nodes[id].children {
            for child in children {
                self.style_in(child, lo, hi, tag);
            }
        } else {
            let n = &self.nodes[id];
            let start = lo.max(n.lo) - n.lo;
            let end = hi.min(n.hi) - n.lo;
            let cells = self.nodes[id]
                .style_cells
                .get_or_insert_with(|| Box::new([None; LEAF]));
            self.work.overlay_scalar_writes += end - start;
            for cell in &mut cells[start..end] {
                *cell = Some(tag);
            }
        }
    }
    fn remember_hidden(&mut self, id: usize, hidden: bool) {
        let context = Context {
            hidden,
            ..Context::default()
        };
        self.remember(id, context, None);
    }
    fn remember(&mut self, id: usize, mut context: Context, replacement: Option<Entity>) {
        context.style_override = None;
        context.style_hidden = false;
        context.hint = false;
        context.foot_hidden = false;
        let hidden = (context.hidden || context.skip)
            && replacement.is_none()
            && !self.descendant_special(id, context);
        let n = &mut self.nodes[id];
        n.decision = Some(Decision {
            epoch: self.epoch,
            semantic: n.semantic,
            hidden,
            context,
            replacement,
        });
    }
    fn capacity_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.atoms.capacity() * std::mem::size_of::<RawAtom>()
            + self.positions.capacity() * std::mem::size_of::<usize>()
            + self.nodes.capacity() * std::mem::size_of::<Node>()
            + self.nodes.iter().filter(|n| n.cache.is_some()).count() * std::mem::size_of::<Cache>()
            + self
                .nodes
                .iter()
                .filter(|n| n.style_cells.is_some())
                .count()
                * std::mem::size_of::<[Option<StyleTag>; LEAF]>()
            + self
                .undo
                .as_ref()
                .map_or(0, |u| u.capacity() * std::mem::size_of::<usize>())
            + self.replacements.len() * (std::mem::size_of::<(usize, ReplacementUndo)>() + 256)
            + self.objects.len()
                * (std::mem::size_of::<(mdtui3::LinkId, (mdtui3::LinkInfo, bool))>() + 256)
            + self.presentations.len() * (std::mem::size_of::<(usize, ScalarPresentation)>() + 256)
            + self.conditional_scopes.len() * (std::mem::size_of::<(usize, Span)>() + 256)
            + self.html.len() * (std::mem::size_of::<(usize, Span)>() + 256)
            + self.footnotes.len()
                * (std::mem::size_of::<(mdtui3::FootnoteReferenceId, Span)>() + 256)
            + self
                .replacements
                .values()
                .map(|u| u.nodes.capacity() * std::mem::size_of::<(usize, Layer)>())
                .sum::<usize>()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum QueryKind {
    Gcb,
    Width,
}
#[derive(Clone, Copy, Default, Debug)]
struct Answer {
    out: u8,
    value: i64,
    bytes: usize,
    scalars: usize,
    first: Option<Span>,
    last: Option<Span>,
}
impl Answer {
    fn empty(input: u8) -> Self {
        Self {
            out: input,
            ..Self::default()
        }
    }
    fn combine(a: Self, b: Self, kind: QueryKind) -> Self {
        let mut result = Self::join(a, b);
        if kind == QueryKind::Width {
            result.first = b.first.or(a.first);
            result.last = a.last.or(b.last);
        }
        result
    }
    fn join(a: Self, b: Self) -> Self {
        Self {
            out: b.out,
            value: a.value + b.value,
            bytes: a.bytes + b.bytes,
            scalars: a.scalars + b.scalars,
            first: a.first.or(b.first),
            last: b.last.or(a.last),
        }
    }
}
#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct Context {
    foot_number: Option<(usize, Span)>,
    code: bool,
    trim: bool,
    skip: bool,
    hidden: bool,
    literal: bool,
    literal_bound: bool,
    foot_bound: bool,
    style_override: Option<ResolvedStyleSpan>,
    style_hidden: bool,
    hint: bool,
    raw_alt: bool,
    foot_hidden: bool,
}
#[derive(Clone, Copy)]
enum Stage {
    Enter,
    Special,
    Style,
    ApplyStyles,
    Gate(usize),
    Footnote,
    Contents,
    Left,
    Right,
    Leaf,
    Finish,
}
struct QueryFrame {
    id: usize,
    input: u8,
    context: Context,
    incoming: Context,
    stage: Stage,
    left: Option<Answer>,
    right: Option<Answer>,
    style_cursor: Option<StyleMarkers>,
    style: Option<ResolvedStyleSpan>,
    activity: Option<LinkActivity>,
    offset: usize,
    leaf_result: Answer,
    cacheable: bool,
    objects: Option<super::objects::ObjectWindow>,
    image_query: Option<Box<ShapeQuery>>,
    special_done: bool,
    replacement: Option<Entity>,
    prepared: bool,
    literal_guard: Option<mdtui3::LinkId>,
    foot_guard: Option<mdtui3::FootnoteReferenceId>,
    foot_done: bool,
    style_query: Option<Box<ShapeQuery>>,
    style_answer: Answer,
    style_mixed: bool,
}
impl QueryFrame {
    fn new(id: usize, input: u8, context: Context) -> Self {
        Self {
            id,
            input,
            context,
            incoming: context,
            stage: Stage::Enter,
            left: None,
            right: None,
            style_cursor: None,
            style: None,
            activity: None,
            offset: 0,
            leaf_result: Answer::empty(input),
            cacheable: false,
            objects: None,
            image_query: None,
            special_done: false,
            replacement: None,
            prepared: false,
            literal_guard: None,
            foot_guard: None,
            foot_done: false,
            style_query: None,
            style_answer: Answer::empty(input),
            style_mixed: false,
        }
    }
}
/// Resumable range fold. Construction stores offsets only. Both machines read
/// already classified properties. One paid step reads one cache/node/scalar.
pub struct ShapeQuery {
    owner: SharedShape,
    source: Span,
    bounds: (usize, usize),
    kind: QueryKind,
    input: u8,
    stack: Vec<QueryFrame>,
    result: Option<Answer>,
    revision: u64,
    initialized: bool,
    node: Option<usize>,
    context: Context,
    cut: Option<(DisplayPosition, DisplayPosition)>,
}
impl ShapeQuery {
    pub fn graphemes(shape: &SharedShape, source: Span) -> Self {
        Self::new(shape, source, QueryKind::Gcb, 0)
    }
    fn new(shape: &SharedShape, source: Span, kind: QueryKind, input: u8) -> Self {
        Self {
            owner: shape.clone(),
            source,
            bounds: (0, 0),
            kind,
            input,
            stack: Vec::new(),
            result: None,
            revision: shape.0.borrow().revision,
            initialized: false,
            node: None,
            context: Context::default(),
            cut: None,
        }
    }
    /// Explicit diagnostic traversal of this task's owned frame buffers. This
    /// is not a frame scheduling operation; the shared index/resolver are
    /// reported separately. Style traversal retains one fixed native snapshot.
    pub fn scratch_capacity_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.stack.capacity() * std::mem::size_of::<QueryFrame>()
            + self
                .stack
                .iter()
                .map(|f| {
                    f.style_query
                        .as_ref()
                        .map_or(0, |q| q.scratch_capacity_bytes())
                        + f.image_query
                            .as_ref()
                            .map_or(0, |q| q.scratch_capacity_bytes())
                        + f.objects.as_ref().map_or(0, |o| o.scratch_capacity_bytes())
                })
                .sum::<usize>()
    }
    fn deliver(&mut self, index: &mut ShapeIndex, answer: Answer) {
        let f = self.stack.pop().unwrap();
        if f.prepared {
            index.remember(f.id, f.context, f.replacement);
        }
        if f.cacheable {
            let n = &mut index.nodes[f.id];
            let cache = n
                .cache
                .get_or_insert_with(|| Box::new(Cache::new(n.generation, index.epoch, f.incoming)));
            if cache.generation != n.generation
                || cache.epoch != index.epoch
                || cache.mode != f.incoming
            {
                **cache = Cache::new(n.generation, index.epoch, f.incoming);
            }
            match self.kind {
                QueryKind::Gcb => cache.gcb[usize::from(f.input)] = Some(answer),
                QueryKind::Width => cache.width[usize::from(f.input)] = Some(answer),
            }
        }
        if let Some(parent) = self.stack.last_mut() {
            if matches!(parent.stage, Stage::Left) {
                parent.left = Some(answer)
            } else {
                parent.right = Some(answer)
            }
        } else {
            self.result = Some(answer)
        }
    }
    fn advance_answer(
        &mut self,
        shape: &SharedShape,
        document: &Document,
        links: &SharedLinks,
        budget: &mut Budget,
    ) -> Result<Option<Answer>, LayoutError> {
        shape.check_context(&self.owner, document, links)?;
        if self.revision != document.revision() {
            if !links.can_rebind(document, self.revision) {
                return Err(LayoutError::StaleContinuation);
            }
            self.revision = document.revision();
        }
        if self.revision != shape.applied_revision() {
            return Err(LayoutError::StaleContinuation);
        }
        if let Some(result) = self.result {
            return Ok(Some(result));
        }
        if !links.prepare(document, budget)? {
            return Ok(None);
        }
        if !self.initialized {
            if !paid(budget) {
                return Ok(None);
            }
            let index = shape.0.borrow();
            self.bounds = (
                index.scalar(self.source.start)?,
                index.scalar(self.source.end)?,
            );
            self.stack.push(QueryFrame::new(
                self.node.unwrap_or(index.root),
                self.input,
                self.context,
            ));
            self.initialized = true;
        }
        loop {
            let Some(f) = self.stack.last_mut() else {
                return Ok(self.result);
            };
            let id = f.id;
            match f.stage {
                Stage::Enter => {
                    if !paid(budget) {
                        return Ok(None);
                    }
                    let mut index = shape.0.borrow_mut();
                    index.work.query_nodes += 1;
                    let n = &index.nodes[id];
                    let lo = index.position(n.lo.min(index.atom_count()));
                    let hi = index.position(n.hi.min(index.atom_count()));
                    if hi <= self.source.start
                        || lo >= self.source.end
                        || n.lo >= index.atom_count()
                    {
                        let a = Answer::empty(f.input);
                        self.deliver(&mut index, a);
                        continue;
                    }
                    if let Some(at) = n.html
                        && index.html_qualified(Span::new(at, at))
                    {
                        index.remember_hidden(id, true);
                        let a = Answer::empty(f.input);
                        self.deliver(&mut index, a);
                        continue;
                    }
                    let pending_overlap = index
                        .pending_code
                        .is_some_and(|p| p.start < hi && lo < p.end)
                        || n.descendant_html;
                    if index
                        .pending_code
                        .is_some_and(|p| p.start <= lo && hi <= p.end)
                    {
                        let a = Answer::empty(f.input);
                        self.deliver(&mut index, a);
                        continue;
                    }
                    f.cacheable = self.source.start <= lo
                        && hi <= self.source.end
                        && !f.context.trim
                        && !f.context.skip;
                    if let Some((start, end)) = self.cut
                        && (start.part != 0 && n.lo <= start.atom && start.atom < n.hi
                            || end.part != 0 && n.lo <= end.atom && end.atom < n.hi)
                    {
                        f.cacheable = false;
                    }
                    if f.cacheable
                        && let Some(cache) = &n.cache
                        && cache.generation == n.generation
                        && cache.epoch == index.epoch
                        && cache.mode == f.incoming
                    {
                        let answer = match self.kind {
                            QueryKind::Gcb => cache.gcb[usize::from(f.input)],
                            QueryKind::Width => cache.width[usize::from(f.input)],
                        };
                        if let Some(answer) = answer {
                            index.work.summary_hits += 1;
                            self.deliver(&mut index, answer);
                            continue;
                        }
                    }
                    f.prepared = true;
                    if !f.special_done {
                        f.literal_guard = if f.incoming.literal_bound {
                            None
                        } else {
                            n.literal
                        };
                        f.foot_guard = if f.incoming.foot_bound {
                            None
                        } else {
                            n.footnote
                        };
                        if !f.incoming.literal_bound && n.literal_lazy.is_some() {
                            f.context.literal_bound = true;
                        }
                        if !f.incoming.foot_bound && n.footnote_lazy.is_some() {
                            f.context.foot_bound = true;
                        }
                    }
                    f.context.code |= n.layer.code;
                    f.context.trim |= n.layer.trim;
                    f.context.skip |= n.layer.skip;
                    f.context.hidden |= n.mark.base_hidden && !f.context.literal;
                    if (f.literal_guard.is_some() || n.image.is_some() && !f.context.literal)
                        && !f.special_done
                    {
                        f.stage = Stage::Special;
                        continue;
                    }
                    if f.context.skip && n.layer.entity.is_none()
                        || (f.context.hidden || f.context.style_hidden)
                            && !index.descendant_special(id, f.context)
                            && f.replacement.is_none()
                    {
                        let answer = Answer::empty(f.input);
                        self.deliver(&mut index, answer);
                        continue;
                    }
                    if f.replacement.is_some() {
                        f.context.hidden = false;
                        f.stage = Stage::Contents;
                        continue;
                    }
                    if f.foot_guard.is_some() && !f.foot_done {
                        f.stage = Stage::Footnote;
                        continue;
                    }
                    // A numbered marker is an inherited display replacement,
                    // including on continuation subtrees with no own guard.
                    // Their raw summaries still contain the original label.
                    if f.context.foot_number.is_some() {
                        f.stage = Stage::Contents;
                        continue;
                    }
                    if !pending_overlap
                        && !n.base_changed
                        && !n.descendant_base
                        && !n.subtree_special(f.context)
                        && f.cacheable
                    {
                        let answer = if f.context.code {
                            n.code.answer(self.kind, f.input)
                        } else {
                            n.raw.answer(self.kind, f.input)
                        };
                        index.work.summary_hits += 1;
                        index.remember_hidden(id, false);
                        self.deliver(&mut index, answer);
                        continue;
                    }
                    if !pending_overlap
                        && !n.descendant_changes
                        && n.layer
                            == (Layer {
                                code: true,
                                ..Layer::default()
                            })
                        && !n.mark.base_hidden
                        && !n.mark.delimiter_hidden
                        && n.mark.hide_owner.is_none()
                        && n.mark.hide_alternative.is_none()
                        && f.cacheable
                    {
                        let answer = n.code.answer(self.kind, f.input);
                        index.remember_hidden(id, false);
                        index.work.summary_hits += 1;
                        self.deliver(&mut index, answer);
                        continue;
                    }
                    if !pending_overlap
                        && f.context.literal
                        && !n.has_replacement()
                        && !n.replacement_descendants
                        && f.cacheable
                    {
                        let answer = if f.context.code {
                            n.code.answer(self.kind, f.input)
                        } else {
                            n.raw.answer(self.kind, f.input)
                        };
                        self.deliver(&mut index, answer);
                        continue;
                    }
                    if let Some(snapshot) = f.context.style_override {
                        if !pending_overlap
                            && !n.permanent
                            && !n.descendant_permanent
                            && !n.subtree_special(f.context)
                            && f.cacheable
                            && (snapshot.authoritative || !n.hints)
                        {
                            if snapshot.authoritative
                                && snapshot.marker_hidden
                                && !f.context.literal
                            {
                                f.context.style_hidden = true;
                                let a = Answer::empty(f.input);
                                self.deliver(&mut index, a);
                                continue;
                            }
                            let a = if f.context.code {
                                n.code.answer(self.kind, f.input)
                            } else {
                                n.raw.answer(self.kind, f.input)
                            };
                            self.deliver(&mut index, a);
                            continue;
                        }
                        f.style = Some(snapshot);
                        f.stage = Stage::Gate(0);
                        continue;
                    }
                    f.stage = if (n.candidate || n.mark.delimiter_hidden) && !f.context.literal {
                        Stage::Style
                    } else {
                        Stage::Gate(0)
                    };
                }
                Stage::Special => {
                    let index = shape.0.borrow();
                    let n = &index.nodes[id];
                    let source = Span::new(
                        index.position(n.lo),
                        index.position(n.hi.min(index.atom_count())),
                    );
                    let literal = f.literal_guard;
                    let image = if f.context.literal { None } else { n.image };
                    drop(index);
                    if literal.is_some() || image.is_some() {
                        if f.objects.is_none() {
                            if !paid(budget) {
                                return Ok(None);
                            }
                            f.objects =
                                super::objects::ObjectWindow::new(document, source, links.clone());
                        }
                        if !f.objects.as_mut().unwrap().advance(document, budget)? {
                            return Ok(None);
                        }
                        if let Some(id) = literal
                            && f.objects.as_ref().unwrap().presentation(id).disposition
                                == Some(LinkDisposition::Unresolved)
                        {
                            f.context.literal = true;
                            f.context.hidden = false;
                        }
                        if let Some(id) = image
                            && f.objects.as_ref().unwrap().presentation(id).image == Some(id)
                        {
                            let info = *document.link_info(id).ok_or(LayoutError::InvalidRange)?;
                            let empty = if info.label.is_empty() {
                                true
                            } else {
                                let query = f.image_query.get_or_insert_with(|| {
                                    let mut q = ShapeQuery::graphemes(shape, info.label);
                                    q.context.raw_alt = true;
                                    Box::new(q)
                                });
                                let Some(answer) =
                                    query.advance_answer(shape, document, links, budget)?
                                else {
                                    return Ok(None);
                                };
                                answer.bytes == 0
                            };
                            if empty {
                                let mut replacement = shape.0.borrow().fallback;
                                replacement.source =
                                    Span::new(info.source.start, info.source.start + 1);
                                f.replacement = Some(replacement);
                                f.context.hidden = false;
                            }
                        }
                    }
                    f.special_done = true;
                    f.stage = Stage::Enter;
                }
                Stage::Footnote => {
                    if let Some(id) = f.foot_guard
                        && !f.context.raw_alt
                    {
                        if links.footnote_numbers() {
                            let Ok(number) = document.footnote_display_number(id, budget) else {
                                return Ok(None);
                            };
                            let mut index = shape.0.borrow_mut();
                            let semantic = index.nodes[f.id].semantic;
                            index.nodes[f.id].foot_state = Some((index.epoch, semantic, false));
                            f.context.foot_number =
                                number.map(|n| (n, document.footnote_references()[id.0].source));
                            f.context.foot_hidden = false;
                            f.foot_done = true;
                            f.stage = Stage::Contents;
                            continue;
                        }
                        let Ok(Some(resolution)) = document.resolve_footnote(id, budget) else {
                            return Ok(None);
                        };
                        let defined =
                            matches!(resolution.target, mdtui3::FootnoteTarget::Defined(_));
                        let mut index = shape.0.borrow_mut();
                        let semantic = index.nodes[f.id].semantic;
                        index.nodes[f.id].foot_state = Some((index.epoch, semantic, defined));
                        f.context.foot_hidden = defined;
                    }
                    f.foot_done = true;
                    if f.context.foot_hidden {
                        let mut index = shape.0.borrow_mut();
                        let a = Answer::empty(f.input);
                        self.deliver(&mut index, a);
                        continue;
                    }
                    if f.context.hidden && !shape.0.borrow().descendant_special(id, f.context) {
                        let mut index = shape.0.borrow_mut();
                        let a = Answer::empty(f.input);
                        self.deliver(&mut index, a);
                        continue;
                    }
                    f.stage = Stage::Contents;
                }
                Stage::Style => {
                    if f.style_cursor.is_none() {
                        if !paid(budget) {
                            return Ok(None);
                        }
                        let index = shape.0.borrow();
                        let n = &index.nodes[id];
                        let source = Span::new(
                            index.position(n.lo).max(self.source.start),
                            index
                                .position(n.hi.min(index.atom_count()))
                                .min(self.source.end),
                        );
                        f.style_cursor = if self.kind == QueryKind::Gcb {
                            document.style_markers(source)
                        } else {
                            document.style_markers_reverse(source)
                        };
                    }
                    let step = links.with_resolver(|r| {
                        f.style_cursor.as_mut().unwrap().rebind_with(document, r)?;
                        f.style_cursor
                            .as_mut()
                            .unwrap()
                            .advance(document, r, budget)
                    })?;
                    match step {
                        StyleMarkerStep::BudgetExhausted => return Ok(None),
                        StyleMarkerStep::Span(style) => {
                            let mut index = shape.0.borrow_mut();
                            index.store_style(style)?;
                            index.work.style_spans_seen += 1;
                            index.work.style_buffer_peak_entries =
                                index.work.style_buffer_peak_entries.max(1);
                            index.work.style_buffer_peak_bytes =
                                std::mem::size_of::<ResolvedStyleSpan>();
                            let n = &index.nodes[id];
                            let begin = index.position(n.lo).max(self.source.start);
                            let end = index
                                .position(n.hi.min(index.atom_count()))
                                .min(self.source.end);
                            f.style = Some(style);
                            if !f.style_mixed && style.source == Span::new(begin, end) {
                                f.stage = Stage::Gate(0);
                            } else {
                                f.style_mixed = true;
                                f.stage = Stage::ApplyStyles;
                            }
                        }
                        StyleMarkerStep::Complete => {
                            if f.style_mixed {
                                let mut index = shape.0.borrow_mut();
                                let answer = f.style_answer;
                                f.prepared = false;
                                self.deliver(&mut index, answer);
                            } else {
                                f.stage = Stage::Gate(0);
                            }
                        }
                    }
                }
                Stage::ApplyStyles => {
                    let snapshot = f.style.unwrap();
                    if f.style_query.is_none() {
                        if !paid(budget) {
                            return Ok(None);
                        }
                        let mut q =
                            ShapeQuery::new(shape, snapshot.source, self.kind, f.style_answer.out);
                        q.node = Some(id);
                        q.context = f.incoming;
                        q.context.style_override = Some(snapshot);
                        f.style_query = Some(Box::new(q));
                    }
                    let Some(answer) = f
                        .style_query
                        .as_mut()
                        .unwrap()
                        .advance_answer(shape, document, links, budget)?
                    else {
                        return Ok(None);
                    };
                    f.style_answer = Answer::combine(f.style_answer, answer, self.kind);
                    f.style_query = None;
                    f.style = None;
                    f.stage = Stage::Style;
                }
                Stage::Gate(at) => {
                    let mark = shape.0.borrow().nodes[id].mark;
                    if at == 0 {
                        let applied = f.style.map_or(mark, |s| s.apply(mark));
                        let marker_hidden =
                            f.style.is_some_and(|s| s.authoritative && s.marker_hidden)
                                || mark.delimiter_hidden && applied.style.contains(Style::HIDDEN);
                        if !f.context.literal
                            && (marker_hidden
                                || applied.style.contains(Style::HIDDEN)
                                    && applied.hide_owner.is_none()
                                    && applied.hide_alternative.is_none())
                        {
                            let mut index = shape.0.borrow_mut();
                            f.context.style_hidden = true;
                            if !index.descendant_special(id, f.context) {
                                let a = Answer::empty(f.input);
                                self.deliver(&mut index, a);
                                continue;
                            }
                        }
                    }
                    let gate = [mark.hide_owner, mark.hide_alternative]
                        .get(at)
                        .copied()
                        .flatten();
                    if at >= 2 || f.context.literal {
                        f.stage = Stage::Footnote;
                        continue;
                    }
                    if let Some(gate) = gate {
                        if f.activity.is_none() {
                            if !paid(budget) {
                                return Ok(None);
                            }
                            f.activity = document.link_activity(gate);
                            shape.0.borrow_mut().work.recovered_guards += 1;
                        }
                        if let Some(activity) = f.activity.as_mut() {
                            let result = links.with_resolver(|r| {
                                activity.rebind_with(document, r)?;
                                activity.advance_with(document, r, budget)
                            })?;
                            let Some(disposition) = result else {
                                return Ok(None);
                            };
                            f.activity = None;
                            if disposition == LinkDisposition::Active {
                                let mut index = shape.0.borrow_mut();
                                f.context.hidden = true;
                                if !index.descendant_special(id, f.context) {
                                    let a = Answer::empty(f.input);
                                    self.deliver(&mut index, a);
                                    continue;
                                }
                            }
                        }
                    }
                    f.stage = Stage::Gate(at + 1);
                }
                Stage::Contents => {
                    {
                        let mut index = shape.0.borrow_mut();
                        if index.nodes[id].children.is_none()
                            && index.nodes[id].hi - index.nodes[id].lo > LEAF
                        {
                            if !paid(budget) {
                                return Ok(None);
                            }
                            index.split(id);
                        }
                    }
                    shape.0.borrow_mut().remember(id, f.context, f.replacement);
                    let index = shape.0.borrow();
                    let n = &index.nodes[id];
                    if let Some([a, b]) = n.children {
                        f.stage = Stage::Left;
                        let child = if self.kind == QueryKind::Gcb { a } else { b };
                        let context = f.context;
                        let input = f.input;
                        self.stack.push(QueryFrame::new(child, input, context));
                    } else {
                        f.offset = if self.kind == QueryKind::Gcb {
                            n.lo.max(self.bounds.0)
                        } else {
                            n.hi.min(self.bounds.1)
                        };
                        f.stage = Stage::Leaf;
                    }
                }
                Stage::Left => {
                    let index = shape.0.borrow();
                    let [a, b] = index.nodes[id].children.unwrap();
                    let child = if self.kind == QueryKind::Gcb { b } else { a };
                    let input = f.left.unwrap().out;
                    let context = f.context;
                    f.stage = Stage::Right;
                    self.stack.push(QueryFrame::new(child, input, context));
                }
                Stage::Right => f.stage = Stage::Finish,
                Stage::Leaf => {
                    let mut index = shape.0.borrow_mut();
                    let n = &index.nodes[id];
                    let done = if self.kind == QueryKind::Gcb {
                        f.offset >= n.hi.min(self.bounds.1)
                    } else {
                        f.offset <= n.lo.max(self.bounds.0)
                    };
                    if done {
                        f.left = Some(f.leaf_result);
                        f.stage = Stage::Finish;
                        continue;
                    }
                    if !paid(budget) {
                        return Ok(None);
                    }
                    let at = if self.kind == QueryKind::Gcb {
                        let a = f.offset;
                        f.offset += 1;
                        a
                    } else {
                        f.offset -= 1;
                        f.offset
                    };
                    let atom = index.atom(at);
                    if atom.source.end <= self.source.start || atom.source.start >= self.source.end
                    {
                        continue;
                    }
                    let display = index.display_atom(at, id, f.context);
                    let parts = usize::from(display.scalars);
                    for i in 0..parts {
                        let part = if self.kind == QueryKind::Gcb {
                            i
                        } else {
                            parts - 1 - i
                        };
                        if self.cut.is_some_and(|(start, end)| {
                            !(at > start.atom
                                || at == start.atom && part >= usize::from(start.part))
                                || !(at < end.atom
                                    || at == end.atom && part < usize::from(end.part))
                        }) {
                            continue;
                        }
                        let ch = display.character(part);
                        let properties = display.property(part);
                        let source = display.source;
                        let (out, value) = match self.kind {
                            QueryKind::Gcb => {
                                let mut s = GcbState(f.leaf_result.out);
                                let b = s.push(properties);
                                (s.0, i64::from(b))
                            }
                            QueryKind::Width => WidthSummary::entry(properties, f.leaf_result.out),
                        };
                        f.leaf_result = Answer::combine(
                            f.leaf_result,
                            Answer {
                                out,
                                value,
                                bytes: ch.len_utf8(),
                                scalars: 1,
                                first: Some(source),
                                last: Some(source),
                            },
                            self.kind,
                        );
                        index.work.query_properties += 1;
                    }
                }
                Stage::Finish => {
                    if !paid(budget) {
                        return Ok(None);
                    }
                    let a = f.left.unwrap_or_else(|| Answer::empty(f.input));
                    let result = if let Some(b) = f.right {
                        Answer::combine(a, b, self.kind)
                    } else {
                        a
                    };
                    let mut index = shape.0.borrow_mut();
                    self.deliver(&mut index, result);
                }
            }
        }
    }
    pub fn advance(
        &mut self,
        shape: &SharedShape,
        document: &Document,
        links: &SharedLinks,
        budget: &mut Budget,
    ) -> Result<Option<usize>, LayoutError> {
        Ok(self
            .advance_answer(shape, document, links, budget)?
            .map(|a| a.value.max(0) as usize))
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ShapeAtom {
    pub utf8: [u8; 80],
    pub len: u8,
    pub source: Span,
    pub atom: usize,
    chars: [char; 20],
    properties: [Properties; 20],
    scalars: u8,
}
#[derive(Clone, Copy)]
struct ReadFrame {
    id: usize,
    context: Context,
    offset: Option<usize>,
}
/// Reads a previously prepared current semantic view. Its stack contains only
/// logarithmic traversal metadata and one fixed-size replacement, never payload.
// One fixed stack buffer covers every digit of a native usize number. Boxing
// Atom would allocate on the ordinary/giant indexed-text hot path per scalar.
#[allow(
    clippy::large_enum_variant,
    reason = "bounded stack atom preserves allocation-free indexed readers"
)]
pub enum ShapeReadStep {
    Atom(ShapeAtom),
    BudgetExhausted,
    Complete,
}
pub struct ShapeReader {
    shape: SharedShape,
    source: Span,
    epoch: usize,
    stack: Vec<ReadFrame>,
    cut: Option<(DisplayPosition, DisplayPosition)>,
    snapshot: Option<(u64, u64)>,
}
impl ShapeReader {
    pub fn new(shape: SharedShape, source: Span) -> Self {
        let index = shape.0.borrow();
        let root = index.root;
        let epoch = index.payload_epoch(source);
        let snapshot = Some((index.semantic_version(source), index.semantic_clock));
        drop(index);
        Self {
            shape,
            source,
            epoch,
            stack: vec![ReadFrame {
                id: root,
                context: Context::default(),
                offset: None,
            }],
            cut: None,
            snapshot,
        }
    }
    pub(crate) fn with_snapshot(mut self, epoch: usize, version: u64, clock: u64) -> Self {
        self.epoch = epoch;
        self.snapshot = Some((version, clock));
        self
    }
    pub fn next(&mut self, budget: &mut Budget) -> Result<ShapeReadStep, LayoutError> {
        {
            let index = self.shape.0.borrow();
            if self.epoch != 0 && index.epoch != self.epoch {
                return Err(LayoutError::StaleContinuation);
            }
            if let Some((version, clock)) = self.snapshot
                && clock != index.semantic_clock
            {
                if !paid(budget) {
                    return Ok(ShapeReadStep::BudgetExhausted);
                }
                if index.payload_epoch(self.source) != self.epoch
                    || index.semantic_version(self.source) != version
                {
                    return Err(LayoutError::StaleContinuation);
                }
                self.snapshot = Some((version, index.semantic_clock));
            }
        }
        loop {
            let Some(f) = self.stack.last_mut() else {
                return Ok(ShapeReadStep::Complete);
            };
            if !paid(budget) {
                return Ok(ShapeReadStep::BudgetExhausted);
            }
            let index = self.shape.0.borrow();
            if self.epoch != 0 && index.epoch != self.epoch {
                return Err(LayoutError::StaleContinuation);
            }
            drop(index);
            let mut index = self.shape.0.borrow_mut();
            index.work.reader_node_visits += 1;
            if f.offset.is_none() && index.nodes[f.id].children.is_none() {
                index.work.reader_leaf_visits += 1;
            }
            let n = &index.nodes[f.id];
            // A saved suffix cut is also a traversal bound. Filtering only
            // after display_atom would re-read every old prefix on each
            // Extend batch even though the wire sent only the suffix.
            let (cut_lo, cut_hi) = self.cut.map_or((0, index.atom_count()), |(start, end)| {
                (start.atom, end.atom + usize::from(end.part != 0))
            });
            if n.hi <= cut_lo || cut_hi <= n.lo {
                self.stack.pop();
                continue;
            }
            if index.pending_code.is_some_and(|p| {
                p.start <= index.position(n.lo.min(index.atom_count()))
                    && index.position(n.hi.min(index.atom_count())) <= p.end
            }) {
                self.stack.pop();
                continue;
            }

            if f.offset.is_none() {
                let lo = index.position(n.lo.min(index.atom_count()));
                let hi = index.position(n.hi.min(index.atom_count()));
                if hi <= self.source.start || lo >= self.source.end || n.lo >= index.atom_count() {
                    self.stack.pop();
                    continue;
                }
                if f.context.hidden && !n.subtree_special(f.context) {
                    self.stack.pop();
                    continue;
                }
                if !f.context.foot_bound && n.footnote_lazy.is_some() {
                    if n.footnote.is_some() && !f.context.raw_alt {
                        let defined = n
                            .foot_state
                            .filter(|&(epoch, semantic, _)| {
                                epoch == index.epoch && semantic == n.semantic
                            })
                            .ok_or(LayoutError::StaleContinuation)?
                            .2;
                        if defined {
                            self.stack.pop();
                            continue;
                        }
                    }
                    f.context.foot_bound = true;
                    if let Some(decision) = n
                        .decision
                        .filter(|d| d.epoch == index.epoch && d.semantic == n.semantic)
                    {
                        f.context.foot_number = decision.context.foot_number;
                    }
                }
                let inherited_style = f.context.style_override;
                let inherited_hint = f.context.hint;
                if n.own_changes(f.context)
                    && (n.permanent
                        || n.html.is_some()
                        || !f.context.literal_bound && n.literal.is_some()
                        || !f.context.literal && n.image.is_some()
                        || !f.context.foot_bound && n.footnote.is_some())
                {
                    let decision = n
                        .decision
                        .filter(|d| d.epoch == index.epoch && d.semantic == n.semantic)
                        .ok_or(LayoutError::StaleContinuation)?;
                    if decision.hidden {
                        self.stack.pop();
                        continue;
                    }
                    f.context = decision.context;
                }
                f.context.style_override = n
                    .resolved_style
                    .filter(|s| s.epoch == index.epoch)
                    .map(|s| s.snapshot)
                    .or(inherited_style);
                f.context.hint = inherited_hint || n.mark.delimiter_hidden;
                if !f.context.literal
                    && f.context.style_override.is_some_and(|s| {
                        if s.authoritative {
                            s.marker_hidden
                        } else {
                            f.context.hint
                        }
                    })
                    && !index.descendant_special(f.id, f.context)
                {
                    self.stack.pop();
                    continue;
                }
                f.context.code |= n.layer.code;
                f.context.trim |= n.layer.trim;
                f.context.skip |= n.layer.skip;
                if let Some([a, b]) = n.children {
                    let context = f.context;
                    self.stack.pop();
                    self.stack.extend([
                        ReadFrame {
                            id: b,
                            context,
                            offset: None,
                        },
                        ReadFrame {
                            id: a,
                            context,
                            offset: None,
                        },
                    ]);
                    continue;
                }
                f.offset = Some(n.lo.max(cut_lo).max(index.lower_bound(self.source.start)));
            }
            let at = f.offset.unwrap();
            if at >= n.hi.min(index.atom_count()).min(cut_hi) {
                self.stack.pop();
                continue;
            }
            f.offset = Some(at + 1);
            let atom = index.atom(at);
            if atom.source.end <= self.source.start || atom.source.start >= self.source.end {
                drop(index);
                self.shape.0.borrow_mut().work.reader_atom_reads += 1;
                continue;
            }
            let snapshot = n
                .style_cells
                .as_ref()
                .and_then(|cells| cells[at - n.lo])
                .filter(|s| s.epoch == index.epoch)
                .map(|s| s.snapshot)
                .or(f.context.style_override);
            if !f.context.literal
                && snapshot.is_some_and(|s| {
                    if s.authoritative {
                        s.marker_hidden
                    } else {
                        f.context.hint
                    }
                })
            {
                continue;
            }
            let display = index.display_atom(at, f.id, f.context);
            let space = index.space;
            drop(index);
            {
                let mut index = self.shape.0.borrow_mut();
                index.work.reader_atom_reads += 1;
                index.work.reader_properties += usize::from(display.scalars);
            }
            let mut result = ShapeAtom {
                utf8: [0; 80],
                len: 0,
                source: display.source,
                atom: at,
                chars: ['\0'; 20],
                properties: [space; 20],
                scalars: 0,
            };
            for part in 0..usize::from(display.scalars) {
                if self.cut.is_some_and(|(start, end)| {
                    !(at > start.atom || at == start.atom && part >= usize::from(start.part))
                        || !(at < end.atom || at == end.atom && part < usize::from(end.part))
                }) {
                    continue;
                }
                let ch = display.character(part);
                let offset = usize::from(result.len);
                result.len += ch.encode_utf8(&mut result.utf8[offset..]).len() as u8;
                result.chars[usize::from(result.scalars)] = ch;
                result.properties[usize::from(result.scalars)] = display.property(part);
                result.scalars += 1;
            }
            if result.len != 0 {
                return Ok(ShapeReadStep::Atom(result));
            }
        }
    }
    pub fn with_cut(mut self, start: DisplayPosition, end: DisplayPosition) -> Self {
        self.cut = Some((start, end));
        self
    }
    pub fn is_complete(&self) -> bool {
        self.stack.is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct DisplayPosition {
    pub atom: usize,
    pub part: u8,
}
/// Finds a display GCB boundary using cached subtree folds, then examines one
/// fixed leaf. It never walks the preceding cluster or RI chain.
pub struct ShapeSelect {
    owner: SharedShape,
    revision: u64,
    source: Span,
    rank: usize,
    state: u8,
    node: usize,
    context: Context,
    query: Option<ShapeQuery>,
    leaf: Option<ShapeReader>,
    pending: Option<ShapeAtom>,
    part: usize,
    initialized: bool,
    result: Option<DisplayPosition>,
}
impl ShapeSelect {
    /// Source mapping of the selected display atom, already read and charged
    /// by advance. The end boundary has no atom and therefore returns None.
    pub fn selected_source(&self) -> Option<Span> {
        self.result.and_then(|_| self.pending.map(|a| a.source))
    }

    pub fn new(shape: &SharedShape, source: Span, rank: usize) -> Self {
        Self {
            owner: shape.clone(),
            revision: shape.applied_revision(),
            source,
            rank,
            state: 0,
            node: shape.0.borrow().root,
            context: Context::default(),
            query: None,
            leaf: None,
            pending: None,
            part: 0,
            initialized: false,
            result: None,
        }
    }
    pub fn advance(
        &mut self,
        shape: &SharedShape,
        document: &Document,
        links: &SharedLinks,
        budget: &mut Budget,
    ) -> Result<Option<DisplayPosition>, LayoutError> {
        shape.check_context(&self.owner, document, links)?;
        if self.revision != document.revision() {
            if !links.can_rebind(document, self.revision) {
                return Err(LayoutError::StaleContinuation);
            }
            self.revision = document.revision();
        }
        if shape.applied_revision() != self.revision {
            return Err(LayoutError::StaleContinuation);
        }
        if self.result.is_some() {
            return Ok(self.result);
        }
        if !self.initialized {
            let query = self
                .query
                .get_or_insert_with(|| ShapeQuery::graphemes(shape, self.source));
            let Some(answer) = query.advance_answer(shape, document, links, budget)? else {
                return Ok(None);
            };
            if self.rank == answer.value as usize {
                let atom = shape.0.borrow().scalar(self.source.end)?;
                self.result = Some(DisplayPosition { atom, part: 0 });
                return Ok(self.result);
            }
            if self.rank > answer.value as usize {
                return Err(LayoutError::InvalidRange);
            }
            self.query = None;
            self.initialized = true;
        }
        loop {
            if let Some(leaf) = self.leaf.as_mut() {
                if self.pending.is_none() {
                    match leaf.next(budget)? {
                        ShapeReadStep::Atom(atom) => {
                            self.pending = Some(atom);
                            self.part = 0
                        }
                        ShapeReadStep::BudgetExhausted => return Ok(None),
                        ShapeReadStep::Complete => return Err(LayoutError::InvalidRange),
                    }
                }
                let atom = self.pending.unwrap();
                if self.part == usize::from(atom.scalars) {
                    self.pending = None;
                    continue;
                }
                if !paid(budget) {
                    return Ok(None);
                }
                let mut state = GcbState(self.state);
                let boundary = state.push(atom.properties[self.part]);
                if boundary {
                    if self.rank == 0 {
                        self.result = Some(DisplayPosition {
                            atom: atom.atom,
                            part: self.part as u8,
                        });
                        return Ok(self.result);
                    }
                    self.rank -= 1;
                }
                self.state = state.0;
                self.part += 1;
                continue;
            }
            {
                let mut index = shape.0.borrow_mut();
                if index.nodes[self.node].children.is_none()
                    && index.nodes[self.node].hi - index.nodes[self.node].lo > LEAF
                {
                    if !paid(budget) {
                        return Ok(None);
                    }
                    index.split(self.node);
                }
            }
            let index = shape.0.borrow();
            let node = &index.nodes[self.node];
            let children = node.children;
            let code = node.layer.code;
            let trim = node.layer.trim;
            if let Some(decision) = node
                .decision
                .filter(|d| d.epoch == index.epoch && d.semantic == node.semantic)
            {
                self.context = decision.context;
            }
            drop(index);
            if let Some([a, b]) = children {
                if self.query.is_none() {
                    if !paid(budget) {
                        return Ok(None);
                    }
                    let mut query = ShapeQuery::new(shape, self.source, QueryKind::Gcb, self.state);
                    let context = Context {
                        code: self.context.code || code,
                        trim: self.context.trim || trim,
                        skip: false,
                        ..self.context
                    };
                    query.node = Some(a);
                    query.context = context;
                    self.query = Some(query);
                }
                let Some(answer) = self
                    .query
                    .as_mut()
                    .unwrap()
                    .advance_answer(shape, document, links, budget)?
                else {
                    return Ok(None);
                };
                self.query = None;
                self.context.code |= code;
                self.context.trim |= trim;
                if self.rank < answer.value as usize {
                    self.node = a
                } else {
                    self.rank -= answer.value as usize;
                    self.state = answer.out;
                    self.node = b;
                }
            } else {
                if !paid(budget) {
                    return Ok(None);
                }
                let mut reader = ShapeReader::new(shape.clone(), self.source);
                reader.stack = vec![ReadFrame {
                    id: self.node,
                    context: self.context,
                    offset: None,
                }];
                self.leaf = Some(reader);
            }
        }
    }
}

/// A complete logical cluster, with first-visible-scalar style ownership and a
/// shared payload descriptor. The source envelope includes internal syntax gaps.
#[derive(Debug, Clone)]
pub struct ShapeGlyph {
    pub text: super::payload::TextPayload,
    pub source: Span,
    pub block: Option<mdtui3::BlockId>,
    pub link: Option<mdtui3::LinkId>,
    pub style: crate::screen::StyleRole,
    pub width: u8,
    pub kind: super::text::GraphemeKind,
    pub start: DisplayPosition,
    pub end: DisplayPosition,
}
/// Publication of one selected cluster. The complete payload is never copied.
pub struct ShapeCluster {
    fragment_barrier: bool,
    outside_fragment: bool,
    owner: SharedShape,
    revision: u64,
    fragment: Span,
    rank: usize,
    start: Option<DisplayPosition>,
    end: Option<DisplayPosition>,
    select: Option<ShapeSelect>,
    query: Option<ShapeQuery>,
    metrics: Option<Answer>,
    raw_width: Option<usize>,
    reader: Option<ShapeReader>,
    first: Option<ShapeAtom>,
    markers: Option<StyleMarkers>,
    snapshot: Option<ResolvedStyleSpan>,
    objects: Option<Box<super::objects::ObjectWindow>>,
    metadata_checked: bool,
    inline: Option<([u8; super::payload::INLINE], usize)>,
    inline_reader: Option<ShapeReader>,
    inline_pending: Option<ShapeAtom>,
    inline_offset: usize,
    presentation: Option<(crate::screen::StyleRole, Option<mdtui3::LinkId>)>,
    result: Option<ShapeGlyph>,
}
impl ShapeCluster {
    pub fn new(shape: &SharedShape, fragment: Span, rank: usize) -> Self {
        Self {
            fragment_barrier: false,
            outside_fragment: false,
            owner: shape.clone(),
            revision: shape.applied_revision(),
            fragment,
            rank,
            start: None,
            end: None,
            select: None,
            query: None,
            metrics: None,
            raw_width: None,
            reader: None,
            first: None,
            markers: None,
            snapshot: None,
            objects: None,
            metadata_checked: false,
            presentation: None,
            inline: None,
            inline_reader: None,
            inline_pending: None,
            inline_offset: 0,
            result: None,
        }
    }
    fn with_fragment_barrier(mut self) -> Self {
        self.fragment_barrier = true;
        self
    }
    pub fn advance(
        &mut self,
        shape: &SharedShape,
        document: &Document,
        links: &SharedLinks,
        budget: &mut Budget,
    ) -> Result<Option<ShapeGlyph>, LayoutError> {
        shape.check_context(&self.owner, document, links)?;
        if self.revision != document.revision() {
            if !links.can_rebind(document, self.revision) {
                return Err(LayoutError::StaleContinuation);
            }
            self.revision = document.revision();
        }
        if shape.applied_revision() != self.revision {
            return Err(LayoutError::StaleContinuation);
        }
        if self.outside_fragment {
            return Ok(None);
        }
        if let Some(result) = &self.result {
            return Ok(Some(result.clone()));
        }
        for edge in 0..2 {
            if if edge == 0 {
                self.start.is_some()
            } else {
                self.end.is_some()
            } {
                continue;
            }
            let select = self
                .select
                .get_or_insert_with(|| ShapeSelect::new(shape, self.fragment, self.rank + edge));
            let Some(position) = select.advance(shape, document, links, budget)? else {
                return Ok(None);
            };
            if edge == 0 {
                self.start = Some(position)
            } else {
                self.end = Some(position)
            }
            self.select = None;
        }
        let (start, end) = (self.start.unwrap(), self.end.unwrap());
        let index = shape.0.borrow();
        let a = index.position(start.atom);
        let b = if end.part == 0 {
            index.position(end.atom)
        } else {
            index.atom(end.atom).source.end
        };
        let source = Span::new(a, b);
        drop(index);
        if self.metrics.is_none() {
            let query = self.query.get_or_insert_with(|| {
                let mut q = ShapeQuery::new(shape, source, QueryKind::Gcb, 0);
                q.cut = Some((start, end));
                q
            });
            let Some(answer) = query.advance_answer(shape, document, links, budget)? else {
                return Ok(None);
            };
            self.metrics = Some(answer);
            self.query = None;
        }
        if self.fragment_barrier
            && self.metrics.as_ref().is_some_and(|metrics| {
                metrics.first.is_some_and(|s| s.start < self.fragment.start)
                    || metrics.last.is_some_and(|s| s.end > self.fragment.end)
            })
        {
            self.outside_fragment = true;
            return Ok(None);
        }
        if self.raw_width.is_none() {
            let query = self.query.get_or_insert_with(|| {
                let mut q = ShapeQuery::new(shape, source, QueryKind::Width, 0);
                q.cut = Some((start, end));
                q
            });
            let Some(answer) = query.advance_answer(shape, document, links, budget)? else {
                return Ok(None);
            };
            self.raw_width = Some(answer.value.max(0) as usize);
            self.query = None;
        }
        if self.first.is_none() {
            let reader = self.reader.get_or_insert_with(|| {
                ShapeReader::new(shape.clone(), source).with_cut(start, end)
            });
            match reader.next(budget)? {
                ShapeReadStep::Atom(atom) => self.first = Some(atom),
                ShapeReadStep::BudgetExhausted => return Ok(None),
                ShapeReadStep::Complete => return Err(LayoutError::InvalidRange),
            }
            self.reader = None;
        }
        let first = self.first.unwrap();
        let metrics = self.metrics.unwrap();
        if metrics.bytes <= super::payload::INLINE && self.inline.is_none() {
            self.inline = Some(([0; super::payload::INLINE], 0));
        }
        if let Some((bytes, at)) = self.inline.as_mut() {
            while *at < metrics.bytes {
                if self.inline_pending.is_none() {
                    if *at == 0 && usize::from(first.len) == metrics.bytes {
                        self.inline_pending = Some(first);
                    } else {
                        let reader = self.inline_reader.get_or_insert_with(|| {
                            ShapeReader::new(shape.clone(), source).with_cut(start, end)
                        });
                        match reader.next(budget)? {
                            ShapeReadStep::Atom(atom) => self.inline_pending = Some(atom),
                            ShapeReadStep::BudgetExhausted => return Ok(None),
                            ShapeReadStep::Complete => return Err(LayoutError::InvalidRange),
                        }
                    }
                    self.inline_offset = 0;
                }
                let atom = self.inline_pending.unwrap();
                let count =
                    (usize::from(atom.len) - self.inline_offset).min(budget.remaining().bytes);
                if count == 0 {
                    return Ok(None);
                }
                assert!(budget.charge(WorkLimit {
                    bytes: count,
                    ..Default::default()
                }));
                bytes[*at..*at + count]
                    .copy_from_slice(&atom.utf8[self.inline_offset..self.inline_offset + count]);
                *at += count;
                self.inline_offset += count;
                if self.inline_offset == usize::from(atom.len) {
                    self.inline_pending = None;
                }
            }
        }
        let first_raw_source = shape.0.borrow().atom(first.atom).source;
        let envelope = Span::new(
            metrics.first.ok_or(LayoutError::InvalidRange)?.start,
            metrics.last.ok_or(LayoutError::InvalidRange)?.end,
        );
        if !self.metadata_checked {
            if !paid(budget) {
                return Ok(None);
            }
            let mut index = shape.0.borrow_mut();
            if let Some(metadata) = index.presentations.get(&first.atom).copied().filter(|p| {
                p.epoch == index.payload_epoch(first_raw_source)
                    && p.version == index.semantic_version(first_raw_source)
            }) {
                self.presentation = Some((metadata.style, metadata.link));
                index.work.presentation_hits += 1;
            }
            self.metadata_checked = true;
        }
        if self.presentation.is_none() {
            if self.snapshot.is_none() {
                let markers = self
                    .markers
                    .get_or_insert_with(|| document.style_markers(first_raw_source).unwrap());
                match links.with_resolver(|r| {
                    markers.rebind_with(document, r)?;
                    markers.advance(document, r, budget)
                })? {
                    StyleMarkerStep::Span(snapshot) => self.snapshot = Some(snapshot),
                    StyleMarkerStep::BudgetExhausted => return Ok(None),
                    StyleMarkerStep::Complete => return Err(LayoutError::InvalidRange),
                }
            }
            if self.objects.is_none() {
                self.objects =
                    super::objects::ObjectWindow::new(document, first_raw_source, links.clone())
                        .map(Box::new);
            }
            if !self.objects.as_mut().unwrap().advance(document, budget)? {
                return Ok(None);
            }
            if !paid(budget) {
                return Ok(None);
            }
            let index = shape.0.borrow();
            let mut id = index.root;
            let mut mark = blank();
            loop {
                let node = &index.nodes[id];
                mark = merge(mark, node.mark);
                let Some([a, b]) = node.children else { break };
                id = if first.atom < index.nodes[b].lo { a } else { b };
            }
            mark = self.snapshot.unwrap().apply(mark);
            let node = &index.nodes[id];
            let fallback =
                node.image.is_some() && node.decision.is_some_and(|d| d.replacement.is_some());
            let literal = node.decision.is_some_and(|d| d.context.literal);
            let object = mark.link.or(node.literal).or(node.image);
            drop(index);
            let presentation = object.map(|id| links.presentation(id)).unwrap_or_default();
            let link = presentation.hit;
            let mut style = super::input::style_role(mark.style, link.is_some());
            if fallback {
                style = crate::screen::StyleRole::LINK.with(crate::screen::StyleRole::IMAGE)
            } else if presentation.image.is_some() {
                style = style.with(crate::screen::StyleRole::IMAGE)
            } else if literal || presentation.disposition == Some(LinkDisposition::Unresolved) {
                style = style.with(crate::screen::StyleRole::LITERAL_LINK)
            }
            let mut index = shape.0.borrow_mut();
            let metadata = ScalarPresentation {
                epoch: index.payload_epoch(first_raw_source),
                version: index.semantic_version(first_raw_source),
                style,
                link,
            };
            index.presentations.insert(first.atom, metadata);
            index.work.presentation_publications += 1;
            let work = self.objects.as_ref().unwrap().index_work();
            index.work.object_index_visits += work.index_visits;
            index.work.object_sort_steps += work.sort_steps;
            index.work.object_candidates += work.objects;
            self.presentation = Some((style, link));
        }
        let (style, link) = self.presentation.unwrap();
        let ch = first.chars[0];
        let raw_width = self.raw_width.unwrap();
        let kind = if ch == '\t' && metrics.scalars == 1 {
            super::text::GraphemeKind::Tab
        } else if ch.is_control() {
            super::text::GraphemeKind::Control
        } else if raw_width > 2 {
            super::text::GraphemeKind::Oversize
        } else {
            match raw_width {
                0 => super::text::GraphemeKind::ZeroWidth,
                1 => super::text::GraphemeKind::Narrow,
                _ => super::text::GraphemeKind::Wide,
            }
        };
        let width = match kind {
            super::text::GraphemeKind::Control | super::text::GraphemeKind::ZeroWidth => 0,
            super::text::GraphemeKind::Tab => 1,
            _ => raw_width.min(2) as u8,
        };
        let result = ShapeGlyph {
            text: super::payload::TextPayload::indexed(
                shape.clone(),
                source,
                (start, end),
                metrics.bytes,
                usize::from(width),
                kind,
                ch,
                metrics.scalars == 1 && ch.is_whitespace(),
                self.inline.as_ref().map(|(bytes, len)| &bytes[..*len]),
            ),
            source: envelope,
            block: document.block_at(first.source.start),
            link,
            style,
            width,
            kind,
            start,
            end,
        };
        self.result = Some(result.clone());
        Ok(Some(result))
    }
}

#[allow(clippy::large_enum_variant)] // Fixed 208-byte publication avoids one allocation per glyph.
pub enum ShapeWindowStep {
    Glyph(ShapeGlyph),
    BudgetExhausted,
    Complete,
}
/// A finite source request inside one explicit rendering fragment. Fragment
/// boundaries reset GCB state; callers use block/cell/fold projection boundaries.
pub struct ShapeWindow {
    following_source: Option<usize>,
    owner: SharedShape,
    revision: u64,
    requested: Span,
    fragment: Span,
    prefix: Option<ShapeQuery>,
    total: Option<ShapeQuery>,
    rank: Option<usize>,
    count: Option<usize>,
    cluster: Option<Box<ShapeCluster>>,
    done: bool,
}
impl ShapeWindow {
    pub fn new(shape: &SharedShape, requested: Span, fragment: Span) -> Result<Self, LayoutError> {
        if requested.start < fragment.start || requested.end > fragment.end {
            return Err(LayoutError::InvalidRange);
        }
        Ok(Self {
            following_source: None,
            owner: shape.clone(),
            revision: shape.applied_revision(),
            requested,
            fragment,
            prefix: Some(ShapeQuery::graphemes(
                shape,
                Span::new(fragment.start, requested.start),
            )),
            total: Some(ShapeQuery::graphemes(shape, fragment)),
            rank: None,
            count: None,
            cluster: None,
            done: requested.is_empty(),
        })
    }
    pub(crate) fn following_source(&self) -> Option<usize> {
        self.following_source
    }
    pub fn advance(
        &mut self,
        shape: &SharedShape,
        document: &Document,
        links: &SharedLinks,
        budget: &mut Budget,
    ) -> Result<ShapeWindowStep, LayoutError> {
        shape.check_context(&self.owner, document, links)?;
        if self.revision != document.revision() {
            if !links.can_rebind(document, self.revision) {
                return Err(LayoutError::StaleContinuation);
            }
            self.revision = document.revision();
        }
        if shape.applied_revision() != self.revision {
            return Err(LayoutError::StaleContinuation);
        }
        if self.done {
            return Ok(ShapeWindowStep::Complete);
        }
        if self.count.is_none() {
            let Some(count) = self
                .total
                .as_mut()
                .unwrap()
                .advance(shape, document, links, budget)?
            else {
                return Ok(ShapeWindowStep::BudgetExhausted);
            };
            self.count = Some(count);
            self.total = None;
        }
        if self.rank.is_none() {
            let Some(rank) = self
                .prefix
                .as_mut()
                .unwrap()
                .advance(shape, document, links, budget)?
            else {
                return Ok(ShapeWindowStep::BudgetExhausted);
            };
            self.rank = Some(rank.saturating_sub(1));
            self.prefix = None;
        }
        loop {
            if self.rank.unwrap() >= self.count.unwrap() {
                self.following_source = Some(self.fragment.end);
                self.done = true;
                return Ok(ShapeWindowStep::Complete);
            }
            let cluster = self.cluster.get_or_insert_with(|| {
                Box::new(
                    ShapeCluster::new(shape, self.fragment, self.rank.unwrap())
                        .with_fragment_barrier(),
                )
            });
            let Some(glyph) = cluster.advance(shape, document, links, budget)? else {
                if cluster.outside_fragment && paid(budget) {
                    self.cluster = None;
                    self.rank = Some(self.rank.unwrap() + 1);
                    continue;
                }
                return Ok(ShapeWindowStep::BudgetExhausted);
            };
            if !paid(budget) {
                return Ok(ShapeWindowStep::BudgetExhausted);
            }
            self.cluster = None;
            self.rank = Some(self.rank.unwrap() + 1);
            if glyph.source.end <= self.requested.start {
                continue;
            }
            if glyph.source.start >= self.requested.end {
                self.following_source = Some(glyph.source.start);
                self.done = true;
                return Ok(ShapeWindowStep::Complete);
            }
            if glyph.end.part == 0
                && shape.0.borrow().position(glyph.end.atom) >= self.requested.end
            {
                self.done = true;
            }
            return Ok(ShapeWindowStep::Glyph(glyph));
        }
    }
}

#[cfg(test)]
mod payload_lifecycle_tests {
    use super::super::payload::PayloadStep;
    use super::*;
    fn publish(d: &Document, shape: &SharedShape, rank: usize) -> ShapeGlyph {
        let mut cluster = ShapeCluster::new(shape, Span::new(0, d.source().len()), rank);
        cluster
            .advance(
                shape,
                d,
                &SharedLinks::new(d),
                &mut Budget::new(WorkLimit::unlimited()),
            )
            .unwrap()
            .unwrap()
    }
    #[test]
    fn saved_payload_rejects_real_binding_and_range_changes_but_keeps_append_cut() {
        let mut d = Document::new();
        d.enable_display_shape_effects().unwrap();
        let shape = SharedShape::new(&d).unwrap();
        let original = format!("a{}", "\u{301}".repeat(5000));
        let u = d.append(&original).unwrap();
        shape.apply(&d, &u).unwrap();
        let glyph = publish(&d, &shape, 0);
        let u = d.append("\u{301}").unwrap();
        shape.apply(&d, &u).unwrap();
        assert_eq!(
            glyph.text.collect_text(),
            original,
            "saved finite cut survives pure growth"
        );
        let old = publish(&d, &shape, 0);
        let u = d.append("\n\n[r]: /u\n").unwrap();
        shape.apply(&d, &u).unwrap();
        assert_eq!(old.text.collect_text(), format!("{original}\u{301}"));
        let mut d = Document::new();
        d.enable_display_shape_effects().unwrap();
        let shape = SharedShape::new(&d).unwrap();
        let u = d.append(&format!("`{original}")).unwrap();
        shape.apply(&d, &u).unwrap();
        let old = publish(&d, &shape, 1);
        let u = d.append("`\n").unwrap();
        assert!(u.display_shape.events().iter().any(|e| matches!(
            e.kind,
            Effect::Replacement {
                replacement: Replacement::Code { .. },
                ..
            }
        )));
        shape.apply(&d, &u).unwrap();
        let mut reader = old.text.reader();
        assert!(matches!(
            reader.next(&mut Budget::new(WorkLimit::unlimited())),
            Err(LayoutError::StaleContinuation)
        ));
        let current = publish(&d, &shape, 0);
        let mut reader = current.text.reader();
        assert!(matches!(
            reader.next(&mut Budget::new(WorkLimit::default())).unwrap(),
            PayloadStep::BudgetExhausted
        ));
    }
}

#[cfg(test)]
mod conditional_presentation_tests {
    use super::*;
    #[test]
    fn current_images_and_conditional_markers_match_the_real_display_policy() {
        for source in [
            "![][r]\n",
            "![](u)\n",
            "![![][r]](v)\n",
            "[a [![](u)](v)](w)\n",
            "![![](u)](v)\n",
            "![**x**](v)\n",
            "🇦*🇧 [x*][r]🇨\n\n",
            "a[^n]\u{301}\n\n",
        ] {
            let mut d = Document::new();
            d.enable_display_shape_effects().unwrap();
            let shape = SharedShape::new(&d).unwrap();
            let u = d.append(source).unwrap();
            shape.apply(&d, &u).unwrap();
            super::reader_tests::check(&d, &shape);
            let u = d.append("[r]: /u\n\n[^n]: note\n").unwrap();
            shape.apply(&d, &u).unwrap();
            super::reader_tests::check(&d, &shape);
            let u = d.finish();
            shape.apply(&d, &u).unwrap();
            super::reader_tests::check(&d, &shape);
        }
    }
}

#[cfg(test)]
mod seed_tests {
    use super::*;
    #[test]
    fn indexed_adoption_reuses_native_append_atoms_across_merged_updates_and_html_crlf() {
        for (prefix, suffix, expected) in [
            ("a", "é", "aé"),
            (
                "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx",
                "é",
                "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxé",
            ),
            ("a\r", "\nb", "a b"),
            ("x<spa", "n>y</span>z", "xyz"),
            ("a", "\u{301}", "a\u{301}"),
            ("", "👩‍💻", "👩‍💻"),
        ] {
            for merged in [false, true] {
                let mut document = Document::new();
                document.append(prefix).unwrap();
                document.subscribe_display_shape_effects();
                let shape = SharedShape::from_current(&document);
                let mut seed = ShapeSeed::indexed(&document);
                assert!(
                    seed.advance(&shape, &document, &mut Budget::new(WorkLimit::unlimited()))
                        .unwrap()
                );
                let mut pending: Option<Update> = None;
                for ch in suffix.chars() {
                    let update = document.append(ch.encode_utf8(&mut [0; 4])).unwrap();
                    if merged {
                        if let Some(pending) = &mut pending {
                            pending.merge(update).unwrap();
                        } else {
                            pending = Some(update);
                        }
                    } else {
                        shape.apply(&document, &update).unwrap();
                    }
                }
                if let Some(update) = pending {
                    shape.apply(&document, &update).unwrap();
                }
                let update = document.finish();
                shape.apply(&document, &update).unwrap();
                assert_eq!(
                    shape.work().source_decode_bytes,
                    0,
                    "{prefix:?} {suffix:?} merged={merged}"
                );
                let links = SharedLinks::new(&document);
                let mut query =
                    ShapeQuery::graphemes(&shape, Span::new(0, document.source().len()));
                assert_eq!(
                    query
                        .advance(
                            &shape,
                            &document,
                            &links,
                            &mut Budget::new(WorkLimit::unlimited())
                        )
                        .unwrap(),
                    Some(
                        unicode_segmentation::UnicodeSegmentation::graphemes(expected, true)
                            .count()
                    )
                );
                let mut reader =
                    ShapeReader::new(shape.clone(), Span::new(0, document.source().len()));
                let mut actual = String::new();
                loop {
                    match reader
                        .next(&mut Budget::new(WorkLimit::unlimited()))
                        .unwrap()
                    {
                        ShapeReadStep::Complete => break,
                        ShapeReadStep::Atom(atom) => actual.push_str(
                            std::str::from_utf8(&atom.utf8[..usize::from(atom.len)]).unwrap(),
                        ),
                        ShapeReadStep::BudgetExhausted => panic!("unlimited reader"),
                    }
                }
                assert_eq!(actual, expected, "{prefix:?} {suffix:?} merged={merged}");
                assert_eq!(shape.0.borrow().atoms.len(), 0);
            }
        }
    }
    #[test]
    fn cancelled_current_seed_retirement_is_retained_in_the_real_shape_counters() {
        let mut document = Document::new();
        document
            .append(&format!("{}\n", "*x* [r] ".repeat(4000)))
            .unwrap();
        document.append(&"*y* [s] ".repeat(4000)).unwrap();
        let shape = SharedShape::from_current(&document);
        let mut seed = ShapeSeed::new(&document);
        let micro = || {
            Budget::new(WorkLimit {
                queries: 1,
                objects: 1,
                bytes: 256,
                ..Default::default()
            })
        };
        for _ in 0..100_000 {
            seed.advance(&shape, &document, &mut micro()).unwrap();
            if shape.work().seed_undo_copies > 8 {
                break;
            }
        }
        let before = shape.work();
        assert!(before.seed_undo_copies > 8);
        assert_eq!(
            seed.retire(&shape, &mut Budget::new(WorkLimit::default())),
            mdtui3::Completion::BudgetExhausted
        );
        assert_eq!(shape.work(), before);
        loop {
            let old = shape.work();
            let done = seed.retire(&shape, &mut micro());
            assert!(shape.work().seed_retired_nodes - old.seed_retired_nodes <= 1);
            if done == mdtui3::Completion::Covered {
                break;
            }
        }
        assert!(
            shape.work().seed_retired_nodes - before.seed_retired_nodes >= before.seed_undo_copies
        );
        assert!(shape.work().seed_steps > before.seed_steps);
    }
    #[test]
    fn current_metadata_seed_keeps_actual_late_header_rollback_and_late_definitions() {
        for prefix in [
            "a|b\n",
            "![![][r]](v)\n\n",
            "[a **b][r]\n\n",
            "` a\r\nb `\n\n",
        ] {
            let mut document = Document::new();
            document.append(prefix).unwrap();
            document.subscribe_display_shape_effects();
            let shape = SharedShape::from_current(&document);
            let mut seed = ShapeSeed::new(&document);
            assert!(
                !seed
                    .advance(&shape, &document, &mut Budget::new(WorkLimit::default()))
                    .unwrap()
            );
            assert_eq!(shape.work().source_decode_bytes, 0);
            let mut done = false;
            for _ in 0..100_000 {
                if seed
                    .advance(
                        &shape,
                        &document,
                        &mut Budget::new(WorkLimit {
                            queries: 1,
                            objects: 1,
                            bytes: 6,
                            ..Default::default()
                        }),
                    )
                    .unwrap()
                {
                    done = true;
                    break;
                }
            }
            assert!(done);
            super::reader_tests::check(&document, &shape);
            for suffix in ["---|---\n", "[r]: /u\n\n", "tail\n"] {
                let update = document.append(suffix).unwrap();
                shape.apply(&document, &update).unwrap();
                super::reader_tests::check(&document, &shape);
            }
            let update = document.finish();
            shape.apply(&document, &update).unwrap();
            super::reader_tests::check(&document, &shape);
            assert_eq!(shape.work().source_decode_bytes, document.source().len());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_digit_of_the_largest_native_footnote_number_has_the_shared_unicode_policy() {
        let number = usize::MAX;
        let entity = Entity {
            number: Some(number),
            chars: ['\0'; 8],
            properties: [Properties::of(' '); 8],
            scalars: (number.ilog10() + 1) as u8,
            source: Span::new(3, 9),
        };
        let actual = (0..usize::from(entity.scalars))
            .map(|at| entity.character(at))
            .collect::<String>();
        let expected = if usize::BITS == 64 {
            "¹⁸⁴⁴⁶⁷⁴⁴⁰⁷³⁷⁰⁹⁵⁵¹⁶¹⁵"
        } else {
            "⁴²⁹⁴⁹⁶⁷²⁹⁵"
        };
        assert_eq!(actual, expected);
        for at in 0..usize::from(entity.scalars) {
            assert_eq!(entity.property(at), Properties::of(entity.character(at)));
        }
    }
    use unicode_segmentation::UnicodeSegmentation;
    #[test]
    fn explicit_projection_fragment_boundaries_are_gcb_context_barriers() {
        let mut d = Document::new();
        d.enable_display_shape_effects().unwrap();
        let shape = SharedShape::new(&d).unwrap();
        let text = format!("a{}", "\u{301}".repeat(10_000));
        let update = d.append(&text).unwrap();
        shape.apply(&d, &update).unwrap();
        let links = SharedLinks::new(&d);
        for fragment in [
            Span::new(0, text.len()),
            Span::new(0, 1),
            Span::new(1, text.len()),
        ] {
            let mut cluster = ShapeCluster::new(&shape, fragment, 0);
            let glyph = cluster
                .advance(&shape, &d, &links, &mut Budget::new(WorkLimit::unlimited()))
                .unwrap()
                .unwrap();
            assert_eq!(
                glyph.text.collect_text(),
                &text[fragment.start..fragment.end]
            );
            assert_eq!(glyph.source, fragment);
            assert_eq!(glyph.width, u8::from(fragment.start == 0));
            assert!(glyph.text.is_indexed());
        }
    }
    fn count(shape: &SharedShape, d: &Document, source: Span, micro: bool) -> usize {
        let links = SharedLinks::new(d);
        let mut query = ShapeQuery::graphemes(shape, source);
        assert_eq!(
            query
                .advance(shape, d, &links, &mut Budget::new(WorkLimit::default()))
                .unwrap(),
            None
        );
        for _ in 0..1_000_000 {
            let limit = if micro {
                WorkLimit {
                    queries: 1,
                    objects: 1,
                    bytes: 6,
                    ..WorkLimit::default()
                }
            } else {
                WorkLimit::unlimited()
            };
            let mut b = Budget::new(limit);
            let result = query.advance(shape, d, &links, &mut b).unwrap();
            if micro {
                assert!(b.used().queries <= 1 && b.used().objects <= 1 && b.used().bytes <= 6);
            }
            if let Some(result) = result {
                return result;
            }
        }
        panic!("shape query did not finish")
    }
    #[test]
    fn typed_source_and_rollback_keep_properties_append_only() {
        for text in [
            "a**\u{301}**z\n",
            "👩&#x200d;💻\n",
            "`a\n b`\n",
            "🇦[🇧][r]🇨\n\n",
        ] {
            let mut d = Document::new();
            d.enable_display_shape_effects().unwrap();
            let shape = SharedShape::new(&d).unwrap();
            let u = d.append(text).unwrap();
            shape.apply(&d, &u).unwrap();
            let source = Span::new(0, d.source().len());
            let expected = super::super::visibility::visible_input(&d, source).unwrap();
            let display: String = expected
                .tokens()
                .iter()
                .map(|t| match t.kind {
                    super::super::input::TokenKind::Text => t.text.collect_text(),
                    super::super::input::TokenKind::SoftBreak => " ".to_owned(),
                    super::super::input::TokenKind::HardBreak => "".to_owned(),
                })
                .collect();
            assert_eq!(
                count(&shape, &d, source, true),
                display.graphemes(true).count(),
                "{text:?}"
            );
            let u = d.append("[r]: /u\n").unwrap();
            shape.apply(&d, &u).unwrap();
            let u = d.finish();
            shape.apply(&d, &u).unwrap();
            assert_eq!(shape.work().source_decode_bytes, d.source().len());
        }
    }
    #[test]
    fn giant_plain_query_uses_summaries_and_preserves_them_on_append() {
        let mut d = Document::new();
        d.enable_display_shape_effects().unwrap();
        let shape = SharedShape::new(&d).unwrap();
        let text = format!("a{}", "\u{301}".repeat(100_000));
        let u = d.append(&text).unwrap();
        shape.apply(&d, &u).unwrap();
        assert_eq!(count(&shape, &d, Span::new(0, text.len()), true), 1);
        let old = shape.work();
        let u = d.append("\u{301}").unwrap();
        shape.apply(&d, &u).unwrap();
        assert_eq!(
            shape.work().source_decode_bytes - old.source_decode_bytes,
            2
        );
        assert_eq!(count(&shape, &d, Span::new(0, d.source().len()), true), 1);
        assert!(shape.work().query_properties - old.query_properties < 128);
    }
}

#[cfg(test)]
mod reader_tests {
    use super::*;
    use unicode_segmentation::UnicodeSegmentation;
    pub(super) fn check(d: &Document, shape: &SharedShape) {
        let source = Span::new(0, d.source().len());
        let links = SharedLinks::new(d);
        let mut query = ShapeQuery::graphemes(shape, source);
        let count = query
            .advance(shape, d, &links, &mut Budget::new(WorkLimit::unlimited()))
            .unwrap()
            .unwrap();
        let expected = super::super::visibility::visible_input(d, source).unwrap();
        let expected: String = expected
            .tokens()
            .iter()
            .map(|t| match t.kind {
                super::super::input::TokenKind::Text => t.text.collect_text(),
                super::super::input::TokenKind::SoftBreak => " ".to_owned(),
                super::super::input::TokenKind::HardBreak => "".to_owned(),
            })
            .collect();
        let mut reader = ShapeReader::new(shape.clone(), source);
        let mut actual = String::new();
        loop {
            match reader
                .next(&mut Budget::new(WorkLimit::unlimited()))
                .unwrap_or_else(|e| panic!("reader {e:?} for {:?}", d.source()))
            {
                ShapeReadStep::Complete => break,
                ShapeReadStep::Atom(atom) => actual
                    .push_str(std::str::from_utf8(&atom.utf8[..usize::from(atom.len)]).unwrap()),
                _ => panic!(),
            }
        }
        if actual != expected {
            eprintln!("objects {:?}", shape.0.borrow().objects);
            for (i, n) in shape.0.borrow().nodes.iter().enumerate() {
                if n.literal.is_some() || n.image.is_some() {
                    eprintln!(
                        "node {i} {}..{} literal {:?} lazy {:?} image {:?} basehidden {} decision {:?}",
                        n.lo,
                        n.hi,
                        n.literal,
                        n.literal_lazy,
                        n.image,
                        n.mark.base_hidden,
                        n.decision
                            .map(|v| (v.hidden, v.context.literal, v.context.literal_bound))
                    );
                }
            }
        }
        assert_eq!(actual, expected, "{:?}", d.source());
        assert_eq!(
            count,
            expected.graphemes(true).count(),
            "source prefix {:?}, source suffix {:?}",
            &d.source()[..d.source().len().min(60)],
            &d.source()[d.source().len().saturating_sub(60)..]
        );
        let atom_count = shape.0.borrow().atoms.len();
        for rank in 0..count {
            let mut select = ShapeSelect::new(shape, source, rank);
            let position = select
                .advance(shape, d, &links, &mut Budget::new(WorkLimit::unlimited()))
                .unwrap()
                .unwrap();
            assert!(position.atom < atom_count);
        }
    }
    #[test]
    fn real_rollback_and_replacement_readers_match_existing_display() {
        for text in [
            "a**\u{301}**z\n",
            "👩&#x200d;💻\n",
            "`a\n b`\n",
            "🇦[🇧][r]🇨\n\n",
        ] {
            let mut d = Document::new();
            d.enable_display_shape_effects().unwrap();
            let shape = SharedShape::new(&d).unwrap();
            let u = d.append(text).unwrap();
            shape.apply(&d, &u).unwrap();
            check(&d, &shape);
            let u = d.append("[r]: /u\n").unwrap();
            shape.apply(&d, &u).unwrap();
            check(&d, &shape);
            let u = d.finish();
            shape.apply(&d, &u).unwrap();
            check(&d, &shape);
        }
        for prior in ["[prior\n", "*prior\n", "`prior\n"] {
            let mut d = Document::new();
            d.enable_display_shape_effects().unwrap();
            let shape = SharedShape::new(&d).unwrap();
            for text in [
                prior,
                &format!("a{}|b\n", "x".repeat(2000)),
                "---|---\n",
                "x|y\n",
            ] {
                let u = d.append(text).unwrap();
                shape.apply(&d, &u).unwrap();
                check(&d, &shape);
            }
            let u = d.finish();
            shape.apply(&d, &u).unwrap();
            check(&d, &shape);
            assert_eq!(shape.work().source_decode_bytes, d.source().len());
        }
    }
}

#[cfg(test)]
mod publication_tests {
    use super::*;
    #[test]
    fn giant_cluster_publication_reads_no_prefix_payload() {
        let mut document = Document::new();
        document.enable_display_shape_effects().unwrap();
        let shape = SharedShape::new(&document).unwrap();
        let text = format!("👩{}‍💻", "\u{301}".repeat(100_000));
        let update = document.append(&text).unwrap();
        shape.apply(&document, &update).unwrap();
        let links = SharedLinks::new(&document);
        let old = shape.work();
        let mut task = ShapeCluster::new(&shape, Span::new(0, text.len()), 0);
        let mut result = None;
        for _ in 0..20_000 {
            let mut budget = Budget::new(WorkLimit {
                queries: 1,
                objects: 1,
                bytes: 6,
                ..WorkLimit::default()
            });
            result = task
                .advance(&shape, &document, &links, &mut budget)
                .unwrap();
            assert!(
                budget.used().queries <= 1
                    && budget.used().objects <= 1
                    && budget.used().bytes <= 6
            );
            if result.is_some() {
                break;
            }
        }
        let glyph = result.unwrap();
        assert_eq!(glyph.width, 2);
        assert_eq!(glyph.source, Span::new(0, text.len()));
        assert!(glyph.text.is_indexed());
        assert_eq!(glyph.text.len(), text.len());
        assert_eq!(shape.work().source_decode_bytes, old.source_decode_bytes);
        assert!(shape.work().query_properties - old.query_properties < 512);
        assert_eq!(glyph.text.collect_text(), text);
    }
}

#[cfg(test)]
mod streamed_style_and_adoption_tests {
    use super::*;
    use unicode_segmentation::UnicodeSegmentation;
    use unicode_width::UnicodeWidthStr;
    #[test]
    fn many_native_style_spans_stream_in_both_fold_directions_with_fixed_snapshot_storage() {
        for n in [8, 64, 512, 4096] {
            let source = format!("{}界{}", "*".repeat(n), "* y".repeat(n));
            let mut d = Document::new();
            d.enable_display_shape_effects().unwrap();
            let shape = SharedShape::new(&d).unwrap();
            let u = d.append(&source).unwrap();
            shape.apply(&d, &u).unwrap();
            let u = d.finish();
            shape.apply(&d, &u).unwrap();
            let visible =
                super::super::visibility::visible_input(&d, Span::new(0, source.len())).unwrap();
            let oracle: String = visible
                .tokens()
                .iter()
                .map(|t| t.text.collect_text())
                .collect();
            let links = SharedLinks::new(&d);
            links.set_shape(shape.clone()).unwrap();
            let mut peak = 0;
            for (kind, span, expected) in [
                (QueryKind::Gcb, Span::new(0, n), 0),
                (
                    QueryKind::Gcb,
                    Span::new(0, source.len()),
                    oracle.graphemes(true).count(),
                ),
                (
                    QueryKind::Width,
                    Span::new(0, source.len()),
                    UnicodeWidthStr::width(oracle.as_str()),
                ),
            ] {
                let mut q = ShapeQuery::new(&shape, span, kind, 0);
                let before = shape.work();
                assert!(
                    q.advance_answer(&shape, &d, &links, &mut Budget::new(WorkLimit::default()))
                        .unwrap()
                        .is_none()
                );
                assert_eq!(shape.work(), before);
                let mut answer = None;
                for _ in 0..(source.len() + 1) * 256 * (source.len().ilog2() as usize + 1) {
                    let mut b = Budget::new(WorkLimit {
                        queries: 1,
                        objects: 1,
                        bytes: 6,
                        ..Default::default()
                    });
                    answer = q.advance(&shape, &d, &links, &mut b).unwrap();
                    peak = peak.max(q.scratch_capacity_bytes());
                    assert!(b.used().queries <= 1 && b.used().objects <= 1 && b.used().bytes <= 6);
                    if answer.is_some() {
                        break;
                    }
                }
                assert_eq!(answer, Some(expected), "n={n} kind={kind:?}");
            }
            let height = source.len().ilog2() as usize + 2;
            assert!(
                peak <= 8 * (height + 2) * std::mem::size_of::<QueryFrame>()
                    + 8 * std::mem::size_of::<ShapeQuery>(),
                "n={n} task bytes={peak}"
            );
            assert_eq!(shape.work().style_buffer_peak_entries, 1);
            assert_eq!(
                shape.work().style_buffer_peak_bytes,
                std::mem::size_of::<ResolvedStyleSpan>()
            );
            assert_eq!(shape.work().source_decode_bytes, source.len());
            eprintln!(
                "streamed native styles n={n} per_frame_snapshot_peak={} snapshot_bytes={} query_task_peak_bytes={peak} index_capacity={} work={:?}",
                shape.work().style_buffer_peak_entries,
                shape.work().style_buffer_peak_bytes,
                shape.capacity_bytes(),
                shape.work()
            );
        }
    }
    #[test]
    fn actual_micro_seed_grows_index_chunks_without_relocating_old_properties_or_nodes() {
        let mut d = Document::new();
        d.append(&format!("a{}", "\u{301}".repeat(65536))).unwrap();
        d.finish();
        let shape = SharedShape::from_current(&d);
        let mut seed = ShapeSeed::new(&d);
        let (position, node) = {
            let index = shape.0.borrow();
            (
                &index.positions[0] as *const usize,
                &index.nodes[0] as *const Node,
            )
        };
        let mut atom = None;
        let mut complete = false;
        for _ in 0..d.source().len() * 8 {
            let before = shape.work();
            let mut b = Budget::new(WorkLimit {
                queries: 1,
                objects: 1,
                bytes: 6,
                ..Default::default()
            });
            let done = seed.advance(&shape, &d, &mut b).unwrap();
            assert!(b.used().queries <= 1 && b.used().objects <= 1 && b.used().bytes <= 6);
            let index = shape.0.borrow();
            assert_eq!(&index.positions[0] as *const usize, position);
            assert_eq!(&index.nodes[0] as *const Node, node);
            if index.atoms.len() != 0 {
                let address = &index.atoms[0] as *const RawAtom;
                if let Some(old) = atom {
                    assert_eq!(old, address)
                } else {
                    atom = Some(address)
                }
            }
            drop(index);
            assert!(shape.work().source_decode_bytes - before.source_decode_bytes <= 6);
            assert_eq!(shape.work().index_relocated_items, 0);
            if done {
                complete = true;
                break;
            }
        }
        assert!(complete);
        assert_eq!(shape.work().source_decode_bytes, d.source().len());
        eprintln!(
            "micro adoption actual_source_bytes={} chunks={} reserved_bytes={} old_items_relocated={} capacity={}",
            d.source().len(),
            shape.work().index_chunk_allocations,
            shape.work().index_reserved_bytes,
            shape.work().index_relocated_items,
            shape.capacity_bytes()
        );
    }
}
