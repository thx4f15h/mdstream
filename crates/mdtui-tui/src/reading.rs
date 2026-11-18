//! Source selections and explicit, bounded reading tasks. No task owns a
//! historical link memo or a document-sized list of matches.
use std::collections::HashMap;
use std::io::{self, Write};

use crossterm::event::{KeyCode, KeyModifiers};
use mdtui3::{
    Budget, Completion, DisplayMatch, DisplaySearch, Document, Selection, Span, TileId, WorkLimit,
};

use crate::render::first_frame::{FirstFrame, FirstFrameConfig, FirstFrameStatus};
use crate::render::objects::{LinkLease, SharedLinks};
use crate::render::table_widths::{TableWidthCache, TableWidthSnapshot};
use crate::screen::{ContentArea, Screen, StyleRole};

pub const MAX_QUERY_BYTES: usize = 256;
pub const MAX_CLIPBOARD_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CopyKind {
    Visible,
    Source,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CursorMove {
    Left,
    Right,
    Up,
    Down,
}

#[derive(Debug, Clone, Copy)]
struct Point {
    row: u16,
    col: u16,
    source: Span,
}

#[derive(Debug, Default)]
pub struct SourceSelection {
    active: bool,
    cursor: Option<Span>,
    anchor: Option<Span>,
    hint: Option<(u16, u16)>,
    pending: Option<CursorMove>,
    visible: bool,
}

impl SourceSelection {
    pub fn active(&self) -> bool {
        self.active
    }
    pub fn cursor(&self) -> Option<Span> {
        self.cursor
    }
    pub(crate) fn set_visible(&mut self, visible: bool) {
        self.visible = visible;
    }
    pub fn extending(&self) -> bool {
        self.anchor.is_some()
    }
    pub fn range(&self) -> Option<Span> {
        self.cursor.map(|cursor| {
            self.anchor.map_or(cursor, |anchor| {
                Span::new(anchor.start.min(cursor.start), anchor.end.max(cursor.end))
            })
        })
    }
    pub fn selection(&self, document: &Document) -> Option<Selection> {
        Selection::new(document, self.range()?).ok()
    }
    fn choose(&mut self, point: Point) {
        self.cursor = Some(point.source);
        self.visible = true;
        self.hint = Some((point.row, point.col));
    }
    pub fn toggle(&mut self, screen: &Screen, area: ContentArea) {
        self.active = !self.active;
        self.pending = None;
        if self.active
            && let Some(point) = points(screen, area).first()
        {
            self.anchor = None;
            self.choose(*point);
        }
    }
    pub fn toggle_extension(&mut self) {
        self.anchor = if self.anchor.is_some() {
            None
        } else {
            self.cursor
        };
    }
    pub fn suspend(&mut self) {
        self.active = false;
        self.pending = None;
    }
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// Returns an edge motion to be satisfied after a real viewport scroll.
    pub fn move_cursor(
        &mut self,
        screen: &Screen,
        area: ContentArea,
        direction: CursorMove,
        extend: bool,
    ) -> Option<CursorMove> {
        if self.pending.is_some() {
            return None;
        }
        let cells = points(screen, area);
        let current = self.cursor.and_then(|source| {
            cells
                .iter()
                .position(|point| {
                    point.source == source && self.hint == Some((point.row, point.col))
                })
                .or_else(|| {
                    cells
                        .iter()
                        .position(|point| overlaps(point.source, source))
                })
        });
        let index = current?;
        if extend && self.anchor.is_none() {
            self.anchor = self.cursor;
        }
        let point = cells[index];
        let next = match direction {
            CursorMove::Left => index.checked_sub(1),
            CursorMove::Right => (index + 1 < cells.len()).then_some(index + 1),
            CursorMove::Up | CursorMove::Down => {
                let row = if direction == CursorMove::Up {
                    cells
                        .iter()
                        .filter(|p| p.row < point.row)
                        .map(|p| p.row)
                        .max()
                } else {
                    cells
                        .iter()
                        .filter(|p| p.row > point.row)
                        .map(|p| p.row)
                        .min()
                };
                row.and_then(|row| {
                    cells
                        .iter()
                        .enumerate()
                        .filter(|(_, p)| p.row == row)
                        .min_by_key(|(_, p)| p.col.abs_diff(point.col))
                        .map(|(i, _)| i)
                })
            }
        };
        if let Some(next) = next {
            self.choose(cells[next]);
            None
        } else {
            self.pending = Some(direction);
            Some(direction)
        }
    }
    pub fn pending_motion(&self) -> Option<CursorMove> {
        self.pending
    }
    pub fn cancel_motion(&mut self) {
        self.pending = None;
    }
    pub fn reproject(&mut self, screen: &Screen, area: ContentArea) {
        let Some(cursor) = self.cursor else {
            return;
        };
        let cells = points(screen, area);
        if let Some(direction) = self.pending {
            let forward = matches!(direction, CursorMove::Right | CursorMove::Down);
            let row = cells
                .iter()
                .filter(|p| {
                    if forward {
                        p.source.start >= cursor.end
                    } else {
                        p.source.end <= cursor.start
                    }
                })
                .map(|p| p.row)
                .reduce(|a, b| if forward { a.min(b) } else { a.max(b) });
            if let Some(row) = row {
                let col = self.hint.map_or(0, |(_, col)| col);
                if let Some(point) = cells
                    .iter()
                    .filter(|p| p.row == row)
                    .min_by_key(|p| p.col.abs_diff(col))
                {
                    self.choose(*point);
                    self.pending = None;
                }
            }
        } else if let Some(point) = cells.iter().find(|p| overlaps(p.source, cursor)) {
            self.visible = true;
            // Reprojection never changes the source selection. A newly hidden
            // endpoint remains selected until explicit cursor movement.
            self.hint = Some((point.row, point.col));
        } else {
            self.visible = false;
        }
    }
}

fn points(screen: &Screen, area: ContentArea) -> Vec<Point> {
    let mut out = Vec::new();
    for row in 0..screen.rows() {
        for col in area.x..area.x.saturating_add(area.width).min(screen.cols()) {
            if let Some(cell) = screen.reading_zero_cell(row, col)
                && let Some(source) = cell.source()
            {
                out.push(Point { row, col, source });
            }
            if let Some(cell) = screen.reading_cell(row, col)
                && !cell.is_continuation()
                && let Some(source) = cell.source().filter(|source| !source.is_empty())
            {
                out.push(Point { row, col, source });
            }
        }
    }
    out
}
pub fn overlaps(a: Span, b: Span) -> bool {
    a.start < b.end && b.start < a.end
}

struct SearchTask {
    rendered: Option<Box<crate::reading_search::RenderedSearch>>,
    search: DisplaySearch,
    _lease: LinkLease,
    before: Option<usize>,
    after: Option<usize>,
    candidate: Option<(DisplayMatch, Option<crate::render::math::MathFocus>)>,
    pending_target: Option<(DisplayMatch, Option<crate::render::math::MathFocus>)>,
}

#[derive(Default)]
pub struct ReadingTools {
    width: u16,
    rendered_query: bool,
    pub selection: SourceSelection,
    input: Option<String>,
    query: String,
    search: Option<SearchTask>,
    current: Option<DisplayMatch>,
    current_focus: Option<crate::render::math::MathFocus>,
    match_visible: bool,
    searching: bool,
    stale: bool,
    revision: Option<u64>,
    copy: Option<Box<CopyTask>>,
    message: &'static str,
    clipboard_bytes: usize,
    clipboard_close_at: Option<usize>,
}

impl ReadingTools {
    pub(crate) fn set_width(&mut self, width: u16) {
        if self.width != 0
            && self.width != width
            && self.rendered_query
            && self.current_focus.is_some()
        {
            self.search = None;
            self.current = None;
            self.current_focus = None;
            self.searching = false;
            self.match_visible = false;
            self.message = "width changed; n/N: search";
        }
        self.width = width;
    }
    pub fn clipboard_bytes(&self) -> usize {
        self.clipboard_bytes
    }
    pub fn input(&self) -> Option<&str> {
        self.input.as_deref()
    }
    pub fn query(&self) -> &str {
        &self.query
    }
    pub fn current_match(&self) -> Option<&DisplayMatch> {
        self.current.as_ref()
    }
    pub(crate) fn current_math_focus(&self) -> Option<crate::render::math::MathFocus> {
        self.current_focus
    }
    pub(crate) fn set_match_visible(&mut self, visible: bool) {
        self.match_visible = visible;
    }
    pub fn stale(&self) -> bool {
        self.stale
    }
    pub fn message(&self) -> &'static str {
        self.message
    }
    pub fn pending(&self) -> bool {
        self.searching || self.copy_pending() || self.selection.pending.is_some()
    }
    pub fn status_visible(&self) -> bool {
        self.input.is_some()
            || self.selection.active
            || !self.query.is_empty()
            || !self.message.is_empty()
    }
    pub fn status(&self) -> Option<String> {
        if let Some(input) = &self.input {
            return Some(format!(
                "/{input} | Enter: search | Esc: cancel{}",
                if !self.message.is_empty() {
                    self.message
                } else if input.len() == MAX_QUERY_BYTES {
                    " | limit"
                } else {
                    ""
                }
            ));
        }
        if self.selection.active {
            let message = self.copy.as_ref().map_or_else(
                || self.message.to_owned(),
                |copy| format!("copy: {} B prepared; Esc cancels", copy.bytes.len()),
            );
            let range = self.selection.range().map_or_else(
                || "no text".to_owned(),
                |span| format!("{}..{}", span.start, span.end),
            );
            return Some(format!(
                "SELECT {} {range} | {} | y: visible Y: source | Space: extend",
                if !self.selection.visible {
                    "HIDDEN (v: reset)"
                } else if self.selection.extending() {
                    "EXTEND"
                } else {
                    "CURSOR"
                },
                message
            ));
        }
        if !self.query.is_empty() {
            return Some(format!(
                "/{} | {} | n/N: next/previous | / Enter: restart | {}",
                self.query,
                if self.stale {
                    "stale"
                } else if self.searching {
                    "searching"
                } else if self.current.is_some() {
                    if self.match_visible {
                        "match"
                    } else {
                        "match not drawn"
                    }
                } else {
                    "no match"
                },
                self.message
            ));
        }
        (!self.message.is_empty()).then(|| self.message.to_owned())
    }
    pub fn start_input(&mut self) {
        self.search = None;
        self.searching = false;
        self.message = "";
        self.input = Some(self.query.clone());
    }
    /// Text input is handled before global bindings, so q remains searchable.
    pub fn input_key(
        &mut self,
        code: KeyCode,
        modifiers: KeyModifiers,
        document: &Document,
        links: &SharedLinks,
    ) -> bool {
        let Some(input) = self.input.as_mut() else {
            return false;
        };
        match code {
            KeyCode::Tab | KeyCode::BackTab => return false,
            KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => return false,
            KeyCode::Esc => {
                self.input = None;
                self.message = "input cancelled";
            }
            KeyCode::Enter => {
                self.query = self.input.take().unwrap();
                self.submit(document, links);
            }
            KeyCode::Backspace => {
                input.pop();
            }
            KeyCode::Char(ch)
                if !modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
                    && !ch.is_control() =>
            {
                if input.len() + ch.len_utf8() <= MAX_QUERY_BYTES {
                    input.push(ch);
                    self.message = "";
                } else {
                    self.message = "query too long";
                }
            }
            _ => {}
        }
        true
    }
    fn task(
        &self,
        document: &Document,
        links: &SharedLinks,
        before: Option<usize>,
    ) -> Option<SearchTask> {
        Some(SearchTask {
            rendered: (document.math_object_stats().active > 0).then(|| {
                Box::new(crate::reading_search::RenderedSearch::new(
                    document,
                    &self.query,
                    if self.width == 0 { 80 } else { self.width },
                ))
            }),
            search: DisplaySearch::new(document, &self.query).ok()?,
            _lease: links.hold(),
            before,
            after: if before.is_none() {
                self.current.as_ref().map(|hit| hit.display.start)
            } else {
                None
            },
            candidate: None,
            pending_target: None,
        })
    }
    fn submit(&mut self, document: &Document, links: &SharedLinks) {
        self.search = None;
        self.current = None;
        self.current_focus = None;
        self.stale = false;
        self.revision = Some(document.revision());
        self.search = self.task(document, links, None);
        self.rendered_query = self
            .search
            .as_ref()
            .is_some_and(|task| task.rendered.is_some());
        self.searching = self.search.is_some();
        self.message = if self.query.is_empty() {
            "empty query"
        } else {
            "searching"
        };
    }
    pub fn navigate(&mut self, document: &Document, links: &SharedLinks, previous: bool) {
        if self.stale {
            self.message = "search stale: / then Enter";
            return;
        }
        if self.searching {
            self.message = "search in progress; Esc cancels";
            return;
        }
        if self.query.is_empty() {
            self.message = "press / to search";
            return;
        }
        if previous {
            let Some(current) = &self.current else {
                return;
            };
            self.search = self.task(document, links, Some(current.display.start));
        } else if self.search.is_none() {
            self.search = self.task(document, links, None);
        }
        self.searching = self.search.is_some();
        self.message = "searching";
    }
    pub(crate) fn cancel_navigation(&mut self) {
        self.search = None;
        self.searching = false;
    }
    pub fn cancel(&mut self, links: &SharedLinks) {
        self.search = None;
        self.searching = false;
        self.current = None;
        self.current_focus = None;
        self.cancel_copy();
        self.selection.clear();
        self.query.clear();
        self.input = None;
        self.message = "cancelled";
        links.request_retirement();
    }
    /// A later explicit directory jump wins over a still-running search. Keep
    /// its query/current hit, draft, selection and copy; n/N can resume explicitly.
    pub(crate) fn pause_search(&mut self, links: &SharedLinks) {
        if self.searching {
            self.search = None;
            self.searching = false;
            self.message = "search paused by directory; n/N: resume";
            links.request_retirement();
        }
    }
    pub fn changed(&mut self, revision: u64) {
        if self.revision.is_some_and(|old| old != revision) {
            self.search = None;
            self.current = None;
            self.current_focus = None;
            self.searching = false;
            self.stale = !self.query.is_empty();
        }
        if self
            .copy
            .as_ref()
            .is_some_and(|copy| copy.kind == CopyKind::Visible && copy.revision != revision)
        {
            self.cancel_copy();
            self.message = "copy cancelled: content changed";
        }
    }
    pub fn view_changed(&mut self) {
        if self
            .copy
            .as_ref()
            .is_some_and(|copy| copy.kind == CopyKind::Visible)
        {
            self.cancel_copy();
            self.message = "copy cancelled: view changed";
        }
    }
    pub fn width_changed(&mut self) {
        if self
            .copy
            .as_ref()
            .is_some_and(|copy| copy.kind == CopyKind::Visible)
        {
            self.cancel_copy();
            self.message = "copy cancelled: width changed";
        }
    }
    fn cancel_copy(&mut self) {
        if self.copy.take().is_some_and(|copy| copy.packet_written > 0) {
            self.clipboard_close_at = Some(0);
        }
    }
    pub fn copy_pending(&self) -> bool {
        self.copy.is_some() || self.clipboard_close_at.is_some()
    }
    pub(crate) fn start_copy_payload(
        &mut self,
        document: &Document,
        kind: CopyKind,
        width: u16,
        payload: Option<crate::render::payload::TextPayload>,
    ) {
        if kind == CopyKind::Visible
            && let Some(payload) = payload
        {
            self.cancel_copy();
            let Some(selection) = self.selection.selection(document) else {
                self.message = "select text first (v)";
                return;
            };
            if payload.len() > MAX_CLIPBOARD_BYTES {
                self.message = "copy too large (64 KiB limit)";
                return;
            }
            let mut task = CopyTask::new(document, selection.source, kind, width);
            task.direct = Some(payload);
            self.copy = Some(Box::new(task));
            self.message = "copy preparing; Esc cancels";
            return;
        }
        self.start_copy(document, kind, width);
    }
    pub fn start_copy(&mut self, document: &Document, kind: CopyKind, width: u16) {
        self.cancel_copy();
        let Some(selection) = self.selection.selection(document) else {
            self.message = "select text first (v)";
            return;
        };
        if kind == CopyKind::Source && selection.source.len() > MAX_CLIPBOARD_BYTES {
            self.message = "copy too large (64 KiB limit)";
            return;
        }
        self.copy = Some(Box::new(CopyTask::new(
            document,
            selection.source,
            kind,
            width,
        )));
        self.message = "copy preparing; Esc cancels";
    }
    pub(crate) fn bind_copy_view(&mut self, version: u64, folded: bool) {
        if let Some(copy) = &mut self.copy
            && copy.kind == CopyKind::Visible
        {
            copy.fold_version = Some(version);
            if folded {
                copy.direct = None;
            }
        }
    }
    /// All source, resolver, layout, projection and encoding work borrows the
    /// frame's existing Budget. Only one hit and a bounded clipboard are held.
    pub fn advance<W: Write + ?Sized>(
        &mut self,
        document: &Document,
        links: &SharedLinks,
        budget: &mut Budget,
        writer: &mut W,
        tables: &mut TableWidthCache,
    ) -> Option<usize> {
        self.advance_with_view(document, links, budget, writer, tables, None)
    }

    pub(crate) fn advance_with_view<W: Write + ?Sized>(
        &mut self,
        document: &Document,
        links: &SharedLinks,
        budget: &mut Budget,
        writer: &mut W,
        tables: &mut TableWidthCache,
        owner: Option<&mut crate::render::checkpoint::RenderCheckpointStore>,
    ) -> Option<usize> {
        self.changed(document.revision());
        let mut target = None;
        if self.searching
            && links.prepare(document, budget).unwrap_or(false)
            && let Some(task) = self.search.as_mut()
        {
            loop {
                if let Some((hit, focus)) = task.pending_target.as_ref() {
                    if let Some(focus) = focus {
                        match links.math().set_focus(document, *focus, budget) {
                            Ok(false) => break,
                            Err(_) => {
                                self.searching = false;
                                self.stale = true;
                                break;
                            }
                            Ok(true) => {}
                        }
                    }
                    target = Some(hit.source.start);
                    self.current = Some(hit.clone());
                    self.current_focus = *focus;
                    task.pending_target = None;
                    self.searching = false;
                    self.message = if task.before.is_some() {
                        "previous match"
                    } else {
                        "match"
                    };
                    break;
                }
                let result = if let Some(rendered) = &mut task.rendered {
                    rendered.next(document, links, budget)
                } else {
                    links.with_resolver(|resolver| {
                        task.search.next_with_resolver(document, resolver, budget)
                    })
                };
                match result {
                    Ok((completion, hit)) => {
                        if let Some(hit) = hit {
                            if task.after.is_some_and(|after| hit.display.start <= after) {
                                continue;
                            }
                            let focus = task.rendered.as_ref().and_then(|r| r.hit_focus());
                            if let Some(before) = task.before {
                                if hit.display.start < before {
                                    task.candidate = Some((hit, focus));
                                    continue;
                                }
                                if let Some(candidate) = task.candidate.take() {
                                    task.pending_target = Some(candidate);
                                    continue;
                                }
                                self.searching = false;
                                self.message = "first match";
                                break;
                            }
                            task.pending_target = Some((hit, focus));
                            continue;
                        }
                        if completion == Completion::EndOfDocument {
                            if let Some(candidate) = task.candidate.take() {
                                task.pending_target = Some(candidate);
                                continue;
                            }
                            self.searching = false;
                            self.message = if self.current.is_some() {
                                "end of matches"
                            } else {
                                "no match"
                            };
                        }
                        break;
                    }
                    Err(_) => {
                        self.searching = false;
                        self.stale = true;
                        break;
                    }
                }
            }
        }
        if !self.searching
            && self
                .search
                .as_ref()
                .is_some_and(|task| task.before.is_some())
        {
            // A backwards rescan cannot be used as the next forward cursor.
            // Forward restart skips through the current display offset below.
            self.search = None;
        }
        if let Some(at) = self.clipboard_close_at.as_mut() {
            const CLOSE: &[u8] = b"\x18\x1b\\";
            if budget.remaining().bytes > 0
                && budget.charge(WorkLimit {
                    queries: 1,
                    objects: 1,
                    ..Default::default()
                })
            {
                let count = (CLOSE.len() - *at).min(budget.remaining().bytes);
                if let Ok(n) = writer.write(&CLOSE[*at..*at + count]) {
                    assert!(budget.charge(WorkLimit {
                        bytes: n,
                        ..Default::default()
                    }));
                    *at += n;
                    self.clipboard_bytes += n;
                }
                if *at == CLOSE.len() {
                    self.clipboard_close_at = None;
                }
            }
            if self.clipboard_close_at.is_some() {
                return target;
            }
        }
        if let Some(copy) = self.copy.as_mut() {
            let before = copy.packet_written;
            let result = copy.advance_with_view(document, links, budget, writer, tables, owner);
            self.clipboard_bytes += copy.packet_written - before;
            match result {
                Ok(false) => {}
                Ok(true) => {
                    self.copy = None;
                    self.message = "clipboard request sent";
                }
                Err(message) => {
                    self.cancel_copy();
                    self.message = message;
                }
            }
        }
        target
    }
}

