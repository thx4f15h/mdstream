use mdtui3::{
    Budget, DisplayExcerpt, Document, DocumentId, ExcerptEnd, ExcerptLimits, ExcerptStep, Heading,
    HeadingId, LayoutError, Span, Update,
};

use crate::render::text::{display_width, fit_prefix};

pub const MIN_SOURCE_BYTES_PER_HEADING: usize = 256;
pub const MAX_SOURCE_BYTES_PER_HEADING: usize = 4 * 1024;
pub const MIN_DISPLAY_BYTES_PER_HEADING: usize = 64;
pub const MAX_DISPLAY_BYTES_PER_HEADING: usize = 4 * 1024;

#[must_use]
pub fn default_excerpt_limits(width: usize) -> ExcerptLimits {
    let width = width.max(1);
    ExcerptLimits {
        source_bytes: width
            .saturating_mul(64)
            .clamp(MIN_SOURCE_BYTES_PER_HEADING, MAX_SOURCE_BYTES_PER_HEADING),
        display_bytes: width
            .saturating_mul(4)
            .saturating_add(16)
            .clamp(MIN_DISPLAY_BYTES_PER_HEADING, MAX_DISPLAY_BYTES_PER_HEADING),
        lines: 1,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SidebarEntry {
    pub id: HeadingId,
    pub level: u8,
    pub source: Span,
    pub text: String,
    pub truncated: bool,
    pub pending: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SidebarStats {
    pub headings_considered: usize,
    pub excerpts_created: usize,
    pub excerpt_steps: usize,
    pub source_bytes_read: usize,
    pub display_bytes_emitted: usize,
    pub budget_exhausted: usize,
    pub reused_entries: usize,
    pub rebuilt_entries: usize,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SidebarMemoryStats {
    pub entries: usize,
    pub display_text_bytes: usize,
    pub scratch_text_bytes: usize,
    /// Retained title text is the visible clipped text plus the current
    /// per-entry excerpt prefix used to refresh that text.
    pub retained_text_bytes: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidebarStatus {
    Complete,
    BudgetExhausted,
}

struct EntryState {
    heading: Heading,
    entry: SidebarEntry,
    raw_text: String,
    excerpt: Option<DisplayExcerpt>,
    lease: Option<crate::render::objects::LinkLease>,
    done: bool,
}

/// A revision-bound model of only the currently requested sidebar rows.
///
/// Construction reads the O(1) heading window slice and copies only visible
/// heading metadata. Title text is loaded lazily through per-entry
/// `DisplayExcerpt` cursors and retained across bounded `advance` calls.
pub struct SidebarModel {
    links: crate::render::objects::SharedLinks,
    document: DocumentId,
    revision: u64,
    first_visible: usize,
    rows: usize,
    width: usize,
    limits: ExcerptLimits,
    entries: Vec<EntryState>,
    stats: SidebarStats,
    complete: bool,
}

pub mod directory;

impl SidebarModel {
    /// UI path: callers obtain sparse metadata under their shared frame Budget.
    /// An empty model does not inspect the heading index or source.
    pub(crate) fn empty(document: &Document, width: usize) -> Self {
        Self {
            links: crate::render::objects::SharedLinks::new(document),
            document: document.id(),
            revision: document.revision(),
            first_visible: 0,
            rows: 0,
            width,
            limits: default_excerpt_limits(width),
            entries: Vec::new(),
            stats: SidebarStats::default(),
            complete: true,
        }
    }

    pub(crate) fn prepare_sparse(
        &mut self,
        document: &Document,
        headings: &[(HeadingId, Heading)],
        width: usize,
        dirty_from: usize,
    ) {
        let mut old = std::mem::take(&mut self.entries);
        let same_width = self.width == width;
        self.entries = headings
            .iter()
            .map(|&(id, heading)| {
                let reused = old
                    .iter()
                    .position(|state| {
                        same_width
                            && state.entry.id == id
                            && state.heading == heading
                            && (self.revision == document.revision()
                                || (state.done && heading.source.end <= dirty_from))
                    })
                    .map(|i| old.swap_remove(i));
                if let Some(state) = reused {
                    self.stats.reused_entries += 1;
                    state
                } else {
                    self.stats.rebuilt_entries += 1;
                    new_entry_state(id, heading, width)
                }
            })
            .collect();
        self.stats.headings_considered = headings.len();
        self.revision = document.revision();
        self.width = width;
        self.limits = default_excerpt_limits(width);
        self.rows = headings.len();
        self.first_visible = headings.first().map_or(0, |(id, _)| id.0);
        self.complete = self.entries.iter().all(|state| state.done);
    }

    pub(crate) fn cancel_pending(&mut self) {
        for state in &mut self.entries {
            state.excerpt = None;
            state.lease = None;
            if !state.done {
                state.raw_text.clear();
                state.entry.text = "…".to_owned();
            }
        }
    }

    pub(crate) fn capacity_bytes(&self) -> usize {
        self.entries.capacity() * std::mem::size_of::<EntryState>()
            + self
                .entries
                .iter()
                .map(|s| s.raw_text.capacity() + s.entry.text.capacity())
                .sum::<usize>()
    }

    #[must_use]
    pub fn new(document: &Document, first_visible: usize, rows: usize, width: usize) -> Self {
        Self::with_limits(
            document,
            first_visible,
            rows,
            width,
            default_excerpt_limits(width),
        )
    }

    #[must_use]
    pub fn with_limits(
        document: &Document,
        first_visible: usize,
        rows: usize,
        width: usize,
        limits: ExcerptLimits,
    ) -> Self {
        let headings = document.headings().entries();
        let start = first_visible.min(headings.len());
        let end = start.saturating_add(rows).min(headings.len());
        let entries: Vec<_> = headings[start..end]
            .iter()
            .copied()
            .enumerate()
            .map(|(offset, heading)| new_entry_state(HeadingId(start + offset), heading, width))
            .collect();
        let complete = entries.iter().all(|entry| entry.done);
        Self {
            links: crate::render::objects::SharedLinks::new(document),
            document: document.id(),
            revision: document.revision(),
            first_visible,
            rows,
            width,
            limits,
            entries,
            stats: SidebarStats {
                headings_considered: end.saturating_sub(start),
                ..SidebarStats::default()
            },
            complete,
        }
    }

    pub fn set_links(&mut self, links: crate::render::objects::SharedLinks) {
        self.links = links;
    }

    #[must_use]
    pub const fn document(&self) -> DocumentId {
        self.document
    }

    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    #[must_use]
    pub const fn first_visible(&self) -> usize {
        self.first_visible
    }

    #[must_use]
    pub const fn rows(&self) -> usize {
        self.rows
    }

    #[must_use]
    pub const fn width(&self) -> usize {
        self.width
    }

    #[must_use]
    pub const fn limits(&self) -> ExcerptLimits {
        self.limits
    }

    #[must_use]
    pub fn matches(&self, document: &Document) -> bool {
        self.document == document.id() && self.revision == document.revision()
    }

    pub fn entries(&self) -> impl ExactSizeIterator<Item = &SidebarEntry> {
        self.entries.iter().map(|entry| &entry.entry)
    }

    #[must_use]
    pub fn entry(&self, index: usize) -> Option<&SidebarEntry> {
        self.entries.get(index).map(|entry| &entry.entry)
    }

    #[must_use]
    pub const fn stats(&self) -> SidebarStats {
        self.stats
    }

    #[must_use]
    pub const fn is_complete(&self) -> bool {
        self.complete
    }

    #[must_use]
    pub fn pending_entries(&self) -> usize {
        self.entries.iter().filter(|entry| !entry.done).count()
    }

    #[must_use]
    pub fn memory_stats(&self) -> SidebarMemoryStats {
        let display_text_bytes = self
            .entries
            .iter()
            .map(|entry| entry.entry.text.len())
            .sum();
        let scratch_text_bytes = self.entries.iter().map(|entry| entry.raw_text.len()).sum();
        SidebarMemoryStats {
            entries: self.entries.len(),
            display_text_bytes,
            scratch_text_bytes,
            retained_text_bytes: display_text_bytes.saturating_add(scratch_text_bytes),
        }
    }

    pub fn apply_update(
        &mut self,
        document: &Document,
        update: &Update,
        first_visible: usize,
        rows: usize,
    ) -> Result<(), LayoutError> {
        if self.document != document.id() || update.document != Some(document.id()) {
            return Err(LayoutError::WrongDocument);
        }
        if update.revision != document.revision() {
            return Err(LayoutError::StaleContinuation);
        }
        if update.revision == self.revision {
            return self.reconfigure(document, first_visible, rows);
        }
        if update.base_revision != self.revision {
            return Err(LayoutError::StaleContinuation);
        }
        let appended_start = update.appended.start;
        let new_headings = update.new_headings.clone();
        let changed_headings = update.changed_headings.clone();
        self.rebuild_window(document, first_visible, rows, |old, id, heading| {
            old.heading == *heading
                && !new_headings.contains(&id.0)
                && (!changed_headings.contains(&id.0) || heading.source.end <= appended_start)
        });
        self.revision = document.revision();
        Ok(())
    }

    pub fn reconfigure(
        &mut self,
        document: &Document,
        first_visible: usize,
        rows: usize,
    ) -> Result<(), LayoutError> {
        if !self.matches(document) {
            return Err(if self.document != document.id() {
                LayoutError::WrongDocument
            } else {
                LayoutError::StaleContinuation
            });
        }
        if self.first_visible == first_visible && self.rows == rows {
            return Ok(());
        }
        self.rebuild_window(document, first_visible, rows, |old, _, heading| {
            old.heading == *heading
        });
        Ok(())
    }

    fn rebuild_window(
        &mut self,
        document: &Document,
        first_visible: usize,
        rows: usize,
        reuse: impl Fn(&EntryState, HeadingId, &Heading) -> bool,
    ) {
        let headings = document.headings().entries();
        let start = first_visible.min(headings.len());
        let end = start.saturating_add(rows).min(headings.len());
        let mut old: Vec<Option<EntryState>> = std::mem::take(&mut self.entries)
            .into_iter()
            .map(Some)
            .collect();
        let mut entries = Vec::with_capacity(end.saturating_sub(start));
        let mut reused = 0_usize;
        let mut rebuilt = 0_usize;
        for (offset, heading) in headings[start..end].iter().copied().enumerate() {
            let id = HeadingId(start + offset);
            let old_index = old
                .iter()
                .position(|entry| entry.as_ref().is_some_and(|entry| entry.entry.id == id));
            let reusable = old_index.and_then(|index| {
                old[index]
                    .as_ref()
                    .is_some_and(|entry| reuse(entry, id, &heading))
                    .then(|| old[index].take())
                    .flatten()
            });
            if let Some(entry) = reusable {
                entries.push(entry);
                reused = reused.saturating_add(1);
            } else {
                entries.push(new_entry_state(id, heading, self.width));
                rebuilt = rebuilt.saturating_add(1);
            }
        }
        self.entries = entries;
        self.first_visible = first_visible;
        self.rows = rows;
        self.stats.headings_considered = end.saturating_sub(start);
        self.stats.reused_entries = self.stats.reused_entries.saturating_add(reused);
        self.stats.rebuilt_entries = self.stats.rebuilt_entries.saturating_add(rebuilt);
        self.complete = self.entries.iter().all(|entry| entry.done);
    }

    pub fn advance(
        &mut self,
        document: &Document,
        budget: &mut Budget,
    ) -> Result<SidebarStatus, LayoutError> {
        if self.document != document.id() {
            return Err(LayoutError::WrongDocument);
        }
        if self.revision != document.revision() {
            return Err(LayoutError::StaleContinuation);
        }
        if !self.links.prepare(document, budget)? {
            return Ok(SidebarStatus::BudgetExhausted);
        }
        if self.complete {
            return Ok(SidebarStatus::Complete);
        }
        if self.width == 0 {
            for state in &mut self.entries {
                state.done = true;
                state.excerpt = None;
                state.lease = None;
                state.entry.pending = false;
                state.entry.truncated = !state.heading.content.is_empty();
                state.entry.text.clear();
            }
            self.complete = true;
            return Ok(SidebarStatus::Complete);
        }

        let width = self.width;
        let limits = self.limits;
        let stats = &mut self.stats;
        for state in &mut self.entries {
            if state.done {
                continue;
            }
            if state.lease.is_none() {
                state.lease = Some(self.links.hold());
            }
            if state.excerpt.is_none() {
                state.excerpt = Some(DisplayExcerpt::new(
                    document,
                    state.heading.content,
                    limits,
                )?);
                stats.excerpts_created = stats.excerpts_created.saturating_add(1);
            }
            loop {
                let before = state
                    .excerpt
                    .as_ref()
                    .expect("excerpt created above")
                    .source_position();
                let step = self.links.with_resolver(|resolver| {
                    let excerpt = state.excerpt.as_mut().expect("excerpt created above");
                    excerpt.swap_link_resolver(resolver)?;
                    let result = excerpt.next(document, budget);
                    excerpt.swap_link_resolver(resolver)?;
                    result
                })?;
                stats.excerpt_steps = stats.excerpt_steps.saturating_add(1);
                let after = state
                    .excerpt
                    .as_ref()
                    .expect("excerpt created above")
                    .source_position();
                stats.source_bytes_read = stats
                    .source_bytes_read
                    .saturating_add(after.saturating_sub(before));
                match step {
                    ExcerptStep::Run(run) => {
                        state.raw_text.push_str(run.text);
                        stats.display_bytes_emitted =
                            stats.display_bytes_emitted.saturating_add(run.text.len());
                        if display_width(&state.raw_text) >= width {
                            state.done = true;
                            state.excerpt = None;
                            state.lease = None;
                            let truncated = after < state.heading.content.end
                                || display_width(&state.raw_text) > width;
                            refresh_entry(state, width, truncated, false);
                            break;
                        }
                    }
                    ExcerptStep::Complete(end) => {
                        state.done = true;
                        state.excerpt = None;
                        state.lease = None;
                        let truncated =
                            end != ExcerptEnd::SourceEnd || display_width(&state.raw_text) > width;
                        refresh_entry(state, width, truncated, false);
                        break;
                    }
                    ExcerptStep::BudgetExhausted => {
                        stats.budget_exhausted = stats.budget_exhausted.saturating_add(1);
                        refresh_entry(state, width, true, true);
                        return Ok(SidebarStatus::BudgetExhausted);
                    }
                }
            }
        }
        self.complete = true;
        Ok(SidebarStatus::Complete)
    }
}

fn new_entry_state(id: HeadingId, heading: Heading, width: usize) -> EntryState {
    let pending = width > 0 && !heading.content.is_empty();
    EntryState {
        heading,
        entry: SidebarEntry {
            id,
            level: heading.level,
            source: heading.source,
            text: if pending {
                "…".to_owned()
            } else {
                String::new()
            },
            truncated: pending,
            pending,
        },
        raw_text: String::new(),
        excerpt: None,
        lease: None,
        done: !pending,
    }
}

fn refresh_entry(state: &mut EntryState, width: usize, truncated: bool, pending: bool) {
    let raw = state.raw_text.trim_end_matches(['\r', '\n']);
    let needs_ellipsis = truncated || pending || display_width(raw) > width;
    let cells = if needs_ellipsis {
        width.saturating_sub(1)
    } else {
        width
    };
    let fit = fit_prefix(raw, cells);
    let overflow = fit.end < raw.len();
    let mut text = raw[..fit.end].to_owned();
    let truncated = needs_ellipsis || overflow;
    if truncated && width > 0 {
        text.push('…');
    }
    state.entry.text = text;
    state.entry.truncated = truncated;
    state.entry.pending = pending;
}
