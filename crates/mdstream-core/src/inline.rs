use crate::annotations::Annotations;
use crate::destination::{Destination, Progress};
use crate::references::{MAX_LABEL_BYTES, References, normalize_label};
#[path = "atom_fallback.rs"]
mod atom_fallback;
#[path = "bare_autolink.rs"]
mod bare_autolink;
#[path = "code_fallback.rs"]
mod code_fallback;
#[path = "flanking.rs"]
mod flanking;
#[path = "label_journal.rs"]
mod label_journal;
use crate::link_index::Children;
use crate::link_semantics::{Gate, Guard};
use crate::{
    ChangeKind, LinkId, LinkInfo, LinkKind, LinkTarget, Span, Style, SyntaxState, Update, Work,
};
use atom_fallback::AtomTrace;
use code_fallback::{Checkpoint as CodeCheckpoint, CodeFallback, Event as CodeEvent};
use label_journal::List as LabelList;

pub(crate) struct Publishing<'a> {
    pub styles: &'a mut crate::inline_styles::Styles,
    pub annotations: &'a mut Annotations,
    pub display: &'a mut crate::display::DisplayIndex,
    pub references: &'a mut References,
    pub links: &'a mut Vec<LinkTarget>,
    pub link_info: &'a mut Vec<LinkInfo>,
    pub link_order: &'a mut crate::link_index::LinkIndex,
    pub link_guards: &'a mut crate::link_semantics::Guards,
    pub direct_title_gaps: &'a mut Vec<Vec<Span>>,
    pub maths: &'a mut Vec<crate::Math>,
    pub math_active: &'a mut Vec<bool>,
    pub math_states: &'a mut Vec<SyntaxState>,
    pub math_index: &'a mut std::collections::BTreeMap<usize, crate::MathId>,
    pub line: &'a mut crate::header_effects::LineEffects,
    pub math_gaps: &'a mut Vec<Span>,
    pub footnotes: &'a mut crate::footnotes::Footnotes,
    pub update: &'a mut Update,
}

impl Publishing<'_> {
    pub fn mark(&mut self, span: Span, style: Style, link: Option<LinkId>) {
        self.annotate(span, style, link, None);
    }

    fn annotate(&mut self, span: Span, style: Style, link: Option<LinkId>, gate: Option<LinkId>) {
        if span.is_empty() {
            return;
        }
        self.update.change(
            span,
            if style.contains(Style::HIDDEN) || gate.is_some() {
                ChangeKind::Visibility
            } else if style != Style::default() {
                ChangeKind::Style
            } else {
                ChangeKind::Structure
            },
        );
        if let Some(id) = gate {
            self.annotations
                .hide_when_resolved(span, id, self.update.revision);
        } else if let Some(id) = link {
            self.annotations
                .apply_object(span, style, id, self.update.revision);
        } else {
            self.annotations
                .apply(span, style, None, self.update.revision);
        }
        // Coalesce adjacent/overlapping mutations without scanning prior changes.
        if let Some(last) = self.update.dirty.last_mut()
            && span.start <= last.end
            && last.start <= span.end
        {
            last.start = last.start.min(span.start);
            last.end = last.end.max(span.end);
        } else {
            self.update.dirty.push(span);
        }
    }

    fn delimiter(&mut self, span: Span, style: Style) {
        if span.is_empty() {
            return;
        }
        self.annotations
            .delimiter(span, style, self.update.revision);
        self.update.change(
            span,
            if style == Style::HIDDEN {
                ChangeKind::Visibility
            } else {
                ChangeKind::Style
            },
        );
        if let Some(last) = self.update.dirty.last_mut()
            && span.start <= last.end
            && last.start <= span.end
        {
            last.start = last.start.min(span.start);
            last.end = last.end.max(span.end);
        } else {
            self.update.dirty.push(span);
        }
    }

    fn hide(&mut self, span: Span) {
        self.mark(span, Style::HIDDEN, None);
    }

    fn marker(&mut self, span: Span, gate: Option<LinkId>) {
        if let Some(id) = gate {
            self.annotate(span, Style::default(), None, Some(id));
        } else {
            self.hide(span);
        }
    }

    fn candidate_marker(
        &mut self,
        candidate: Candidate,
        span: Span,
        gate: Option<LinkId>,
        id: LinkId,
    ) {
        if let Some(parent) = candidate.alternative_parent {
            let end = span.end.min(self.link_info[parent.0].source.end);
            if span.start < end {
                let overlap = Span::new(span.start, end);
                self.update.change(overlap, ChangeKind::Visibility);
                self.annotations
                    .hide_when_either(overlap, parent, id, self.update.revision);
                self.update.dirty.push(overlap);
            }
            if end < span.end {
                self.marker(Span::new(end.max(span.start), span.end), gate);
            }
        } else {
            self.marker(span, gate);
        }
    }
    fn complete_link(
        &mut self,
        candidate: Candidate,
        target: LinkTarget,
        suffix: Option<Span>,
        title: Option<Span>,
        title_gaps: Vec<Span>,
        work: &mut Work,
    ) -> LinkId {
        debug_assert_eq!(self.links.len(), self.link_info.len());
        debug_assert_eq!(self.links.len(), self.direct_title_gaps.len());
        debug_assert!(
            title_gaps.is_empty() || (matches!(target, LinkTarget::Direct(_)) && title.is_some())
        );
        let info = LinkInfo {
            kind: if candidate.image.is_some() {
                LinkKind::Image
            } else {
                LinkKind::Link
            },
            label: Span::new(candidate.open + 1, candidate.close),
            source: Span::new(
                candidate.image.unwrap_or(candidate.open),
                suffix.map_or(candidate.close + 1, |s| s.end),
            ),
            title,
            state: if suffix.is_some() {
                SyntaxState::Sealed
            } else {
                SyntaxState::Provisional
            },
        };
        if let LinkTarget::Reference(symbol) = target {
            let first = &mut self.references.entries[symbol.0].first_use;
            *first = Some(first.map_or(info.source.start, |old| old.min(info.source.start)));
        }
        // Only a previously published shortcut supplies the visible-label witness.
        // Do not infer syntax ownership from the label annotation.
        let qualified = candidate.link.is_some();
        let reuse = candidate.link.or_else(|| self.line.reuse_link(info));
        let id = match reuse {
            Some(id) => {
                self.links[id.0] = target;
                self.link_info[id.0] = info;
                self.direct_title_gaps[id.0] = title_gaps;
                id
            }
            None => {
                let id = LinkId(self.links.len());
                self.links.push(target);
                self.link_info.push(info);
                self.direct_title_gaps.push(title_gaps);
                id
            }
        };
        self.link_info[id.0].state = self.line.state(id, info.state);
        self.update.changed_links.start = self.update.changed_links.start.min(id.0);
        self.link_order.insert(info.source, id);
        let selected = candidate.gate.unwrap_or_else(|| {
            if candidate.image.is_some() {
                Gate::default()
            } else {
                Gate::unless(candidate.blockers.root)
            }
        });
        let nested = candidate.nested || candidate.children.head.is_some();
        self.link_order.set_gate(id, selected, nested);
        let fixed = if matches!(target, LinkTarget::Reference(_)) {
            None
        } else {
            match selected.root {
                Guard::Never => Some(!selected.positive),
                Guard::Always => Some(selected.positive),
                Guard::Node(_) => None,
            }
        };
        if let Some(changed) = self.styles.bind(candidate.open, id, fixed, work) {
            self.mark(changed, Style::default(), None);
        }
        if candidate.link.is_none() {
            self.link_order.adopt(candidate.children, id, self.update);
        }
        if nested || !selected.unconditional() {
            self.link_guards.use_at(info.source.start);
        }
        self.mark(
            Span::new(candidate.open + 1, candidate.close),
            Style::default(),
            Some(id),
        );
        let gate = if !selected.unconditional() {
            Some(id)
        } else {
            match (target, suffix) {
                (LinkTarget::Reference(_), None) => Some(id),
                (LinkTarget::Reference(_), Some(_)) if qualified => Some(id),
                _ => None,
            }
        };
        self.candidate_marker(
            candidate,
            Span::new(candidate.open, candidate.open + 1),
            gate,
            id,
        );
        self.candidate_marker(
            candidate,
            Span::new(candidate.close, candidate.close + 1),
            gate,
            id,
        );
        if let Some(image) = candidate.image {
            self.candidate_marker(candidate, Span::new(image, image + 1), gate, id);
        }
        if let Some(suffix) = suffix {
            self.candidate_marker(candidate, suffix, gate, id);
        }
        id
    }
}

// The observer capability contains only source and local counters. All public
// storage writes are confined to Publishing and these finite effects exits.
pub(crate) struct Effects<'a> {
    pub source: &'a str,
    pub work: &'a mut Work,
    publishing: Option<Publishing<'a>>,
    style_sink: Option<(&'a mut crate::inline_styles::Styles, usize, Option<LinkId>)>,
}

#[derive(Clone, Copy, Debug)]
enum AcceptedReference {
    Published(crate::SymbolId),
    Observed,
}

impl AcceptedReference {
    fn target(self) -> Option<LinkTarget> {
        match self {
            Self::Published(symbol) => Some(LinkTarget::Reference(symbol)),
            Self::Observed => None,
        }
    }
}

impl<'a> Effects<'a> {
    pub fn publishing(source: &'a str, work: &'a mut Work, publishing: Publishing<'a>) -> Self {
        Self {
            source,
            work,
            publishing: Some(publishing),
            style_sink: None,
        }
    }

    fn observing(source: &'a str, work: &'a mut Work) -> Self {
        Self {
            source,
            work,
            publishing: None,
            style_sink: None,
        }
    }

    fn style_touch(&mut self, key: usize, span: Span) {
        if let Some(p) = &mut self.publishing {
            p.styles.touch(key, span, self.work);
        }
    }
    fn style_enter(&mut self, key: usize, open: usize) {
        if let Some(p) = &mut self.publishing {
            p.styles.enter(key, open, self.work);
        } else if let Some((styles, owner, _)) = &mut self.style_sink {
            styles.enter(*owner, open, self.work);
        }
    }
    fn style_leave(&mut self, key: usize, open: usize, end: usize) {
        if let Some(p) = &mut self.publishing {
            p.styles.leave(key, open, end, self.work);
        } else if let Some((styles, owner, _)) = &mut self.style_sink {
            styles.leave(*owner, open, end, self.work);
        }
    }
    fn style_run(&mut self, key: usize, run: crate::inline_styles::DelimiterRun) {
        if let Some(p) = &mut self.publishing {
            if let Some(changed) = p.styles.run(key, run, None, self.work) {
                p.mark(changed, Style::default(), None);
            }
        } else if let Some((styles, owner, unless)) = &mut self.style_sink {
            styles.run(*owner, run, *unless, self.work);
        }
    }

    pub fn mark(&mut self, span: Span, style: Style, link: Option<LinkId>) {
        if let Some(p) = &mut self.publishing {
            p.mark(span, style, link);
        }
    }

    fn hide(&mut self, span: Span) {
        self.mark(span, Style::HIDDEN, None);
    }

    fn reference(&mut self, label: &str) -> Option<AcceptedReference> {
        match &mut self.publishing {
            Some(p) => p
                .references
                .intern(label, self.work)
                .map(AcceptedReference::Published),
            None => normalize_label(label, self.work).map(|_| AcceptedReference::Observed),
        }
    }

    fn complete_link(
        &mut self,
        candidate: Candidate,
        target: Option<LinkTarget>,
        suffix: Option<Span>,
        title: Option<Span>,
        title_gaps: Vec<Span>,
    ) -> Option<LinkId> {
        self.publishing.as_mut().map(|p| {
            p.complete_link(
                candidate,
                target.expect("published target"),
                suffix,
                title,
                title_gaps,
                self.work,
            )
        })
    }

    fn seal_link(&mut self, id: Option<LinkId>) {
        if let Some(p) = &mut self.publishing
            && let Some(id) = id
        {
            p.link_info[id.0].state = p.line.state(id, SyntaxState::Sealed);
        }
    }

    fn code(&mut self, pair: &CodePair) {
        if let Some(p) = &mut self.publishing {
            p.styles.fixed_pair(
                crate::StylePairId {
                    open: pair.code.open,
                    close: pair.close,
                    kind: crate::InlineStyleKind::Code,
                },
                self.work,
            );
            let content = Span::new(pair.code.open.end, pair.close.start);
            p.display.code(
                content,
                self.source,
                pair.code.all_spaces,
                pair.code.last,
                pair.code.last_space,
            );
            p.update.change(content, ChangeKind::Text);
        }
    }

    fn math(&mut self, pair: &MathPair) {
        if let Some(p) = &mut self.publishing {
            let source = Span::new(pair.open.start, pair.close.end);
            let math = crate::Math {
                kind: if pair.open.len() == 1 {
                    crate::MathKind::Inline
                } else {
                    crate::MathKind::Display
                },
                source,
                content: Span::new(pair.open.end, pair.close.start),
            };
            let id = if let Some(id) = p.line.reuse_math(math) {
                assert_eq!(p.maths[id.0], math);
                p.math_active[id.0] = true;
                id
            } else {
                let id = crate::MathId(p.maths.len());
                p.maths.push(math);
                p.math_active.push(true);
                p.math_states.push(SyntaxState::Provisional);
                id
            };
            p.math_states[id.0] = if p.line.pending {
                SyntaxState::Provisional
            } else {
                SyntaxState::Sealed
            };
            p.math_index.insert(source.start, id);
            p.update.change(source, ChangeKind::Structure);
        }
    }

    fn math_gap(&mut self, span: Span) {
        if let Some(p) = &mut self.publishing {
            debug_assert!(p.math_gaps.last().is_none_or(|g| g.end <= span.start));
            p.math_gaps.push(span);
        }
    }

    fn autolink(
        &mut self,
        start: usize,
        end: usize,
        target: crate::autolink::Target,
    ) -> Option<LinkId> {
        let label = Span::new(start + 1, end);
        let target = match target {
            crate::autolink::Target::Uri => LinkTarget::Direct(label),
            crate::autolink::Target::Email => LinkTarget::Email(label),
        };
        let info = LinkInfo {
            kind: LinkKind::Link,
            label,
            source: Span::new(start, end + 1),
            title: None,
            state: SyntaxState::Sealed,
        };
        let id = self.publish_plain_link(info, target);
        if let Some(p) = &mut self.publishing {
            p.mark(label, Style::default(), id);
            p.hide(Span::new(start, start + 1));
            p.hide(Span::new(end, end + 1));
        }
        id
    }
    fn bare_autolink(&mut self, label: Span, www: bool, email: bool) {
        let target = if www {
            LinkTarget::Www(label)
        } else if email {
            LinkTarget::Email(label)
        } else {
            LinkTarget::Direct(label)
        };
        let info = LinkInfo {
            kind: LinkKind::Link,
            label,
            source: label,
            title: None,
            state: SyntaxState::Sealed,
        };
        let id = self.publish_plain_link(info, target);
        if let Some(p) = &mut self.publishing {
            p.mark(label, Style::default(), id);
        }
    }
    fn publish_plain_link(&mut self, info: LinkInfo, target: LinkTarget) -> Option<LinkId> {
        let p = self.publishing.as_mut()?;
        let id = if let Some(id) = p.line.reuse_link(info) {
            p.links[id.0] = target;
            p.link_info[id.0] = info;
            p.direct_title_gaps[id.0].clear();
            id
        } else {
            let id = LinkId(p.links.len());
            p.links.push(target);
            p.link_info.push(info);
            p.direct_title_gaps.push(Vec::new());
            id
        };
        p.link_info[id.0].state = p.line.state(id, info.state);
        p.update.changed_links.start = p.update.changed_links.start.min(id.0);
        p.link_order.insert(info.source, id);
        Some(id)
    }

    fn entity(&mut self, span: Span, text: String) {
        if let Some(p) = &mut self.publishing {
            p.display.entity(span, text);
            p.mark(span, Style::default(), None);
            p.update.change(span, ChangeKind::Text);
        }
    }

