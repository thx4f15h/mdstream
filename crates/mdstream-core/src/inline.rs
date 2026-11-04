use crate::annotations::Annotations;
use crate::destination::{Destination, Progress};
use crate::references::{MAX_LABEL_BYTES, References};
use crate::{
    ChangeKind, LinkId, LinkInfo, LinkKind, LinkTarget, Span, Style, SyntaxState, Update, Work,
};

pub(crate) struct Effects<'a> {
    pub source: &'a str,
    pub annotations: &'a mut Annotations,
    pub display: &'a mut crate::display::DisplayIndex,
    pub references: &'a mut References,
    pub links: &'a mut Vec<LinkTarget>,
    pub link_info: &'a mut Vec<LinkInfo>,
    pub maths: &'a mut Vec<crate::Math>,
    pub math_gaps: &'a mut Vec<Span>,
    pub footnotes: &'a mut crate::footnotes::Footnotes,
    pub update: &'a mut Update,
    pub work: &'a mut Work,
}

impl Effects<'_> {
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

#[derive(Clone, Copy)]
struct Delimiter {
    kind: usize,
    source: Span,
    previous: Option<usize>,
}

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
struct Run {
    byte: u8,
    source: Span,
    before: Option<u8>,
}

#[derive(Clone, Copy)]
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

const MAX_CODE_FALLBACK_RUNS: usize = 32;

// Summaries between mismatched backtick runs, never copied/replayed body text.
// At most 32 runs: greedy matching takes at most 32 comparisons per run.
// Other inline syntax stays opaque even when the outer opener fails.
#[derive(Default)]
struct CodeFallback {
    runs: Vec<Code>,
    overflow: bool,
    // A late table header ends the old sequence at the preceding line. Keep
    // its last summary, including when the new line exceeds the run bound.
    boundary: Option<(usize, Option<Code>, bool)>,
}

impl CodeFallback {
    fn clear(&mut self) {
        self.runs.clear();
        self.overflow = false;
        self.boundary = None;
    }

    fn push(&mut self, open: Span) {
        if self.runs.len() == MAX_CODE_FALLBACK_RUNS {
            self.overflow = true;
        } else {
            self.runs.push(Code::new(open));
        }
    }

    fn isolate(&mut self) {
        self.boundary = Some((self.runs.len(), self.runs.last().copied(), self.overflow));
    }

    fn confirm_header(&mut self, effects: &mut Effects<'_>) {
        if let Some((len, last, overflow)) = self.boundary.take() {
            self.runs.truncate(len);
            if let Some(last) = last {
                self.runs[len - 1] = last;
            }
            self.overflow = overflow;
            self.commit(effects);
        }
    }

