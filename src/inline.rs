use crate::annotations::Annotations;
use crate::destination::{Destination, Progress};
use crate::references::{MAX_LABEL_BYTES, References};
use crate::{LinkId, LinkInfo, LinkKind, LinkTarget, Span, Style, Update, Work};

pub(crate) struct Effects<'a> {
    pub source: &'a str,
    pub annotations: &'a mut Annotations,
    pub references: &'a mut References,
    pub links: &'a mut Vec<LinkTarget>,
    pub link_info: &'a mut Vec<LinkInfo>,
    pub update: &'a mut Update,
    pub work: &'a mut Work,
}

impl Effects<'_> {
    pub fn mark(&mut self, span: Span, style: Style, link: Option<LinkId>) {
        if span.is_empty() {
            return;
        }
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
        };
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

struct LineBreak {
    newline: Span,
    hidden: Span,
    hard: bool,
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
    code: Option<Span>,
    bracket: Option<Bracket>,
    bang: Option<usize>,
    tail: Option<Tail>,
    escape: Option<usize>,
    previous: Option<u8>,
    spaces: Option<usize>,
    carriage: bool,
    line_break: Option<LineBreak>,
}

impl Inline {
    pub fn feed(&mut self, span: Span, effects: &mut Effects<'_>) {
        for pos in span.range() {
            let byte = effects.source.as_bytes()[pos];
            effects.work.inline_bytes += 1;
            self.byte(byte, pos, effects);
            self.previous = Some(byte);
        }
    }

    fn byte(&mut self, byte: u8, pos: usize, effects: &mut Effects<'_>) {
        // Only a subsequent byte in the same inline block confirms a break.
        // Block closure discards this candidate, so terminal newlines stay plain.
        if let Some(line_break) = self.line_break.take() {
            effects.mark(
                line_break.newline,
                if line_break.hard {
                    Style::HARD_BREAK
                } else {
                    Style::SOFT_BREAK
                },
                None,
            );
            effects.hide(line_break.hidden);
        }
        if let Some(run) = &mut self.run {
            if byte == run.byte {
                run.source.end = pos + 1;
                return;
            }
            self.finish_run(Some(byte), effects);
        }
        if self.code.is_some() {
            if byte == b'`' {
                self.start_run(byte, pos);
            }
            return;
        }
        if self.tail.is_some() && self.tail_byte(byte, pos, effects) {
            return;
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
            b'\\' => self.escape = Some(pos),
            b'*' | b'_' | b'~' | b'`' => self.start_run(byte, pos),
            b'!' => self.bang = Some(pos),
            b'[' => {
                self.bracket = Some(Bracket {
                    open: pos,
                    image: bang.filter(|at| *at + 1 == pos),
                })
            }
            b']' => {
                if let Some(Bracket { open, image }) = self.bracket.take() {
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
        if run.byte == b'`' {
            if let Some(open) = self.code {
                if open.len() == run.source.len() {
                    effects.mark(Span::new(open.end, run.source.start), Style::CODE, None);
                    effects.hide(open);
                    effects.hide(run.source);
                    self.code = None;
                }
            } else {
                self.code = Some(run.source);
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
                _ => return false,
            },
            Tail::Destination {
                candidate,
                mut parser,
            } => match parser.byte(byte, pos) {
                Progress::Invalid => return false,
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
                    }
                } else {
                    self.tail = Some(Tail::Label { candidate, start });
                }
            }
        }
        true
    }

    pub fn finish(&mut self, effects: &mut Effects<'_>) {
        self.finish_run(None, effects);
        effects.work.delimiter_steps += self.stack.len() as u64;
        self.stack.clear();
        self.last = [None; 5];
        self.code = None;
        self.bracket = None;
        self.bang = None;
        self.tail = None;
        self.escape = None;
        self.previous = None;
        self.spaces = None;
        self.carriage = false;
        self.line_break = None;
    }
}