    fn join(&mut self, a: Guard, b: Guard) -> Guard {
        self.publishing
            .as_mut()
            .map_or(Guard::Never, |p| p.link_guards.or(a, b))
    }
    fn target_guard(&mut self, id: Option<LinkId>) -> Guard {
        match (&mut self.publishing, id) {
            (Some(p), Some(id)) => p.link_guards.target(id),
            _ => Guard::Never,
        }
    }
    fn attach(&mut self, destination: &mut Children, source: Children) {
        if let Some(p) = &mut self.publishing {
            p.link_order.append(destination, source);
        }
    }
    fn footnote(&mut self, start: usize, end: usize) {
        let label = &self.source[start + 2..end];
        if let Some(p) = &mut self.publishing {
            let symbol = p.footnotes.intern(label, self.work);
            let source = Span::new(start, end + 1);
            p.footnotes.reference(symbol, source, p.update, self.work);
            p.mark(source, Style::default(), None);
        } else {
            // Same bounded accepted label work, without a symbol table or ID.
            self.work.footnote_steps += 2;
            self.work.footnote_label_bytes += label.len() as u64;
        }
    }

    pub fn row_changed(&mut self, table: crate::TableId, row: usize) {
        if let Some(p) = &mut self.publishing {
            p.update.row_changed(table, row);
        }
    }

    pub fn table_width(&mut self, table: crate::TableId, source: Span) {
        if let Some(p) = &mut self.publishing {
            p.annotations
                .apply(source, Style::default(), None, p.update.revision);
            p.update.dirty.push(source);
            p.update.change(source, ChangeKind::TableWidth);
            if p.update.tables_changed.last() != Some(&table) {
                p.update.tables_changed.push(table);
            }
        }
    }
}

#[derive(Clone, Copy)]
#[cfg_attr(test, derive(Debug))]
struct Delimiter {
    email: Option<usize>,
    // Stars/underscores: marker * 6 + original can-close * 3 + original len % 3.
    // Original run properties survive partial consumption (rule of three).
    // Tildes use the final two, exact-length categories.
    kind: usize,
    source: Span,
    previous: Option<usize>,
}

#[cfg_attr(test, derive(Debug))]
struct EmphasisPair {
    open: Span,
    close: Span,
    style: Style,
}

impl EmphasisPair {
    fn commit(self, effects: &mut Effects<'_>) {
        if let Some(p) = &mut effects.publishing {
            let id = crate::StylePairId {
                open: self.open,
                close: self.close,
                kind: if self.style == Style::EMPHASIS {
                    crate::InlineStyleKind::Emphasis
                } else if self.style == Style::STRONG {
                    crate::InlineStyleKind::Strong
                } else {
                    crate::InlineStyleKind::Strike
                },
            };
            p.styles.pair(id, effects.work);
            for (span, style) in [
                (Span::new(self.open.end, self.close.start), self.style),
                (self.open, Style::HIDDEN),
                (self.close, Style::HIDDEN),
            ] {
                p.delimiter(span, style);
            }
        }
    }
}

#[derive(Clone, Copy)]
#[cfg_attr(test, derive(Debug))]
struct PendingEmail {
    candidate: Option<bare_autolink::Candidate>,
    openers: usize,
}

#[derive(Clone, Copy)]
#[cfg_attr(test, derive(Debug))]
struct Run {
    byte: u8,
    source: Span,
    before: Option<char>,
    label_len: usize,
    label_overflow: bool,
}

// Source/flanking summaries only. An unresolved label must not consume outer
// openers before its publication is known; no body bytes or stack are copied.
#[cfg_attr(test, derive(Debug))]
#[derive(Clone, Copy)]
struct LabelDelimiter {
    run: Run,
    next: Option<char>,
}

#[derive(Clone, Copy)]
#[cfg_attr(test, derive(Debug))]
struct Code {
    open: Span,
    all_spaces: bool,
    // Last payload byte (or CRLF); container prefixes are never payload.
    last: Span,
    // Semantic whitespace can be a virtual column owned by a raw tab byte.
    last_space: bool,
}

impl Code {
    fn new(open: Span) -> Self {
        Self {
            open,
            all_spaces: true,
            last: Span::new(open.end, open.end),
            last_space: false,
        }
    }

    fn payload(&mut self, byte: u8, pos: usize, previous: Option<u8>) {
        self.all_spaces &= crate::display::code_space(byte);
        self.last_space = crate::display::code_space(byte);
        self.last = Span::new(
            pos - usize::from(byte == b'\n' && previous == Some(b'\r')),
            pos + 1,
        );
    }
}

#[cfg_attr(test, derive(Debug))]
struct CodePair {
    code: Code,
    close: Span,
}

impl CodePair {
    fn commit(self, effects: &mut Effects<'_>) {
        let content = Span::new(self.code.open.end, self.close.start);
        effects.code(&self);
        effects.mark(content, Style::CODE, None);
        effects.hide(self.code.open);
        effects.hide(self.close);
    }
}

#[derive(Clone, Copy)]
#[cfg_attr(test, derive(Debug))]
struct MathCandidate {
    open: Span,
    escaped: bool,
    // End of the last feed; same-scope container prefixes are not fed.
    end: usize,
}

#[cfg_attr(test, derive(Debug))]
struct MathPair {
    open: Span,
    close: Span,
}

impl MathPair {
    fn commit(self, effects: &mut Effects<'_>) {
        let content = Span::new(self.open.end, self.close.start);
        // The parser never decodes, copies or revisits the body.
        effects.mark(content, Style::MATH, None);
        effects.hide(self.open);
        effects.hide(self.close);
        effects.math(&self);
    }
}

#[derive(Clone, Copy, Default)]
struct Blockers {
    known: bool,
    potential: bool,
    root: Guard,
}
impl std::fmt::Debug for Blockers {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Blockers")
            .field("known", &self.known)
            .field("potential", &self.potential)
            .finish()
    }
}
impl Blockers {
    fn merge(&mut self, other: Self, effects: &mut Effects<'_>) {
        self.known |= other.known;
        self.potential |= other.potential;
        self.root = if self.known {
            Guard::Always
        } else {
            effects.join(self.root, other.root)
        };
    }
}
#[derive(Clone, Copy)]
struct Candidate {
    open: usize,
    close: usize,
    link: Option<LinkId>,
    reference: Option<AcceptedReference>,
    image: Option<usize>,
    journal: LabelList,
    children: Children,
    blockers: Blockers,
    nested: bool,
    accepted: bool,
    alternate: bool,
    alternative_parent: Option<LinkId>,
    gate: Option<Gate>,
}
impl Candidate {
    fn bare(open: usize, close: usize, image: Option<usize>) -> Self {
        Self {
            open,
            close,
            image,
            link: None,
            reference: None,
            journal: LabelList::default(),
            children: Children::default(),
            blockers: Blockers::default(),
            nested: false,
            accepted: false,
            alternate: false,
            alternative_parent: None,
            gate: None,
        }
    }
}
#[derive(Clone, Copy)]
struct Bracket {
    open: usize,
    image: Option<usize>,
    journal: LabelList,
    children: Children,
    blockers: Blockers,
    nested: bool,
}
impl Bracket {
    fn new(open: usize, image: Option<usize>) -> Self {
        Self {
            open,
            image,
            journal: Default::default(),
            children: Default::default(),
            blockers: Default::default(),
            nested: false,
        }
    }
}
#[cfg(test)]
impl std::fmt::Debug for Bracket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Bracket")
            .field("open", &self.open)
            .field("image", &self.image)
            .field("journal", &self.journal)
            .field("blockers", &self.blockers)
            .field("nested", &self.nested)
            .finish()
    }
}

#[derive(Clone, Copy)]
#[cfg_attr(test, derive(Debug))]
pub(crate) struct LineBreak {
    newline: Span,
    hidden: Span,
    hard: bool,
}

impl LineBreak {
    pub fn commit(self, effects: &mut Effects<'_>) {
        effects.mark(
            self.newline,
            if self.hard {
                Style::HARD_BREAK
            } else {
                Style::SOFT_BREAK
            },
            None,
        );
        effects.hide(self.hidden);
    }
}

#[derive(Clone, Copy)]
enum Origin {
    Raw,
    Virtual,
}

enum Tail {
    After(Candidate),
    Destination {
        candidate: Candidate,
        parser: Destination,
        title_gaps: Vec<Span>,
        atoms: Option<AtomTrace>,
    },
    Label {
        candidate: Candidate,
        start: usize,
        escaped: bool,
    },
}

// Compare syntax in tests without conflating acceptance with published IDs.
#[cfg(test)]
impl std::fmt::Debug for Candidate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Candidate")
            .field("open", &self.open)
            .field("close", &self.close)
            .field("accepted_reference", &self.reference.is_some())
            .field("image", &self.image)
            .finish()
    }
}

#[cfg(test)]
impl std::fmt::Debug for Tail {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::After(candidate) => f.debug_tuple("After").field(candidate).finish(),
            Self::Destination {
                candidate,
                parser,
                title_gaps,
                ..
            } => {
                // Destination has no effects-dependent state. Future byte events
                // exercise its pending/invalid/complete branches in both cores.
                f.debug_tuple("Destination")
                    .field(candidate)
                    .field(parser)
                    .field(title_gaps)
                    .finish()
            }
            Self::Label {
                candidate,
                start,
                escaped,
            } => f
                .debug_tuple("Label")
                .field(candidate)
                .field(start)
                .field(escaped)
                .finish(),
        }
    }
}

// Only syntax-free suffix payloads can share the already consumed cell prefix.
// Keep spans, not copied targets or prematurely interned reference symbols.
#[cfg_attr(test, derive(Debug))]
enum HeaderLinkSuffix {
    Destination {
        source: Span,
        destination: Span,
        title: Option<Span>,
        title_gaps: Vec<Span>,
    },
    Label(Span),
}

/// Single-pass byte scanner. Each emphasis delimiter is pushed/popped once.
/// A failed primary code candidate releases its logical payload once, using
/// indexed future closers to prevent recursively rescanning failed suffixes.
#[derive(Default)]
#[cfg_attr(test, derive(Debug))]
pub(crate) struct Inline {
    header_saved: Option<LineCheckpoint>,
    style_scope: Option<usize>,
    root_paragraph: bool,
    container_paragraph: bool,
    paragraph_key: Option<crate::table::ParagraphKey>,
    // Sticky until paragraph closure, even after the suffix completes/fails.
    root_separator_seen: bool,
    // Non-space/tab endpoints of the raw sequence, excluding its final LF/CRLF.
    // Updated in the existing byte scan, including opaque syntax. Setext can
    // trim the outer edges without replaying text or visiting physical leaves.
    raw_content: Option<Span>,
    raw_cr: Option<usize>,
    stack: crate::rewind::Vector<Delimiter>,
    last: [Option<usize>; 14],
    run: Option<Run>,
    code: Option<Code>,
    code_fallback: CodeFallback,
    // Present only during one synchronous failure release; no nested journal.
    code_recovery: Option<Vec<Span>>,
    math: Option<MathCandidate>,
    footnote: Option<usize>,
    bracket: Option<Bracket>,
    brackets: crate::rewind::Vector<Bracket>,
    bracket_allocations: u64,
    // Raw payload only, without container prefixes. One extra byte lets a
    // closing ']' complete an exactly MAX_LABEL_BYTES reference label. Direct
    // labels remain unbounded source envelopes; never keep their whole text.
    label: crate::rewind::Vector<u8>,
    label_overflow: bool,
    label_delimiters: label_journal::Arena<LabelDelimiter>,
    // Prefix from the preceding line, resolved as prose on header success.
    header_label_delimiters: usize,
    header_label_tail: Option<std::num::NonZeroUsize>,
    bang: Option<usize>,
    tail: Option<Tail>,
    escape: Option<usize>,
    previous: Option<u8>,
    // Last semantic scalar, including virtual spaces. Decode once at its
    // leading byte; continuation bytes and omitted container prefixes do not
    // overwrite it. No lookbehind through source or earlier feed spans.
    previous_scalar: Option<char>,
    spaces: Option<usize>,
    // Entire raw space/tab suffix to hide on a confirmed ordinary break.
    // `spaces` separately tracks only the immediate ASCII-space hard marker.
    whitespace: Option<usize>,
    carriage: bool,
    line_break: Option<LineBreak>,
    autolink: Option<usize>,
    bare: Option<bare_autolink::Candidate>,
    bare_replay: bool,
    bare_recovery: Option<bare_autolink::Recovery>,
    bare_collect: Option<Span>,
    bare_markers: crate::rewind::Map<usize, usize>,
    email_pending: crate::rewind::Vector<PendingEmail>,
    email_waiting: crate::rewind::Queue<usize>,
    email_ready: crate::rewind::Queue<usize>,
    header_email_pending: usize,
    entity: Option<usize>,
    // Inherited openers can close without affecting the independent cell's
    // delimiter machine when the closer cannot open and pops no local opener.
    // Isolate only these pairs until the header succeeds or fails. Each pair
    // consumes an old opener, so this storage/work is amortized over input.
    header_pairs: Vec<EmphasisPair>,
    header_floor: usize,
    // Other crossings still cancel, even if the stack subsequently regrows.
    header_crossed: bool,
    // Labels are not opaque: preserve local effects, but never reuse a label
    // that started on a preceding line as part of the first table cell.
    header_bracket: Option<usize>,
    // Inherited code or math keeps this line opaque until table confirmation.
    // Its prefix has no published effects and needs a fresh cell parser then.
    header_opaque: bool,
    // At most one inherited code closure can wait. Only inert suffix bytes
    // are allowed until the pipe: no later display entry/object may precede
    // this mapping's commit, nor survive a fresh independent-cell parse.
    header_code: Option<CodePair>,
    // As with code, isolate at most one inherited math closure. Do not allocate
    // a MathId until failure: success reparses the independent current row.
    header_math: Option<MathPair>,
    // A closed inherited label keeps using the bounded buffer. Delay its LinkId
    // and symbol/first_use effects, including inert explicit suffixes. New inline
    // syntax cancels before it can overwrite the label or emit later links.
    header_link: Option<Candidate>,
    header_link_suffix: Option<HeaderLinkSuffix>,
}

struct LineCheckpoint(Box<Inline>);
impl std::fmt::Debug for LineCheckpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("LineCheckpoint")
    }
}

// One fresh core for the active ordinary paragraph. No document, tree,
// reference store, published IDs or copied old parser state is retained here.
#[derive(Default)]
pub(crate) struct Eligibility {
    // Keep Document moves/drop frames bounded as the grammar checkpoint grows.
    inline: Box<Inline>,
    cursor: Option<usize>,
    last_virtual_tab: Option<usize>,
    container: bool,
    key: Option<crate::table::ParagraphKey>,
    work: Work,
}

pub(crate) type RootEligibility = Eligibility;

impl Eligibility {
    pub(crate) fn undo_stats(&self) -> crate::rewind::UndoStats {
        self.inline.undo_stats()
    }

    pub fn container() -> Self {
        Self {
            container: true,
            ..Self::default()
        }
    }

    pub fn bind_key(&mut self, key: crate::table::ParagraphKey) {
        assert!(self.key.is_none_or(|old| old == key));
        self.key = Some(key);
    }

    pub fn key(&self) -> Option<crate::table::ParagraphKey> {
        self.key
    }

    pub fn can_isolate_line(&self) -> bool {
        self.inline.can_isolate_line()
    }

    pub fn header_crossed(&self) -> bool {
        self.inline.header_crossed()
    }

    fn account(&mut self, total: &mut Work) {
        let w = std::mem::take(&mut self.work);
        debug_assert_eq!(w.tree_visits, 0);
        debug_assert_eq!(w.link_title_gap_steps, 0);
        debug_assert_eq!(w.link_fallback_bytes, 0);
        debug_assert_eq!(w.link_fallback_steps, 0);
        total.code_fallback_eligibility_bytes += w.code_fallback_bytes;
        total.code_fallback_eligibility_steps +=
            w.code_fallback_steps + w.code_fallback_comparisons;
        total.bare_eligibility_bytes += w.bare_autolink_bytes;
        total.link_label_steps += w.link_label_steps;
        total.eligibility_undo_peak = total.eligibility_undo_peak.max(w.inline_undo_peak);
        total.eligibility_undo_bytes_peak = total
            .eligibility_undo_bytes_peak
            .max(w.inline_undo_bytes_peak);

        let steps = w.inline_bytes
            + w.delimiter_steps
            + w.delimiter_search_steps
            + w.flanking_steps
            + w.label_bytes
            + w.label_scalars
            + w.label_fold_steps
            + w.label_key_bytes
            + w.autolink_bytes
            + w.entity_bytes
            + w.math_steps
            + w.footnote_steps
            + w.footnote_label_bytes;
        if self.container {
            total.container_eligibility_bytes += w.inline_bytes;
            total.container_eligibility_steps += steps;
        } else {
            total.root_eligibility_bytes += w.inline_bytes;
            total.root_eligibility_steps += steps;
        }
    }

