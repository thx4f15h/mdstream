//! Paid image-row preparation over the already selected native row window.
//! No URI, decoder, protocol, alt text collection or source-gap scan lives here.
use super::*;
use crate::image::layout::{Band, BandLine, ImageFocus, ImageKey};
use mdtui3::{Budget, WorkLimit};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    sync::Arc,
};
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum Row {
    Block(BlockId, usize),
    Pending(usize),
}
#[derive(Clone, Copy)]
struct At {
    band: Band,
    column: u16,
    width: u16,
    top: u32,
}
pub(super) struct Group {
    block: Option<BlockId>,
    origin: u64,
    top: u64,
    added: u64,
    pub height: u32,
    borders: Vec<u16>,
    places: Vec<At>,
    pub start: u32,
    pub end: u32,
    pub prepared: Vec<SinkLine>,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Index,
    Scan,
    Resolve,
    Shift,
    Window,
    Rows,
    Done,
}
pub(super) struct Preparation {
    input: Option<Arc<Vec<Band>>>,
    input_at: usize,
    index: BTreeMap<(usize, LinkId), Band>,
    seen: HashSet<ImageKey>,
    phase: Phase,
    tile: usize,
    item: usize,
    row: Option<Row>,
    cell: usize,
    column: u16,
    span: Option<Span>,
    span_column: u16,
    after: Option<(usize, LinkId)>,
    candidates: Vec<(Band, u16)>,
    borders: Vec<u16>,
    resolve: usize,
    places: Vec<At>,
    stacks: BTreeMap<u16, u32>,
    height: u32,
    base_row: u64,
    added: u64,
    rows: HashMap<Row, usize>,
    positions: HashMap<ImageKey, (usize, usize)>,
    pub groups: Vec<Group>,
    math_after: Option<mdtui3::MathObjectId>,
    window_at: usize,
    prefix: u64,
    total: u64,
    window: Option<(u64, u64)>,
    group: usize,
    output_row: u32,
    output: Option<SinkLine>,
    output_column: u16,
    output_place: usize,
    output_hit_band: usize,
    output_hit_column: u16,
}
impl Default for Preparation {
    fn default() -> Self {
        Self::new(None)
    }
}
fn pay(b: &mut Budget, bytes: usize) -> bool {
    b.charge(WorkLimit {
        bytes,
        queries: 1,
        objects: 1,
        ..Default::default()
    })
}
impl Preparation {
    pub fn new(input: Option<Arc<Vec<Band>>>) -> Self {
        let active = input.as_ref().is_some_and(|v| !v.is_empty());
        Self {
            input,
            input_at: 0,
            index: BTreeMap::new(),
            seen: HashSet::new(),
            phase: if active { Phase::Index } else { Phase::Done },
            tile: 0,
            item: 0,
            row: None,
            cell: 0,
            column: 0,
            span: None,
            span_column: 0,
            after: None,
            candidates: Vec::new(),
            borders: Vec::new(),
            resolve: 0,
            places: Vec::new(),
            stacks: BTreeMap::new(),
            height: 0,
            base_row: 0,
            added: 0,
            rows: HashMap::new(),
            positions: HashMap::new(),
            groups: Vec::new(),
            math_after: None,
            window_at: 0,
            prefix: 0,
            total: 0,
            window: None,
            group: 0,
            output_row: 0,
            output: None,
            output_column: 0,
            output_place: 0,
            output_hit_band: 0,
            output_hit_column: 0,
        }
    }
    pub fn retire_line(line: &mut SinkLine, b: &mut Budget) -> bool {
        while !line.cells.is_empty() {
            if !pay(b, 0) {
                return false;
            }
            line.cells.pop();
        }
        while !line.images.is_empty() {
            if !pay(b, 0) {
                return false;
            }
            line.images.pop();
        }
        true
    }
    pub fn retire(&mut self, b: &mut Budget) -> bool {
        if let Some(line) = self.output.as_mut() {
            if !Self::retire_line(line, b) {
                return false;
            }
            if !pay(b, 0) {
                return false;
            }
            self.output = None;
        }
        while let Some(g) = self.groups.last_mut() {
            while let Some(line) = g.prepared.last_mut() {
                if !Self::retire_line(line, b) {
                    return false;
                }
                if !pay(b, 0) {
                    return false;
                }
                g.prepared.pop();
            }
            while !g.places.is_empty() {
                if !pay(b, 0) {
                    return false;
                }
                g.places.pop();
            }
            while !g.borders.is_empty() {
                if !pay(b, 0) {
                    return false;
                }
                g.borders.pop();
            }
            if !pay(b, 0) {
                return false;
            }
            self.groups.pop();
        }
        while !self.candidates.is_empty() {
            if !pay(b, 0) {
                return false;
            }
            self.candidates.pop();
        }
        while !self.places.is_empty() {
            if !pay(b, 0) {
                return false;
            }
            self.places.pop();
        }
        while !self.borders.is_empty() {
            if !pay(b, 0) {
                return false;
            }
            self.borders.pop();
        }
        while !self.index.is_empty() {
            if !pay(b, 0) {
                return false;
            }
            self.index.pop_first();
        }
        while !self.stacks.is_empty() {
            if !pay(b, 0) {
                return false;
            }
            self.stacks.pop_first();
        }
        while let Some(k) = self.rows.keys().next().copied() {
            if !pay(b, 0) {
                return false;
            }
            self.rows.remove(&k);
        }
        while let Some(k) = self.positions.keys().next().copied() {
            if !pay(b, 0) {
                return false;
            }
            self.positions.remove(&k);
        }
        while let Some(k) = self.seen.iter().next().copied() {
            if !pay(b, 0) {
                return false;
            }
            self.seen.remove(&k);
        }
        if let Some(input) = self.input.as_mut() {
            if let Some(v) = Arc::get_mut(input) {
                while !v.is_empty() {
                    if !pay(b, 0) {
                        return false;
                    }
                    v.pop();
                }
            }
            if !pay(b, 0) {
                return false;
            }
            self.input = None;
        }
        true
    }
    pub fn group_for(&self, row: Row) -> Option<usize> {
        self.rows.get(&row).copied()
    }
    pub fn skip_band(&self, row: u64) -> Option<u64> {
        let at = self.groups.partition_point(|g| g.top <= row);
        let group = self.groups.get(at.checked_sub(1)?)?;
        let end = group.top + u64::from(group.height);
        (row < end).then_some(end)
    }
    pub fn image_row(&self, focus: ImageFocus) -> Option<u64> {
        let (g, p) = *self.positions.get(&focus.key)?;
        let group = self.groups.get(g)?;
        let place = group.places.get(p)?;
        Some(group.top + u64::from(place.top + focus.row.min(place.band.geometry.rows - 1)))
    }
    fn line(s: &TileLayoutSink, row: Row) -> &SinkLine {
        match row {
            Row::Pending(i) => &s.pending_rows[i].line,
            Row::Block(id, i) => match &s.blocks[&id].content {
                PreparedContent::Rows(rows) | PreparedContent::MathRows { rows, .. } => {
                    &rows[i].line
                }
                _ => unreachable!("band rows are prepared as native Rows"),
            },
        }
    }
    fn next_row(&mut self) {
        self.base_row += 1;
        self.item += 1;
        self.row = None;
        self.cell = 0;
        self.column = 0;
        self.span = None;
        self.after = None;
        self.candidates.clear();
        self.borders.clear();
        self.places.clear();
        self.stacks.clear();
        self.height = 0;
        self.resolve = 0;
    }
    pub fn advance(
        &mut self,
        s: &mut TileLayoutSink,
        view: Option<super::super::table_window::TableView>,
        limit: u64,
        focus: Option<ImageFocus>,
        b: &mut Budget,
    ) -> Result<bool, SinkError> {
        loop {
            match self.phase {
                Phase::Done => return Ok(true),
                Phase::Index => {
                    let input = self.input.as_ref().unwrap();
                    if self.input_at == input.len() {
                        self.phase = Phase::Scan;
                        continue;
                    }
                    if !pay(b, 0) {
                        return Ok(false);
                    }
                    let band = input[self.input_at];
                    self.input_at += 1;
                    self.index.insert((band.anchor, band.key.object), band);
                }
                Phase::Scan => {
                    if self.tile == s.work.len() {
                        self.phase = Phase::Shift;
                        continue;
                    }
                    if self.item == s.work[self.tile].len() {
                        if !pay(b, 0) {
                            return Ok(false);
                        }
                        self.tile += 1;
                        self.item = 0;
                        continue;
                    }
                    if self.row.is_none() {
                        if !pay(b, 0) {
                            return Ok(false);
                        }
                        let kind = s.work[self.tile][self.item].kind;
                        self.row = match kind {
                            WorkKind::Row(id, i) => Some(Row::Block(id, i)),
                            WorkKind::PendingRow(i) => Some(Row::Pending(i)),
                            _ => None,
                        };
                        if self.row.is_none() {
                            self.base_row += match kind {
                                WorkKind::Event(i) => match &s.events[i].kind {
                                    PlanEventKind::Spacing { blank_lines } => {
                                        u64::from(*blank_lines)
                                    }
                                    PlanEventKind::FoldLabel { summary, .. } => {
                                        1 + summary.len() as u64
                                    }
                                    _ => 1,
                                },
                                WorkKind::SkippedRows(n) => n,
                                WorkKind::MathObject(_) | WorkKind::MathOrigins(_) => 0,
                                _ => {
                                    return Err(SinkError::Render(
                                        "streamed row escaped image preparation".into(),
                                    ));
                                }
                            };
                            self.item += 1;
                            continue;
                        }
                    }
                    if let Some(span) = self.span {
                        if !pay(b, 0) {
                            return Ok(false);
                        }
                        use std::ops::Bound::{Excluded, Included, Unbounded};
                        let lower = self
                            .after
                            .map_or(Included((span.start, LinkId(0))), Excluded);
                        let next = self
                            .index
                            .range((lower, Unbounded))
                            .next()
                            .filter(|(k, _)| k.0 < span.end)
                            .map(|(k, v)| (*k, *v));
                        if let Some((key, band)) = next {
                            self.after = Some(key);
                            if self.seen.insert(band.key) {
                                self.candidates.push((band, self.span_column));
                            }
                        } else {
                            self.span = None;
                            self.after = None
                        }
                        continue;
                    }
                    let line = Self::line(s, self.row.unwrap());
                    if self.cell == line.cells.len() {
                        if self.candidates.is_empty() {
                            self.next_row()
                        } else {
                            self.phase = Phase::Resolve
                        }
                        continue;
                    }
                    if !pay(b, 0) {
                        return Ok(false);
                    }
                    let cell = &line.cells[self.cell];
                    if cell.style == StyleRole::TABLE_BORDER {
                        if !b.charge(WorkLimit {
                            bytes: 3,
                            ..Default::default()
                        }) {
                            return Ok(false);
                        }
                        if cell.text.contiguous() == Some("│") {
                            self.borders.push(self.column)
                        }
                    }
                    self.span = cell.source.filter(|p| !p.is_empty());
                    self.span_column = self.column;
                    self.column = self.column.saturating_add(u16::from(cell.width));
                    self.cell += 1;
                }
                Phase::Resolve => {
                    if self.resolve < self.candidates.len() {
                        if !pay(b, 0) {
                            return Ok(false);
                        }
                        let (band, x) = self.candidates[self.resolve];
                        self.resolve += 1;
                        let table = self.borders.len() >= 2;
                        let left = if table {
                            self.borders
                                .iter()
                                .copied()
                                .take_while(|v| *v < x)
                                .last()
                                .map_or(0, |v| v + 1)
                        } else {
                            0
                        };
                        let right = if table {
                            self.borders
                                .iter()
                                .copied()
                                .find(|v| *v > x)
                                .unwrap_or(s.width)
                        } else {
                            s.width
                        };
                        let width = band
                            .geometry
                            .columns
                            .min(u32::from(right.saturating_sub(left)))
                            as u16;
                        if width == 0 {
                            continue;
                        }
                        let top = if table {
                            *self.stacks.get(&left).unwrap_or(&0)
                        } else {
                            self.height
                        };
                        let end = top.saturating_add(band.geometry.rows);
                        self.stacks.insert(left, end);
                        self.height = self.height.max(end);
                        self.positions
                            .insert(band.key, (self.groups.len(), self.places.len()));
                        self.places.push(At {
                            band,
                            column: left,
                            width,
                            top,
                        });
                        continue;
                    }
                    if !pay(b, 0) {
                        return Ok(false);
                    }
                    if self.height > 0 {
                        let origin = self.base_row + 1;
                        self.added += u64::from(self.height);
                        s.tile_rows[self.tile] += u64::from(self.height);
                        self.rows.insert(self.row.unwrap(), self.groups.len());
                        self.groups.push(Group {
                            block: Self::line(s, self.row.unwrap())
                                .source
                                .and_then(|s| s.block),
                            origin,
                            top: origin + self.added - u64::from(self.height),
                            added: self.added,
                            height: self.height,
                            borders: std::mem::take(&mut self.borders),
                            places: std::mem::take(&mut self.places),
                            start: 0,
                            end: 0,
                            prepared: Vec::new(),
                        });
                    }
                    self.next_row();
                    self.phase = Phase::Scan;
                }
                Phase::Shift => {
                    if !pay(b, 0) {
                        return Ok(false);
                    }
                    use std::ops::Bound::{Excluded, Unbounded};
                    let next = s
                        .math_placements
                        .range((self.math_after.map_or(Unbounded, Excluded), Unbounded))
                        .next()
                        .map(|(id, p)| (*id, p.top));
                    if let Some((id, top)) = next {
                        self.math_after = Some(id);
                        let at = self.groups.partition_point(|g| g.origin <= top);
                        let add = at.checked_sub(1).map_or(0, |i| self.groups[i].added);
                        s.math_placements.get_mut(&id).unwrap().top += add;
                    } else {
                        self.phase = Phase::Window
                    }
                }
                Phase::Window => {
                    if self.window_at < s.tile_rows.len() {
                        if !pay(b, 0) {
                            return Ok(false);
                        }
                        let i = self.window_at;
                        self.window_at += 1;
                        self.total += s.tile_rows[i];
                        if view.is_some_and(|v| s.tiles[i] < v.anchor.tile) {
                            self.prefix += s.tile_rows[i]
                        }
                        continue;
                    }
                    if !pay(b, 0) {
                        return Ok(false);
                    }
                    let rows = view.map_or(limit, |v| v.rows as u64).min(4096);
                    let top = match view {
                        Some(v) if v.tail.is_some() => self.total.saturating_sub(rows),
                        Some(v) => self.prefix + v.anchor.offset,
                        None => 0,
                    };
                    let top = focus.and_then(|f| self.image_row(f)).unwrap_or(top);
                    self.window = Some((top, top.saturating_add(rows)));
                    self.phase = Phase::Rows;
                }
                Phase::Rows => {
                    if self.group == self.groups.len() {
                        self.phase = Phase::Done;
                        continue;
                    }
                    let g = &mut self.groups[self.group];
                    let (top, end) = self.window.unwrap();
                    if self.output.is_none() {
                        if !pay(b, 0) {
                            return Ok(false);
                        }
                        if self.output_row == 0 {
                            g.start = top.saturating_sub(g.top).min(u64::from(g.height)) as u32;
                            g.end = end.saturating_sub(g.top).min(u64::from(g.height)) as u32;
                            self.output_row = g.start;
                        }
                        if self.output_row >= g.end {
                            self.group += 1;
                            self.output_row = 0;
                            continue;
                        }
                        self.output = Some(SinkLine::default());
                        self.output_column = 0;
                        self.output_place = 0;
                        self.output_hit_band = 0;
                        self.output_hit_column = 0;
                    }
                    if self.output_column < s.width {
                        let vertical = g.borders.binary_search(&self.output_column).is_ok();
                        if !pay(b, if vertical { 3 } else { 1 }) {
                            return Ok(false);
                        }
                        self.output.as_mut().unwrap().cells.push(SinkCell {
                            text: if vertical { "│".into() } else { " ".into() },
                            style: if vertical {
                                StyleRole::TABLE_BORDER
                            } else {
                                StyleRole::DEFAULT
                            },
                            width: 1,
                            source: None,
                            link: None,
                        });
                        self.output_column += 1;
                        continue;
                    }
                    if self.output_place < g.places.len() {
                        if !pay(b, 0) {
                            return Ok(false);
                        }
                        let p = g.places[self.output_place];
                        self.output_place += 1;
                        if self.output_row >= p.top
                            && self.output_row < p.top + p.band.geometry.rows
                        {
                            let line = self.output.as_mut().unwrap();
                            line.source.get_or_insert(LineSource {
                                source: p.band.source,
                                block: g.block,
                            });
                            line.images.push(BandLine {
                                band: p.band,
                                column: p.column,
                                row: self.output_row - p.top,
                                width: p.width,
                            });
                        }
                        continue;
                    }
                    if let Some(image) = self
                        .output
                        .as_ref()
                        .unwrap()
                        .images
                        .get(self.output_hit_band)
                        .copied()
                    {
                        if self.output_hit_column >= image.width {
                            self.output_hit_band += 1;
                            self.output_hit_column = 0;
                            continue;
                        }
                        if !pay(b, 0) {
                            return Ok(false);
                        }
                        let column = image.column + self.output_hit_column;
                        self.output_hit_column += 1;
                        if let Some(cell) = self
                            .output
                            .as_mut()
                            .unwrap()
                            .cells
                            .get_mut(usize::from(column))
                            && cell.style != StyleRole::TABLE_BORDER
                        {
                            // This is the native image object's exact source
                            // envelope, not a glyph mapping for the blank. It
                            // also lets the actual note hit path qualify it
                            // with its BodyKey. Borders and padding outside
                            // the owned rectangle keep source/link = None.
                            cell.link = Some(image.band.hit);
                            cell.source = Some(image.band.source);
                        }
                        continue;
                    }
                    if !pay(b, 0) {
                        return Ok(false);
                    }
                    g.prepared.push(self.output.take().unwrap());
                    self.output_row += 1;
                    if self.output_row == g.end {
                        self.group += 1;
                        self.output_row = 0;
                    }
                }
            }
        }
    }
}

