//! One release per failed primary code candidate. Original producer events
//! retain logical payload/prefix ownership; replay never records itself.
//! Raw runs index future closers, including those in would-be opaque grammars.
use super::Effects;
use crate::{Span, Work};

#[derive(Clone, Copy, Debug)]
pub(super) enum Event {
    Raw(Span),
    Virtual { at: usize, columns: u8 },
    Prefix(Span),
}

impl Event {
    pub fn end(self) -> usize {
        match self {
            Self::Raw(s) | Self::Prefix(s) => s.end,
            Self::Virtual { at, .. } => at + 1,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Checkpoint {
    pub open: Span,
    pub label_len: usize,
    pub label_overflow: bool,
}

#[derive(Default)]
pub(super) struct CodeFallback {
    pub runs: Vec<Span>,
    pub events: Vec<Event>,
    pub checkpoint: Option<Checkpoint>,
    pub boundary: Option<(usize, Option<Event>, usize)>,
}

#[cfg(test)]
impl std::fmt::Debug for CodeFallback {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The per-event syntax differential must not format a growing trace
        // merely to exclude its producer-only routing data afterward.
        f.debug_struct("CodeFallback")
            .field("runs", &self.runs)
            .field("checkpoint", &self.checkpoint)
            .field(
                "boundary",
                &self
                    .boundary
                    .map(|(_, last, runs)| (last.map(Event::end), runs)),
            )
            .finish()
    }
}

impl CodeFallback {
    pub fn clear(&mut self) {
        self.runs.clear();
        self.checkpoint = None;
        self.boundary = None;
    }

    pub fn record(&mut self, event: Event, work: &mut Work) {
        work.code_fallback_steps += 1;
        if let Event::Raw(span) = event
            && let Some(Event::Raw(last)) = self.events.last_mut()
            && last.end == span.start
        {
            last.end = span.end;
            return;
        }
        self.events.push(event);
    }

    pub fn push(&mut self, open: Span) {
        self.runs.push(open);
    }

    pub fn isolate(&mut self) {
        self.boundary = Some((
            self.events.len(),
            self.events.last().copied(),
            self.runs.len(),
        ));
    }

    pub fn truncate_header(&mut self) -> bool {
        let Some((len, last, runs)) = self.boundary.take() else {
            return false;
        };
        self.events.truncate(len);
        if let Some(last) = last {
            self.events[len - 1] = last;
        }
        self.runs.truncate(runs);
        true
    }

    pub fn index(&mut self, effects: &mut Effects<'_>) -> Vec<Span> {
        let mut runs = self.runs.clone();
        effects.work.code_fallback_steps += runs.len() as u64;
        runs.sort_unstable_by(|a, b| {
            effects.work.code_fallback_comparisons += 1;
            (a.len(), a.start).cmp(&(b.len(), b.start))
        });
        self.clear();
        runs
    }
}

pub(super) fn has_closer(index: &[Span], open: Span, work: &mut Work) -> bool {
    work.code_fallback_steps += 1;
    let at = index.partition_point(|s| {
        work.code_fallback_comparisons += 1;
        (s.len(), s.start) <= (open.len(), open.start)
    });
    index.get(at).is_some_and(|s| s.len() == open.len())
}
