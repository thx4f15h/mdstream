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
}

struct MathCandidate {
    open: Span,
    escaped: bool,
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
    },
}

/// Single-pass delimiter machine. Each delimiter is pushed/popped at most once.
/// Crossing pairs discard intervening openers; there is no backtracking.
#[derive(Default)]
pub(crate) struct Inline {
    stack: Vec<Delimiter>,
    last: [Option<usize>; 5],
    run: Option<Run>,
    code: Option<Code>,
    math: Option<MathCandidate>,
    footnote: Option<usize>,
    bracket: Option<Bracket>,
    bang: Option<usize>,
    tail: Option<Tail>,
    escape: Option<usize>,
    previous: Option<u8>,
    spaces: Option<usize>,
    carriage: bool,
    line_break: Option<LineBreak>,
    autolink: Option<usize>,
    entity: Option<usize>,
    // A late header can inherit emphasis openers until one actually closes.
    // Track that event, not just stack length (which may grow again).
    header_floor: usize,
    header_crossed: bool,
    // Labels are not opaque: preserve local effects, but never reuse a label
    // that started on a preceding line as part of the first table cell.
    header_bracket: Option<usize>,
    // Inherited code or math keeps this line opaque until table confirmation.
    // Its prefix has no published effects and needs a fresh cell parser then.
    header_opaque: bool,
}

