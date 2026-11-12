use crate::screen::{Cell, ContentArea, Screen};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffRun {
    pub row: u16,
    pub start_col: u16,
    /// Back-buffer cells to draw. Width-2 bases are followed by their zero-width
    /// continuation cell so every pair remains visible to the diff consumer.
    pub cells: Vec<Cell>,
}

impl DiffRun {
    #[must_use]
    pub fn end_col(&self) -> u16 {
        self.cells.iter().fold(self.start_col, |column, cell| {
            column.saturating_add(u16::from(cell.width()))
        })
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.cells.is_empty()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScreenDiff {
    pub runs: Vec<DiffRun>,
    pub rows_changed: usize,
    pub cells_changed: usize,
}

impl ScreenDiff {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.runs.is_empty()
    }
}

/// Compare the back buffer against the committed front buffer without mutating
/// either one. Only the screen content area participates; line sources are not
/// visual state and do not create runs.
#[must_use]
pub fn diff_screen(screen: &Screen) -> ScreenDiff {
    let content = screen.content_area();
    if content.width == 0 {
        return ScreenDiff::default();
    }
    let mut diff = ScreenDiff::default();
    for row in 0..screen.rows() {
        let Some(front) = screen.front_row(row) else {
            continue;
        };
        let Some(back) = screen.back_row(row) else {
            continue;
        };
        let runs = diff_row(row, front, back, content);
        if runs.is_empty() {
            continue;
        }
        diff.rows_changed += 1;
        diff.cells_changed = diff
            .cells_changed
            .saturating_add(runs.iter().map(|run| run.cells.len()).sum());
        diff.runs.extend(runs);
    }
    diff
}

fn diff_row(row: u16, front: &[Cell], back: &[Cell], content: ContentArea) -> Vec<DiffRun> {
    debug_assert!(front.len() >= usize::from(content.right()));
    debug_assert!(back.len() >= usize::from(content.right()));
    let width = usize::from(content.width);
    if width == 0 {
        return Vec::new();
    }

    let mut changed = vec![false; width];
    let mut any = false;
    for column in content.x..content.right() {
        if !front[usize::from(column)].same_visual(&back[usize::from(column)]) {
            mark_pair(
                &mut changed,
                front,
                back,
                content.x,
                content.right(),
                column,
            );
            any = true;
        }
    }
    if !any {
        return Vec::new();
    }

    let base = usize::from(content.x);
    let mut runs = Vec::new();
    let mut run_start = 0_usize;
    while run_start < width {
        if !changed[run_start] {
            run_start += 1;
            continue;
        }
        let mut run_end = run_start + 1;
        while run_end < width && changed[run_end] {
            run_end += 1;
        }
        run_end = run_start
            + trim_safe_blank_tail(
                &front[base + run_start..base + run_end],
                &back[base + run_start..base + run_end],
            );
        if run_end > run_start {
            let start_col = content.x
                + u16::try_from(run_start).expect("run offset came from a u16 content width");
            let end_col = content.x
                + u16::try_from(run_end).expect("run offset came from a u16 content width");
            runs.push(DiffRun {
                row,
                start_col,
                cells: back[usize::from(start_col)..usize::from(end_col)].to_vec(),
            });
            run_start = run_end;
        } else {
            run_start += 1;
        }
    }
    runs
}

/// A trailing cell from the change mask is safe to omit only when both buffers
/// show the same default blank. If either side holds old or new visible state,
/// the cell stays in the run so P41 can clear or draw it.
fn trim_safe_blank_tail(front: &[Cell], back: &[Cell]) -> usize {
    debug_assert_eq!(front.len(), back.len());
    let mut length = front.len();
    while length > 0
        && front[length - 1].same_visual(&Cell::blank())
        && back[length - 1].same_visual(&Cell::blank())
    {
        length -= 1;
    }
    length
}

fn mark_pair(
    changed: &mut [bool],
    front: &[Cell],
    back: &[Cell],
    content_x: u16,
    content_right: u16,
    column: u16,
) {
    let Some(offset) = column
        .checked_sub(content_x)
        .map(usize::from)
        .filter(|offset| *offset < changed.len())
    else {
        return;
    };
    if changed[offset] {
        return;
    }
    changed[offset] = true;

    if (front[usize::from(column)].is_continuation() || back[usize::from(column)].is_continuation())
        && column > content_x
    {
        mark_pair(changed, front, back, content_x, content_right, column - 1);
    }
    if (front[usize::from(column)].width() == 2 || back[usize::from(column)].width() == 2)
        && column.saturating_add(1) < content_right
    {
        mark_pair(changed, front, back, content_x, content_right, column + 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::screen::{ContentArea, LineSource, StyleRole};
    use mdtui3::{BlockId, LinkId, Span};

    fn narrow(text: &str) -> Cell {
        Cell::new(text, StyleRole::DEFAULT, 1, None, None).unwrap()
    }

    fn wide(text: &str) -> Cell {
        Cell::new(text, StyleRole::DEFAULT, 2, None, None).unwrap()
    }

    fn styled(text: &str, style: StyleRole, link: Option<LinkId>, width: u8) -> Cell {
        Cell::new(text, style, width, link, None).unwrap()
    }

    fn with_front_and_back(
        rows: u16,
        cols: u16,
        content: ContentArea,
        front: impl FnOnce(&mut Screen),
        back: impl FnOnce(&mut Screen),
    ) -> Screen {
        let mut screen = Screen::new(rows, cols, content).unwrap();
        front(&mut screen);
        screen.swap_buffers();
        back(&mut screen);
        screen
    }

    fn draw_row(screen: &mut Screen, row: u16, start: u16, cells: &[Cell]) {
        let mut column = start;
        for cell in cells {
            if cell.is_continuation() {
                continue;
            }
            screen.write_cell(row, column, cell.clone()).unwrap();
            column += u16::from(cell.width());
        }
    }

    fn continuation_cell() -> Cell {
        let mut screen = Screen::new(1, 2, ContentArea::new(0, 2)).unwrap();
        draw_row(&mut screen, 0, 0, &[wide("你")]);
        screen.back_row(0).unwrap()[1].clone()
    }

    fn assert_apply_matches(screen: &Screen) {
        let diff = diff_screen(screen);
        let mut applied = Screen::new(screen.rows(), screen.cols(), screen.content_area()).unwrap();
        for row in 0..screen.rows() {
            let cells = screen.front_row(row).unwrap().to_vec();
            let mut column = screen.content_area().x;
            for cell in cells
                .into_iter()
                .skip(usize::from(screen.content_area().x))
                .take(usize::from(screen.content_area().width))
            {
                if cell.is_continuation() {
                    continue;
                }
                applied.write_cell(row, column, cell).unwrap();
                column += u16::from(applied.cell(row, column).unwrap().width());
            }
        }
        for run in diff.runs {
            draw_row(&mut applied, run.row, run.start_col, &run.cells);
        }
        for row in 0..screen.rows() {
            for column in screen.content_area().x..screen.content_area().right() {
                assert!(
                    applied
                        .cell(row, column)
                        .unwrap()
                        .same_visual(screen.cell(row, column).unwrap()),
                    "row {row} column {column}"
                );
            }
        }
    }

    #[test]
    fn no_visual_change_has_empty_diff_even_when_sources_differ() {
        let mut screen = Screen::new(2, 8, ContentArea::new(1, 6)).unwrap();
        screen
            .set_line_source(0, LineSource::new(Span::new(0, 1), BlockId(1)))
            .unwrap();
        screen.swap_buffers();
        screen
            .set_line_source(0, LineSource::new(Span::new(9, 10), BlockId(2)))
            .unwrap();
        assert!(diff_screen(&screen).is_empty());
    }

    #[test]
    fn single_adjacent_and_separated_runs_are_minimal() {
        let mut screen = Screen::new(1, 10, ContentArea::new(0, 10)).unwrap();
        screen.swap_buffers();
        screen.write_cell(0, 2, narrow("a")).unwrap();
        let diff = diff_screen(&screen);
        assert_eq!(diff.runs.len(), 1);
        assert_eq!((diff.runs[0].start_col, diff.runs[0].cells.len()), (2, 1));

        let mut screen = Screen::new(1, 10, ContentArea::new(0, 10)).unwrap();
        screen.swap_buffers();
        for (column, text) in [(2, "a"), (3, "b"), (5, "c")] {
            screen.write_cell(0, column, narrow(text)).unwrap();
        }
        let diff = diff_screen(&screen);
        assert_eq!(
            diff.runs
                .iter()
                .map(|run| (run.start_col, run.cells.len()))
                .collect::<Vec<_>>(),
            vec![(2, 2), (5, 1)]
        );
    }

    #[test]
    fn wide_base_changes_expand_to_the_continuation_pair() {
        let screen = with_front_and_back(
            1,
            6,
            ContentArea::new(0, 6),
            |screen| draw_row(screen, 0, 0, &[wide("你")]),
            |screen| draw_row(screen, 0, 0, &[wide("🙂")]),
        );
        let diff = diff_screen(&screen);
        assert_eq!(diff.runs.len(), 1);
        assert_eq!(diff.runs[0].start_col, 0);
        assert_eq!(diff.runs[0].cells.len(), 2);
        assert_eq!(diff.runs[0].end_col(), 2);
        assert!(diff.runs[0].cells[1].is_continuation());
        assert_apply_matches(&screen);
    }

    #[test]
    fn continuation_change_alone_marks_its_base() {
        let blank = Cell::blank();
        let front = vec![
            narrow("a"),
            continuation_cell(),
            blank.clone(),
            blank.clone(),
        ];
        let back = vec![narrow("a"), blank.clone(), blank.clone(), blank];
        let runs = diff_row(0, &front, &back, ContentArea::new(0, 4));

        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].start_col, 0);
        assert_eq!(runs[0].cells.len(), 2);
        assert_eq!(runs[0].cells[0].text(), "a");
        assert!(runs[0].cells[1].same_visual(&Cell::blank()));
    }

    #[test]
    fn narrow_wide_transitions_invalidate_both_halves() {
        let wide_to_narrow = with_front_and_back(
            1,
            5,
            ContentArea::new(0, 5),
            |screen| draw_row(screen, 0, 0, &[wide("你")]),
            |screen| draw_row(screen, 0, 0, &[narrow("x")]),
        );
        let diff = diff_screen(&wide_to_narrow);
        assert_eq!(diff.runs[0].cells.len(), 2);
        assert!(diff.runs[0].cells[1].same_visual(&Cell::blank()));
        assert_apply_matches(&wide_to_narrow);

        let narrow_to_wide = with_front_and_back(
            1,
            5,
            ContentArea::new(0, 5),
            |screen| draw_row(screen, 0, 0, &[narrow("x")]),
            |screen| draw_row(screen, 0, 0, &[wide("你")]),
        );
        let diff = diff_screen(&narrow_to_wide);
        assert_eq!(diff.runs[0].cells.len(), 2);
        assert!(diff.runs[0].cells[1].is_continuation());
        assert_apply_matches(&narrow_to_wide);
    }

    #[test]
    fn clearing_content_to_blank_keeps_required_cells_but_trims_safe_blanks() {
        let screen = with_front_and_back(
            1,
            6,
            ContentArea::new(0, 6),
            |screen| draw_row(screen, 0, 0, &[wide("你"), narrow("x")]),
            |_| {},
        );
        let diff = diff_screen(&screen);
        assert_eq!(diff.runs[0].start_col, 0);
        assert_eq!(diff.runs[0].cells.len(), 3);
        assert!(
            diff.runs[0]
                .cells
                .iter()
                .all(|cell| cell.same_visual(&Cell::blank()))
        );
        assert!(!diff.runs[0].cells.iter().any(Cell::is_continuation));
        assert_apply_matches(&screen);
    }

    #[test]
    fn tail_trim_distinguishes_safe_blanks_from_old_content() {
        let blank = Cell::blank();
        let visible = narrow("x");
        assert_eq!(
            trim_safe_blank_tail(&[visible.clone(), blank.clone()], &[visible, blank.clone()]),
            1
        );

        let old = narrow("x");
        let clear = vec![old, blank.clone()];
        let cleared = vec![blank.clone(), blank.clone()];
        assert_eq!(trim_safe_blank_tail(&clear, &cleared), 1);

        let old_wide_tail = vec![narrow("x"), continuation_cell()];
        assert_eq!(trim_safe_blank_tail(&old_wide_tail, &cleared), 2);
    }

    #[test]
    fn style_and_link_changes_are_visual_but_source_is_not() {
        let mut screen = Screen::new(1, 8, ContentArea::new(0, 8)).unwrap();
        draw_row(
            &mut screen,
            0,
            0,
            &[styled("x", StyleRole::DEFAULT, None, 1)],
        );
        screen.swap_buffers();
        draw_row(
            &mut screen,
            0,
            0,
            &[styled("x", StyleRole::STRONG, Some(LinkId(3)), 1)],
        );
        let diff = diff_screen(&screen);
        assert_eq!(diff.runs.len(), 1);
        assert_eq!(diff.runs[0].cells[0].style(), StyleRole::STRONG);
        assert_eq!(diff.runs[0].cells[0].link_id(), Some(LinkId(3)));
        assert_apply_matches(&screen);

        let mut source_only = Screen::new(1, 4, ContentArea::new(0, 4)).unwrap();
        source_only
            .write_cell(
                0,
                0,
                Cell::new("x", StyleRole::DEFAULT, 1, None, Some(Span::new(0, 1))).unwrap(),
            )
            .unwrap();
        source_only.swap_buffers();
        source_only
            .write_cell(
                0,
                0,
                Cell::new("x", StyleRole::DEFAULT, 1, None, Some(Span::new(2, 3))).unwrap(),
            )
            .unwrap();
        assert!(diff_screen(&source_only).is_empty());
    }

    #[test]
    fn multi_row_content_offset_and_final_application_match_full_redraw() {
        let screen = with_front_and_back(
            3,
            12,
            ContentArea::new(3, 7),
            |screen| {
                draw_row(screen, 0, 3, &[wide("你"), narrow("a")]);
                draw_row(screen, 2, 4, &[narrow("old")]);
            },
            |screen| {
                draw_row(screen, 0, 3, &[narrow("x"), wide("🙂"), narrow("z")]);
                draw_row(screen, 1, 5, &[styled("n", StyleRole::RULE, None, 1)]);
                draw_row(screen, 2, 3, &[wide("界")]);
                screen
                    .set_line_source(2, LineSource::pending(Span::new(0, 1)))
                    .unwrap();
            },
        );
        let diff = diff_screen(&screen);
        assert_eq!(diff.rows_changed, 3);
        for run in &diff.runs {
            assert!(run.start_col >= 3);
            assert!(run.end_col() <= 10);
        }
        assert_apply_matches(&screen);

        let mut blank = Screen::new(3, 12, ContentArea::new(3, 7)).unwrap();
        let empty = diff_screen(&blank);
        assert!(empty.is_empty());
        blank.clear_back();
        assert!(diff_screen(&blank).is_empty());
    }

    #[test]
    fn diff_does_not_mutate_buffers_or_line_sources() {
        let screen = with_front_and_back(
            2,
            8,
            ContentArea::new(1, 6),
            |screen| draw_row(screen, 0, 1, &[narrow("old")]),
            |screen| {
                draw_row(screen, 0, 1, &[wide("你"), narrow("x")]);
                screen
                    .set_line_source(0, LineSource::new(Span::new(4, 8), BlockId(9)))
                    .unwrap();
            },
        );
        let front: Vec<_> = (0..screen.rows())
            .map(|row| screen.front_row(row).unwrap().to_vec())
            .collect();
        let back: Vec<_> = (0..screen.rows())
            .map(|row| screen.back_row(row).unwrap().to_vec())
            .collect();
        let line_source = screen.line_source(0);

        let _ = diff_screen(&screen);

        for (row, (front_row, back_row)) in front.iter().zip(&back).enumerate() {
            assert_eq!(screen.front_row(row as u16).unwrap(), front_row.as_slice());
            assert_eq!(screen.back_row(row as u16).unwrap(), back_row.as_slice());
        }
        assert_eq!(screen.line_source(0), line_source);
    }

    #[test]
    fn a_wide_pair_is_never_split_by_run_boundaries() {
        let screen = with_front_and_back(
            1,
            8,
            ContentArea::new(1, 6),
            |screen| draw_row(screen, 0, 1, &[narrow("a"), wide("你")]),
            |screen| draw_row(screen, 0, 1, &[narrow("a"), narrow("b")]),
        );
        let diff = diff_screen(&screen);
        assert_eq!(diff.runs.len(), 1);
        assert_eq!(diff.runs[0].start_col, 2);
        assert_eq!(diff.runs[0].cells.len(), 2);
        assert_apply_matches(&screen);
    }
}
