//! Thin hosted adapters around the existing query and layout machines.
use super::*;

/// A decoded metadata chunk with an explicit original or projected address.
pub struct BodyTextChunk<'a> {
    pub source: BodyAddress,
    pub text: &'a str,
}
struct Target<'a> {
    view: BodyView<'a>,
    id: BodyId<crate::LinkId>,
    title: bool,
}
pub struct BodyTextCursor<'a> {
    origin: Option<BodyKey>,
    document: DocumentId,
    inner: Option<crate::CodeInfo<'a>>,
    target: Option<Target<'a>>,
}

/// Owned decoder for one explicitly addressed body link target. Reuses the
/// same detached scalar/entity machine as the root destination task.
pub struct BodyDestinationTask {
    id: BodyId<crate::LinkId>,
    revision: u64,
    source: Option<BodyAddress>,
    literal: bool,
    resolved: bool,
    inner: Option<crate::code_info::DetachedCodeInfo>,
}
impl BodyView<'_> {
    pub fn link_destination_task(
        self,
        id: BodyId<crate::LinkId>,
    ) -> Result<BodyDestinationTask, LayoutError> {
        self.check(id.body())?;
        Ok(BodyDestinationTask {
            id,
            revision: self.host.revision(),
            source: None,
            literal: false,
            resolved: false,
            inner: None,
        })
    }
}
impl BodyDestinationTask {
    pub fn source(&self) -> Option<BodyAddress> {
        self.source
    }
    pub fn advance<'a>(
        &'a mut self,
        view: BodyView<'_>,
        budget: &mut Budget,
    ) -> Result<Result<Option<BodyTextChunk<'a>>, Completion>, LayoutError> {
        view.check(self.id.body())?;
        if !self.resolved || self.revision != view.host.revision() {
            if !budget.charge(WorkLimit {
                queries: 1,
                objects: 1,
                dependencies: 1,
                ..Default::default()
            }) {
                return Ok(Err(Completion::BudgetExhausted));
            }
            let source = view.link_destination(self.id)?;
            let literal = match view.body.engine.links.get(self.id.local().0) {
                Some(crate::LinkTarget::Email(_) | crate::LinkTarget::Www(_)) => true,
                Some(crate::LinkTarget::Direct(span)) => view
                    .body
                    .engine
                    .link_info(self.id.local())
                    .is_some_and(|i| i.label == *span),
                _ => false,
            };
            if self.resolved && (source != self.source || literal != self.literal) {
                return Err(LayoutError::StaleContinuation);
            }
            if !self.resolved {
                self.source = source;
                self.literal = literal;
                self.inner = source.map(|address| {
                    crate::code_info::DetachedCodeInfo::new(
                        match address {
                            BodyAddress::Original { span, .. } => span,
                            BodyAddress::Projected(span) => span.span,
                        },
                        literal,
                    )
                });
                self.resolved = true;
            }
            self.revision = view.host.revision();
        }
        let Some(inner) = self.inner.as_mut() else {
            return Ok(Ok(None));
        };
        let source = match self.source.unwrap() {
            BodyAddress::Original { document, .. } => {
                if document != view.host.id() {
                    return Err(LayoutError::WrongDocument);
                }
                view.host.source()
            }
            BodyAddress::Projected(span) => {
                view.check(span.body)?;
                view.body.engine.source()
            }
        };
        Ok(inner.advance(source, budget).map(|chunk| {
            chunk.map(|chunk| BodyTextChunk {
                source: match self.source.unwrap() {
                    BodyAddress::Original { document, .. } => BodyAddress::Original {
                        document,
                        span: chunk.source,
                    },
                    BodyAddress::Projected(span) => BodyAddress::Projected(BodySpan {
                        body: span.body,
                        span: chunk.source,
                    }),
                },
                text: chunk.text,
            })
        }))
    }
}
impl<'a> BodyTextCursor<'a> {
    pub(super) fn projected(
        key: BodyKey,
        document: DocumentId,
        inner: crate::CodeInfo<'a>,
    ) -> Self {
        Self {
            origin: Some(key),
            document,
            inner: Some(inner),
            target: None,
        }
    }
    pub fn advance(
        &mut self,
        budget: &mut Budget,
    ) -> Result<Option<BodyTextChunk<'_>>, Completion> {
        if let Some(request) = &self.target {
            let view = request.view;
            let id = request.id;
            let local = view.body.engine.links[id.local.0];
            if !budget.charge(WorkLimit {
                queries: 1,
                objects: 1,
                dependencies: usize::from(matches!(local, crate::LinkTarget::Reference(_))),
                ..Default::default()
            }) {
                return Err(Completion::BudgetExhausted);
            }
            let address = if request.title {
                view.link_title(id)
            } else {
                view.link_destination(id)
            }
            .expect("cursor retains its checked body");
            if let Some(address) = address {
                let (origin, source, span) = match address {
                    BodyAddress::Original { span, .. } => (None, view.host.source(), span),
                    BodyAddress::Projected(s) => {
                        (Some(view.key()), view.body.engine.source(), s.span)
                    }
                };
                let inner = if request.title {
                    let (gaps, spaces) = match local {
                        crate::LinkTarget::Reference(symbol) => {
                            let global = view.body.link_symbols[symbol.0];
                            (
                                &view.host.references.title_gaps[global.0][..],
                                &view.host.references.title_spaces[global.0][..],
                            )
                        }
                        _ => (&view.body.engine.direct_title_gaps[id.local.0][..], &[][..]),
                    };
                    crate::CodeInfo::title_with_spaces(source, span, gaps, spaces)
                } else {
                    let literal = match local {
                        crate::LinkTarget::Email(_) | crate::LinkTarget::Www(_) => true,
                        crate::LinkTarget::Direct(_) => {
                            view.body.engine.link_info[id.local.0].label == span
                        }
                        _ => false,
                    };
                    if literal {
                        crate::CodeInfo::literal(source, span)
                    } else {
                        crate::CodeInfo::new(source, span)
                    }
                };
                self.origin = origin;
                self.inner = Some(inner);
            }
            self.target = None;
        }
        let Some(inner) = &mut self.inner else {
            return Ok(None);
        };
        Ok(inner.advance(budget)?.map(|c| BodyTextChunk {
            source: self.origin.map_or(
                BodyAddress::Original {
                    document: self.document,
                    span: c.source,
                },
                |body| {
                    BodyAddress::Projected(BodySpan {
                        body,
                        span: c.source,
                    })
                },
            ),
            text: c.text,
        }))
    }
}
pub struct BodyMathPayload<'a> {
    pub(super) key: BodyKey,
    pub(super) inner: crate::MathContent<'a>,
}
impl BodyMathPayload<'_> {
    pub fn advance(
        &mut self,
        budget: &mut Budget,
    ) -> Result<Option<BodyTextChunk<'_>>, Completion> {
        Ok(self.inner.advance(budget)?.map(|c| BodyTextChunk {
            source: BodyAddress::Projected(BodySpan {
                body: self.key,
                span: c.source,
            }),
            text: c.text,
        }))
    }
}

