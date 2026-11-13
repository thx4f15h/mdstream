//! One movable ownership ledger for undecided indented-code blank sources.
use crate::containers::CodeContinuation;
use crate::deferred::SourceOwner;
use crate::{BlockId, BlockKind, Document, Span, Style, Update};

/// Completed blank sources whose membership in a growing Code is undecided.
/// They are withheld from display, but remain readable in the original source.
/// This current-revision metadata is separate from bounded `Document::pending`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PendingCodeBlanks {
    pub owner: BlockId,
    pub source: Span,
}

/// Ledger metadata only; excludes source, annotations, allocator headers and Updates.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CodeBlankStorage {
    pub records: usize,
    pub capacity_bytes: usize,
    pub allocation_calls: u64,
}

struct Fragment {
    source: Span,
    structural_start: usize,
    code_start: usize,
    structural_virtual: u8,
    code_virtual: u8,
    _owner: SourceOwner,
}

#[derive(Default)]
pub(crate) struct Blanks {
    pending: Option<PendingCodeBlanks>,
    fragments: Vec<Fragment>,
    from: Option<usize>,
    allocations: u64,
}

impl Document {
    pub fn indented_code_pending(&self) -> Option<PendingCodeBlanks> {
        self.code_blanks.pending
    }

    pub fn code_blank_storage(&self) -> CodeBlankStorage {
        CodeBlankStorage {
            records: self.code_blanks.fragments.len(),
            capacity_bytes: self.code_blanks.fragments.capacity() * std::mem::size_of::<Fragment>(),
            allocation_calls: self.code_blanks.allocations,
        }
    }

    /// Called before any successor container/inline/header side effects.
    pub(crate) fn indented_source(
        &mut self,
        span: Span,
        complete: bool,
        update: &mut Update,
    ) -> bool {
        let Some(owner) = self.current.filter(|id| {
            self.fence.is_none() && matches!(self.blocks[id.0].kind, BlockKind::Code { .. })
        }) else {
            return false;
        };
        self.work.code_blank_probes += 1;
        match self
            .containers
            .code_continuation(&self.source, span, complete, &mut self.work)
        {
            Some(CodeContinuation::Blank {
                prefix,
                virtual_spaces,
                column,
                from,
            }) => {
                let mut start = prefix;
                let mut remaining = 4 - virtual_spaces;
                let mut column = column;
                let mut spaces = 0;
                let end = crate::block::body_end(&self.source, span);
                while remaining > 0 && start < end {
                    self.work.code_blank_bytes += 1;
                    let width = match self.source.as_bytes()[start] {
                        b' ' => 1,
                        b'\t' => 4 - column,
                        _ => break,
                    };
                    start += 1;
                    column = (column + width) % 4;
                    let removed = remaining.min(width);
                    remaining -= removed;
                    spaces = width - removed;
                }
                let blanks = &mut self.code_blanks;
                let pending = blanks.pending.get_or_insert(PendingCodeBlanks {
                    owner,
                    source: Span::new(span.start, span.start),
                });
                debug_assert_eq!(pending.owner, owner);
                debug_assert_eq!(pending.source.end, span.start);
                pending.source.end = span.end;
                if let Some(from) = from {
                    blanks.from = Some(blanks.from.map_or(from, |old| old.min(from)));
                }
                if blanks.fragments.len() == blanks.fragments.capacity() {
                    blanks.allocations += 1;
                }
                blanks.fragments.push(Fragment {
                    source: span,
                    structural_start: prefix,
                    code_start: start,
                    structural_virtual: virtual_spaces,
                    code_virtual: spaces,
                    _owner: SourceOwner::new(),
                });
                self.work.code_blank_admissions += 1;
                // Dynamic withholding has no irreversible HIDDEN annotation.
                // This fact invalidates provisional raw rows already displayed.
                update.dirty.push(span);
                update.change(span, crate::ChangeKind::Structure);
                true
            }
            Some(CodeContinuation::Indented) => {
                self.resolve_code_blanks(true, update);
                false
            }
            None => {
                self.resolve_code_blanks(false, update);
                false
            }
        }
    }

    pub(crate) fn resolve_code_blanks(&mut self, code: bool, update: &mut Update) {
        let Some(pending) = self.code_blanks.pending.take() else {
            return;
        };
        self.work.code_blank_decisions += 1;
        let from = self.code_blanks.from.take();
        let mut fragments = std::mem::take(&mut self.code_blanks.fragments);
        if !code {
            self.containers.code_blank_separator(
                from,
                pending.source.start,
                &mut self.work,
                update,
            );
            self.close_current(update);
        }
        for fragment in fragments.drain(..) {
            let Fragment {
                source,
                structural_start,
                code_start,
                structural_virtual,
                code_virtual,
                _owner,
            } = fragment;
            let (mut start, spaces) = if code {
                (code_start, code_virtual)
            } else {
                (structural_start, structural_virtual)
            };
            if spaces > 0 {
                start -= 1;
                self.display.spaces(Span::new(start, start + 1), spaces);
            }
            self.mark(Span::new(source.start, start), Style::HIDDEN, update);
            let end = crate::block::body_end(&self.source, source);
            if end + 2 == source.end {
                self.mark(Span::new(end, end + 1), Style::HIDDEN, update);
            }
            if !code {
                let id = self.push_block(source, BlockKind::Blank, update);
                self.blocks[id.0].state = crate::SyntaxState::Sealed;
                self.current = None;
            }
            self.work.code_blank_retirements += 1;
            // The first-owned right is moved here and consumed exactly once.
            let _ = _owner;
        }
        self.code_blanks.fragments = fragments;
        if code {
            self.blocks[pending.owner.0].source.end = pending.source.end;
        }
        update.changed_blocks.start = update.changed_blocks.start.min(pending.owner.0);
        update.dirty.push(pending.source);
        update.change(pending.source, crate::ChangeKind::Structure);
    }
}