    pub fn virtual_spaces(&mut self, source: &str, at: usize, columns: u8, total: &mut Work) {
        debug_assert!((1..=3).contains(&columns));
        if self.last_virtual_tab == Some(at) {
            return;
        }
        debug_assert!(self.last_virtual_tab.is_none_or(|last| last < at));
        self.inline
            .virtual_spaces(at, columns, &mut Effects::observing(source, &mut self.work));
        self.last_virtual_tab = Some(at);
        self.account(total);
    }

    #[cfg(test)]
    pub fn assert_same(&self, publisher: &Inline) {
        tests::same(publisher, &self.inline, "container event");
        assert_eq!(self.work, Work::default(), "observer counters drained");
    }

    pub fn feed(&mut self, source: &str, span: Span, indent: usize, total: &mut Work) {
        // A deferred header's real owner may release an already observed
        // prefix. Only its unobserved suffix enters the old grammar again.
        let end = self.cursor.unwrap_or(span.start - indent);
        self.work.inline_bytes += span.start.saturating_sub(end).min(indent) as u64;
        let span = Span::new(span.start.max(end).min(span.end), span.end);
        self.inline
            .feed(span, &mut Effects::observing(source, &mut self.work));
        self.cursor = Some(end.max(span.end));
        self.account(total);
    }

    pub fn isolate_line(&mut self, total: &mut Work) {
        self.inline.isolate_line();
        // Fixed-state header isolation is additional observer work too.
        self.work.delimiter_steps += 1;
        self.account(total);
    }

    pub fn cancel_header(&mut self, source: &str, total: &mut Work) {
        self.inline
            .cancel_header(&mut Effects::observing(source, &mut self.work));
        self.account(total);
    }

    pub fn confirm_header(&mut self, source: &str, total: &mut Work) -> bool {
        let reuse = self
            .inline
            .confirm_header(&mut Effects::observing(source, &mut self.work));
        self.account(total);
        reuse
    }

    pub fn finish(&mut self, source: &str, total: &mut Work) {
        self.inline
            .finish(&mut Effects::observing(source, &mut self.work));
        self.account(total);
    }
}

impl Inline {
    pub fn root_paragraph(&mut self, key: crate::table::ParagraphKey) {
        self.root_paragraph = true;
        self.paragraph_key = Some(key);
    }

    pub fn container_paragraph(&mut self, key: crate::table::ParagraphKey) {
        self.container_paragraph = true;
        self.paragraph_key = Some(key);
    }

    pub(crate) fn parser_stats(&self) -> crate::LinkParserStats {
        crate::LinkParserStats {
            candidates: crate::ArenaStats {
                nodes: self.brackets.len()
                    + usize::from(self.bracket.is_some())
                    + usize::from(self.tail.is_some() || self.header_link.is_some()),
                capacity_bytes: self.brackets.capacity() * std::mem::size_of::<Bracket>()
                    + std::mem::size_of::<Option<Bracket>>()
                    + std::mem::size_of::<Option<Tail>>()
                    + std::mem::size_of::<Option<Candidate>>(),
                allocation_calls: self.bracket_allocations,
            },
            delimiters: self.label_delimiters.stats(),
            label_key_capacity: self.label.capacity(),
        }
    }

    pub fn paragraph_key(&self) -> Option<crate::table::ParagraphKey> {
        self.paragraph_key
    }

    pub fn component_prefix(&mut self, prefix: Span, work: &mut Work) {
        if self.code_recovery.is_none() {
            self.code_fallback.record(CodeEvent::Prefix(prefix), work);
        }
        if let Some(Tail::Destination {
            parser, title_gaps, ..
        }) = &mut self.tail
        {
            if parser.open_title() {
                parser.title_prefix(prefix);
                if !prefix.is_empty() {
                    debug_assert!(title_gaps.last().is_none_or(|gap| gap.end <= prefix.start));
                    title_gaps.push(prefix);
                    work.link_title_gap_steps += 1;
                }
            } else {
                parser.component_prefix(prefix);
            }
        }
    }

    pub fn root_separator_seen(&self) -> bool {
        self.root_separator_seen
    }

    /// Feed at most three semantic spaces owned by one original tab byte.
    /// Used before nonblank container text (new or continuing), so it can
    /// neither create a delimiter nor become a trailing ordinary line break.
    pub fn virtual_spaces(&mut self, at: usize, columns: u8, effects: &mut Effects<'_>) {
        if self.code_recovery.is_none() {
            self.code_fallback
                .record(CodeEvent::Virtual { at, columns }, effects.work);
        }
        self.math_prefix(at, effects);
        let title_body = self.container_paragraph
            && matches!(&self.tail, Some(Tail::Destination { parser, .. }) if parser.open_title());
        for _ in 0..columns {
            if self.code_recovery.is_some() {
                effects.work.code_fallback_bytes += 1;
            } else {
                effects.work.inline_bytes += 1;
                if effects
                    .publishing
                    .as_ref()
                    .is_some_and(|p| p.line.replay.is_some_and(|r| r.start <= at && at < r.end))
                {
                    effects.work.header_replay_bytes += 1;
                }
            }
            if !title_body {
                self.byte(b' ', at, Origin::Virtual, effects);
            }
            self.previous = Some(b' ');
            self.previous_scalar = Some(' ');
        }
        if let Some(math) = &mut self.math {
            math.end = at + 1;
        }
    }

    pub(crate) fn undo_stats(&self) -> crate::rewind::UndoStats {
        self.stack
            .undo_stats()
            .plus(self.brackets.undo_stats())
            .plus(self.label.undo_stats())
            .plus(self.bare_markers.undo_stats())
            .plus(self.email_pending.undo_stats())
            .plus(self.email_waiting.undo_stats())
            .plus(self.email_ready.undo_stats())
            .plus(self.label_delimiters.undo_stats())
            .plus(self.code_fallback.runs.undo_stats())
            .plus(self.code_fallback.events.undo_stats())
    }
    fn account_undo_peak(&self, work: &mut Work) {
        let s = self.undo_stats();
        work.inline_undo_peak = work.inline_undo_peak.max(s.entries as u64);
        work.inline_undo_bytes_peak = work.inline_undo_bytes_peak.max(s.bytes as u64);
    }
    pub(crate) fn style_scope_key(&self) -> Option<usize> {
        self.style_scope
    }
    fn save_line(&mut self) {
        assert!(self.header_saved.is_none());
        assert!(
            self.tail.is_none() && self.code_recovery.is_none() && self.bare_recovery.is_none()
        );
        let mut saved = Box::new(Self::default());
        saved.style_scope = self.style_scope;
        saved.root_paragraph = self.root_paragraph;
        saved.container_paragraph = self.container_paragraph;
        saved.paragraph_key = self.paragraph_key;
        saved.root_separator_seen = self.root_separator_seen;
        saved.raw_content = self.raw_content;
        saved.raw_cr = self.raw_cr;
        saved.last = self.last;
        saved.run = self.run;
        saved.code = self.code;
        saved.math = self.math;
        saved.footnote = self.footnote;
        saved.bracket = self.bracket;
        saved.label_overflow = self.label_overflow;
        saved.bang = self.bang;
        saved.escape = self.escape;
        saved.previous = self.previous;
        saved.previous_scalar = self.previous_scalar;
        saved.spaces = self.spaces;
        saved.whitespace = self.whitespace;
        saved.carriage = self.carriage;
        saved.line_break = self.line_break;
        saved.autolink = self.autolink;
        saved.bare = self.bare;
        saved.bare_replay = self.bare_replay;
        saved.bare_collect = self.bare_collect;
        saved.entity = self.entity;
        saved.code_fallback.checkpoint = self.code_fallback.checkpoint;
        saved.code_fallback.boundary = self.code_fallback.boundary;
        self.stack.begin();
        self.brackets.begin();
        self.label.begin();
        self.bare_markers.begin();
        self.email_pending.begin();
        self.email_waiting.begin();
        self.email_ready.begin();
        self.label_delimiters.begin();
        self.code_fallback.runs.begin();
        self.code_fallback.events.begin();
        self.header_saved = Some(LineCheckpoint(saved));
    }
    fn commit_line(&mut self, work: &mut Work) {
        self.account_undo_peak(work);
        if self.header_saved.take().is_none() {
            return;
        }
        work.inline_checkpoint_steps += 1;
        work.inline_checkpoint_steps += self.stack.commit();
        work.inline_checkpoint_steps += self.brackets.commit();
        work.inline_checkpoint_steps += self.label.commit();
        work.inline_checkpoint_steps += self.bare_markers.commit();
        work.inline_checkpoint_steps += self.email_pending.commit();
        work.inline_checkpoint_steps += self.email_waiting.commit();
        work.inline_checkpoint_steps += self.email_ready.commit();
        work.inline_checkpoint_steps += self.label_delimiters.commit();
        work.inline_checkpoint_steps +=
            self.code_fallback.runs.commit() + self.code_fallback.events.commit();
    }
    fn restore_line(&mut self, work: &mut Work) {
        self.account_undo_peak(work);
        let mut saved = *self.header_saved.take().unwrap().0;
        work.inline_rollback_steps += self.stack.rollback();
        saved.stack = std::mem::take(&mut self.stack);
        work.inline_rollback_steps += self.brackets.rollback();
        saved.brackets = std::mem::take(&mut self.brackets);
        work.inline_rollback_steps += self.label.rollback();
        saved.label = std::mem::take(&mut self.label);
        work.inline_rollback_steps += self.bare_markers.rollback();
        saved.bare_markers = std::mem::take(&mut self.bare_markers);
        work.inline_rollback_steps += self.email_pending.rollback();
        saved.email_pending = std::mem::take(&mut self.email_pending);
        work.inline_rollback_steps += self.email_waiting.rollback();
        saved.email_waiting = std::mem::take(&mut self.email_waiting);
        work.inline_rollback_steps += self.email_ready.rollback();
        saved.email_ready = std::mem::take(&mut self.email_ready);
        work.inline_rollback_steps += self.label_delimiters.rollback();
        saved.label_delimiters = std::mem::take(&mut self.label_delimiters);
        work.inline_rollback_steps +=
            self.code_fallback.runs.rollback() + self.code_fallback.events.rollback();
        saved.code_fallback.runs = std::mem::take(&mut self.code_fallback.runs);
        saved.code_fallback.events = std::mem::take(&mut self.code_fallback.events);
        saved.bracket_allocations = self.bracket_allocations;
        *self = saved;
    }

    /// Emphasis openers, code/math, a label and the pending break may reach the
    /// preceding line. Inspect fixed state, never walk or clone the stack.
    pub fn can_isolate_line(&self) -> bool {
        self.previous == Some(b'\n')
            && self.run.is_none()
            && self.footnote.is_none()
            && self.bang.is_none()
            && self.tail.is_none()
            && self.escape.is_none()
            && self.spaces.is_none()
            && self.whitespace.is_none()
            && !self.carriage
            && self.autolink.is_none()
            && self.entity.is_none()
    }

    pub fn isolate_line(&mut self) -> Option<LineBreak> {
        debug_assert!(self.can_isolate_line());
        debug_assert!(
            self.header_pairs.is_empty()
                && self.header_code.is_none()
                && self.header_math.is_none()
                && self.header_link.is_none()
                && self.header_link_suffix.is_none()
        );
        self.save_line();
        // Preserve the existing separable summaries. The outer checkpoint also
        // covers the ordinary path after one of these summaries must flush.
        self.header_email_pending = self.email_pending.len();
        self.header_floor = self.stack.len();
        self.header_label_delimiters = self.label_delimiters.len();
        self.header_label_tail = self.bracket.and_then(|b| b.journal.tail);
        self.header_crossed = false;
        self.header_bracket = self.bracket.map(|b| b.open);
        self.header_opaque = self.code.is_some() || self.math.is_some();
        if self.code.is_some() {
            self.code_fallback.isolate();
        }
        self.previous = None;
        self.previous_scalar = None;
        self.line_break.take()
    }

    pub fn header_crossed(&self) -> bool {
        self.header_saved.is_none() && self.header_crossed
    }

    pub fn cancel_header(&mut self, effects: &mut Effects<'_>) {
        self.flush_header(effects);
        self.commit_line(effects.work);
        if let Some(p) = &mut effects.publishing {
            crate::header_effects::commit(p, effects.work);
        }
    }

    fn flush_header(&mut self, effects: &mut Effects<'_>) {
        if let Some(candidate) = self.header_link.take() {
            effects.work.delimiter_steps += 1;
            let candidate = self.shortcut(candidate, effects);
            if let Some(suffix) = self.header_link_suffix.take() {
                match suffix {
                    HeaderLinkSuffix::Destination {
                        source,
                        destination,
                        title,
                        title_gaps,
                    } => {
                        let mut candidate = candidate;
                        self.end_label(&mut candidate, effects);
                        self.complete_candidate(
                            candidate,
                            Some(LinkTarget::Direct(destination)),
                            Some(source),
                            title,
                            title_gaps,
                            effects,
                        );
                    }
                    HeaderLinkSuffix::Label(source) => {
                        if let Some(fallback) = self.reference_tail(candidate, source, effects) {
                            self.tail = Some(Tail::After(fallback));
                        }
                    }
                }
            } else if let Some(tail) = &mut self.tail {
                // A rejecting byte can arrive partway through a bounded suffix.
                // Resume it with the original shortcut ID and parser position.
                match tail {
                    Tail::After(waiting)
                    | Tail::Destination {
                        candidate: waiting, ..
                    }
                    | Tail::Label {
                        candidate: waiting, ..
                    } => *waiting = candidate,
                }
            } else {
                self.seal_candidate(candidate, effects);
            }
        }
        if let Some(pair) = self.header_code.take() {
            effects.work.delimiter_steps += 1;
            pair.commit(effects);
            self.code_fallback.clear();
        }
        if let Some(pair) = self.header_math.take() {
            effects.work.math_steps += 1;
            pair.commit(effects);
        }
        effects.work.delimiter_steps += self.header_pairs.len() as u64;
        let collect = self.bare_collect.is_some() || !self.email_waiting.is_empty();
        for pair in self.header_pairs.drain(..) {
            if collect {
                for marker in [pair.open, pair.close] {
                    self.bare_markers.insert(marker.start, marker.end);
                }
            }
            pair.commit(effects);
        }
        self.clear_header();
        self.publish_ready_emails(effects);
        self.account_labels(effects);
    }

    fn clear_header(&mut self) {
        self.code_fallback.boundary = None;
        self.header_floor = 0;
        self.header_crossed = false;
        self.header_bracket = None;
        self.header_label_delimiters = 0;
        self.header_label_tail = None;
        self.header_opaque = false;
    }