pub struct BodyMathWindow {
    key: BodyKey,
    inner: crate::MathWindow,
}
pub enum BodyMathWindowStep {
    Object(
        BodyId<crate::MathObjectId>,
        BodyObject<crate::MathObjectMeta>,
    ),
    BudgetExhausted,
    Complete,
}
impl BodyMathWindow {
    pub fn advance(
        &mut self,
        view: BodyView<'_>,
        budget: &mut Budget,
    ) -> Result<BodyMathWindowStep, LayoutError> {
        view.check(self.key)?;
        Ok(match self.inner.advance(&view.body.engine, budget)? {
            crate::MathWindowStep::Object(meta) => BodyMathWindowStep::Object(
                BodyId {
                    body: self.key,
                    local: meta.id,
                },
                BodyObject {
                    body: self.key,
                    value: meta,
                },
            ),
            crate::MathWindowStep::BudgetExhausted => BodyMathWindowStep::BudgetExhausted,
            crate::MathWindowStep::Complete => BodyMathWindowStep::Complete,
        })
    }
}

/// Stable BodyKey and underlying object epoch survive real prefix appends.
/// Every chunk stays explicitly projected; callers resolve it through BodyView.
pub struct BodyMathPayloadCursor {
    key: BodyKey,
    inner: crate::MathPayloadCursor,
}
pub enum BodyMathPayloadStep<'a> {
    Chunk(BodyTextChunk<'a>),
    Pending,
    BudgetExhausted,
    Complete,
}
impl BodyMathPayloadCursor {
    pub fn source_position(&self) -> BodySpan {
        BodySpan {
            body: self.key,
            span: Span::new(self.inner.source_position(), self.inner.source_position()),
        }
    }
    pub fn advance<'a>(
        &mut self,
        view: BodyView<'a>,
        budget: &mut Budget,
    ) -> Result<BodyMathPayloadStep<'a>, LayoutError> {
        view.check(self.key)?;
        Ok(match self.inner.advance(&view.body.engine, budget)? {
            crate::MathPayloadStep::Chunk(chunk) => BodyMathPayloadStep::Chunk(BodyTextChunk {
                source: BodyAddress::Projected(BodySpan {
                    body: self.key,
                    span: chunk.source,
                }),
                text: chunk.text,
            }),
            crate::MathPayloadStep::Pending => BodyMathPayloadStep::Pending,
            crate::MathPayloadStep::BudgetExhausted => BodyMathPayloadStep::BudgetExhausted,
            crate::MathPayloadStep::Complete => BodyMathPayloadStep::Complete,
        })
    }
}

impl BodyView<'_> {
    /// Read the actual hosted parser with an explicit owner/revision guard.
    /// The borrow cannot escape this callback. Native IDs/spans inside remain
    /// local to this BodyKey, just as in BodyObject; qualify them before crossing
    /// a renderer boundary. This does not parse, copy source, or fabricate a host.
    pub fn with_renderer<R>(
        self,
        owner: BodyKey,
        host_revision: u64,
        read: impl FnOnce(&crate::Document) -> R,
    ) -> Result<R, LayoutError> {
        self.check(owner)?;
        if host_revision != self.host.revision() {
            return Err(LayoutError::StaleContinuation);
        }
        Ok(read(&self.body.engine))
    }
    pub fn host_revision(self) -> u64 {
        self.host.revision()
    }
    pub fn parser_revision(self) -> u64 {
        self.body.engine.revision
    }
    pub fn binding_epoch(self) -> usize {
        self.host.display_shape_stats().binding_epoch
    }

    pub fn math_object(
        self,
        id: BodyId<crate::MathObjectId>,
    ) -> Result<Option<BodyObject<crate::MathObjectMeta>>, LayoutError> {
        self.check(id.body)?;
        Ok(self
            .body
            .engine
            .math_object(id.local)
            .map(|value| BodyObject {
                body: self.key(),
                value,
            }))
    }
    pub fn math_window(self, source: BodySpan) -> Result<BodyMathWindow, LayoutError> {
        Ok(BodyMathWindow {
            key: self.key(),
            inner: self.body.engine.math_window(self.span(source)?)?,
        })
    }
    pub fn math_payload_cursor(
        self,
        id: BodyId<crate::MathObjectId>,
    ) -> Result<Option<BodyMathPayloadCursor>, LayoutError> {
        self.check(id.body)?;
        Ok(self
            .body
            .engine
            .math_payload_cursor(id.local)
            .map(|inner| BodyMathPayloadCursor {
                key: self.key(),
                inner,
            }))
    }
}

