//! The production paragraph line-placement state, shared with paid prefix
//! preparation. This owns no source, glyph buffer, parser or view.
#[derive(Clone, Copy, Debug)]
pub(crate) struct FlowLine {
    pub top: usize,
    pub ascent: usize,
    pub descent: usize,
}
#[derive(Clone, Copy, Debug)]
pub(crate) struct FlowStep {
    pub flush: Option<FlowLine>,
    pub column: Option<usize>,
}
#[derive(Clone, Copy, Debug)]
pub(crate) struct FlowCursor {
    pub capacity: usize,
    continued: usize,
    pub x: usize,
    pub y: usize,
    pub ascent: usize,
    pub descent: usize,
    placed: bool,
}
impl FlowCursor {
    pub fn new(first: usize, continued: usize) -> Self {
        Self {
            capacity: first,
            continued,
            x: 0,
            y: 0,
            ascent: 0,
            descent: 0,
            placed: false,
        }
    }
    pub fn height(self) -> usize {
        self.y
            + if self.placed {
                self.ascent + self.descent + 1
            } else {
                0
            }
    }
    pub fn push(
        &mut self,
        width: usize,
        ascent: usize,
        descent: usize,
        native: bool,
    ) -> Result<FlowStep, super::sink::SinkError> {
        let intrinsic = width;
        let mut width = if native {
            width.min(self.capacity)
        } else {
            width
        };
        if !native && width > self.capacity {
            return Ok(FlowStep {
                flush: None,
                column: None,
            });
        }
        let flush = if self.placed && self.x.saturating_add(width) > self.capacity {
            let line = FlowLine {
                top: self.y,
                ascent: self.ascent,
                descent: self.descent,
            };
            self.y = self
                .y
                .checked_add(self.ascent + self.descent + 1)
                .ok_or(super::sink::SinkError::RowOverflow)?;
            self.capacity = self.continued;
            self.x = 0;
            self.ascent = 0;
            self.descent = 0;
            self.placed = false;
            if native {
                width = intrinsic.min(self.capacity);
            }
            Some(line)
        } else {
            None
        };
        if !native && width > self.capacity {
            return Ok(FlowStep {
                flush,
                column: None,
            });
        }
        let column = self.x;
        self.x += width;
        self.ascent = self.ascent.max(ascent);
        self.descent = self.descent.max(descent);
        self.placed = true;
        Ok(FlowStep {
            flush,
            column: Some(column),
        })
    }
}