    /// On success discard inherited openers, retaining the first cell's live
    /// candidates and already published objects. Each surviving opener is
    /// rebased only once, when this inline sequence becomes a table cell.
    /// Return whether the first-cell state can be reused. An opaque code/math prefix
    /// is instead parsed once as a cell during the existing row scan. Never
    /// finish its pending run: that could publish a cross-line mapping/object.
    pub fn confirm_header(&mut self, effects: &mut Effects<'_>) -> bool {
        if self.header_saved.is_some() {
            self.restore_line(effects.work);
            if let Some(p) = &mut effects.publishing {
                crate::header_effects::rollback(p, effects.work);
            }
            self.finish(effects);
            return false;
        }
        if self.code_fallback.truncate_header() {
            // The current row has no committed effects while inherited code is
            // opaque (or its closure isolated). Finish only the old paragraph,
            // preserving its frames/mode, then use the existing fresh-cell path.
            effects.work.delimiter_steps += u64::from(self.header_code.take().is_some());
            self.clear_header();
            self.bare = None;
            self.run = None;
            self.code = Some(Code::new(self.code_fallback.checkpoint.unwrap().open));
            self.finish(effects);
            return false;
        }
        self.finish_emails(self.header_email_pending, effects);
        self.header_email_pending = 0;
        self.root_paragraph = false;
        self.container_paragraph = false;
        self.paragraph_key = None;
        self.root_separator_seen = false;
        debug_assert!(!self.header_crossed);
        let old_open = self.header_bracket;
        let mut inherited = if let Some(candidate) = &mut self.header_link {
            Some(std::mem::take(&mut candidate.journal))
        } else if self.bracket.is_some_and(|b| Some(b.open) == old_open) {
            let bracket = self.bracket.take().unwrap();
            self.bracket = self.brackets.pop();
            Some(bracket.journal)
        } else if let Some(index) = self.brackets.iter().position(|b| Some(b.open) == old_open) {
            effects.work.link_label_steps += self.brackets.len() as u64;
            Some(self.brackets.remove(index).journal)
        } else {
            None
        };
        let mut current = LabelList::default();
        if let Some(prefix) = &mut inherited {
            current = self
                .label_delimiters
                .split_after(prefix, self.header_label_tail);
            self.header_floor = 0;
            self.resolve_journal(*prefix, None, old_open, effects);
            self.header_floor = self.stack.len();
        }
        // In an independent cell a full reference suffix is itself a shortcut.
        // Its inert label has no other local effects or pending delimiter state.
        // Publish it before any later cell objects, without replaying the row.
        if let Some(HeaderLinkSuffix::Label(source)) = self.header_link_suffix.take()
            && let Some(symbol) =
                effects.reference(&effects.source[source.start + 1..source.end - 1])
        {
            let id = effects.complete_link(
                Candidate::bare(source.start, source.end - 1, None),
                symbol.target(),
                None,
                None,
                Vec::new(),
            );
            effects.seal_link(id);
        }
        let work = &mut effects.work;
        work.delimiter_steps += self.header_pairs.len() as u64;
        self.header_pairs.clear();
        work.delimiter_steps += u64::from(self.header_code.take().is_some());
        work.math_steps += u64::from(self.header_math.take().is_some());
        if self.header_link.take().is_some() {
            work.delimiter_steps += 1;
            self.tail = None;
        }
        if self.header_opaque {
            work.delimiter_steps += self.stack.len() as u64;
            self.account_labels(effects);
            *self = Self::default();
            return false;
        }
        if self.header_floor != 0 {
            work.delimiter_steps += self.stack.len() as u64;
            self.stack.drain(..self.header_floor);
            self.last = [None; 14];
            for (index, delimiter) in self.stack.iter_mut().enumerate() {
                delimiter.previous = self.last[delimiter.kind];
                self.last[delimiter.kind] = Some(index);
            }
        }
        if self
            .bracket
            .is_some_and(|b| Some(b.open) == self.header_bracket)
        {
            self.bracket = None;
        }
        self.clear_header();
        self.resolve_journal(current, None, None, effects);
        if self.bracket.is_none() {
            self.label.clear();
            self.label_overflow = false;
        }
        self.publish_ready_emails(effects);
        self.account_labels(effects);
        true
    }

    fn math_prefix(&self, start: usize, effects: &mut Effects<'_>) {
        if let Some(math) = &self.math
            && math.end < start
            && effects.source.as_bytes()[math.end - 1] == b'\n'
        {
            // Record omitted source once, without inspecting/copying the body
            // or walking ancestors. Failed candidates may leave unused gaps;
            // they cannot overlap a later confirmed object's content.
            // An ATX closing suffix is a same-line gap, not a container prefix.
            effects.math_gap(Span::new(math.end, start));
        }
    }

    pub fn feed(&mut self, span: Span, effects: &mut Effects<'_>) {
        if span.is_empty() {
            return;
        }
        let style_scope = *self.style_scope.get_or_insert(span.start);
        effects.style_touch(style_scope, span);
        if self.code_recovery.is_none() {
            self.code_fallback
                .record(CodeEvent::Raw(span), effects.work);
        }
        self.math_prefix(span.start, effects);
        for pos in span.range() {
            let byte = effects.source.as_bytes()[pos];
            if self.code_recovery.is_some() {
                effects.work.code_fallback_bytes += 1;
            } else {
                effects.work.inline_bytes += 1;
            }
            if self.code_recovery.is_none() {
                if effects
                    .publishing
                    .as_ref()
                    .is_some_and(|p| p.line.replay.is_some_and(|r| r.start <= pos && pos < r.end))
                {
                    effects.work.header_replay_bytes += 1;
                }
                if let Some(cr) = self.raw_cr.take()
                    && byte != b'\n'
                {
                    self.raw_nonspace(cr);
                }
                match byte {
                    b'\r' => self.raw_cr = Some(pos),
                    b' ' | b'\t' | b'\n' => (),
                    _ => self.raw_nonspace(pos),
                }
            }
            self.byte(byte, pos, Origin::Raw, effects);
            self.previous = Some(byte);
            if byte.is_ascii() {
                self.previous_scalar = Some(char::from(byte));
            } else if byte >= 0xc0 {
                self.previous_scalar = effects.source[pos..].chars().next();
            }
        }
        if let Some(math) = &mut self.math {
            math.end = span.end;
        }
        self.account_labels(effects);
    }

    fn raw_nonspace(&mut self, pos: usize) {
        self.raw_content.get_or_insert(Span::new(pos, pos + 1)).end = pos + 1;
    }

    /// Called only after the last payload LF, before the Setext underline is
    /// consumed. Internal whitespace stays inside this source envelope.
    pub fn setext_content(&self) -> Option<Span> {
        self.raw_content
    }

    /// Raw paragraph suffix, excluding its physical LF/CRLF. The existing
    /// byte scan includes opaque/failed inline candidates and never trims
    /// decoded entity or code-span output. A lone final CR is content.
    pub fn paragraph_tail(&self, end: usize) -> Option<Span> {
        let start = self
            .raw_content?
            .end
            .max(self.raw_cr.map_or(0, |cr| cr + 1));
        (start < end).then_some(Span::new(start, end))
    }

    fn flush_bare(&mut self, next: Option<char>, effects: &mut Effects<'_>) {
        let Some(candidate) = self.bare.take() else {
            return;
        };
        if matches!(
            candidate.kind,
            bare_autolink::Kind::Email | bare_autolink::Kind::Protocol { domain: true, .. }
        ) {
            self.flush_email(candidate, next, effects);
            return;
        }
        let linked = candidate.seal(effects.source, &mut effects.work.bare_autolink_bytes);
        let web = matches!(candidate.kind, bare_autolink::Kind::Web { .. });
        debug_assert!(self.bare_recovery.is_none() || !web || linked.is_some());
        self.previous = candidate.before;
        self.previous_scalar = candidate.before_scalar;
        if linked.is_none() && web && self.bare_recovery.is_none() {
            // Synchronous, one-level recovery. The index admits only valid
            // inner Web candidates; ordinary releases still disable discovery.
            // Source nesting therefore cannot add replay passes or stack depth.
            self.bare_recovery = Some(bare_autolink::Recovery::new(
                candidate.span,
                effects.source,
                &mut effects.work.bare_autolink_bytes,
            ));
            self.bare_plain(candidate.span, effects);
            self.flush_bare(next, effects);
            self.bare_recovery = None;
            return;
        }
        self.bare_replay = true;
        let prefix_end = linked.map_or(candidate.span.end, |(span, _)| span.start);
        self.bare_plain(Span::new(candidate.span.start, prefix_end), effects);
        if let Some((label, www)) = linked {
            // New objects cannot overtake an isolated inherited object.
            if self.header_code.is_some()
                || self.header_math.is_some()
                || self.header_link.is_some()
            {
                self.flush_header(effects);
                self.header_crossed = true;
            }
            self.finish_run(effects.source[label.start..].chars().next(), effects);
            effects.bare_autolink(
                label,
                www,
                matches!(candidate.kind, bare_autolink::Kind::Email),
            );
            self.previous = effects.source.as_bytes().get(label.end - 1).copied();
            self.previous_scalar = effects.source[label.range()].chars().next_back();
            self.spaces = None;
            self.whitespace = None;
            self.bang = None;
            self.bare_plain(Span::new(label.end, candidate.span.end), effects);
        }
        self.bare_replay = false;
    }

    fn flush_email(
        &mut self,
        candidate: bare_autolink::Candidate,
        next: Option<char>,
        effects: &mut Effects<'_>,
    ) {
        self.previous = candidate.before;
        self.previous_scalar = candidate.before_scalar;
        self.bare_replay = true;
        if self.email_waiting.is_empty() {
            self.bare_markers.clear();
        }
        self.bare_collect = Some(candidate.span);
        let isolated = self.header_pairs.len();
        self.bare_plain(candidate.span, effects);
        self.finish_run(next, effects);
        self.bare_collect = None;
        // The stack is source ordered. Register each local opener once, so a
        // later match/discard can settle this token without visiting old uses.
        let first = self
            .stack
            .partition_point(|d| d.source.start < candidate.span.start);
        let header_wait = self.header_pairs.len() != isolated;
        if first < self.stack.len() || header_wait {
            let id = self.email_pending.len();
            self.email_pending.push(PendingEmail {
                candidate: Some(candidate),
                openers: self.stack.len() - first,
            });
            self.email_waiting.push_back(id);
            if first == self.stack.len() {
                self.email_ready.push(id);
            }
            for delimiter in &mut self.stack[first..] {
                effects.work.bare_autolink_bytes += 1;
                delimiter.email = Some(id);
            }
        } else {
            self.publish_email(candidate, true, effects);
        }
        self.bare_replay = false;
    }

    fn email_pair(&mut self, pair: &EmphasisPair) {
        if self.bare_collect.is_some() || !self.email_waiting.is_empty() {
            for marker in [pair.open, pair.close] {
                self.bare_markers.insert(marker.start, marker.end);
            }
        }
    }

    fn publish_email(
        &mut self,
        mut candidate: bare_autolink::Candidate,
        cancel_header: bool,
        effects: &mut Effects<'_>,
    ) {
        let span = candidate.span;
        let raw = &effects.source[span.range()];
        effects.work.bare_autolink_bytes += raw.len() as u64;
        let at = candidate.at.unwrap();
        let mut label = span;
        if matches!(candidate.kind, bare_autolink::Kind::Email) {
            label.start = candidate.address;
        }
        for (&start, &end) in self.bare_markers.range(span.range()) {
            effects.work.bare_autolink_bytes += 1;
            if end <= at {
                label.start = label.start.max(end);
            } else if start > at {
                label.end = label.end.min(start);
            }
        }
        effects.work.bare_autolink_bytes += label.len() as u64;
        let native_protocol = matches!(candidate.kind, bare_autolink::Kind::Protocol { .. })
            && label.start <= candidate.address;
        let accepted = if native_protocol {
            candidate.span.end = label.end;
            candidate
                .seal(effects.source, &mut effects.work.bare_autolink_bytes)
                .map(|(label, _)| label)
        } else {
            // A marker may put the protocol prefix in another text node. Then
            // the address in this node follows the ordinary email rules.
            if let Some(slash) = effects.source[at..label.end].find('/') {
                label.end = at + slash;
            }
            bare_autolink::email(&effects.source[label.range()])
                .map(|len| Span::new(label.start, label.start + len))
        };
        if let Some(label) = accepted {
            if cancel_header
                && (self.header_code.is_some()
                    || self.header_math.is_some()
                    || self.header_link.is_some())
            {
                self.flush_header(effects);
                self.header_crossed = true;
            }
            effects.bare_autolink(label, false, !native_protocol);
        }
    }

    fn finish_emails(&mut self, before: usize, effects: &mut Effects<'_>) {
        while self.email_waiting.front().is_some_and(|&id| id < before) {
            let id = self.email_waiting.pop_front().unwrap();
            if let Some(candidate) = self.email_pending[id].candidate.take() {
                self.publish_email(candidate, false, effects);
            }
        }
    }

    fn email_popped(&mut self, delimiter: Delimiter) {
        if let Some(id) = delimiter.email {
            let pending = &mut self.email_pending[id];
            pending.openers -= 1;
            if pending.openers == 0 && pending.candidate.is_some() {
                self.email_ready.push(id);
            }
        }
    }

    fn publish_ready_emails(&mut self, effects: &mut Effects<'_>) {
        if !self.header_pairs.is_empty() {
            return;
        }
        let ready = self.email_ready.len();
        for _ in 0..ready {
            let id = self.email_ready.pop_front().unwrap();
            if let Some(candidate) = self.email_pending[id].candidate.take() {
                self.publish_email(candidate, true, effects);
            }
        }
        while self
            .email_waiting
            .front()
            .is_some_and(|&id| self.email_pending[id].candidate.is_none())
        {
            self.email_waiting.pop_front();
        }
    }

    fn bare_plain(&mut self, span: Span, effects: &mut Effects<'_>) {
        for pos in span.range() {
            effects.work.bare_autolink_bytes += 1;
            let byte = effects.source.as_bytes()[pos];
            self.byte(byte, pos, Origin::Raw, effects);
            self.previous = Some(byte);
            if byte.is_ascii() {
                self.previous_scalar = Some(char::from(byte));
            } else if byte >= 0xc0 {
                self.previous_scalar = effects.source[pos..].chars().next();
            }
        }
    }

