use crossterm::event::{KeyCode, KeyModifiers};
use mdtui_tui::{
    cli::LinkMode,
    fold_ui::FoldUi,
    render::frame::{FrameGeometry, FrameRuntime},
};
use mdtui3::*;

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
fn draw(r: &mut FrameRuntime, p: &mut vt100::Parser) -> String {
    let mut wire = Vec::new();
    for _ in 0..100_000 {
        let mut b = Budget::new(micro());
        let c = r.frame_to(&mut wire, &mut b).unwrap();
        let u = b.used();
        assert!(
            u.queries <= 1
                && u.objects <= 1
                && u.bytes <= 6
                && u.tiles <= 1
                && u.dependencies <= 1
                && u.recovery <= 1
        );
        if c.committed && !r.fold_pending() && !r.has_pending_jump() && !r.reading().pending() {
            p.process(&wire);
            return p.screen().contents();
        }
    }
    panic!("table microbudget frame stalled");
}
fn runtime(source: &str, width: u16, rows: u16) -> (FrameRuntime, vt100::Parser) {
    (
        FrameRuntime::from_finished_source(
            source,
            FrameGeometry {
                rows,
                cols: width,
                content_width: width,
            },
            LinkMode::Default,
        )
        .unwrap(),
        vt100::Parser::new(rows.try_into().unwrap(), width.try_into().unwrap(), 0),
    )
}
fn open(r: &mut FrameRuntime, p: &mut vt100::Parser) {
    r.open_fold_menu(false);
    draw(r, p);
    assert!(r.fold_menu_key(KeyCode::Char('t')));
    draw(r, p);
}
fn action(r: &mut FrameRuntime, p: &mut vt100::Parser, index: usize) -> String {
    open(r, p);
    for _ in 0..index {
        r.fold_menu_key(KeyCode::Down);
    }
    r.fold_menu_key(KeyCode::Enter);
    draw(r, p)
}
fn close(r: &mut FrameRuntime, p: &mut vt100::Parser) {
    r.fold_menu_key(KeyCode::Esc);
    draw(r, p);
}
fn input(r: &mut FrameRuntime, p: &mut vt100::Parser, s: &str) {
    for c in s.chars() {
        r.fold_menu_key(KeyCode::Char(c));
    }
    r.fold_menu_key(KeyCode::Enter);
    draw(r, p);
}
fn add(r: &mut FrameRuntime, p: &mut vt100::Parser, s: &str) {
    if let Some(u) = r.append_bytes(s.as_bytes()).unwrap() {
        r.apply_update(&u).unwrap();
    }
    draw(r, p);
}
fn info(r: &FrameRuntime, table: TableId) -> Vec<TableRowFoldInfo> {
    let mut task = TableRowCandidates::all(r.document(), r.viewport().folds(), table).unwrap();
    let mut all = Vec::new();
    loop {
        match task
            .next(
                r.document(),
                r.viewport().folds(),
                &mut Budget::new(micro()),
            )
            .unwrap()
        {
            TableRowCandidateStep::Candidate(i) => all.push(i),
            TableRowCandidateStep::Complete => return all,
            _ => panic!(),
        }
    }
}

#[test]
fn table_submenu_preserves_six_pages_and_all_new_actions_are_distinct_at_20_40() {
    for width in [20, 40] {
        let mut seen = std::collections::BTreeSet::new();
        for index in 0..16 {
            let mut ui = FoldUi::default();
            ui.open(0, None, Some(FoldId::Table(TableId(0))));
            ui.key(KeyCode::Char('t'));
            for _ in 0..index {
                ui.key(KeyCode::Down);
            }
            let s = ui.status(width).unwrap();
            assert!(s.contains("Esc"), "{s}");
            assert!(mdtui_tui::render::text::display_width(&s) <= width as usize);
            assert!(seen.insert(s.clone()), "{index} {s}");
        }
    }
}