impl Inline {
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
            && !self.carriage
            && self.autolink.is_none()
            && self.entity.is_none()
    }

    pub fn isolate_line(&mut self) -> Option<LineBreak> {
        debug_assert!(self.can_isolate_line());
        self.header_floor = self.stack.len();
        self.header_crossed = false;
        self.header_bracket = self.bracket.map(|b| b.open);
        self.header_opaque = self.code.is_some() || self.math.is_some();
        self.previous = None;
        self.line_break.take()
    }

    pub fn header_crossed(&self) -> bool {
        self.header_crossed
    }

    pub fn cancel_header(&mut self) {
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
    pub fn confirm_header(&mut self, work: &mut Work) -> bool {
        debug_assert!(!self.header_crossed);
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
        self.cancel_header();
        true
    }

    pub fn feed(&mut self, span: Span, effects: &mut Effects<'_>) {
        for pos in span.range() {
            let byte = effects.source.as_bytes()[pos];
            effects.work.inline_bytes += 1;
            self.byte(byte, pos, effects);
            self.previous = Some(byte);
        }
    }

    fn byte(&mut self, byte: u8, pos: usize, effects: &mut Effects<'_>) {
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
                code.all_spaces &= crate::display::code_space(byte);
                code.last = Span::new(
                    pos - usize::from(byte == b'\n' && self.previous == Some(b'\r')),
                    pos + 1,
                );
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
            self.line_break = Some(LineBreak {
                newline: Span::new(pos, pos + 1),
                hidden: Span::new(escape.or(spaces).unwrap_or(end), end),
                hard: escape.is_some() || spaces.is_some_and(|start| end - start >= 2),
            });
            self.bang = None;
            return;
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
                self.bracket = Some(Bracket {
                    open: pos,
                    image: bang.filter(|at| *at + 1 == pos),
                })
            }
            b']' => {
                if let Some(Bracket { open, image }) = self.bracket.take() {
                    // Even an overlong label can acquire a direct destination
                    // later. Cancel at its close, before any object is published.
                    self.header_crossed |= self.header_bracket == Some(open);
                    let label = &effects.source[open + 1..pos];
                    let mut candidate = Candidate {
                        open,
                        close: pos,
                        link: None,
                        image,
                    };
                    if let Some(symbol) = effects.references.intern(label, effects.work) {
                        candidate.link = Some(effects.complete_link(
                            candidate,
                            LinkTarget::Reference(symbol),
                            None,
                            None,
                        ));
                    }
                    self.tail = Some(Tail::After(candidate));
                }
            }
            _ => {}
        }
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
                    self.header_crossed |= self.header_opaque;
                    let content = Span::new(open.end, run.source.start);
                    effects.maths.push(crate::Math {
                        kind: if open.len() == 1 {
                            crate::MathKind::Inline
                        } else {
                            crate::MathKind::Display
                        },
                        source: Span::new(open.start, run.source.end),
                        content,
                    });
                    // The parser never decodes, copies or revisits the body.
                    effects.mark(content, Style::MATH, None);
                    effects.hide(open);
                    effects.hide(run.source);
                    effects
                        .update
                        .change(Span::new(open.start, run.source.end), ChangeKind::Structure);
                    self.math = None;
                }
            } else if run.source.len() <= 2 {
                self.math = Some(MathCandidate {
                    open: run.source,
                    escaped: false,
                });
            }
            return;
        }
        if run.byte == b'`' {
            if let Some(code) = &mut self.code {
                let open = code.open;
                if open.len() == run.source.len() {
                    self.header_crossed |= self.header_opaque;
                    let content = Span::new(open.end, run.source.start);
                    effects
                        .display
                        .code(content, effects.source, code.all_spaces, code.last);
                    effects.update.change(content, ChangeKind::Text);
                    effects.mark(Span::new(open.end, run.source.start), Style::CODE, None);
                    effects.hide(open);
                    effects.hide(run.source);
                    self.code = None;
                } else {
                    code.all_spaces = false;
                    code.last = run.source;
                }
            } else {
                self.code = Some(Code {
                    open: run.source,
                    all_spaces: true,
                    last: Span::new(run.source.end, run.source.end),
                });
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
            self.header_crossed |= index < self.header_floor;
            let open = self.stack[index].source;
            while self.stack.len() > index {
                let delimiter = self.stack.pop().unwrap();
                self.last[delimiter.kind] = delimiter.previous;
                effects.work.delimiter_steps += 1;
            }
            effects.mark(Span::new(open.end, run.source.start), style, None);
            effects.hide(open);
            effects.hide(run.source);
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
                    self.seal_candidate(candidate, effects);
                    return false;
                }
                Progress::Complete { destination, title } => {
                    effects.complete_link(
                        candidate,
                        LinkTarget::Direct(destination),
                        Some(Span::new(parser.start - 1, pos + 1)),
                        title,
                    );
                }
                Progress::Pending => self.tail = Some(Tail::Destination { candidate, parser }),
            },
            Tail::Label { candidate, start } => {
                if pos - start - 1 > MAX_LABEL_BYTES || matches!(byte, b'\n' | b'\r' | b'[') {
                    self.seal_candidate(candidate, effects);
                    return false;
                }
                if byte == b']' {
                    let label = if pos == start + 1 {
                        &effects.source[candidate.open + 1..candidate.close]
                    } else {
                        &effects.source[start + 1..pos]
                    };
                    if let Some(symbol) = effects.references.intern(label, effects.work) {
                        effects.complete_link(
                            candidate,
                            LinkTarget::Reference(symbol),
                            Some(Span::new(start, pos + 1)),
                            None,
                        );
                    } else {
                        self.seal_candidate(candidate, effects);
                    }
                } else {
                    self.tail = Some(Tail::Label { candidate, start });
                }
            }
        }
        true
    }

    fn seal_candidate(&self, candidate: Candidate, effects: &mut Effects<'_>) {
        if let Some(id) = candidate.link {
            effects.link_info[id.0].state = SyntaxState::Sealed;
        }
    }

    /// Source-borrowing labels/math cannot contain hidden container prefixes.
    /// Their failed prefixes stay literal/opaque; never replay them. Delimiters,
    /// code spans and the pending line break continue in the same exact scope.
    pub fn end_container_line(&mut self) {
        self.bracket = None;
        self.math = None;
    }

    pub fn finish(&mut self, effects: &mut Effects<'_>) {
        if let Some(tail) = &self.tail {
            let candidate = match tail {
                Tail::After(c)
                | Tail::Destination { candidate: c, .. }
                | Tail::Label { candidate: c, .. } => *c,
            };
            self.seal_candidate(candidate, effects);
        }
        self.finish_run(None, effects);
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
        self.carriage = false;
        self.line_break = None;
        self.autolink = None;
        self.entity = None;
        self.cancel_header();
    }
}
