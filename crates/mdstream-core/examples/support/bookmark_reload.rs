//! Host-only prototype. Resource/version tokens are trusted application facts,
//! not Document IDs or fingerprints computed by this example. See docs/bookmark-reload.md.
use mdstream::{
    Bookmark, Budget, Completion, Document, DocumentId, FoldNavigation, FoldView, LayoutError,
    ScrollTarget, WorkLimit,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Version {
    pub resource: u64,
    pub content: u64,
}

/// Only this transient binding contains a DocumentId. Never persist it.
#[derive(Clone, Copy)]
pub struct Binding {
    pub version: Version,
    document: DocumentId,
    revision: u64,
}

impl Binding {
    /// The caller attests which exact content version was loaded. No text scan.
    pub fn new(document: &Document, version: Version) -> Self {
        Self {
            version,
            document: document.id(),
            revision: document.revision(),
        }
    }

    fn check(self, document: &Document) -> Result<(), Error> {
        if self.document != document.id() {
            return Err(Error::Core(LayoutError::WrongDocument));
        }
        if self.revision != document.revision() {
            return Err(Error::Core(LayoutError::StaleContinuation));
        }
        Ok(())
    }
}

/// Persistence payload only: no runtime IDs, text, layout, or fold tree.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Saved {
    pub version: Version,
    pub source: usize,
    pub viewport_offset: u64,
}

impl Saved {
    pub fn capture(
        document: &Document,
        binding: Binding,
        bookmark: Bookmark,
    ) -> Result<Self, Error> {
        binding.check(document)?;
        if bookmark.anchor().document_id() != document.id() {
            return Err(Error::Core(LayoutError::WrongDocument));
        }
        Ok(Self {
            version: binding.version,
            source: bookmark.anchor().source(),
            viewport_offset: bookmark.viewport_offset(),
        })
    }
}

/// Each terminal answer is the host's authoritative answer for ONE old point.
/// Exact means unique correspondence, including edits only if explicitly mapped.
#[derive(Clone, Copy)]
pub enum MapStep {
    Working,
    Exact(usize),
    Changed,
    Deleted,
    Missing,
    Ambiguous,
}

/// Immutable, precomputed bounded-step stand-in for an external mapping job.
/// Creating this evidence is outside the core Budget. An exhausted script is
/// unresolved; restart with a completed report when the host has more evidence.
pub struct PointMap<'a> {
    pub from: Version,
    pub source: usize,
    pub to: Version,
    pub steps: &'a [MapStep],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    WrongResource,
    WrongMapping,
    Changed,
    Deleted,
    Missing,
    Ambiguous,
    Core(LayoutError),
}

impl From<LayoutError> for Error {
    fn from(error: LayoutError) -> Self {
        Self::Core(error)
    }
}

pub struct Restore<'a> {
    saved: Saved,
    binding: Binding,
    map: PointMap<'a>,
    map_position: usize,
    mapped: Option<usize>,
    bookmark: Option<Bookmark>,
    navigation: Option<FoldNavigation>,
    failure: Option<Error>,
    viewport_rows: u64,
}

impl<'a> Restore<'a> {
    /// O(1): no bookmark list, map script, source, or heading scan.
    pub fn new(
        saved: Saved,
        document: &Document,
        binding: Binding,
        map: PointMap<'a>,
        viewport_rows: u64,
    ) -> Result<Self, Error> {
        binding.check(document)?;
        if saved.version.resource != binding.version.resource {
            return Err(Error::WrongResource);
        }
        if map.from != saved.version || map.source != saved.source || map.to != binding.version {
            return Err(Error::WrongMapping);
        }
        Ok(Self {
            saved,
            binding,
            map,
            map_position: 0,
            mapped: None,
            bookmark: None,
            navigation: None,
            failure: None,
            viewport_rows,
        })
    }

    pub fn map_position(&self) -> usize {
        self.map_position
    }

    /// None means pending, including an unresolved external map. Only Some
    /// authorizes the host to commit its active history slot / estimated target.
    /// Actual row layout and viewport-offset application are a separate step.
    pub fn advance<S>(
        &mut self,
        document: &Document,
        view: &mut FoldView<S>,
        budget: &mut Budget,
    ) -> Result<Option<(Bookmark, ScrollTarget)>, Error> {
        self.binding.check(document)?;
        // Reuse the public checked accessor solely as an owner/update preflight.
        // Do not mutate either layout index through the returned references.
        let _ = view.layout_parts_mut(document)?;
        if let Some(error) = self.failure {
            return Err(error);
        }
        while self.mapped.is_none() {
            let Some(step) = self.map.steps.get(self.map_position) else {
                return Ok(None);
            };
            if !budget.charge(host_step()) {
                return Ok(None);
            }
            self.map_position += 1;
            let error = match *step {
                MapStep::Working => continue,
                MapStep::Exact(source) => {
                    self.mapped = Some(source);
                    break;
                }
                MapStep::Changed => Error::Changed,
                MapStep::Deleted => Error::Deleted,
                MapStep::Missing => Error::Missing,
                MapStep::Ambiguous => Error::Ambiguous,
            };
            self.failure = Some(error);
            return Err(error);
        }
        if self.bookmark.is_none() {
            if !budget.charge(host_step()) {
                return Ok(None);
            }
            // Recompute document-local metadata from the NEW source/document.
            let result = Bookmark::new(document, self.mapped.unwrap(), self.saved.viewport_offset)
                .and_then(|bookmark| {
                    let navigation = bookmark.navigation(document, view, self.viewport_rows)?;
                    Ok((bookmark, navigation))
                });
            match result {
                Ok((bookmark, navigation)) => {
                    self.bookmark = Some(bookmark);
                    self.navigation = Some(navigation);
                }
                Err(error) => {
                    self.failure = Some(Error::Core(error));
                    return Err(Error::Core(error));
                }
            }
        }
        let progress = self
            .navigation
            .as_mut()
            .unwrap()
            .advance(document, view, budget)?;
        Ok(if progress.completion == Completion::Covered {
            Some((
                self.bookmark.unwrap(),
                progress.target.expect("validated source"),
            ))
        } else {
            None
        })
    }
}

/// One bounded host-map step or one indexed Bookmark creation, separately from
/// FoldNavigation's 1 query / 1 object steps. No external callback is invoked.
pub fn host_step() -> WorkLimit {
    WorkLimit {
        queries: 1,
        objects: 1,
        recovery: 1,
        ..Default::default()
    }
}
