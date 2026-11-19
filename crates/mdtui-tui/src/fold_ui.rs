//! One view's concrete keyboard menu and paid fold operations.
use crate::render::text::{display_width, fit_prefix};
use crate::render::{checkpoint::RenderCheckpoint, prefix::PrefixLines};
use crossterm::event::KeyCode;
use mdtui3::{
    Budget, Completion, ContainerFoldFilter, Document, FoldBatch, FoldBatchAction, FoldCandidate,
    FoldId, FoldView, LayoutError, SourceFoldCandidates, SourceFoldRegistration, Span,
    TableRowAutoFold, TableRowCandidateStep, TableRowCandidates, TableRowEdit, TableRowExtent,
    TableRowFoldInfo, TableRowPolicy, TableRowRange, TableRowRegistration, WorkLimit,
};
use std::collections::VecDeque;

type View = FoldView<RenderCheckpoint>;
const EMPTY_TARGET_MESSAGE: &str = "No fold target yet; Esc to resume input";

#[derive(Clone, Copy)]
enum Command {
    Toggle,
    Open,
    Close,
    Parent,
    NextRange,
    NextAnyRange,
    RowTarget,
    Preview,
    EndPreview,
    Summary,
    Subtree(bool),
    Siblings(bool),
    All(bool),
    Level(u8, bool),
    Focus,
    Containers(ContainerFoldFilter, bool),
    Default(u8),
    Overlay,
    Strict,
    Stream,
    Seal,
    Remove,
    Rows,
    Paragraph,
    ParagraphLines(PrefixLines),
    DefaultLines(PrefixLines),
    TablePrefix(usize),
    TablePrefixInput,
    TableDefault(Option<usize>),
    TableDefaultInput,
    TableOverlay,
    TableLive,
    TableSeal,
    TableRetarget,
    TableRemove,
    TableNext(bool),
}

enum Operation {
    Command(Command),
    Batch(Box<FoldBatch>),
    Register(SourceFoldRegistration, bool),
    Collapse(FoldId),
    Preview(Option<FoldId>),
    Ranges(SourceFoldCandidates),
    Summary(Box<super::render::fold_summary::SummaryJob>),
    PublishSummary(super::render::fold_summary::FoldSummary),
    TableRegister(TableRowRegistration, bool),
    TableEdit(TableRowEdit, bool),
    TableRanges(TableRowCandidates),
    TablePolicy(Option<TableRowPolicy>),
}

#[derive(Default)]
pub struct FoldUi {
    prefixes: crate::render::prefix_policy::ParagraphPrefixes,
    auto: Option<mdtui3::ParagraphAutoFold>,
    auto_range: Option<Span>,
    menu: bool,
    index: usize,
    page: usize,
    source: usize,
    selection: Option<Span>,
    target: Option<FoldId>,
    operation: Option<Operation>,
    entries: Vec<(String, Command)>,
    range_next: Option<SourceFoldCandidates>,
    range_all: Option<bool>,
    message: String,
    failed: bool,
    summary: Option<super::render::fold_summary::FoldSummary>,
    changed: bool,
    row_auto: VecDeque<TableRowAutoFold>,
    row_retiring: usize,
    table_range_next: Option<TableRowCandidates>,
    table_range_all: Option<bool>,
    table_identity: Option<String>,
    number_input: Option<(bool, String)>,
}

