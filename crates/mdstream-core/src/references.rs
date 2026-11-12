use std::collections::BTreeMap;

use crate::{Reference, Span, SymbolId, Work};

#[path = "case_folding.rs"]
mod case_folding;

pub const MAX_LABEL_BYTES: usize = 999;

pub(crate) fn normalize_label(label: &str, work: &mut Work) -> Option<String> {
    if label.len() > MAX_LABEL_BYTES {
        return None;
    }
    work.label_bytes += label.len() as u64;
    let mut normalized = String::new();
    let mut space = false;
    for c in label.chars() {
        work.label_scalars += 1;
        if c.is_whitespace() {
            space = !normalized.is_empty();
        } else {
            if space {
                normalized.push(' ');
                space = false;
            }
            // Fixed Unicode 17.0.0 C+F table. One lookup per non-space
            // scalar, <= 12 comparisons; absent scalars map to themselves.
            match case_folding::MAPPINGS.binary_search_by_key(&c, |&(key, _)| {
                work.label_fold_steps += 1;
                key
            }) {
                Ok(index) => normalized.push_str(case_folding::MAPPINGS[index].1),
                Err(_) => normalized.push(c),
            }
        }
    }
    // The pinned data proves <= 3 output bytes per raw input byte, even
    // when the normalized key grows beyond the raw-label limit of 999.
    work.label_key_bytes += normalized.len() as u64;
    if normalized.is_empty() {
        return None;
    }
    Some(normalized)
}

/// BTreeMap avoids relying on expected-time hashing for the complexity contract.
#[derive(Default)]
pub(crate) struct References {
    pub(crate) shape: crate::DisplayShapeEffects,
    pub(crate) journal: Option<Vec<Event>>,
    pub(crate) positions: Vec<Option<usize>>,
    names: BTreeMap<String, SymbolId>,
    pub entries: Vec<Reference>,
    // Parallel to entries: O(1) lookup, retained only for winning definitions.
    pub title_gaps: Vec<Vec<Span>>,
    pub(crate) title_spaces: Vec<Vec<(Span, u8)>>,
    pub revision: u64,
    pub(crate) slots: Vec<std::sync::Arc<crate::symbol_context::Symbol>>,
    pub(crate) bindings: Vec<Option<std::sync::Arc<crate::symbol_context::Symbol>>>,
}

#[derive(Debug)]
pub(crate) enum Event {
    Use(SymbolId, usize),
    Definition {
        symbol: SymbolId,
        position: usize,
        destination: Span,
        title: Option<Span>,
        gaps: Vec<Span>,
    },
    Title {
        symbol: SymbolId,
        position: usize,
        title: Span,
        gaps: Vec<Span>,
    },
}

impl References {
    pub fn intern(&mut self, label: &str, work: &mut Work) -> Option<SymbolId> {
        let normalized = normalize_label(label, work)?;
        Some(self.intern_key(&normalized))
    }
    // The local producer already checked and normalized the original label.
    pub(crate) fn intern_key(&mut self, normalized: &str) -> SymbolId {
        if let Some(&id) = self.names.get(normalized) {
            return id;
        }
        let id = SymbolId(self.entries.len());
        self.names.insert(normalized.to_owned(), id);
        self.entries.push(Reference {
            label: normalized.to_owned(),
            destination: None,
            title: None,
            revision: 0,
            first_use: None,
        });
        self.title_gaps.push(Vec::new());
        self.title_spaces.push(Vec::new());
        self.slots.push(Default::default());
        self.bindings.push(None);
        self.positions.push(None);
        id
    }

    pub fn define(
        &mut self,
        id: SymbolId,
        position: usize,
        destination: Span,
        title: Option<Span>,
        gaps: Vec<Span>,
        revision: u64,
    ) -> bool {
        if let Some(journal) = &mut self.journal {
            journal.push(Event::Definition {
                symbol: id,
                position,
                destination,
                title,
                gaps: gaps.clone(),
            });
        }
        let entry = &mut self.entries[id.0];
        if self.positions[id.0].is_some_and(|old| old <= position) {
            return false;
        }
        self.positions[id.0] = Some(position);
        entry.destination = Some(destination);
        entry.title = title;
        self.title_gaps[id.0] = gaps;
        self.title_spaces[id.0].clear();
        entry.revision += 1;
        self.revision = revision;
        self.shape.emit(crate::DisplayShapeEffectKind::Reference {
            id,
            generation: entry.revision,
        });
        self.slots[id.0].publish(true, entry.revision);
        true
    }

