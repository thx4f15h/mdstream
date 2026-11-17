//! Body-local parser facts carried by their original host input update.
use super::*;

/// Actual child-parser changes, explicitly qualified by their hosted arena.
/// Local parser revisions are private; consumers apply the enclosing host Update.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BodyUpdate {
    pub(super) body: BodyKey,
    pub(super) local: Box<Update>,
}
impl BodyUpdate {
    pub(crate) fn into_local(self) -> Update {
        *self.local
    }

    pub fn body(&self) -> BodyKey {
        self.body
    }
    /// The actual parser Update, with this body's mandatory namespace. Every
    /// source, ID, effect sequence and parser revision inside `value` is local.
    /// Apply the enclosing host Update to BodyHeights/BodyCheckpoints; do not
    /// apply these native parser revisions to a host index. Host-global binding
    /// effects remain in the enclosing Update and must also be consumed.
    pub fn facts(&self) -> BodyObject<&Update> {
        BodyObject {
            body: self.body,
            value: &self.local,
        }
    }
    pub fn appended(&self) -> BodySpan {
        BodySpan {
            body: self.body,
            span: self.local.appended,
        }
    }
    pub fn changes(&self) -> impl Iterator<Item = (BodySpan, ChangeKind)> + '_ {
        self.local.changes.iter().map(|c| {
            (
                BodySpan {
                    body: self.body,
                    span: c.source,
                },
                c.kind,
            )
        })
    }
    pub(crate) fn merge(&mut self, next: Self) -> Result<(), LayoutError> {
        if self.body != next.body {
            return Err(LayoutError::InvalidRange);
        }
        self.local.merge(*next.local)
    }
    pub(crate) fn consecutive(&self, next: &Self) -> bool {
        self.local.revision == next.local.base_revision
    }
}

#[derive(Clone, Copy)]
pub(super) struct Applied {
    local_revision: u64,
    source_len: usize,
    finished: bool,
}
pub(super) struct Plan {
    pub start: Option<usize>,
    pub revision: u64,
    pub next: Applied,
}
impl Applied {
    pub(super) fn new(view: BodyView<'_>) -> Self {
        Self {
            local_revision: view.body.engine.revision,
            source_len: view.body.engine.source.len(),
            finished: view.host.finished,
        }
    }
    pub(super) fn plan(
        self,
        view: BodyView<'_>,
        host: &Update,
        previous: u64,
        policy: crate::LayoutPolicy,
    ) -> Result<Option<Plan>, LayoutError> {
        if host.document != Some(view.host.id()) {
            return Err(LayoutError::WrongDocument);
        }
        if host.revision != view.host.revision() || host.base_revision > host.revision {
            return Err(LayoutError::OutOfOrderUpdate);
        }
        if previous == host.revision {
            return Ok(None);
        }
        if previous != host.base_revision {
            return Err(LayoutError::OutOfOrderUpdate);
        }
        let i = host
            .body_updates
            .partition_point(|u| u.body.definition < view.key().definition);
        let local = host
            .body_updates
            .get(i)
            .filter(|u| u.body == view.key())
            .map(|u| &*u.local);
        if let Some(local) = local {
            if local.document != Some(view.body.engine.id())
                || local.base_revision != self.local_revision
                || local.revision != view.body.engine.revision
                || local.appended.start != self.source_len
                || local.appended.end != view.body.engine.source.len()
            {
                return Err(LayoutError::OutOfOrderUpdate);
            }
        } else if self.local_revision != view.body.engine.revision
            || self.source_len != view.body.engine.source.len()
        {
            return Err(LayoutError::OutOfOrderUpdate);
        }
        // No reverse-use walk: canonical binding changes conservatively expire
        // one body interval. Ordinary source appends retain the stable prefix.
        let global = !host.references_changed.is_empty()
            || (!view.body.engine.footnotes.references.is_empty()
                && (!host.footnotes_changed.is_empty() || self.finished != view.host.finished));
        let start = if global {
            Some(0)
        } else {
            local.and_then(|u| u.layout_start(policy))
        };
        Ok(Some(Plan {
            start,
            revision: host.revision,
            next: Self::new(view),
        }))
    }
}
