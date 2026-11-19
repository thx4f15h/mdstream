//! Paid geometry from actual native BandLine records. No URL discovery or
//! selected-target semantics live here; the current native frame owns those.
use super::{
    layout::{Band, BandLine, ImageKey, MAX_OBJECTS, Surface},
    protocol::Placement,
    session::Session,
    worker::{CellPixels, Geometry},
};
use mdtui3::{Budget, WorkLimit};
use std::collections::HashMap;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SceneError {
    WrongSurface,
    InvalidGeometry,
    ChangedObject,
    Capacity,
    Replaced,
    Budget,
}
#[derive(Clone, Copy)]
struct Rect {
    band: Band,
    column: u16,
    top: u16,
    start: u32,
    width: u16,
    rows: u16,
}
pub struct Builder {
    surface: Surface,
    cell: CellPixels,
    x: u16,
    width: u16,
    rows: u16,
    index: HashMap<ImageKey, usize>,
    rects: Vec<Rect>,
    required: [Option<super::cache::ResourceId>; 64],
}
fn pay(b: &mut Budget) -> bool {
    b.charge(WorkLimit {
        queries: 1,
        objects: 1,
        ..Default::default()
    })
}
impl Builder {
    pub fn new(
        surface: Surface,
        cell: CellPixels,
        x: u16,
        width: u16,
        rows: u16,
    ) -> Result<Self, SceneError> {
        if width == 0
            || rows == 0
            || width > 1000
            || rows > 1000
            || x.checked_add(width).is_none()
            || cell.width == 0
            || cell.height == 0
            || cell.width > 4096
            || cell.height > 4096
        {
            return Err(SceneError::InvalidGeometry);
        }
        Ok(Self {
            surface,
            cell,
            x,
            width,
            rows,
            index: HashMap::with_capacity(MAX_OBJECTS),
            rects: Vec::with_capacity(MAX_OBJECTS),
            required: [None; 64],
        })
    }
    /// false consumes no record; retry that fixed Copy input when credit is
    /// available. A changed Band cannot be appended to an earlier continuation.
    pub fn push(&mut self, row: u16, line: BandLine, b: &mut Budget) -> Result<bool, SceneError> {
        if !pay(b) {
            return Ok(false);
        }
        if line.band.key.surface != self.surface {
            return Err(SceneError::WrongSurface);
        }
        if row >= self.rows
            || line.width == 0
            || line
                .column
                .checked_add(line.width)
                .is_none_or(|n| n > self.width)
            || line.row >= line.band.geometry.rows
            || u32::from(line.width) > line.band.geometry.columns
        {
            return Err(SceneError::InvalidGeometry);
        }
        let slot = usize::from(line.band.resource.slot);
        if slot >= 64 {
            return Err(SceneError::InvalidGeometry);
        }
        if self.required[slot].is_some_and(|id| id != line.band.resource) {
            return Err(SceneError::ChangedObject);
        }
        if let Some(&at) = self.index.get(&line.band.key) {
            let r = &mut self.rects[at];
            if r.band != line.band {
                return Err(SceneError::ChangedObject);
            }
            if r.column != line.column
                || r.width != line.width
                || u32::from(r.top) + u32::from(r.rows) != u32::from(row)
                || r.start + u32::from(r.rows) != line.row
            {
                return Err(SceneError::InvalidGeometry);
            }
            r.rows += 1;
        } else {
            if self.rects.len() == MAX_OBJECTS {
                return Err(SceneError::Capacity);
            }
            self.index.insert(line.band.key, self.rects.len());
            self.rects.push(Rect {
                band: line.band,
                column: line.column,
                top: row,
                start: line.row,
                width: line.width,
                rows: 1,
            });
        }
        self.required[slot] = Some(line.band.resource);
        Ok(true)
    }
    pub fn len(&self) -> usize {
        self.rects.len()
    }
    pub fn is_empty(&self) -> bool {
        self.rects.is_empty()
    }
    pub fn finish(self) -> Scene {
        Scene {
            cell: self.cell,
            x: self.x,
            rects: self.rects,
            required: self.required,
            retire_at: 0,
            at: 0,
            started: None,
            done: false,
            cancelling: false,
        }
    }
    /// Conservative bucket upper bound; this is owned capacity, not process RSS.
    pub fn capacity_bound(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.rects.capacity() * std::mem::size_of::<Rect>()
            + (self.index.capacity() * 8 / 7 + 1).next_power_of_two()
                * (std::mem::size_of::<(ImageKey, usize)>() + 1)
    }
}
pub struct Scene {
    cell: CellPixels,
    x: u16,
    rects: Vec<Rect>,
    required: [Option<super::cache::ResourceId>; 64],
    retire_at: usize,
    at: usize,
    started: Option<u64>,
    done: bool,
    cancelling: bool,
}
impl Scene {
    pub fn len(&self) -> usize {
        self.rects.len()
    }
    pub fn is_empty(&self) -> bool {
        self.rects.is_empty()
    }
    /// All rectangles and disappearance controls yield, including an empty
    /// frame. The writer cannot publish a half-replaced placement scene.
    pub fn apply(&mut self, session: &mut Session, b: &mut Budget) -> Result<bool, SceneError> {
        if self.cancelling {
            return Ok(false);
        }
        if self.done {
            return Ok(true);
        }
        if self.started.is_some_and(|id| !session.owns_scene(id)) {
            return Err(SceneError::Replaced);
        }
        if self.started.is_none() && session.clearing_scene() {
            return Ok(false);
        }
        if !self.rects.is_empty()
            && session.capability.support != super::protocol::Support::Ready(self.cell)
        {
            return Ok(false);
        }
        if self.started.is_none() {
            if !pay(b) {
                return Ok(false);
            }
            self.started = Some(session.pause_scene().ok_or(SceneError::Replaced)?);
        }
        while self.at < MAX_OBJECTS {
            let accepted = if let Some(r) = self.rects.get(self.at) {
                let p = Placement {
                    image: 0,
                    placement: 0,
                    row: r.top,
                    column: self.x + r.column,
                    cells: Geometry {
                        columns: u32::from(r.width),
                        rows: u32::from(r.rows),
                    },
                    crop_x: 0,
                    crop_y: r.start * self.cell.height,
                    crop: Geometry {
                        columns: u32::from(r.width) * self.cell.width,
                        rows: u32::from(r.rows) * self.cell.height,
                    },
                };
                session.place(self.at, r.band.resource, p, b)
            } else {
                session.hide(self.at, b)
            };
            if !accepted {
                return Ok(false);
            }
            self.at += 1;
        }
        while self.retire_at < 64 {
            if !session.retire_unneeded(self.retire_at, self.required[self.retire_at], b) {
                return Ok(false);
            }
            self.retire_at += 1;
        }
        if !pay(b) {
            return Ok(false);
        }
        let finished = session.finish_scene(self.started.unwrap());
        debug_assert!(finished);
        self.done = true;
        Ok(true)
    }
    /// Cancel the whole provisional frame, including old slots not reached
    /// yet. A replaced continuation cannot clear its successor's scene.
    pub fn cancel(&mut self, session: &mut Session, b: &mut Budget) -> Result<bool, SceneError> {
        let Some(id) = self.started else {
            return Ok(true);
        };
        if !session.owns_scene(id) {
            return Err(SceneError::Replaced);
        }
        if !self.cancelling {
            if !pay(b) {
                return Ok(false);
            }
            self.cancelling = true;
            self.at = MAX_OBJECTS;
            self.retire_at = 0;
        }
        while self.at > 0 {
            if !session.hide(self.at - 1, b) {
                return Ok(false);
            }
            self.at -= 1;
        }
        while self.retire_at < 64 {
            if !session.retire_unneeded(self.retire_at, None, b) {
                return Ok(false);
            }
            self.retire_at += 1;
        }
        if !pay(b) {
            return Ok(false);
        }
        let finished = session.finish_scene(id);
        debug_assert!(finished);
        self.started = None;
        self.done = true;
        Ok(true)
    }
    pub fn retire(&mut self, b: &mut Budget) -> bool {
        while !self.rects.is_empty() {
            if !pay(b) {
                return false;
            }
            self.rects.pop();
        }
        true
    }
    pub fn capacity_bytes(&self) -> usize {
        std::mem::size_of::<Self>() + self.rects.capacity() * std::mem::size_of::<Rect>()
    }
}

