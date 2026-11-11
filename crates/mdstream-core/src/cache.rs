use crate::{
    Budget, Document, LayoutError, LinkTarget, Span, SymbolId, TileId, TileKey, WorkLimit,
};

/// Direct semantic dependencies. Incoming layout state still needs a measurement
/// and checkpoint ticket; this key alone is not a layout dependency guarantee.
#[derive(Clone, Debug)]
pub struct DependencyKey {
    tile: TileId,
    key: TileKey,
    symbols: Vec<(SymbolId, u64)>,
    footnotes: Vec<(crate::FootnoteId, u64)>,
    finished: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CacheValidity {
    Valid,
    Changed,
    BudgetExhausted,
}

impl DependencyKey {
    /// A tile has at most TILE_BYTES object starts plus one overlapping object.
    /// If exhausted, no partial key is published; consumed charges remain spent.
    pub fn capture(
        document: &Document,
        tile: TileId,
        budget: &mut Budget,
    ) -> Result<Option<Self>, LayoutError> {
        let key = document.tile_key(tile).ok_or(LayoutError::InvalidRange)?;
        if !budget.charge(WorkLimit {
            queries: 1,
            ..Default::default()
        }) {
            return Ok(None);
        }
        let mut symbols = Vec::new();
        for (id, _) in document.links_in(key.source) {
            if !budget.charge(WorkLimit {
                objects: 1,
                dependencies: 1,
                ..Default::default()
            }) {
                return Ok(None);
            }
            if let LinkTarget::Reference(symbol) = document.links()[id.0] {
                symbols.push((symbol, document.references()[symbol.0].revision));
            }
        }
        let mut footnotes = Vec::new();
        let entries = document.footnote_references();
        let start = entries.partition_point(|r| r.source.end <= key.source.start);
        for reference in entries[start..]
            .iter()
            .take_while(|r| r.source.start < key.source.end)
        {
            if !budget.charge(WorkLimit {
                objects: 1,
                dependencies: 1,
                ..Default::default()
            }) {
                return Ok(None);
            }
            footnotes.push((
                reference.symbol,
                document.footnote(reference.symbol).unwrap().revision,
            ));
        }
        Ok(Some(Self {
            tile,
            key,
            symbols,
            footnotes,
            finished: document.is_finished(),
        }))
    }

    pub fn validate(
        &self,
        document: &Document,
        budget: &mut Budget,
    ) -> Result<CacheValidity, LayoutError> {
        if self.key.document != document.id() {
            return Err(LayoutError::WrongDocument);
        }
        if !budget.charge(WorkLimit {
            queries: 1,
            dependencies: self.symbols.len() + self.footnotes.len(),
            ..Default::default()
        }) {
            return Ok(CacheValidity::BudgetExhausted);
        }
        let Some(key) = document.tile_key(self.tile) else {
            return Ok(CacheValidity::Changed);
        };
        Ok(
            if key.source != self.key.source
                || key.syntax_revision != self.key.syntax_revision
                || self.footnotes.iter().any(|(id, version)| {
                    let note = document.footnote(*id).unwrap();
                    note.revision != *version
                        || (note.definition.is_none() && self.finished != document.is_finished())
                })
                || self
                    .symbols
                    .iter()
                    .any(|(id, version)| document.references()[id.0].revision != *version)
            {
                CacheValidity::Changed
            } else {
                CacheValidity::Valid
            },
        )
    }
}

/// Explicit bounded source copy for background jobs. This is source, not a
/// serialized semantic AST. Publish asynchronous heights using a measurement ticket.
#[derive(Clone, Debug)]
pub struct SourceSnapshot {
    pub document: crate::DocumentId,
    pub revision: u64,
    pub source: Span,
    pub text: String,
}

impl Document {
    pub fn snapshot(
        &self,
        source: Span,
        max_bytes: usize,
        budget: &mut Budget,
    ) -> Result<Option<SourceSnapshot>, LayoutError> {
        let text = self.text(source).ok_or(LayoutError::InvalidRange)?;
        if source.len() > max_bytes
            || !budget.charge(WorkLimit {
                bytes: source.len(),
                objects: 1,
                queries: 1,
                ..Default::default()
            })
        {
            return Ok(None);
        }
        Ok(Some(SourceSnapshot {
            document: self.id(),
            revision: self.revision(),
            source,
            text: text.to_owned(),
        }))
    }
}

#[cfg(test)]
mod reference_boundary_tests {
    use super::*;

    #[test]
    fn marker_and_suffix_only_tiles_pay_before_looking_up_the_current_target() {
        let mut doc = Document::new();
        let label = "b".repeat(999);
        doc.append(&format!(
            "{}[a][{label}]\n\n",
            "x".repeat(crate::TILE_BYTES - 1)
        ))
        .unwrap();
        let id = crate::LinkId(0);
        let info = *doc.link_info(id).unwrap();
        let LinkTarget::Reference(b) = doc.links()[0] else {
            panic!()
        };
        assert_eq!(doc.references()[b.0].label, label);
        let mut keys = Vec::new();
        for tile in [TileId(0), TileId(2), TileId(3)] {
            let source = doc.tile(tile).unwrap();
            assert!(source.end <= info.label.start || info.label.end <= source.start);
            assert_eq!(
                doc.links_in(source).map(|(id, _)| id).collect::<Vec<_>>(),
                [id]
            );
            let key = DependencyKey::capture(&doc, tile, &mut Budget::new(WorkLimit::unlimited()))
                .unwrap()
                .unwrap();
            assert_eq!(key.symbols, [(b, 0)]);
            keys.push(key);
        }
        // Empty references intentionally make an early lookup panic. All three
        // failure charges must return before indexing the winning symbol.
        let references = std::mem::take(&mut doc.references.entries);
        for limit in [
            WorkLimit {
                queries: 0,
                ..WorkLimit::unlimited()
            },
            WorkLimit {
                objects: 0,
                ..WorkLimit::unlimited()
            },
            WorkLimit {
                dependencies: 0,
                ..WorkLimit::unlimited()
            },
        ] {
            assert!(
                DependencyKey::capture(&doc, TileId(2), &mut Budget::new(limit))
                    .unwrap()
                    .is_none()
            );
        }
        doc.references.entries = references;
        doc.append("[a]: /old\n").unwrap();
        assert_eq!(doc.link_destination(id), None);
        for key in &keys {
            assert_eq!(
                key.validate(&doc, &mut Budget::new(WorkLimit::unlimited())),
                Ok(CacheValidity::Valid)
            );
        }
        doc.append(&format!("[{label}]: /new\n")).unwrap();
        assert_eq!(doc.link_destination(id), Some("/new"));
        assert_eq!(doc.link_info(id), Some(&info));
        for key in &keys {
            assert_eq!(
                key.validate(&doc, &mut Budget::new(WorkLimit::unlimited())),
                Ok(CacheValidity::Changed)
            );
        }
    }
}
