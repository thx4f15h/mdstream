use mdstream::*;
use std::collections::BTreeMap;

fn one() -> Budget {
    Budget::new(WorkLimit {
        queries: 1,
        objects: 1,
        bytes: 6,
        ..Default::default()
    })
}
fn blank() -> DisplayShapeMark {
    DisplayShapeMark {
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
fn merge(a: DisplayShapeMark, b: DisplayShapeMark) -> DisplayShapeMark {
    let replace = b.link.is_some()
        && if a.object_syntax && b.object_syntax {
            b.link < a.link || b.link == a.link && b.order >= a.order
        } else {
            b.order >= a.order
        };
    DisplayShapeMark {
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
#[derive(Clone)]
struct Node {
    children: Option<[usize; 2]>,
    mark: DisplayShapeMark,
    saved: Option<DisplayShapeMark>,
}
struct Model {
    raw: Vec<(usize, char, usize)>,
    end: usize,
    decoded: usize,
    nodes: Vec<Node>,
    root: usize,
    capacity: usize,
    undo: Option<Vec<usize>>,
    entries: Vec<(Span, DisplayShapeReplacement)>,
    spaces: BTreeMap<usize, u8>,
    last_event: usize,
    visits: usize,
    undo_peak: usize,
    links: BTreeMap<LinkId, (LinkInfo, LinkTarget, bool)>,
    parents: BTreeMap<LinkId, Option<LinkId>>,
    parent_undo: Option<BTreeMap<LinkId, Option<Option<LinkId>>>>,
    maths: BTreeMap<MathId, (Math, Option<SyntaxState>)>,
    feet: BTreeMap<FootnoteReferenceId, (FootnoteReference, Option<SyntaxState>)>,
}
impl Model {
    fn new() -> Self {
        Self {
            raw: vec![],
            end: 0,
            decoded: 0,
            nodes: vec![Node {
                children: None,
                mark: blank(),
                saved: None,
            }],
            root: 0,
            capacity: 1,
            undo: None,
            entries: vec![],
            spaces: BTreeMap::new(),
            last_event: 0,
            visits: 0,
            undo_peak: 0,
            links: BTreeMap::new(),
            parents: BTreeMap::new(),
            parent_undo: None,
            maths: BTreeMap::new(),
            feet: BTreeMap::new(),
        }
    }
    fn node(&mut self) -> usize {
        let id = self.nodes.len();
        self.nodes.push(Node {
            children: None,
            mark: blank(),
            saved: None,
        });
        id
    }
    fn mark(&mut self, id: usize, lo: usize, hi: usize, span: Span, mark: DisplayShapeMark) {
        self.visits += 1;
        if span.start <= lo && hi <= span.end {
            if let Some(undo) = &mut self.undo {
                if self.nodes[id].saved.is_none() {
                    self.nodes[id].saved = Some(self.nodes[id].mark);
                    undo.push(id);
                }
                self.undo_peak = self.undo_peak.max(undo.len());
            }
            self.nodes[id].mark = merge(self.nodes[id].mark, mark);
            return;
        }
        let children = if let Some(c) = self.nodes[id].children {
            c
        } else {
            let a = self.node();
            let b = self.node();
            self.nodes[id].children = Some([a, b]);
            [a, b]
        };
        let mid = (lo + hi) / 2;
        for (child, start, end) in [(children[0], lo, mid), (children[1], mid, hi)] {
            if span.start < end && start < span.end {
                self.mark(child, start, end, span, mark);
            }
        }
    }
    fn at(&self, at: usize) -> DisplayShapeMark {
        let (mut id, mut lo, mut hi, mut mark) = (self.root, 0, self.capacity, blank());
        loop {
            mark = merge(mark, self.nodes[id].mark);
            let Some(c) = self.nodes[id].children else {
                return mark;
            };
            let mid = (lo + hi) / 2;
            if at < mid {
                id = c[0];
                hi = mid;
            } else {
                id = c[1];
                lo = mid;
            }
        }
    }
    fn apply(&mut self, d: &Document, u: &Update) {
        assert!(u.display_shape.enabled());
        assert_eq!(u.document, Some(d.id()));
        for effect in u.display_shape.events() {
            assert!(effect.sequence > self.last_event);
            self.last_event = effect.sequence;
            match &effect.kind {
                DisplayShapeEffectKind::SourceAppend(span) => {
                    assert_eq!(span.start, self.end);
                    for (offset, ch) in d.source()[span.range()].char_indices() {
                        self.raw.push((span.start + offset, ch, ch.len_utf8()));
                        self.decoded += ch.len_utf8();
                    }
                    self.end = span.end;
                    while self.capacity < self.end {
                        let old = self.root;
                        let root = self.node();
                        let right = self.node();
                        self.nodes[root].children = Some([old, right]);
                        self.root = root;
                        self.capacity *= 2;
                    }
                }
                DisplayShapeEffectKind::Begin(DisplayShapeDomain::Annotations) => {
                    assert!(self.undo.is_none());
                    self.undo = Some(Vec::new());
                }
                DisplayShapeEffectKind::Commit(DisplayShapeDomain::Annotations) => {
                    for id in self.undo.take().unwrap() {
                        self.nodes[id].saved = None;
                    }
                }
                DisplayShapeEffectKind::Rollback(DisplayShapeDomain::Annotations) => {
                    for id in self.undo.take().unwrap() {
                        self.nodes[id].mark = self.nodes[id].saved.take().unwrap();
                    }
                }
                DisplayShapeEffectKind::Mark { source, mark } => {
                    self.mark(self.root, 0, self.capacity, *source, *mark)
                }
                DisplayShapeEffectKind::Replacement {
                    source,
                    replacement: DisplayShapeReplacement::Spaces { columns },
                } => {
                    self.spaces.insert(source.start, *columns);
                }
                DisplayShapeEffectKind::Replacement {
                    source,
                    replacement,
                } => self.entries.push((*source, *replacement)),
                DisplayShapeEffectKind::TruncateReplacements { entries } => {
                    self.entries.truncate(*entries)
                }
                DisplayShapeEffectKind::LinkSource { id, source } => {
                    if let Some(value) = self.links.get_mut(id) {
                        value.0.source = *source;
                    }
                }
                DisplayShapeEffectKind::Link { id, info, target } => {
                    self.links.insert(*id, (*info, *target, true));
                }
                DisplayShapeEffectKind::LinkActive { id, active } => {
                    if let Some(value) = self.links.get_mut(id) {
                        value.2 = *active;
                    }
                }
                DisplayShapeEffectKind::LinkState { id, state } => {
                    if let Some(value) = self.links.get_mut(id) {
                        value.0.state = *state;
                    }
                }
                DisplayShapeEffectKind::Begin(DisplayShapeDomain::LinkParents) => {
                    self.parent_undo = Some(BTreeMap::new())
                }
                DisplayShapeEffectKind::Commit(DisplayShapeDomain::LinkParents) => {
                    self.parent_undo = None
                }
                DisplayShapeEffectKind::Rollback(DisplayShapeDomain::LinkParents) => {
                    for (id, old) in self.parent_undo.take().unwrap() {
                        if let Some(old) = old {
                            self.parents.insert(id, old);
                        } else {
                            self.parents.remove(&id);
                        }
                    }
                }
                DisplayShapeEffectKind::LinkParent { id, parent } => {
                    if let Some(undo) = &mut self.parent_undo {
                        undo.entry(*id).or_insert(self.parents.get(id).copied());
                    }
                    self.parents.insert(*id, *parent);
                }
                DisplayShapeEffectKind::Math { id, math, state } => {
                    self.maths.insert(*id, (*math, *state));
                }
                DisplayShapeEffectKind::FootnoteReference {
                    id,
                    reference,
                    state,
                } => {
                    self.feet.insert(*id, (*reference, *state));
                }
                _ => {}
            }
        }
        for (&id, (info, _, active)) in &self.links {
            if *active {
                assert_eq!(d.link_info(id), Some(info));
                assert_eq!(d.link_parent(id), self.parents.get(&id).copied().flatten());
            }
        }
        for (&id, (_, state)) in &self.maths {
            assert_eq!(d.math_state(id), *state);
        }
        for (&id, (_, state)) in &self.feet {
            assert_eq!(d.footnote_reference_state(id), *state);
        }
        assert_eq!(self.end, d.source().len());
        assert_eq!(self.decoded, self.end);
    }
    fn text(&self, d: &Document, source: Span, resolver: &mut LinkResolver) -> String {
        self.text_with_literals(d, source, resolver, false)
    }
    fn text_with_literals(
        &self,
        d: &Document,
        source: Span,
        resolver: &mut LinkResolver,
        presentation: bool,
    ) -> String {
        let mut literals = Vec::new();
        if presentation {
            for (&id, (info, _, active)) in &self.links {
                if !active {
                    continue;
                }
                let mut activity = d.link_activity(id).unwrap();
                let value = loop {
                    if let Some(value) = activity.advance_with(d, resolver, &mut one()).unwrap() {
                        break value;
                    }
                };
                if value == LinkDisposition::Unresolved {
                    literals.extend([
                        Span::new(info.source.start, info.label.start),
                        Span::new(info.label.end, info.source.end),
                    ]);
                }
            }
        }
        if source.is_empty() {
            return String::new();
        }
        let mut cursor = d.style_markers(source).unwrap();
        let mut spans = Vec::new();
        assert_eq!(
            cursor
                .advance(d, resolver, &mut Budget::new(WorkLimit::default()))
                .unwrap(),
            StyleMarkerStep::BudgetExhausted
        );
        loop {
            let mut budget = one();
            match cursor.advance(d, resolver, &mut budget).unwrap() {
                StyleMarkerStep::Span(s) => spans.push(s),
                StyleMarkerStep::Complete => break,
                _ => {}
            }
            assert!(
                budget.used().queries <= 1
                    && budget.used().objects <= 1
                    && budget.used().bytes == 0
            );
        }
        let mut out = String::new();
        let mut style = 0;
        let first = self.raw.partition_point(|&(at, _, _)| at < source.start);
        for &(at, ch, len) in &self.raw[first..] {
            if at >= source.end {
                break;
            }
            while spans[style].source.end <= at {
                style += 1;
            }
            let mark = spans[style].apply(self.at(at));
            let mut hidden = mark.style.contains(Style::HIDDEN);
            for id in [mark.hide_owner, mark.hide_alternative]
                .into_iter()
                .flatten()
            {
                let mut task = d.link_activity(id).unwrap();
                loop {
                    if let Some(value) = task.advance_with(d, resolver, &mut one()).unwrap() {
                        hidden |= value == LinkDisposition::Active;
                        break;
                    }
                }
            }
            if hidden && !literals.iter().any(|s| s.start <= at && at < s.end) {
                continue;
            }
            let entry = self
                .entries
                .get(self.entries.partition_point(|(s, _)| s.end <= at))
                .filter(|(s, _)| s.start <= at);
            if let Some(&(span, DisplayShapeReplacement::Entity { utf8, len })) = entry {
                if at == span.start {
                    out.push_str(std::str::from_utf8(&utf8[..usize::from(len)]).unwrap());
                }
                continue;
            }
            let columns = self.spaces.get(&at).copied();
            if let Some(&(_, DisplayShapeReplacement::Code { leading, trailing })) = entry {
                let trimmed = leading.start <= at && at < leading.end
                    || trailing.start <= at && at < trailing.end;
                if let Some(columns) = columns {
                    out.extend(std::iter::repeat_n(
                        ' ',
                        usize::from(columns) - usize::from(trimmed),
                    ));
                } else if !trimmed {
                    out.push(if ch == '\n' { ' ' } else { ch });
                }
            } else if let Some(columns) = columns {
                out.extend(std::iter::repeat_n(' ', usize::from(columns)));
            } else {
                out.push(ch);
            }
            let _ = len;
        }
        out
    }
    // Consumer policy: only outer active images own presentation. A nested
    // empty image does not recursively create a fallback inside that alt.
    fn presented(&self, d: &Document) -> String {
        let mut resolver = d.link_resolver();
        let mut selected = Vec::new();
        for (&id, (info, _, active)) in &self.links {
            if !active || info.kind != LinkKind::Image {
                continue;
            }
            let mut task = d.link_activity(id).unwrap();
            let disposition = loop {
                if let Some(v) = task.advance_with(d, &mut resolver, &mut one()).unwrap() {
                    break v;
                }
            };
            if disposition == LinkDisposition::Active {
                selected.push((id, *info));
            }
        }
        selected.sort_by_key(|(id, info)| (info.source.start, *id));
        let mut roots = Vec::<(LinkId, LinkInfo)>::new();
        for item in selected {
            if !roots.last().is_some_and(|(_, outer)| {
                outer.source.start <= item.1.source.start && item.1.source.end <= outer.source.end
            }) {
                roots.push(item);
            }
        }
        let mut out = String::new();
        let mut at = 0;
        for (_, info) in roots {
            out.push_str(&self.text_with_literals(
                d,
                Span::new(at, info.source.start),
                &mut resolver,
                true,
            ));
            let alt = self.text_with_literals(d, info.label, &mut resolver, true);
            out.push_str(if alt.is_empty() { "[image]" } else { &alt });
            at = info.source.end;
        }
        out.push_str(&self.text_with_literals(d, Span::new(at, self.end), &mut resolver, true));
        out
    }
    fn check(&self, d: &Document) {
        let mut resolver = d.link_resolver();
        let actual = self.text(d, Span::new(0, self.end), &mut resolver);
        let expected: String = d
            .display_runs_in(Span::new(0, self.end))
            .unwrap()
            .filter(|r| !r.style.contains(Style::HIDDEN))
            .map(|r| r.text)
            .collect();
        assert_eq!(actual, expected, "{}", d.source());
        let mut cursor = d.style_markers(Span::new(0, self.end)).unwrap();
        let mut markers = Vec::new();
        loop {
            match cursor.advance(d, &mut resolver, &mut one()).unwrap() {
                StyleMarkerStep::Span(s) => markers.push(s),
                StyleMarkerStep::Complete => break,
                _ => {}
            }
        }
        for run in d.runs_in(Span::new(0, self.end)).unwrap() {
            let snapshot = markers[markers.partition_point(|m| m.source.end <= run.source.start)];
            let mark = snapshot.apply(self.at(run.source.start));
            assert_eq!(
                (mark.emphasis_depth, mark.strong_depth, snapshot.style_pair),
                (run.emphasis_depth, run.strong_depth, run.style_pair)
            );
            for style in [
                Style::EMPHASIS,
                Style::STRONG,
                Style::STRIKE,
                Style::CODE,
                Style::MATH,
            ] {
                assert_eq!(mark.style.contains(style), run.style.contains(style));
            }
        }
        while resolver.memo_entries() != 0 {
            resolver.retire(&mut one());
        }
    }
}

#[test]
fn real_effects_rewind_only_semantics_while_raw_properties_keep_growing() {
    for prior in ["[prior\n", "*prior\n", "`prior\n", "$prior\n"] {
        for ending in ["|b\n---|---\n", "|b\nwrong\n", " no pipe\n", ""] {
            let first = match prior.as_bytes()[0] {
                b'[' => "end](u) *x* ",
                b'*' => "end*x* ",
                b'`' => "end` *x* ",
                _ => "end$ *x* ",
            };
            let mut d = Document::new();
            d.enable_display_shape_effects().unwrap();
            let mut m = Model::new();
            for chunk in [
                prior,
                &format!("{first}[h](v) $q$ [^n] &amp; {}", "a\u{301}".repeat(1200)),
                ending,
            ] {
                let u = d.append(chunk).unwrap();
                m.apply(&d, &u);
                m.check(&d);
            }
            let u = d.finish();
            m.apply(&d, &u);
            m.check(&d);
            if ending == "|b\n---|---\n" {
                assert_eq!(d.tables().len(), 1);
                assert_eq!(
                    m.text(&d, Span::new(0, prior.len()), &mut d.link_resolver()),
                    prior
                );
            }
            assert_eq!(m.decoded, d.source().len());
            assert!(m.undo_peak <= m.visits);
        }
    }
}

#[test]
fn effects_merge_order_and_immutable_replacements_survive_slot_reuse() {
    let mut d = Document::new();
    d.enable_display_shape_effects().unwrap();
    let mut step = Model::new();
    let mut all = d.append("`prior\n").unwrap();
    step.apply(&d, &all);
    step.check(&d);
    let pending = d
        .append(&format!("end` &amp; {}", "a".repeat(1100)))
        .unwrap();
    step.apply(&d, &pending);
    step.check(&d);
    let old = pending
        .display_shape
        .events()
        .iter()
        .find_map(|e| match e.kind {
            DisplayShapeEffectKind::Replacement {
                source,
                replacement: replacement @ DisplayShapeReplacement::Code { .. },
            } => Some((source, replacement)),
            _ => None,
        })
        .expect("old spanning code is published before header confirmation");
    let old_source = d.text(old.0).unwrap().to_owned();
    let old_payload = match old.1 {
        DisplayShapeReplacement::Code { leading, trailing } => d
            .text(Span::new(leading.end, trailing.start))
            .unwrap()
            .replace('\n', " "),
        _ => unreachable!(),
    };
    assert_eq!(old_payload, "prior end");
    all.merge(pending).unwrap();
    let replay = d.append("|b\n---|---\n").unwrap();
    step.apply(&d, &replay);
    step.check(&d);
    assert_eq!(d.tables().len(), 1);
    // Replay the actual entry-count operations: old Code occupied slot zero,
    // confirmation truncates it and publishes a distinct Entity into that slot.
    let mut slot = 0;
    let mut old_slot = None;
    let mut truncated = false;
    let mut reused = None;
    for e in all
        .display_shape
        .events()
        .iter()
        .chain(replay.display_shape.events())
    {
        match e.kind {
            DisplayShapeEffectKind::TruncateReplacements { entries } => {
                slot = entries;
                truncated = true;
            }
            DisplayShapeEffectKind::Replacement {
                replacement: DisplayShapeReplacement::Spaces { .. },
                ..
            } => {}
            DisplayShapeEffectKind::Replacement {
                source,
                replacement,
            } => {
                if (source, replacement) == old {
                    old_slot = Some(slot);
                } else if truncated && old_slot == Some(slot) {
                    reused = Some((source, replacement));
                }
                slot += 1;
            }
            _ => {}
        }
    }
    assert!(truncated);
    let new = reused.expect("real truncation followed by reuse of the old entry slot");
    assert_ne!(old, new);
    match new.1 {
        DisplayShapeReplacement::Entity { utf8, len } => {
            assert_eq!(&utf8[..usize::from(len)], b"&")
        }
        _ => panic!("new slot owns an entity, not the former code"),
    }
    assert_eq!(d.text(old.0), Some(old_source.as_str()));
    match old.1 {
        DisplayShapeReplacement::Code { leading, trailing } => assert_eq!(
            d.text(Span::new(leading.end, trailing.start))
                .unwrap()
                .replace('\n', " "),
            old_payload
        ),
        _ => unreachable!(),
    }
    all.merge(replay).unwrap();
    let tail = d.append("&#65;\n").unwrap();
    step.apply(&d, &tail);
    step.check(&d);
    all.merge(tail).unwrap();
    let eof = d.finish();
    step.apply(&d, &eof);
    step.check(&d);
    all.merge(eof).unwrap();
    let mut merged = Model::new();
    merged.apply(&d, &all);
    merged.check(&d);
    assert_eq!(
        step.text(&d, Span::new(0, d.source().len()), &mut d.link_resolver()),
        merged.text(&d, Span::new(0, d.source().len()), &mut d.link_resolver())
    );
    assert!(
        all.display_shape
            .events()
            .windows(2)
            .all(|w| w[0].sequence < w[1].sequence)
    );
    assert_eq!(d.text(old.0), Some(old_source.as_str()));
    assert!(d.enable_display_shape_effects().is_err());
}

#[test]
fn classified_style_cursor_recovers_once_and_preserves_extent_generation() {
    let mut d = Document::new();
    d.enable_display_shape_effects().unwrap();
    let first = d.append("🇦*🇧 [x*][r]🇨\n\n").unwrap();
    let generation = first
        .display_shape
        .events()
        .iter()
        .filter_map(|e| {
            if let DisplayShapeEffectKind::ScopeEvent { generation, .. } = e.kind {
                Some(generation)
            } else {
                None
            }
        })
        .max()
        .unwrap();
    let mut m = Model::new();
    m.apply(&d, &first);
    m.check(&d);
    let mut resolver = d.link_resolver();
    let source = Span::new(0, d.source().len());
    let mut cursor = d.style_markers(source).unwrap();
    let before = resolver.style_work();
    assert_eq!(
        cursor
            .advance(&d, &mut resolver, &mut Budget::new(WorkLimit::default()))
            .unwrap(),
        StyleMarkerStep::BudgetExhausted
    );
    assert_eq!(resolver.style_work(), before);
    let mut hidden = Vec::new();
    loop {
        match cursor.advance(&d, &mut resolver, &mut one()).unwrap() {
            StyleMarkerStep::Span(s) => {
                if s.marker_hidden {
                    hidden.push(d.text(s.source).unwrap().to_owned());
                }
            }
            StyleMarkerStep::Complete => break,
            _ => {}
        }
    }
    assert_eq!(hidden, vec!["*", "*"]);
    let events = resolver.style_work().events;
    let mut again = d.style_markers(source).unwrap();
    while again.advance(&d, &mut resolver, &mut one()).unwrap() != StyleMarkerStep::Complete {}
    assert_eq!(resolver.style_work().events, events);
    let u = d.append("[r]: /u\n").unwrap();
    m.apply(&d, &u);
    m.check(&d);
    assert_eq!(m.text(&d, source, &mut d.link_resolver()), "🇦*🇧 x*🇨\n\n");
    assert!(u.display_shape.events().iter().all(
        |e| !matches!(e.kind,DisplayShapeEffectKind::ScopeEvent{generation:g,..} if g==generation)
    ));
    assert_eq!(
        cursor.advance(&d, &mut resolver, &mut one()),
        Err(LayoutError::StaleContinuation)
    );
}

#[test]
fn typed_objects_drive_outer_empty_alt_policy_without_recursive_fallbacks() {
    for (source, shown) in [
        ("![](u)", "[image]"),
        ("![![](u)](v)", "[image]"),
        ("![a ![](u)](v)", "a "),
        ("[![](u)](v)", "[image]"),
        ("![][r]", "![][r]"),
        ("![![][r]](v)", "![][r]"),
        ("[![][r]](v)", "![][r]"),
        ("![[x](u)](v)", "x"),
        ("[a [![](u)](v)](w)", "[a [image]](w)"),
    ] {
        for split in source
            .char_indices()
            .map(|(at, _)| at)
            .chain([source.len()])
        {
            let mut d = Document::new();
            d.enable_display_shape_effects().unwrap();
            let mut m = Model::new();
            for chunk in [&source[..split], &source[split..]] {
                let u = d.append(chunk).unwrap();
                m.apply(&d, &u);
                m.check(&d);
            }
            let u = d.finish();
            m.apply(&d, &u);
            m.check(&d);
            assert_eq!(
                m.presented(&d),
                shown,
                "{source} split={split} actual={:?} links={:?}",
                m.text(&d, Span::new(0, m.end), &mut d.link_resolver()),
                m.links
            );
        }
    }
    let mut d = Document::new();
    d.enable_display_shape_effects().unwrap();
    let mut m = Model::new();
    let u = d.append("![![][r]](v)\n\n").unwrap();
    m.apply(&d, &u);
    assert_eq!(m.presented(&d), "![][r]\n\n");
    let u = d.append("[r]: /u\n").unwrap();
    m.apply(&d, &u);
    assert_eq!(m.presented(&d), "[image]\n\n");
}

#[test]
fn pure_append_extents_do_not_invalidate_classified_scope_generation() {
    let mut d = Document::new();
    d.enable_display_shape_effects().unwrap();
    d.append("a").unwrap();
    let before = d.display_shape_stats();
    let u = d.append(&"\u{301}".repeat(4096)).unwrap();
    assert_eq!(d.display_shape_stats().binding_epoch, before.binding_epoch);
    assert!(
        u.display_shape
            .events()
            .iter()
            .all(|e| !matches!(e.kind, DisplayShapeEffectKind::ScopeEvent { .. }))
    );
    assert!(
        u.display_shape
            .events()
            .iter()
            .any(|e| matches!(e.kind, DisplayShapeEffectKind::ScopeExtent { .. }))
    );
}

#[test]
fn producer_events_and_semantic_visits_grow_with_input_not_covered_bytes() {
    for n in [1024, 4096, 16384] {
        let mut d = Document::new();
        d.enable_display_shape_effects().unwrap();
        let mut m = Model::new();
        for chunk in [
            "[prior\n".to_owned(),
            format!("end](u) &amp; a|{}\n", "界\u{301}".repeat(n)),
            "---|---\n".to_owned(),
        ] {
            let u = d.append(&chunk).unwrap();
            m.apply(&d, &u);
        }
        let u = d.finish();
        m.apply(&d, &u);
        m.check(&d);
        let stats = d.display_shape_stats();
        assert_eq!(m.decoded, d.source().len());
        assert!(stats.emitted < d.source().len() * 3);
        assert!(m.visits < stats.emitted * usize::BITS as usize * 8);
        assert!(m.nodes.len() < d.source().len() * 8);
        eprintln!(
            "bytes={} scalars={} effects={} visits={} nodes={} undo_peak={} producer_capacity={} raw_capacity={} node_capacity={}",
            d.source().len(),
            m.raw.len(),
            stats.emitted,
            m.visits,
            m.nodes.len(),
            m.undo_peak,
            stats.producer_capacity_bytes,
            m.raw.capacity() * std::mem::size_of::<(usize, char, usize)>(),
            m.nodes.capacity() * std::mem::size_of::<Node>()
        );
    }
}

#[test]
fn utf8_all_byte_cuts_and_eof_use_real_producer_not_raw_markdown_reparse() {
    for source in [
        "🇦*🇧 [x*][r]🇨\n\n[r]: /u\n",
        "![![][r]](v)\n\n[r]: /u\n",
        "> - a &amp; ` 界\n🙂 ` $q$ [^n]\n\n[^n]: note\n",
        "-  - a|b\n\t中🙂 &amp;\n",
        "> - ` x\n> \ty `\n",
    ] {
        for cut in 0..=source.len() {
            let mut d = Utf8Document::new();
            d.enable_display_shape_effects().unwrap();
            let mut m = Model::new();
            for chunk in [&source.as_bytes()[..cut], &source.as_bytes()[cut..]] {
                let u = d.append(chunk).unwrap();
                m.apply(d.document(), &u);
                m.check(d.document());
            }
            let u = d.finish();
            m.apply(d.document(), &u);
            m.check(d.document());
            assert_eq!(m.decoded, source.len());
        }
    }
}

#[test]
fn binding_epoch_rejects_late_footnote_and_eof_presentation_cache() {
    let mut d = Document::new();
    d.enable_display_shape_effects().unwrap();
    let mut m = Model::new();
    let u = d.append("a[^n]\n\n").unwrap();
    m.apply(&d, &u);
    let symbol = d.footnote_references()[0].symbol;
    let cached = (
        d.display_shape_stats().binding_epoch,
        d.footnote(symbol).unwrap().definition.is_some(),
        d.is_finished(),
    );
    assert!(!cached.1 && !cached.2);
    let u = d.append("plain\n\n").unwrap();
    m.apply(&d, &u);
    assert_eq!(cached.0, d.display_shape_stats().binding_epoch);
    let u = d.append("[^n]: note\n").unwrap();
    m.apply(&d, &u);
    assert!(
        u.display_shape
            .events()
            .iter()
            .any(|e| matches!(e.kind,DisplayShapeEffectKind::Footnote{id} if id==symbol))
    );
    assert!(d.display_shape_stats().binding_epoch > cached.0);
    assert!(d.footnote(symbol).unwrap().definition.is_some());
    let before = d.display_shape_stats().binding_epoch;
    let u = d.finish();
    m.apply(&d, &u);
    assert!(d.display_shape_stats().binding_epoch > before);
    assert!(
        u.display_shape
            .events()
            .iter()
            .any(|e| e.kind == DisplayShapeEffectKind::Finish)
    );
    assert!(d.is_finished());
    // Rollback restores first-use/numbering without withdrawing raw input.
    let mut d = Document::new();
    d.enable_display_shape_effects().unwrap();
    let mut m = Model::new();
    for chunk in [
        "*prior\n".to_owned(),
        format!("end*x* [^n] {}", "z".repeat(1100)),
        "|b\n---|---\n".to_owned(),
    ] {
        let before = d.display_shape_stats().binding_epoch;
        let u = d.append(&chunk).unwrap();
        m.apply(&d, &u);
        if u.display_shape
            .events()
            .iter()
            .any(|e| matches!(e.kind, DisplayShapeEffectKind::Rollback(_)))
        {
            assert!(d.display_shape_stats().binding_epoch > before);
            assert!(u.display_shape.events().iter().any(|e| matches!(
                e.kind,
                DisplayShapeEffectKind::FootnoteReference { state: None, .. }
            )));
        }
    }
    let u = d.finish();
    m.apply(&d, &u);
    m.check(&d);
}

#[test]
fn delimiter_effects_are_classified_and_object_envelopes_update_across_appends() {
    let mut d = Document::new();
    d.enable_display_shape_effects().unwrap();
    let mut m = Model::new();
    let mut seen = Vec::new();
    for s in ["*a* _b_ ~c~ **d** [![!", "[](u)](v)](w)", "\n\n"] {
        let u = d.append(s).unwrap();
        for e in u.display_shape.events() {
            if let DisplayShapeEffectKind::ScopeEvent {
                event: DisplayShapeStyleEvent::Run { kind, source, .. },
                ..
            } = e.kind
            {
                seen.push(kind);
                assert!(d.text(source).unwrap().chars().all(|c| c
                    == match kind {
                        DisplayDelimiterKind::Asterisk => '*',
                        DisplayDelimiterKind::Underscore => '_',
                        DisplayDelimiterKind::Tilde => '~',
                    }));
            }
        }
        m.apply(&d, &u);
        m.check(&d);
    }
    let u = d.finish();
    m.apply(&d, &u);
    m.check(&d);
    for kind in [
        DisplayDelimiterKind::Asterisk,
        DisplayDelimiterKind::Underscore,
        DisplayDelimiterKind::Tilde,
    ] {
        assert!(seen.contains(&kind));
    }
    assert_eq!(m.presented(&d), "a b c d [image]\n\n");
}