// The core end_tile callback has no Budget parameter. It only records finite
// insertion descriptors; moving prepared row handles happens here afterwards.
pub(super) struct Insertion {
    pub at: usize,
    pub group: usize,
    pub offset: u64,
}
pub(super) struct Adoption {
    normal: std::vec::IntoIter<SinkLine>,
    offsets: std::vec::IntoIter<u64>,
    at: usize,
    pending: std::collections::VecDeque<Insertion>,
    active: Option<(std::vec::IntoIter<SinkLine>, u64)>,
}
impl Adoption {
    pub fn new(
        lines: Vec<SinkLine>,
        offsets: Vec<u64>,
        pending: std::collections::VecDeque<Insertion>,
    ) -> Self {
        Self {
            normal: lines.into_iter(),
            offsets: offsets.into_iter(),
            at: 0,
            pending,
            active: None,
        }
    }
    pub fn advance(
        &mut self,
        images: &mut Preparation,
        lines: &mut Vec<SinkLine>,
        offsets: &mut Vec<u64>,
        stats: &mut super::LayoutSinkStats,
        b: &mut Budget,
    ) -> bool {
        loop {
            if !pay(b, 0) {
                return false;
            }
            if let Some((rows, offset)) = self.active.as_mut() {
                if let Some(line) = rows.next() {
                    lines.push(line);
                    offsets.push(*offset);
                    *offset += 1;
                    stats.image_rows_adopted += 1;
                } else {
                    self.active = None;
                }
                continue;
            }
            if let Some(p) = self.pending.pop_front_if(|p| p.at == self.at) {
                let group = &mut images.groups[p.group];
                self.active = Some((std::mem::take(&mut group.prepared).into_iter(), p.offset));
                continue;
            }
            if let Some(line) = self.normal.next() {
                lines.push(line);
                offsets.push(self.offsets.next().unwrap());
                self.at += 1;
                stats.image_normal_rows_moved += 1;
                continue;
            }
            debug_assert!(self.pending.is_empty());
            return true;
        }
    }
    pub fn retire(&mut self, b: &mut Budget) -> bool {
        if let Some((rows, _)) = self.active.as_mut() {
            while let Some(line) = rows.as_mut_slice().first_mut() {
                if !Preparation::retire_line(line, b) || !pay(b, 0) {
                    return false;
                }
                rows.next();
            }
        }
        while let Some(line) = self.normal.as_mut_slice().first_mut() {
            if !Preparation::retire_line(line, b) || !pay(b, 0) {
                return false;
            }
            self.normal.next();
        }
        while !self.pending.is_empty() {
            if !pay(b, 0) {
                return false;
            }
            self.pending.pop_front();
        }
        true
    }
}
