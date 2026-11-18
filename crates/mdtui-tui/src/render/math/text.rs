//! Row-major projection in bounded horizontal windows. Payload readers, not
//! descriptors, pay for the actual UTF-8 bytes (including arbitrarily long EGCs).
use super::{MathFocus, MathQuery, MathView, Metrics, Rect, SharedMath, ViewCell};
use mdtui3::{Budget, Document, LayoutError, MathObjectMeta, Span, WorkLimit};
use std::rc::Rc;
const COLUMNS: usize = 256;

// A fixed descriptor stays on the stack; boxing would allocate per glyph.
#[allow(clippy::large_enum_variant)]
pub enum MathTextStep {
    Cell { cell: ViewCell, focus: MathFocus },
    Space(Span),
    Newline(Span),
    Complete,
    BudgetExhausted,
}
pub struct MathTextCursor {
    shared: SharedMath,
    meta: MathObjectMeta,
    metrics: Option<Metrics>,
    query: Option<MathQuery>,
    view: Option<Rc<MathView>>,
    after: Option<(usize, usize, usize)>,
    row: usize,
    column: usize,
    window_x: usize,
    window_end: usize,
}
impl MathTextCursor {
    pub fn new(
        document: &Document,
        shared: SharedMath,
        meta: MathObjectMeta,
    ) -> Result<Self, LayoutError> {
        if document.id() != meta.document {
            return Err(LayoutError::WrongDocument);
        }
        if document.math_object(meta.id) != Some(meta) {
            return Err(LayoutError::StaleContinuation);
        }
        let query = shared.query(
            document,
            meta,
            Rect {
                x: 0,
                y: 0,
                width: 0,
                height: 0,
            },
        )?;
        Ok(Self {
            shared,
            meta,
            metrics: None,
            query: Some(query),
            view: None,
            after: None,
            row: 0,
            column: 0,
            window_x: 0,
            window_end: 0,
        })
    }
    pub fn metrics(&self) -> Option<Metrics> {
        self.metrics
    }
    pub fn output_storage(&self) -> super::StorageWork {
        self.query.as_ref().map_or_default(|q| q.output_storage())
    }
    pub fn advance(
        &mut self,
        document: &Document,
        budget: &mut Budget,
    ) -> Result<MathTextStep, LayoutError> {
        if document.id() != self.meta.document {
            return Err(LayoutError::WrongDocument);
        }
        if document.math_object(self.meta.id) != Some(self.meta) {
            return Err(LayoutError::StaleContinuation);
        }
        loop {
            if self.metrics.is_some_and(|m| self.row >= m.height) {
                return Ok(MathTextStep::Complete);
            }
            if budget.remaining().queries == 0 || budget.remaining().objects == 0 {
                return Ok(MathTextStep::BudgetExhausted);
            }
            if self.metrics.is_none() {
                if self.query.is_none() {
                    self.query = Some(self.shared.query(
                        document,
                        self.meta,
                        Rect {
                            x: 0,
                            y: 0,
                            width: 0,
                            height: 0,
                        },
                    )?);
                }
                if !self.query.as_mut().unwrap().advance(document, budget)? {
                    return Ok(MathTextStep::BudgetExhausted);
                }
                self.metrics = Some(self.query.as_ref().unwrap().result().unwrap().metrics);
                self.query = None;
                continue;
            }
            let width = self.metrics.unwrap().width.saturating_add(1);
            if self.query.is_none() {
                self.window_end = self.window_x.saturating_add(COLUMNS).min(width);
                // The shared oracle's maximum two-column glyph is admitted
                // whole at the right seam, then emitted only by its start tile.
                self.query = Some(self.shared.query(
                    document,
                    self.meta,
                    Rect {
                        x: self.window_x,
                        y: self.row,
                        width: (COLUMNS + 1).min(width - self.window_x),
                        height: 1,
                    },
                )?);
            }
            if !self.query.as_mut().unwrap().advance(document, budget)? {
                return Ok(MathTextStep::BudgetExhausted);
            }
            if self.view.is_none() {
                self.view = self.query.as_ref().unwrap().result();
            }
            self.shared.maintenance(budget);
            if !budget.charge(WorkLimit {
                queries: 1,
                objects: 1,
                ..Default::default()
            }) {
                return Ok(MathTextStep::BudgetExhausted);
            }
            let view = self.view.as_ref().unwrap();
            if let Some((key, cell)) = view.cells.next_after(self.after)
                && cell.x < self.window_end
            {
                if cell.x > self.column {
                    self.column += 1;
                    return Ok(MathTextStep::Space(self.meta.content));
                }
                self.after = Some(key);
                self.column = self.column.max(cell.x + usize::from(cell.width));
                return Ok(MathTextStep::Cell {
                    cell: cell.clone(),
                    focus: view.focus_cell(cell),
                });
            }
            self.query = None;
            self.view = None;
            self.after = None;
            if self.window_end < width {
                self.window_x = self.window_end;
            } else {
                self.row += 1;
                self.column = 0;
                self.window_x = 0;
                return Ok(MathTextStep::Newline(self.meta.content));
            }
        }
    }
}
