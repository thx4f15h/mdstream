use crate::{BlockId, BlockKind, Budget, Completion, Document, Span, WorkLimit};

impl Document {
    /// O(1) budgeted reader of a code block's committed info string, decoding
    /// backslash escapes of ASCII punctuation and semicolon-terminated entities.
    /// Returns None for invalid IDs, other blocks, or absent info.
    ///
    /// The original `BlockKind::Code::info` range and `Document::text` remain
    /// unchanged. Long opening lines commit their complete info at LF/EOF;
    /// provisional raw candidates expose no code-info reader. No language selection,
    /// further trimming, inline parsing or recursive decoding is performed.
    /// The borrow prevents appends while this cursor is being consumed.
    ///
    /// ```
    /// use mdstream::{BlockId, Budget, Document, WorkLimit};
    /// let mut doc = Document::new();
    /// doc.append("~~~ foo\\+bar &ouml;\nbody\n~~~\n").unwrap();
    /// let mut info = doc.code_info(BlockId(0)).unwrap();
    /// let mut budget = Budget::new(WorkLimit::unlimited());
    /// let mut decoded = String::new();
    /// while let Some(chunk) = info.advance(&mut budget).unwrap() {
    ///     decoded.push_str(chunk.text);
    /// }
    /// assert_eq!(decoded, "foo+bar ö");
    /// ```
    pub fn code_info(&self, id: BlockId) -> Option<CodeInfo<'_>> {
        let BlockKind::Code { info: Some(info) } = self.block(id)?.kind else {
            return None;
        };
        Some(CodeInfo::new(self.source(), info))
    }
}

/// One scalar or decoded entity (possibly two scalars), with its original bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CodeInfoChunk<'a> {
    /// The complete original scalar, escape pair, or entity reference.
    pub source: Span,
    /// Borrows source text or cursor scratch, valid until the next cursor step.
    pub text: &'a str,
}

/// O(1) cursor state, no allocation or document mutation. Reading the entire
/// committed info takes O(info bytes); code bodies and other blocks are not read.
pub struct CodeInfo<'a> {
    source: &'a str,
    remaining: Span,
    probe: Probe,
    decoded: [u8; 8],
    literal: bool,
    normalize_crlf: bool,
    gaps: &'a [Span],
    spaces: &'a [(Span, u8)],
}

#[derive(Clone, Copy)]
enum Probe {
    Start,
    Scanning(usize),
    // text_len <= 8 fits the fixed decoded buffer and keeps the cursor <=96B.
    // text_len == 0 means a failed candidate: emit the literal '&'.
    Ready { source_len: usize, text_len: u8 },
}

impl<'a> CodeInfo<'a> {
    // Internal reuse for committed metadata spans with the same decoding rules.
    pub(crate) fn new(source: &'a str, remaining: Span) -> Self {
        Self {
            source,
            remaining,
            probe: Probe::Start,
            decoded: [0; 8],
            literal: false,
            normalize_crlf: false,
            gaps: &[],
            spaces: &[],
        }
    }

    pub(crate) fn title(source: &'a str, remaining: Span, gaps: &'a [Span]) -> Self {
        Self {
            normalize_crlf: true,
            gaps,
            ..Self::new(source, remaining)
        }
    }

    pub(crate) fn title_with_spaces(
        source: &'a str,
        remaining: Span,
        gaps: &'a [Span],
        spaces: &'a [(Span, u8)],
    ) -> Self {
        Self {
            spaces,
            ..Self::title(source, remaining, gaps)
        }
    }

    // Autolink destinations retain literal backslashes and ampersands.
    pub(crate) fn literal(source: &'a str, remaining: Span) -> Self {
        Self {
            literal: true,
            ..Self::new(source, remaining)
        }
    }

    /// First source byte not yet emitted; a paused probe may have inspected ahead.
    pub fn source_position(&self) -> usize {
        self.remaining.start
    }

