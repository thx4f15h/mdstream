use crate::{Budget, CodeInfo, Completion, Document, LinkId, Span};

impl Document {
    /// O(1) reader of the currently resolved link or image title, decoding ASCII
    /// punctuation escapes and semicolon-terminated named/numeric entities.
    /// Returns None for an invalid ID, unresolved reference or absent title.
    /// An explicitly empty title returns a completed cursor.
    ///
    /// Reads only the committed title span (without its delimiters), skipping
    /// indexed structural container prefixes inside the raw source envelope. Reference
    /// titles map to the winning definition, not to the link's use site. Raw
    /// `link_title` text, destination/URL and label semantics remain unchanged.
    /// Physical CRLF emits one LF mapped to both source bytes; LF is retained.
    /// No trimming, inline parsing or recursive decoding is performed. The
    /// borrow prevents document changes while the cursor is alive.
    ///
    /// ```
    /// use mdstream::{Budget, Document, LinkId, WorkLimit};
    /// let mut doc = Document::new();
    /// doc.append("[label][r]\n\n[r]: /url \"a\\+b &ouml;\"\n").unwrap();
    /// let mut title = doc.decoded_link_title(LinkId(0)).unwrap();
    /// let mut budget = Budget::new(WorkLimit::unlimited());
    /// let mut text = String::new();
    /// while let Some(chunk) = title.advance(&mut budget).unwrap() {
    ///     text.push_str(chunk.text);
    /// }
    /// assert_eq!(text, "a+b ö");
    /// assert_eq!(doc.link_title(LinkId(0)), Some("a\\+b &ouml;"));
    /// ```
    pub fn decoded_link_title(&self, id: LinkId) -> Option<LinkTitle<'_>> {
        let gaps = match self.links.get(id.0)? {
            crate::LinkTarget::Reference(symbol) => &self.references.title_gaps[symbol.0][..],
            _ => &[],
        };
        Some(LinkTitle {
            inner: CodeInfo::title(self.source(), self.link_title_span(id)?, gaps),
        })
    }
}

/// One scalar or decoded entity, mapped to its complete original title bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LinkTitleChunk<'a> {
    pub source: Span,
    /// Borrows source or cursor scratch, valid until the next cursor step.
    pub text: &'a str,
}

/// Fixed-size, allocation-free title cursor. Total reading work is O(title
/// bytes); it does not visit link uses, labels, bodies or container ancestors.
pub struct LinkTitle<'a> {
    inner: CodeInfo<'a>,
}

impl LinkTitle<'_> {
    /// First source byte not emitted or skipped; a paused probe may inspect ahead.
    pub fn source_position(&self) -> usize {
        self.inner.source_position()
    }

    /// Uses the same charging and resumable probe contract as
    /// [`CodeInfo::advance`]. Each scalar/escape costs a query, an object and its
    /// source bytes (including both bytes of a normalized CRLF). Entity probes
    /// charge inspected bytes incrementally and a query; successful emission
    /// charges output bytes, a query and an object.
    /// A probe may exhaust budget without emitting or advancing source_position.
    /// Structural prefix gaps cost one query and their source bytes, charged
    /// incrementally; a nonzero object allowance is required. Skipping may
    /// advance source_position without emitting. Semantic indentation is kept.
    /// Six bytes, one query and one object per fresh frame guarantee progress.
    /// Output is atomic, and calls after completion are free.
    pub fn advance(
        &mut self,
        budget: &mut Budget,
    ) -> Result<Option<LinkTitleChunk<'_>>, Completion> {
        Ok(self.inner.advance(budget)?.map(|chunk| LinkTitleChunk {
            source: chunk.source,
            text: chunk.text,
        }))
    }
}