    fn byte(&mut self, byte: u8, pos: usize, origin: Origin, effects: &mut Effects<'_>) {
        if let Some(candidate) = &mut self.bare {
            effects.work.bare_autolink_bytes += 1;
            if candidate.byte(byte, pos, effects.source, self.bare_recovery.as_ref()) {
                return;
            }
            let next = if matches!(origin, Origin::Virtual) {
                Some(' ')
            } else {
                effects.source[pos..].chars().next()
            };
            self.flush_bare(next, effects);
        }
        if self.bracket.is_some() && !self.label_overflow {
            if self.label.len() <= MAX_LABEL_BYTES {
                self.label.push(byte);
            } else {
                self.label_overflow = true;
            }
        }
        // Only a subsequent byte in the same inline sequence confirms a break.
        // Sequence closure discards it, so terminal newlines stay plain.
        if let Some(line_break) = self.line_break.take() {
            line_break.commit(effects);
        }
        if let Some(run) = &mut self.run {
            if byte == run.byte {
                run.source.end = pos + 1;
                run.label_len = self.label.len();
                run.label_overflow = self.label_overflow;
                return;
            }
            let next = if byte.is_ascii() {
                Some(char::from(byte))
            } else {
                effects.source[pos..].chars().next()
            };
            self.finish_run(next, effects);
        }
        let syntax = matches!(
            byte,
            b'&' | b'<' | b'\\' | b'*' | b'_' | b'~' | b'`' | b'$' | b'!' | b'[' | b']'
        );
        let suffix_delimiter = matches!(
            (&self.tail, byte),
            (Some(Tail::After(_)), b'[') | (Some(Tail::Label { .. }), b']')
        );
        if ((self.header_code.is_some() || self.header_math.is_some()) && syntax)
            || (self.header_link.is_some()
                && ((syntax && !suffix_delimiter)
                    || (byte == b'^' && matches!(self.tail, Some(Tail::Label { .. })))
                    // A completed [key] could take a different tail in the cell.
                    || (byte == b'(' && matches!(self.header_link_suffix, Some(HeaderLinkSuffix::Label(_))))))
        {
            // Flush before this byte can publish a source-ordered replacement
            // or object. Rejection is permanent
            // for this line, even if its next opaque candidate is open at '|'.
            self.flush_header(effects);
            self.header_crossed = true;
        }
        if let Some(math) = &mut self.math {
            if math.escaped {
                math.escaped = false;
            } else if byte == b'\\' {
                math.escaped = true;
            } else if byte == b'$' {
                self.start_run(byte, pos);
            }
            return;
        }
        if let Some(code) = &mut self.code {
            if byte == b'`' {
                self.start_run(byte, pos);
            } else {
                code.payload(byte, pos, self.previous);
            }
            return;
        }
        if self.tail.is_some() && self.tail_byte(byte, pos, origin, effects) {
            if (self.root_paragraph || self.container_paragraph) && byte == b'\n' {
                // An admitted separator remains a provisional ordinary break
                // until the suffix succeeds. Failed/EOF candidates retain its
                // original mapping; completion hides the whole suffix.
                self.carriage = false;
                self.newline(pos);
            }
            return;
        }
        if let Some(start) = self.autolink.take() {
            if byte == b'>' {
                effects.work.autolink_bytes += (pos - start - 1) as u64;
                if let Some(target) = crate::autolink::classify(&effects.source[start + 1..pos]) {
                    let id = effects.autolink(start, pos, target);
                    self.attach_object(id, effects);
                    if let Some(parent) = &mut self.bracket {
                        parent.blockers.known = true;
                        parent.blockers.root = Guard::Always;
                    }
                }
                return;
            }
            if byte > b' '
                && byte != 0x7f
                && byte != b'<'
                && pos - start <= crate::MAX_AUTOLINK_BYTES
            {
                self.autolink = Some(start);
                return;
            }
            // Consume the rejecting byte normally, without replaying the prefix.
        }
        if let Some(start) = self.entity.take() {
            if byte == b';' {
                effects.work.entity_bytes += (pos - start - 1) as u64;
                if let Some(text) = crate::entity::decode(&effects.source[start + 1..pos]) {
                    let span = Span::new(start, pos + 1);
                    effects.entity(span, text);
                }
                return;
            }
            if pos - start <= crate::entity::MAX_ENTITY_BODY
                && (byte.is_ascii_alphanumeric() || (byte == b'#' && pos == start + 1))
            {
                self.entity = Some(start);
                return;
            }
            // Failed candidates remain literal; process just the rejecting byte.
        }
        if let Some(start) = self.footnote.take() {
            if byte == b']' {
                let label = &effects.source[start + 2..pos];
                if crate::footnotes::valid_label(label) {
                    effects.footnote(start, pos);
                }
                return;
            }
            if pos - start - 2 < crate::MAX_FOOTNOTE_LABEL_BYTES
                && (byte >= 128 || byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
            {
                self.footnote = Some(start);
                return;
            }
            // Failed candidates never replay their opaque prefix. Only the
            // rejecting byte is processed normally, like other inline candidates.
        }
        if self.carriage && byte != b'\n' {
            self.spaces = None;
            self.whitespace = None;
            self.escape = None;
        }
        self.carriage = byte == b'\r';
        if self.carriage {
            return;
        }
        if byte == b'\n' {
            self.newline(pos);
            return;
        }
        if matches!(byte, b' ' | b'\t') {
            self.whitespace.get_or_insert(pos);
        } else {
            self.whitespace = None;
        }
        if byte == b' ' {
            self.spaces.get_or_insert(pos);
        } else {
            self.spaces = None;
        }
        if let Some(escape) = self.escape.take()
            && byte.is_ascii_punctuation()
        {
            effects.hide(Span::new(escape, escape + 1));
            return;
        }
        if !self.bare_replay
            && matches!(origin, Origin::Raw)
            && self.bracket.is_none()
            && bare_autolink::local(byte)
        {
            let mut candidate =
                bare_autolink::Candidate::new(pos, self.previous, self.previous_scalar);
            effects.work.bare_autolink_bytes += 1;
            assert!(candidate.byte(byte, pos, effects.source, self.bare_recovery.as_ref()));
            self.bare = Some(candidate);
            return;
        }
        let bang = self.bang.take();
        match byte {
            b'^' if self
                .bracket
                .is_some_and(|b| b.image.is_none() && b.open + 1 == pos) =>
            {
                let bracket = self.bracket.take().unwrap();
                if let Some(key) = self.style_scope {
                    effects.style_leave(key, bracket.open, pos);
                }
                self.footnote = Some(bracket.open);
                self.bracket = self.brackets.pop();
                self.label.clear();
                self.label_overflow = self.bracket.is_some();
            }
            b'&' => self.entity = Some(pos),
            b'<' => {
                self.autolink = Some(pos);
            }
            b'\\' => self.escape = Some(pos),
            b'*' | b'_' | b'~' | b'`' | b'$' => self.start_run(byte, pos),
            b'!' => self.bang = Some(pos),
            b'[' => {
                effects.work.link_label_steps += 1;
                if let Some(mut parent) = self.bracket.take() {
                    parent.nested = true;
                    if self.brackets.len() == self.brackets.capacity() {
                        self.bracket_allocations += 1;
                    }
                    self.brackets.push(parent);
                }
                self.label.clear();
                self.label_overflow = false;
                self.bracket = Some(Bracket::new(pos, bang.filter(|at| *at + 1 == pos)));
                if let Some(key) = self.style_scope {
                    effects.style_enter(key, pos);
                }
            }
            b']' => {
                if let Some(bracket) = self.bracket.take() {
                    if let Some(key) = self.style_scope {
                        effects.style_leave(key, bracket.open, pos);
                    }
                    effects.work.link_label_steps += 1;
                    self.bracket = self.brackets.pop();
                    if let Some(parent) = &mut self.bracket {
                        parent.blockers.merge(bracket.blockers, effects);
                    }
                    let mut candidate = Candidate::bare(bracket.open, pos, bracket.image);
                    candidate.journal = bracket.journal;
                    candidate.children = bracket.children;
                    candidate.blockers = bracket.blockers;
                    candidate.nested = bracket.nested;
                    if candidate.image.is_none() && candidate.blockers.known {
                        self.fail_candidate(candidate, effects);
                    } else {
                        if self.header_bracket == Some(candidate.open) && !self.header_crossed {
                            debug_assert!(self.header_link.is_none());
                            self.header_link = Some(candidate);
                        } else {
                            candidate = self.shortcut(candidate, effects);
                        }
                        self.tail = Some(Tail::After(candidate));
                    }
                    if self.header_link.is_none() {
                        self.label.clear();
                        self.label_overflow = self.bracket.is_some();
                    }
                }
            }
            _ => {}
        }
    }

    fn newline(&mut self, pos: usize) {
        let end = pos - usize::from(self.previous == Some(b'\r'));
        let escape = self.escape.take().filter(|at| *at + 1 == end);
        let spaces = self.spaces.take();
        let whitespace = self.whitespace.take();
        self.line_break = Some(LineBreak {
            newline: Span::new(pos, pos + 1),
            hidden: Span::new(escape.or(whitespace).unwrap_or(end), end),
            hard: escape.is_some() || spaces.is_some_and(|start| end - start >= 2),
        });
        self.bang = None;
    }

    fn account_labels(&mut self, effects: &mut Effects<'_>) {
        effects.work.link_label_steps += std::mem::take(&mut self.label_delimiters.steps);
    }
    fn attach_object(&mut self, id: Option<LinkId>, effects: &mut Effects<'_>) {
        if let (Some(id), Some(parent)) = (id, &mut self.bracket) {
            effects.attach(
                &mut parent.children,
                Children {
                    head: Some(id),
                    tail: Some(id),
                },
            );
        }
    }
    fn complete_candidate(
        &mut self,
        mut candidate: Candidate,
        target: Option<LinkTarget>,
        suffix: Option<Span>,
        title: Option<Span>,
        title_gaps: Vec<Span>,
        effects: &mut Effects<'_>,
    ) -> Candidate {
        let direct = matches!(
            target,
            Some(LinkTarget::Direct(_) | LinkTarget::Email(_) | LinkTarget::Www(_))
        );
        let first = !candidate.accepted;
        candidate.link = effects.complete_link(candidate, target, suffix, title, title_gaps);
        candidate.accepted = true;
        candidate.children = Children::default();
        if first {
            self.attach_object(candidate.link, effects);
        }
        // Positive suffix alternatives are already witnessed by a blocker.
        // A complementary negative alternative can introduce the first link;
        // its raw target (or direct success) must join the enclosing summary.
        if candidate.image.is_none()
            && (!candidate.alternate || candidate.gate.is_some_and(|gate| !gate.positive))
        {
            let leaf = if first && !direct {
                effects.target_guard(candidate.link)
            } else {
                Guard::Never
            };
            if let Some(parent) = &mut self.bracket {
                if direct {
                    parent.blockers.known = true;
                    parent.blockers.root = Guard::Always;
                } else if first {
                    parent.blockers.potential = true;
                    if !parent.blockers.known {
                        parent.blockers.root = effects.join(parent.blockers.root, leaf);
                    }
                }
            }
        }
        candidate
    }
    fn shortcut(&mut self, mut candidate: Candidate, effects: &mut Effects<'_>) -> Candidate {
        let symbol = (!self.label_overflow && !candidate.nested)
            .then(|| {
                let label = std::str::from_utf8(&self.label[..self.label.len() - 1]).unwrap();
                effects.reference(label)
            })
            .flatten();
        if let Some(symbol) = symbol {
            candidate.reference = Some(symbol);
            self.end_label(&mut candidate, effects);
            candidate = self.complete_candidate(
                candidate,
                symbol.target(),
                None,
                None,
                Vec::new(),
                effects,
            );
        }
        candidate
    }
    fn resolve_journal(
        &mut self,
        mut list: LabelList,
        open: Option<usize>,
        owner: Option<usize>,
        effects: &mut Effects<'_>,
    ) {
        let inherited =
            owner.is_some() && owner == self.header_bracket && self.header_label_delimiters != 0;
        let boundary = if inherited {
            self.header_label_delimiters = 0;
            self.header_floor = 0;
            self.header_label_tail.take()
        } else {
            None
        };
        while let Some(head) = list.head {
            let delimiter = self.label_delimiters.pop(&mut list).unwrap();
            effects.work.delimiter_steps += 1;
            self.finish_delimiter(delimiter.run, delimiter.next, open, effects);
            if Some(head) == boundary {
                self.header_floor = self.stack.len();
            }
        }
    }
    fn end_label(&mut self, candidate: &mut Candidate, effects: &mut Effects<'_>) {
        let journal = std::mem::take(&mut candidate.journal);
        self.resolve_journal(journal, Some(candidate.open), Some(candidate.open), effects);
        while self
            .stack
            .last()
            .is_some_and(|d| d.source.start > candidate.open)
        {
            let delimiter = self.stack.pop().unwrap();
            self.email_popped(delimiter);
            self.last[delimiter.kind] = delimiter.previous;
            effects.work.delimiter_steps += 1;
        }
        self.header_floor = self.header_floor.min(self.stack.len());
        self.publish_ready_emails(effects);
    }
    fn fail_candidate(&mut self, mut candidate: Candidate, effects: &mut Effects<'_>) {
        if let Some(parent) = &mut self.bracket {
            self.label_delimiters
                .splice(&mut parent.journal, &mut candidate.journal);
            effects.attach(&mut parent.children, candidate.children);
        } else {
            self.resolve_journal(candidate.journal, None, Some(candidate.open), effects);
        }
    }

    fn start_run(&mut self, byte: u8, pos: usize) {
        self.run = Some(Run {
            byte,
            source: Span::new(pos, pos + 1),
            before: self.previous_scalar,
            label_len: self.label.len(),
            label_overflow: self.label_overflow,
        });
    }

    fn finish_run(&mut self, next: Option<char>, effects: &mut Effects<'_>) {
        let Some(run) = self.run.take() else {
            return;
        };
        effects.work.delimiter_steps += 1;
        if run.byte == b'$' {
            effects.work.math_steps += 1;
            if let Some(math) = &self.math {
                let open = math.open;
                if open.len() == run.source.len() {
                    let pair = MathPair {
                        open,
                        close: run.source,
                    };
                    if self.header_opaque && !self.header_crossed {
                        debug_assert!(self.header_math.is_none());
                        self.header_math = Some(pair);
                    } else {
                        pair.commit(effects);
                    }
                    self.math = None;
                }
            } else if run.source.len() <= 2 {
                self.math = Some(MathCandidate {
                    open: run.source,
                    escaped: false,
                    end: run.source.end,
                });
            }
            return;
        }
        if run.byte == b'`' {
            if let Some(code) = &mut self.code {
                let open = code.open;
                if open.len() == run.source.len() {
                    let pair = CodePair {
                        code: *code,
                        close: run.source,
                    };
                    if self.header_opaque && !self.header_crossed {
                        debug_assert!(self.header_code.is_none());
                        self.header_code = Some(pair);
                    } else {
                        pair.commit(effects);
                        self.code_fallback.clear();
                    }
                    self.code = None;
                } else {
                    code.all_spaces = false;
                    code.last = run.source;
                    code.last_space = false;
                    if self.code_recovery.is_none() {
                        self.code_fallback.push(run.source);
                    }
                }
            } else if self
                .code_recovery
                .as_ref()
                .is_none_or(|index| code_fallback::has_closer(index, run.source, effects.work))
            {
                self.code = Some(Code::new(run.source));
                if self.code_recovery.is_none() {
                    self.code_fallback.checkpoint = Some(CodeCheckpoint {
                        open: run.source,
                        label_len: run.label_len,
                        label_overflow: run.label_overflow,
                    });
                }
            }
            return;
        }
        if let Some(bracket) = &mut self.bracket {
            self.label_delimiters
                .push(&mut bracket.journal, LabelDelimiter { run, next });
        } else {
            self.finish_delimiter(run, next, None, effects);
        }
    }

    fn finish_delimiter(
        &mut self,
        run: Run,
        next: Option<char>,
        label_open: Option<usize>,
        effects: &mut Effects<'_>,
    ) {
        if run.byte == b'~' && run.source.len() > 2 {
            return;
        }
        let (can_open, can_close) = if run.byte == b'~' {
            // Preserve the independent single/double-tilde dialect.
            (
                next.is_some_and(|b| !b.is_ascii_whitespace()),
                run.before.is_some_and(|b| !b.is_ascii_whitespace()),
            )
        } else {
            use flanking::Class::{Punct, White};
            let before = flanking::class(run.before, effects.work);
            let after = flanking::class(next, effects.work);
            let before_white = before == White;
            let after_white = after == White;
            let before_punct = before == Punct;
            let after_punct = after == Punct;
            let left = !after_white && (!after_punct || before_white || before_punct);
            let right = !before_white && (!before_punct || after_white || after_punct);
            if run.byte == b'_' {
                (
                    left && (!right || before_punct),
                    right && (!left || after_punct),
                )
            } else {
                (left, right)
            }
        };
        let base = usize::from(run.byte == b'_') * 6;
        let residue = run.source.len() % 3;
        let kind = if run.byte == b'~' {
            12 + run.source.len() - 1
        } else {
            base + usize::from(can_close) * 3 + residue
        };
        if let Some(key) = self.style_scope {
            effects.style_run(
                key,
                crate::inline_styles::DelimiterRun {
                    source: run.source,
                    kind,
                    can_open,
                    can_close,
                },
            );
        }
        let mut close = run.source;
        while can_close && !close.is_empty() {
            let index = if run.byte == b'~' {
                self.last[kind].filter(|&index| {
                    label_open.is_none_or(|open| self.stack[index].source.start > open)
                })
            } else {
                // Eligibility is uniform within each category. Six head reads
                // replace an unbounded reverse search, including failed ones.
                let mut newest = None;
                for bucket in base..base + 6 {
                    effects.work.delimiter_search_steps += 1;
                    if (can_open || bucket - base >= 3)
                        && residue != 0
                        && (bucket % 3 + residue).is_multiple_of(3)
                    {
                        continue;
                    }
                    if let Some(index) = self.last[bucket]
                        && label_open.is_none_or(|open| self.stack[index].source.start > open)
                    {
                        newest = newest.max(Some(index));
                    }
                }
                newest
            };
            let Some(index) = index else { break };
            let inherited = index < self.header_floor;
            // Without an old opener this close-only run would be literal and
            // leave the local stack intact. No replay, shadow stack or object
            // rollback is needed for this independently verifiable subset.
            let isolate = inherited
                && !self.header_crossed
                && !can_open
                && self.stack.len() == self.header_floor;
            self.header_crossed |= inherited && !isolate;
            let open = self.stack[index].source;
            let used = if run.byte == b'~' {
                close.len()
            } else {
                2.min(open.len()).min(close.len())
            };
            while self.stack.len() > index + 1 {
                let delimiter = self.stack.pop().unwrap();
                self.email_popped(delimiter);
                self.last[delimiter.kind] = delimiter.previous;
                effects.work.delimiter_steps += 1;
            }
            self.stack[index].source.end -= used;
            effects.work.delimiter_steps += 1;
            if self.stack[index].source.is_empty() {
                let delimiter = self.stack.pop().unwrap();
                self.email_popped(delimiter);
                self.last[delimiter.kind] = delimiter.previous;
            }
            self.header_floor = self.header_floor.min(self.stack.len());
            let pair = EmphasisPair {
                open: Span::new(open.end - used, open.end),
                close: Span::new(close.start, close.start + used),
                style: if run.byte == b'~' {
                    Style::STRIKE
                } else if used == 2 {
                    Style::STRONG
                } else {
                    Style::EMPHASIS
                },
            };
            if isolate {
                self.header_pairs.push(pair);
            } else {
                self.email_pair(&pair);
                pair.commit(effects);
            }
            close.start += used;
        }
        if can_open && !close.is_empty() {
            let index = self.stack.len();
            self.stack.push(Delimiter {
                email: None,
                kind,
                source: close,
                previous: self.last[kind],
            });
            self.last[kind] = Some(index);
            effects.work.delimiter_steps += 1;
        }
        self.publish_ready_emails(effects);
    }

    /// Returns false when the current byte must be processed by the normal lexer.
    fn tail_byte(
        &mut self,
        byte: u8,
        pos: usize,
        origin: Origin,
        effects: &mut Effects<'_>,
    ) -> bool {
        match self.tail.take().unwrap() {
            Tail::After(candidate) => match byte {
                b'(' => {
                    self.tail = Some(Tail::Destination {
                        candidate,
                        parser: Destination::new(pos + 1),
                        title_gaps: Vec::new(),
                        atoms: effects.publishing.as_ref().map(|_| AtomTrace::default()),
                    })
                }
                b'[' => {
                    self.tail = Some(Tail::Label {
                        candidate,
                        start: pos,
                        escaped: false,
                    })
                }
                _ => {
                    self.seal_candidate(candidate, effects);
                    return false;
                }
            },
            Tail::Destination {
                candidate,
                mut parser,
                title_gaps,
                mut atoms,
            } => match if self.root_paragraph {
                let progress = parser.root_byte(byte, pos);
                self.root_separator_seen |= byte == b'\n' && matches!(progress, Progress::Pending);
                progress
            } else if self.container_paragraph {
                let progress = parser.container_byte(byte, pos);
                self.root_separator_seen |= byte == b'\n' && matches!(progress, Progress::Pending);
                progress
            } else {
                parser.byte(byte, pos)
            } {
                Progress::Invalid => {
                    if self.header_link.is_some() {
                        self.flush_header(effects);
                        self.header_crossed = true;
                    } else {
                        self.seal_candidate(candidate, effects);
                    }
                    if let Some(atoms) = atoms {
                        atoms.recover(effects);
                    }
                    return false;
                }
                Progress::Complete { destination, title } => {
                    let source = Span::new(parser.start - 1, pos + 1);
                    if self.header_link.is_some() {
                        self.header_link_suffix = Some(HeaderLinkSuffix::Destination {
                            source,
                            destination,
                            title,
                            title_gaps,
                        });
                    } else {
                        let mut candidate = candidate;
                        self.end_label(&mut candidate, effects);
                        self.complete_candidate(
                            candidate,
                            Some(LinkTarget::Direct(destination)),
                            Some(source),
                            title,
                            title_gaps,
                            effects,
                        );
                    }
                }
                Progress::Pending => {
                    if matches!(origin, Origin::Raw)
                        && let Some(atoms) = &mut atoms
                    {
                        atoms.push(pos, effects.work);
                    }
                    self.tail = Some(Tail::Destination {
                        candidate,
                        parser,
                        title_gaps,
                        atoms,
                    })
                }
            },
            Tail::Label {
                candidate,
                start,
                escaped,
            } => {
                if pos - start - 1 > MAX_LABEL_BYTES
                    || matches!(byte, b'\n' | b'\r')
                    || (byte == b'[' && !escaped)
                {
                    if self.header_link.is_some() {
                        self.flush_header(effects);
                        self.header_crossed = true;
                    } else {
                        self.seal_candidate(candidate, effects);
                    }
                    return false;
                }
                if byte == b']' && !escaped {
                    let source = Span::new(start, pos + 1);
                    if self.header_link.is_some() {
                        self.header_link_suffix = Some(HeaderLinkSuffix::Label(source));
                    } else {
                        if let Some(fallback) = self.reference_tail(candidate, source, effects) {
                            self.tail = Some(Tail::After(fallback));
                        }
                    }
                } else {
                    // Track parity while consuming the suffix once. Keep its
                    // raw spelling for symbol matching, including escapes.
                    self.tail = Some(Tail::Label {
                        candidate,
                        start,
                        escaped: !escaped && byte == b'\\',
                    });
                }
            }
        }
        true
    }

    fn style_suffix(&mut self, source: Span, owner: Option<LinkId>, effects: &mut Effects<'_>) {
        let (Some(key), Some(id), Some(publishing)) =
            (self.style_scope, owner, &mut effects.publishing)
        else {
            return;
        };
        publishing.styles.enter(key, source.start, effects.work);
        let mut work = Work::default();
        let mut local = Inline {
            style_scope: Some(key),
            previous: Some(b'['),
            previous_scalar: Some('['),
            ..Inline::default()
        };
        let mut sink = Effects {
            source: effects.source,
            work: &mut work,
            publishing: None,
            style_sink: Some((publishing.styles, key, Some(id))),
        };
        local.feed(Span::new(source.start + 1, source.end), &mut sink);
        local.finish(&mut sink);
        publishing
            .styles
            .leave(key, source.start, source.end - 1, effects.work);
        if let Some(changed) = publishing.styles.conditional_source(key) {
            publishing.mark(changed, Style::default(), None);
        }
        effects.work.style_events += work.style_events;
        effects.work.style_scope_steps += work.style_scope_steps;
        effects.work.style_suffix_bytes += work.inline_bytes + work.code_fallback_bytes;
        let mut observer = Eligibility {
            inline: Box::new(local),
            work,
            ..Eligibility::default()
        };
        let mut accounted = Work::default();
        observer.account(&mut accounted);
        effects.work.style_suffix_steps += accounted.root_eligibility_steps;
    }

    fn reference_tail(
        &mut self,
        mut candidate: Candidate,
        source: Span,
        effects: &mut Effects<'_>,
    ) -> Option<Candidate> {
        let symbol = if source.len() == 2 {
            candidate.reference
        } else {
            effects.reference(&effects.source[source.start + 1..source.end - 1])
        };
        if let Some(symbol) = symbol {
            self.end_label(&mut candidate, effects);
            candidate = self.complete_candidate(
                candidate,
                symbol.target(),
                Some(source),
                None,
                Vec::new(),
                effects,
            );
            if source.len() != 2 {
                self.style_suffix(source, candidate.link, effects);
            }
            let gate = candidate
                .gate
                .unwrap_or_else(|| Gate::unless(candidate.blockers.root));
            // A full suffix is also an ordinary label on the complementary
            // branch. Following labels can alternate these two interpretations
            // indefinitely, with one new candidate per suffix, never replay.
            // Grammar eligibility has no public IDs/guard nodes. Keep this
            // syntactic conditional bit separate from its effect-only root.
            let conditional = candidate.gate.is_some()
                || (candidate.blockers.potential && !candidate.blockers.known);
            if candidate.image.is_none() && conditional && source.len() != 2 {
                let mut fallback = Candidate::bare(source.start, source.end - 1, None);
                fallback.reference = Some(symbol);
                fallback.alternate = true;
                fallback.alternative_parent = candidate.link;
                fallback.gate = Some(if gate.positive {
                    Gate::unless(gate.root)
                } else {
                    Gate::only(gate.root)
                });
                fallback = self.complete_candidate(
                    fallback,
                    symbol.target(),
                    None,
                    None,
                    Vec::new(),
                    effects,
                );
                return Some(fallback);
            }
        } else {
            self.seal_candidate(candidate, effects);
        }
        None
    }

    fn seal_candidate(&mut self, candidate: Candidate, effects: &mut Effects<'_>) {
        if self.header_link.is_none() && !candidate.accepted {
            self.fail_candidate(candidate, effects);
        }
        effects.seal_link(candidate.link);
    }

    fn recover_code(&mut self, effects: &mut Effects<'_>) {
        let checkpoint = self
            .code_fallback
            .checkpoint
            .expect("primary code checkpoint");
        let events = std::mem::take(&mut self.code_fallback.events);
        let first = events.partition_point(|event| {
            effects.work.code_fallback_comparisons += 1;
            event.end() <= checkpoint.open.end
        });
        self.code_recovery = Some(self.code_fallback.index(effects));
        self.code = None;
        self.run = None;
        self.label.truncate(checkpoint.label_len);
        self.label_overflow = checkpoint.label_overflow;
        self.previous = Some(b'`');
        self.previous_scalar = Some('`');
        self.spaces = None;
        self.whitespace = None;
        self.carriage = false;
        self.line_break = None;
        for event in events[first..].iter().copied() {
            effects.work.code_fallback_steps += 1;
            match event {
                CodeEvent::Raw(span) => self.feed(
                    Span::new(span.start.max(checkpoint.open.end), span.end),
                    effects,
                ),
                CodeEvent::Virtual { at, columns } => self.virtual_spaces(at, columns, effects),
                CodeEvent::Prefix(span) => self.component_prefix(span, effects.work),
            }
        }
        self.flush_bare(None, effects);
        self.finish_run(None, effects);
        debug_assert!(self.code.is_none(), "indexed replay code must close");
        self.code_recovery = None;
        self.code_fallback.events = events;
    }

    pub fn finish(&mut self, effects: &mut Effects<'_>) {
        self.flush_bare(None, effects);
        self.cancel_header(effects);
        self.finish_run(None, effects);
        if self.code.is_some() {
            self.recover_code(effects);
        }
        self.raw_content = None;
        self.raw_cr = None;
        if let Some(tail) = self.tail.take() {
            let (candidate, atoms) = match tail {
                Tail::After(c) | Tail::Label { candidate: c, .. } => (c, None),
                Tail::Destination {
                    candidate, atoms, ..
                } => (candidate, atoms),
            };
            self.seal_candidate(candidate, effects);
            if let Some(atoms) = atoms {
                atoms.recover(effects);
            }
        }
        self.finish_run(None, effects);
        while let Some(bracket) = self.bracket.take() {
            if let (Some(key), Some(p)) = (self.style_scope, &mut effects.publishing) {
                let end = p.styles.scope_end(key);
                p.styles.leave(key, bracket.open, end, effects.work);
            }
            self.bracket = self.brackets.pop();
            let mut candidate = Candidate::bare(bracket.open, bracket.open, bracket.image);
            candidate.journal = bracket.journal;
            candidate.children = bracket.children;
            self.fail_candidate(candidate, effects);
            effects.work.link_label_steps += 1;
        }
        self.finish_emails(usize::MAX, effects);
        self.email_pending.clear();
        self.email_ready.clear();
        self.bare_markers.clear();
        self.header_email_pending = 0;
        effects.work.delimiter_steps += self.stack.len() as u64;
        self.stack.clear();
        self.last = [None; 14];
        self.code = None;
        self.math = None;
        self.footnote = None;
        self.bracket = None;
        self.bang = None;
        self.tail = None;
        self.escape = None;
        self.previous = None;
        self.previous_scalar = None;
        self.spaces = None;
        self.whitespace = None;
        self.carriage = false;
        self.line_break = None;
        self.autolink = None;
        self.entity = None;
        self.root_paragraph = false;
        self.container_paragraph = false;
        self.paragraph_key = None;
        self.root_separator_seen = false;
        self.code_fallback.clear();
        self.code_fallback.events.clear();
        self.style_scope = None;
        self.account_labels(effects);
    }
}

#[cfg(test)]
mod tests {
    use crate::{Document, LinkId};