/// An incremental clock for the same existing renderer when its source is a
/// body. The update protocol is exactly the BodyHeights/BodyCheckpoints plan;
/// only document/revision coordinates are translated at this checked boundary.
pub struct BodyRendererClock {
    owner: BodyKey,
    revision: u64,
    applied: super::updates::Applied,
}
impl BodyRendererClock {
    pub fn new(view: BodyView<'_>) -> Self {
        Self {
            owner: view.key(),
            revision: view.host.revision(),
            applied: super::updates::Applied::new(view),
        }
    }
    pub fn apply(
        &mut self,
        view: BodyView<'_>,
        host: &Update,
    ) -> Result<Option<BodyObject<Update>>, LayoutError> {
        view.check(self.owner)?;
        let Some(plan) =
            self.applied
                .plan(view, host, self.revision, crate::LayoutPolicy::default())?
        else {
            return Ok(None);
        };
        let i = host
            .body_updates
            .partition_point(|u| u.body().definition() < self.owner.definition());
        let mut local = host
            .body_updates
            .get(i)
            .filter(|u| u.body() == self.owner)
            .map(|u| u.facts().value.clone())
            .unwrap_or_else(|| Update {
                appended: Span::new(view.body.engine.source.len(), view.body.engine.source.len()),
                display_shape: view.body.engine.shape.subscription(),
                ..Default::default()
            });
        local.document = Some(view.body.engine.id());
        local.base_revision = self.revision;
        local.revision = host.revision;
        if let Some(start) = plan.start {
            // Renderer invalidation is not a new parser/shape event. In
            // particular never emit through a cloned producer subscription.
            let source = Span::new(start, view.body.engine.source.len());
            local.changes.push(crate::Change {
                source,
                kind: ChangeKind::Structure,
            });
            local.dirty.push(source);
        }
        self.revision = plan.revision;
        self.applied = plan.next;
        Ok(Some(BodyObject {
            body: self.owner,
            value: local,
        }))
    }
    /// Check owner and both input clocks before a consumer changes its caches.
    pub fn validate(&self, view: BodyView<'_>, host: &Update) -> Result<(), LayoutError> {
        view.check(self.owner)?;
        self.applied
            .plan(view, host, self.revision, crate::LayoutPolicy::default())?;
        Ok(())
    }
}
impl<'a> BodyView<'a> {
    /// Borrow fixed target metadata with its actual address. As for root raw
    /// target getters, consumers charge source bytes before inspecting text.
    pub fn raw_link_destination(
        self,
        id: BodyId<crate::LinkId>,
    ) -> Result<Option<BodyTextChunk<'a>>, LayoutError> {
        self.check(id.body())?;
        let Some(source) = self.link_destination(id)? else {
            return Ok(None);
        };
        let text = match source {
            BodyAddress::Original { document, span } => {
                if document != self.host.id() {
                    return Err(LayoutError::WrongDocument);
                }
                self.host.text(span).ok_or(LayoutError::InvalidRange)?
            }
            BodyAddress::Projected(span) => self
                .body
                .engine
                .text(self.span(span)?)
                .ok_or(LayoutError::InvalidRange)?,
        };
        Ok(Some(BodyTextChunk { source, text }))
    }
    /// Read already stored projected source under the shared byte budget.
    pub fn source_cursor(self, span: BodySpan) -> Result<BodyTextCursor<'a>, LayoutError> {
        Ok(BodyTextCursor::projected(
            self.key(),
            self.host.id(),
            crate::CodeInfo::literal(self.body.engine.source(), self.span(span)?),
        ))
    }

    /// The producer already bound this local key to the document-global symbol.
    /// This is fixed metadata; it does not evaluate target activity or read text.
    pub fn canonical_symbol(
        self,
        id: BodyId<crate::SymbolId>,
    ) -> Result<Option<crate::SymbolId>, LayoutError> {
        self.check(id.body)?;
        Ok(self.body.link_symbols.get(id.local.0).copied())
    }
    pub fn reset_link_resolver(
        self,
        resolver: &mut crate::LinkResolver,
    ) -> Result<(), LayoutError> {
        resolver.reset(&self.body.engine)
    }
    pub fn tile_count(self) -> usize {
        self.body.engine.tile_count()
    }
    pub fn tile(self, local: crate::TileId) -> Option<BodySpan> {
        self.body.engine.tile(local).map(|span| BodySpan {
            body: self.key(),
            span,
        })
    }
    /// Raw target location only; interaction still requires the paid LinkActivity.
    pub fn link_destination(
        self,
        id: BodyId<crate::LinkId>,
    ) -> Result<Option<BodyAddress>, LayoutError> {
        self.check(id.body)?;
        Ok(match self.body.engine.links.get(id.local.0) {
            Some(crate::LinkTarget::Reference(local)) => self.host.references.entries
                [self.body.link_symbols[local.0].0]
                .destination
                .map(|span| BodyAddress::Original {
                    document: self.host.id(),
                    span,
                }),
            Some(
                crate::LinkTarget::Direct(span)
                | crate::LinkTarget::Email(span)
                | crate::LinkTarget::Www(span),
            ) => Some(BodyAddress::Projected(BodySpan {
                body: self.key(),
                span: *span,
            })),
            None => None,
        })
    }
    pub fn link_title(self, id: BodyId<crate::LinkId>) -> Result<Option<BodyAddress>, LayoutError> {
        self.check(id.body)?;
        Ok(match self.body.engine.links.get(id.local.0) {
            Some(crate::LinkTarget::Reference(local)) => self.host.references.entries
                [self.body.link_symbols[local.0].0]
                .title
                .map(|span| BodyAddress::Original {
                    document: self.host.id(),
                    span,
                }),
            Some(_) => self
                .body
                .engine
                .link_info(id.local)
                .and_then(|i| i.title)
                .map(|span| {
                    BodyAddress::Projected(BodySpan {
                        body: self.key(),
                        span,
                    })
                }),
            None => None,
        })
    }
    /// Defers canonical target resolution until a paid advance. A valid raw
    /// candidate can have no destination; its cursor then completes without text.
    pub fn decoded_link_destination(
        self,
        id: BodyId<crate::LinkId>,
    ) -> Result<Option<BodyTextCursor<'a>>, LayoutError> {
        self.target_cursor(id, false)
    }
    pub fn decoded_link_title(
        self,
        id: BodyId<crate::LinkId>,
    ) -> Result<Option<BodyTextCursor<'a>>, LayoutError> {
        self.target_cursor(id, true)
    }
    fn target_cursor(
        self,
        id: BodyId<crate::LinkId>,
        title: bool,
    ) -> Result<Option<BodyTextCursor<'a>>, LayoutError> {
        self.check(id.body)?;
        Ok(self
            .body
            .engine
            .links
            .get(id.local.0)
            .map(|_| BodyTextCursor {
                origin: None,
                document: self.host.id(),
                inner: None,
                target: Some(Target {
                    view: self,
                    id,
                    title,
                }),
            }))
    }
    pub fn display_search(self, query: &str) -> Result<BodyDisplaySearch, LayoutError> {
        Ok(BodyDisplaySearch {
            key: self.key(),
            inner: crate::DisplaySearch::new(&self.body.engine, query)?,
        })
    }
    pub fn excerpt(
        self,
        span: BodySpan,
        limits: crate::ExcerptLimits,
    ) -> Result<BodyExcerpt, LayoutError> {
        Ok(BodyExcerpt {
            key: self.key(),
            inner: crate::DisplayExcerpt::new(&self.body.engine, self.span(span)?, limits)?,
        })
    }
    pub fn dependency_capture(
        self,
        tile: BodyId<crate::TileId>,
    ) -> Result<BodyDependencyCapture, LayoutError> {
        self.check(tile.body)?;
        Ok(BodyDependencyCapture {
            key: self.key(),
            inner: crate::DependencyCapture::new(&self.body.engine, tile.local)?,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BodyDisplayMatch {
    pub body: BodyKey,
    pub source: BodySpan,
    pub display: std::ops::Range<usize>,
}
pub struct BodyDisplaySearch {
    key: BodyKey,
    inner: crate::DisplaySearch,
}
impl BodyDisplaySearch {
    pub fn advance(
        &mut self,
        view: BodyView<'_>,
        budget: &mut Budget,
        out: &mut Vec<BodyDisplayMatch>,
    ) -> Result<Completion, LayoutError> {
        view.check(self.key)?;
        self.inner.advance_with(&view.body.engine, budget, |hit| {
            out.push(BodyDisplayMatch {
                body: self.key,
                source: BodySpan {
                    body: self.key,
                    span: hit.source,
                },
                display: hit.display,
            })
        })
    }
    pub fn take_link_resolver(&mut self) -> crate::LinkResolver {
        self.inner.take_link_resolver()
    }
    pub fn swap_link_resolver(
        &mut self,
        resolver: &mut crate::LinkResolver,
    ) -> Result<(), LayoutError> {
        self.inner.swap_link_resolver(resolver)
    }
    pub fn retire(&mut self, budget: &mut Budget) -> Completion {
        self.inner.retire(budget)
    }
}
pub struct BodyExcerpt {
    key: BodyKey,
    inner: crate::DisplayExcerpt,
}
impl BodyExcerpt {
    pub fn next<'a>(
        &mut self,
        view: BodyView<'a>,
        budget: &mut Budget,
    ) -> Result<BodyExcerptStep<'a>, LayoutError> {
        view.check(self.key)?;
        self.inner
            .next(&view.body.engine, budget)
            .map(|step| match step {
                crate::ExcerptStep::Run(r) => BodyExcerptStep::Run(BodyRun::display(self.key, r)),
                crate::ExcerptStep::Complete(e) => BodyExcerptStep::Complete(e),
                crate::ExcerptStep::BudgetExhausted => BodyExcerptStep::BudgetExhausted,
            })
    }
    pub fn take_link_resolver(&mut self) -> crate::LinkResolver {
        self.inner.take_link_resolver()
    }
    pub fn swap_link_resolver(
        &mut self,
        resolver: &mut crate::LinkResolver,
    ) -> Result<(), LayoutError> {
        self.inner.swap_link_resolver(resolver)
    }
    pub fn retire(&mut self, budget: &mut Budget) -> Completion {
        self.inner.retire(budget)
    }
}
pub struct BodyDependencyKey {
    key: BodyKey,
    inner: crate::DependencyKey,
}
impl BodyDependencyKey {
    pub fn validate(
        &self,
        view: BodyView<'_>,
        budget: &mut Budget,
    ) -> Result<crate::CacheValidity, LayoutError> {
        view.check(self.key)?;
        self.inner.validate(&view.body.engine, budget)
    }
}
pub enum BodyDependencyStep {
    Captured(BodyDependencyKey),
    BudgetExhausted,
    Complete,
}
pub struct BodyDependencyCapture {
    key: BodyKey,
    inner: crate::DependencyCapture,
}
impl BodyDependencyCapture {
    pub fn advance(
        &mut self,
        view: BodyView<'_>,
        budget: &mut Budget,
    ) -> Result<BodyDependencyStep, LayoutError> {
        view.check(self.key)?;
        Ok(match self.inner.advance(&view.body.engine, budget)? {
            crate::DependencyCaptureStep::Captured(inner) => {
                BodyDependencyStep::Captured(BodyDependencyKey {
                    key: self.key,
                    inner,
                })
            }
            crate::DependencyCaptureStep::BudgetExhausted => BodyDependencyStep::BudgetExhausted,
            crate::DependencyCaptureStep::Complete => BodyDependencyStep::Complete,
        })
    }
    pub fn memo_entries(&self) -> usize {
        self.inner.memo_entries()
    }
    pub fn retire(&mut self, budget: &mut Budget) -> Completion {
        self.inner.retire(budget)
    }
}

/// A body's independent estimated-height index; construction/reset uses the
/// existing logarithmic metadata index and does not read or lay out source.
pub struct BodyHeights {
    key: BodyKey,
    inner: crate::HeightIndex,
    applied: super::updates::Applied,
}
pub struct BodyMeasurement {
    key: BodyKey,
    inner: crate::Measurement,
}
impl BodyHeights {
    pub fn new(view: BodyView<'_>, estimate: u64) -> Result<Self, LayoutError> {
        let mut inner = crate::HeightIndex::new(estimate)?;
        inner.rebuild(&view.body.engine)?;
        Ok(Self {
            key: view.key(),
            inner,
            applied: super::updates::Applied::new(view),
        })
    }
    /// Explicit conservative reset after input/global symbols change. Old
    /// measurement tickets are rejected; no historical tile is remeasured.
    pub fn refresh(&mut self, view: BodyView<'_>) -> Result<(), LayoutError> {
        view.check(self.key)?;
        self.inner.rebuild(&view.body.engine)?;
        self.applied = super::updates::Applied::new(view);
        Ok(())
    }
    pub fn apply(&mut self, view: BodyView<'_>, update: &Update) -> Result<(), LayoutError> {
        self.apply_with_policy(view, update, crate::LayoutPolicy::default())
    }
    pub fn apply_with_policy(
        &mut self,
        view: BodyView<'_>,
        update: &Update,
        policy: crate::LayoutPolicy,
    ) -> Result<(), LayoutError> {
        view.check(self.key)?;
        let Some(plan) =
            self.applied
                .plan(view, update, self.inner.applied_revision().unwrap(), policy)?
        else {
            return Ok(());
        };
        self.inner.apply_hosted(
            &view.body.engine,
            update.base_revision,
            plan.revision,
            plan.start,
        )?;
        self.applied = plan.next;
        Ok(())
    }
    pub fn base_height(
        &self,
        view: BodyView<'_>,
        tile: BodyId<crate::TileId>,
    ) -> Result<Option<u64>, LayoutError> {
        view.check(self.key)?;
        view.check(tile.body)?;
        self.inner.check_document(&view.body.engine)?;
        Ok(self.inner.base_height(tile.local))
    }
    pub fn height_state(
        &self,
        view: BodyView<'_>,
        tile: BodyId<crate::TileId>,
    ) -> Result<Option<crate::HeightState>, LayoutError> {
        view.check(self.key)?;
        view.check(tile.body)?;
        self.inner.check_document(&view.body.engine)?;
        Ok(self.inner.height_state(tile.local))
    }
    pub fn ticket(
        &self,
        view: BodyView<'_>,
        tile: BodyId<crate::TileId>,
    ) -> Result<Option<BodyMeasurement>, LayoutError> {
        view.check(self.key)?;
        view.check(tile.body)?;
        self.inner.check_document(&view.body.engine)?;
        Ok(self.inner.ticket(tile.local).map(|inner| BodyMeasurement {
            key: self.key,
            inner,
        }))
    }
    pub fn measure(
        &mut self,
        view: BodyView<'_>,
        ticket: BodyMeasurement,
        rows: u64,
    ) -> Result<(), LayoutError> {
        view.check(self.key)?;
        view.check(ticket.key)?;
        self.inner.check_document(&view.body.engine)?;
        self.inner.measure(ticket.inner, rows)
    }
    pub fn total_rows(&self) -> u128 {
        self.inner.total_rows()
    }
}
/// Callbacks are qualified by the body key and run under the caller's frame Budget.
pub trait BodyLayoutSink {
    fn begin_tile(&mut self, id: BodyId<crate::TileId>);
    fn text(&mut self, run: BodyRun<'_>);
    fn object(
        &mut self,
        id: BodyId<crate::LinkId>,
        info: BodyObject<&crate::LinkInfo>,
        disposition: crate::LinkDisposition,
    );
    fn end_tile(&mut self, id: BodyId<crate::TileId>) -> u64;
}
struct Sink<'a, S> {
    key: BodyKey,
    sink: &'a mut S,
}
impl<S: BodyLayoutSink> crate::LayoutSink for Sink<'_, S> {
    fn begin_tile(&mut self, id: crate::TileId) {
        self.sink.begin_tile(BodyId {
            body: self.key,
            local: id,
        });
    }
    fn text(&mut self, run: crate::RenderRun<'_>) {
        self.sink.text(BodyRun::render(self.key, run));
    }
    fn object(&mut self, _: crate::LinkId, _: &crate::LinkInfo) {
        unreachable!("existing pass delivers disposition");
    }
    fn object_with_disposition(
        &mut self,
        id: crate::LinkId,
        info: &crate::LinkInfo,
        state: crate::LinkDisposition,
    ) {
        self.sink.object(
            BodyId {
                body: self.key,
                local: id,
            },
            BodyObject {
                body: self.key,
                value: info,
            },
            state,
        );
    }
    fn end_tile(&mut self, id: crate::TileId) -> u64 {
        self.sink.end_tile(BodyId {
            body: self.key,
            local: id,
        })
    }
}
pub struct BodyLayoutPass {
    key: BodyKey,
    inner: crate::LayoutPass,
}
impl BodyLayoutPass {
    pub fn new(
        view: BodyView<'_>,
        heights: &BodyHeights,
        start: BodyId<crate::TileId>,
        rows: u64,
        restored: bool,
    ) -> Result<Self, LayoutError> {
        view.check(heights.key)?;
        view.check(start.body)?;
        Ok(Self {
            key: view.key(),
            inner: crate::LayoutPass::new(
                &view.body.engine,
                &heights.inner,
                start.local,
                rows,
                restored,
            )?,
        })
    }
    pub fn advance(
        &mut self,
        view: BodyView<'_>,
        heights: &mut BodyHeights,
        budget: &mut Budget,
        sink: &mut impl BodyLayoutSink,
    ) -> Result<Completion, LayoutError> {
        view.check(self.key)?;
        view.check(heights.key)?;
        self.inner.advance(
            &view.body.engine,
            &mut heights.inner,
            budget,
            &mut Sink {
                key: self.key,
                sink,
            },
        )
    }
    pub fn link_resolver(&mut self) -> &mut crate::LinkResolver {
        self.inner.link_resolver()
    }
    pub fn take_link_resolver(&mut self) -> crate::LinkResolver {
        self.inner.take_link_resolver()
    }
    pub fn retire(&mut self, budget: &mut Budget) -> Completion {
        self.inner.retire(budget)
    }
}