struct MathAtomCopy {
    cursor: crate::render::math::MathTextCursor,
    reader: Option<crate::render::payload::PayloadReader>,
    pending: [u8; 8],
    at: usize,
    end: usize,
    row: Option<usize>,
    column: usize,
    newlines: usize,
    spaces: usize,
}
impl MathAtomCopy {
    fn advance(
        &mut self,
        document: &Document,
        selected: Span,
        budget: &mut Budget,
        bytes: &mut Vec<u8>,
    ) -> Result<bool, &'static str> {
        loop {
            if self.newlines > 0 || self.spaces > 0 || self.at < self.end {
                if bytes.len() == MAX_CLIPBOARD_BYTES {
                    return Err("copy too large (64 KiB limit)");
                }
                if !budget.charge(WorkLimit {
                    bytes: 1,
                    ..Default::default()
                }) {
                    return Ok(false);
                }
                let byte = if self.newlines > 0 {
                    self.newlines -= 1;
                    b'\n'
                } else if self.spaces > 0 {
                    self.spaces -= 1;
                    b' '
                } else {
                    let byte = self.pending[self.at];
                    self.at += 1;
                    byte
                };
                bytes.push(byte);
                continue;
            }
            if let Some(reader) = &mut self.reader {
                match reader.next(budget).map_err(|_| "copy changed")? {
                    crate::render::payload::PayloadStep::BudgetExhausted => return Ok(false),
                    crate::render::payload::PayloadStep::Bytes { utf8, len } => {
                        self.pending = utf8;
                        self.at = 0;
                        self.end = len as usize;
                    }
                    crate::render::payload::PayloadStep::Complete => self.reader = None,
                }
                continue;
            }
            match self
                .cursor
                .advance(document, budget)
                .map_err(|_| "copy changed")?
            {
                crate::render::math::MathTextStep::BudgetExhausted => return Ok(false),
                crate::render::math::MathTextStep::Complete => return Ok(true),
                crate::render::math::MathTextStep::Cell { cell, .. }
                    if overlaps(cell.source, selected) =>
                {
                    self.newlines = self.row.map_or(0, |row| cell.y.saturating_sub(row));
                    self.spaces = if self.row == Some(cell.y) {
                        cell.x.saturating_sub(self.column)
                    } else {
                        0
                    };
                    self.row = Some(cell.y);
                    self.column = cell.x + usize::from(cell.width);
                    self.reader = Some(cell.text.reader());
                }
                _ => {}
            }
        }
    }
}

