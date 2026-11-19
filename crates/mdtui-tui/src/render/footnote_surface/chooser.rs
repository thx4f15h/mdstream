//! Paid construction of a finite chooser frame; wire drains its cached frame.
use super::*;
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct ChooserGeneration {
    revision: u64,
    width: u16,
    rows: u16,
    selected: Option<FootnoteId>,
    backlinks: Option<FootnoteId>,
    reference: Option<(usize, usize)>,
    offset: usize,
}
pub(super) struct ChooserBuild {
    generation: ChooserGeneration,
    lines: Vec<super::super::super::sink::SinkLine>,
    note: Option<Order>,
    reference: Option<(usize, usize)>,
}
fn line(text: &str, width: u16, offset: usize) -> super::super::super::sink::SinkLine {
    let mut result = ui_line("", width);
    let mut used = 0;
    for glyph in graphemes(text).skip(offset) {
        if used + u16::from(glyph.width) > width {
            break;
        }
        used += u16::from(glyph.width);
        result.cells.push(super::super::super::sink::SinkCell {
            text: glyph.text.into(),
            width: glyph.width,
            style: StyleRole::META,
            ..Default::default()
        });
    }
    result
}
impl FootnoteUi {
    pub(super) fn chooser_frame(
        &mut self,
        doc: &Document,
        width: u16,
        rows: u16,
        budget: &mut Budget,
    ) -> Result<Option<ViewportFrame>, FrameError> {
        let generation = ChooserGeneration {
            revision: doc.revision(),
            width,
            rows,
            selected: self.selected,
            backlinks: self.backlinks,
            reference: self.selected_reference,
            offset: self.chooser_offset,
        };
        if let Some((key, frame)) = &self.chooser_paint
            && *key == generation
        {
            return Ok(Some(frame.clone()));
        }
        if self
            .chooser_build
            .as_ref()
            .is_none_or(|b| b.generation != generation)
        {
            if !budget.charge(WorkLimit {
                queries: 1,
                objects: 1,
                ..Default::default()
            }) {
                return Ok(None);
            }
            self.chooser_build = Some(ChooserBuild {
                generation,
                lines: vec![ui_line(
                    if self.backlinks.is_some() {
                        "Footnote sources"
                    } else {
                        "Footnotes"
                    },
                    width,
                )],
                note: None,
                reference: None,
            });
        }
        let build = self.chooser_build.as_mut().unwrap();
        while build.lines.len() < usize::from(rows) {
            if !budget.charge(WorkLimit {
                queries: 1,
                objects: 1,
                ..Default::default()
            }) {
                return Ok(None);
            }
            let label = if let Some(symbol) = self.backlinks {
                let start = self.selected_reference.unwrap_or((symbol.0, 0));
                let next = if let Some(after) = build.reference {
                    self.references
                        .range((
                            std::ops::Bound::Excluded(after),
                            std::ops::Bound::Included((symbol.0, usize::MAX)),
                        ))
                        .next()
                } else {
                    self.references.range(start..=(symbol.0, usize::MAX)).next()
                };
                let Some((&position, occurrence)) = next else {
                    break;
                };
                build.reference = Some(position);
                match occurrence.address {
                    ReturnAddress::Root(_, source) => format!("main @{source}"),
                    ReturnAddress::Body(source) => format!(
                        "note {} @{}",
                        number(doc.footnote(source.body.symbol()).and_then(|n| n.number)),
                        source.span.start
                    ),
                }
            } else {
                let start = self
                    .selected
                    .and_then(|id| self.positions.get(&id).copied())
                    .unwrap_or((false, 0, 0));
                let next = if let Some(after) = build.note {
                    self.entries
                        .range((std::ops::Bound::Excluded(after), std::ops::Bound::Unbounded))
                        .next()
                } else {
                    self.entries.range(start..).next()
                };
                let Some((&position, &id)) = next else { break };
                build.note = Some(position);
                let note = doc.footnote(id).ok_or(LayoutError::InvalidRange)?;
                format!(
                    "{} {} [{}]",
                    number(note.number),
                    note.label,
                    target(doc, id)
                )
            };
            build.lines.push(line(&label, width, self.chooser_offset));
        }
        let build = self.chooser_build.take().unwrap();
        let mut lines = build.lines;
        if lines.len() == 1 && rows > 1 {
            lines.push(ui_line(
                if self.backlinks.is_some() {
                    "No references"
                } else {
                    "No footnotes"
                },
                width,
            ));
        }
        let frame = ViewportFrame {
            document: doc.id(),
            revision: doc.revision(),
            lines,
            top: TopPosition::Empty,
            rows,
            status: ViewportFrameStatus::Ready,
            stats: Default::default(),
        };
        self.chooser_paint = Some((generation, frame.clone()));
        Ok(Some(frame))
    }
}