pub struct BodyBlocks {
    key: BodyKey,
    revision: u64,
    range: Span,
    at: Option<usize>,
    complete: bool,
}
#[derive(Debug)]
pub enum BodyBlockStep<'a> {
    Block(BodyId<BlockId>, BodyObject<&'a crate::Block>),
    BudgetExhausted,
    Complete,
}
impl BodyView<'_> {
    pub fn blocks_in(self, span: BodySpan) -> Result<BodyBlocks, LayoutError> {
        Ok(BodyBlocks {
            key: self.key(),
            revision: self.host.revision(),
            range: self.span(span)?,
            at: None,
            complete: span.span.is_empty(),
        })
    }
    pub fn math_state(self, id: BodyId<crate::MathId>) -> Result<Option<SyntaxState>, LayoutError> {
        self.check(id.body)?;
        Ok(self.body.engine.math_state(id.local))
    }
}
impl BodyBlocks {
    pub fn advance<'a>(
        &mut self,
        view: BodyView<'a>,
        budget: &mut Budget,
    ) -> Result<BodyBlockStep<'a>, LayoutError> {
        view.check(self.key)?;
        if view.host.revision() != self.revision {
            return Err(LayoutError::StaleContinuation);
        }
        if self.complete {
            return Ok(BodyBlockStep::Complete);
        }
        if !budget.charge(WorkLimit {
            queries: 1,
            objects: 1,
            ..Default::default()
        }) {
            return Ok(BodyBlockStep::BudgetExhausted);
        }
        let i = self.at.unwrap_or_else(|| {
            view.body
                .engine
                .blocks
                .partition_point(|b| b.source.end <= self.range.start)
        });
        if let Some(block) = view.body.engine.blocks.get(i)
            && block.source.start < self.range.end
        {
            self.at = Some(i + 1);
            Ok(BodyBlockStep::Block(
                view.qualify(BlockId(i)),
                BodyObject {
                    body: self.key,
                    value: block,
                },
            ))
        } else {
            self.complete = true;
            Ok(BodyBlockStep::Complete)
        }
    }
}