    fn commit(&mut self, effects: &mut Effects<'_>) {
        if !self.overflow {
            let mut i = 0;
            while i < self.runs.len() {
                let mut code = self.runs[i];
                let close = (i + 1..self.runs.len()).find(|&j| {
                    effects.work.delimiter_steps += 1;
                    code.open.len() == self.runs[j].open.len()
                });
                if let Some(j) = close {
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
        }
        self.clear();
    }
}

struct CodePair {
    code: Code,
    close: Span,
}

impl CodePair {
    fn commit(self, effects: &mut Effects<'_>) {
        let content = Span::new(self.code.open.end, self.close.start);
        effects.display.code(
            content,
            effects.source,
            self.code.all_spaces,
            self.code.last,
            self.code.last_space,
        );
        effects.update.change(content, ChangeKind::Text);
        effects.mark(content, Style::CODE, None);
        effects.hide(self.code.open);
        effects.hide(self.close);
    }
}

struct MathCandidate {
    open: Span,
    escaped: bool,
    // End of the last feed; same-scope container prefixes are not fed.
    end: usize,
}

struct MathPair {
    open: Span,
    close: Span,
}

impl MathPair {
    fn commit(self, effects: &mut Effects<'_>) {
        let content = Span::new(self.open.end, self.close.start);
        let source = Span::new(self.open.start, self.close.end);
        effects.maths.push(crate::Math {
            kind: if self.open.len() == 1 {
                crate::MathKind::Inline
            } else {
                crate::MathKind::Display
            },
            source,
            content,
        });
        // The parser never decodes, copies or revisits the body.
        effects.mark(content, Style::MATH, None);
        effects.hide(self.open);
        effects.hide(self.close);
        effects.update.change(source, ChangeKind::Structure);
    }
}

#[derive(Clone, Copy)]
struct Candidate {
    open: usize,
    close: usize,
    link: Option<LinkId>,
    image: Option<usize>,
}

#[derive(Clone, Copy)]
struct Bracket {
    open: usize,
    image: Option<usize>,
}

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

// Only syntax-free suffix payloads can share the already consumed cell prefix.
// Keep spans, not copied targets or prematurely interned reference symbols.
enum HeaderLinkSuffix {
    Destination {
        source: Span,
        destination: Span,
        title: Option<Span>,
    },
    Label(Span),
}

/// Single-pass byte scanner. Each emphasis delimiter is pushed/popped once.
/// Failed code matches bounded run summaries; it never replays body bytes.
#[derive(Default)]
pub(crate) struct Inline {
    // Non-space/tab endpoints of the raw sequence, excluding its final LF/CRLF.
    // Updated in the existing byte scan, including opaque syntax. Setext can
    // trim the outer edges without replaying text or visiting physical leaves.
    raw_content: Option<Span>,
    raw_cr: Option<usize>,
    stack: Vec<Delimiter>,
    last: [Option<usize>; 5],
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
    bang: Option<usize>,
    tail: Option<Tail>,
    escape: Option<usize>,
    previous: Option<u8>,
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

impl Inline {
    /// Feed at most three semantic spaces owned by one original tab byte.
    /// Used before nonblank container text (new or continuing), so it can
    /// neither create a delimiter nor become a trailing ordinary line break.
    pub fn virtual_spaces(&mut self, at: usize, columns: u8, effects: &mut Effects<'_>) {
        self.math_prefix(at, effects);
        for _ in 0..columns {
            effects.work.inline_bytes += 1;
            self.byte(b' ', at, effects);
            self.previous = Some(b' ');
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
        self.header_crossed = false;
        self.header_bracket = self.bracket.map(|b| b.open);
        self.header_opaque = self.code.is_some() || self.math.is_some();
        if self.code.is_some() {
            self.code_fallback.isolate();
        }
        self.previous = None;
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
                        effects.complete_link(
                            candidate,
                            LinkTarget::Direct(destination),
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
        self.header_opaque = false;
    }

    /// On success discard inherited openers, retaining the first cell's live
    /// candidates and already published objects. Each surviving opener is
    /// rebased only once, when this inline sequence becomes a table cell.
    /// Return whether the first-cell state can be reused. An opaque code/math prefix
    /// is instead parsed once as a cell during the existing row scan. Never
    /// finish its pending run: that could publish a cross-line mapping/object.
    pub fn confirm_header(&mut self, effects: &mut Effects<'_>) -> bool {
        debug_assert!(!self.header_crossed);
        self.code_fallback.confirm_header(effects);
        // In an independent cell a full reference suffix is itself a shortcut.
        // Its inert label has no other local effects or pending delimiter state.
        // Publish it before any later cell objects, without replaying the row.
        if let Some(HeaderLinkSuffix::Label(source)) = self.header_link_suffix.take()
            && let Some(symbol) = effects.references.intern(
                &effects.source[source.start + 1..source.end - 1],
                effects.work,
            )
        {
            let id = effects.complete_link(
                Candidate {
                    open: source.start,
                    close: source.end - 1,
                    link: None,
                    image: None,
                },
                LinkTarget::Reference(symbol),
                None,
                None,
            );
            effects.link_info[id.0].state = SyntaxState::Sealed;
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
            self.last = [None; 5];
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
            debug_assert!(effects.math_gaps.last().is_none_or(|g| g.end <= math.end));
            effects.math_gaps.push(Span::new(math.end, start));
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
            self.finish_run(Some(byte), effects);
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
                if !self.code_fallback.overflow
                    && let Some(inner) = self.code_fallback.runs.last_mut()
                {
                    inner.payload(byte, pos, self.previous);
                }
            }
            return;
        }
        if self.tail.is_some() && self.tail_byte(byte, pos, effects) {
            return;
        }
        if let Some(start) = self.autolink.take() {
            if byte == b'>' {
                effects.work.autolink_bytes += (pos - start - 1) as u64;
                if let Some(target) = crate::autolink::classify(&effects.source[start + 1..pos]) {
                    let label = Span::new(start + 1, pos);
                    let id = LinkId(effects.links.len());
                    effects.links.push(match target {
                        crate::autolink::Target::Uri => LinkTarget::Direct(label),
                        crate::autolink::Target::Email => LinkTarget::Email(label),
                    });
                    effects.link_info.push(LinkInfo {
                        kind: LinkKind::Link,
                        label,
                        source: Span::new(start, pos + 1),
                        title: None,
                        state: SyntaxState::Sealed,
                    });
                    effects.mark(label, Style::default(), Some(id));
                    effects.hide(Span::new(start, start + 1));
                    effects.hide(Span::new(pos, pos + 1));
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
                    effects.display.entity(span, text);
                    // Display text changed even if raw syntax/style is unchanged.
                    effects.mark(span, Style::default(), None);
                    effects.update.change(span, ChangeKind::Text);
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
                    let symbol = effects.footnotes.intern(label, effects.work);
                    let source = Span::new(start, pos + 1);
                    effects
                        .footnotes
                        .reference(symbol, source, effects.update, effects.work);
                    effects.mark(source, Style::default(), None);
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
                self.footnote = self.bracket.take().map(|b| b.open);
            }
            b'&' => self.entity = Some(pos),
            b'<' => {
                self.autolink = Some(pos);
                self.bracket = None;
            }
            b'\\' => self.escape = Some(pos),
            b'*' | b'_' | b'~' | b'`' | b'$' => self.start_run(byte, pos),
            b'!' => self.bang = Some(pos),
            b'[' => {
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

    fn shortcut(&self, mut candidate: Candidate, effects: &mut Effects<'_>) -> Candidate {
        // The closing ASCII delimiter leaves a UTF-8 boundary. The buffer is
        // capped and excludes container prefixes; never rescan the envelope.
        let symbol = (!self.label_overflow)
            .then(|| {
                let label = std::str::from_utf8(&self.label[..self.label.len() - 1]).unwrap();
                effects.references.intern(label, effects.work)
            })
            .flatten();
        if let Some(symbol) = symbol {
            candidate.link =
                Some(effects.complete_link(candidate, LinkTarget::Reference(symbol), None, None));
        }
        candidate
    }

    fn start_run(&mut self, byte: u8, pos: usize) {
        self.run = Some(Run {
            byte,
            source: Span::new(pos, pos + 1),
            before: self.previous,
        });
    }

    fn finish_run(&mut self, next: Option<u8>, effects: &mut Effects<'_>) {
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
        let (kind, style) = match (run.byte, run.source.len()) {
            (b'*', 1) => (0, Style::EMPHASIS),
            (b'*', 2) => (1, Style::STRONG),
            (b'_', 1) => (2, Style::EMPHASIS),
            (b'_', 2) => (3, Style::STRONG),
            (b'~', 2) => (4, Style::STRIKE),
            _ => return,
        };
        // ASCII flanking rules are intentional; full CommonMark is not claimed.
        let can_open = next.is_some_and(|b| !b.is_ascii_whitespace());
        let can_close = run.before.is_some_and(|b| !b.is_ascii_whitespace());
        if run.byte == b'_'
            && run.before.is_some_and(|b| b.is_ascii_alphanumeric())
            && next.is_some_and(|b| b.is_ascii_alphanumeric())
        {
            return;
        }
        if can_close && let Some(index) = self.last[kind] {
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
            while self.stack.len() > index {
                let delimiter = self.stack.pop().unwrap();
                self.last[delimiter.kind] = delimiter.previous;
                effects.work.delimiter_steps += 1;
            }
            self.header_floor = self.header_floor.min(self.stack.len());
            let pair = EmphasisPair {
                open,
                close: run.source,
                style,
            };
            if isolate {
                self.header_pairs.push(pair);
            } else {
                pair.commit(effects);
            }
        } else if can_open {
            let index = self.stack.len();
            self.stack.push(Delimiter {
                kind,
                source: run.source,
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
            } => match parser.byte(byte, pos) {
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
                        effects.complete_link(
                            candidate,
                            LinkTarget::Direct(destination),
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

    fn reference_tail(&self, candidate: Candidate, source: Span, effects: &mut Effects<'_>) {
        let symbol = if source.len() == 2 {
            // The source envelope can contain hidden container prefixes.
            candidate.link.and_then(|id| match effects.links[id.0] {
                LinkTarget::Reference(symbol) => Some(symbol),
                _ => None,
            })
        } else {
            effects.references.intern(
                &effects.source[source.start + 1..source.end - 1],
                effects.work,
            )
        };
        if let Some(symbol) = symbol {
            effects.complete_link(candidate, LinkTarget::Reference(symbol), Some(source), None);
        } else {
            self.seal_candidate(candidate, effects);
        }
    }

    fn seal_candidate(&self, candidate: Candidate, effects: &mut Effects<'_>) {
        if let Some(id) = candidate.link {
            effects.link_info[id.0].state = SyntaxState::Sealed;
        }
    }

    pub fn finish(&mut self, effects: &mut Effects<'_>) {
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
        if self.code.is_some() {
            self.code_fallback.commit(effects);
        }
        effects.work.delimiter_steps += self.stack.len() as u64;
        self.stack.clear();
        self.last = [None; 5];
        self.code = None;
        self.math = None;
        self.footnote = None;
        self.bracket = None;
        self.bang = None;
        self.tail = None;
        self.escape = None;
        self.previous = None;
        self.spaces = None;
        self.whitespace = None;
        self.carriage = false;
        self.line_break = None;
        self.autolink = None;
        self.entity = None;
        self.cancel_header(effects);
    }
}

#[cfg(test)]
mod tests {
    use crate::{Document, LinkId};

    #[test]
    fn failed_code_journal_storage_is_bounded_and_reused() {
        let mut doc = Document::new();
        doc.append(&format!("`x{}", "``x".repeat(1000))).unwrap();
        let journal = &doc.inline.code_fallback;
        assert!(journal.overflow);
        assert_eq!(journal.runs.len(), super::MAX_CODE_FALLBACK_RUNS);
        let capacity = journal.runs.capacity();
        assert_eq!(capacity, super::MAX_CODE_FALLBACK_RUNS);
        for _ in 0..1000 {
            doc.append("``x").unwrap();
            assert_eq!(doc.inline.code_fallback.runs.capacity(), capacity);
        }
        doc.append("` end\n\n").unwrap();
        assert!(doc.inline.code_fallback.runs.is_empty());
        assert!(!doc.inline.code_fallback.overflow);
        assert!(doc.inline.code_fallback.boundary.is_none());
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