#[test]
fn completed_deep_batch_preserves_80_column_outcome_before_optional_table_help() {
    use mdtui_tui::render::{
        checkpoint::RenderCheckpoint, objects::SharedLinks, text::display_width,
    };
    let mut d = Document::new();
    d.append(&format!("{}DEEP_READY\n\n", "> ".repeat(600)))
        .unwrap();
    let mut v = FoldView::<RenderCheckpoint>::with_state(&d, 1).unwrap();
    let links = SharedLinks::new(&d);
    let mut ui = FoldUi::default();
    ui.open(0, None, Some(FoldId::Container(ContainerId(0))));
    for key in [KeyCode::Right, KeyCode::Down, KeyCode::Down, KeyCode::Enter] {
        ui.key(key);
    }
    for _ in 0..10_000 {
        ui.advance(&d, &mut v, &links, 80, &mut Budget::new(micro()));
        if !ui.pending() {
            break;
        }
    }
    assert!(!ui.pending());
    let status = ui.status(80).unwrap();
    assert_eq!(d.container_count(), 600);
    let outcome = format!("{} visited, 600 changed", 600 + d.blocks().len());
    assert!(
        status.contains("collapse document") && status.contains(&outcome),
        "{status}"
    );
    assert!(!status.contains("Esc cancel"));
    assert_eq!(display_width(&status), 78); //33 action +18 help +27 outcome.
    assert_eq!(display_width(&status) + display_width(" t:rows"), 85);
    eprintln!(
        "STATUS80 width=78 optional_hint_total=85 source_blocks={} text={status}",
        d.blocks().len()
    );
    assert!(matches!(
        v.hidden_at(&d, 1200, &mut Budget::new(micro())).unwrap(),
        FoldLookup::Hidden(c) if c.id == FoldId::Container(ContainerId(0))
    ));
}

#[test]
fn bounded_numeric_prefix_keeps_20_column_controls_and_registration_identity() {
    let (mut r, mut p) = runtime("| H |\n| - |\n| A |\n| B |\n", 20, 12);
    action(&mut r, &mut p, 0);
    close(&mut r, &mut p);
    let id = info(&r, TableId(0))[0].id;
    action(&mut r, &mut p, 3);
    for ch in "1000001".chars() {
        r.fold_menu_key(KeyCode::Char(ch));
    }
    let text = draw(&mut r, &mut p);
    assert!(text.lines().last().unwrap().contains("Esc"), "{text}");
    r.fold_menu_key(KeyCode::Enter);
    let text = draw(&mut r, &mut p);
    assert!(text.contains("0..1000000 Esc"), "{text}");
    r.fold_menu_key(KeyCode::Backspace);
    r.fold_menu_key(KeyCode::Char('0'));
    r.fold_menu_key(KeyCode::Enter);
    let text = draw(&mut r, &mut p);
    let status = text.lines().last().unwrap();
    assert!(
        status.contains(&format!("R{}C", id.index())) && status.contains("Esc"),
        "{text}"
    );
    assert_eq!(info(&r, TableId(0))[0].id, id);
    assert_eq!(
        info(&r, TableId(0))[0].extent,
        TableRowExtent::Live { start: 1_000_001 }
    );
    close(&mut r, &mut p);
    action(&mut r, &mut p, 8);
    input(&mut r, &mut p, "1000000");
    let text = draw(&mut r, &mut p);
    assert!(text.contains("new rows=1000000 Esc"), "{text}");
    assert_eq!(
        r.viewport().folds().table_row_policy(),
        Some(TableRowPolicy {
            visible_body_rows: 1_000_000
        })
    );
    assert_eq!(info(&r, TableId(0))[0].id, id);
}

#[test]
fn actual_prefix_quick_numeric_stable_id_zero_budget_header_borders_pan_and_min_geometry() {
    let source = "| H0 | H1 |\r\n| :-- | --: |\r\n| A1 | B1 |\r\n| A2 | B2 |\r\n| A3 | B3 |\r\n";
    for width in [20, 40] {
        let (mut r, mut p) = runtime(source, width, 12);
        draw(&mut r, &mut p);
        open(&mut r, &mut p);
        r.fold_menu_key(KeyCode::Down);
        r.fold_menu_key(KeyCode::Enter);
        let before = r.viewport().folds().version();
        let mut z = Budget::new(WorkLimit::default());
        let mut wire = Vec::new();
        r.frame_to(&mut wire, &mut z).unwrap();
        assert_eq!(z.used(), WorkLimit::default());
        assert_eq!(r.viewport().folds().version(), before);
        let text = draw(&mut r, &mut p);
        assert!(
            text.contains("A1") && !text.contains("A2") && !text.contains("A3"),
            "{text}"
        );
        assert_eq!(text.matches("[+ rows").count(), 1);
        assert_eq!(text.matches('┌').count(), 1);
        assert_eq!(text.matches('└').count(), 1);
        assert_eq!(r.viewport().heights().total_rows(), 6);
        let ranges = info(&r, TableId(0));
        assert_eq!(ranges.len(), 1);
        let id = ranges[0].id;
        assert!(ranges[0].collapsed);
        assert_eq!(ranges[0].rows, Some(2..4));
        assert!(
            text.lines()
                .last()
                .unwrap()
                .contains(&format!("R{}C", id.index())),
            "{text}"
        );
        close(&mut r, &mut p);
        action(&mut r, &mut p, 3);
        input(&mut r, &mut p, "2");
        assert_eq!(info(&r, TableId(0))[0].id, id);
        let text = draw(&mut r, &mut p);
        assert!(
            text.contains("A1") && text.contains("A2") && !text.contains("A3"),
            "{text}"
        );
        assert_eq!(r.viewport().heights().total_rows(), 7);
        close(&mut r, &mut p);
        {
            let (d, v) = r.document_and_viewport_mut();
            v.set_table_policy(
                d,
                mdtui_tui::render::table_widths::TableWidthPolicy::new(3, Some(6)).unwrap(),
            )
            .unwrap();
            v.set_table_first_column(d, TableId(0), 1).unwrap();
        }
        let text = draw(&mut r, &mut p);
        assert!(text.contains("H1") && !text.contains("H0"), "{text}");
        assert_eq!(r.viewport().table_widths().first_column(TableId(0)), 1);
        r.resize(3, 20).unwrap();
        p = vt100::Parser::new(3u16.try_into().unwrap(), 20u16.try_into().unwrap(), 0);
        draw(&mut r, &mut p);
        r.resize(3, 1).unwrap();
        p = vt100::Parser::new(3u16.try_into().unwrap(), 1u16.try_into().unwrap(), 0);
        draw(&mut r, &mut p);
        assert_eq!(info(&r, TableId(0))[0].id, id);
    }
}

