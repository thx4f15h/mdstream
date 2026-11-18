//! Bounded word metadata for the production list placement. The canonical
//! scanner supplies complete EGCs; this does not decode source or retain text.
use super::list::{ListFlow, ListWordClass, word_class};
use mdtui3::{Budget, Span, WorkLimit};
use std::collections::VecDeque;
#[derive(Clone, Copy)]
pub(crate) struct WordAtom {
    pub source: Span,
    pub width: usize,
    pub whitespace: bool,
    pub first: char,
    pub preserved: bool,
}
pub(crate) struct PrefixList {
    flow: ListFlow,
    rows: usize,
    max_word: usize,
    atoms: VecDeque<WordAtom>,
    width: usize,
    separated: bool,
    boundary: bool,
    prior_preserved: bool,
    pending_space: bool,
    saw_word: bool,
    hard_word: bool,
    draining: Option<bool>,
    finish: bool,
    ended: bool,
    draining_class: ListWordClass,
    pending: Option<WordAtom>,
    admitted: usize,
    cut: Option<usize>,
    pub placements: usize,
}
impl PrefixList {
    pub fn new(first: usize, continued: usize, rows: usize, start: usize) -> Self {
        Self {
            flow: ListFlow::new(first, continued),
            rows,
            max_word: first.max(continued),
            atoms: VecDeque::new(),
            width: 0,
            separated: false,
            boundary: false,
            prior_preserved: false,
            pending_space: false,
            saw_word: false,
            hard_word: false,
            draining: None,
            finish: false,
            ended: false,
            draining_class: ListWordClass::Other,
            pending: None,
            admitted: start,
            cut: None,
            placements: 0,
        }
    }
    pub fn feed(&mut self, atom: WordAtom) {
        self.pending = Some(atom);
    }
    pub fn finish(&mut self) {
        if !self.ended {
            self.finish = true;
            self.ended = true;
        }
    }
    fn start_drain(&mut self, last: bool) {
        if self.atoms.is_empty() {
            if last {
                self.hard_word = false;
                self.flow.finish_word(ListWordClass::Other);
            }
            return;
        }
        let class = word_class(self.atoms[0].first);
        let hard = if self.hard_word {
            true
        } else {
            let Some((_, hard)) = self
                .flow
                .word(self.width, class, self.separated, self.boundary)
            else {
                self.cut = Some(self.atoms[0].source.start.min(self.admitted));
                return;
            };
            if self.flow.height() > self.rows {
                self.cut = Some(self.atoms[0].source.start.min(self.admitted));
                return;
            }
            hard
        };
        self.draining_class = class;
        self.draining = Some(hard);
        self.hard_word = !last;
        self.width = 0;
    }
    /// Each metadata placement/transition pays independently, including a
    /// continuation after a word larger than the entire terminal width.
    pub fn advance(&mut self, budget: &mut Budget, steps: &mut usize) -> bool {
        loop {
            if self.cut.is_some() {
                return true;
            }
            if self.draining.is_none() && self.pending.is_none() && !self.finish {
                return true;
            }
            if !budget.charge(WorkLimit {
                queries: 1,
                objects: 1,
                ..Default::default()
            }) {
                return false;
            }
            *steps += 1;
            if let Some(hard) = self.draining {
                if let Some(atom) = self.atoms.pop_front() {
                    self.placements += 1;
                    self.flow.glyph(atom.width, hard);
                    if self.flow.height() > self.rows {
                        self.cut = Some(atom.source.start.min(self.admitted));
                        return true;
                    }
                    self.admitted = self.admitted.max(atom.source.end);
                } else {
                    self.draining = None;
                    self.saw_word = true;
                    self.flow.finish_word(self.draining_class);
                }
                continue;
            }
            if let Some(atom) = self.pending {
                if atom.whitespace {
                    self.pending = None;
                    self.pending_space = self.saw_word || !self.atoms.is_empty();
                    self.prior_preserved = atom.preserved;
                    self.start_drain(true);
                    continue;
                }
                let class = word_class(atom.first);
                if class != ListWordClass::Other && !self.atoms.is_empty() {
                    self.start_drain(true);
                    continue;
                }
                self.pending = None;
                let boundary = atom.preserved || self.prior_preserved;
                self.prior_preserved = atom.preserved;
                if atom.width == 0 {
                    // Zero-width glyphs attach to the preceding word. Retain
                    // only its source envelope; no unbounded zero-width queue.
                    if let Some(last) = self.atoms.back_mut() {
                        last.source.end = atom.source.end;
                    } else {
                        self.admitted = self.admitted.max(atom.source.end);
                    }
                    continue;
                }
                if self.atoms.is_empty() && !self.hard_word {
                    self.separated = self.pending_space && self.saw_word;
                    self.boundary = boundary;
                }
                self.pending_space = false;
                self.width = self.width.saturating_add(atom.width);
                self.atoms.push_back(atom);
                if class != ListWordClass::Other {
                    self.start_drain(true);
                    self.flow.finish_word(class);
                } else if self.hard_word || self.width > self.max_word {
                    self.start_drain(false);
                }
                continue;
            }
            if self.finish {
                self.finish = false;
                self.start_drain(true);
            }
        }
    }
    pub fn cut(&self) -> Option<usize> {
        self.cut
    }
}
