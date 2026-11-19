//! Resumable URI-reference percent serialization. This module does not parse,
//! resolve, select or decode Markdown targets.
use crate::{Budget, Completion, WorkLimit};
use std::{borrow::Cow, ops::Range};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UriWork {
    pub input_bytes: usize,
    pub output_bytes: usize,
}

/// One ASCII output atom. `input` indexes the immutable decoded input bytes;
/// it is not a Document source span and may index one byte of a UTF-8 scalar.
pub struct UriChunk<'a> {
    pub text: &'a str,
    pub input: Range<usize>,
}

/// Owns or borrows an immutable input for the entire continuation lifetime.
/// Reserved ASCII delimiters and valid existing `%HH` spelling are preserved.
/// Other bytes, including UTF-8 and malformed literal percent, become `%XX`.
/// This is serialization, not scheme-specific URI validation or normalization.
///
/// Borrowed `&str` and owned `String` become one fixed standard `Cow` at
/// construction; no provider callbacks run during queries or resumption. These
/// conversions neither allocate nor copy the input. Six bytes, one query and one
/// object per fresh Budget suffice for progress. All source probes are charged
/// before access, including lookahead used to recognize existing escapes.
pub struct UriEncoder<'a> {
    input: Cow<'a, str>,
    position: usize,
    start: usize,
    phase: Phase,
    output: [u8; 3],
    output_len: usize,
    work: UriWork,
}
#[derive(Clone, Copy)]
enum Phase {
    Read,
    Percent(usize),
    Emit,
}
impl<'a> UriEncoder<'a> {
    pub fn new(input: impl Into<Cow<'a, str>>) -> Self {
        Self {
            input: input.into(),
            position: 0,
            start: 0,
            phase: Phase::Read,
            output: [0; 3],
            output_len: 0,
            work: UriWork::default(),
        }
    }
    pub fn input(&self) -> &str {
        self.input.as_ref()
    }
    pub fn input_position(&self) -> usize {
        self.position
    }
    pub fn work(&self) -> UriWork {
        self.work
    }
    fn pay(b: &mut Budget, bytes: usize) -> Result<(), Completion> {
        if b.charge(WorkLimit {
            bytes,
            queries: 1,
            objects: 1,
            ..WorkLimit::default()
        }) {
            Ok(())
        } else {
            Err(Completion::BudgetExhausted)
        }
    }
    fn escaped(&mut self, byte: u8) {
        const HEX: &[u8; 16] = b"0123456789ABCDEF";
        self.output = [
            b'%',
            HEX[usize::from(byte >> 4)],
            HEX[usize::from(byte & 15)],
        ];
        self.output_len = 3;
        self.phase = Phase::Emit;
    }
    pub fn advance(&mut self, budget: &mut Budget) -> Result<Option<UriChunk<'_>>, Completion> {
        loop {
            match self.phase {
                Phase::Read => {
                    if self.position == self.input.as_ref().len() {
                        return Ok(None);
                    }
                    Self::pay(budget, 1)?;
                    let byte = self.input.as_ref().as_bytes()[self.position];
                    self.work.input_bytes += 1;
                    self.start = self.position;
                    self.position += 1;
                    if byte == b'%' {
                        self.phase = Phase::Percent(0);
                    } else if byte.is_ascii_alphanumeric()
                        || b"-._~:/?#[]@!$&'()*+,;=".contains(&byte)
                    {
                        self.output[0] = byte;
                        self.output_len = 1;
                        self.phase = Phase::Emit;
                    } else {
                        self.escaped(byte);
                    }
                }
                Phase::Percent(peek) => {
                    let at = self.position + peek;
                    if at >= self.input.as_ref().len() {
                        self.escaped(b'%');
                        continue;
                    }
                    Self::pay(budget, 1)?;
                    let byte = self.input.as_ref().as_bytes()[at];
                    self.work.input_bytes += 1;
                    if !byte.is_ascii_hexdigit() {
                        self.escaped(b'%');
                    } else if peek == 1 {
                        self.output[0] = b'%';
                        self.output_len = 1;
                        self.phase = Phase::Emit;
                    } else {
                        self.phase = Phase::Percent(1);
                    }
                }
                Phase::Emit => {
                    Self::pay(budget, self.output_len)?;
                    self.work.output_bytes += self.output_len;
                    self.phase = Phase::Read;
                    return Ok(Some(UriChunk {
                        text: std::str::from_utf8(&self.output[..self.output_len])
                            .expect("ASCII URI atom"),
                        input: self.start..self.position,
                    }));
                }
            }
        }
    }
}