impl FoldUi {
    pub(crate) fn suspend(&mut self, budget: &mut Budget) -> bool {
        if !self.prefixes.suspend(budget) {
            return false;
        }
        self.operation = None;
        self.menu = false;
        self.range_next = None;
        self.range_all = None;
        self.auto = None;
        self.auto_range = None;
        self.summary = None;
        self.table_range_next = None;
        self.table_range_all = None;
        self.number_input = None;
        debug_assert!(self.row_auto.is_empty());
        self.row_auto = VecDeque::new();
        self.row_retiring = 0;
        true
    }
    pub(crate) fn resume(&mut self, doc: &Document, paragraph_auto: bool) {
        self.prefixes.resume(doc);
        self.changed = false;
        if paragraph_auto && self.prefixes.default.is_none() {
            self.auto_range = Some(Span::new(0, doc.source().len()));
        }
    }
    pub(crate) fn has_policies(&self) -> bool {
        self.prefixes.has_policies()
    }
    pub fn end_preview(&mut self) {
        self.operation = Some(Operation::Preview(None));
    }
    fn current_table(&self, doc: &Document, view: &View) -> Result<mdtui3::TableId, LayoutError> {
        match self.target {
            Some(FoldId::Table(table)) => Ok(table),
            Some(FoldId::TableRows(rows)) => Ok(rows.table),
            Some(FoldId::TableRange(id)) => view
                .table_row_fold_info(doc, id)?
                .map(|i| i.table)
                .ok_or(LayoutError::InvalidRange),
            _ => doc.table_at(self.source).ok_or(LayoutError::InvalidRange),
        }
    }
    pub fn activate(&mut self, id: FoldId) {
        self.target = Some(id);
        self.operation = Some(Operation::Command(Command::Open));
    }
    pub fn active(&self) -> bool {
        self.menu || self.pending()
    }
    pub fn pending(&self) -> bool {
        self.operation.is_some()
            || self.auto.is_some()
            || self.auto_range.is_some()
            || self.prefixes.pending()
            || !self.row_auto.is_empty()
    }
    pub fn on_update(
        &mut self,
        doc: &Document,
        view: &View,
        update: &mdtui3::Update,
    ) -> Result<(), LayoutError> {
        self.prefixes.on_update(doc, view, update)?;
        self.table_range_next = None;
        if view.table_row_policy().is_some() && !update.new_tables.is_empty() {
            self.row_auto
                .push_back(TableRowAutoFold::for_update(doc, view, update)?);
        }
        if self.prefixes.default.is_some() {
            return Ok(());
        }
        let old = self
            .auto
            .take()
            .map(|task| task.remaining_range())
            .or(self.auto_range.take());
        if view.defaults().paragraphs.is_none() {
            return Ok(());
        }
        let new = mdtui3::ParagraphAutoFold::for_update(doc, view, update)?.remaining_range();
        self.auto_range = Some(old.map_or(new, |old| {
            Span::new(old.start.min(new.start), old.end.max(new.end))
        }));
        Ok(())
    }
    pub fn status(&self, width: u16) -> Option<String> {
        let width = usize::from(width);
        let status = if let Some((defaults, text)) = &self.number_input {
            if self.failed {
                "N 0..1000000 Esc".into()
            } else {
                let full = format!(
                    "{} N={text} Enter Esc",
                    if *defaults { "New" } else { "Table" }
                );
                if display_width(&full) <= width {
                    full
                } else {
                    format!("{} N={text} ↵ Esc", if *defaults { "New" } else { "T" })
                }
            }
        } else if self.page == 6
            && self.menu
            && self.operation.is_none()
            && !self.failed
            && let Some(identity) = &self.table_identity
        {
            let end = fit_prefix(identity, width.saturating_sub(4)).end;
            if end < identity.len() {
                let end = fit_prefix(identity, width.saturating_sub(5)).end;
                format!("{}… Esc", &identity[..end])
            } else {
                format!("{identity} Esc")
            }
        } else if self.operation.is_some() {
            format!("Esc cancel | {}", self.message)
        } else if self.menu && self.message == EMPTY_TARGET_MESSAGE {
            if width < EMPTY_TARGET_MESSAGE.len() {
                "No target Esc=resume".into()
            } else {
                self.message.clone()
            }
        } else if self.menu && self.failed {
            format!("{} | Esc", self.message)
        } else if let Some((label, command)) =
            self.menu.then(|| self.entries.get(self.index)).flatten()
        {
            let detailed = format!(
                "{}: {} ({}/{})",
                [
                    "node",
                    "document",
                    "range",
                    "level",
                    "container",
                    "new defaults",
                    "table rows"
                ][self.page],
                label,
                self.index + 1,
                self.entries.len(),
            );
            // Keep the whole action before spending cells on context or help.
            // Finite short labels retain the type, level and prefix mode.
            let help = [
                " | ↑↓ Enter Esc ←→",
                " Enter Esc",
                " ↑↓ ↵ Esc ←→",
                " ↑↓ ↵ Esc",
                " ↵ Esc",
                " Esc",
            ];
            let mut status = if display_width(&detailed) + display_width(help[0]) <= width {
                detailed
            } else if display_width(label) + display_width(" Esc") <= width {
                label.clone()
            } else {
                compact_label(label, *command)
            };
            for help in help {
                if width < 32 && help == " Enter Esc" {
                    continue;
                }
                if display_width(&status) + display_width(help) <= width {
                    status.push_str(help);
                    break;
                }
            }
            let message = format!(" | {}", self.message);
            if display_width(&status) + display_width(&message) <= width {
                status.push_str(&message);
            }
            let table_help = " t:rows";
            if self.page != 6 && display_width(&status) + display_width(table_help) <= width {
                status.push_str(table_help);
            }
            status
        } else if self.pending() {
            format!("fold: {} | Esc cancel", self.message)
        } else {
            return None;
        };
        Some(status[..fit_prefix(&status, width).end].into())
    }
    pub fn open(&mut self, source: usize, selection: Option<Span>, target: Option<FoldId>) {
        self.menu = true;
        self.index = 0;
        self.page = 0;
        self.source = source;
        self.selection = selection;
        self.target = target;
        self.entries = entries(self.page);
        self.message = "reader paused".into();
        self.failed = false;
        self.range_next = None;
        self.range_all = None;
        self.table_range_next = None;
        self.table_range_all = None;
        self.table_identity = None;
        self.number_input = None;
        if self.target.is_none() {
            self.operation = Some(Operation::Command(Command::Parent));
        }
    }
    pub fn key(&mut self, code: KeyCode) -> bool {
        if !self.active() {
            return false;
        }
        if let Some((defaults, text)) = &mut self.number_input {
            match code {
                KeyCode::Char(c) if c.is_ascii_digit() && text.len() < 7 => {
                    text.push(c);
                    self.failed = false;
                }
                KeyCode::Backspace => {
                    text.pop();
                    self.failed = false;
                }
                KeyCode::Esc => {
                    self.number_input = None;
                }
                KeyCode::Enter => {
                    if let Ok(n) = text.parse::<usize>()
                        && n <= 1_000_000
                    {
                        let command = if *defaults {
                            Command::TableDefault(Some(n))
                        } else {
                            Command::TablePrefix(n)
                        };
                        self.number_input = None;
                        self.operation = Some(Operation::Command(command));
                    } else {
                        self.message = "N must be 0..1000000".into();
                        self.failed = true;
                    }
                }
                KeyCode::Char('q') => return false,
                _ => {}
            }
            return true;
        }
        if self.operation.is_some() && !matches!(code, KeyCode::Esc | KeyCode::Char('q' | 'c')) {
            return true;
        }
        if self.menu
            && (self.failed || self.message == EMPTY_TARGET_MESSAGE)
            && matches!(
                code,
                KeyCode::Up
                    | KeyCode::Down
                    | KeyCode::Left
                    | KeyCode::Right
                    | KeyCode::Home
                    | KeyCode::End
            )
        {
            self.message = "reader paused".into();
            self.failed = false;
        }
        match code {
            KeyCode::Esc => {
                self.menu = false;
                self.operation = None;
                self.range_next = None;
                self.auto = None;
                self.auto_range = None;
                self.prefixes.cancel();
                self.row_retiring = self.row_auto.len();
                self.table_range_next = None;
                self.table_identity = None;
                self.message = if self.prefixes.validation_pending() {
                    "menu closed; current prefix validation continues; q exits".into()
                } else {
                    "cancelled; completed choices retained".into()
                };
            }
            KeyCode::Up if self.menu => {
                self.table_identity = None;
                self.index = self.index.saturating_sub(1);
            }
            KeyCode::Down if self.menu => {
                self.table_identity = None;
                self.index = (self.index + 1).min(self.entries.len().saturating_sub(1));
            }
            KeyCode::Left | KeyCode::Right if self.menu => {
                self.table_identity = None;
                self.page = if code == KeyCode::Right {
                    if self.page == 6 {
                        0
                    } else {
                        (self.page + 1) % 6
                    }
                } else {
                    if self.page == 6 {
                        0
                    } else {
                        (self.page + 5) % 6
                    }
                };
                self.entries = entries(self.page);
                self.index = 0;
            }
            KeyCode::Char('t') if self.menu => {
                self.page = 6;
                self.index = 0;
                self.entries = entries(6);
                self.table_identity = None;
            }
            KeyCode::Home if self.menu => {
                self.index = 0;
                self.table_identity = None;
            }
            KeyCode::End if self.menu => {
                self.index = self.entries.len().saturating_sub(1);
                self.table_identity = None;
            }
            KeyCode::Enter if self.menu && self.operation.is_none() => {
                self.failed = false;
                self.operation = Some(Operation::Command(self.entries[self.index].1));
            }
            KeyCode::Char('q') | KeyCode::Char('c') => return false,
            _ => {}
        }
        true
    }
    pub fn prefix_validation_pending(&self) -> bool {
        self.prefixes.validation_pending()
    }
    pub fn prefix_query_starts(&self) -> usize {
        self.prefixes.query_starts()
    }
    pub fn resize(&mut self) {
        self.prefixes.resize();
        self.table_range_next = None;
    }
    pub fn take_changed(&mut self) -> bool {
        std::mem::take(&mut self.changed)
    }

