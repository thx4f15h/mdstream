use super::*;

pub(super) struct NoteLink {
    id: BodyId<LinkId>,
    activity: Option<mdtui3::LinkActivity>,
    active: bool,
    destination: mdtui3::BodyDestinationTask,
    decoded: String,
    request: Option<mdtui3::FragmentContinuation>,
    pub(super) cancelled: bool,
}
pub(super) enum NoteLinkResult {
    Pending,
    Done,
    Notice(&'static str),
    Source(BodySpan),
    External(mdtui3::BodyObject<crate::external::ExternalTarget>),
}
impl NoteLink {
    pub(super) fn retire(&mut self, budget: &mut Budget) -> Completion {
        if let Some(activity) = &mut self.activity
            && activity.retire_continuation(budget) != Completion::Covered
        {
            return Completion::BudgetExhausted;
        }
        self.activity = None;
        Completion::Covered
    }
}
impl NoteViewport {
    pub(super) fn advance_link(
        &mut self,
        body: BodyView<'_>,
        native: &Document,
        budget: &mut Budget,
    ) -> Result<NoteLinkResult, FrameError> {
        if body.key() != self.key {
            return Err(LayoutError::InvalidRange.into());
        }
        if self.link.as_ref().is_some_and(|p| p.cancelled) {
            if self.link.as_mut().unwrap().retire(budget) != Completion::Covered {
                return Ok(NoteLinkResult::Pending);
            }
            self.link = None;
        }
        if self.retire_anchors {
            if let Some(index) = &mut self.anchors
                && index.retire(budget) != Completion::Covered
            {
                return Ok(NoteLinkResult::Pending);
            }
            self.anchors = None;
            self.retire_anchors = false;
        }
        if self.link.is_none()
            && let Some(id) = self.queued_link
        {
            if id.body() != self.key {
                return Err(LayoutError::InvalidRange.into());
            }
            if !budget.charge(WorkLimit {
                queries: 1,
                objects: 1,
                ..Default::default()
            }) {
                return Ok(NoteLinkResult::Pending);
            }
            self.queued_link = None;
            let Some(activity) = native.link_activity(id.local()) else {
                return Ok(NoteLinkResult::Notice("Link no longer active"));
            };
            let prefix = match native.links().get(id.local().0) {
                Some(mdtui3::LinkTarget::Email(_)) => "mailto:",
                Some(mdtui3::LinkTarget::Www(_)) => "http://",
                _ => "",
            };
            self.link = Some(NoteLink {
                id,
                activity: Some(activity),
                active: false,
                destination: body.link_destination_task(id)?,
                decoded: prefix.to_owned(),
                request: None,
                cancelled: false,
            });
        }
        let Some(p) = &mut self.link else {
            return Ok(NoteLinkResult::Done);
        };
        if p.id.body() != self.key {
            return Err(LayoutError::InvalidRange.into());
        }
        let links = self.viewport.shared_links();
        if !links.prepare(native, budget)? {
            return Ok(NoteLinkResult::Pending);
        }
        if !p.active {
            let activity = p.activity.as_mut().unwrap();
            let Some(disposition) =
                links.with_resolver(|r| activity.advance_with(native, r, budget))?
            else {
                return Ok(NoteLinkResult::Pending);
            };
            if activity.retire_continuation(budget) != Completion::Covered {
                return Ok(NoteLinkResult::Pending);
            }
            p.activity = None;
            if disposition != mdtui3::LinkDisposition::Active {
                self.link = None;
                return Ok(NoteLinkResult::Notice("Link no longer active"));
            }
            p.active = true;
        }
        if p.request.is_none() {
            loop {
                match p.destination.advance(body, budget)? {
                    Err(_) => return Ok(NoteLinkResult::Pending),
                    Ok(Some(chunk)) => {
                        if p.decoded.len().saturating_add(chunk.text.len())
                            > crate::external::MAX_EXTERNAL_URL_BYTES
                        {
                            self.link = None;
                            return Ok(NoteLinkResult::Notice(
                                "Link target exceeds supported limit",
                            ));
                        }
                        p.decoded.push_str(chunk.text);
                    }
                    Ok(None) => break,
                }
            }
            if !p.decoded.starts_with('#') {
                let target = crate::external::ExternalTarget::from_url(&p.decoded);
                self.link = None;
                return Ok(match target {
                    Ok(value) => NoteLinkResult::External(mdtui3::BodyObject {
                        body: self.key,
                        value,
                    }),
                    Err(_) => NoteLinkResult::Notice("Invalid or unsupported external link"),
                });
            }
            if self.anchors.is_none() {
                self.anchors = Some(mdtui3::HeadingAnchors::new(native));
            }
            let index = self.anchors.as_mut().unwrap();
            if index.advance(native, budget)? == Completion::BudgetExhausted {
                return Ok(NoteLinkResult::Pending);
            }
            p.request = Some(index.fragment_owned(Arc::new(std::mem::take(&mut p.decoded)))?);
        }
        let result =
            p.request
                .as_mut()
                .unwrap()
                .advance(self.anchors.as_ref().unwrap(), native, budget);
        let value = match result {
            Ok(mdtui3::FragmentStep::BudgetExhausted | mdtui3::FragmentStep::IndexIncomplete) => {
                return Ok(NoteLinkResult::Pending);
            }
            Ok(mdtui3::FragmentStep::Resolved(target)) => NoteLinkResult::Source(BodySpan {
                body: self.key,
                span: mdtui3::Span::new(target.source, target.source),
            }),
            Ok(mdtui3::FragmentStep::Unresolved) => {
                NoteLinkResult::Notice("Heading not yet defined in this note")
            }
            Ok(mdtui3::FragmentStep::NotFound) => {
                NoteLinkResult::Notice("Heading not found in this note")
            }
            Err(LayoutError::InvalidRange) => NoteLinkResult::Notice("Invalid heading fragment"),
            Err(error) => return Err(error.into()),
        };
        self.link = None;
        Ok(value)
    }
}
