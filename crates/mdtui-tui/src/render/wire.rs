//! A finite diff descriptor and resumable, byte-credit-limited terminal writer.
//! Payloads remain shared handles; only one scalar can be staged ahead of ACK.
use super::{
    diff::ScreenDiff,
    output::{OutputOptions, OutputStats, segment_boundaries, validate_diff},
    payload::{PayloadReader, PayloadStep, TextPayload},
};
use crate::screen::{Cell, Screen};
use mdtui3::{Budget, Document, WorkLimit};
use std::{
    collections::{HashMap, VecDeque},
    io::{self, Write},
};

pub(crate) const CLOSE: &[u8] = b"\x18\x1b\\\x1b]8;;\x1b\\\x1b[0m\x1b[?2026l";
const CLOSE_NO_LINKS: &[u8] = b"\x18\x1b\\\x1b[0m\x1b[?2026l";
pub(crate) fn cleanup(options: OutputOptions) -> &'static [u8] {
    if options.allow_osc8 && options.link_mode != crate::cli::LinkMode::NoOsc8 {
        CLOSE
    } else {
        CLOSE_NO_LINKS
    }
}
#[derive(Clone)]
pub(crate) struct OpenCell {
    pub row: u16,
    pub column: u16,
    pub cell: Cell,
}
enum Item {
    Control(Vec<u8>),
    Text(TextPayload),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Progress {
    Pending,
    OpenCut,
    Complete,
}
pub(crate) struct Transaction {
    items: VecDeque<Item>,
    reader: Option<PayloadReader>,
    control_at: usize,
    pending: [u8; 8],
    at: usize,
    end: usize,
    pub open: Option<OpenCell>,
    pub closed: Option<OpenCell>,
    tail: Vec<u8>,
    reported: bool,
    cancel: bool,
    closing: bool,
    cleanup: &'static [u8],
    pub stats: OutputStats,
    pub bytes: usize,
    pub cancelled: bool,
}
impl Transaction {
    pub fn prepare(
        document: &Document,
        screen: &Screen,
        diff: &ScreenDiff,
        options: OutputOptions,
        clear: bool,
        open: Option<OpenCell>,
    ) -> io::Result<Self> {
        validate_diff(screen, diff)?;
        let mut tx = Self {
            items: VecDeque::new(),
            reader: None,
            control_at: 0,
            pending: [0; 8],
            at: 0,
            end: 0,
            open: None,
            closed: None,
            tail: Vec::new(),
            reported: false,
            cancel: false,
            closing: false,
            cleanup: cleanup(options),
            stats: OutputStats::default(),
            bytes: 0,
            cancelled: false,
        };
        let mut destinations = HashMap::new();
        if clear {
            tx.control(super::frame::FIRST_FRAME_CLEAR.to_vec());
        }
        let has_output = diff
            .runs
            .iter()
            .any(|r| r.cells.iter().any(|c| !c.is_continuation()));
        if has_output {
            tx.control(super::frame::BEGIN_SYNCHRONIZED_UPDATE.to_vec());
        }
        for run in &diff.runs {
            let mut column = run.start_col;
            let mut group: Vec<Cell> = Vec::new();
            let mut group_start = column;
            for cell in &run.cells {
                if cell.is_continuation() {
                    tx.stats.continuations += 1;
                    continue;
                }
                let held = open.as_ref().is_some_and(|o| {
                    o.row == run.row && o.column == column && o.cell.width() == cell.width()
                });
                let changed = group.last().is_some_and(|old| {
                    old.style() != cell.style() || old.link_id() != cell.link_id()
                });
                if (held || changed) && !group.is_empty() {
                    tx.group(
                        document,
                        run.row,
                        group_start,
                        &group,
                        options,
                        &mut destinations,
                        false,
                    );
                    group.clear();
                }
                if !held {
                    if group.is_empty() {
                        group_start = column;
                    }
                    group.push(cell.clone());
                }
                column += u16::from(cell.width());
            }
            if !group.is_empty() {
                tx.group(
                    document,
                    run.row,
                    group_start,
                    &group,
                    options,
                    &mut destinations,
                    false,
                );
            }
        }
        if let Some(open) = open {
            // Finish the ordinary frame before starting the open EGC. A
            // terminal may now display its growing glyph without a sync hold.
            if has_output {
                tx.control(super::frame::END_SYNCHRONIZED_UPDATE.to_vec());
            }
            tx.group(
                document,
                open.row,
                open.column,
                std::slice::from_ref(&open.cell),
                options,
                &mut destinations,
                true,
            );
            tx.open = Some(open);
        }
        if has_output && tx.open.is_none() {
            tx.control(super::frame::END_SYNCHRONIZED_UPDATE.to_vec());
        }
        Ok(tx)
    }
    fn control(&mut self, bytes: Vec<u8>) {
        if !bytes.is_empty() {
            self.items.push_back(Item::Control(bytes));
        }
    }
    #[allow(clippy::too_many_arguments)]
    fn group(
        &mut self,
        document: &Document,
        row: u16,
        column: u16,
        cells: &[Cell],
        options: OutputOptions,
        destinations: &mut HashMap<mdtui3::LinkId, Option<String>>,
        hold: bool,
    ) {
        let cell = &cells[0];
        let (prefix, suffix, linked) = segment_boundaries(
            document,
            cell.style(),
            cell.link_id(),
            options,
            &mut self.stats,
            destinations,
        );
        self.control(
            format!("\x1b[{};{}H", u32::from(row) + 1, u32::from(column) + 1).into_bytes(),
        );
        self.control(prefix.into_bytes());
        for cell in cells {
            self.items.push_back(Item::Text(cell.text().clone()));
        }
        if hold {
            self.tail.extend_from_slice(suffix.as_bytes())
        } else {
            self.control(suffix.into_bytes())
        }
        self.stats.runs += 1;
        self.stats.segments += 1;
        self.stats.cells += cells.len();
        self.stats.osc8_links += usize::from(linked);
    }
    /// Extend only after proving that the already acknowledged logical prefix
    /// still has identical source, binding, style and terminal position.
    pub fn extend(&mut self, next: OpenCell, budget: &mut Budget) -> bool {
        let Some(old) = &self.open else { return false };
        if old.row != next.row
            || old.column != next.column
            || old.cell.style() != next.cell.style()
            || old.cell.link_id() != next.cell.link_id()
        {
            return false;
        }
        if old.cell.text() == next.cell.text() {
            return true;
        }
        if !budget.charge(WorkLimit {
            queries: 1,
            objects: 1,
            ..Default::default()
        }) {
            return false;
        }
        let Some(suffix) = next.cell.text().suffix_after(old.cell.text()) else {
            return false;
        };
        if !suffix.is_empty() {
            self.items.push_back(Item::Text(suffix));
            self.reported = false;
        }
        self.open = Some(next);
        true
    }
    pub fn report_open_cut(&mut self) {
        if self.open.is_some() && self.items.is_empty() && self.at == self.end {
            self.reported = false;
        }
    }
    pub fn close(&mut self) {
        self.closed = self.open.take();
        let tail = std::mem::take(&mut self.tail);
        self.control(tail);
        self.reported = false;
    }
    pub fn cancel(&mut self) {
        if self.cancel || self.closing {
            return;
        }
        self.cancelled = true;
        self.cancel = true;
        // An entity atom can contain several scalars. Finish only the scalar
        // whose prefix is already on the wire, not the rest of that atom.
        let mut boundary = self.at;
        while boundary < self.end && std::str::from_utf8(&self.pending[..boundary]).is_err() {
            boundary += 1;
        }
        self.end = boundary;
        self.open = None;
        self.closed = None;
        self.tail.clear();
    }
    pub fn pending(&self) -> bool {
        !self.items.is_empty() || self.at < self.end || self.open.is_none()
    }
    pub fn advance<W: Write + ?Sized>(
        &mut self,
        writer: &mut W,
        budget: &mut Budget,
    ) -> io::Result<Progress> {
        loop {
            if self.cancel
                && (self.at == 0
                    || self.at == self.end
                    || std::str::from_utf8(&self.pending[..self.at]).is_ok())
            {
                self.items.clear();
                self.reader = None;
                self.at = 0;
                self.end = 0;
                self.control_at = 0;
                self.control(self.cleanup.to_vec());
                self.cancel = false;
                self.closing = true;
            }
            if self.at < self.end {
                let count = (self.end - self.at).min(budget.remaining().bytes);
                if count == 0 {
                    return Ok(Progress::Pending);
                }
                let written = match writer.write(&self.pending[self.at..self.at + count]) {
                    Ok(0) => {
                        return Err(io::Error::new(
                            io::ErrorKind::WriteZero,
                            "terminal write returned zero",
                        ));
                    }
                    Ok(n) => n,
                    Err(e)
                        if matches!(
                            e.kind(),
                            io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                        ) =>
                    {
                        return Ok(Progress::Pending);
                    }
                    Err(e) => return Err(e),
                };
                assert!(budget.charge(WorkLimit {
                    bytes: written,
                    ..Default::default()
                }));
                self.at += written;
                self.bytes += written;
                self.stats.bytes += written;
                continue;
            }
            self.at = 0;
            self.end = 0;
            match self.items.front() {
                Some(Item::Control(bytes)) => {
                    if self.control_at == bytes.len() {
                        self.items.pop_front();
                        self.control_at = 0;
                        continue;
                    }
                    if budget.remaining().bytes == 0 {
                        return Ok(Progress::Pending);
                    }
                    let mut count = (bytes.len() - self.control_at).min(self.pending.len());
                    // Control strings (notably OSC8 destinations) are UTF-8
                    // too. Stage complete scalars so cancellation can finish
                    // at most three tail bytes, even across chunk boundaries.
                    while self.control_at + count < bytes.len()
                        && bytes[self.control_at + count] & 0xc0 == 0x80
                    {
                        count -= 1;
                    }
                    self.pending[..count]
                        .copy_from_slice(&bytes[self.control_at..self.control_at + count]);
                    self.end = count;
                    self.control_at += count;
                }
                Some(Item::Text(payload)) => {
                    let reader = self.reader.get_or_insert_with(|| payload.reader());
                    let step = match reader.next(budget) {
                        Ok(step) => step,
                        Err(mdtui3::LayoutError::StaleContinuation) => {
                            self.cancel();
                            continue;
                        }
                        Err(e) => {
                            return Err(io::Error::new(io::ErrorKind::InvalidData, e.to_string()));
                        }
                    };
                    match step {
                        PayloadStep::BudgetExhausted => return Ok(Progress::Pending),
                        PayloadStep::Complete => {
                            self.reader = None;
                            self.items.pop_front();
                        }
                        PayloadStep::Bytes { mut utf8, len } => {
                            if len == 1 && utf8[0] == b'\t' {
                                utf8[0] = b' ';
                            }
                            self.pending = utf8;
                            self.end = usize::from(len);
                        }
                    }
                }
                None => {
                    if self.open.is_none() {
                        return Ok(Progress::Complete);
                    }
                    if !self.reported {
                        self.reported = true;
                        return Ok(Progress::OpenCut);
                    }
                    return Ok(Progress::Pending);
                }
            }
        }
    }
}