    pub(crate) fn take_summary(&mut self) -> Option<super::render::fold_summary::FoldSummary> {
        self.summary.take()
    }
    pub(crate) fn settle_table_rows(
        &mut self,
        doc: &Document,
        view: &mut View,
        budget: &mut Budget,
    ) -> bool {
        if self.row_retiring > 0 {
            if charge(budget) {
                self.row_auto.pop_front();
                self.row_retiring -= 1;
            }
            self.message = "cancel table work; paid choices kept".into();
            return false;
        }
        if let Some(task) = self.row_auto.front_mut() {
            let old = view.version();
            let result = task
                .rebind(doc, view)
                .and_then(|()| task.advance(doc, view, budget));
            match result {
                Ok(p) => {
                    self.message = "new table row policy".into();
                    if p.completion == Completion::Covered {
                        self.row_auto.pop_front();
                    }
                }
                Err(e) => {
                    self.message = e.to_string();
                    self.failed = true;
                    self.row_retiring = self.row_auto.len();
                }
            }
            self.changed |= old != view.version();
            return false;
        }
        self.row_auto.is_empty()
    }
    pub fn advance(
        &mut self,
        doc: &Document,
        view: &mut View,
        links: &super::render::objects::SharedLinks,
        width: u16,
        budget: &mut Budget,
    ) {
        let Some(mut operation) = self.operation.take() else {
            if self.row_retiring > 0 || !self.row_auto.is_empty() {
                self.settle_table_rows(doc, view, budget);
                return;
            }
            if self.prefixes.pending() {
                let old = view.version();
                if let Err(error) = self.prefixes.advance(doc, view, links, width, budget) {
                    self.message = error;
                    self.failed = true;
                    self.prefixes.cancel();
                } else {
                    self.message = self.prefixes.message.clone();
                }
                if let Some(id) = self.prefixes.selected.take() {
                    self.target = Some(id);
                }
                self.changed |= old != view.version();
                return;
            }
            if self.auto.is_none()
                && let Some(range) = self.auto_range.take()
            {
                match mdtui3::ParagraphAutoFold::new(doc, view, range) {
                    Ok(task) => self.auto = Some(task),
                    Err(error) => {
                        self.message = error.to_string();
                        self.failed = true;
                    }
                }
            }
            if let Some(task) = &mut self.auto {
                let old = view.version();
                match task.advance(doc, view, budget) {
                    Ok(progress) => {
                        self.message = format!(
                            "paragraphs: {} inspected, {} registered",
                            progress.inspected, progress.registered
                        );
                        if progress.completion != Completion::BudgetExhausted {
                            self.auto = None;
                        }
                    }
                    Err(error) => {
                        self.message = error.to_string();
                        self.failed = true;
                        self.auto_range = Some(task.remaining_range());
                        self.auto = None;
                    }
                }
                self.changed |= old != view.version();
            }
            return;
        };
        let old = view.version();
        let result = self.advance_operation(doc, view, links, width, budget, &mut operation);
        self.changed |= old != view.version();
        if old != view.version() {
            self.table_range_next = None;
        }
        if old != view.version()
            && let Some(task) = self.auto.take()
        {
            self.auto_range = Some(task.remaining_range());
        }
        match result {
            Ok(false) => self.operation = Some(operation),
            Ok(true) => {}
            Err(error) => {
                self.failed = true;
                self.message = if error == LayoutError::InvalidRange
                    && doc.source().is_empty()
                    && self.target.is_none()
                {
                    EMPTY_TARGET_MESSAGE.into()
                } else {
                    error.to_string()
                };
                self.range_next = None;
            }
        }
    }