struct CopyTask {
    math_probe: Option<mdtui3::MathWindow>,
    math_meta: Option<mdtui3::MathObjectMeta>,
    math_checked: bool,
    math: Option<Box<MathAtomCopy>>,
    windowed: bool,
    window_offset: u64,
    math_newlines: usize,
    copied_placeholders: std::collections::BTreeSet<usize>,
    fold_version: Option<u64>,
    projection_start: Option<usize>,
    direct: Option<crate::render::payload::TextPayload>,
    direct_at: usize,
    packet_written: usize,
    cell_reader: Option<crate::render::payload::PayloadReader>,
    cell_pending: [u8; 8],
    cell_pending_at: usize,
    cell_pending_end: usize,
    kind: CopyKind,
    revision: u64,
    source: Span,
    width: u16,
    position: usize,
    layout: Option<FirstFrame>,
    layout_ready: bool,
    row: usize,
    line_serial: usize,
    extra_block: usize,
    extra_row: usize,
    extra_ready: bool,
    extra_pending: Option<(mdtui3::BlockId, usize, usize)>,
    cell: usize,
    column: usize,
    emitted_row: Option<usize>,
    emitted_block: Option<mdtui3::BlockId>,
    emitted_math: bool,
    bytes: Vec<u8>,
    encoded: Vec<u8>,
    encoding: usize,
    transfer_charged: usize,
    tables: Option<usize>,
    snapshots: HashMap<mdtui3::TableId, TableWidthSnapshot>,
    snapshots_ready: bool,
    separator: bool,
    word_space: bool,
    last_source_end: Option<usize>,
    gap_first: Option<bool>,
    gap_checked: bool,
}
impl CopyTask {
    fn new(document: &Document, source: Span, kind: CopyKind, width: u16) -> Self {
        Self {
            math_probe: None,
            math_meta: None,
            math_checked: false,
            math: None,
            windowed: false,
            window_offset: 0,
            math_newlines: 0,
            copied_placeholders: Default::default(),
            fold_version: None,
            projection_start: None,
            direct: None,
            direct_at: 0,
            packet_written: 0,
            cell_reader: None,
            cell_pending: [0; 8],
            cell_pending_at: 0,
            cell_pending_end: 0,
            kind,
            revision: document.revision(),
            source,
            width,
            position: source.start,
            layout: None,
            layout_ready: false,
            row: 0,
            line_serial: 0,
            extra_block: 0,
            extra_row: 0,
            extra_ready: false,
            extra_pending: None,
            cell: 0,
            column: 0,
            emitted_row: None,
            emitted_block: None,
            emitted_math: false,
            bytes: Vec::new(),
            encoded: Vec::new(),
            encoding: 0,
            transfer_charged: 0,
            tables: None,
            snapshots: HashMap::new(),
            snapshots_ready: false,
            separator: false,
            word_space: false,
            last_source_end: None,
            gap_first: None,
            gap_checked: false,
        }
    }
    #[cfg(test)]
    fn advance<W: Write + ?Sized>(
        &mut self,
        document: &Document,
        links: &SharedLinks,
        budget: &mut Budget,
        writer: &mut W,
        tables: &mut TableWidthCache,
    ) -> Result<bool, &'static str> {
        self.advance_with_view(document, links, budget, writer, tables, None)
    }
    fn advance_with_view<W: Write + ?Sized>(
        &mut self,
        document: &Document,
        links: &SharedLinks,
        budget: &mut Budget,
        writer: &mut W,
        tables: &mut TableWidthCache,
        mut owner: Option<&mut crate::render::checkpoint::RenderCheckpointStore>,
    ) -> Result<bool, &'static str> {
        if let Some(version) = self.fold_version
            && owner
                .as_ref()
                .is_none_or(|owner| owner.view().version() != version)
        {
            return Err("copy cancelled: view changed");
        }
        if self.kind == CopyKind::Visible && self.direct.is_none() && !self.math_checked {
            if document.math_object_stats().active == 0 {
                self.math_checked = true;
            } else {
                if self.math_meta.is_none() {
                    if self.math_probe.is_none() {
                        self.math_probe = Some(
                            document
                                .math_window(self.source)
                                .map_err(|_| "copy changed")?,
                        );
                    }
                    match self
                        .math_probe
                        .as_mut()
                        .unwrap()
                        .advance(document, budget)
                        .map_err(|_| "copy changed")?
                    {
                        mdtui3::MathWindowStep::BudgetExhausted => return Ok(false),
                        mdtui3::MathWindowStep::Object(meta) => self.math_meta = Some(meta),
                        mdtui3::MathWindowStep::Complete => self.math_checked = true,
                    }
                }
                if let Some(meta) = self.math_meta {
                    let visible = if let Some(owner) =
                        owner.as_ref().filter(|owner| owner.view().has_collapsed())
                    {
                        match owner
                            .view()
                            .visible_fragment(document, meta.source.start, budget)
                        {
                            Ok(Some(fragment)) => {
                                fragment.start <= meta.source.start
                                    && meta.source.end <= fragment.end
                            }
                            Ok(None) => return Ok(false),
                            Err(mdtui3::LayoutError::InvalidRange) => false,
                            Err(_) => return Err("copy changed"),
                        }
                    } else {
                        true
                    };
                    if visible
                        && meta.source.start <= self.source.start
                        && self.source.end <= meta.source.end
                    {
                        self.math = Some(Box::new(MathAtomCopy {
                            cursor: crate::render::math::MathTextCursor::new(
                                document,
                                links.math(),
                                meta,
                            )
                            .map_err(|_| "copy changed")?,
                            reader: None,
                            pending: [0; 8],
                            at: 0,
                            end: 0,
                            row: None,
                            column: 0,
                            newlines: 0,
                            spaces: 0,
                        }));
                    }
                    self.math_checked = true;
                }
                if self.math_checked {
                    self.math_probe = None;
                    self.math_meta = None;
                }
            }
        }
        if let Some(math) = &mut self.math {
            if !math.advance(document, self.source, budget, &mut self.bytes)? {
                return Ok(false);
            }
        } else if self.kind == CopyKind::Source {
            while self.position < self.source.end {
                // Charge before even looking at the next UTF-8 scalar.
                if !budget.charge(WorkLimit {
                    queries: 1,
                    objects: 1,
                    bytes: 4,
                    ..WorkLimit::default()
                }) {
                    return Ok(false);
                }
                let ch = document.source()[self.position..self.source.end]
                    .chars()
                    .next()
                    .ok_or("copy failed")?;
                let mut scalar = [0; 4];
                self.bytes
                    .extend_from_slice(ch.encode_utf8(&mut scalar).as_bytes());
                self.position += ch.len_utf8();
            }
        } else if let Some(payload) = &self.direct {
            let reader = self.cell_reader.get_or_insert_with(|| payload.reader());
            while self.direct_at < payload.len() {
                if self.cell_pending_at == self.cell_pending_end {
                    match reader.next(budget).map_err(|_| "copy changed")? {
                        crate::render::payload::PayloadStep::BudgetExhausted => return Ok(false),
                        crate::render::payload::PayloadStep::Complete => break,
                        crate::render::payload::PayloadStep::Bytes { utf8, len } => {
                            self.cell_pending = utf8;
                            self.cell_pending_at = 0;
                            self.cell_pending_end = usize::from(len);
                        }
                    }
                }
                if !budget.charge(WorkLimit {
                    bytes: 1,
                    ..Default::default()
                }) {
                    return Ok(false);
                }
                self.bytes.push(self.cell_pending[self.cell_pending_at]);
                self.cell_pending_at += 1;
                self.direct_at += 1;
            }
        } else {
            if self.layout.is_none() && self.projection_start.is_none() {
                if let Some(owner) = owner.as_ref().filter(|owner| owner.view().has_collapsed()) {
                    let anchor = mdtui3::FoldAnchor::new(document, self.source.start)
                        .map_err(|_| "copy source changed")?;
                    let Some(target) = anchor
                        .resolve(document, owner.view(), 1, budget)
                        .map_err(|_| "copy source changed")?
                    else {
                        return Ok(false);
                    };
                    self.projection_start = Some(target.target.source.unwrap_or(self.source.start));
                } else {
                    self.projection_start = Some(self.source.start);
                }
            }
            if self.layout.is_none() {
                if !budget.charge(WorkLimit {
                    queries: 1,
                    objects: 1,
                    ..WorkLimit::default()
                }) {
                    return Ok(false);
                }
                let start = document
                    .tile_at(self.source.start)
                    .ok_or("copy has no visible text")?;
                let end = document
                    .tile_at(self.source.end.saturating_sub(1))
                    .ok_or("copy has no visible text")?;
                // At most eight tiles of preceding block context; never lay out
                // an arbitrary hidden prefix just to copy a small selection.
                let block_start = document
                    .block_at(self.source.start)
                    .and_then(|id| document.block(id))
                    .and_then(|block| document.tile_at(block.source.start))
                    .unwrap_or(start);
                let top = TileId(block_start.0.max(start.0.saturating_sub(8)));
                let projection_start = self.projection_start.unwrap();
                let config = FirstFrameConfig {
                    top_tile: top,
                    tile_limit: end.0 - top.0 + 1,
                    width: self.width,
                    row_limit: u64::MAX,
                };
                let mut first = if let Some(owner) = owner.as_ref() {
                    FirstFrame::new_for_view(document, config, owner)
                } else {
                    FirstFrame::new(document, config)
                }
                .map_err(|_| "copy layout failed")?;
                if document.math_object_stats().active > 0 {
                    first.set_math_copy_window(0, 16);
                    self.windowed = true;
                }
                first.set_projection_range(Span::new(projection_start, self.source.end));
                first.prepare_only();
                first.set_links(links.clone());
                first
                    .set_table_options(document, tables.policy(), &tables.column_positions())
                    .map_err(|_| "copy table view failed")?;
                self.layout = Some(first);
            }
            let layout = self.layout.as_mut().unwrap();
            if owner
                .as_ref()
                .is_some_and(|owner| owner.view().has_collapsed())
            {
                self.snapshots_ready = true;
            }
            if !self.snapshots_ready {
                if self.tables.is_none() {
                    if !budget.charge(WorkLimit {
                        queries: 1,
                        objects: 1,
                        ..WorkLimit::default()
                    }) {
                        return Ok(false);
                    }
                    self.tables =
                        Some(document.tables().partition_point(|table| {
                            table.source.end <= layout.plan().visible.start
                        }));
                }
                while let Some(_table) = document
                    .tables()
                    .get(self.tables.unwrap())
                    .filter(|table| table.source.start < layout.plan().visible.end)
                {
                    let snapshot = tables
                        .snapshot_budgeted(
                            document,
                            mdtui3::TableId(self.tables.unwrap()),
                            usize::from(self.width),
                            budget,
                        )
                        .map_err(|_| "copy table layout failed")?;
                    let Some(snapshot) = snapshot else {
                        return Ok(false);
                    };
                    self.snapshots
                        .insert(mdtui3::TableId(self.tables.unwrap()), snapshot);
                    *self.tables.as_mut().unwrap() += 1;
                }
                layout
                    .set_table_widths(std::mem::take(&mut self.snapshots))
                    .map_err(|_| "copy layout failed")?;
                self.snapshots_ready = true;
            }
            if !self.layout_ready {
                let step = if let Some(owner) = owner.as_mut() {
                    layout.advance_using(document, owner, budget)
                } else {
                    layout.advance(document, budget)
                }
                .map_err(|_| "copy layout failed")?;
                if !matches!(
                    step.status,
                    FirstFrameStatus::Covered | FirstFrameStatus::EndOfDocument
                ) {
                    return Ok(false);
                }
                if self.window_offset == 0 {
                    self.windowed &= !layout.scanner().math_views().is_empty();
                }
                self.layout_ready = true;
            }
            loop {
                // Table cells were prepared through the paid indexed window,
                // even when their physical row owner lies outside its tiles.
                // Merge only those unpublished rows; never extend the layout
                // across the containing row's potentially huge source range.
                if !self.extra_ready {
                    loop {
                        let Some(block) = layout.plan().blocks.get(self.extra_block) else {
                            self.extra_ready = true;
                            break;
                        };
                        if !budget.charge(WorkLimit {
                            queries: 1,
                            objects: 1,
                            ..WorkLimit::default()
                        }) {
                            return Ok(false);
                        }
                        if matches!(block.kind, mdtui3::BlockKind::Table { .. })
                            && let Some((line, outside)) =
                                layout.prepared_copy_row(block.id, self.extra_row)
                        {
                            if outside {
                                self.extra_pending = Some((
                                    block.id,
                                    self.extra_row,
                                    line.source.map_or(block.source.start, |s| s.source.start),
                                ));
                                self.extra_ready = true;
                                break;
                            }
                            self.extra_row += 1;
                        } else {
                            self.extra_block += 1;
                            self.extra_row = 0;
                        }
                    }
                }
                let mut published = layout.lines().get(self.row);
                if self.windowed {
                    if let Some(offset) = layout.sink().and_then(|sink| sink.line_offset(self.row))
                    {
                        if offset < self.window_offset {
                            if !budget.charge(WorkLimit {
                                queries: 1,
                                objects: 1,
                                ..Default::default()
                            }) {
                                return Ok(false);
                            }
                            self.row += 1;
                            continue;
                        }
                        if offset >= self.window_offset + 16 {
                            published = None;
                        }
                    }
                    if let Some((block, row, _)) = self.extra_pending {
                        let offset = layout.prepared_copy_row_offset(block, row).unwrap_or(0);
                        if offset < self.window_offset {
                            self.extra_row += 1;
                            self.extra_pending = None;
                            self.extra_ready = false;
                            continue;
                        }
                    }
                }
                let extra_in_window = self.extra_pending.is_some_and(|(block, row, _)| {
                    !self.windowed
                        || layout.prepared_copy_row_offset(block, row).unwrap_or(0)
                            < self.window_offset + 16
                });
                if published.is_none() && !extra_in_window {
                    if self.windowed && self.window_offset + 16 < layout.copy_logical_rows() {
                        if !budget.charge(WorkLimit {
                            queries: 1,
                            objects: 1,
                            ..Default::default()
                        }) {
                            return Ok(false);
                        }
                        self.window_offset += 16;
                        layout.set_math_copy_window(self.window_offset, 16);
                        self.layout_ready = false;
                        self.row = 0;
                        self.cell = 0;
                        self.extra_block = 0;
                        self.extra_row = 0;
                        self.extra_ready = false;
                        self.extra_pending = None;
                        return Ok(false);
                    }
                    break;
                }
                let extra = extra_in_window
                    && self.extra_pending.is_some_and(|(_, _, start)| {
                        published
                            .is_none_or(|line| line.source.is_some_and(|s| start <= s.source.start))
                    });
                // One object/query and up to six bytes makes progress even for
                // a cell containing many combining scalars. Output is chunked.
                if self.cell_reader.is_none()
                    && !budget.charge(WorkLimit {
                        queries: 1,
                        objects: 1,
                        bytes: 1,
                        ..WorkLimit::default()
                    })
                {
                    return Ok(false);
                }
                if self.windowed && self.cell == 0 {
                    self.line_serial = if extra {
                        let (block, row, _) = self.extra_pending.unwrap();
                        layout
                            .prepared_copy_row_offset(block, row)
                            .unwrap_or(self.line_serial as u64) as usize
                    } else {
                        layout
                            .sink()
                            .and_then(|sink| sink.line_offset(self.row))
                            .unwrap_or(self.line_serial as u64) as usize
                    };
                }
                let line = if extra {
                    let (block, row, _) = self.extra_pending.unwrap();
                    layout.prepared_copy_row(block, row).unwrap().0
                } else {
                    published.unwrap()
                };
                let Some(cell) = line.cells.get(self.cell) else {
                    // An empty code row owns its physical LF even though its
                    // indentation cells have no source. Serialize that row
                    // before moving on; the next selected row supplies the
                    // following separator. The one terminator byte is covered
                    // by this paid step, never a scan of the source gap.
                    if self.emitted_row != Some(self.line_serial)
                        && let Some(owner) = line.source
                        && owner
                            .block
                            .and_then(|id| document.block(id))
                            .is_some_and(|b| matches!(b.kind, mdtui3::BlockKind::Code { .. }))
                        && owner.source.len() == 1
                        && overlaps(owner.source, self.source)
                        && document.source().as_bytes()[owner.source.start] == b'\n'
                    {
                        if self.emitted_row.is_some() {
                            if self.bytes.len() >= MAX_CLIPBOARD_BYTES {
                                return Err("copy too large (64 KiB limit)");
                            }
                            self.bytes.push(b'\n');
                        }
                        self.emitted_row = Some(self.line_serial);
                        self.emitted_block = owner.block;
                    }
                    if extra {
                        self.extra_row += 1;
                        self.extra_ready = false;
                        self.extra_pending = None;
                    } else {
                        self.row += 1;
                    }
                    self.line_serial += 1;
                    self.cell = 0;
                    self.column = 0;
                    self.separator = false;
                    self.word_space = false;
                    continue;
                };
                let placeholder = cell.source.and_then(|source| {
                    let at = layout
                        .placeholders()
                        .partition_point(|p| p.candidate.hidden.start < source.start);
                    layout
                        .placeholders()
                        .get(at)
                        .filter(|p| p.candidate.hidden == source)
                        .copied()
                });
                if let Some(placeholder) = placeholder
                    && self
                        .copied_placeholders
                        .contains(&placeholder.candidate.hidden.start)
                {
                    self.column += usize::from(cell.width);
                    self.cell += 1;
                    continue;
                }
                let replacement = placeholder.map(|placeholder| {
                    let mut replacement = cell.clone();
                    replacement.text =
                        crate::render::plan::placeholder_label(placeholder.candidate).into();
                    replacement
                });
                let cell = replacement.as_ref().unwrap_or(cell);
                if self.column + usize::from(cell.width) <= usize::from(self.width)
                    && cell
                        .source
                        .is_some_and(|source| overlaps(source, self.source))
                {
                    if self.position == self.source.start {
                        if self.emitted_row.is_some_and(|old| old != self.line_serial) {
                            let block = line.source.and_then(|line| line.block);
                            let separate = self.emitted_math
                                || cell.style.contains(StyleRole::MATH)
                                || block != self.emitted_block
                                || block
                                    .and_then(|id| document.block(id))
                                    .is_some_and(|block| {
                                        matches!(
                                            block.kind,
                                            mdtui3::BlockKind::Code { .. }
                                                | mdtui3::BlockKind::Table { .. }
                                                | mdtui3::BlockKind::Math { .. }
                                        )
                                    });
                            if separate {
                                let math = self.emitted_math
                                    || cell.style.contains(StyleRole::MATH)
                                    || block.and_then(|id| document.block(id)).is_some_and(|b| {
                                        matches!(b.kind, mdtui3::BlockKind::Math { .. })
                                    });
                                let count = if math {
                                    self.line_serial
                                        .saturating_sub(self.emitted_row.unwrap())
                                        .max(1)
                                } else {
                                    1
                                };
                                while self.math_newlines < count {
                                    if self.bytes.len() >= MAX_CLIPBOARD_BYTES {
                                        return Err("copy too large (64 KiB limit)");
                                    }
                                    if !budget.charge(WorkLimit {
                                        bytes: 1,
                                        ..Default::default()
                                    }) {
                                        return Ok(false);
                                    }
                                    self.bytes.push(b'\n');
                                    self.math_newlines += 1;
                                }
                                self.math_newlines = 0;
                            } else if self.fold_version.is_none()
                                && block
                                    .and_then(|id| document.block(id))
                                    .is_some_and(|block| {
                                        matches!(block.kind, mdtui3::BlockKind::ListItem { .. })
                                    })
                            {
                                // List word wrapping does not draw its normalized
                                // separator at either edge. Recover one logical
                                // space from the source gap's boundary scalars,
                                // never from hidden markup inside the gap.
                                if let Some(last) = self
                                    .last_source_end
                                    .filter(|last| *last < cell.source.unwrap().start)
                                {
                                    let end = cell.source.unwrap().start;
                                    if self.gap_first.is_none() {
                                        if !budget.charge(WorkLimit {
                                            bytes: 4,
                                            ..WorkLimit::default()
                                        }) {
                                            return Ok(false);
                                        }
                                        self.gap_first = Some(
                                            document.source()[last..end]
                                                .chars()
                                                .next()
                                                .is_some_and(char::is_whitespace),
                                        );
                                        self.gap_checked = self.gap_first == Some(true);
                                    }
                                    if !self.gap_checked {
                                        if !budget.charge(WorkLimit {
                                            bytes: 4,
                                            ..WorkLimit::default()
                                        }) {
                                            return Ok(false);
                                        }
                                        self.gap_first = Some(
                                            document.source()[last..end]
                                                .chars()
                                                .next_back()
                                                .is_some_and(char::is_whitespace),
                                        );
                                        self.gap_checked = true;
                                    }
                                    if self.gap_first == Some(true) {
                                        self.bytes.push(b' ');
                                    }
                                }
                            }
                        } else if self.emitted_row == Some(self.line_serial) {
                            if self.separator {
                                self.bytes.push(b'\t');
                            } else if self.word_space {
                                self.bytes.push(b' ');
                            }
                        }
                        self.separator = false;
                        self.word_space = false;
                        self.gap_first = None;
                        self.gap_checked = false;
                    }
                    self.emitted_row = Some(self.line_serial);
                    self.emitted_block = line.source.and_then(|line| line.block);
                    self.emitted_math = cell.style.contains(StyleRole::MATH);
                    if self.bytes.len() + cell.text.len() - (self.position - self.source.start)
                        > MAX_CLIPBOARD_BYTES
                    {
                        return Err("copy too large (64 KiB limit)");
                    }
                    if self.cell_reader.is_none() {
                        self.cell_reader = Some(cell.text.reader());
                    }
                    while self.position - self.source.start < cell.text.len() {
                        if self.cell_pending_at == self.cell_pending_end {
                            match self
                                .cell_reader
                                .as_mut()
                                .unwrap()
                                .next(budget)
                                .map_err(|_| "copy changed")?
                            {
                                crate::render::payload::PayloadStep::BudgetExhausted => {
                                    return Ok(false);
                                }
                                crate::render::payload::PayloadStep::Complete => break,
                                crate::render::payload::PayloadStep::Bytes { utf8, len } => {
                                    self.cell_pending = utf8;
                                    self.cell_pending_at = 0;
                                    self.cell_pending_end = usize::from(len);
                                }
                            }
                        }
                        if !budget.charge(WorkLimit {
                            bytes: 1,
                            ..WorkLimit::default()
                        }) {
                            return Ok(false);
                        }
                        self.bytes.push(self.cell_pending[self.cell_pending_at]);
                        self.cell_pending_at += 1;
                        self.position += 1;
                    }
                    self.cell_reader = None;
                    self.cell_pending_at = 0;
                    self.cell_pending_end = 0;
                    self.position = self.source.start;
                    self.last_source_end = cell.source.map(|source| source.end);
                    if let Some(placeholder) = placeholder {
                        self.copied_placeholders
                            .insert(placeholder.candidate.hidden.start);
                    }
                }
                if cell.source.is_none()
                    && cell.text == " "
                    && self.emitted_row == Some(self.line_serial)
                {
                    self.word_space = true;
                }
                if cell.source.is_none()
                    && cell.style.contains(StyleRole::TABLE_BORDER)
                    && cell.text == "│"
                {
                    self.separator = true;
                }
                self.column += usize::from(cell.width);
                self.cell += 1;
            }
        }
        const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        while self.encoding < self.bytes.len() {
            let count = (self.bytes.len() - self.encoding).min(3);
            if !budget.charge(WorkLimit {
                bytes: 6,
                objects: 1,
                ..WorkLimit::default()
            }) {
                return Ok(false);
            }
            let data = &self.bytes[self.encoding..self.encoding + count];
            let a = data[0];
            let b = data.get(1).copied().unwrap_or(0);
            let c = data.get(2).copied().unwrap_or(0);
            self.encoded.extend_from_slice(&[
                B64[usize::from(a >> 2)],
                B64[usize::from((a & 3) << 4 | b >> 4)],
                if count > 1 {
                    B64[usize::from((b & 15) << 2 | c >> 6)]
                } else {
                    b'='
                },
                if count > 2 {
                    B64[usize::from(c & 63)]
                } else {
                    b'='
                },
            ]);
            self.encoding += count;
        }
        while self.transfer_charged < self.encoded.len() + 8 {
            let bytes = budget
                .remaining()
                .bytes
                .min(self.encoded.len() + 8 - self.transfer_charged);
            if bytes == 0
                || !budget.charge(WorkLimit {
                    bytes,
                    ..WorkLimit::default()
                })
            {
                return Ok(false);
            }
            self.transfer_charged += bytes;
        }
        if !budget.charge(WorkLimit {
            queries: 1,
            objects: 1,
            ..WorkLimit::default()
        }) {
            return Ok(false);
        }
        // The complete bounded OSC52 packet has already earned its byte
        // credit. It remains atomic with respect to other terminal output,
        // including across short writes and EAGAIN; its reserved credit is
        // intentionally independent of the current six-byte micro budget.
        while self.packet_written < self.encoded.len() + 8 {
            let at = self.packet_written;
            let bytes = if at < 7 {
                &b"\x1b]52;c;"[at..]
            } else if at < 7 + self.encoded.len() {
                &self.encoded[at - 7..]
            } else {
                b"\x07"
            };
            match writer.write(bytes) {
                Ok(0) => return Err("clipboard write failed"),
                Ok(n) => self.packet_written += n,
                Err(e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                    ) =>
                {
                    return Ok(false);
                }
                Err(_) => return Err("clipboard write failed"),
            }
        }
        Ok(true)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    pub(super) fn decode(packet: &[u8]) -> Vec<u8> {
        assert!(packet.starts_with(b"\x1b]52;c;"));
        assert_eq!(packet.last(), Some(&7));
        let mut out = Vec::new();
        let mut bits = 0_u32;
        let mut count = 0;
        for byte in packet[7..packet.len() - 1]
            .iter()
            .take_while(|byte| **byte != b'=')
        {
            let alphabet = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
            bits = (bits << 6) | alphabet.iter().position(|b| b == byte).unwrap() as u32;
            count += 6;
            if count >= 8 {
                count -= 8;
                out.push((bits >> count) as u8);
            }
        }
        out
    }
    fn copy(source: &str, kind: CopyKind, width: u16, limit: WorkLimit) -> (Vec<u8>, WorkLimit) {
        let mut doc = Document::new();
        doc.append(source).unwrap();
        doc.finish();
        let links = SharedLinks::new(&doc);
        let mut tables = TableWidthCache::new(&doc);
        tables.set_links(links.clone());
        let mut task = CopyTask::new(&doc, Span::new(0, source.len()), kind, width);
        let mut zero = Budget::new(WorkLimit::default());
        let mut packet = Vec::new();
        assert!(
            !task
                .advance(&doc, &links, &mut zero, &mut packet, &mut tables)
                .unwrap()
        );
        assert!(task.layout.is_none());
        assert!(task.bytes.is_empty());
        assert!(packet.is_empty());
        assert_eq!(zero.used(), WorkLimit::default());
        let mut used = WorkLimit::default();
        for _ in 0..200000 {
            let mut budget = Budget::new(limit);
            let done = task
                .advance(&doc, &links, &mut budget, &mut packet, &mut tables)
                .unwrap();
            used.bytes += budget.used().bytes;
            used.objects += budget.used().objects;
            assert!(budget.used().bytes <= limit.bytes);
            assert!(budget.used().queries <= limit.queries);
            assert!(budget.used().objects <= limit.objects);
            if done {
                return (decode(&packet), used);
            }
            assert!(packet.is_empty(), "partial OSC must never escape");
        }
        panic!("copy stuck");
    }
    fn tiny() -> WorkLimit {
        WorkLimit {
            queries: 1,
            objects: 1,
            bytes: 6,
            tiles: 1,
            ..WorkLimit::default()
        }
    }
    #[test]
    fn independently_written_projection_bytes_cover_tables_breaks_images_and_replacements() {
        for (source, width, expected) in [
            ("| a | b |\n| - | - |\n| c | d |\n", 30, "a\tb\nc\td"),
            ("abcd efgh ijkl\n", 6, "abcd efgh ijkl"),
            ("- before [secret](url) after\n", 40, "before after"),
            ("- one two three\n", 6, "one two three"),
            ("- abcdefghij kl\n", 6, "abcdefghij kl"),
            ("- abc`def`gh ij\n", 5, "abcdefgh ij"),
            ("# before [secret](url) after\n", 40, "before  after"),
            ("| a | b |\n| - | - |\n| c | d |\n", 5, "a\nc"),
            ("\\*x\\* &NotEqualTilde; &fjlig;\n", 30, "*x* ≂̸ fj"),
            ("ab\r\ncd  \r\nef\\\ngh\n", 30, "ab cdefgh"),
            (
                "&fjlig; ` x  y ` ![A &amp; 界](x) ![](x)\n",
                80,
                "fj x  y A & 界 [image]",
            ),
            ("```txt\nabcdefghijkl\n```\n", 5, "abc"),
        ] {
            let (actual, work) = copy(source, CopyKind::Visible, width, tiny());
            assert_eq!(actual, expected.as_bytes(), "source {source:?}");
            assert_eq!(
                copy(source, CopyKind::Visible, width, WorkLimit::unlimited()).0,
                expected.as_bytes()
            );
            assert_eq!(
                copy(source, CopyKind::Source, width, tiny()).0,
                source.as_bytes()
            );
            assert!(work.bytes >= actual.len());
        }
    }
    #[test]
    fn source_utf8_and_control_bytes_are_exact_and_base64_transport_is_paid() {
        let source = "**界**\r\n\u{1b}]52;c;bad\u{7}\n";
        let (actual, work) = copy(source, CopyKind::Source, 20, tiny());
        assert_eq!(actual, source.as_bytes());
        assert!(work.bytes >= source.len() + source.len().div_ceil(3) * 4 + 8);
    }
    #[test]
    fn bounded_copy_cancel_oversize_and_write_failure() {
        let mut doc = Document::new();
        doc.append(&"a".repeat(MAX_CLIPBOARD_BYTES + 1)).unwrap();
        let mut tools = ReadingTools::default();
        tools.selection.cursor = Some(Span::new(0, MAX_CLIPBOARD_BYTES + 1));
        tools.start_copy(&doc, CopyKind::Source, 80);
        assert!(tools.copy.is_none());
        assert!(tools.message.contains("too large"));
        let links = SharedLinks::new(&doc);
        tools.start_input();
        tools.cancel(&links);
        assert!(!tools.pending());
        struct Fail;
        impl Write for Fail {
            fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                Err(io::Error::other("test"))
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let mut task = CopyTask::new(&doc, Span::new(0, 1), CopyKind::Source, 20);
        assert_eq!(
            task.advance(
                &doc,
                &links,
                &mut Budget::new(WorkLimit::unlimited()),
                &mut Fail,
                &mut TableWidthCache::new(&doc)
            ),
            Err("clipboard write failed")
        );
    }
    #[test]
    fn twenty_thousand_guards_share_and_retire_the_single_memo_under_budget() {
        let mut doc = Document::new();
        doc.append(&format!(
            "[![alt](/img) {}](/outer)\n\n",
            (0..20_000).map(|n| format!("[r{n}] ")).collect::<String>()
        ))
        .unwrap();
        doc.finish();
        let links = SharedLinks::new(&doc);
        let mut tables = TableWidthCache::new(&doc);
        tables.set_links(links.clone());
        let mut tools = ReadingTools {
            query: "never".to_owned(),
            ..ReadingTools::default()
        };
        tools.submit(&doc, &links);
        let limit = WorkLimit {
            queries: 100,
            objects: 100,
            bytes: 1024,
            tiles: 4,
            dependencies: 100,
            recovery: 100,
        };
        for _ in 0..100_000 {
            tools.advance(
                &doc,
                &links,
                &mut Budget::new(limit),
                &mut Vec::new(),
                &mut tables,
            );
            if !tools.pending() {
                break;
            }
        }
        assert!(!tools.pending());
        assert!(tools.current.is_none());
        let entries = links.stats().memo_entries;
        assert!(entries >= 20_000);
        tools.cancel(&links);
        assert_eq!(
            links.stats().memo_entries,
            entries,
            "cancel must not drop a history tree"
        );
        assert!(
            !links
                .prepare(&doc, &mut Budget::new(WorkLimit::default()))
                .unwrap()
        );
        assert_eq!(links.stats().memo_entries, entries);
        assert!(!links.prepare(&doc, &mut Budget::new(tiny())).unwrap());
        assert!(links.stats().memo_entries >= entries - 1);
        assert!(links.stats().resolution.retired_entries <= 1);
        for _ in 0..entries + 10 {
            if links.prepare(&doc, &mut Budget::new(tiny())).unwrap() {
                break;
            }
        }
        assert_eq!(links.stats().memo_entries, 0);
    }

    #[test]
    fn full_64k_clipboard_is_resumable_and_dense_hits_do_not_accumulate() {
        let source = "界".repeat(MAX_CLIPBOARD_BYTES / 3);
        assert_eq!(
            copy(&source, CopyKind::Source, 20, tiny()).0,
            source.as_bytes()
        );
        let mut doc = Document::new();
        doc.append(&"a".repeat(100_000)).unwrap();
        doc.finish();
        let links = SharedLinks::new(&doc);
        let mut tables = TableWidthCache::new(&doc);
        let mut tools = ReadingTools {
            query: "aa".to_owned(),
            ..ReadingTools::default()
        };
        tools.submit(&doc, &links);
        for start in 0..1000 {
            while tools.pending() {
                tools.advance(
                    &doc,
                    &links,
                    &mut Budget::new(tiny()),
                    &mut Vec::new(),
                    &mut tables,
                );
            }
            assert_eq!(tools.current.as_ref().unwrap().display, start..start + 2);
            assert!(tools.search.as_ref().unwrap().candidate.is_none());
            tools.navigate(&doc, &links, false);
        }
        tools.cancel(&links);
        assert!(tools.search.is_none());
    }

    #[test]
    #[ignore = "manual 64 MiB source and bounded reading-task acceptance"]
    fn manual_reading_64mib_keeps_small_selection_and_query_work_bounded() {
        let mut doc = Document::new();
        doc.append(&"a".repeat(64 * 1024 * 1024)).unwrap();
        doc.finish();
        let links = SharedLinks::new(&doc);
        let mut tables = TableWidthCache::new(&doc);
        tables.set_links(links.clone());
        let at = 32 * 1024 * 1024 + 17;
        let mut task = CopyTask::new(&doc, Span::new(at, at + 31), CopyKind::Visible, 23);
        let mut packet = Vec::new();
        let mut bytes = 0;
        for _ in 0..1000 {
            let mut budget = Budget::new(WorkLimit {
                queries: 1000,
                objects: 1000,
                bytes: 8192,
                tiles: 8,
                dependencies: 1000,
                recovery: 1000,
            });
            let done = task
                .advance(&doc, &links, &mut budget, &mut packet, &mut tables)
                .unwrap();
            bytes += budget.used().bytes;
            if done {
                break;
            }
        }
        assert_eq!(decode(&packet), b"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        assert!(
            task.layout
                .as_ref()
                .unwrap()
                .scanner()
                .stats()
                .scanned_bytes
                < 128 * 1024
        );
        assert!(bytes < 2 * 1024 * 1024);
        let mut tools = ReadingTools {
            query: "aa".to_owned(),
            ..ReadingTools::default()
        };
        tools.submit(&doc, &links);
        for _ in 0..100 {
            tools.advance(
                &doc,
                &links,
                &mut Budget::new(tiny()),
                &mut Vec::new(),
                &mut tables,
            );
            if !tools.pending() {
                break;
            }
        }
        assert_eq!(tools.current.unwrap().source, Span::new(0, 2));
        eprintln!(
            "64 MiB reading: copy scanned={} work_bytes={bytes}",
            task.layout
                .as_ref()
                .unwrap()
                .scanner()
                .stats()
                .scanned_bytes
        );
    }
}

