//! Current canonical display metadata without visiting literal payload bytes.
//! This is a child of display.rs so the replacement precedence is shared with
//! DisplayCursor. It deliberately has no screen presentation transformations.
use super::*;

#[derive(Clone, Copy, Debug)]
pub(crate) enum Value {
    Raw,
    Code(Span),
    Hidden,
    Entity {
        source: Span,
        utf8: [u8; 8],
        len: u8,
    },
    Spaces(u8),
}
#[derive(Clone, Copy, Debug)]
pub(crate) struct Unit {
    pub source: Span,
    pub value: Value,
    pub links: [Option<crate::LinkId>; 3],
}
struct Pending {
    unit: Unit,
    task: crate::link_semantics::MarkTask,
    revision: u64,
}
pub(crate) struct MetadataCursor {
    pub position: usize,
    end: usize,
    pending: Option<Pending>,
}
impl MetadataCursor {
    pub fn new(source: Span) -> Self {
        Self {
            position: source.start,
            end: source.end,
            pending: None,
        }
    }
    pub fn capacity(&self) -> usize {
        self.pending
            .as_ref()
            .map_or(0, |p| p.task.continuation_capacity())
    }
    pub fn retire(&mut self, b: &mut Budget) -> bool {
        if let Some(p) = &mut self.pending
            && !p.task.retire_continuation(b)
        {
            return false;
        }
        self.pending = None;
        true
    }
    pub fn next(
        &mut self,
        doc: &Document,
        resolver: &mut crate::LinkResolver,
        b: &mut Budget,
    ) -> Result<Result<Option<Unit>, crate::Completion>, LayoutError> {
        resolver.check(doc)?;
        loop {
            if self.position == self.end {
                return Ok(Ok(None));
            }
            if let Some(p) = &mut self.pending {
                if p.revision != doc.revision() && resolver.can_rebind(doc, p.revision) {
                    p.task.retain_revision(doc);
                    p.revision = doc.revision();
                }
                if p.revision != doc.revision() {
                    if !p.task.retire_continuation(b) {
                        return Ok(Err(crate::Completion::BudgetExhausted));
                    }
                    self.pending = None;
                    continue;
                }
                if !p.task.valid_memo(&resolver.resolution) && !p.task.retire_continuation(b) {
                    return Ok(Err(crate::Completion::BudgetExhausted));
                }
                if !p.task.advance(doc, &mut resolver.resolution, b, 0) {
                    return Ok(Err(crate::Completion::BudgetExhausted));
                }
                let mut unit = p.unit;
                if p.task.result.unwrap().0 {
                    unit.value = Value::Hidden;
                }
                if !p.task.retire_continuation(b) {
                    return Ok(Err(crate::Completion::BudgetExhausted));
                }
                self.position = unit.source.end;
                self.pending = None;
                return Ok(Ok(Some(unit)));
            }
            if !resolver.resolution.prepare_styles(doc, self.position, b)? {
                return Ok(Err(crate::Completion::BudgetExhausted));
            }
            if !b.charge(WorkLimit {
                queries: 1,
                objects: 1,
                ..Default::default()
            }) {
                return Ok(Err(crate::Completion::BudgetExhausted));
            }
            let snapshot = resolver
                .resolution
                .style_snapshot(doc, self.position, self.end);
            let (mut source, raw) = doc
                .annotations
                .current_leaf(self.position, self.end.min(snapshot.next));
            // Tree subdivisions may cut a UTF-8 scalar. An annotation/style
            // mutation never does; extending through at most three continuation
            // bytes preserves that scalar and its identical mark.
            while !doc.source().is_char_boundary(source.end) {
                source.end += 1;
            }
            let mut mark = snapshot.apply(raw);
            if let Some(p) = doc.indented_code_pending() {
                if source.start < p.source.start {
                    source.end = source.end.min(p.source.start);
                } else if source.start < p.source.end {
                    source.end = source.end.min(p.source.end);
                    mark.style = mark.style.union(Style::HIDDEN);
                }
            }
            let style = mark.style;
            let mut end = source.end;
            let at = source.start;
            let mut value = Value::Raw;
            let entry = doc.display.entries.partition_point(|e| e.source.end <= at);
            let space = doc.display.spaces.partition_point(|(s, _)| s.end <= at);
            let mut columns = None;
            if let Some(&(s, n)) = doc.display.spaces.get(space) {
                if at < s.start {
                    end = end.min(s.start);
                } else {
                    end = end.min(s.end);
                    columns = Some(n);
                }
            }
            if let Some(e) = doc.display.entries.get(entry) {
                if at < e.source.start {
                    end = end.min(e.source.start);
                } else {
                    end = end.min(e.source.end);
                    match &e.kind {
                        Kind::Entity(text) => {
                            if at == e.source.start {
                                let mut utf8 = [0; 8];
                                utf8[..text.len()].copy_from_slice(text.as_bytes());
                                value = Value::Entity {
                                    source: e.source,
                                    utf8,
                                    len: text.len() as u8,
                                };
                            } else {
                                value = Value::Hidden;
                            }
                        }
                        Kind::Code { leading, trailing } => {
                            if at < leading.end {
                                end = end.min(leading.end);
                                value = Value::Hidden;
                            } else if at >= trailing.start {
                                value = Value::Hidden;
                                if let Some(n) = &mut columns {
                                    *n -= 1;
                                }
                            } else {
                                end = end.min(trailing.start);
                                value = Value::Code(e.source);
                            }
                        }
                    }
                }
            }
            if let Some(n) = columns {
                value = Value::Spaces(n);
            }
            if style.contains(Style::HIDDEN) {
                value = Value::Hidden;
            }
            let unit = Unit {
                source: Span::new(at, end),
                value,
                links: [mark.link, mark.hide_owner(), mark.alternative_owner()],
            };
            if crate::link_semantics::Resolution::needs(doc, mark) {
                self.pending = Some(Pending {
                    unit,
                    task: crate::link_semantics::MarkTask::new(mark),
                    revision: doc.revision(),
                });
            } else {
                let mut unit = unit;
                if mark
                    .effective_style(&doc.references, &doc.links)
                    .contains(Style::HIDDEN)
                {
                    unit.value = Value::Hidden;
                }
                self.position = end;
                return Ok(Ok(Some(unit)));
            }
        }
    }
}