#[test]
fn actual_new_only_default_late_header_append_finish_and_live_modes() {
    let geometry = FrameGeometry {
        rows: 12,
        cols: 40,
        content_width: 40,
    };
    let mut r = FrameRuntime::new(geometry, LinkMode::Default).unwrap();
    let mut p = vt100::Parser::new(12u16.try_into().unwrap(), 40u16.try_into().unwrap(), 0);
    draw(&mut r, &mut p);
    add(&mut r, &mut p, "| OLD |\n| - |\n| OLD_BODY |\n");
    action(&mut r, &mut p, 6);
    close(&mut r, &mut p);
    assert!(info(&r, TableId(0)).is_empty());
    add(&mut r, &mut p, "\n| NEW |\n");
    assert!(r.document().table(TableId(1)).is_none());
    add(&mut r, &mut p, "| - |\n");
    let ranges = info(&r, TableId(1));
    assert_eq!(ranges.len(), 1);
    assert!(ranges[0].rows.is_none());
    assert!(!ranges[0].closed);
    let id = ranges[0].id;
    add(&mut r, &mut p, "| FIRST |\n| SECRET |\n");
    let text = draw(&mut r, &mut p);
    assert!(text.contains("FIRST") && !text.contains("SECRET"), "{text}");
    let ranges = info(&r, TableId(1));
    assert_eq!(ranges[0].id, id);
    assert_eq!(ranges[0].rows, Some(2..3));
    add(&mut r, &mut p, "| NEW_TAIL |\n");
    assert_eq!(info(&r, TableId(1))[0].rows, Some(2..4));
    if let Some(u) = r.finish_input().unwrap() {
        r.apply_update(&u).unwrap();
    }
    draw(&mut r, &mut p);
    assert!(info(&r, TableId(1))[0].closed);
    assert!(info(&r, TableId(0)).is_empty());
    assert_eq!(
        r.viewport().folds().table_row_policy(),
        Some(TableRowPolicy {
            visible_body_rows: 1
        })
    );
}