    use super::*;

    pub(super) fn same(publisher: &Inline, observer: &Inline, event: &str) {
        // Debug derives cover every core field: stack categories/previous links,
        // all 14 heads, raw/Unicode/opaque/label summaries and every header field.
        // The publisher may be configured for root separators; these old-mode
        // differential inputs have not admitted one. Compare all grammar state.
        assert!(!publisher.root_separator_seen);
        assert!(!observer.root_paragraph);
        fn syntax(inline: &Inline) -> String {
            // Compare reachable journal contents by owner, separately from
            // reusable free-list slot identities/allocation history.
            let mut state = format!("{inline:?}");
            let start = state.find(", label_delimiters: ").unwrap();
            let end = state.find(", header_label_delimiters: ").unwrap();
            state.replace_range(start..end, ", label_delimiters: <arena>");
            let start = state.find(", header_label_tail: ").unwrap();
            let end = state[start..].find(", bang: ").unwrap() + start;
            state.replace_range(
                start..end,
                &format!(
                    ", header_label_tail: {:?}",
                    inline.label_delimiters.value(inline.header_label_tail)
                ),
            );
            let start = state.find(", bracket_allocations: ").unwrap();
            let end = state[start..].find(", label: ").unwrap() + start;
            state.replace_range(start..end, ", bracket_allocations: <capacity>");
            let mut journals = Vec::new();
            for bracket in inline.brackets.iter().chain(inline.bracket.iter()) {
                journals.push((
                    bracket.open,
                    inline.label_delimiters.snapshot(bracket.journal),
                ));
            }
            if let Some(candidate) = inline.header_link {
                journals.push((
                    candidate.open,
                    inline.label_delimiters.snapshot(candidate.journal),
                ));
            }
            if let Some(tail) = &inline.tail {
                let c = match tail {
                    Tail::After(c)
                    | Tail::Destination { candidate: c, .. }
                    | Tail::Label { candidate: c, .. } => c,
                };
                journals.push((c.open, inline.label_delimiters.snapshot(c.journal)));
            }
            state.push_str(&format!(" journals={journals:?}"));
            // finish retains bounded label scratch for allocation reuse. Its
            // dormant bytes/overflow are not grammar state: the next '[' resets
            // both. Compare them in full whenever any label consumer is live.
            if inline.bracket.is_none() && inline.tail.is_none() && inline.header_link.is_none() {
                let start = state.find(", label: ").unwrap();
                let end = state.find(", label_delimiters: ").unwrap();
                state.replace_range(start..end, ", label: <dormant scratch>");
            }
            let state = state
                .replacen("root_paragraph: true", "root_paragraph: false", 1)
                .replacen("container_paragraph: true", "container_paragraph: false", 1);
            let start = state.find("paragraph_key: ").unwrap();
            let end = state[start..].find(", root_separator_seen:").unwrap() + start;
            let mut state = state;
            state.replace_range(start..end, "paragraph_key: None");
            state
        }
        assert_eq!(syntax(publisher), syntax(observer), "{event}");
        assert_eq!(
            publisher.can_isolate_line(),
            observer.can_isolate_line(),
            "{event}"
        );
        assert_eq!(
            publisher.header_crossed(),
            observer.header_crossed(),
            "{event}"
        );
    }

    struct Differential {
        doc: Document,
        observer: Inline,
        work: Work,
        update: Update,
    }

    impl Differential {
        fn new(source: &str) -> Self {
            let mut doc = Document::new();
            doc.source = source.to_owned();
            Self {
                doc,
                observer: Inline::default(),
                work: Work::default(),
                update: Update::default(),
            }
        }

        fn compare(&self, event: &str) {
            same(&self.doc.inline, &self.observer, event);
            let mut publishing = self.doc.work;
            publishing.link_fallback_bytes = 0;
            publishing.link_fallback_steps = 0;
            // Publication indexes are absent from the grammar-only observer.
            publishing.style_events = 0;
            publishing.style_scope_steps = 0;
            publishing.style_index_steps = 0;
            publishing.style_suffix_bytes = 0;
            publishing.style_suffix_steps = 0;
            publishing.header_effect_steps = 0;
            assert_eq!(publishing, self.work, "work after {event}");
            assert_eq!(self.work.tree_visits, 0);
        }

        fn feed(&mut self, span: Span) {
            let mut publishing = std::mem::take(&mut self.doc.inline);
            publishing.feed(span, &mut self.doc.effects(&mut self.update));
            self.doc.inline = publishing;
            self.observer.feed(
                span,
                &mut Effects::observing(&self.doc.source, &mut self.work),
            );
            self.compare(&format!("feed {span:?}"));
        }

        fn spaces(&mut self, at: usize, count: u8) {
            let mut publishing = std::mem::take(&mut self.doc.inline);
            publishing.virtual_spaces(at, count, &mut self.doc.effects(&mut self.update));
            self.doc.inline = publishing;
            self.observer.virtual_spaces(
                at,
                count,
                &mut Effects::observing(&self.doc.source, &mut self.work),
            );
            self.compare("virtual spaces");
        }

        fn isolate(&mut self) {
            assert!(self.doc.inline.can_isolate_line());
            let key = self.doc.inline.style_scope_key().unwrap();
            self.doc.annotations.begin_line();
            self.doc.inline_styles.begin_line(key);
            self.doc.link_order.begin_line();
            self.doc.footnotes.begin_reference_line();
            self.doc.header_effects.begin(
                self.doc.links.len(),
                self.doc.maths.len(),
                self.doc.display.entries_len(),
                self.doc.math_gaps.len(),
            );
            let published_break = self.doc.inline.isolate_line();
            let observed_break = self.observer.isolate_line();
            assert_eq!(
                format!("{published_break:?}"),
                format!("{observed_break:?}")
            );
            self.compare("isolate");
        }

