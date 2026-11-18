//! Online finite TeX parser. All user-sized ownership is in fixed pages.
use super::input::Input;
use super::storage::{Edge, List, Paged, StorageWork};
use super::symbols::symbol;
use mdtui3::{Budget, Span, WorkLimit};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ink {
    Payload(Span),
    Static(&'static str),
    Space(usize),
    Glyphs(usize),
    Glyph(usize),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Text {
        ink: Ink,
        width: Option<usize>,
    },
    Row,
    CellEnd,
    Fraction,
    Root,
    Script {
        upper: bool,
        lower: bool,
    },
    Fence {
        left: char,
        right: char,
        left_source: Span,
        right_source: Span,
    },
    Matrix {
        columns: usize,
        fence: char,
    },
    Error,
}
#[derive(Clone, Copy, Debug)]
pub(super) struct Node {
    pub kind: Kind,
    pub logical: Span,
    pub source: Span,
    pub origin: Span,
    pub children: List,
    pub needs_shape: bool,
    pub flow: usize,
}
#[derive(Clone, Copy, Debug, Default)]
pub struct SyntaxWork {
    pub bytes: usize,
    pub tokens: usize,
    pub nodes: usize,
    pub edges: usize,
    pub reductions: usize,
    pub steps: usize,
    pub max_frames: usize,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SyntaxStep {
    Progress,
    Input,
    Complete,
    BudgetExhausted,
}
#[derive(Clone, Copy)]
struct Name {
    bytes: [u8; 16],
    len: usize,
}
impl Name {
    fn new() -> Self {
        Self {
            bytes: [0; 16],
            len: 0,
        }
    }
    fn push(&mut self, b: u8) {
        if self.len < self.bytes.len() {
            self.bytes[self.len] = b
        }
        self.len += 1;
    }
    fn text(&self) -> Option<&str> {
        (self.len <= self.bytes.len())
            .then(|| std::str::from_utf8(&self.bytes[..self.len]).expect("ASCII control word"))
    }
}
#[derive(Clone, Copy)]
enum Lex {
    Plain,
    Slash(usize),
    Word(usize, Name),
    Utf8 {
        start: usize,
        bytes: [u8; 4],
        used: usize,
        want: usize,
    },
}
#[derive(Clone, Copy)]
enum Token {
    Char(char, Span),
    Command(Span, Name),
    End,
}
#[derive(Clone, Copy)]
enum FrameKind {
    Row,
    Group,
    Call(Kind, usize),
    Script(usize, bool),
    Fence(char, Span),
    Matrix(char),
    Env(bool),
}
#[derive(Clone, Copy)]
struct Frame {
    kind: FrameKind,
    start: usize,
    origin: Span,
    children: List,
    column: usize,
    columns: usize,
    name: Name,
    env_open: bool,
    needs_shape: bool,
}
impl Frame {
    fn new(kind: FrameKind, start: usize, origin: Span) -> Self {
        Self {
            kind,
            start,
            origin,
            children: List::default(),
            column: 1,
            columns: 0,
            name: Name::new(),
            env_open: false,
            needs_shape: false,
        }
    }
}
#[derive(Clone, Copy)]
struct SmallBuild {
    kind: Kind,
    logical: Span,
    origin: Span,
    children: [usize; 3],
    len: usize,
    at: usize,
    list: List,
    needs_shape: bool,
}
#[derive(Clone, Copy)]
pub(super) struct Prefix {
    pub list: List,
    pub ghost: Option<usize>,
    pub version: u64,
    pub complete: bool,
    pub error: bool,
}
pub(super) struct Syntax {
    pub nodes: Paged<Node>,
    pub edges: Paged<Edge>,
    frames: Paged<Frame>,
    pub work: SyntaxWork,
    pub root: Option<usize>,
    p: usize,
    lex: Lex,
    token: Option<Token>,
    value: Option<usize>,
    build: Option<SmallBuild>,
    closing: bool,
    done: bool,
    error: bool,
    fence_next: Option<(bool, Span)>,
    close_matrix: Option<Span>,
    root_version: u64,
    flow: usize,
}
fn pay(b: &mut Budget, bytes: usize) -> bool {
    b.charge(WorkLimit {
        queries: 1,
        objects: 1,
        bytes,
        ..WorkLimit::default()
    })
}
impl Syntax {
    pub fn new() -> Self {
        Self {
            nodes: Paged::default(),
            edges: Paged::default(),
            frames: Paged::default(),
            work: SyntaxWork::default(),
            root: None,
            p: 0,
            lex: Lex::Plain,
            token: None,
            value: None,
            build: None,
            closing: false,
            done: false,
            error: false,
            fence_next: None,
            close_matrix: None,
            root_version: 0,
            flow: 0,
        }
    }
    pub fn position(&self) -> usize {
        self.p
    }
    pub fn prefix(&self) -> Option<Prefix> {
        if self.value.is_some()
            || self.build.is_some()
            || self.close_matrix.is_some()
            || self.closing && !self.done
        {
            return None;
        }
        if let Some(root) = self.root {
            return Some(Prefix {
                list: self.nodes[root].children,
                ghost: None,
                version: self.root_version,
                complete: true,
                error: self.error,
            });
        }
        if self.frames.is_empty() {
            return None;
        }
        let root = self.frames[0];
        let ghost = if self.frames.len() > 1 {
            match self.frames[1].kind {
                FrameKind::Script(base, _) => Some(base),
                _ => None,
            }
        } else {
            None
        };
        Some(Prefix {
            list: root.children,
            ghost,
            version: self.root_version,
            complete: false,
            error: self.error,
        })
    }
    fn push(&mut self, kind: FrameKind, start: usize, origin: Span) {
        self.frames.push(Frame::new(kind, start, origin));
        self.work.max_frames = self.work.max_frames.max(self.frames.len())
    }
    fn node(
        &mut self,
        input: &Input,
        kind: Kind,
        logical: Span,
        origin: Span,
        children: List,
        needs_shape: bool,
    ) -> usize {
        let id = self.nodes.push(Node {
            kind,
            logical,
            source: input.root(logical),
            origin,
            children,
            needs_shape,
            flow: self.flow,
        });
        self.work.nodes += 1;
        id
    }
    fn leaf(&mut self, input: &Input, kind: Kind, span: Span, needs_shape: bool) {
        let id = self.node(
            input,
            kind,
            span,
            input.root(span),
            List::default(),
            needs_shape,
        );
        self.value = Some(id)
    }
    fn fail(&mut self, input: &Input, span: Span) {
        self.error = true;
        self.leaf(input, Kind::Error, span, false)
    }
    fn marker(&mut self, input: &Input) {
        self.flow += 1;
        let span = Span::new(self.p, self.p);
        self.leaf(input, Kind::CellEnd, span, false)
    }
    pub fn step(&mut self, input: &Input, b: &mut Budget) -> SyntaxStep {
        if self.done {
            return SyntaxStep::Complete;
        }
        if b.remaining().queries == 0 || b.remaining().objects == 0 {
            return SyntaxStep::BudgetExhausted;
        }
        if self.frames.is_empty() {
            if !pay(b, 0) {
                return SyntaxStep::BudgetExhausted;
            }
            self.push(FrameKind::Row, 0, Span::new(input.origin, input.origin));
            self.work.steps += 1;
            return SyntaxStep::Progress;
        }
        if let Some(mut build) = self.build {
            if !pay(b, 0) {
                return SyntaxStep::BudgetExhausted;
            }
            self.work.steps += 1;
            if build.at < build.len {
                build.list.push(&mut self.edges, build.children[build.at]);
                self.work.edges += 1;
                build.at += 1;
                self.build = Some(build)
            } else {
                let id = self.node(
                    input,
                    build.kind,
                    build.logical,
                    build.origin,
                    build.list,
                    build.needs_shape,
                );
                self.value = Some(id);
                self.build = None
            }
            return SyntaxStep::Progress;
        }
        if let Some(value) = self.value {
            if !pay(b, 0) {
                return SyntaxStep::BudgetExhausted;
            }
            self.value = None;
            self.work.steps += 1;
            self.work.reductions += 1;
            let top = *self.frames.last().unwrap();
            match top.kind {
                FrameKind::Call(kind, count) => {
                    self.flow += 1;
                    let f = self.frames.last_mut().unwrap();
                    f.children.push(&mut self.edges, value);
                    f.needs_shape |= self.nodes[value].needs_shape;
                    self.work.edges += 1;
                    if f.children.len == count {
                        let f = self.frames.pop().unwrap();
                        let end = self.nodes[value].logical.end;
                        let id = self.node(
                            input,
                            kind,
                            Span::new(f.start, end),
                            f.origin,
                            f.children,
                            f.needs_shape,
                        );
                        self.value = Some(id)
                    }
                }
                FrameKind::Script(base, upper) => {
                    self.flow += 1;
                    let f = self.frames.pop().unwrap();
                    let mut actual = base;
                    let mut up = None;
                    let mut low = None;
                    if let Kind::Script { upper, lower } = self.nodes[base].kind {
                        let n = self.nodes[base];
                        actual = n.children.nth_small(&self.edges, 0).unwrap();
                        if upper {
                            up = n.children.nth_small(&self.edges, 1)
                        }
                        if lower {
                            low = n.children.nth_small(&self.edges, if upper { 2 } else { 1 })
                        }
                    }
                    if upper && up.is_some() || !upper && low.is_some() {
                        self.fail(input, Span::new(f.start, self.nodes[value].logical.end))
                    } else {
                        if upper {
                            up = Some(value)
                        } else {
                            low = Some(value)
                        }
                        let mut children = [actual; 3];
                        let mut len = 1;
                        if let Some(id) = up {
                            children[len] = id;
                            len += 1
                        }
                        if let Some(id) = low {
                            children[len] = id;
                            len += 1
                        }
                        self.build = Some(SmallBuild {
                            kind: Kind::Script {
                                upper: up.is_some(),
                                lower: low.is_some(),
                            },
                            logical: Span::new(
                                self.nodes[actual].logical.start,
                                self.nodes[value].logical.end,
                            ),
                            origin: f.origin,
                            children,
                            len,
                            at: 0,
                            list: List::default(),
                            needs_shape: self.nodes[base].needs_shape
                                || self.nodes[value].needs_shape,
                        });
                    }
                }
                _ => {
                    let is_root = self.frames.len() == 1;
                    let f = self.frames.last_mut().unwrap();
                    f.children.push(&mut self.edges, value);
                    f.needs_shape |= self.nodes[value].needs_shape;
                    self.work.edges += 1;
                    if is_root {
                        self.root_version += 1
                    }
                }
            }
            return SyntaxStep::Progress;
        }
        if let Some(end) = self.close_matrix.take() {
            if !pay(b, 0) {
                self.close_matrix = Some(end);
                return SyntaxStep::BudgetExhausted;
            }
            self.work.steps += 1;
            let f = self.frames.pop().unwrap();
            let FrameKind::Matrix(fence) = f.kind else {
                self.fail(input, end);
                return SyntaxStep::Progress;
            };
            let kind = if f.columns > 0 && f.columns != f.column {
                self.error = true;
                Kind::Error
            } else {
                Kind::Matrix {
                    columns: f.columns.max(f.column),
                    fence,
                }
            };
            let id = self.node(
                input,
                kind,
                Span::new(f.start, end.end),
                f.origin,
                f.children,
                f.needs_shape,
            );
            self.value = Some(id);
            return SyntaxStep::Progress;
        }
        if self.closing {
            if !pay(b, 0) {
                return SyntaxStep::BudgetExhausted;
            }
            self.work.steps += 1;
            let f = self.frames.pop().unwrap();
            let root = matches!(f.kind, FrameKind::Row);
            if !root {
                self.error = true
            }
            let id = self.node(
                input,
                if self.error { Kind::Error } else { Kind::Row },
                Span::new(f.start, self.p),
                f.origin,
                f.children,
                f.needs_shape,
            );
            if root {
                self.root = Some(id);
                self.done = true;
                self.root_version += 1;
                return SyntaxStep::Complete;
            }
            self.value = Some(id);
            return SyntaxStep::Progress;
        }
        if let Some(token) = self.token {
            if !pay(b, 0) {
                return SyntaxStep::BudgetExhausted;
            }
            self.token = None;
            self.work.steps += 1;
            self.work.tokens += 1;
            self.parse(input, token);
            return SyntaxStep::Progress;
        }
        if self.p == input.len() {
            if !input.sealed {
                return SyntaxStep::Input;
            }
            if !pay(b, 0) {
                return SyntaxStep::BudgetExhausted;
            }
            self.work.steps += 1;
            self.token = Some(match self.lex {
                Lex::Slash(start) => {
                    let mut name = Name::new();
                    name.push(b'\\');
                    Token::Command(Span::new(start, self.p), name)
                }
                Lex::Word(start, name) => Token::Command(Span::new(start, self.p), name),
                Lex::Utf8 { .. } => {
                    self.error = true;
                    Token::End
                }
                Lex::Plain => Token::End,
            });
            self.lex = Lex::Plain;
            return SyntaxStep::Progress;
        }
        let byte = input.byte(self.p);
        match self.lex {
            Lex::Plain => {
                if !pay(b, 1) {
                    return SyntaxStep::BudgetExhausted;
                }
                let start = self.p;
                self.p += 1;
                self.work.bytes += 1;
                self.work.steps += 1;
                if byte == b'\\' {
                    self.lex = Lex::Slash(start)
                } else if byte < 128 {
                    self.token = Some(Token::Char(char::from(byte), Span::new(start, self.p)))
                } else {
                    let want = if byte < 0xe0 {
                        2
                    } else if byte < 0xf0 {
                        3
                    } else {
                        4
                    };
                    let mut bytes = [0; 4];
                    bytes[0] = byte;
                    self.lex = Lex::Utf8 {
                        start,
                        bytes,
                        used: 1,
                        want,
                    }
                }
                SyntaxStep::Progress
            }
            Lex::Slash(start) => {
                if byte >= 128 {
                    if !pay(b, 0) {
                        return SyntaxStep::BudgetExhausted;
                    }
                    let mut name = Name::new();
                    name.push(b'\\');
                    self.token = Some(Token::Command(Span::new(start, self.p), name));
                    self.lex = Lex::Plain;
                    self.work.steps += 1;
                    return SyntaxStep::Progress;
                }
                if !pay(b, 1) {
                    return SyntaxStep::BudgetExhausted;
                }
                self.p += 1;
                self.work.bytes += 1;
                self.work.steps += 1;
                let mut name = Name::new();
                name.push(b'\\');
                name.push(byte);
                if byte.is_ascii_alphabetic() {
                    self.lex = Lex::Word(start, name)
                } else {
                    self.lex = Lex::Plain;
                    self.token = Some(Token::Command(Span::new(start, self.p), name))
                }
                SyntaxStep::Progress
            }
            Lex::Word(start, mut name) => {
                if byte.is_ascii_alphabetic() {
                    if !pay(b, 1) {
                        return SyntaxStep::BudgetExhausted;
                    }
                    self.p += 1;
                    self.work.bytes += 1;
                    self.work.steps += 1;
                    name.push(byte);
                    self.lex = Lex::Word(start, name)
                } else {
                    if !pay(b, 0) {
                        return SyntaxStep::BudgetExhausted;
                    }
                    self.work.steps += 1;
                    self.token = Some(Token::Command(Span::new(start, self.p), name));
                    self.lex = Lex::Plain
                }
                SyntaxStep::Progress
            }
            Lex::Utf8 {
                start,
                mut bytes,
                mut used,
                want,
            } => {
                if !pay(b, 1) {
                    return SyntaxStep::BudgetExhausted;
                }
                bytes[used] = byte;
                used += 1;
                self.p += 1;
                self.work.bytes += 1;
                self.work.steps += 1;
                if used == want {
                    let ch = std::str::from_utf8(&bytes[..want])
                        .expect("typed payload UTF8")
                        .chars()
                        .next()
                        .unwrap();
                    self.token = Some(Token::Char(ch, Span::new(start, self.p)));
                    self.lex = Lex::Plain
                } else {
                    self.lex = Lex::Utf8 {
                        start,
                        bytes,
                        used,
                        want,
                    }
                }
                SyntaxStep::Progress
            }
        }
    }
    fn parse(&mut self, input: &Input, token: Token) {
        if let Token::End = token {
            if self.fence_next.take().is_some() {
                self.error = true
            }
            self.closing = true;
            return;
        }
        if let FrameKind::Env(end) = self.frames.last().unwrap().kind {
            let Token::Char(ch, span) = token else {
                self.fail(input, Span::new(self.p, self.p));
                return;
            };
            let f = self.frames.last_mut().unwrap();
            if ch == '{' && !f.env_open {
                f.env_open = true;
                return;
            }
            if ch != '}' {
                if f.env_open && ch.is_ascii() {
                    f.name.push(ch as u8)
                } else {
                    self.error = true
                }
                return;
            }
            let f = self.frames.pop().unwrap();
            let fence = match f.name.text() {
                Some("matrix") => ' ',
                Some("pmatrix") => '(',
                Some("bmatrix") => '[',
                Some("aligned") => '=',
                _ => {
                    self.fail(input, Span::new(f.start, span.end));
                    return;
                }
            };
            if !end {
                self.push(FrameKind::Matrix(fence), f.start, f.origin)
            } else {
                if !matches!(self.frames.last().map(|f|f.kind),Some(FrameKind::Matrix(c)) if c==fence)
                {
                    self.fail(input, span);
                    return;
                }
                self.marker(input);
                self.close_matrix = Some(span);
            }
            return;
        }
        if self.fence_next.is_some()
            && matches!(token, Token::Char(ch, _) if ch.is_ascii_whitespace())
        {
            return;
        }
        if let Some((close, command)) = self.fence_next.take() {
            let Token::Char(ch, span) = token else {
                self.fail(input, command);
                return;
            };
            if !matches!(ch, '(' | ')' | '[' | ']' | '|' | '.') {
                self.fail(input, span);
                return;
            }
            if close {
                self.flow += 1;
                if let Some(FrameKind::Fence(left, left_source)) =
                    self.frames.last().map(|f| f.kind)
                {
                    let f = self.frames.pop().unwrap();
                    let row = self.node(
                        input,
                        Kind::Row,
                        Span::new(f.start, span.end),
                        f.origin,
                        f.children,
                        f.needs_shape,
                    );
                    self.build = Some(SmallBuild {
                        kind: Kind::Fence {
                            left,
                            right: ch,
                            left_source,
                            right_source: input.root(Span::new(command.start, span.end)),
                        },
                        logical: Span::new(f.start, span.end),
                        origin: left_source,
                        children: [row; 3],
                        len: 1,
                        at: 0,
                        list: List::default(),
                        needs_shape: f.needs_shape,
                    })
                } else {
                    self.fail(input, span)
                }
            } else {
                self.push(
                    FrameKind::Fence(ch, input.root(Span::new(command.start, span.end))),
                    command.start,
                    input.root(command),
                )
            }
            return;
        }
        match token {
            Token::Char('[', span)
                if matches!(
                    self.frames.last().map(|f| f.kind),
                    Some(FrameKind::Call(Kind::Root, 1))
                ) =>
            {
                // Indexed roots are outside this finite language. Preserve a
                // visible object error instead of treating '[' as the radicand.
                self.fail(input, span)
            }
            Token::Char('{', span) => self.push(FrameKind::Group, span.start, input.root(span)),
            Token::Char('}', span) => {
                if matches!(self.frames.last().map(|f| f.kind), Some(FrameKind::Group)) {
                    let f = self.frames.pop().unwrap();
                    let id = self.node(
                        input,
                        Kind::Row,
                        Span::new(f.start, span.end),
                        f.origin,
                        f.children,
                        f.needs_shape,
                    );
                    self.value = Some(id)
                } else {
                    self.fail(input, span)
                }
            }
            Token::Char(ch @ ('^' | '_'), span) => {
                self.flow += 1;
                let is_root = self.frames.len() == 1;
                if let Some(base) = self
                    .frames
                    .last_mut()
                    .unwrap()
                    .children
                    .pop(&mut self.edges)
                {
                    self.push(
                        FrameKind::Script(base, ch == '^'),
                        span.start,
                        input.root(span),
                    );
                    if is_root {
                        self.root_version += 1
                    }
                } else {
                    self.fail(input, span)
                }
            }
            Token::Char('&', span) => {
                if matches!(
                    self.frames.last().map(|f| f.kind),
                    Some(FrameKind::Matrix(_))
                ) {
                    self.marker(input);
                    self.frames.last_mut().unwrap().column += 1
                } else {
                    self.leaf(
                        input,
                        Kind::Text {
                            ink: Ink::Payload(span),
                            width: Some(1),
                        },
                        span,
                        false,
                    )
                }
            }
            Token::Char(ch, span) => {
                if !ch.is_ascii_whitespace() {
                    let plain = ch.is_ascii() && !ch.is_control();
                    self.leaf(
                        input,
                        Kind::Text {
                            ink: Ink::Payload(span),
                            width: plain.then_some(1),
                        },
                        span,
                        !plain,
                    )
                }
            }
            Token::Command(span, name) => {
                let text = name.text();
                let root = input.root(span);
                if matches!(
                    text,
                    Some("\\frac" | "\\dfrac" | "\\tfrac" | "\\sqrt" | "\\left" | "\\begin")
                ) {
                    self.flow += 1;
                }
                match text {
                    Some("\\frac" | "\\dfrac" | "\\tfrac") => {
                        self.push(FrameKind::Call(Kind::Fraction, 2), span.start, root)
                    }
                    Some("\\sqrt") => self.push(FrameKind::Call(Kind::Root, 1), span.start, root),
                    Some("\\left") => self.fence_next = Some((false, span)),
                    Some("\\right") => self.fence_next = Some((true, span)),
                    Some("\\begin") => self.push(FrameKind::Env(false), span.start, root),
                    Some("\\end") => self.push(FrameKind::Env(true), span.start, root),
                    Some("\\\\")
                        if matches!(
                            self.frames.last().map(|f| f.kind),
                            Some(FrameKind::Matrix(_))
                        ) =>
                    {
                        self.marker(input);
                        let f = self.frames.last_mut().unwrap();
                        if f.columns > 0 && f.columns != f.column {
                            self.error = true
                        }
                        f.columns = f.column;
                        f.column = 1
                    }
                    Some("\\," | "\\;" | "\\quad" | "\\ ") => {
                        let width = if text == Some("\\quad") { 2 } else { 1 };
                        self.leaf(
                            input,
                            Kind::Text {
                                ink: Ink::Space(width),
                                width: Some(width),
                            },
                            span,
                            false,
                        )
                    }
                    _ => {
                        if let Some(label) = text.and_then(symbol) {
                            let width = crate::render::text::display_width(label);
                            self.leaf(
                                input,
                                Kind::Text {
                                    ink: Ink::Static(label),
                                    width: Some(width),
                                },
                                span,
                                false,
                            )
                        } else {
                            let needs_shape = text.is_some_and(|s| s.chars().any(char::is_control));
                            self.leaf(
                                input,
                                Kind::Text {
                                    ink: Ink::Payload(span),
                                    width: (!needs_shape).then_some(span.len()),
                                },
                                span,
                                needs_shape,
                            )
                        }
                    }
                }
            }
            Token::End => unreachable!(),
        }
    }
    pub fn storage(&self) -> [StorageWork; 3] {
        [self.nodes.stats(), self.edges.stats(), self.frames.stats()]
    }
    pub fn retire_one(&mut self) -> bool {
        self.value = None;
        self.build = None;
        self.token = None;
        if self.frames.pop().is_some() {
            return false;
        }
        if self.edges.pop().is_some() {
            return false;
        }
        self.nodes.pop();
        self.nodes.is_empty()
    }
}
