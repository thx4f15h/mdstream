use crate::annotations::Annotations;
use crate::destination::{Destination, Progress};
use crate::references::{MAX_LABEL_BYTES, References, normalize_label};
#[path = "flanking.rs"]
mod flanking;
use crate::{
    ChangeKind, LinkId, LinkInfo, LinkKind, LinkTarget, Span, Style, SyntaxState, Update, Work,
};

pub(crate) struct Publishing<'a> {
    pub annotations: &'a mut Annotations,
    pub display: &'a mut crate::display::DisplayIndex,
    pub references: &'a mut References,
    pub links: &'a mut Vec<LinkTarget>,
    pub link_info: &'a mut Vec<LinkInfo>,
    pub maths: &'a mut Vec<crate::Math>,
    pub math_gaps: &'a mut Vec<Span>,
    pub footnotes: &'a mut crate::footnotes::Footnotes,
    pub update: &'a mut Update,
}

impl Publishing<'_> {
    pub fn mark(&mut self, span: Span, style: Style, link: Option<LinkId>) {
        if span.is_empty() {
            return;
        }
        self.update.change(
            span,
            if style.contains(Style::HIDDEN) {
                ChangeKind::Visibility
            } else if style != Style::default() {
                ChangeKind::Style
            } else {
                ChangeKind::Structure
            },
        );
        self.annotations
            .apply(span, style, link, self.update.revision);
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

    fn hide(&mut self, span: Span) {
        self.mark(span, Style::HIDDEN, None);
    }

    fn complete_link(
        &mut self,
        candidate: Candidate,
        target: LinkTarget,
        suffix: Option<Span>,
        title: Option<Span>,
    ) -> LinkId {
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
        let id = match candidate.link {
            Some(id) => {
                self.links[id.0] = target;
                self.link_info[id.0] = info;
                id
            }
            None => {
                let id = LinkId(self.links.len());
                self.links.push(target);
                self.link_info.push(info);
                id
            }
        };
        self.mark(
            Span::new(candidate.open + 1, candidate.close),
            Style::default(),
            Some(id),
        );
        self.hide(Span::new(candidate.open, candidate.open + 1));
        self.hide(Span::new(candidate.close, candidate.close + 1));
        if let Some(image) = candidate.image {
            self.hide(Span::new(image, image + 1));
        }
        if let Some(suffix) = suffix {
            self.hide(suffix);
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
        }
    }

    fn observing(source: &'a str, work: &'a mut Work) -> Self {
        Self {
            source,
            work,
            publishing: None,
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
    ) -> Option<LinkId> {
        self.publishing
            .as_mut()
            .map(|p| p.complete_link(candidate, target.expect("published target"), suffix, title))
    }

    fn seal_link(&mut self, id: Option<LinkId>) {
        if let Some(p) = &mut self.publishing
            && let Some(id) = id
        {
            p.link_info[id.0].state = SyntaxState::Sealed;
        }
    }

    fn code(&mut self, pair: &CodePair) {
        if let Some(p) = &mut self.publishing {
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
            p.maths.push(crate::Math {
                kind: if pair.open.len() == 1 {
                    crate::MathKind::Inline
                } else {
                    crate::MathKind::Display
                },
                source,
                content: Span::new(pair.open.end, pair.close.start),
            });
            p.update.change(source, ChangeKind::Structure);
        }
    }

    fn math_gap(&mut self, span: Span) {
        if let Some(p) = &mut self.publishing {
            debug_assert!(p.math_gaps.last().is_none_or(|g| g.end <= span.start));
            p.math_gaps.push(span);
        }
    }

    fn autolink(&mut self, start: usize, end: usize, target: crate::autolink::Target) {
        if let Some(p) = &mut self.publishing {
            let label = Span::new(start + 1, end);
            let id = LinkId(p.links.len());
            p.links.push(match target {
                crate::autolink::Target::Uri => LinkTarget::Direct(label),
                crate::autolink::Target::Email => LinkTarget::Email(label),
            });
            p.link_info.push(LinkInfo {
                kind: LinkKind::Link,
                label,
                source: Span::new(start, end + 1),
                title: None,
                state: SyntaxState::Sealed,
            });
            p.mark(label, Style::default(), Some(id));
            p.hide(Span::new(start, start + 1));
            p.hide(Span::new(end, end + 1));
        }
    }

    fn entity(&mut self, span: Span, text: String) {
        if let Some(p) = &mut self.publishing {
            p.display.entity(span, text);
            p.mark(span, Style::default(), None);
            p.update.change(span, ChangeKind::Text);
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
        effects.mark(Span::new(self.open.end, self.close.start), self.style, None);
        effects.hide(self.open);
        effects.hide(self.close);
    }
}

#[derive(Clone, Copy)]
#[cfg_attr(test, derive(Debug))]
struct Run {
    byte: u8,
    source: Span,
    before: Option<char>,
}

// Source/flanking summaries only. An unresolved label must not consume outer
// openers before its publication is known; no body bytes or stack are copied.
#[cfg_attr(test, derive(Debug))]
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

// Summaries between mismatched backtick runs, never copied/replayed body text.
// At resolution, index equal-length successors in O(r log r), then greedily
// match in source order. Each run belongs to only one primary candidate.
// Other inline syntax stays opaque even when the outer opener fails.
#[derive(Default)]
#[cfg_attr(test, derive(Debug))]
struct CodeFallback {
    runs: Vec<Code>,
    // A late table header ends the old sequence at the preceding line. Keep
    // its last summary; later payload must not affect the preceding sequence.
    boundary: Option<(usize, Option<Code>)>,
}

impl CodeFallback {
    fn clear(&mut self) {
        self.runs.clear();
        self.boundary = None;
    }

    fn push(&mut self, open: Span) {
        self.runs.push(Code::new(open));
    }

    fn isolate(&mut self) {
        self.boundary = Some((self.runs.len(), self.runs.last().copied()));
    }

    fn confirm_header(&mut self, effects: &mut Effects<'_>) {
        if let Some((len, last)) = self.boundary.take() {
            self.runs.truncate(len);
            if let Some(last) = last {
                self.runs[len - 1] = last;
            }
            self.commit(effects);
        }
    }

    fn commit(&mut self, effects: &mut Effects<'_>) {
        let len = self.runs.len();
        let mut order: Vec<_> = (0..len).collect();
        let mut next = vec![len; len];
        effects.work.delimiter_steps += 2 * len as u64;
        order.sort_unstable_by(|&a, &b| {
            effects.work.delimiter_search_steps += 1;
            (self.runs[a].open.len(), a).cmp(&(self.runs[b].open.len(), b))
        });
        for pair in order.windows(2) {
            effects.work.delimiter_steps += 1;
            let (a, b) = (pair[0], pair[1]);
            if self.runs[a].open.len() == self.runs[b].open.len() {
                next[a] = b;
            }
        }
        let mut i = 0;
        while i < len {
            effects.work.delimiter_steps += 1;
            let j = next[i];
            if j < len {
                let mut code = self.runs[i];
                code.all_spaces &= j == i + 1;
                code.last = self.runs[j - 1].last;
                code.last_space = self.runs[j - 1].last_space;
                CodePair {
                    code,
                    close: self.runs[j].open,
                }
                .commit(effects);
                i = j + 1;
            } else {
                i += 1;
            }
        }
        self.clear();
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

#[derive(Clone, Copy)]
struct Candidate {
    open: usize,
    close: usize,
    link: Option<LinkId>,
    reference: Option<AcceptedReference>,
    image: Option<usize>,
}

#[derive(Clone, Copy)]
#[cfg_attr(test, derive(Debug))]
struct Bracket {
    open: usize,
    image: Option<usize>,
}

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

enum Tail {
    After(Candidate),
    Destination {
        candidate: Candidate,
        parser: Destination,
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
            Self::Destination { candidate, parser } => {
                // Destination has no effects-dependent state. Future byte events
                // exercise its pending/invalid/complete branches in both cores.
                f.debug_tuple("Destination")
                    .field(candidate)
                    .field(parser)
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
    },
    Label(Span),
}

/// Single-pass byte scanner. Each emphasis delimiter is pushed/popped once.
/// Failed code matches indexed run summaries; it never replays body bytes.
#[derive(Default)]
#[cfg_attr(test, derive(Debug))]
pub(crate) struct Inline {
    root_paragraph: bool,
    // Sticky until paragraph closure, even after the suffix completes/fails.
    root_separator_seen: bool,
    // Non-space/tab endpoints of the raw sequence, excluding its final LF/CRLF.
    // Updated in the existing byte scan, including opaque syntax. Setext can
    // trim the outer edges without replaying text or visiting physical leaves.
    raw_content: Option<Span>,
    raw_cr: Option<usize>,
    stack: Vec<Delimiter>,
    last: [Option<usize>; 14],
    run: Option<Run>,
    code: Option<Code>,
    code_fallback: CodeFallback,
    math: Option<MathCandidate>,
    footnote: Option<usize>,
    bracket: Option<Bracket>,
    // Raw payload only, without container prefixes. One extra byte lets a
    // closing ']' complete an exactly MAX_LABEL_BYTES reference label. Direct
    // labels remain unbounded source envelopes; never keep their whole text.
    label: Vec<u8>,
    label_overflow: bool,
    label_delimiters: Vec<LabelDelimiter>,
    // Prefix from the preceding line, resolved as prose on header success.
    header_label_delimiters: usize,
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

// One fresh core for the active ordinary paragraph. No document, tree,
// reference store, published IDs or copied old parser state is retained here.
#[derive(Default)]
pub(crate) struct Eligibility {
    inline: Inline,
    cursor: Option<usize>,
    last_virtual_tab: Option<usize>,
    container: bool,
    work: Work,
}

pub(crate) type RootEligibility = Eligibility;

impl Eligibility {
    pub fn container() -> Self {
        Self {
            container: true,
            ..Self::default()
        }
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
    pub fn root_paragraph(&mut self) {
        self.root_paragraph = true;
    }

    pub fn root_separator_seen(&self) -> bool {
        self.root_separator_seen
    }

    /// Feed at most three semantic spaces owned by one original tab byte.
    /// Used before nonblank container text (new or continuing), so it can
    /// neither create a delimiter nor become a trailing ordinary line break.
    pub fn virtual_spaces(&mut self, at: usize, columns: u8, effects: &mut Effects<'_>) {
        self.math_prefix(at, effects);
        for _ in 0..columns {
            effects.work.inline_bytes += 1;
            self.byte(b' ', at, effects);
            self.previous = Some(b' ');
            self.previous_scalar = Some(' ');
        }
        if let Some(math) = &mut self.math {
            math.end = at + 1;
        }
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
        debug_assert!(self.header_pairs.is_empty());
        debug_assert!(self.header_code.is_none());
        debug_assert!(self.header_math.is_none());
        debug_assert!(self.header_link.is_none());
        debug_assert!(self.header_link_suffix.is_none());
        self.header_floor = self.stack.len();
        self.header_label_delimiters = self.label_delimiters.len();
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
        self.header_crossed
    }

    pub fn cancel_header(&mut self, effects: &mut Effects<'_>) {
        if let Some(candidate) = self.header_link.take() {
            effects.work.delimiter_steps += 1;
            let candidate = self.shortcut(candidate, effects);
            if let Some(suffix) = self.header_link_suffix.take() {
                match suffix {
                    HeaderLinkSuffix::Destination {
                        source,
                        destination,
                        title,
                    } => {
                        self.end_label(candidate.open, effects);
                        effects.complete_link(
                            candidate,
                            Some(LinkTarget::Direct(destination)),
                            Some(source),
                            title,
                        );
                    }
                    HeaderLinkSuffix::Label(source) => {
                        self.reference_tail(candidate, source, effects);
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
        for pair in self.header_pairs.drain(..) {
            pair.commit(effects);
        }
        self.clear_header();
    }

    fn clear_header(&mut self) {
        self.code_fallback.boundary = None;
        self.header_floor = 0;
        self.header_crossed = false;
        self.header_bracket = None;
        self.header_label_delimiters = 0;
        self.header_opaque = false;
    }

    /// On success discard inherited openers, retaining the first cell's live
    /// candidates and already published objects. Each surviving opener is
    /// rebased only once, when this inline sequence becomes a table cell.
    /// Return whether the first-cell state can be reused. An opaque code/math prefix
    /// is instead parsed once as a cell during the existing row scan. Never
    /// finish its pending run: that could publish a cross-line mapping/object.
    pub fn confirm_header(&mut self, effects: &mut Effects<'_>) -> bool {
        self.root_paragraph = false;
        self.root_separator_seen = false;
        debug_assert!(!self.header_crossed);
        self.code_fallback.confirm_header(effects);
        let inherited_label = self.header_link.is_some()
            || self
                .bracket
                .is_some_and(|b| Some(b.open) == self.header_bracket);
        if inherited_label {
            // The old label fails at the preceding line, which remains ordinary
            // prose. Resolve that prefix before discarding its remaining openers;
            // the current row's summaries belong to the independent first cell.
            effects.work.delimiter_steps +=
                (self.label_delimiters.len() - self.header_label_delimiters) as u64;
            let current = self
                .label_delimiters
                .split_off(self.header_label_delimiters);
            self.header_floor = 0;
            self.resolve_label_delimiters(None, effects);
            self.header_floor = self.stack.len();
            self.label_delimiters = current;
        }
        // In an independent cell a full reference suffix is itself a shortcut.
        // Its inert label has no other local effects or pending delimiter state.
        // Publish it before any later cell objects, without replaying the row.
        if let Some(HeaderLinkSuffix::Label(source)) = self.header_link_suffix.take()
            && let Some(symbol) =
                effects.reference(&effects.source[source.start + 1..source.end - 1])
        {
            let id = effects.complete_link(
                Candidate {
                    open: source.start,
                    close: source.end - 1,
                    link: None,
                    reference: None,
                    image: None,
                },
                symbol.target(),
                None,
                None,
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
        if inherited_label {
            self.resolve_label_delimiters(None, effects);
        }
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
        self.math_prefix(span.start, effects);
        for pos in span.range() {
            let byte = effects.source.as_bytes()[pos];
            effects.work.inline_bytes += 1;
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
            self.byte(byte, pos, effects);
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

    fn byte(&mut self, byte: u8, pos: usize, effects: &mut Effects<'_>) {
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
            self.cancel_header(effects);
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
                if let Some(inner) = self.code_fallback.runs.last_mut() {
                    inner.payload(byte, pos, self.previous);
                }
            }
            return;
        }
        if self.tail.is_some() && self.tail_byte(byte, pos, effects) {
            if self.root_paragraph && byte == b'\n' {
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
                    effects.autolink(start, pos, target);
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
        let bang = self.bang.take();
        match byte {
            b'^' if self
                .bracket
                .is_some_and(|b| b.image.is_none() && b.open + 1 == pos) =>
            {
                self.resolve_label_delimiters(None, effects);
                self.footnote = self.bracket.take().map(|b| b.open);
            }
            b'&' => self.entity = Some(pos),
            b'<' => {
                self.resolve_label_delimiters(None, effects);
                self.autolink = Some(pos);
                self.bracket = None;
            }
            b'\\' => self.escape = Some(pos),
            b'*' | b'_' | b'~' | b'`' | b'$' => self.start_run(byte, pos),
            b'!' => self.bang = Some(pos),
            b'[' => {
                self.resolve_label_delimiters(None, effects);
                self.label.clear();
                self.label_overflow = false;
                self.bracket = Some(Bracket {
                    open: pos,
                    image: bang.filter(|at| *at + 1 == pos),
                })
            }
            b']' => {
                if let Some(Bracket { open, image }) = self.bracket.take() {
                    let mut candidate = Candidate {
                        open,
                        close: pos,
                        link: None,
                        reference: None,
                        image,
                    };
                    if self.header_bracket == Some(open) && !self.header_crossed {
                        debug_assert!(self.header_link.is_none());
                        self.header_link = Some(candidate);
                    } else {
                        candidate = self.shortcut(candidate, effects);
                    }
                    self.tail = Some(Tail::After(candidate));
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

    fn shortcut(&mut self, mut candidate: Candidate, effects: &mut Effects<'_>) -> Candidate {
        // The closing ASCII delimiter leaves a UTF-8 boundary. The buffer is
        // capped and excludes container prefixes; never rescan the envelope.
        let symbol = (!self.label_overflow)
            .then(|| {
                let label = std::str::from_utf8(&self.label[..self.label.len() - 1]).unwrap();
                effects.reference(label)
            })
            .flatten();
        if let Some(symbol) = symbol {
            candidate.reference = Some(symbol);
            candidate.link = effects.complete_link(candidate, symbol.target(), None, None);
            self.end_label(candidate.open, effects);
        }
        candidate
    }

    // Resolve each summary once, preserving allocation across candidates. The
    // optional boundary rejects outside openers only for stars/underscores;
    // tilde's existing independent dialect is unchanged.
    fn resolve_label_delimiters(&mut self, open: Option<usize>, effects: &mut Effects<'_>) {
        let inherited = std::mem::take(&mut self.header_label_delimiters);
        if inherited != 0 {
            // A replacement can release the old line's summaries before the
            // header decision. Its pairs are ordinary prose, and its remaining
            // openers still precede the independent first cell.
            self.header_floor = 0;
        }
        let mut pending = std::mem::take(&mut self.label_delimiters);
        for (index, delimiter) in pending.drain(..).enumerate() {
            effects.work.delimiter_steps += 1;
            self.finish_delimiter(delimiter.run, delimiter.next, open, effects);
            if index + 1 == inherited {
                self.header_floor = self.stack.len();
            }
        }
        self.label_delimiters = pending;
    }

    /// A published label can neither consume outside emphasis openers nor lend
    /// unmatched openers to following text. Each summary/delimiter is processed
    /// once; no label/body replay or saved stack is needed.
    fn end_label(&mut self, open: usize, effects: &mut Effects<'_>) {
        self.resolve_label_delimiters(Some(open), effects);
        while self.stack.last().is_some_and(|d| d.source.start > open) {
            let delimiter = self.stack.pop().unwrap();
            self.last[delimiter.kind] = delimiter.previous;
            effects.work.delimiter_steps += 1;
        }
        self.header_floor = self.header_floor.min(self.stack.len());
    }

    fn start_run(&mut self, byte: u8, pos: usize) {
        self.run = Some(Run {
            byte,
            source: Span::new(pos, pos + 1),
            before: self.previous_scalar,
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
                    self.code_fallback.push(run.source);
                }
            } else {
                self.code = Some(Code::new(run.source));
            }
            return;
        }
        if self.bracket.is_some() {
            self.label_delimiters.push(LabelDelimiter { run, next });
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
        let mut close = run.source;
        while can_close && !close.is_empty() {
            let index = if run.byte == b'~' {
                self.last[kind]
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
                self.last[delimiter.kind] = delimiter.previous;
                effects.work.delimiter_steps += 1;
            }
            self.stack[index].source.end -= used;
            effects.work.delimiter_steps += 1;
            if self.stack[index].source.is_empty() {
                let delimiter = self.stack.pop().unwrap();
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
                pair.commit(effects);
            }
            close.start += used;
        }
        if can_open && !close.is_empty() {
            let index = self.stack.len();
            self.stack.push(Delimiter {
                kind,
                source: close,
                previous: self.last[kind],
            });
            self.last[kind] = Some(index);
            effects.work.delimiter_steps += 1;
        }
    }

    /// Returns false when the current byte must be processed by the normal lexer.
    fn tail_byte(&mut self, byte: u8, pos: usize, effects: &mut Effects<'_>) -> bool {
        match self.tail.take().unwrap() {
            Tail::After(candidate) => match byte {
                b'(' => {
                    self.tail = Some(Tail::Destination {
                        candidate,
                        parser: Destination::new(pos + 1),
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
            } => match if self.root_paragraph {
                let progress = parser.root_byte(byte, pos);
                self.root_separator_seen |= byte == b'\n' && matches!(progress, Progress::Pending);
                progress
            } else {
                parser.byte(byte, pos)
            } {
                Progress::Invalid => {
                    if self.header_link.is_some() {
                        self.cancel_header(effects);
                        self.header_crossed = true;
                    }
                    self.seal_candidate(candidate, effects);
                    return false;
                }
                Progress::Complete { destination, title } => {
                    let source = Span::new(parser.start - 1, pos + 1);
                    if self.header_link.is_some() {
                        self.header_link_suffix = Some(HeaderLinkSuffix::Destination {
                            source,
                            destination,
                            title,
                        });
                    } else {
                        self.end_label(candidate.open, effects);
                        effects.complete_link(
                            candidate,
                            Some(LinkTarget::Direct(destination)),
                            Some(source),
                            title,
                        );
                    }
                }
                Progress::Pending => self.tail = Some(Tail::Destination { candidate, parser }),
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
                        self.cancel_header(effects);
                        self.header_crossed = true;
                    }
                    self.seal_candidate(candidate, effects);
                    return false;
                }
                if byte == b']' && !escaped {
                    let source = Span::new(start, pos + 1);
                    if self.header_link.is_some() {
                        self.header_link_suffix = Some(HeaderLinkSuffix::Label(source));
                    } else {
                        self.reference_tail(candidate, source, effects);
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

    fn reference_tail(&mut self, candidate: Candidate, source: Span, effects: &mut Effects<'_>) {
        let symbol = if source.len() == 2 {
            // The source envelope can contain hidden container prefixes.
            candidate.reference
        } else {
            effects.reference(&effects.source[source.start + 1..source.end - 1])
        };
        if let Some(symbol) = symbol {
            self.end_label(candidate.open, effects);
            effects.complete_link(candidate, symbol.target(), Some(source), None);
        } else {
            self.seal_candidate(candidate, effects);
        }
    }

    fn seal_candidate(&mut self, candidate: Candidate, effects: &mut Effects<'_>) {
        if self.header_link.is_none() {
            self.resolve_label_delimiters(None, effects);
        }
        effects.seal_link(candidate.link);
    }

    pub fn finish(&mut self, effects: &mut Effects<'_>) {
        self.cancel_header(effects);
        self.raw_content = None;
        self.raw_cr = None;
        if let Some(tail) = &self.tail {
            let candidate = match tail {
                Tail::After(c)
                | Tail::Destination { candidate: c, .. }
                | Tail::Label { candidate: c, .. } => *c,
            };
            self.seal_candidate(candidate, effects);
        }
        self.finish_run(None, effects);
        self.resolve_label_delimiters(None, effects);
        if self.code.is_some() {
            self.code_fallback.commit(effects);
        }
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
        self.root_separator_seen = false;
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
            let mut state = format!("{inline:?}");
            // finish retains bounded label scratch for allocation reuse. Its
            // dormant bytes/overflow are not grammar state: the next '[' resets
            // both. Compare them in full whenever any label consumer is live.
            if inline.bracket.is_none() && inline.tail.is_none() && inline.header_link.is_none() {
                let start = state.find(", label: ").unwrap();
                let end = state.find(", label_delimiters: ").unwrap();
                state.replace_range(start..end, ", label: <dormant scratch>");
            }
            state.replacen("root_paragraph: true", "root_paragraph: false", 1)
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
            assert_eq!(self.doc.work, self.work, "work after {event}");
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
    fn replaced_inherited_summaries_resolve_once_and_retain_the_line_floor() {
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
                    assert!(doc.inline.label_delimiters.is_empty());
                    assert_eq!(doc.inline.label_delimiters.capacity(), capacity);
                    assert_eq!(doc.inline.header_label_delimiters, 0);
                    assert_eq!(doc.inline.header_pairs.len(), 1);
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
                    assert!(work.inline_bytes <= bytes);
                    assert!(work.delimiter_steps <= 3 * bytes);
                    assert!(work.delimiter_search_steps <= 6 * work.delimiter_steps);
                    assert!(work.tree_visits <= 24 * bytes * usize::BITS as u64);
                    assert!(doc.annotation_arena_stats().nodes <= 4 * bytes as usize);
                }
            }
        }
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
            let label = doc.inline.label.clone();
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
            assert!(doc.work().inline_bytes <= doc.source().len() as u64);
        }
    }

    #[test]
    fn isolated_shortcut_reuses_bounded_label_storage_and_releases_on_resolution() {
        let mut doc = Document::new();
        doc.append(&format!("> a ![old\n> end] {}", "x".repeat(1030)))
            .unwrap();
        let candidate = doc.inline.header_link.unwrap();
        let label = doc.inline.label.clone();
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
        assert!(doc.work().inline_bytes <= doc.source().len() as u64);
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
        assert!(doc.work().inline_bytes <= doc.source().len() as u64);
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
        assert_eq!(doc.inline.label, b"a\nb]");
        assert_eq!(doc.link_destination(LinkId(0)), Some("u"));
        assert_eq!(doc.references().len(), 1);
        assert_eq!(doc.references()[0].label, "a b");
    }
}