        fn cancel(&mut self) {
            let mut publishing = std::mem::take(&mut self.doc.inline);
            publishing.cancel_header(&mut self.doc.effects(&mut self.update));
            self.doc.inline = publishing;
            self.observer
                .cancel_header(&mut Effects::observing(&self.doc.source, &mut self.work));
            self.compare("cancel");
        }

        fn confirm(&mut self) {
            let mut publishing = std::mem::take(&mut self.doc.inline);
            let reuse = publishing.confirm_header(&mut self.doc.effects(&mut self.update));
            self.doc.inline = publishing;
            assert_eq!(
                reuse,
                self.observer
                    .confirm_header(&mut Effects::observing(&self.doc.source, &mut self.work))
            );
            self.compare("confirm");
        }

        fn finish(&mut self) {
            let mut publishing = std::mem::take(&mut self.doc.inline);
            publishing.finish(&mut self.doc.effects(&mut self.update));
            self.doc.inline = publishing;
            self.observer
                .finish(&mut Effects::observing(&self.doc.source, &mut self.work));
            self.compare("finish");
        }
    }

    #[test]
    fn eligibility_backend_matches_every_syntax_event_and_acceptance_branch() {
        let mut sources: Vec<String> = [
            "*前🙂 [*签* _中_ unmatched*][] tail* [a][ Straße ] ![图](<my url> 'title')",
            "[ ][] [\u{2003}][] [a][ ] [a][bad[nested] [a][escaped\\]]",
            "[a](bad title) [b](unfinished [c][broken\n *outside [literal*",
            "[a](u\n 't') [a](u\n 'a&amp;'x) [a](\n*u\nend*and",
            "a***b**c* _中_，~~🙂~~ \\* &amp; &#x1f600; &bad; <https://x> <a@b.co> [^签]",
            "`x``a```b`` y `ok` $x\\$y$ $$中$$ $unfinished",
            "a  \r\nb\\\n c\t\n[<https://x> [replacement] [_x*]",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();
        for label in ["x".repeat(999), "x".repeat(1000), "İ".repeat(499)] {
            sources.push(format!(
                "*outside [{label}][] [{label}][valid] [{label}](u) tail*"
            ));
        }
        sources.push(format!("`x{}", "``body```other`` ".repeat(1024)));
        for source in sources {
            let mut differential = Differential::new(&source);
            for (pos, c) in source.char_indices() {
                differential.feed(Span::new(pos, pos + c.len_utf8()));
            }
            differential.finish();
        }
    }

    #[test]
    fn observer_keeps_conditional_suffix_branching_without_published_ids() {
        for source in [
            "[a [r]][x][y](v) ",
            "[a [r]][x][y][z] ",
            "[outer [a [r]][x][y]](u) ",
        ] {
            let mut d = Differential::new(source);
            for (at, ch) in source.char_indices() {
                d.feed(Span::new(at, at + ch.len_utf8()));
            }
            d.finish();
        }
    }

    #[test]
    fn eligibility_backend_matches_all_header_state_and_head_transitions() {
        for previous in [
            "*old\n",
            "**old\n",
            "_old\n",
            "~~old\n",
            "`old``a\n",
            "$old\n",
            "[old *a\n",
            "[old\n",
            "plain\n",
        ] {
            for row in [
                "end* ",
                "end*and ",
                "end** ",
                "end_ ",
                "end~~ ",
                "end` ",
                "end`` ",
                "end$ ",
                "end][key] ",
                "end][] ",
                "end](u 't') ",
                "end](bad title) ",
                "local *x* [b] &amp; ",
                "*",
                "end`[new] ",
            ] {
                for confirm in [false, true] {
                    let source = format!("{previous}{row}|b");
                    let mut d = Differential::new(&source);
                    d.feed(Span::new(0, previous.len()));
                    d.isolate();
                    for (pos, c) in row.char_indices() {
                        d.feed(Span::new(
                            previous.len() + pos,
                            previous.len() + pos + c.len_utf8(),
                        ));
                    }
                    // Pending runs next to pipe are deliberately not flushed.
                    if confirm && !d.doc.inline.header_crossed() {
                        d.confirm();
                    } else {
                        d.cancel();
                    }
                    d.finish();
                }
            }
        }
        // Known same-scope omitted prefixes and split-tab semantic payloads
        // exercise math gaps/code mappings without giving the sink any stores.
        for source in ["$a\n>\tb$ ", "`a\n>\tb` "] {
            let mut d = Differential::new(source);
            d.feed(Span::new(0, 3));
            d.spaces(4, 3);
            d.feed(Span::new(5, source.len()));
            d.finish();
        }
    }

    #[test]
    fn root_observer_lifecycle_mirrors_real_payload_without_prefix_replay() {
        for prefix in [
            "root *old\n",
            "`old``a\n",
            "$old\n",
            "[old *x\n",
            "[r\n",
            "[r]: /u 'pending\n",
        ] {
            for ending in [
                "|b\n-| -\n",
                "|b\nwrong\n",
                "\n",
                "|b\n===\n",
                "|b\n> new\n",
                "",
            ] {
                let source = format!("{prefix}end*and {}{ending}", "x".repeat(1100));
                let mut doc = Document::new();
                for c in source.chars() {
                    doc.append(c.encode_utf8(&mut [0; 4])).unwrap();
                    if let Some(root) = &doc.root_eligibility {
                        same(&doc.inline, &root.inline, "actual root event");
                        assert!(root.cursor.unwrap() <= doc.source.len());
                        assert_eq!(root.work, Work::default(), "event counters drained once");
                    }
                }
                doc.finish();
                assert!(doc.root_eligibility.is_none());
                assert!(doc.work().root_eligibility_bytes <= doc.source.len() as u64);
            }
        }
        let mut ledger = Document::new();
        ledger.append("[open\n").unwrap();
        assert!(ledger.root_eligibility.is_none());
        ledger.append("failed\n").unwrap();
        ledger.finish();
        assert!(ledger.work().root_eligibility_bytes > 0);
    }

    #[test]
    fn container_observer_keeps_first_leaf_and_never_revives_ended_scopes() {
        fn key(doc: &Document) -> (crate::BlockId, crate::ContainerId) {
            let active = doc.container_eligibility.as_ref().unwrap();
            (active.first, active.scope)
        }
        for ending in ["\n", "\r\n"] {
            let mut doc = Document::new();
            doc.append(&format!("> **a{ending}")).unwrap();
            let first = key(&doc);
            doc.append(&format!("> b**{ending}")).unwrap();
            assert_eq!(key(&doc), first);
            assert_ne!(doc.current.unwrap(), first.0);
            doc.append(&format!(">{ending}")).unwrap();
            assert!(doc.container_eligibility.is_none());
            doc.append(&format!("> *new{ending}")).unwrap();
            let next = key(&doc);
            assert_eq!(next.1, first.1);
            assert_ne!(next.0, first.0);
            doc.append(&format!("> > # child{ending}")).unwrap();
            assert!(doc.container_eligibility.is_none());
            doc.append(&format!("> back*{ending}")).unwrap();
            assert_eq!(key(&doc).1, first.1);
            assert_ne!(key(&doc).0, next.0);
            doc.finish();
            assert!(doc.container_eligibility.is_none());

            for boundary in ["# head", "===", "---", "```", "$$", "[^f]: text"] {
                let mut doc = Document::new();
                doc.append(&format!("> ordinary{ending}")).unwrap();
                doc.append(&format!("> {boundary}{ending}")).unwrap();
                assert!(doc.container_eligibility.is_none(), "{boundary}");
            }
        }
        let mut doc = Document::new();
        doc.append("- first\n").unwrap();
        let first = key(&doc);
        doc.append("- sibling\n").unwrap();
        assert_ne!(key(&doc).0, first.0);
        assert_ne!(key(&doc).1, first.1);
        doc.append("  > nested\n").unwrap();
        assert_ne!(key(&doc).1, first.1);
        doc.append("\nroot\n").unwrap();
        assert!(doc.container_eligibility.is_none());
        assert!(doc.root_eligibility.is_some());
    }

    #[test]
    fn container_observer_waits_for_definition_owner_and_deduplicates_virtual_groups() {
        for prefix in [">\t", " >\t", "   >\t"] {
            let mut doc = Document::new();
            doc.append(&format!("{prefix}[r\n")).unwrap();
            let observer = &doc.container_eligibility.as_ref().unwrap().observer;
            assert!(observer.cursor.is_none());
            assert!(observer.last_virtual_tab.is_none());
            assert_eq!(doc.work.container_eligibility_bytes, 0);
            doc.append("> label]: /u\n").unwrap();
            assert!(doc.container_eligibility.is_none());
            assert_eq!(doc.work.container_eligibility_bytes, 0);

            let mut fallback = Document::new();
            fallback.append(&format!("{prefix}[r\n")).unwrap();
            fallback.append("> bad]\n").unwrap();
            assert!(fallback.work.container_eligibility_bytes > 0);
            fallback.finish();
        }
        for columns in 1..=3 {
            let mut observer = Eligibility::container();
            let mut total = Work::default();
            let source = "\ta\n\tb";
            // A virtual-only event cannot consume or advance the raw cursor.
            observer.virtual_spaces(source, 0, columns, &mut total);
            assert!(observer.cursor.is_none());
            observer.virtual_spaces(source, 0, columns, &mut total);
            assert_eq!(total.container_eligibility_bytes, u64::from(columns));
            observer.feed(source, Span::new(1, 3), 0, &mut total);
            observer.virtual_spaces(source, 3, columns, &mut total);
            observer.feed(source, Span::new(4, 5), 0, &mut total);
            let once = total;
            observer.virtual_spaces(source, 3, columns, &mut total);
            observer.feed(source, Span::new(4, 5), 0, &mut total);
            assert_eq!(total, once);
            assert_eq!(
                total.container_eligibility_bytes,
                3 + 2 * u64::from(columns)
            );
            assert_eq!(total.inline_bytes, 0);
            assert_eq!(total.root_eligibility_bytes, 0);
        }
    }

    #[test]
    fn actual_container_observer_mirrors_header_reuse_cancel_finish_and_all_live_state() {
        for prefix in ["> ", ">\t", " >\t", "   >\t", "- "] {
            let next = if prefix == "- " { "  " } else { prefix };
            for prior in [
                "*old", "**old", "_old", "~~old", "`old``a", "$old", "[old *a", "[old", "plain",
            ] {
                for row in [
                    "end* ",
                    "end*and ",
                    "end** ",
                    "end_ ",
                    "end~~ ",
                    "end` ",
                    "end`` ",
                    "end$ ",
                    "end][key] ",
                    "end][] ",
                    "end](u 't') ",
                    "end](bad title) ",
                    "*",
                    "end`[new] ",
                    "中🙂 &amp; \\* _a_ ~~b~~ ",
                ] {
                    for decision in ["-|-", "wrong"] {
                        for ending in ["\n", "\r\n"] {
                            let source = format!(
                                "{prefix}{prior}{ending}{next}{row}{} |b{ending}{next}{decision}{ending}",
                                "x".repeat(1025)
                            );
                            // These use the real producers, not a second event
                            // grammar. Each mirrored event compares every live
                            // Inline field, including all Destination state.
                            let mut stream = crate::Utf8Document::new();
                            for chunk in source.as_bytes().chunks(127) {
                                stream.append(chunk).unwrap();
                            }
                            stream.finish();
                            assert!(stream.document().container_eligibility.is_none());
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn container_divergence_keeps_old_observer_semantics_and_exact_deferred_key() {
        for prefix in ["> ", ">\t", "- "] {
            let next = if prefix == "- " { "  " } else { prefix };
            for (prior, row) in [
                ("*prior [a](u", "end*and"),
                ("_prior [a](u", "end_and"),
                ("~~prior [a](u", "end~~and"),
                ("*prior [a](u", "end* "),
                ("[a](u", "end"),
                ("[a](u", "'t') "),
                ("[a](u", "'t'x) "),
            ] {
                let head = format!("{prefix}{prior}\n");
                let source = format!("{head}{next}{row}{}", "x".repeat(1100));
                let mut doc = Document::new();
                doc.append(&head).unwrap();
                assert!(doc.inline.root_separator_seen());
                let key = doc.inline.paragraph_key().unwrap();
                doc.append(&source[head.len()..]).unwrap();
                assert_eq!(doc.inline.paragraph_key(), Some(key));
                let active = doc.container_eligibility.as_ref().unwrap();
                assert_eq!(active.observer.key(), Some(key));
                // Independently run old single-line syntax over the producer's
                // known spans. No new publisher effects/state enter this model.
                let mut old = Inline::default();
                let mut work = Work::default();
                let virtuals = u8::from(prefix == ">\t") * 2;
                if virtuals > 0 {
                    old.virtual_spaces(1, virtuals, &mut Effects::observing(&source, &mut work));
                }
                old.feed(
                    Span::new(prefix.len(), head.len()),
                    &mut Effects::observing(&source, &mut work),
                );
                assert!(old.can_isolate_line());
                old.isolate_line();
                if virtuals > 0 {
                    old.virtual_spaces(
                        head.len() + 1,
                        virtuals,
                        &mut Effects::observing(&source, &mut work),
                    );
                }
                old.feed(
                    Span::new(head.len() + next.len(), source.len()),
                    &mut Effects::observing(&source, &mut work),
                );
                if old.header_crossed() {
                    old.cancel_header(&mut Effects::observing(&source, &mut work));
                    assert!(doc.streaming_header.is_none());
                } else {
                    assert!(matches!(doc.streaming_header.as_ref().unwrap().header.mode,
                        crate::table::HeaderMode::Deferred(saved) if saved == key));
                }
                same(
                    &old,
                    &active.observer.inline,
                    "old observer after publisher divergence",
                );
                assert!(doc.inline.root_separator_seen());
                doc.finish();
            }
        }
    }

    #[test]
    fn label_summaries_track_runs_reuse_storage_and_wait_for_confirmation() {
        let mut doc = Document::new();
        doc.append(&format!("p {} *前 [签* _中_ ", "x".repeat(1024)))
            .unwrap();
        assert_eq!(doc.inline.label_delimiters.len(), 3);
        let capacity = doc.inline.label_delimiters.capacity();
        let before = doc.work();
        for _ in 0..1000 {
            doc.append("中😀body").unwrap();
            assert_eq!(doc.inline.label_delimiters.len(), 3);
            assert_eq!(doc.inline.label_delimiters.capacity(), capacity);
        }
        assert_eq!(doc.work().delimiter_steps, before.delimiter_steps);
        assert_eq!(
            doc.work().delimiter_search_steps,
            before.delimiter_search_steps
        );
        assert_eq!(doc.work().tree_visits, before.tree_visits);
        doc.append("](u) ").unwrap();
        assert!(doc.inline.label_delimiters.is_empty());
        assert_eq!(doc.inline.label_delimiters.capacity(), capacity);
        for _ in 0..1000 {
            doc.append("[签* _中_](u) ").unwrap();
            assert!(doc.inline.label_delimiters.is_empty());
            assert_eq!(doc.inline.label_delimiters.capacity(), capacity);
        }
        let mut unfinished = Document::new();
        unfinished
            .append(&format!("p {} *前 [签* ", "x".repeat(1024)))
            .unwrap();
        let at = unfinished.source().find('签').unwrap();
        assert_eq!(
            unfinished
                .runs_in(crate::Span::new(at, at + 3))
                .unwrap()
                .next()
                .unwrap()
                .emphasis_depth,
            0
        );
        unfinished.finish();
        assert_eq!(
            unfinished
                .runs_in(crate::Span::new(at, at + 3))
                .unwrap()
                .next()
                .unwrap()
                .emphasis_depth,
            1
        );
        assert!(unfinished.inline.label_delimiters.is_empty());
    }

    #[test]
    fn nested_inherited_summaries_resolve_once_and_retain_the_line_floor() {
        for count in [64, 1024, 4096] {
            for replacement in ["[new](u)", "<https://x>"] {
                for ending in ["-|-\n", "wrong\n", ""] {
                    let mut doc = Document::new();
                    doc.append(&format!("p [old {}*inside\n", "_a_ ".repeat(count)))
                        .unwrap();
                    doc.append(&format!("_cell_ {} ", "x".repeat(1100)))
                        .unwrap();
                    assert_eq!(doc.inline.header_label_delimiters, 2 * count + 1);
                    assert_eq!(doc.inline.label_delimiters.len(), 2 * count + 3);
                    let capacity = doc.inline.label_delimiters.capacity();
                    let before = doc.work();
                    for _ in 0..100 {
                        doc.append("body").unwrap();
                    }
                    assert_eq!(doc.inline.label_delimiters.len(), 2 * count + 3);
                    assert_eq!(doc.inline.label_delimiters.capacity(), capacity);
                    assert_eq!(doc.work().delimiter_steps, before.delimiter_steps);
                    assert_eq!(
                        doc.work().delimiter_search_steps,
                        before.delimiter_search_steps
                    );

                    doc.append(&format!(" {replacement} end* ")).unwrap();
                    // A child object no longer replaces an inherited bracket.
                    // Its parent's summaries wait for success/failure or EOF.
                    assert_eq!(doc.inline.label_delimiters.len(), 2 * count + 4);
                    assert_eq!(doc.inline.label_delimiters.capacity(), capacity);
                    assert_eq!(doc.inline.header_label_delimiters, 2 * count + 1);
                    assert_eq!(doc.inline.header_pairs.len(), 0);
                    assert!(!doc.inline.header_crossed);
                    // One resolution per summary, one push/pop per local pair,
                    // and constant work for the replacement and crossing pair.
                    assert!(
                        doc.work().delimiter_steps - before.delimiter_steps
                            <= 4 * count as u64 + 24
                    );
                    let resolved = doc.work();
                    for _ in 0..100 {
                        doc.append("body").unwrap();
                    }
                    assert_eq!(doc.work().delimiter_steps, resolved.delimiter_steps);
                    assert_eq!(
                        doc.work().delimiter_search_steps,
                        resolved.delimiter_search_steps
                    );
                    doc.append(&format!("|b\n{ending}")).unwrap();
                    doc.finish();
                    assert_eq!(doc.tables().len(), usize::from(ending == "-|-\n"));
                    let work = doc.work();
                    let bytes = doc.source().len() as u64;
                    assert!(work.inline_bytes <= bytes + work.header_replay_bytes);
                    assert!(work.delimiter_steps <= 3 * bytes);
                    assert!(work.delimiter_search_steps <= 6 * work.delimiter_steps);
                    assert!(work.tree_visits <= 24 * bytes * usize::BITS as u64);
                    assert!(doc.annotation_arena_stats().nodes <= 4 * bytes as usize);
                }
            }
        }
    }

    #[test]
    fn code_trace_keeps_producer_payload_virtuals_and_prefixes_once() {
        let lead = "p".repeat(1100);
        let mut doc = Document::new();
        doc.append(&format!("- {lead} ` a")).unwrap();
        let capacity = doc.inline.code_fallback.events.capacity();
        for _ in 0..1000 {
            doc.append("b").unwrap();
            assert_eq!(doc.inline.code_fallback.events.capacity(), capacity);
        }
        doc.append("\n\tc &amp;\n").unwrap();
        let mut logical = String::new();
        let mut virtuals = 0;
        let mut prefixes = 0;
        for event in &doc.inline.code_fallback.events {
            match *event {
                CodeEvent::Raw(span) => logical.push_str(&doc.source[span.range()]),
                CodeEvent::Virtual { at, columns } => {
                    assert_eq!(&doc.source[at..at + 1], "\t");
                    assert_eq!(columns, 2);
                    logical.push_str(&" ".repeat(usize::from(columns)));
                    virtuals += 1;
                }
                CodeEvent::Prefix(_) => prefixes += 1,
            }
        }
        assert_eq!(
            logical,
            format!("{lead} ` a{}\n  c &amp;\n", "b".repeat(1000))
        );
        assert_eq!(virtuals, 1);
        assert!(prefixes >= 1);
        assert_eq!(doc.display.space_mappings().len(), 1);
        let expected = format!(" a{}\n  c &amp;\n", "b".repeat(1000));
        doc.finish();
        assert_eq!(doc.work.code_fallback_bytes, expected.len() as u64);
        assert_eq!(doc.display.space_mappings().len(), 1);
        assert!(doc.inline.code_fallback.events.is_empty());
        assert!(doc.inline.code_fallback.checkpoint.is_none());
        assert!(doc.inline.code_recovery.is_none());
        let work = doc.work();
        doc.finish();
        assert_eq!(doc.work(), work);
    }

    #[test]
    fn failed_code_journal_storage_tracks_runs_and_is_reused() {
        let mut doc = Document::new();
        doc.append(&format!("`x{}", "``x".repeat(1000))).unwrap();
        let journal = &doc.inline.code_fallback;
        assert_eq!(journal.runs.len(), 1000);
        let capacity = journal.runs.capacity();
        assert!(capacity <= 2 * journal.runs.len());
        for _ in 0..1000 {
            doc.append("body").unwrap();
            assert_eq!(doc.inline.code_fallback.runs.capacity(), capacity);
        }
        doc.append("` end\n\n").unwrap();
        assert!(doc.inline.code_fallback.runs.is_empty());
        assert!(doc.inline.code_fallback.boundary.is_none());
        assert_eq!(doc.inline.code_fallback.runs.capacity(), capacity);
        doc.append(&format!("`x{}", "``x".repeat(1000))).unwrap();
        assert_eq!(doc.inline.code_fallback.runs.len(), 1000);
        assert_eq!(doc.inline.code_fallback.runs.capacity(), capacity);
    }

    #[test]
    fn isolated_link_suffixes_keep_fixed_spans_and_release_them_on_resolution() {
        for suffix in ["(u 'title')", "[key]", "[]"] {
            let mut doc = Document::new();
            doc.append(&format!("> a ![old\n> end]{suffix} {}", "x".repeat(1030)))
                .unwrap();
            assert!(doc.inline.header_link_suffix.is_some());
            let candidate = doc.inline.header_link.unwrap();
            let label = doc.inline.label.to_vec();
            let capacity = doc.inline.label.capacity();
            let before = doc.work();
            doc.append(&"x".repeat(1_000_000)).unwrap();
            assert_eq!(doc.inline.header_link.unwrap().open, candidate.open);
            assert_eq!(doc.inline.header_link.unwrap().close, candidate.close);
            assert_eq!(doc.inline.label, label);
            assert_eq!(doc.inline.label.capacity(), capacity);
            assert_eq!(doc.work().label_bytes, before.label_bytes);
            assert_eq!(doc.work().delimiter_steps, before.delimiter_steps);
            doc.append("\n").unwrap();
            assert!(doc.inline.header_link.is_none());
            assert!(doc.inline.header_link_suffix.is_none());
            for i in 0..200 {
                doc.append(&format!("> a ![old\n> end]{suffix} {}", "x".repeat(1030)))
                    .unwrap();
                assert!(doc.inline.header_link_suffix.is_some());
                doc.append(match i % 4 {
                    0 => "\n",
                    1 => "|b\n> -|-\n\n",
                    2 => "|b\n> wrong\n",
                    _ => "[new](u)\n",
                })
                .unwrap();
                assert!(doc.inline.header_link.is_none());
                assert!(doc.inline.header_link_suffix.is_none());
                assert!(doc.inline.label.capacity() <= capacity);
            }
            assert!(
                doc.work().inline_bytes
                    <= doc.source().len() as u64 + doc.work().header_replay_bytes
            );
        }
    }

    #[test]
    fn isolated_shortcut_reuses_bounded_label_storage_and_releases_on_resolution() {
        let mut doc = Document::new();
        doc.append(&format!("> a ![old\n> end] {}", "x".repeat(1030)))
            .unwrap();
        let candidate = doc.inline.header_link.unwrap();
        let label = doc.inline.label.to_vec();
        let capacity = doc.inline.label.capacity();
        let before = doc.work();
        doc.append(&"x".repeat(1_000_000)).unwrap();
        assert_eq!(doc.inline.header_link.unwrap().open, candidate.open);
        assert_eq!(doc.inline.header_link.unwrap().close, candidate.close);
        assert_eq!(doc.inline.label, label);
        assert_eq!(doc.inline.label.capacity(), capacity);
        assert_eq!(doc.work().label_bytes, before.label_bytes);
        assert!(doc.references().is_empty());
        doc.append("\n").unwrap();
        assert!(doc.inline.header_link.is_none());
        assert_eq!(doc.work().delimiter_steps - before.delimiter_steps, 1);
        assert_eq!(doc.work().label_bytes - before.label_bytes, 7);
        for i in 0..200 {
            doc.append(&format!("> a ![old\n> end] {}", "x".repeat(1030)))
                .unwrap();
            assert!(doc.inline.header_link.is_some());
            doc.append(match i % 4 {
                0 => "\n",
                1 => "|b\n> -|-\n\n",
                2 => "|b\n> wrong\n",
                _ => "[new](u)\n",
            })
            .unwrap();
            assert!(doc.inline.header_link.is_none());
            // Table confirmation moves the first-cell parser out; ordinary
            // fallback reuses this buffer. Neither path retains history.
            assert!(doc.inline.label.capacity() <= capacity);
        }
        assert!(
            doc.work().inline_bytes <= doc.source().len() as u64 + doc.work().header_replay_bytes
        );
    }

    #[test]
    fn isolated_math_stores_only_endpoints_and_releases_them_on_resolution() {
        let mut doc = Document::new();
        doc.append(&format!("> a $ old\n> end$ {}", "x".repeat(1030)))
            .unwrap();
        let pair = doc.inline.header_math.as_ref().unwrap();
        let endpoints = (pair.open, pair.close);
        let gaps = doc.math_gaps.clone();
        let gap_capacity = doc.math_gaps.capacity();
        let before = doc.work();
        doc.append(&"x".repeat(1_000_000)).unwrap();
        let pair = doc.inline.header_math.as_ref().unwrap();
        assert_eq!((pair.open, pair.close), endpoints);
        assert_eq!(doc.math_gaps, gaps);
        assert_eq!(doc.math_gaps.capacity(), gap_capacity);
        assert_eq!(doc.work().math_steps, before.math_steps);
        doc.append("\n").unwrap();
        assert!(doc.inline.header_math.is_none());
        assert_eq!(doc.work().math_steps - before.math_steps, 1);
        for i in 0..200 {
            doc.append(&format!("> a $ old\n> end$ {}", "x".repeat(1030)))
                .unwrap();
            assert!(doc.inline.header_math.is_some());
            doc.append(match i % 4 {
                0 => "\n",
                1 => "|b\n> -|-\n\n",
                2 => "|b\n> wrong\n",
                _ => "$new$\n",
            })
            .unwrap();
            assert!(doc.inline.header_math.is_none());
            assert!(
                doc.maths()
                    .windows(2)
                    .all(|m| m[0].source.end <= m[1].source.start)
            );
        }
        assert!(doc.work().inline_bytes <= doc.source().len() as u64 + doc.work().table_bytes);
    }

    #[test]
    fn isolated_code_stores_one_summary_and_releases_it_on_every_resolution() {
        let mut doc = Document::new();
        doc.append(&format!("> a ` old\n> end` {}", "x".repeat(1030)))
            .unwrap();
        let pair = doc.inline.header_code.as_ref().unwrap();
        let endpoints = (pair.code.open, pair.code.last, pair.close);
        let before = doc.work();
        doc.append(&"x".repeat(1_000_000)).unwrap();
        let pair = doc.inline.header_code.as_ref().unwrap();
        assert_eq!((pair.code.open, pair.code.last, pair.close), endpoints);
        assert_eq!(doc.work().delimiter_steps, before.delimiter_steps);
        doc.append("\n").unwrap();
        assert!(doc.inline.header_code.is_none());
        assert_eq!(doc.work().delimiter_steps - before.delimiter_steps, 1);
        for i in 0..200 {
            doc.append(&format!("> a ` old\n> end` {}", "x".repeat(1030)))
                .unwrap();
            assert!(doc.inline.header_code.is_some());
            doc.append(match i % 4 {
                0 => "\n",
                1 => "|b\n> -|-\n\n",
                2 => "|b\n> wrong\n",
                _ => "&amp; [h](u)\n",
            })
            .unwrap();
            assert!(doc.inline.header_code.is_none());
        }
        assert!(doc.work().inline_bytes <= doc.source().len() as u64 + doc.work().table_bytes);
    }

    #[test]
    fn isolated_emphasis_storage_tracks_consumed_openers_not_body_or_history() {
        let mut doc = Document::new();
        doc.append(&format!(
            "> {}\n> {}",
            "*old ".repeat(4096),
            "end* ".repeat(4096)
        ))
        .unwrap();
        assert!(doc.inline.stack.is_empty());
        assert_eq!(doc.inline.header_pairs.len(), 4096);
        let capacity = doc.inline.header_pairs.capacity();
        let before = doc.work();
        doc.append(&"x".repeat(1_000_000)).unwrap();
        assert_eq!(doc.inline.header_pairs.capacity(), capacity);
        assert_eq!(doc.work().delimiter_steps, before.delimiter_steps);
        doc.append("\n").unwrap();
        assert!(doc.inline.header_pairs.is_empty());
        assert_eq!(doc.work().delimiter_steps - before.delimiter_steps, 4096);
        for _ in 0..200 {
            doc.append(&format!("> *old\n> end* {}\n", "x".repeat(1030)))
                .unwrap();
            assert!(doc.inline.header_pairs.is_empty());
            assert_eq!(doc.inline.header_pairs.capacity(), capacity);
        }
        assert!(
            doc.work().inline_bytes <= doc.source().len() as u64 + doc.work().header_replay_bytes
        );
    }

    #[test]
    fn reference_payload_storage_stops_growing_and_is_reused_after_overflow() {
        let mut doc = Document::new();
        doc.append(&format!("> [{}", "x".repeat(2048))).unwrap();
        let capacity = doc.inline.label.capacity();
        assert!(capacity <= 2 * super::MAX_LABEL_BYTES);
        assert_eq!(doc.inline.label.len(), super::MAX_LABEL_BYTES + 1);
        assert!(doc.inline.label_overflow);
        doc.append(&"x".repeat(1_000_000)).unwrap();
        assert_eq!(doc.inline.label.capacity(), capacity);
        assert_eq!(doc.inline.label.len(), super::MAX_LABEL_BYTES + 1);
        doc.append("\n> end](u) [a\n> b][]\n").unwrap();
        assert_eq!(doc.inline.label.capacity(), capacity);
        assert!(!doc.inline.label_overflow);
        assert!(doc.inline.label.is_empty()); // Closed payload is now dormant scratch.
        assert_eq!(doc.link_destination(LinkId(0)), Some("u"));
        assert_eq!(doc.references().len(), 1);
        assert_eq!(doc.references()[0].label, "a b");
    }
}

#[cfg(test)]
mod direct_title_storage_tests {
    use super::*;
    use crate::Document;

    #[test]
    fn link_slots_cover_all_allocation_roles_and_move_candidate_storage_once() {
        let mut d = Document::new();
        d.append("<https://example.org> <a@b.co> [r] ![i](v '')\n\n> [a](u 'a\n")
            .unwrap();
        assert_eq!(d.links.len(), 5);
        assert_eq!(d.links.len(), d.link_info.len());
        assert_eq!(d.links.len(), d.direct_title_gaps.len());
        assert!(d.direct_title_gaps.iter().all(|g| g.capacity() == 0));
        // Add enough source to release the retained header while the title is
        // still pending. Storage belongs solely to this candidate until `)`.
        d.append("> b\n").unwrap();
        let Some(Tail::Destination { title_gaps, .. }) = &d.inline.tail else {
            panic!("live candidate")
        };
        assert_eq!(title_gaps.len(), 1);
        let pointer = title_gaps.as_ptr();
        let capacity = title_gaps.capacity();
        assert!(d.direct_title_gaps[4].is_empty());
        d.append("')\n").unwrap();
        d.finish();
        assert_eq!(d.direct_title_gaps[4].as_ptr(), pointer);
        assert_eq!(d.direct_title_gaps[4].capacity(), capacity);
        assert_eq!(d.links.len(), d.direct_title_gaps.len());
        assert!(d.direct_title_gaps[..4].iter().all(|g| g.capacity() == 0));
        // Private storage is one Vec header per stable LinkId, plus only the
        // owned Span capacity. Existing public arena metrics cover annotation
        // arenas, not link/reference metadata or their payload heaps.
        let slot_bytes = d.direct_title_gaps.capacity() * std::mem::size_of::<Vec<Span>>();
        let gap_bytes: usize = d
            .direct_title_gaps
            .iter()
            .map(|g| g.capacity() * std::mem::size_of::<Span>())
            .sum();
        assert!(slot_bytes >= d.links.len() * std::mem::size_of::<Vec<Span>>());
        assert_eq!(gap_bytes, capacity * std::mem::size_of::<Span>());
    }
}
