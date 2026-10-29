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
        Ok(Some(Self { tile, key, symbols }))
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
            dependencies: self.symbols.len(),
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
