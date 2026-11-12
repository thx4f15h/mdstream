//! Publication checkpoint for one physical candidate header. Only effects
//! created by that row are revisited; historical arenas and use sites are not.
use crate::{LinkId, LinkInfo, Math, MathId, MathKind, SyntaxState, Work};
use std::collections::BTreeMap;

type LinkKey = (usize, usize, bool);
type MathKey = (usize, usize, bool);
fn link_key(info: LinkInfo) -> LinkKey {
    (
        info.source.start,
        info.label.start,
        info.kind == crate::LinkKind::Image,
    )
}
fn math_key(math: Math) -> MathKey {
    (
        math.source.start,
        math.source.end,
        math.kind == MathKind::Display,
    )
}

#[derive(Default)]
pub(crate) struct LineEffects {
    pub(crate) pending: bool,
    pub(crate) replay: Option<crate::Span>,
    links: usize,
    maths: usize,
    display: usize,
    gaps: usize,
    states: BTreeMap<LinkId, SyntaxState>,
    peak_entries: usize,
    link_reuse: BTreeMap<LinkKey, LinkId>,
    math_reuse: BTreeMap<MathKey, MathId>,
}
impl LineEffects {
    pub(crate) fn begin(&mut self, links: usize, maths: usize, display: usize, gaps: usize) {
        assert!(!self.pending);
        self.clear_reuse();
        self.pending = true;
        self.links = links;
        self.maths = maths;
        self.display = display;
        self.gaps = gaps;
    }
    pub(crate) fn state(&mut self, id: LinkId, state: SyntaxState) -> SyntaxState {
        if self.pending {
            self.states.insert(id, state);
            self.note_peak();
            SyntaxState::Provisional
        } else {
            state
        }
    }
    pub(crate) fn reuse_link(&mut self, info: LinkInfo) -> Option<LinkId> {
        self.link_reuse.remove(&link_key(info))
    }
    pub(crate) fn reuse_math(&mut self, math: Math) -> Option<MathId> {
        self.math_reuse.remove(&math_key(math))
    }
    fn note_peak(&mut self) {
        self.peak_entries = self
            .peak_entries
            .max(self.states.len() + self.link_reuse.len() + self.math_reuse.len());
    }
    pub(crate) fn clear_reuse(&mut self) {
        self.note_peak();
        self.link_reuse.clear();
        self.math_reuse.clear();
        self.replay = None;
    }
}

pub(crate) fn commit(p: &mut crate::inline::Publishing<'_>, work: &mut Work) {
    if !p.line.pending {
        return;
    }
    let mut line = std::mem::take(p.line);
    line.pending = false;
    p.display.shape.emit(crate::DisplayShapeEffectKind::Commit(
        crate::DisplayShapeDomain::Replacements,
    ));
    work.header_effect_steps += p.annotations.commit_line()
        + p.styles.commit_line()
        + p.link_order.commit_line()
        + p.footnotes.commit_reference_line();
    for (id, state) in std::mem::take(&mut line.states) {
        p.link_info[id.0].state = state;
        p.link_order
            .shape
            .emit(crate::DisplayShapeEffectKind::LinkState { id, state });
        p.update.changed_links.start = p.update.changed_links.start.min(id.0);
        work.header_effect_steps += 1;
    }
    for id in line.maths..p.maths.len() {
        p.math_states[id] = SyntaxState::Sealed;
        p.update
            .display_shape
            .emit(crate::DisplayShapeEffectKind::Math {
                id: MathId(id),
                math: p.maths[id],
                state: Some(SyntaxState::Sealed),
            });
        work.header_effect_steps += 1;
    }
    line.note_peak();
    *p.line = line;
}

