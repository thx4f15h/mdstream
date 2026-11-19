//! ASCII gaps keep the existing cursor; numbered atoms use the shared shape.
use super::*;
pub(super) struct NoteScan {
    next: usize,
    initialized: bool,
    reference: Option<(mdtui3::FootnoteReferenceId, mdtui3::FootnoteReference)>,
    pub(super) child: Option<Box<DisplayScanner>>,
}
impl DisplayScanner {
    pub(super) fn advance_notes(
        &mut self,
        document: &Document,
        context: Span,
        budget: &mut Budget,
    ) -> Result<ScanStatus, LayoutError> {
        let end = self
            .requested
            .end
            .min(self.source_cut.unwrap_or(usize::MAX));
        if end <= self.requested.start {
            self.note_bypass = true;
            self.force_indexed = true;
            return self.advance(document, budget);
        }
        if self.note_scan.is_none() {
            self.note_scan = Some(Box::new(NoteScan {
                next: self.requested.start,
                initialized: false,
                reference: None,
                child: None,
            }));
        }
        loop {
            let scan = self.note_scan.as_mut().unwrap();
            if let Some(child) = &mut scan.child {
                if child.advance(document, budget)? != ScanStatus::Complete {
                    return Ok(ScanStatus::BudgetExhausted);
                }
                let mut child = scan.child.take().unwrap();
                self.display_source_work += child.display_source_work;
                self.display_bytes += child.display_bytes;
                self.cursor_steps += child.cursor_steps;
                self.edge_source_bytes += child.edge_source_bytes;
                self.edge_queries += child.edge_queries;
                self.zero_ranges.append(&mut child.zero_ranges);
                self.layout_segments.append(&mut child.layout_segments);
                self.segments.append(&mut child.segments);
                self.following_source = child.following_source.or(self.following_source);
                continue;
            }
            if !scan.initialized {
                let mut cursor = document
                    .footnote_references_in(Span::new(self.requested.start, end))
                    .ok_or(LayoutError::InvalidRange)?;
                let Ok(first) = cursor.advance(budget) else {
                    return Ok(ScanStatus::BudgetExhausted);
                };
                scan.reference = first;
                scan.initialized = true;
                if first.is_none() {
                    self.note_scan = None;
                    self.note_bypass = true;
                    return self.advance(document, budget);
                }
            }
            if scan.next >= end {
                self.scanned_through = self.requested.end;
                self.resolved_start = Some(self.requested.start);
                self.resolved_end = Some(self.requested.end);
                self.source_complete = true;
                self.complete = true;
                self.lease = None;
                self.refresh_complete_tiles(document)?;
                return Ok(ScanStatus::Complete);
            }
            let reference = scan.reference.filter(|(_, r)| r.source.start < end);
            let (source, indexed, fragment) = match reference {
                Some((_, r)) if r.source.start > scan.next => {
                    (Span::new(scan.next, r.source.start), false, None)
                }
                Some((id, r)) => {
                    if !budget.charge(mdtui3::WorkLimit {
                        queries: 1,
                        objects: 1,
                        ..Default::default()
                    }) {
                        return Ok(ScanStatus::BudgetExhausted);
                    }
                    scan.reference = document
                        .next_footnote_reference(id)
                        .map(|next| (next, document.footnote_references()[next.0]));
                    if r.source.start < scan.next || r.source.end > end {
                        // Retain the full existing cluster/object policy when
                        // the requested window starts or ends inside a marker.
                        (Span::new(scan.next, end), true, Some(context))
                    } else {
                        (r.source, true, Some(r.source))
                    }
                }
                None => (Span::new(scan.next, end), false, None),
            };
            let mut child = Self::exact(document, source, self.links.clone())
                .ok_or(LayoutError::InvalidRange)?;
            child.math_bypass = true;
            child.math_maintenance = false;
            child.note_bypass = true;
            child.force_indexed = indexed;
            child.fold_fragment = fragment;
            child.table_mode = self.table_mode;
            child.table_context = Some(source);
            child.math_rect = self.math_rect;
            scan.next = source.end;
            scan.child = Some(Box::new(child));
        }
    }
}