/// Native metadata accompanied by its mandatory body namespace. Nested native
/// spans and IDs in `value` are qualified with span()/id() before another query.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BodyObject<T> {
    pub body: BodyKey,
    pub value: T,
}
impl<T> BodyObject<T> {
    pub fn span(&self, span: Span) -> BodySpan {
        BodySpan {
            body: self.body,
            span,
        }
    }
    pub fn id<I: Copy>(&self, local: I) -> BodyId<I> {
        BodyId {
            body: self.body,
            local,
        }
    }
}
/// Display/render text whose source, objects and actual style-pair identity are
/// all explicitly scoped to the hosted arena.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BodyRun<'a> {
    pub source: BodySpan,
    pub replacement: Option<BodySpan>,
    pub text: &'a str,
    pub block: Option<BodyId<BlockId>>,
    pub style: crate::Style,
    pub emphasis_depth: usize,
    pub strong_depth: usize,
    pub style_pair: Option<BodyId<crate::StylePairId>>,
    pub link: Option<BodyId<crate::LinkId>>,
}
impl<'a> BodyRun<'a> {
    pub(super) fn display(body: BodyKey, r: crate::DisplayRun<'a>) -> Self {
        Self {
            source: BodySpan {
                body,
                span: r.source,
            },
            replacement: r.replacement.map(|span| BodySpan { body, span }),
            text: r.text,
            block: r.block.map(|local| BodyId { body, local }),
            style: r.style,
            emphasis_depth: r.emphasis_depth,
            strong_depth: r.strong_depth,
            style_pair: r.style_pair.map(|local| BodyId { body, local }),
            link: r.link.map(|local| BodyId { body, local }),
        }
    }
    fn render(body: BodyKey, r: crate::RenderRun<'a>) -> Self {
        Self {
            source: BodySpan {
                body,
                span: r.source,
            },
            replacement: None,
            text: r.text,
            block: r.block.map(|local| BodyId { body, local }),
            style: r.style,
            emphasis_depth: r.emphasis_depth,
            strong_depth: r.strong_depth,
            style_pair: r.style_pair.map(|local| BodyId { body, local }),
            link: r.link.map(|local| BodyId { body, local }),
        }
    }
}
// Keep borrowed runs inline: a frame must not allocate once per emitted run.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BodyDisplayStep<'a> {
    Run(BodyRun<'a>),
    Complete,
    BudgetExhausted,
}
impl<'a> BodyDisplayStep<'a> {
    pub(super) fn from_step(body: BodyKey, step: crate::DisplayStep<'a>) -> Self {
        match step {
            crate::DisplayStep::Run(r) => Self::Run(BodyRun::display(body, r)),
            crate::DisplayStep::Complete => Self::Complete,
            crate::DisplayStep::BudgetExhausted => Self::BudgetExhausted,
        }
    }
}
// Match the allocation-free display cursor, including the same typed run.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, PartialEq, Eq)]
pub enum BodyExcerptStep<'a> {
    Run(BodyRun<'a>),
    Complete(crate::ExcerptEnd),
    BudgetExhausted,
}