    fn advance_operation(
        &mut self,
        doc: &Document,
        view: &mut View,
        links: &super::render::objects::SharedLinks,
        width: u16,
        budget: &mut Budget,
        operation: &mut Operation,
    ) -> Result<bool, LayoutError> {
        match operation {
            Operation::TablePolicy(policy) => {
                if !view.set_table_row_policy(doc, *policy, budget)? {
                    return Ok(false);
                }
                self.table_identity = Some(policy.map_or_else(
                    || "new rows off".into(),
                    |p| format!("new rows={}", p.visible_body_rows),
                ));
                self.message = policy.map_or_else(
                    || "new table row policy off".into(),
                    |p| format!("new tables: first {} body rows", p.visible_body_rows),
                );
                Ok(true)
            }
            Operation::TableRegister(task, collapse) => {
                let p = task.advance(doc, view, budget)?;
                if p.completion == Completion::BudgetExhausted {
                    return Ok(false);
                }
                self.target = p.id;
                if let Some(FoldId::TableRange(id)) = p.id {
                    self.table_identity = view.table_row_fold_info(doc, id)?.map(table_identity);
                    if *collapse {
                        *operation = Operation::Collapse(FoldId::TableRange(id));
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            Operation::TableEdit(task, remove) => {
                let p = task.advance(doc, view, budget)?;
                if p.completion == Completion::BudgetExhausted {
                    return Ok(false);
                }
                if *remove {
                    self.target = None;
                    self.table_identity = None;
                    self.message = "table range removed".into();
                } else if let Some(FoldId::TableRange(id)) = p.id {
                    self.target = p.id;
                    self.table_identity = view.table_row_fold_info(doc, id)?.map(table_identity);
                }
                Ok(true)
            }
            Operation::TableRanges(task) => match task.next(doc, view, budget)? {
                TableRowCandidateStep::BudgetExhausted => Ok(false),
                TableRowCandidateStep::Complete => {
                    self.table_identity = None;
                    self.message = "no more registered table IDs".into();
                    Ok(true)
                }
                TableRowCandidateStep::Candidate(info) => {
                    self.target = Some(FoldId::TableRange(info.id));
                    let placeholder = TableRowCandidates::all(doc, view, info.table)?;
                    self.table_identity = Some(table_identity(info));
                    self.table_range_next = Some(std::mem::replace(task, placeholder));
                    Ok(true)
                }
            },
            Operation::PublishSummary(summary) => {
                summary.check_binding(doc, view)?;
                if !charge(budget) {
                    return Ok(false);
                }
                let empty = Operation::Command(Command::Toggle);
                let Operation::PublishSummary(summary) = std::mem::replace(operation, empty) else {
                    unreachable!()
                };
                self.message = format!(
                    "summary {}..{} ready; {} source work bytes",
                    summary.source.start, summary.source.end, summary.charged_source_bytes
                );
                self.summary = Some(summary);
                Ok(true)
            }
            Operation::Summary(task) => {
                if let Some(summary) = task.advance(doc, view, links, budget)? {
                    *operation = Operation::PublishSummary(summary);
                    return Ok(false);
                }
                self.message = "summary pending; source ≤256B, display ≤128B".into();
                Ok(false)
            }
            Operation::Batch(task) => {
                let progress = task.advance(doc, view, budget)?;
                self.message =
                    format!("{} visited, {} changed", progress.visited, progress.changed);
                Ok(progress.completion != Completion::BudgetExhausted)
            }
            Operation::Register(task, collapse) => {
                let progress = task.advance(doc, view, budget)?;
                self.message = format!("range validation: {} ancestors", progress.visited);
                if progress.completion == Completion::BudgetExhausted {
                    return Ok(false);
                }
                if let Some(id) = progress.id {
                    self.target = Some(id);
                    if *collapse {
                        *operation = Operation::Collapse(id);
                        return Ok(false);
                    }
                    self.message = "sealed; selected choice preserved".into();
                }
                Ok(true)
            }
            Operation::Collapse(id) => {
                if !charge(budget) {
                    return Ok(false);
                }
                view.set_collapsed(doc, *id, true)?;
                self.message = "registered and collapsed".into();
                Ok(true)
            }
            Operation::Preview(id) => {
                let progress = view.set_preview(doc, *id, budget)?;
                self.message = "preview; saved choices retained".into();
                Ok(progress.completion != Completion::BudgetExhausted)
            }
            Operation::Ranges(task) => {
                // At most one candidate per scheduling unit. Keep the cursor
                // between menu activations to disambiguate same-start ranges.
                let mut one = Budget::new(WorkLimit {
                    queries: 1,
                    objects: 1,
                    ..Default::default()
                });
                if !charge(budget) {
                    return Ok(false);
                }
                let mut candidates = Vec::new();
                let completion = task.advance(doc, view, &mut one, &mut candidates)?;
                if let Some(candidate) = candidates.first() {
                    self.target = Some(candidate.id);
                    self.message = candidate_label(*candidate);
                    if completion == Completion::BudgetExhausted {
                        let placeholder = SourceFoldCandidates::new(doc, view, Span::new(0, 0))?;
                        self.range_next = Some(std::mem::replace(task, placeholder));
                    }
                    return Ok(true);
                }
                Ok(completion != Completion::BudgetExhausted)
            }
            Operation::Command(command) => {
                if !charge(budget) {
                    return Ok(false);
                }
                let target = self.target;
                match *command {
                    Command::TablePrefixInput | Command::TableDefaultInput => {
                        self.number_input =
                            Some((matches!(command, Command::TableDefaultInput), String::new()));
                        self.table_identity = None;
                    }
                    Command::TableDefault(n) => {
                        *operation = Operation::TablePolicy(n.map(|n| TableRowPolicy {
                            visible_body_rows: n,
                        }));
                        return Ok(false);
                    }
                    Command::TablePrefix(n) => {
                        let table = self.current_table(doc, view)?;
                        *operation = Operation::TableRegister(
                            TableRowRegistration::prefix(doc, view, table, n)?,
                            false,
                        );
                        return Ok(false);
                    }
                    Command::TableOverlay | Command::TableRetarget => {
                        let (table, rows) = selected_table_rows(
                            doc,
                            self.selection.ok_or(LayoutError::InvalidRange)?,
                        )?;
                        if matches!(command, Command::TableOverlay) {
                            *operation = Operation::TableRegister(
                                TableRowRegistration::fixed_overlay(doc, view, table, rows)?,
                                true,
                            );
                        } else {
                            let Some(FoldId::TableRange(id)) = target else {
                                return Err(LayoutError::InvalidRange);
                            };
                            if view
                                .table_row_fold_info(doc, id)?
                                .ok_or(LayoutError::InvalidRange)?
                                .table
                                != table
                            {
                                return Err(LayoutError::InvalidRange);
                            }
                            *operation = Operation::TableEdit(
                                TableRowEdit::retarget(
                                    doc,
                                    view,
                                    id,
                                    TableRowExtent::Fixed {
                                        start: rows.start,
                                        end: rows.end,
                                    },
                                )?,
                                false,
                            );
                        }
                        return Ok(false);
                    }
                    Command::TableLive => {
                        let selection = self.selection.ok_or(LayoutError::InvalidRange)?;
                        let table = doc
                            .table_at(selection.start)
                            .ok_or(LayoutError::InvalidRange)?;
                        let row = doc
                            .table(table)
                            .and_then(|t| t.row_at(selection.start))
                            .filter(|&r| r > 0)
                            .ok_or(LayoutError::InvalidRange)?;
                        *operation = Operation::TableRegister(
                            TableRowRegistration::live(doc, view, table, row)?,
                            true,
                        );
                        return Ok(false);
                    }
                    Command::TableSeal | Command::TableRemove => {
                        let Some(FoldId::TableRange(id)) = target else {
                            return Err(LayoutError::InvalidRange);
                        };
                        let remove = matches!(command, Command::TableRemove);
                        let task = if remove {
                            TableRowEdit::remove(doc, view, id)?
                        } else {
                            TableRowEdit::seal(doc, view, id)?
                        };
                        *operation = Operation::TableEdit(task, remove);
                        return Ok(false);
                    }
                    Command::TableNext(all) => {
                        if self.table_range_all != Some(all) {
                            self.table_range_next = None;
                            self.table_range_all = Some(all);
                        }
                        let table = self.current_table(doc, view)?;
                        let task = if let Some(task) = self.table_range_next.take() {
                            task
                        } else if all {
                            TableRowCandidates::all(doc, view, table)?
                        } else {
                            let row = doc
                                .table(table)
                                .and_then(|t| t.row_at(self.source))
                                .filter(|&r| r > 0)
                                .ok_or(LayoutError::InvalidRange)?;
                            TableRowCandidates::intersecting(doc, view, table, row..row + 1)?
                        };
                        *operation = Operation::TableRanges(task);
                        return Ok(false);
                    }
                    Command::Parent => {
                        let candidate = if let Some(id) = target {
                            view.fold_candidate(doc, id)?
                                .and_then(|c| c.parent)
                                .map(|id| view.fold_candidate(doc, id))
                                .transpose()?
                                .flatten()
                        } else {
                            doc.fold_candidate_at(
                                self.source.min(doc.source().len().saturating_sub(1)),
                            )
                        };
                        self.target = candidate.map(|c| c.id);
                        self.message =
                            candidate.map_or_else(|| "no candidate".into(), candidate_label);
                    }
                    Command::NextRange | Command::NextAnyRange => {
                        let all = matches!(command, Command::NextAnyRange);
                        if self.range_all != Some(all) {
                            self.range_next = None;
                            self.range_all = Some(all);
                        }
                        let range = if all {
                            Span::new(0, doc.source().len())
                        } else {
                            Span::new(
                                self.source,
                                self.source.saturating_add(1).min(doc.source().len()),
                            )
                        };
                        let task = self
                            .range_next
                            .take()
                            .map(Ok)
                            .unwrap_or_else(|| SourceFoldCandidates::new(doc, view, range))?;
                        *operation = Operation::Ranges(task);
                        return Ok(false);
                    }
                    Command::Summary => {
                        *operation = Operation::Summary(Box::new(
                            super::render::fold_summary::SummaryJob::new(
                                doc,
                                view,
                                target.ok_or(LayoutError::InvalidRange)?,
                                width,
                                links,
                            )?,
                        ));
                        return Ok(false);
                    }
                    Command::RowTarget => {
                        let candidate = view.table_row_fold_at(doc, self.source)?;
                        self.target = candidate.map(|c| c.id);
                        self.message = candidate
                            .map_or_else(|| "no registered rows here".into(), candidate_label);
                    }
                    Command::Toggle | Command::Open | Command::Close => {
                        let id = target.ok_or(LayoutError::InvalidRange)?;
                        match command {
                            Command::Toggle => {
                                view.toggle(doc, id)?;
                            }
                            Command::Open => {
                                view.set_collapsed(doc, id, false)?;
                            }
                            _ => {
                                view.set_collapsed(doc, id, true)?;
                            }
                        }
                        self.message = "choice changed".into();
                    }
                    Command::Preview | Command::EndPreview => {
                        *operation = Operation::Preview(if matches!(command, Command::Preview) {
                            Some(target.ok_or(LayoutError::InvalidRange)?)
                        } else {
                            None
                        });
                        return Ok(false);
                    }
                    Command::Subtree(collapsed) | Command::Siblings(collapsed) => {
                        let id = target.ok_or(LayoutError::InvalidRange)?;
                        let action = if matches!(command, Command::Subtree(_)) {
                            FoldBatchAction::Subtree {
                                root: id,
                                collapsed,
                            }
                        } else {
                            FoldBatchAction::Siblings {
                                target: id,
                                collapsed,
                            }
                        };
                        *operation = Operation::Batch(Box::new(FoldBatch::new(doc, view, action)?));
                        return Ok(false);
                    }
                    Command::All(collapsed) => {
                        *operation = Operation::Batch(Box::new(FoldBatch::new(
                            doc,
                            view,
                            FoldBatchAction::All { collapsed },
                        )?));
                        return Ok(false);
                    }
                    Command::Level(level, collapsed) => {
                        *operation = Operation::Batch(Box::new(FoldBatch::new(
                            doc,
                            view,
                            FoldBatchAction::AtLevel { level, collapsed },
                        )?));
                        return Ok(false);
                    }
                    Command::Focus => {
                        *operation = Operation::Batch(Box::new(FoldBatch::new(
                            doc,
                            view,
                            FoldBatchAction::Focus {
                                target: target.ok_or(LayoutError::InvalidRange)?,
                            },
                        )?));
                        return Ok(false);
                    }
                    Command::Containers(filter, collapsed) => {
                        *operation = Operation::Batch(Box::new(FoldBatch::new(
                            doc,
                            view,
                            FoldBatchAction::Containers { filter, collapsed },
                        )?));
                        return Ok(false);
                    }
                    Command::Default(kind) => {
                        if kind == 15 {
                            self.prefixes.default = None;
                            self.prefixes.cancel();
                        }
                        let mut defaults = view.defaults();
                        match kind {
                            0..=5 => defaults.heading_levels[kind as usize] ^= true,
                            6 => defaults.code ^= true,
                            7 => defaults.math ^= true,
                            8 => defaults.table ^= true,
                            9 => defaults.containers.quotes ^= true,
                            10 => defaults.containers.lists ^= true,
                            11 => defaults.containers.items ^= true,
                            12 => defaults.containers.completed_tasks ^= true,
                            13 => defaults.containers.incomplete_tasks ^= true,
                            14 => {
                                defaults.container_depth = if defaults.container_depth.is_some() {
                                    None
                                } else {
                                    Some(2)
                                }
                            }
                            _ => {
                                defaults.paragraphs = if defaults.paragraphs.is_some() {
                                    None
                                } else {
                                    Some(mdtui3::ParagraphFoldPolicy {
                                        min_bytes: 4096,
                                        preview_bytes: 256,
                                    })
                                }
                            }
                        }
                        view.set_defaults(defaults);
                        self.message = "policy toggled; only new nodes".into();
                    }
                    Command::Overlay | Command::Strict | Command::Stream => {
                        let selection = self.selection.ok_or(LayoutError::InvalidRange)?;
                        let task = match command {
                            Command::Overlay => {
                                SourceFoldRegistration::overlapping(doc, view, selection)?
                            }
                            Command::Strict => SourceFoldRegistration::new(doc, view, selection)?,
                            _ => SourceFoldRegistration::stream(doc, view, selection.start)?,
                        };
                        *operation = Operation::Register(task, true);
                        return Ok(false);
                    }
                    Command::Seal => {
                        let Some(FoldId::Source(id)) = target else {
                            return Err(LayoutError::InvalidRange);
                        };
                        *operation = Operation::Register(
                            SourceFoldRegistration::seal_tail(doc, view, id)?,
                            false,
                        );
                        return Ok(false);
                    }
                    Command::Remove => {
                        match target.ok_or(LayoutError::InvalidRange)? {
                            FoldId::Source(id) => {
                                view.remove_source_fold(doc, id)?;
                            }
                            FoldId::TableRows(range) => {
                                view.remove_table_rows(doc, range)?;
                            }
                            FoldId::TableRange(id) => {
                                *operation = Operation::TableEdit(
                                    TableRowEdit::remove(doc, view, id)?,
                                    true,
                                );
                                return Ok(false);
                            }
                            _ => return Err(LayoutError::InvalidRange),
                        }
                        self.target = None;
                        self.range_next = None;
                        self.message = "removed selected ID".into();
                    }
                    Command::Rows => {
                        let selection = self.selection.ok_or(LayoutError::InvalidRange)?;
                        let table_id = doc
                            .table_at(selection.start)
                            .ok_or(LayoutError::InvalidRange)?;
                        let table = doc.table(table_id).ok_or(LayoutError::InvalidRange)?;
                        let start = table
                            .row_at(selection.start)
                            .ok_or(LayoutError::InvalidRange)?
                            .max(1);
                        let end = table
                            .row_at(selection.end.saturating_sub(1))
                            .ok_or(LayoutError::InvalidRange)?
                            + 1;
                        let range = TableRowRange {
                            table: table_id,
                            start,
                            end,
                        };
                        view.insert_table_rows(doc, range.table, range.start..range.end)?;
                        self.target = Some(FoldId::TableRows(range));
                        *operation = Operation::Collapse(FoldId::TableRows(range));
                        return Ok(false);
                    }
                    Command::ParagraphLines(mode) => {
                        self.prefixes.request(doc, self.source, mode)?;
                        self.message = "body-prefix pending".into();
                    }
                    Command::DefaultLines(mode) => {
                        self.prefixes.default = if self.prefixes.default == Some(mode) {
                            None
                        } else {
                            Some(mode)
                        };
                        self.auto = None;
                        self.auto_range = None;
                        let mut defaults = view.defaults();
                        defaults.paragraphs =
                            self.prefixes.default.map(|_| mdtui3::ParagraphFoldPolicy {
                                min_bytes: 4096,
                                preview_bytes: 0,
                            });
                        view.set_defaults(defaults);
                        self.message =
                            format!("new paragraph line default: {:?}", self.prefixes.default);
                    }
                    Command::Paragraph => {
                        let block = doc.block_at(self.source).ok_or(LayoutError::InvalidRange)?;
                        *operation = Operation::Register(
                            SourceFoldRegistration::paragraph_overlay(doc, view, block, 256)?,
                            true,
                        );
                        return Ok(false);
                    }
                }
                self.range_next = None;
                Ok(true)
            }
        }
    }
}
fn charge(budget: &mut Budget) -> bool {
    budget.charge(WorkLimit {
        queries: 1,
        objects: 1,
        ..Default::default()
    })
}
fn candidate_label(c: FoldCandidate) -> String {
    crate::render::plan::placeholder_label(c)
}
fn selected_table_rows(
    doc: &Document,
    selection: Span,
) -> Result<(mdtui3::TableId, std::ops::Range<usize>), LayoutError> {
    if selection.is_empty() {
        return Err(LayoutError::InvalidRange);
    }
    let id = doc
        .table_at(selection.start)
        .ok_or(LayoutError::InvalidRange)?;
    if doc.table_at(selection.end - 1) != Some(id) {
        return Err(LayoutError::InvalidRange);
    }
    let table = doc.table(id).ok_or(LayoutError::InvalidRange)?;
    let first = table
        .row_at(selection.start)
        .or_else(|| {
            table
                .delimiter
                .range()
                .contains(&selection.start)
                .then_some(1)
        })
        .ok_or(LayoutError::InvalidRange)?
        .max(1);
    let end = table
        .row_at(selection.end - 1)
        .ok_or(LayoutError::InvalidRange)?
        + 1;
    if first >= end {
        return Err(LayoutError::InvalidRange);
    }
    Ok((id, first..end))
}
fn table_identity(info: TableRowFoldInfo) -> String {
    let (start, end, mode) = match info.extent {
        TableRowExtent::Fixed { start, end } => {
            (start, end.to_string(), if info.closed { 'C' } else { 'G' })
        }
        TableRowExtent::Live { start } => (
            start,
            "*".into(),
            if info.closed {
                'C'
            } else if info.rows.is_none() {
                'D'
            } else {
                'L'
            },
        ),
        TableRowExtent::SealedEmpty { start, .. } => (start, "empty".into(), 'E'),
    };
    format!(
        "R{}{} T{} {start}..{end}",
        info.id.index(),
        mode,
        info.table.0 + 1
    )
}
fn compact_label(label: &str, command: Command) -> String {
    match command {
        Command::TablePrefix(n) => format!("table first{n}"),
        Command::TablePrefixInput => "table first N".into(),
        Command::TableDefault(Some(n)) => format!("new table {n}"),
        Command::TableDefault(None) => "new table off".into(),
        Command::TableDefaultInput => "new table N".into(),
        Command::TableOverlay => "table overlay".into(),
        Command::TableLive => "table live tail".into(),
        Command::TableSeal => "table seal ID".into(),
        Command::TableRetarget => "row retarget ID".into(),
        Command::TableRemove => "table remove ID".into(),
        Command::TableNext(false) => "table next here".into(),
        Command::TableNext(true) => "table next any".into(),
        Command::NextRange => "source next here".into(),
        Command::NextAnyRange => "source next any".into(),
        Command::RowTarget => "table rows here".into(),
        Command::Summary => "summary 2 rows".into(),
        Command::Siblings(true) => "close siblings".into(),
        Command::Focus => "keep path".into(),
        Command::All(true) => "close document".into(),
        Command::Level(level, collapsed) => {
            format!("h{level} {}", if collapsed { "collapse" } else { "open" })
        }
        Command::Overlay => "source fixed".into(),
        Command::Strict => "source strict".into(),
        Command::Stream => "source live EOF".into(),
        Command::Seal => "source seal".into(),
        Command::Remove => "source remove".into(),
        Command::Rows => "table rows fixed".into(),
        Command::Paragraph => "para prefix256B".into(),
        Command::ParagraphLines(lines) => format!("{} para", prefix_label(lines)),
        Command::DefaultLines(lines) => format!("{} para default", prefix_label(lines)),
        Command::Default(kind) => format!(
            "{} default",
            [
                "h1", "h2", "h3", "h4", "h5", "h6", "code", "math", "table", "quotes", "lists",
                "items", "done", "todo", "depth>=2", "para>=4K"
            ][usize::from(kind)]
        ),
        Command::Containers(filter, collapsed)
            if filter.completed_tasks || filter.incomplete_tasks =>
        {
            format!(
                "{} {}",
                if filter.completed_tasks {
                    "done"
                } else {
                    "todo"
                },
                if collapsed { "collapse" } else { "open" }
            )
        }
        _ => label.into(),
    }
}
fn prefix_label(lines: PrefixLines) -> String {
    match lines {
        PrefixLines::Display(0) => "0".into(),
        PrefixLines::Logical(count) => format!("L{count}"),
        PrefixLines::Display(count) => format!("D{count}"),
    }
}
fn entries(page: usize) -> Vec<(String, Command)> {
    // An explicit t submenu preserves the six established pages and their
    // Home/End/left/right command positions, including narrow-screen controls.
    if page == 6 {
        return vec![
            ("table: first 0 body rows".into(), Command::TablePrefix(0)),
            ("table: first 1 body row".into(), Command::TablePrefix(1)),
            ("table: first 3 body rows".into(), Command::TablePrefix(3)),
            ("table: first N body rows".into(), Command::TablePrefixInput),
            ("new table policy: off".into(), Command::TableDefault(None)),
            (
                "new tables: first 0 body rows".into(),
                Command::TableDefault(Some(0)),
            ),
            (
                "new tables: first 1 body row".into(),
                Command::TableDefault(Some(1)),
            ),
            (
                "new tables: first 3 body rows".into(),
                Command::TableDefault(Some(3)),
            ),
            (
                "new tables: first N body rows".into(),
                Command::TableDefaultInput,
            ),
            ("selection: table row overlay".into(), Command::TableOverlay),
            (
                "selection start: live table tail".into(),
                Command::TableLive,
            ),
            ("seal selected table range ID".into(), Command::TableSeal),
            (
                "retarget selected ID to rows".into(),
                Command::TableRetarget,
            ),
            (
                "remove selected table range ID".into(),
                Command::TableRemove,
            ),
            (
                "next overlapping table range".into(),
                Command::TableNext(false),
            ),
            (
                "next registered table range".into(),
                Command::TableNext(true),
            ),
        ];
    }
    let mut result = vec![
        ("toggle".into(), Command::Toggle),
        ("open".into(), Command::Open),
        ("collapse".into(), Command::Close),
        ("parent candidate".into(), Command::Parent),
        ("next overlapping source range".into(), Command::NextRange),
        ("preview".into(), Command::Preview),
        ("end preview".into(), Command::EndPreview),
        ("bounded summary (2 rows)".into(), Command::Summary),
        ("open subtree".into(), Command::Subtree(false)),
        ("collapse subtree".into(), Command::Subtree(true)),
        ("open siblings".into(), Command::Siblings(false)),
        ("collapse siblings".into(), Command::Siblings(true)),
        ("keep current path".into(), Command::Focus),
        ("open document".into(), Command::All(false)),
        ("collapse document".into(), Command::All(true)),
        ("next registered source range".into(), Command::NextAnyRange),
        ("registered table rows here".into(), Command::RowTarget),
        ("selection: fixed overlay".into(), Command::Overlay),
        ("selection: strict stable range".into(), Command::Strict),
        ("selection start: live EOF".into(), Command::Stream),
        ("seal selected live EOF".into(), Command::Seal),
        ("remove selected range ID".into(), Command::Remove),
        ("selection: fixed table rows".into(), Command::Rows),
        ("paragraph: source prefix 256B".into(), Command::Paragraph),
        (
            "0 body lines: paragraph".into(),
            Command::ParagraphLines(PrefixLines::Display(0)),
        ),
        (
            "L1 paragraph: 1 logical line".into(),
            Command::ParagraphLines(PrefixLines::Logical(1)),
        ),
        (
            "L3 paragraph: 3 logical lines".into(),
            Command::ParagraphLines(PrefixLines::Logical(3)),
        ),
        (
            "D1 paragraph: 1 display row".into(),
            Command::ParagraphLines(PrefixLines::Display(1)),
        ),
        (
            "D3 paragraph: 3 display rows".into(),
            Command::ParagraphLines(PrefixLines::Display(3)),
        ),
    ];
    for level in 1..=6 {
        for collapsed in [false, true] {
            result.push((
                format!(
                    "h{level} only: {}",
                    if collapsed { "collapse" } else { "open" }
                ),
                Command::Level(level, collapsed),
            ));
        }
    }
    for (label, filter) in [
        (
            "quotes",
            ContainerFoldFilter {
                quotes: true,
                ..Default::default()
            },
        ),
        (
            "lists",
            ContainerFoldFilter {
                lists: true,
                ..Default::default()
            },
        ),
        (
            "items",
            ContainerFoldFilter {
                items: true,
                ..Default::default()
            },
        ),
        (
            "completed tasks",
            ContainerFoldFilter {
                completed_tasks: true,
                ..Default::default()
            },
        ),
        (
            "incomplete tasks",
            ContainerFoldFilter {
                incomplete_tasks: true,
                ..Default::default()
            },
        ),
    ] {
        for collapsed in [false, true] {
            result.push((
                format!("{label}: {}", if collapsed { "collapse" } else { "open" }),
                Command::Containers(filter, collapsed),
            ));
        }
    }
    for (kind, label) in [
        "h1",
        "h2",
        "h3",
        "h4",
        "h5",
        "h6",
        "code",
        "math",
        "table",
        "quotes",
        "lists",
        "items",
        "completed tasks",
        "incomplete tasks",
        "depth >=2",
        "paragraph >=4096B",
    ]
    .into_iter()
    .enumerate()
    {
        result.push((
            format!("default toggle {label} (only new)"),
            Command::Default(kind as u8),
        ));
    }
    for (label, mode) in [
        ("0 body lines", PrefixLines::Display(0)),
        ("1 logical line", PrefixLines::Logical(1)),
        ("3 logical lines", PrefixLines::Logical(3)),
        ("1 display row", PrefixLines::Display(1)),
        ("3 display rows", PrefixLines::Display(3)),
    ] {
        result.push((
            format!("default paragraph >=4096B: {label}"),
            Command::DefaultLines(mode),
        ));
    }
    result.retain(|(_, command)| match command {
        Command::All(_) | Command::Focus => page == 1,
        Command::Overlay
        | Command::Strict
        | Command::Stream
        | Command::Seal
        | Command::Remove
        | Command::Rows
        | Command::Paragraph
        | Command::ParagraphLines(_)
        | Command::NextRange
        | Command::NextAnyRange
        | Command::RowTarget => page == 2,
        Command::Level(_, _) => page == 3,
        Command::Containers(_, _) => page == 4,
        Command::Default(_) | Command::DefaultLines(_) => page == 5,
        _ => page == 0,
    });
    result
}
