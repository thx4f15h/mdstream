//! Source-coordinate normalization, independent of Markdown classification.
//! Keeping both raw and code summaries makes late Setext/code classification a
//! metadata operation. Only the newly appended scalar is decoded/normalized.
use super::{
    pages::BYTE_PAGE,
    rope::{CODE, Memory, RAW, Rope, Small},
    runtime::Op,
};
use crate::Span;
use std::collections::BTreeMap;

struct Page {
    raw: [usize; BYTE_PAGE + 1],
    code: [usize; BYTE_PAGE + 1],
    flags: [u8; BYTE_PAGE],
}
impl Default for Page {
    fn default() -> Self {
        Self {
            raw: [0; BYTE_PAGE + 1],
            code: [0; BYTE_PAGE + 1],
            flags: [0; BYTE_PAGE],
        }
    }
}
#[derive(Default)]
pub(super) struct Tape {
    pages: BTreeMap<usize, Box<Page>>,
    pub len: usize,
    previous_cr: bool,
}

pub(super) fn normalized(ch: char) -> Small {
    let mut out = Small::EMPTY;
    let mut push = |ch: char| {
        let start = usize::from(out.len);
        out.len += ch.encode_utf8(&mut out.bytes[start..]).len() as u8;
    };
    if ch.is_alphanumeric() {
        for lower in ch.to_lowercase() {
            push(lower);
        }
    } else if ch == '-' || ch == '_' {
        push(ch);
    } else if ch.is_whitespace() {
        push('-');
    }
    out
}
impl Tape {
    fn set(&mut self, pos: usize, raw: usize, code: usize) {
        let page = self.pages.entry(pos / BYTE_PAGE).or_default();
        page.raw[pos % BYTE_PAGE] = raw;
        page.code[pos % BYTE_PAGE] = code;
        if pos != 0 && pos.is_multiple_of(BYTE_PAGE) {
            let previous = self.pages.get_mut(&(pos / BYTE_PAGE - 1)).unwrap();
            previous.raw[BYTE_PAGE] = raw;
            previous.code[BYTE_PAGE] = code;
        }
    }
    fn prefix(&self, pos: usize, code: bool) -> usize {
        assert!(pos <= self.len);
        let p = &self.pages[&(pos / BYTE_PAGE)];
        if code {
            p.code[pos % BYTE_PAGE]
        } else {
            p.raw[pos % BYTE_PAGE]
        }
    }
    fn flag(&self, pos: usize) -> u8 {
        if pos >= self.len {
            return 0;
        }
        self.pages[&(pos / BYTE_PAGE)].flags[pos % BYTE_PAGE]
    }
    pub async fn append(&mut self, mem: &mut Memory, ch: char) -> Option<Span> {
        mem.cx.charge(Op::Normalize(ch.len_utf8())).await;
        let out = normalized(ch);
        let raw_start = mem.buffer_len(RAW);
        let code_end = mem.buffer_len(CODE);
        let crlf = ch == '\n' && self.previous_cr;
        let code_start = code_end - usize::from(crlf);
        for i in 0..usize::from(out.len) {
            mem.cx.charge(Op::Emit).await;
            mem.push(RAW, out.get(i));
            if !crlf {
                mem.cx.charge(Op::Copy).await;
                mem.push(CODE, out.get(i));
            }
        }
        for at in self.len..self.len + ch.len_utf8() {
            self.set(at, raw_start, code_start);
        }
        self.pages.get_mut(&(self.len / BYTE_PAGE)).unwrap().flags[self.len % BYTE_PAGE] = match ch
        {
            '\r' => 1,
            '\n' => 2,
            _ => 0,
        };
        let old = self.len;
        self.len += ch.len_utf8();
        self.set(self.len, mem.buffer_len(RAW), mem.buffer_len(CODE));
        mem.cx
            .storage(5, self.capacity(), self.directory_entries(), 0);
        self.previous_cr = ch == '\r';
        crlf.then(|| Span::new(old - 1, self.len))
    }
    pub async fn raw(&self, mem: &mut Memory, source: Span) -> Rope {
        mem.cx.charge(Op::Rope).await;
        let a = self.prefix(source.start, false);
        let b = self.prefix(source.end, false);
        mem.span(RAW, a, b - a)
    }
    pub async fn code(&self, mem: &mut Memory, source: Span, context: Span) -> Rope {
        mem.cx.charge(Op::Rope).await;
        let a = self.prefix(source.start, true);
        let b = self.prefix(source.end, true);
        let mut rope = mem.span(CODE, a, b - a);
        // Canonical code conversion examines CRLF inside the code envelope,
        // independently of visibility/style boundaries inside that envelope.
        if source.start == context.start
            && self.flag(source.start) == 2
            && source.start != 0
            && self.flag(source.start - 1) == 1
        {
            rope = mem.join(Rope::Small(Small::from_bytes(b"-")), rope).await;
        }
        if source.end == context.end
            && source.end != 0
            && self.flag(source.end - 1) == 1
            && self.flag(source.end) == 2
        {
            rope = mem.join(rope, Rope::Small(Small::from_bytes(b"-"))).await;
        }
        rope
    }
    pub fn capacity(&self) -> usize {
        self.pages.len() * (std::mem::size_of::<Page>() + std::mem::size_of::<(usize, Box<Page>)>())
    }
    pub fn directory_entries(&self) -> usize {
        self.pages.len()
    }
    pub fn retire_page(&mut self) -> bool {
        self.pages.pop_first().is_some()
    }
}