#[cfg(test)]
mod style_confirmation_tests {
    use super::*;

    fn parsed(source: &str) -> Document {
        let mut doc = Document::new();
        doc.append(source).unwrap();
        doc.finish();
        doc
    }
    fn micro() -> WorkLimit {
        WorkLimit {
            queries: 1,
            objects: 1,
            bytes: 6,
            tiles: 1,
            recovery: 1,
            dependencies: 1,
        }
    }
    fn copy_range(doc: &Document, source: Span, kind: CopyKind) -> (Vec<u8>, usize, u64) {
        copy_range_in_columns(doc, source, kind, None)
    }
    fn copy_range_in_columns(
        doc: &Document,
        source: Span,
        kind: CopyKind,
        column: Option<usize>,
    ) -> (Vec<u8>, usize, u64) {
        let links = SharedLinks::new(doc);
        // One input/adoption pass precedes the per-query source-work meter.
        assert!(
            links
                .prepare_shape(doc, &mut Budget::new(WorkLimit::unlimited()))
                .unwrap()
        );
        assert_eq!(
            links.shape().unwrap().work().source_decode_bytes,
            doc.source().len()
        );
        let mut tables = TableWidthCache::new(doc);
        tables.set_links(links.clone());
        if let Some(column) = column {
            tables.set_policy(
                crate::render::table_widths::TableWidthPolicy::new(3, Some(3)).unwrap(),
            );
            tables
                .set_first_column(doc, mdtui3::TableId(0), column)
                .unwrap();
        }
        let mut tools = ReadingTools::default();
        tools.selection.cursor = Some(source);
        tools.start_copy(doc, kind, 120);
        let before = links.stats();
        let mut wire = Vec::new();
        let mut zero = Budget::new(WorkLimit::default());
        tools.advance(doc, &links, &mut zero, &mut wire, &mut tables);
        assert_eq!(links.stats(), before);
        assert_eq!(zero.used(), WorkLimit::default());
        assert!(wire.is_empty());
        let mut bytes = 0;
        for _ in 0..200_000 {
            let mut budget = Budget::new(micro());
            tools.advance(doc, &links, &mut budget, &mut wire, &mut tables);
            let used = budget.used();
            bytes += used.bytes;
            assert!(used.queries <= 1 && used.objects <= 1 && used.bytes <= 6);
            assert!(used.tiles <= 1 && used.recovery <= 1 && used.dependencies <= 1);
            if !tools.pending() {
                assert_eq!(tools.message(), "clipboard request sent");
                return (
                    super::tests::decode(&wire),
                    bytes,
                    links.stats().styles.events,
                );
            }
            assert!(wire.is_empty(), "no partial clipboard packet");
        }
        panic!("combined reading copy did not finish");
    }

