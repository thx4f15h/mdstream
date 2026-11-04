use crate::{Budget, CodeInfo, Completion, Document, LinkId, LinkTarget, Span};

impl Document {
    /// O(1) reader of a resolved link or image destination. Explicit destinations
    /// decode ASCII punctuation escapes and semicolon-terminated named/numeric
    /// entities; URI and email autolinks retain their literal source text.
    /// Returns None for invalid IDs or unresolved references. Empty destinations
    /// return a completed cursor. Reference chunks map to the winning definition.
    ///
    /// Reads only the committed destination, without delimiters. No trimming,
    /// recursive decoding, URL encoding/filtering or implied mailto: is applied.
    /// Raw `link_destination` and `link_url` retain their existing behavior. The
    /// borrow prevents document changes while the cursor is alive.
    ///
    /// ```
    /// use mdstream::{Budget, Document, LinkId, WorkLimit};
    /// let mut doc = Document::new();
    /// doc.append("[label][r]\n\n[r]: /a\\+b?x=1&amp;y=2\n").unwrap();
    /// let mut target = doc.decoded_link_destination(LinkId(0)).unwrap();
    /// let mut budget = Budget::new(WorkLimit::unlimited());
    /// let mut text = String::new();
    /// while let Some(chunk) = target.advance(&mut budget).unwrap() {
    ///     text.push_str(chunk.text);
    /// }
    /// assert_eq!(text, "/a+b?x=1&y=2");
    /// assert_eq!(doc.link_destination(LinkId(0)), Some("/a\\+b?x=1&amp;y=2"));
    /// ```
    pub fn decoded_link_destination(&self, id: LinkId) -> Option<LinkDestination<'_>> {
        let span = self.link_destination_span(id)?;
        // Only autolinks use the same span for their label and direct target.
        // References may coincidentally share spans with other objects, so their
        // resolved destination must always use explicit-destination decoding.
        let literal = match self.links.get(id.0)? {
            LinkTarget::Email(_) => true,
            LinkTarget::Direct(_) => self.link_info(id)?.label == span,
            LinkTarget::Reference(_) => false,
        };
        Some(LinkDestination {
            inner: if literal {
                CodeInfo::literal(self.source(), span)
            } else {
                CodeInfo::new(self.source(), span)
            },
        })
    }
}

/// One scalar or decoded entity, mapped to its complete destination source bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LinkDestinationChunk<'a> {
    pub source: Span,
    /// Borrows source or cursor scratch, valid until the next cursor step.
    pub text: &'a str,
}

/// Fixed-size, allocation-free cursor. Total work is O(destination bytes),
/// independent of labels, document size, reference use count or container depth.
pub struct LinkDestination<'a> {
    inner: CodeInfo<'a>,
}

impl LinkDestination<'_> {
    /// First source byte not emitted; an entity probe may have inspected ahead.
    pub fn source_position(&self) -> usize {
        self.inner.source_position()
    }

    /// Shares [`CodeInfo::advance`]'s charging and resumable probe contract.
    /// Autolinks cost one query, one object and source bytes per literal scalar.
    /// Six bytes, one query and one object per fresh frame guarantee progress.
    /// Probes may consume budget without output or a changed source_position;
    /// emission is atomic, and calls after completion are free.
    pub fn advance(
        &mut self,
        budget: &mut Budget,
    ) -> Result<Option<LinkDestinationChunk<'_>>, Completion> {
        Ok(self
            .inner
            .advance(budget)?
            .map(|chunk| LinkDestinationChunk {
                source: chunk.source,
                text: chunk.text,
            }))
    }
}