/// Immutable actual native frame input. The caller moves its existing frame
/// into Rc; capture never clones its text/cells or queries source gaps/URLs.
/// A current Root/with_renderer Note owner must validate before publication.
pub struct Capture {
    frame: Option<std::rc::Rc<crate::render::viewport::ViewportFrame>>,
    builder: Option<Builder>,
    row: usize,
    band: usize,
    limit: usize,
    pending: Option<BandLine>,
    ready: bool,
    retiring: bool,
    pub work: CaptureWork,
}
#[derive(Clone, Copy, Default, Debug)]
pub struct CaptureWork {
    pub row_queries: u64,
    pub record_queries: u64,
    pub records: u64,
    pub retired_cells: u64,
    pub retired_records: u64,
    pub retired_rows: u64,
}
impl Builder {
    fn retire(&mut self, b: &mut Budget) -> bool {
        while let Some(rect) = self.rects.last() {
            if !pay(b) {
                return false;
            }
            let key = rect.band.key;
            self.index.remove(&key);
            self.rects.pop();
        }
        true
    }
}
impl Capture {
    pub fn new(
        frame: std::rc::Rc<crate::render::viewport::ViewportFrame>,
        surface: Surface,
        cell: CellPixels,
        x: u16,
        width: u16,
        rows: u16,
        b: &mut Budget,
    ) -> Result<
        Self,
        (
            SceneError,
            std::rc::Rc<crate::render::viewport::ViewportFrame>,
        ),
    > {
        if !pay(b) {
            return Err((SceneError::Budget, frame));
        }
        if matches!(surface,Surface::Root(id) if frame.document!=id) {
            return Err((SceneError::WrongSurface, frame));
        }
        let builder = match Builder::new(surface, cell, x, width, rows) {
            Ok(v) => v,
            Err(e) => return Err((e, frame)),
        };
        let limit = frame
            .lines
            .len()
            .min(usize::from(rows))
            .min(usize::from(frame.rows));
        Ok(Self {
            frame: Some(frame),
            builder: Some(builder),
            row: 0,
            band: 0,
            limit,
            pending: None,
            ready: false,
            retiring: false,
            work: CaptureWork::default(),
        })
    }
    pub fn advance(&mut self, b: &mut Budget) -> Result<bool, SceneError> {
        if self.retiring {
            return Err(SceneError::Replaced);
        }
        if self.ready {
            return Ok(true);
        }
        loop {
            if let Some(line) = self.pending {
                if !self
                    .builder
                    .as_mut()
                    .unwrap()
                    .push(self.row as u16, line, b)?
                {
                    return Ok(false);
                }
                self.pending = None;
                self.band += 1;
                self.work.records += 1;
                continue;
            }
            if !pay(b) {
                return Ok(false);
            }
            if self.row == self.limit {
                self.ready = true;
                return Ok(true);
            }
            self.work.row_queries += 1;
            let row = &self.frame.as_ref().unwrap().lines[self.row];
            if self.band == row.images.len() {
                self.row += 1;
                self.band = 0;
                continue;
            }
            self.work.record_queries += 1;
            self.pending = Some(row.images[self.band]);
        }
    }
    pub fn take(&mut self) -> Option<Scene> {
        if self.retiring || !self.ready {
            return None;
        }
        self.builder.take().map(Builder::finish)
    }
    pub fn retire(&mut self, b: &mut Budget) -> bool {
        if !self.retiring {
            if !pay(b) {
                return false;
            }
            self.retiring = true;
        }
        if let Some(builder) = self.builder.as_mut() {
            if !builder.retire(b) {
                return false;
            }
            if !pay(b) {
                return false;
            }
            self.builder = None;
        }
        if let Some(pin) = self.frame.as_mut() {
            if let Some(frame) = std::rc::Rc::get_mut(pin) {
                while let Some(row) = frame.lines.last_mut() {
                    while !row.cells.is_empty() {
                        if !pay(b) {
                            return false;
                        }
                        row.cells.pop();
                        self.work.retired_cells += 1;
                    }
                    while !row.images.is_empty() {
                        if !pay(b) {
                            return false;
                        }
                        row.images.pop();
                        self.work.retired_records += 1;
                    }
                    if !pay(b) {
                        return false;
                    }
                    frame.lines.pop();
                    self.work.retired_rows += 1;
                }
            }
            if !pay(b) {
                return false;
            }
            self.frame = None;
        }
        self.pending = None;
        true
    }
    /// Frame capacities are a pin subset when another native owner still
    /// holds Rc. Bytes behind shared TextPayloads are owned by native Shape,
    /// not duplicated here. This diagnostic visits only this visible frame.
    pub fn frame_handle_capacity(&self) -> usize {
        self.frame.as_ref().map_or(0, |f| {
            std::mem::size_of::<crate::render::viewport::ViewportFrame>()
                + f.lines.capacity() * std::mem::size_of::<crate::render::sink::SinkLine>()
                + f.lines
                    .iter()
                    .map(|r| {
                        r.cells.capacity() * std::mem::size_of::<crate::render::sink::SinkCell>()
                            + r.images.capacity() * std::mem::size_of::<BandLine>()
                    })
                    .sum::<usize>()
        })
    }
    pub fn capacity_bound(&self) -> usize {
        std::mem::size_of::<Self>() + self.builder.as_ref().map_or(0, Builder::capacity_bound)
    }
}