    #[test]
    fn visible_and_raw_reading_copies_use_actual_conditional_scope_and_pair_order() {
        for (body, definitions, expected) in [
            ("*foo [bar* baz]", "", "foo [bar baz]"),
            ("*foo [bar* baz]", "[bar* baz]: /u\n", "*foo bar* baz"),
            (
                "*foo [bar* [kid] baz][outer]",
                "[outer]: /u\n[kid]: /v\n",
                "foo [bar kid baz]outer",
            ),
            ("~~***a***~~_b_ &fjlig;", "", "ab fj"),
        ] {
            let doc = parsed(&format!("{body}\n\n{definitions}"));
            let range = Span::new(0, body.len());
            assert_eq!(
                copy_range(&doc, range, CopyKind::Visible).0,
                expected.as_bytes()
            );
            assert_eq!(copy_range(&doc, range, CopyKind::Source).0, body.as_bytes());
        }
        let doc = parsed(&format!("*{} [x* z]\n\n", "a".repeat(20_000)));
        let (visible, bytes, events) = copy_range(&doc, Span::new(0, 8), CopyKind::Visible);
        assert_eq!(visible, b"aaaaaaa");
        assert!(
            events <= 8,
            "related scope recovery must be shared across glyphs"
        );
        assert!(
            bytes < 20_000,
            "no full paragraph source scan for the small copy: {bytes}"
        );
        println!("conditional reading small copy: bytes={bytes} style_events={events}");
    }

