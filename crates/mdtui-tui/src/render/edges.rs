use mdtui3::{Budget, WorkLimit};
use unicode_segmentation::{GraphemeCursor, GraphemeIncomplete};

pub(crate) const MAX_EDGE_OVERSCAN: usize = 4096;

/// Only Unicode boundary context is inspected here: no layout or source copy.
/// GraphemeCursor retains RI/ZWJ/InCB lookbehind state across budget pauses.
pub(crate) struct EdgeProbe {
    cursor: GraphemeCursor,
    requested: usize,
    before: bool,
    moving: bool,
    context: Option<usize>,
    pub bytes: usize,
    pub queries: usize,
    pub context_limited: bool,
    max_context: usize,
}

impl EdgeProbe {
    pub fn new(at: usize, len: usize, before: bool) -> Self {
        Self {
            cursor: GraphemeCursor::new(at, len, true),
            requested: at,
            before,
            moving: false,
            context: None,
            bytes: 0,
            queries: 0,
            context_limited: false,
            max_context: MAX_EDGE_OVERSCAN,
        }
    }

    pub fn with_context_limit(mut self, bytes: usize) -> Self {
        self.max_context = bytes;
        self
    }

    /// `true` means the edge cluster exceeds the bounded payload overscan and
    /// must be omitted rather than rendered as an independent partial cluster.
    pub fn advance(&mut self, source: &str, budget: &mut Budget) -> Option<(usize, bool)> {
        loop {
            let at = self.cursor.cur_cursor();
            if self
                .context
                .is_some_and(|offset| self.requested.abs_diff(offset) > self.max_context)
            {
                self.context_limited = true;
                return Some((self.requested, true));
            }
            if self.context.is_none() && (at == 0 || at == source.len()) {
                return Some((at, false));
            }
            if self.moving && at.abs_diff(self.requested) > MAX_EDGE_OVERSCAN {
                return Some((self.requested, true));
            }
            let available = budget.remaining().bytes.min(mdtui3::TILE_BYTES);
            let backwards = self.context.is_some() || (self.moving && self.before);
            let (lo, hi) = if backwards {
                let hi = self.context.unwrap_or(at);
                let mut lo = hi.saturating_sub(available);
                while lo < hi && !source.is_char_boundary(lo) {
                    lo += 1;
                }
                (lo, hi)
            } else {
                let mut hi = at.saturating_add(available).min(source.len());
                while hi > at && !source.is_char_boundary(hi) {
                    hi -= 1;
                }
                (at, hi)
            };
            if lo == hi
                || !budget.charge(WorkLimit {
                    queries: 1,
                    bytes: hi - lo,
                    ..WorkLimit::default()
                })
            {
                return None;
            }
            self.bytes += hi - lo;
            self.queries += 1;
            let chunk = &source[lo..hi];
            if self.context.take().is_some() {
                self.cursor.provide_context(chunk, lo);
                continue;
            }
            let result = if !self.moving {
                match self.cursor.is_boundary(chunk, lo) {
                    Ok(true) => return Some((self.requested, false)),
                    Ok(false) => {
                        self.moving = true;
                        continue;
                    }
                    Err(error) => Err(error),
                }
            } else if self.before {
                self.cursor.prev_boundary(chunk, lo)
            } else {
                self.cursor.next_boundary(chunk, lo)
            };
            match result {
                Ok(Some(boundary)) => return Some((boundary, false)),
                Ok(None) => return Some((if self.before { 0 } else { source.len() }, false)),
                Err(GraphemeIncomplete::PreContext(at)) => self.context = Some(at),
                Err(GraphemeIncomplete::PrevChunk | GraphemeIncomplete::NextChunk) => {}
                Err(GraphemeIncomplete::InvalidOffset) => {
                    unreachable!("UTF-8 aligned cursor chunk")
                }
            }
        }
    }
}
