//! Explicit, bounded previews. These bytes never become visible original copy.
use super::{
    checkpoint::RenderCheckpoint,
    objects::{LinkLease, SharedLinks},
};
use mdtui3::{
    Budget, CodeInfoStep, Document, DocumentId, ExcerptStep, FoldId, FoldView, LayoutError, Span,
    TablePreviewStep,
};

enum Reader {
    Code(mdtui3::CodeInfoCursor),
    Display(mdtui3::DisplayExcerpt),
    Table(mdtui3::TablePreview),
}
#[derive(Clone)]
pub(crate) struct FoldSummary {
    binding: std::rc::Rc<mdtui3::FoldProjection>,
    pub id: FoldId,
    pub source: Span,
    pub lines: Vec<String>,
    pub charged_source_bytes: usize,
}
impl FoldSummary {
    pub fn check_binding(
        &self,
        doc: &Document,
        view: &FoldView<RenderCheckpoint>,
    ) -> Result<(), LayoutError> {
        self.binding.check_binding(doc, view)
    }
}
pub(crate) struct SummaryJob {
    binding: std::rc::Rc<mdtui3::FoldProjection>,
    document: DocumentId,
    revision: u64,
    version: u64,
    id: FoldId,
    source: Span,
    width: u16,
    reader: Reader,
    text: String,
    row: Option<usize>,
    lfs: usize,
    source_bytes: usize,
    _lease: LinkLease,
}
impl SummaryJob {
    pub fn new(
        doc: &Document,
        view: &FoldView<RenderCheckpoint>,
        id: FoldId,
        width: u16,
        links: &SharedLinks,
    ) -> Result<Self, LayoutError> {
        let candidate = view
            .fold_candidate(doc, id)?
            .ok_or(LayoutError::InvalidRange)?;
        let source = candidate.summary.unwrap_or(candidate.source);
        let reader = match id {
            FoldId::Code(block) if doc.code_info_prefix(block, 256).is_some() => {
                Reader::Code(doc.code_info_prefix(block, 256).unwrap())
            }
            FoldId::Table(table) => Reader::Table(mdtui3::TablePreview::new(
                doc,
                table,
                mdtui3::TablePreviewLimits {
                    rows: 2,
                    source_bytes: 256,
                    display_bytes: 128,
                },
            )?),
            _ => Reader::Display(mdtui3::DisplayExcerpt::new(
                doc,
                source,
                mdtui3::ExcerptLimits {
                    source_bytes: 256,
                    display_bytes: 128,
                    lines: 2,
                },
            )?),
        };
        let binding = mdtui3::FoldProjection::new(doc, view, Span::new(0, 0))?;
        Ok(Self {
            binding: std::rc::Rc::new(binding),
            document: doc.id(),
            revision: doc.revision(),
            version: view.version(),
            id,
            source,
            width,
            reader,
            text: String::new(),
            row: None,
            lfs: 0,
            source_bytes: 0,
            _lease: links.hold(),
        })
    }
    fn push(&mut self, text: &str) -> bool {
        for ch in text.chars() {
            if self.text.len() + ch.len_utf8() > 128 || self.lfs >= 2 {
                return true;
            }
            if ch == '\n' {
                self.lfs += 1;
            }
            if ch == '\t' || ch == '\r' {
                self.text.push(' ');
            } else if !ch.is_control() || ch == '\n' {
                self.text.push(ch);
            }
        }
        false
    }
    pub fn advance(
        &mut self,
        doc: &Document,
        view: &FoldView<RenderCheckpoint>,
        links: &SharedLinks,
        budget: &mut Budget,
    ) -> Result<Option<FoldSummary>, LayoutError> {
        self.binding.check_binding(doc, view)?;
        if self.document != doc.id() {
            return Err(LayoutError::WrongDocument);
        }
        if self.revision != doc.revision() || self.version != view.version() {
            return Err(LayoutError::StaleContinuation);
        }
        loop {
            let before = budget.used().bytes;
            let mut finished = false;
            let mut pending = false;
            let mut text = String::new();
            let mut next_row = None;
            match &mut self.reader {
                Reader::Code(reader) => match reader.next(doc, budget)? {
                    CodeInfoStep::Atom { utf8, len, .. } => text.push_str(
                        std::str::from_utf8(&utf8[..usize::from(len)])
                            .expect("code info emits UTF-8"),
                    ),
                    CodeInfoStep::Complete => finished = true,
                    CodeInfoStep::BudgetExhausted => pending = true,
                },
                Reader::Display(reader) => {
                    let result = links.with_resolver(|resolver| {
                        reader.swap_link_resolver(resolver)?;
                        let step = reader.next(doc, budget);
                        reader.swap_link_resolver(resolver)?;
                        step
                    })?;
                    match result {
                        ExcerptStep::Run(run) => text.push_str(run.text),
                        ExcerptStep::Complete(_) => finished = true,
                        ExcerptStep::BudgetExhausted => pending = true,
                    }
                }
                Reader::Table(reader) => match links
                    .with_resolver(|resolver| reader.next_with_resolver(doc, resolver, budget))?
                {
                    TablePreviewStep::Run { row, run } => {
                        next_row = Some(row);
                        text.push_str(run.text);
                    }
                    TablePreviewStep::Complete(_) => finished = true,
                    TablePreviewStep::BudgetExhausted => pending = true,
                },
            }
            self.source_bytes += budget.used().bytes - before;
            if let Some(row) = next_row {
                if self.row.is_some_and(|old| old != row) && !self.text.ends_with('\n') {
                    finished |= self.push("\n");
                }
                self.row = Some(row);
            }
            finished |= self.push(&text);
            if finished {
                let lines = if self.width < 3 {
                    Vec::new()
                } else {
                    self.text
                        .lines()
                        .take(2)
                        .filter(|line| !line.is_empty())
                        .map(|line| {
                            let text = format!(" {line}");
                            let fit = super::text::fit_prefix(&text, usize::from(self.width));
                            text[..fit.end].to_owned()
                        })
                        .collect()
                };
                return Ok(Some(FoldSummary {
                    binding: self.binding.clone(),
                    id: self.id,
                    source: self.source,
                    lines,
                    charged_source_bytes: self.source_bytes,
                }));
            }
            if pending {
                return Ok(None);
            }
        }
    }
}