fn copy(r: &mut FrameRuntime, p: &mut vt100::Parser, key: char) -> String {
    copy_with_bound(r, p, key, 100_000)
}
fn copy_with_bound(
    r: &mut FrameRuntime,
    p: &mut vt100::Parser,
    key: char,
    frames: usize,
) -> String {
    r.reading_key(KeyCode::Char(key), KeyModifiers::NONE, true)
        .unwrap();
    let mut wire = Vec::new();
    let (mut steps, mut queries, mut objects, mut bytes) = (0, 0, 0, 0);
    for frame in 0..frames {
        let mut b = Budget::new(micro());
        r.advance_reading(&mut wire, &mut b);
        assert!(b.used().queries <= 1 && b.used().objects <= 1 && b.used().bytes <= 6);
        steps = frame + 1;
        queries += b.used().queries;
        objects += b.used().objects;
        bytes += b.used().bytes;
        if !r.reading().copy_pending() {
            break;
        }
    }
    assert!(!r.reading().copy_pending());
    if frames > 100_000 {
        eprintln!(
            "COPY64K frames={steps} derived_bound={frames} queries={queries} objects={objects} charged_bytes={bytes} packet_bytes={}",
            wire.len()
        );
    }
    let payload = wire
        .strip_prefix(b"\x1b]52;c;")
        .unwrap()
        .strip_suffix(b"\x07")
        .unwrap();
    let mut out = Vec::new();
    let mut acc = 0u32;
    let mut bits = 0;
    for b in payload.iter().take_while(|b| **b != b'=') {
        acc = (acc << 6)
            | b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/"
                .iter()
                .position(|c| c == b)
                .unwrap() as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    draw(r, p);
    String::from_utf8(out).unwrap()
}

#[test]
fn actual_typed_crossing_ph_has_full_crlf_source_unique_visible_copy_and_true_covering_search() {
    let source = "| H |\r\n| - |\r\n| A1 |\r\n| B2 |\r\n| C3 |\r\n| D4 |\r\n";
    let (mut r, mut p) = runtime(source, 40, 12);
    let (a, b, expected);
    {
        let (d, v) = r.document_and_viewport_mut();
        let table = d.table(TableId(0)).unwrap();
        expected = Span::new(
            table.row(1).unwrap().source.start,
            table.row(4).unwrap().source.end,
        );
        let mut ids = Vec::new();
        for rows in [1..3, 2..5] {
            let mut task =
                TableRowRegistration::fixed_overlay(d, v.folds(), TableId(0), rows).unwrap();
            let id = task
                .advance(d, v.folds_mut(), &mut Budget::new(micro()))
                .unwrap()
                .id
                .unwrap();
            v.folds_mut().set_collapsed(d, id, true).unwrap();
            ids.push(id);
        }
        a = ids[0];
        b = ids[1];
        v.fold_changed(d);
    }
    let text = draw(&mut r, &mut p);
    assert_eq!(text.matches("[+ rows").count(), 1, "{text}");
    assert_eq!(r.fold_hits().len(), 1);
    assert_eq!(r.fold_hits()[0].id, a);
    assert_eq!(r.fold_hits()[0].hidden_envelope, expected);
    assert!(r.fold_hits()[0].registered.end < expected.end);
    r.reading_key(KeyCode::Char('v'), KeyModifiers::NONE, true)
        .unwrap();
    r.reading_key(KeyCode::Down, KeyModifiers::NONE, true)
        .unwrap();
    assert_eq!(r.reading().selection.range(), Some(expected));
    let visible = copy(&mut r, &mut p, 'y');
    assert_eq!(visible.matches("[+ rows").count(), 1);
    assert!(!visible.contains("A1") && !visible.contains("D4"));
    assert_eq!(copy(&mut r, &mut p, 'Y'), &source[expected.range()]);
    if r.reading().selection.active() {
        r.reading_key(KeyCode::Char('v'), KeyModifiers::NONE, true)
            .unwrap();
    }
    r.reading_key(KeyCode::Char('/'), KeyModifiers::NONE, true)
        .unwrap();
    for c in "D4".chars() {
        r.reading_key(KeyCode::Char(c), KeyModifiers::NONE, true)
            .unwrap();
    }
    r.reading_key(KeyCode::Enter, KeyModifiers::NONE, true)
        .unwrap();
    let text = draw(&mut r, &mut p);
    assert!(text.lines().next().unwrap().contains("D4"), "{text}");
    assert!(r.viewport().folds().is_collapsed(a));
    assert!(!r.viewport().folds().is_collapsed(b));
}

#[test]
fn typed_ph_source_copy_preserves_exact_64k_boundary_and_refuses_one_more_byte() {
    for bytes in [64 * 1024, 64 * 1024 + 1] {
        let body = format!("| {} |\r\n", "x".repeat(bytes - 6));
        let source = format!("| H |\r\n| - |\r\n{body}");
        let (mut r, mut p) = runtime(&source, 40, 12);
        action(&mut r, &mut p, 0);
        close(&mut r, &mut p);
        let expected = r
            .document()
            .table(TableId(0))
            .unwrap()
            .row(1)
            .unwrap()
            .source;
        assert_eq!(expected.len(), bytes);
        assert_eq!(r.fold_hits().len(), 1);
        assert_eq!(r.fold_hits()[0].hidden_envelope, expected);
        r.reading_key(KeyCode::Char('v'), KeyModifiers::NONE, true)
            .unwrap();
        r.reading_key(KeyCode::Down, KeyModifiers::NONE, true)
            .unwrap();
        assert_eq!(r.reading().selection.range(), Some(expected));
        let visible = copy(&mut r, &mut p, 'y');
        assert_eq!(visible.matches("[+ rows").count(), 1);
        assert!(visible.len() < 256);
        if bytes == 64 * 1024 {
            // The actual clipboard protocol pays one scalar/object, one
            // object per three encoded bytes, then the full OSC52 byte credit.
            // At1q/1o/6B,64KiB needs101947+ steps, beyond the small-copy helper.
            let frames = bytes + bytes.div_ceil(3) + (bytes.div_ceil(3) * 4 + 8).div_ceil(6) + 2;
            assert_eq!(copy_with_bound(&mut r, &mut p, 'Y', frames), body);
        } else {
            r.reading_key(KeyCode::Char('Y'), KeyModifiers::NONE, true)
                .unwrap();
            assert!(!r.reading().copy_pending());
            assert!(r.reading().message().contains("64 KiB"));
        }
    }
}

#[test]
fn typed_live_reveal_refines_actual_target_row_after_a_giant_cell() {
    let source = format!(
        "> | H |\r\n> | - |\r\n> | {} |\r\n> | EXACT_TARGET |\r\n> | AFTER |\r\n",
        "x".repeat(64 * 1024)
    );
    let (mut r, mut p) = runtime(&source, 40, 12);
    action(&mut r, &mut p, 0);
    close(&mut r, &mut p);
    let id = info(&r, TableId(0))[0].id;
    assert!(!draw(&mut r, &mut p).contains("EXACT_TARGET"));
    r.reading_key(KeyCode::Char('/'), KeyModifiers::NONE, true)
        .unwrap();
    for ch in "EXACT_TARGET".chars() {
        r.reading_key(KeyCode::Char(ch), KeyModifiers::NONE, true)
            .unwrap();
    }
    r.reading_key(KeyCode::Enter, KeyModifiers::NONE, true)
        .unwrap();
    let text = draw(&mut r, &mut p);
    assert!(
        text.lines().next().unwrap().contains("EXACT_TARGET"),
        "{text}"
    );
    assert!(!r.viewport().folds().is_collapsed(FoldId::TableRange(id)));
    let at = source.find("EXACT_TARGET").unwrap();
    let row = r.document().table(TableId(0)).unwrap().row(2).unwrap();
    assert!(row.source.start <= at && at < row.source.end);
    let top = r.top_source().unwrap();
    assert!(row.source.start <= top && top < row.source.end);
}

#[test]
fn actual_selection_live_seal_retarget_remove_and_all_id_enumeration() {
    let source = "| H |\n| - |\n| FIRST |\n| SECRET |\n| LAST |\n";
    let (mut r, mut p) = runtime(source, 40, 12);
    draw(&mut r, &mut p);
    r.reading_key(KeyCode::Char('v'), KeyModifiers::NONE, true)
        .unwrap();
    for _ in 0..2 {
        r.reading_key(KeyCode::Down, KeyModifiers::NONE, true)
            .unwrap();
    }
    let text = action(&mut r, &mut p, 10);
    assert!(!text.contains("SECRET") && !text.contains("LAST"), "{text}");
    let ranges = info(&r, TableId(0));
    assert_eq!(ranges.len(), 1);
    let id = ranges[0].id;
    assert!(matches!(
        ranges[0].extent,
        TableRowExtent::Live { start: 2 }
    ));
    // Keep the selected typed ID: t submenu navigation itself never rebinds it.
    r.fold_menu_key(KeyCode::Home);
    for _ in 0..11 {
        r.fold_menu_key(KeyCode::Down);
    }
    r.fold_menu_key(KeyCode::Enter);
    draw(&mut r, &mut p);
    assert_eq!(
        info(&r, TableId(0))[0].extent,
        TableRowExtent::Fixed { start: 2, end: 4 }
    );
    r.fold_menu_key(KeyCode::Home);
    for _ in 0..12 {
        r.fold_menu_key(KeyCode::Down);
    }
    r.fold_menu_key(KeyCode::Enter);
    draw(&mut r, &mut p);
    assert_eq!(info(&r, TableId(0))[0].id, id);
    assert_eq!(
        info(&r, TableId(0))[0].extent,
        TableRowExtent::Fixed { start: 2, end: 3 }
    );
    r.fold_menu_key(KeyCode::End);
    r.fold_menu_key(KeyCode::Enter);
    let text = draw(&mut r, &mut p);
    assert!(text.contains(&format!("R{}C", id.index())), "{text}");
    r.fold_menu_key(KeyCode::Home);
    for _ in 0..13 {
        r.fold_menu_key(KeyCode::Down);
    }
    r.fold_menu_key(KeyCode::Enter);
    let text = draw(&mut r, &mut p);
    assert!(text.contains("SECRET") && text.contains("LAST"), "{text}");
    assert!(info(&r, TableId(0)).is_empty());
}

#[test]
fn actual_menu_selects_true_covering_ids_and_restarts_cursor_after_resize() {
    let (mut r, _) = runtime("| H |\n| - |\n| A1 |\n| B2 |\n| C3 |\n| D4 |\n", 40, 12);
    let (d, v) = r.document_and_viewport_mut();
    let mut ids = Vec::new();
    for rows in [1..3, 2..5] {
        let mut task = TableRowRegistration::fixed_overlay(d, v.folds(), TableId(0), rows).unwrap();
        let id = task
            .advance(d, v.folds_mut(), &mut Budget::new(micro()))
            .unwrap()
            .id
            .unwrap();
        v.folds_mut().set_collapsed(d, id, true).unwrap();
        ids.push(id);
    }
    let at = d.table(TableId(0)).unwrap().row(3).unwrap().source.start;
    assert!(matches!(
        v.folds().hidden_at(d, at, &mut Budget::new(micro())).unwrap(),
        FoldLookup::Hidden(c) if c.id == ids[0]
    ));
    let links = v.shared_links().clone();
    let mut ui = FoldUi::default();
    ui.open(at, None, Some(ids[0]));
    ui.key(KeyCode::Char('t'));
    for _ in 0..14 {
        ui.key(KeyCode::Down);
    }
    let choose = |ui: &mut FoldUi, v: &mut mdtui_tui::render::viewport::ViewportState| {
        ui.key(KeyCode::Enter);
        for _ in 0..10 {
            ui.advance(d, v.folds_mut(), &links, 40, &mut Budget::new(micro()));
            if !ui.pending() {
                return;
            }
        }
        panic!("paid candidate selection stalled");
    };
    choose(&mut ui, v);
    let FoldId::TableRange(b) = ids[1] else {
        panic!()
    };
    assert!(ui.status(40).unwrap().contains(&format!("R{}C", b.index())));
    v.folds_mut().reset(1).unwrap();
    ui.resize();
    choose(&mut ui, v);
    assert!(ui.status(20).unwrap().contains(&format!("R{}C", b.index())));
    ui.key(KeyCode::End);
    choose(&mut ui, v);
    let FoldId::TableRange(a) = ids[0] else {
        panic!()
    };
    assert!(ui.status(40).unwrap().contains(&format!("R{}C", a.index())));
    choose(&mut ui, v);
    assert!(ui.status(40).unwrap().contains(&format!("R{}C", b.index())));
}

fn wide_sources() -> [String; 2] {
    [
        format!(
            "{}\n{}\n| x |\n| y |\n",
            "| h ".repeat(100_000),
            "| - ".repeat(100_000)
        ),
        format!(
            "- seed\n\n\tH | V\n  - | -\n\tFIRST | x\n\t{} | hidden\n\tLAST | z\n",
            "界".repeat(350_000)
        ),
    ]
}
fn assert_wide_window(
    d: &Document,
    v: &mut mdtui_tui::render::viewport::ViewportState,
    cols: usize,
) {
    use mdtui_tui::render::{table_widths::TableWidthPolicy, viewport::ViewportFrameStatus};
    let table = TableId(0);
    v.set_table_policy(d, TableWidthPolicy::new(3, Some(8)).unwrap())
        .unwrap();
    if cols > 10 {
        v.set_table_first_column(d, table, cols - 2).unwrap();
    }
    let mut task = TableRowRegistration::prefix(d, v.folds(), table, 0).unwrap();
    task.advance(d, v.folds_mut(), &mut Budget::new(micro()))
        .unwrap();
    v.fold_changed(d);
    let mut source_bytes = 0;
    let mut queries = 0;
    let mut objects = 0;
    let mut done = false;
    let query_start = std::time::Instant::now();
    for frame in 0..200_000 {
        let mut b = Budget::new(micro());
        let f = v.frame(d, &mut b).unwrap();
        source_bytes += b.used().bytes;
        queries += b.used().queries;
        objects += b.used().objects;
        if frame % 50000 == 49999 {
            eprintln!(
                "wide probe columns={cols} frames={} status={:?} bytes={source_bytes} shape_ready={} width_work={:?} frame_stats={:?}",
                frame + 1,
                f.status,
                v.shared_links().shape_ready(),
                v.table_widths().work(),
                f.stats
            );
        }
        if matches!(
            f.status,
            ViewportFrameStatus::Ready | ViewportFrameStatus::EndOfDocument
        ) {
            let text = f
                .lines
                .iter()
                .map(|l| {
                    l.cells
                        .iter()
                        .map(|c| c.text.collect_text())
                        .collect::<String>()
                })
                .collect::<Vec<_>>()
                .join("\n");
            assert_eq!(text.matches("[+ rows").count(), 1, "{text}");
            assert_eq!(text.matches('┌').count(), 1, "{text}");
            assert_eq!(text.matches('└').count(), 1, "{text}");
            assert!(!text.contains("hidden") && !text.contains("LAST"), "{text}");
            // This counter contains terminal rows, including decoration:
            // one top border + one header + one separator + one bottom.
            assert_eq!(f.stats.layout.table_rows_prepared, 4, "{:?}", f.stats);
            if cols > 10 {
                assert_eq!(v.heights().total_rows(), 5);
            }
            eprintln!(
                "QUERY columns={cols} frames={} queries={queries} objects={objects} source_bytes={source_bytes} cp_entries={} table_terminal_rows={} total_height={} elapsed_ms={} width_work={:?}",
                frame + 1,
                v.checkpoints().len(),
                f.stats.layout.table_rows_prepared,
                v.heights().total_rows(),
                query_start.elapsed().as_millis(),
                v.table_widths().work()
            );
            done = true;
            break;
        }
    }
    assert!(
        done,
        "columns={cols}, source bytes={source_bytes}, width work={:?}",
        v.table_widths().work()
    );
    assert!(source_bytes < 16_384, "source bytes={source_bytes}");
    let work = v.table_widths().work();
    assert!(work.column_metadata_reads < 64, "{work:?}");
    assert!(work.measurement_source_bytes < 4096, "{work:?}");
    if cols > 10 {
        assert_eq!(v.table_widths().first_column(table), cols - 2);
    }
}

#[test]
fn actual_input_owner_wide_sparse_and_giant_row_policy_uses_bounded_frame_windows() {
    for source in wide_sources() {
        let input_start = std::time::Instant::now();
        let mut input = FrameRuntime::from_finished_source(
            &source,
            FrameGeometry {
                rows: 12,
                cols: 40,
                content_width: 40,
            },
            LinkMode::Default,
        )
        .unwrap();
        let input_elapsed = input_start.elapsed();
        let (d, v) = input.document_and_viewport_mut();
        let cols = d.table(TableId(0)).unwrap().columns.len();
        let input_work = v.shared_links().shape().unwrap().work();
        eprintln!(
            "INPUT owner source_bytes={} core_table_bytes={} input_shape_decoded={} input_shape_indexed={} shape_ready={} source_cursor={} elapsed_ms={} columns={cols} core_work={:?} shape_work={input_work:?}",
            d.source().len(),
            d.work().table_bytes,
            input_work.source_decode_bytes,
            input_work.source_indexed_bytes,
            v.shared_links().shape_ready(),
            v.shared_links().shape().unwrap().source_len(),
            input_elapsed.as_millis(),
            d.work()
        );
        assert!(v.shared_links().shape_ready());
        assert_eq!(
            v.shared_links().shape().unwrap().source_len(),
            d.source().len()
        );
        assert_wide_window(d, v, cols);
    }
}

#[test]
fn table_default_queue_cancel_is_paid_and_never_cancels_future_recognition() {
    use mdtui_tui::render::{checkpoint::RenderCheckpoint, objects::SharedLinks};
    let mut d = Document::new();
    let mut v = FoldView::<RenderCheckpoint>::with_state(&d, 1).unwrap();
    let mut ui = FoldUi::default();
    v.set_table_row_policy(
        &d,
        Some(TableRowPolicy {
            visible_body_rows: 0,
        }),
        &mut Budget::new(micro()),
    )
    .unwrap();
    for _ in 0..2 {
        let u = d.append("| H |\n| - |\n| a |\n\n").unwrap();
        v.apply(&d, &u).unwrap();
        ui.on_update(&d, &v, &u).unwrap();
    }
    let links = SharedLinks::new(&d);
    let before = v.version();
    let mut z = Budget::new(WorkLimit::default());
    ui.advance(&d, &mut v, &links, 20, &mut z);
    assert_eq!(v.version(), before);
    assert_eq!(v.table_row_work().registrations, 0);
    assert_eq!(z.used(), WorkLimit::default());
    ui.advance(&d, &mut v, &links, 20, &mut Budget::new(micro()));
    assert_eq!(v.table_row_work().registrations, 1);
    assert!(ui.key(KeyCode::Esc));
    let before = v.version();
    ui.advance(
        &d,
        &mut v,
        &links,
        20,
        &mut Budget::new(WorkLimit::default()),
    );
    assert_eq!(v.version(), before);
    assert!(ui.pending());
    // The first one-table task completed already, so only the remaining queued
    // task retires. It consumes the current shared query/object, no view edit.
    let mut b = Budget::new(micro());
    ui.advance(&d, &mut v, &links, 20, &mut b);
    assert_eq!(b.used().queries, 1);
    assert_eq!(v.version(), before);
    assert!(!ui.pending());
    let u = d.append("| N |\n| - |\n| FUTURE |\n").unwrap();
    v.apply(&d, &u).unwrap();
    ui.on_update(&d, &v, &u).unwrap();
    ui.advance(&d, &mut v, &links, 20, &mut Budget::new(micro()));
    assert_eq!(v.table_row_work().registrations, 2);
    let mut q = TableRowCandidates::all(&d, &v, TableId(2)).unwrap();
    assert!(
        matches!(q.next(&d,&v,&mut Budget::new(micro())).unwrap(),TableRowCandidateStep::Candidate(i) if i.collapsed)
    );
}

#[test]
fn typed_live_end_follow_frozen_cut_and_history_commit_only_after_exact_restore() {
    use mdtui_tui::render::frame::HistoryRestoreStatus;
    let mut r = FrameRuntime::new(
        FrameGeometry {
            rows: 8,
            cols: 40,
            content_width: 40,
        },
        LinkMode::Default,
    )
    .unwrap();
    let mut p = vt100::Parser::new(8u16.try_into().unwrap(), 40u16.try_into().unwrap(), 0);
    draw(&mut r, &mut p);
    add(&mut r, &mut p, "| H |\n| - |\n| FIRST |\n| OLD |\n");
    action(&mut r, &mut p, 0);
    close(&mut r, &mut p);
    let id = info(&r, TableId(0))[0].id;
    r.seek_end(true).unwrap();
    draw(&mut r, &mut p);
    assert!(r.viewport().following());
    add(&mut r, &mut p, "| FOLLOW_HIDDEN |\n");
    assert!(r.viewport().following());
    assert_eq!(r.fold_hits().len(), 1);
    assert_eq!(
        r.fold_hits()[0].hidden_envelope.end,
        r.document().source().len()
    );
    r.seek_end(false).unwrap();
    draw(&mut r, &mut p);
    let cut = r.document().source().len();
    add(&mut r, &mut p, "| NEW_AFTER_CUT |\n");
    assert!(!r.viewport().following());
    {
        let (d, v) = r.document_and_viewport_mut();
        v.folds_mut()
            .set_collapsed(d, FoldId::TableRange(id), false)
            .unwrap();
        v.fold_changed(d);
    }
    let text = draw(&mut r, &mut p);
    assert!(
        text.contains("FOLLOW_HIDDEN") && !text.contains("NEW_AFTER_CUT"),
        "{text}"
    );
    assert_eq!(&r.document().source()[cut..], "| NEW_AFTER_CUT |\n");
    let earlier = r.navigation_position().unwrap();
    r.reading_key(KeyCode::Char('/'), KeyModifiers::NONE, true)
        .unwrap();
    for ch in "NEW_AFTER_CUT".chars() {
        r.reading_key(KeyCode::Char(ch), KeyModifiers::NONE, true)
            .unwrap();
    }
    r.reading_key(KeyCode::Enter, KeyModifiers::NONE, true)
        .unwrap();
    let text = draw(&mut r, &mut p);
    assert!(
        text.lines().next().unwrap().contains("NEW_AFTER_CUT"),
        "{text}"
    );
    let saved = r.navigation_position().unwrap();
    assert!(matches!(
        r.restore_navigation_position(earlier, &mut Budget::new(WorkLimit::unlimited()))
            .unwrap(),
        HistoryRestoreStatus::Ready { .. }
    ));
    draw(&mut r, &mut p);
    let before = r.navigation_position();
    let result = r
        .restore_navigation_position(saved, &mut Budget::new(WorkLimit::default()))
        .unwrap();
    assert_eq!(result, HistoryRestoreStatus::BudgetExhausted);
    assert_eq!(r.navigation_position(), before);
    assert!(r.history_restore_result().is_none());
    let text = draw(&mut r, &mut p);
    assert!(
        text.lines().next().unwrap().contains("NEW_AFTER_CUT"),
        "{text}"
    );
    assert!(
        matches!(r.history_restore_result(),Some((requested,HistoryRestoreStatus::Ready{..})) if requested==saved)
    );
}