    #[test]
    fn reading_copy_of_confirmed_headers_uses_fresh_cells_and_active_footnote_order() {
        for (old, body, definitions, expected) in [
            ("[prior", "end](u*x*) *x*", "", "end](ux) x"),
            ("$prior", "end$ [^n]", "[^n]: note\n", "end$ [^n]"),
            (
                "`prior",
                "[^early]A`tail[^later]B",
                "[^early]: E\n[^later]: L\n",
                "A`tailB",
            ),
        ] {
            let doc = parsed(&format!(
                "{old}\n{body} {}|b\n-|-\nx|y\n\n{definitions}",
                "z".repeat(1100)
            ));
            assert_eq!(doc.tables().len(), 1);
            let start = doc.tables()[0].cell(0, 0).unwrap().source.start;
            let selected = Span::new(start, start + body.len());
            assert_eq!(
                copy_range(&doc, selected, CopyKind::Visible).0,
                expected.as_bytes()
            );
            assert_eq!(
                copy_range(&doc, selected, CopyKind::Source).0,
                body.as_bytes()
            );
            if old == "$prior" {
                assert!(
                    doc.footnote_reference_state(mdtui3::FootnoteReferenceId(0))
                        .is_none()
                );
            }
            if old == "`prior" {
                let mut refs = doc.footnote_references_in(selected).unwrap();
                let first = refs
                    .advance(&mut Budget::new(WorkLimit::unlimited()))
                    .unwrap()
                    .unwrap();
                let next = refs
                    .advance(&mut Budget::new(WorkLimit::unlimited()))
                    .unwrap()
                    .unwrap();
                assert!(first.0.0 > next.0.0);
                assert_eq!(doc.next_footnote_reference(first.0), Some(next.0));
            }
        }
    }

