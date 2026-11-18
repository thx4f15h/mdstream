//! One view's concrete keyboard menu and paid fold operations.
use crate::render::{checkpoint::RenderCheckpoint, prefix::PrefixLines};
use crossterm::event::KeyCode;
use mdtui3::{
    Budget, Completion, ContainerFoldFilter, Document, FoldBatch, FoldBatchAction, FoldCandidate,
    FoldId, FoldView, LayoutError, SourceFoldCandidates, SourceFoldRegistration, Span,
    TableRowRange, WorkLimit,
};

type View = FoldView<RenderCheckpoint>;

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
    summary: Option<super::render::fold_summary::FoldSummary>,
    changed: bool,
}

impl FoldUi {
    pub fn end_preview(&mut self) {
        self.operation = Some(Operation::Preview(None));
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
    }
    pub fn on_update(
        &mut self,
        doc: &Document,
        view: &View,
        update: &mdtui3::Update,
    ) -> Result<(), LayoutError> {
        self.prefixes.on_update(doc, view, update)?;
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
        if self.operation.is_some() {
            return Some(format!("Esc cancel | {}", self.message));
        }
        if let Some((label, _)) = self.menu.then(|| self.entries.get(self.index)).flatten() {
            if width < 32 {
                let fit = super::render::text::fit_prefix(
                    label,
                    usize::from(width.saturating_sub(12)).max(1),
                );
                return Some(format!("{} ↑↓ ↵ Esc ←→", &label[..fit.end]));
            }
            Some(format!(
                "z ↑↓ Enter Esc ←→ | {}: {} ({}/{}) | {}",
                [
                    "node",
                    "document",
                    "range",
                    "level",
                    "container",
                    "new defaults"
                ][self.page],
                label,
                self.index + 1,
                self.entries.len(),
                self.message
            ))
        } else if self.pending() {
            Some(format!("fold: {} | Esc cancel", self.message))
        } else {
            None
        }
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
        self.range_next = None;
        self.range_all = None;
        if self.target.is_none() {
            self.operation = Some(Operation::Command(Command::Parent));
        }
    }
    pub fn key(&mut self, code: KeyCode) -> bool {
        if !self.active() {
            return false;
        }
        if self.operation.is_some() && !matches!(code, KeyCode::Esc | KeyCode::Char('q' | 'c')) {
            return true;
        }
        match code {
            KeyCode::Esc => {
                self.menu = false;
                self.operation = None;
                self.range_next = None;
                self.auto = None;
                self.auto_range = None;
                self.prefixes.cancel();
                self.message = if self.prefixes.validation_pending() {
                    "menu closed; current prefix validation continues; q exits".into()
                } else {
                    "cancelled; completed choices retained".into()
                };
            }
            KeyCode::Up if self.menu => {
                self.index = self.index.saturating_sub(1);
            }
            KeyCode::Down if self.menu => {
                self.index = (self.index + 1).min(self.entries.len().saturating_sub(1));
            }
            KeyCode::Left | KeyCode::Right if self.menu => {
                self.page = if code == KeyCode::Right {
                    (self.page + 1) % 6
                } else {
                    (self.page + 5) % 6
                };
                self.entries = entries(self.page);
                self.index = 0;
            }
            KeyCode::Home if self.menu => self.index = 0,
            KeyCode::End if self.menu => self.index = self.entries.len().saturating_sub(1),
            KeyCode::Enter if self.menu && self.operation.is_none() => {
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
    }
    pub fn take_changed(&mut self) -> bool {
        std::mem::take(&mut self.changed)
    }

    pub(crate) fn take_summary(&mut self) -> Option<super::render::fold_summary::FoldSummary> {
        self.summary.take()
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
            if self.prefixes.pending() {
                let old = view.version();
                if let Err(error) = self.prefixes.advance(doc, view, links, width, budget) {
                    self.message = error;
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
        if old != view.version()
            && let Some(task) = self.auto.take()
        {
            self.auto_range = Some(task.remaining_range());
        }
        match result {
            Ok(false) => self.operation = Some(operation),
            Ok(true) => {}
            Err(error) => {
                self.message = error.to_string();
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
fn entries(page: usize) -> Vec<(String, Command)> {
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