pub struct BodyDependencyValidation<'a> {
    key: BodyKey,
    inner: crate::cache::DependencyValidation<'a>,
}
impl BodyDependencyKey {
    pub fn validation(&self) -> BodyDependencyValidation<'_> {
        BodyDependencyValidation {
            key: self.key,
            inner: self.inner.validation(),
        }
    }
}
impl BodyDependencyValidation<'_> {
    pub fn advance(
        &mut self,
        view: BodyView<'_>,
        budget: &mut Budget,
    ) -> Result<crate::CacheValidity, LayoutError> {
        view.check(self.key)?;
        self.inner.advance(&view.body.engine, budget)
    }
}

/// Sparse renderer-owned body states. Invalidation uses the existing subtree
/// index and never drops old user payloads or rebuilds a stable prefix.
pub struct BodyCheckpoints<S> {
    key: BodyKey,
    inner: crate::CheckpointIndex<S>,
    applied: super::updates::Applied,
}
impl<S> BodyCheckpoints<S> {
    pub fn new(view: BodyView<'_>) -> Self {
        let mut inner = crate::CheckpointIndex::new();
        inner.rebuild(&view.body.engine);
        Self {
            key: view.key(),
            inner,
            applied: super::updates::Applied::new(view),
        }
    }
    pub fn apply(&mut self, view: BodyView<'_>, update: &Update) -> Result<(), LayoutError> {
        self.apply_with_policy(view, update, crate::LayoutPolicy::default())
    }
    pub fn apply_with_policy(
        &mut self,
        view: BodyView<'_>,
        update: &Update,
        policy: crate::LayoutPolicy,
    ) -> Result<(), LayoutError> {
        view.check(self.key)?;
        let Some(plan) =
            self.applied
                .plan(view, update, self.inner.applied_revision().unwrap(), policy)?
        else {
            return Ok(());
        };
        self.inner.apply_hosted(
            &view.body.engine,
            update.base_revision,
            plan.revision,
            plan.start,
        )?;
        self.applied = plan.next;
        Ok(())
    }
    fn check(&self, view: BodyView<'_>) -> Result<(), LayoutError> {
        view.check(self.key)?;
        if self.inner.applied_revision() != Some(view.host.revision()) {
            return Err(LayoutError::OutOfOrderUpdate);
        }
        Ok(())
    }
    pub fn insert(
        &mut self,
        view: BodyView<'_>,
        tile: BodyId<crate::TileId>,
        state: S,
    ) -> Result<Option<S>, LayoutError> {
        self.check(view)?;
        view.check(tile.body)?;
        if tile.local.0 >= view.tile_count() {
            return Err(LayoutError::InvalidRange);
        }
        self.inner.insert(tile.local, state)
    }
    pub fn insert_for_layout(
        &mut self,
        view: BodyView<'_>,
        heights: &mut BodyHeights,
        tile: BodyId<crate::TileId>,
        state: S,
    ) -> Result<Option<S>, LayoutError> {
        self.check(view)?;
        view.check(heights.key)?;
        view.check(tile.body)?;
        self.inner
            .insert_for_layout(&mut heights.inner, tile.local, state)
    }
    pub fn get(
        &self,
        view: BodyView<'_>,
        tile: BodyId<crate::TileId>,
    ) -> Result<Option<&S>, LayoutError> {
        self.check(view)?;
        view.check(tile.body)?;
        Ok(self.inner.get(tile.local))
    }
    pub fn recovery(
        &self,
        view: BodyView<'_>,
        tile: BodyId<crate::TileId>,
        max_bytes: usize,
        budget: &mut Budget,
    ) -> Result<Option<(BodyId<crate::TileId>, &S)>, LayoutError> {
        self.check(view)?;
        view.check(tile.body)?;
        Ok(self
            .inner
            .recovery(&view.body.engine, tile.local, max_bytes, budget)?
            .map(|(local, state)| (view.qualify(local), state)))
    }
}
impl BodyHeights {
    pub fn ticket_from<S>(
        &self,
        view: BodyView<'_>,
        tile: BodyId<crate::TileId>,
        checkpoints: &BodyCheckpoints<S>,
        input: BodyId<crate::TileId>,
    ) -> Result<Option<BodyMeasurement>, LayoutError> {
        view.check(self.key)?;
        view.check(tile.body)?;
        view.check(input.body)?;
        checkpoints.check(view)?;
        Ok(self
            .inner
            .ticket_from(tile.local, &checkpoints.inner, input.local)
            .map(|inner| BodyMeasurement {
                key: self.key,
                inner,
            }))
    }
    pub fn measure_from<S>(
        &mut self,
        view: BodyView<'_>,
        ticket: BodyMeasurement,
        rows: u64,
        checkpoints: &BodyCheckpoints<S>,
    ) -> Result<(), LayoutError> {
        view.check(self.key)?;
        view.check(ticket.key)?;
        checkpoints.check(view)?;
        self.inner.check_document(&view.body.engine)?;
        self.inner
            .measure_from(ticket.inner, rows, &checkpoints.inner)
    }
}
impl BodyLayoutPass {
    pub fn from_checkpoint(
        view: BodyView<'_>,
        heights: &BodyHeights,
        input: BodyId<crate::TileId>,
        target: BodyId<crate::TileId>,
        rows: u64,
        max_bytes: usize,
    ) -> Result<Option<Self>, LayoutError> {
        view.check(heights.key)?;
        view.check(input.body)?;
        view.check(target.body)?;
        Ok(crate::LayoutPass::from_checkpoint(
            &view.body.engine,
            &heights.inner,
            input.local,
            target.local,
            rows,
            max_bytes,
        )?
        .map(|inner| Self {
            key: view.key(),
            inner,
        }))
    }
}