    #[test]
    fn panned_copy_reuses_prepared_rows_across_a_million_byte_owner_gap() {
        let source = format!(
            "|{}|**tiny**|\n|---|---|\n|a|other|\n",
            "z".repeat(1_000_000)
        );
        let doc = parsed(&source);
        let selected = Span::new(
            source.find("**tiny**").unwrap(),
            source.find("other").unwrap() + 5,
        );
        assert!(selected.len() < 40);
        let (text, bytes, _) = copy_range_in_columns(&doc, selected, CopyKind::Visible, Some(1));
        assert_eq!(text, b"tin\noth");
        assert!(
            bytes < 4096,
            "no row envelope scan or owner-gap prelayout: {bytes}"
        );
        assert_eq!(
            copy_range_in_columns(&doc, selected, CopyKind::Source, Some(1)).0,
            source.as_bytes()[selected.start..selected.end]
        );
        println!(
            "panned copy across owner gap: selection={} work_bytes={bytes}",
            selected.len()
        );
    }

    #[test]
    fn cancelling_reading_search_retires_the_shared_conditional_style_memo_per_paid_node() {
        let marker = "*".repeat(4097);
        let doc = parsed(&format!("{marker}界{marker} [r]\n\n"));
        let links = SharedLinks::new(&doc);
        let mut tables = TableWidthCache::new(&doc);
        tables.set_links(links.clone());
        let mut tools = ReadingTools::default();
        tools.start_input();
        tools.input_key(KeyCode::Char('界'), KeyModifiers::NONE, &doc, &links);
        tools.input_key(KeyCode::Enter, KeyModifiers::NONE, &doc, &links);
        let before = links.stats();
        tools.advance(
            &doc,
            &links,
            &mut Budget::new(WorkLimit::default()),
            &mut Vec::new(),
            &mut tables,
        );
        assert_eq!(links.stats(), before);
        for _ in 0..100_000 {
            let mut budget = Budget::new(micro());
            tools.advance(&doc, &links, &mut budget, &mut Vec::new(), &mut tables);
            assert!(
                budget.used().queries <= 1
                    && budget.used().objects <= 1
                    && budget.used().bytes <= 6
            );
            if links.stats().style_memo_entries > 128 {
                break;
            }
        }
        assert!(links.stats().style_memo_entries > 128);
        tools.cancel(&links);
        let before = links.stats();
        assert!(
            !links
                .prepare(&doc, &mut Budget::new(WorkLimit::default()))
                .unwrap()
        );
        assert_eq!(links.stats(), before);
        let mut paid = 0;
        loop {
            let mut budget = Budget::new(micro());
            let done = links.prepare(&doc, &mut budget).unwrap();
            paid += budget.used().objects;
            assert!(budget.used().objects <= 1 && budget.used().queries <= 1);
            if done {
                break;
            }
        }
        assert_eq!(paid, before.memo_entries + before.presentation_entries);
        assert_eq!(links.stats().memo_entries, 0);
        println!("reading conditional cancellation: retired={paid}");
    }
    #[test]
    fn hidden_mathematical_atom_copies_the_visible_placeholder_before_any_tex_preparation() {
        let doc = parsed("\\(\\frac{SECRET}{b}\\)\n");
        let links = SharedLinks::new(&doc);
        let mut tables = TableWidthCache::new(&doc);
        tables.set_links(links.clone());
        let mut owner = crate::render::checkpoint::RenderCheckpointStore::new(&doc);
        let source = Span::new(0, doc.source().len() - 1);
        let view = owner.view_mut().unwrap();
        let mut registration =
            mdtui3::SourceFoldRegistration::overlapping(&doc, view, source).unwrap();
        let id = registration
            .advance(&doc, view, &mut Budget::new(WorkLimit::unlimited()))
            .unwrap()
            .id
            .unwrap();
        view.set_collapsed(&doc, id, true).unwrap();
        let mut copy = CopyTask::new(&doc, source, CopyKind::Visible, 80);
        copy.fold_version = Some(view.version());
        let mut packet = Vec::new();
        let mut complete = false;
        for _ in 0..10000 {
            let mut budget = Budget::new(micro());
            if copy
                .advance_with_view(
                    &doc,
                    &links,
                    &mut budget,
                    &mut packet,
                    &mut tables,
                    Some(&mut owner),
                )
                .unwrap()
            {
                complete = true;
                break;
            }
            assert!(
                budget.used().queries <= 1
                    && budget.used().objects <= 1
                    && budget.used().bytes <= 6
            );
        }
        assert!(complete);
        let text = String::from_utf8(super::tests::decode(&packet)).unwrap();
        assert!(text.contains("[+ source"), "{text}");
        assert!(!text.contains("SECRET") && !text.contains("frac"), "{text}");
        assert_eq!(links.math().retained_owners(), 0);
    }

    #[test]
    fn math_copy_pages_keep_every_structural_row_and_exact_raw_source() {
        let labels = (0..40).map(|i| format!("r{i}")).collect::<Vec<_>>();
        let source = format!(
            "\\[\n\\begin{{matrix}}{}\\end{{matrix}}\n\\]\n",
            labels.join("\\\\")
        );
        let doc = parsed(&source);
        let selected = Span::new(0, source.len());
        let (visible, _, _) = copy_range(&doc, selected, CopyKind::Visible);
        assert_eq!(visible, labels.join("\n\n").as_bytes());
        assert_eq!(
            copy_range(&doc, selected, CopyKind::Source).0,
            source.as_bytes()
        );
        let source = "A \\(\\frac{a}{b}\\) Z\n";
        let doc = parsed(source);
        assert_eq!(
            copy_range(&doc, Span::new(0, source.len()), CopyKind::Visible).0,
            "a\nA ─── Z\nb".as_bytes()
        );
    }
}