    /// Ordinary scalars/escapes cost one query, one object and their source
    /// bytes (up to four). An '&' probe costs an additional query and up to 33
    /// inspected bytes, charged incrementally; its bounded table lookup is
    /// included in that query. Successful entities then cost one query, one
    /// object and their output bytes (up to six). Failed probes emit a literal
    /// '&' with the ordinary scalar cost; their following bytes are read again.
    ///
    /// Probes can consume budget without producing a chunk or changing
    /// source_position(). Exhaustion preserves probe progress, including its
    /// decoded result. Emission itself is atomic. Six bytes, one query and one
    /// object per fresh frame guarantee progress; completed calls are free.
    /// Escaped ampersands are literal; decoded text is never decoded again.
    pub fn advance(
        &mut self,
        budget: &mut Budget,
    ) -> Result<Option<CodeInfoChunk<'_>>, Completion> {
        while let Some(gap) = self.gaps.first()
            && self.remaining.start >= gap.start
        {
            debug_assert!(self.remaining.start < gap.end && gap.end <= self.remaining.end);
            let available = budget.remaining();
            if available.queries == 0 || available.objects == 0 || available.bytes == 0 {
                return Err(Completion::BudgetExhausted);
            }
            let bytes = (gap.end - self.remaining.start).min(available.bytes);
            let charged = budget.charge(WorkLimit {
                queries: 1,
                bytes,
                ..WorkLimit::default()
            });
            debug_assert!(charged);
            self.remaining.start += bytes;
            if self.remaining.start == gap.end {
                self.gaps = &self.gaps[1..];
            }
        }
        if self.remaining.is_empty() {
            return Ok(None);
        }
        let available = budget.remaining();
        if available.queries == 0 || available.objects == 0 || available.bytes == 0 {
            return Err(Completion::BudgetExhausted);
        }
        if let Some(&(source, columns)) = self.spaces.first()
            && source.start == self.remaining.start
        {
            if !budget.charge(WorkLimit {
                queries: 1,
                objects: 1,
                bytes: source.len().max(usize::from(columns)),
                ..Default::default()
            }) {
                return Err(Completion::BudgetExhausted);
            }
            self.remaining.start = source.end;
            self.spaces = &self.spaces[1..];
            return Ok(Some(CodeInfoChunk {
                source,
                text: &"   "[..usize::from(columns)],
            }));
        }
        let text = &self.source[self.remaining.range()];
        if !self.literal && text.as_bytes()[0] == b'&' {
            if matches!(self.probe, Probe::Start) {
                if !budget.charge(WorkLimit {
                    queries: 1,
                    bytes: 1,
                    ..WorkLimit::default()
                }) {
                    return Err(Completion::BudgetExhausted);
                }
                self.probe = Probe::Scanning(1);
            }
            while let Probe::Scanning(offset) = self.probe {
                if offset == text.len() {
                    self.probe = Probe::Ready {
                        source_len: 1,
                        text_len: 0,
                    };
                    break;
                }
                if !budget.charge(WorkLimit {
                    bytes: 1,
                    ..WorkLimit::default()
                }) {
                    return Err(Completion::BudgetExhausted);
                }
                let byte = text.as_bytes()[offset];
                if byte == b';' {
                    let mut scratch = [0; 4];
                    let decoded = crate::entity::decode_into(&text[1..offset], &mut scratch);
                    self.probe = if let Some(decoded) = decoded {
                        self.decoded[..decoded.len()].copy_from_slice(decoded.as_bytes());
                        Probe::Ready {
                            source_len: offset + 1,
                            text_len: decoded.len() as u8,
                        }
                    } else {
                        Probe::Ready {
                            source_len: 1,
                            text_len: 0,
                        }
                    };
                } else if offset > crate::entity::MAX_ENTITY_BODY
                    || !(byte.is_ascii_alphanumeric() || byte == b'#')
                {
                    self.probe = Probe::Ready {
                        source_len: 1,
                        text_len: 0,
                    };
                } else {
                    self.probe = Probe::Scanning(offset + 1);
                }
            }
            if let Probe::Ready {
                source_len,
                text_len,
            } = self.probe
                && text_len != 0
            {
                if !budget.charge(WorkLimit {
                    queries: 1,
                    objects: 1,
                    bytes: usize::from(text_len),
                    ..WorkLimit::default()
                }) {
                    return Err(Completion::BudgetExhausted);
                }
                let source = Span::new(self.remaining.start, self.remaining.start + source_len);
                self.remaining.start = source.end;
                self.probe = Probe::Start;
                return Ok(Some(CodeInfoChunk {
                    source,
                    text: std::str::from_utf8(&self.decoded[..usize::from(text_len)]).unwrap(),
                }));
            }
        }
        let escaped = !self.literal
            && text.as_bytes()[0] == b'\\'
            && text.as_bytes().get(1).is_some_and(u8::is_ascii_punctuation);
        let crlf = self.normalize_crlf && text.starts_with("\r\n");
        let len = if escaped || crlf {
            2
        } else {
            text.chars().next().unwrap().len_utf8()
        };
        if !budget.charge(WorkLimit {
            queries: 1,
            objects: 1,
            bytes: len,
            ..WorkLimit::default()
        }) {
            return Err(Completion::BudgetExhausted);
        }
        let source = Span::new(self.remaining.start, self.remaining.start + len);
        self.remaining.start = source.end;
        self.probe = Probe::Start;
        Ok(Some(CodeInfoChunk {
            source,
            text: &text[usize::from(escaped || crlf)..len],
        }))
    }
}

/// Owned fixed decoder continuation used by revision-spanning destination tasks.
pub(crate) struct DetachedCodeInfo {
    remaining: Span,
    probe: Probe,
    decoded: [u8; 8],
    literal: bool,
}
impl DetachedCodeInfo {
    pub(crate) fn new(source: Span, literal: bool) -> Self {
        Self {
            remaining: source,
            probe: Probe::Start,
            decoded: [0; 8],
            literal,
        }
    }
    pub(crate) fn advance<'a>(
        &'a mut self,
        source: &str,
        b: &mut Budget,
    ) -> Result<Option<CodeInfoChunk<'a>>, Completion> {
        let mut cursor = CodeInfo::new(source, self.remaining);
        cursor.probe = self.probe;
        cursor.decoded = self.decoded;
        cursor.literal = self.literal;
        let result = match cursor.advance(b) {
            Ok(Some(chunk)) => {
                let mut bytes = [0; 8];
                bytes[..chunk.text.len()].copy_from_slice(chunk.text.as_bytes());
                Ok(Some((chunk.source, bytes, chunk.text.len())))
            }
            Ok(None) => Ok(None),
            Err(e) => Err(e),
        };
        self.remaining = cursor.remaining;
        self.probe = cursor.probe;
        self.decoded = cursor.decoded;
        match result {
            Ok(Some((source, bytes, len))) => {
                self.decoded = bytes;
                Ok(Some(CodeInfoChunk {
                    source,
                    text: std::str::from_utf8(&self.decoded[..len]).unwrap(),
                }))
            }
            Ok(None) => Ok(None),
            Err(e) => Err(e),
        }
    }
}