/// Already classified style metadata with a mandatory body namespace. Uses the
/// same canonical resolver and shared frame budget as body display and layout.
pub struct BodyStyleMarkers {
    key: BodyKey,
    inner: crate::StyleMarkers,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BodyStyleMarkerStep {
    Span(BodyObject<crate::ResolvedStyleSpan>),
    BudgetExhausted,
    Complete,
}
impl BodyView<'_> {
    pub fn style_markers(self, source: BodySpan) -> Result<BodyStyleMarkers, LayoutError> {
        Ok(BodyStyleMarkers {
            key: self.key(),
            inner: self.body.engine.style_markers(self.span(source)?).unwrap(),
        })
    }
    /// Local shape sequence/epoch are body scoped. Host-global binding changes
    /// additionally use Document::display_shape_stats and the host Update.
    pub fn display_shape_stats(self) -> BodyObject<crate::DisplayShapeStats> {
        BodyObject {
            body: self.key(),
            value: self.body.engine.display_shape_stats(),
        }
    }
    pub fn canonical_footnote_symbol(
        self,
        id: BodyId<FootnoteId>,
    ) -> Result<Option<FootnoteId>, LayoutError> {
        self.check(id.body)?;
        Ok(self.body.note_symbols.get(id.local.0).copied())
    }
}
impl BodyStyleMarkers {
    pub fn source_position(&self) -> BodySpan {
        let at = self.inner.source_position();
        BodySpan {
            body: self.key,
            span: Span::new(at, at),
        }
    }
    pub fn advance(
        &mut self,
        view: BodyView<'_>,
        resolver: &mut crate::LinkResolver,
        budget: &mut Budget,
    ) -> Result<BodyStyleMarkerStep, LayoutError> {
        view.check(self.key)?;
        Ok(
            match self.inner.advance(&view.body.engine, resolver, budget)? {
                crate::StyleMarkerStep::Span(value) => BodyStyleMarkerStep::Span(BodyObject {
                    body: self.key,
                    value,
                }),
                crate::StyleMarkerStep::BudgetExhausted => BodyStyleMarkerStep::BudgetExhausted,
                crate::StyleMarkerStep::Complete => BodyStyleMarkerStep::Complete,
            },
        )
    }
}
