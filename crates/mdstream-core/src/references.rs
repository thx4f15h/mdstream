use std::collections::BTreeMap;

use crate::{Reference, Span, SymbolId, Work};

#[path = "case_folding.rs"]
mod case_folding;

pub const MAX_LABEL_BYTES: usize = 999;

/// BTreeMap avoids relying on expected-time hashing for the complexity contract.
#[derive(Default)]
pub(crate) struct References {
    names: BTreeMap<String, SymbolId>,
    pub entries: Vec<Reference>,
    pub revision: u64,
}

impl References {
    pub fn intern(&mut self, label: &str, work: &mut Work) -> Option<SymbolId> {
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
        if let Some(&id) = self.names.get(&normalized) {
            return Some(id);
        }
        let id = SymbolId(self.entries.len());
        self.names.insert(normalized.clone(), id);
        self.entries.push(Reference {
            label: normalized,
            destination: None,
            title: None,
            revision: 0,
            first_use: None,
        });
        Some(id)
    }

    pub fn define(
        &mut self,
        id: SymbolId,
        destination: Span,
        title: Option<Span>,
        revision: u64,
    ) -> bool {
        let entry = &mut self.entries[id.0];
        if entry.destination.is_some() {
            return false;
        }
        entry.destination = Some(destination);
        entry.title = title;
        entry.revision += 1;
        self.revision = revision;
        true
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
