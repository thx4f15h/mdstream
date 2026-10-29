use std::collections::BTreeMap;

use crate::{Reference, Span, SymbolId, Work};

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
            if c.is_whitespace() {
                space = !normalized.is_empty();
            } else {
                if space {
                    normalized.push(' ');
                    space = false;
                }
                normalized.extend(c.to_lowercase());
            }
        }
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
        self.revision = revision;
        true
    }
}