    /// Only the winning definition may retain a continuation capability.
    pub fn define_title(&mut self, id: SymbolId, title: Span, gaps: Vec<Span>, revision: u64) {
        if let Some(journal) = &mut self.journal {
            journal.push(Event::Title {
                symbol: id,
                position: self.positions[id.0].unwrap(),
                title,
                gaps: gaps.clone(),
            });
        }
        let entry = &mut self.entries[id.0];
        debug_assert!(entry.destination.is_some() && entry.title.is_none());
        entry.title = Some(title);
        self.title_gaps[id.0] = gaps;
        self.title_spaces[id.0].clear();
        entry.revision += 1;
        self.revision = revision;
        self.shape.emit(crate::DisplayShapeEffectKind::Reference {
            id,
            generation: entry.revision,
        });
        self.slots[id.0].publish(true, entry.revision);
    }
    pub(crate) fn semantic_revision(&self, id: SymbolId) -> u64 {
        self.bindings[id.0]
            .as_ref()
            .map_or(self.entries[id.0].revision, |s| s.revision())
    }
}

pub(crate) trait Lookup {
    fn resolved(&self, id: SymbolId) -> bool;
}
impl Lookup for References {
    fn resolved(&self, id: SymbolId) -> bool {
        self.bindings[id.0]
            .as_ref()
            .map_or(self.entries[id.0].destination.is_some(), |s| s.resolved())
    }
}
impl Lookup for [Reference] {
    fn resolved(&self, id: SymbolId) -> bool {
        self[id.0].destination.is_some()
    }
}
impl Lookup for Vec<Reference> {
    fn resolved(&self, id: SymbolId) -> bool {
        self.as_slice().resolved(id)
    }
}
impl<const N: usize> Lookup for [Reference; N] {
    fn resolved(&self, id: SymbolId) -> bool {
        self.as_slice().resolved(id)
    }
}

#[cfg(test)]
#[path = "../tests/support/case_folding.rs"]
mod model;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_unicode_scalar_matches_independent_data_and_work_bounds() {
        let table = model::mappings();
        assert_eq!(case_folding::MAPPINGS.len(), table.len());
        for (&(actual, value), (&expected, mapping)) in case_folding::MAPPINGS.iter().zip(&table) {
            assert_eq!((actual, value), (expected, mapping.as_str()));
        }
        let mut count = 0;
        // All valid scalars, including every unlisted identity and whitespace.
        for c in (0..=0x10ffff).filter_map(char::from_u32) {
            let raw = format!("x{c}z");
            let expected = model::normalize(&raw, &table).unwrap();
            let mut references = References::default();
            let mut work = Work::default();
            assert_eq!(references.intern(&raw, &mut work), Some(SymbolId(0)));
            assert_eq!(references.entries[0].label, expected, "U+{:04X}", c as u32);
            assert_eq!(work.label_bytes, raw.len() as u64);
            assert_eq!(work.label_scalars, 3);
            assert!(work.label_fold_steps <= 12 * work.label_scalars);
            assert_eq!(work.label_key_bytes, expected.len() as u64);
            assert!(work.label_key_bytes <= 3 * work.label_bytes);
            assert!(expected.chars().count() <= 9);
            count += 1;
        }
        assert_eq!(count, 1_112_064);
    }

    #[test]
    fn raw_length_is_checked_before_any_folding_work() {
        let mut references = References::default();
        let mut work = Work::default();
        let long = "ΐ".repeat(500);
        assert_eq!(references.intern(&long, &mut work), None);
        assert_eq!(work, Work::default());
        let raw = format!("{}a", "ΐ".repeat(499));
        assert_eq!(references.intern(&raw, &mut work), Some(SymbolId(0)));
        assert_eq!(references.entries[0].label.len(), 2995);
        assert_eq!(work.label_bytes, 999);
        assert_eq!(work.label_scalars, 500);
        assert_eq!(work.label_key_bytes, 2995);
        let before = work;
        assert_eq!(references.intern(&format!("{raw}a"), &mut work), None);
        assert_eq!(work, before);
    }
}
