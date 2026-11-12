use crate::{
    BlockId, Budget, ContainerKind, Document, DocumentId, FoldId, FoldView, LayoutError,
    SourceFoldId, SourceFoldProgress, SourceFoldRegistration, Span,
};

/// Explicit host identity supplied by the application. Document-local IDs must
/// be mapped by the caller after reload; they are never treated as persistent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceFoldHost {
    Document,
    Syntax(FoldId),
    Paragraph(BlockId),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Shape {
    Document,
    Heading,
    Code,
    Math,
    Table,
    Quote,
    List,
    Item,
    Paragraph,
}

impl SourceFoldHost {
    fn resolve(self, doc: &Document) -> Result<(Span, Shape), LayoutError> {
        let result = match self {
            Self::Document => Some((Span::new(0, doc.source().len()), Shape::Document)),
            Self::Paragraph(id) => doc
                .open_paragraph_source(id)
                .map(|(s, _)| (s, Shape::Paragraph)),
            Self::Syntax(id) => doc.fold_candidate(id).and_then(|c| {
                let shape = match id {
                    FoldId::Heading(_) => Shape::Heading,
                    FoldId::Code(_) => Shape::Code,
                    FoldId::Math(_) => Shape::Math,
                    FoldId::Table(_) => Shape::Table,
                    FoldId::Container(id) => match doc.container(id)?.kind {
                        ContainerKind::Quote => Shape::Quote,
                        ContainerKind::List { .. } => Shape::List,
                        ContainerKind::Item { .. } => Shape::Item,
                    },
                    FoldId::Source(_) | FoldId::TableRows(_) => return None,
                };
                Some((c.source, shape))
            }),
        };
        result.ok_or(LayoutError::InvalidRange)
    }
}

/// Constant-size source cuts relative to an explicitly chosen host. No body or
/// ancestor chain is copied. This value is an in-process mapping record, not a
/// serialization format or a content fingerprint. Persist an application host
/// key alongside it and supply that key's mapped host to `SourceFoldRestore`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SourceFoldSnapshot {
    pub document: DocumentId,
    pub id: SourceFoldId,
    pub host: SourceFoldHost,
    source: Span,
    preview_bytes: Option<usize>,
    shape: Shape,
    open: bool,
    tail: bool,
    flexible: bool,
    collapsed: bool,
}

impl SourceFoldSnapshot {
    /// Metadata lookup only. Fixed registrations require a containing envelope;
    /// paragraph registrations use their exact paragraph host. A document tail
    /// may use a syntax/paragraph host containing its start, including after
    /// finish or after that host has closed;
    /// that host maps the cut, while the tail still follows the document EOF.
    /// Preview's saved local choice is captured, not transient preview state.
    pub fn capture<S>(
        document: &Document,
        view: &FoldView<S>,
        id: SourceFoldId,
        host: SourceFoldHost,
    ) -> Result<Self, LayoutError> {
        let c = view
            .fold_candidate(document, FoldId::Source(id))?
            .ok_or(LayoutError::InvalidRange)?;
        let (span, shape) = host.resolve(document)?;
        let paragraph = view.paragraph_fold_block(document, id)?;
        let live_tail = view.source_fold_is_tail(id);
        if span.start > c.source.start
            || (live_tail && c.source.start >= span.end)
            || (!live_tail && c.source.end > span.end)
            || paragraph.is_some_and(|p| host != SourceFoldHost::Paragraph(p))
        {
            return Err(LayoutError::InvalidRange);
        }
        Ok(Self {
            document: document.id(),
            id,
            host,
            source: Span::new(c.source.start - span.start, c.source.end - span.start),
            preview_bytes: paragraph.map(|_| c.hidden.start - c.source.start),
            shape,
            open: c.open,
            tail: live_tail,
            flexible: view.source_fold_is_flexible(id),
            collapsed: view.is_collapsed(c.id),
        })
    }
}

/// Explicit, revision/view-bound recovery. The caller asserts correspondence of
/// host identity and unchanged relative cuts; this task checks host kind, bounds,
/// UTF-8, paragraph lifetime and registration rules. It never searches for a
/// similar object or promises unambiguous recovery after arbitrary editing.
/// Advance shares the frame Budget, including a recovery unit for publication.
/// Cancellation/staleness leaves no partial registration or restored choice.
pub struct SourceFoldRestore {
    previous: SourceFoldId,
    registration: SourceFoldRegistration,
}

impl SourceFoldRestore {
    pub fn new<S>(
        document: &Document,
        view: &FoldView<S>,
        saved: &SourceFoldSnapshot,
        mapped_host: SourceFoldHost,
    ) -> Result<Self, LayoutError> {
        view.check(document)?;
        let (host, shape) = mapped_host.resolve(document)?;
        if shape != saved.shape {
            return Err(LayoutError::InvalidRange);
        }
        let mut registration = if let Some(prefix) = saved.preview_bytes {
            let SourceFoldHost::Paragraph(block) = mapped_host else {
                return Err(LayoutError::InvalidRange);
            };
            let entry = document.block(block).ok_or(LayoutError::InvalidRange)?;
            if saved.flexible {
                SourceFoldRegistration::paragraph_overlay(document, view, block, prefix)?
            } else if entry.container.is_some() {
                SourceFoldRegistration::container_paragraph(document, view, block, prefix)?
            } else if document
                .open_paragraph_source(block)
                .is_some_and(|(_, open)| open)
            {
                SourceFoldRegistration::open_paragraph(document, view, block, prefix)?
            } else {
                SourceFoldRegistration::paragraph(document, view, block, prefix)?
            }
        } else {
            let start = host
                .start
                .checked_add(saved.source.start)
                .ok_or(LayoutError::InvalidRange)?;
            if saved.open && !document.is_finished() {
                if start >= host.end {
                    return Err(LayoutError::InvalidRange);
                }
                if saved.flexible {
                    SourceFoldRegistration::stream(document, view, start)?
                } else {
                    SourceFoldRegistration::tail(document, view, start)?
                }
            } else {
                let end = if saved.open {
                    document.source().len()
                } else {
                    host.start
                        .checked_add(saved.source.end)
                        .ok_or(LayoutError::InvalidRange)?
                };
                if (saved.tail && start >= host.end)
                    || (!saved.tail && end > host.end)
                    || end > document.source().len()
                {
                    return Err(LayoutError::InvalidRange);
                }
                let source = Span::new(start, end);
                if saved.flexible {
                    SourceFoldRegistration::overlapping(document, view, source)?
                } else {
                    SourceFoldRegistration::new(document, view, source)?
                }
            }
        };
        registration.restore_choice(saved.collapsed);
        Ok(Self {
            previous: saved.id,
            registration,
        })
    }

    pub fn previous_id(&self) -> SourceFoldId {
        self.previous
    }

    /// On Covered, progress.id is the new view-local identity corresponding to
    /// previous_id(). Repeated completed calls are free and return that same ID.
    pub fn advance<S>(
        &mut self,
        document: &Document,
        view: &mut FoldView<S>,
        budget: &mut Budget,
    ) -> Result<SourceFoldProgress, LayoutError> {
        self.registration.advance(document, view, budget)
    }
}