pub(crate) fn rollback(p: &mut crate::inline::Publishing<'_>, work: &mut Work) {
    assert!(p.line.pending);
    let mut line = std::mem::take(p.line);
    line.pending = false;
    p.display
        .shape
        .emit(crate::DisplayShapeEffectKind::Rollback(
            crate::DisplayShapeDomain::Replacements,
        ));
    work.header_effect_steps += p.annotations.rollback_line() + p.link_order.rollback_line();
    let (changed, steps) = p.styles.rollback_line();
    work.header_effect_steps += steps;
    work.header_effect_steps += p.display.truncate_entries(line.display);
    p.math_gaps.truncate(line.gaps);
    for id in line.links..p.links.len() {
        assert_ne!(
            p.link_info[id].state,
            SyntaxState::Sealed,
            "a header must not revoke sealed syntax"
        );
        line.link_reuse
            .insert(link_key(p.link_info[id]), LinkId(id));
        p.link_order.deactivate(LinkId(id));
        p.update.changed_links.start = p.update.changed_links.start.min(id);
        work.header_effect_steps += 1;
    }
    for id in line.maths..p.maths.len() {
        assert_ne!(
            p.math_states[id],
            SyntaxState::Sealed,
            "a header must not revoke sealed math"
        );
        line.math_reuse.insert(math_key(p.maths[id]), MathId(id));
        p.math_active[id] = false;
        p.update
            .display_shape
            .emit(crate::DisplayShapeEffectKind::Math {
                id: MathId(id),
                math: p.maths[id],
                state: None,
            });
        p.math_index.remove(&p.maths[id].source.start);
        work.header_effect_steps += 1;
    }
    work.header_effect_steps += p.footnotes.rollback_reference_line(p.update);
    work.header_effect_steps += line.states.len() as u64;
    line.note_peak();
    line.states.clear();
    p.annotations
        .apply(changed, crate::Style::default(), None, p.update.revision);
    p.update.change(changed, crate::ChangeKind::Visibility);
    p.update.change(changed, crate::ChangeKind::Text);
    p.update.change(changed, crate::ChangeKind::Structure);
    p.update.dirty.push(changed);
    line.note_peak();
    *p.line = line;
}

/// Actual source-parser storage observations. Each peak is a sum of actual
/// component peaks, a conservative bound on simultaneous use. Undo bytes count
/// allocated log capacity and moved buffers; BTree indexes count entries.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ConfirmationStats {
    pub inline_undo_peak: usize,
    pub inline_undo_bytes_peak: usize,
    pub eligibility_undo_peak: usize,
    pub eligibility_undo_bytes_peak: usize,
    pub annotation_undo_peak: usize,
    pub annotation_undo_bytes_peak: usize,
    pub style_undo_peak: usize,
    pub style_undo_bytes_peak: usize,
    pub parent_undo_peak: usize,
    pub parent_undo_bytes_peak: usize,
    pub footnote_undo_peak: usize,
    pub publication_peak: usize,
    pub style_event_entries: usize,
    pub style_event_peak: usize,
    pub style_index_entries: usize,
    pub style_index_peak: usize,
    pub style_scopes: usize,
}
impl crate::Document {
    /// Fixed component observations and at most one O(log n) active-scope lookup;
    /// does not traverse history or allocate. Query resolver storage is reported
    /// separately by LinkResolver::memo_entries/style_work.
    pub fn confirmation_stats(&self) -> ConfirmationStats {
        let w = self.work();
        let a = self.annotations.undo_stats();
        let p = self.link_order.undo_stats();
        let (events, event_peak, index, index_peak, styles, scopes) =
            self.inline_styles.confirmation_stats();
        ConfirmationStats {
            inline_undo_peak: w.inline_undo_peak as usize,
            inline_undo_bytes_peak: w.inline_undo_bytes_peak as usize,
            eligibility_undo_peak: w.eligibility_undo_peak as usize,
            eligibility_undo_bytes_peak: w.eligibility_undo_bytes_peak as usize,
            annotation_undo_peak: a.entries,
            annotation_undo_bytes_peak: a.bytes,
            style_undo_peak: styles.entries,
            style_undo_bytes_peak: styles.bytes,
            parent_undo_peak: p.entries,
            parent_undo_bytes_peak: p.bytes,
            footnote_undo_peak: self.footnotes.confirmation_peak(),
            publication_peak: self.header_effects.peak_entries,
            style_event_entries: events,
            style_event_peak: event_peak,
            style_index_entries: index,
            style_index_peak: index_peak,
            style_scopes: scopes,
        }
    }
}
