mod support;

use std::time::Duration;

use crossterm::event::{KeyCode, KeyModifiers};
use mdtui_tui::app::{AppState, FocusTarget};
use mdtui_tui::cli::LinkMode;
use mdtui_tui::render::frame::{
    BEGIN_SYNCHRONIZED_UPDATE, END_SYNCHRONIZED_UPDATE, FrameGeometry, FrameRuntime,
};
use mdtui_tui::screen::StyleRole;
use mdtui_tui::stream::{IngestResult, MAX_BATCH_DELAY, StreamEngine};
use mdtui3::{Budget, LinkId, WorkLimit};
use support::terminal::{FrameOutcome, Vt100Writer, assert_terminal_matches_screen};

fn geometry(rows: u16, cols: u16) -> FrameGeometry {
    FrameGeometry {
        rows,
        cols,
        content_width: cols,
    }
}

fn unlimited() -> Budget {
    Budget::new(WorkLimit::unlimited())
}

fn commit_due(engine: &mut StreamEngine, writer: &mut Vt100Writer, now: Duration) {
    writer.begin_frame();
    match engine.render_due(writer, &mut unlimited(), now) {
        Ok(Some(commit)) if commit.committed => {
            writer.finish_frame(FrameOutcome::Committed);
        }
        Ok(_) => {
            writer.finish_frame(FrameOutcome::NoOutput);
            panic!("expected a committed frame");
        }
        Err(error) => {
            writer.finish_frame(FrameOutcome::Failed);
            panic!("unexpected frame error: {error}");
        }
    }
}

fn settle(engine: &mut StreamEngine, writer: &mut Vt100Writer, now: Duration) {
    if engine.calibration_pending() {
        for _ in 0..100_000 {
            if engine
                .advance_calibration(&mut unlimited())
                .expect("calibration")
                .is_some()
            {
                break;
            }
        }
    }
    if engine.due(now) {
        commit_due(engine, writer, now);
    }
}

fn finished_engine(source: &str, rows: u16, cols: u16, writer: &mut Vt100Writer) -> StreamEngine {
    let mut engine = StreamEngine::new(
        FrameRuntime::new(geometry(rows, cols), LinkMode::Default).expect("runtime"),
    );
    assert_eq!(
        engine
            .ingest_data(source.as_bytes(), Duration::ZERO)
            .expect("ingest"),
        IngestResult::FirstFrame
    );
    commit_due(&mut engine, writer, Duration::ZERO);
    assert!(engine.finish_input(Duration::ZERO).expect("finish"));
    settle(&mut engine, writer, Duration::ZERO);
    engine
}

fn frame_bytes(writer: &Vt100Writer, frame: usize) -> Vec<u8> {
    writer
        .frame_writes(frame)
        .iter()
        .flat_map(|write| write.bytes.iter().copied())
        .collect()
}

fn assert_incremental_frame_protocol(bytes: &[u8]) {
    assert!(!bytes.windows(4).any(|window| window == b"\x1b[2J"));
    assert!(!bytes.windows(3).any(|window| window == b"\x1b[H"));
    assert!(!bytes.windows(3).any(|window| window == b"\x1b[K"));
    assert_eq!(
        bytes
            .windows(BEGIN_SYNCHRONIZED_UPDATE.len())
            .filter(|window| *window == BEGIN_SYNCHRONIZED_UPDATE)
            .count(),
        1
    );
    assert_eq!(
        bytes
            .windows(END_SYNCHRONIZED_UPDATE.len())
            .filter(|window| *window == END_SYNCHRONIZED_UPDATE)
            .count(),
        1
    );
}

fn assert_focus_style(engine: &StreamEngine, id: LinkId, focused: bool) {
    let hits: Vec<_> = engine
        .runtime()
        .link_table()
        .expect("link table")
        .hits_for(id)
        .copied()
        .collect();
    assert!(!hits.is_empty());
    for hit in hits {
        for column in hit.x0..hit.x1 {
            let cell = engine
                .runtime()
                .screen()
                .front_cell(hit.row, column)
                .expect("hit cell");
            assert_eq!(cell.link_id(), Some(id));
            assert_eq!(
                cell.style().contains(StyleRole::LINK_FOCUS),
                focused,
                "row {} column {column}",
                hit.row
            );
        }
    }
}

#[test]
fn link_focus_cycles_visible_links_and_highlights_all_occurrences() {
    let source = "Start [alpha](https://alpha.example/) and [beta](https://beta.example/) end.\n\nLast [gamma](https://gamma.example/).\n";
    let mut writer = Vt100Writer::new(12, 80);
    let mut engine = finished_engine(source, 12, 80, &mut writer);
    let mut app = AppState::default();
    let ids: Vec<_> = engine
        .runtime()
        .link_table()
        .expect("link table")
        .links()
        .to_vec();
    assert_eq!(ids.len(), 3);
    let text_before = writer.terminal().text();

    let entered = app
        .handle_key(&mut engine, KeyCode::Tab, KeyModifiers::NONE)
        .expect("enter links");
    assert!(entered.redraw);
    assert_eq!(app.focus(), FocusTarget::Link);
    assert_eq!(engine.link_focus(), Some(ids[0]));
    settle(&mut engine, &mut writer, Duration::ZERO);
    assert_focus_style(&engine, ids[0], true);
    assert_eq!(writer.terminal().text(), text_before);
    assert_terminal_matches_screen(writer.terminal(), engine.runtime().screen());
    let frame = writer.frames().len() - 1;
    let highlighted = frame_bytes(&writer, frame);
    assert_incremental_frame_protocol(&highlighted);
    assert!(
        highlighted
            .windows(b"https://alpha.example/".len())
            .any(|window| window == b"https://alpha.example/")
    );

    let next = app
        .handle_key(&mut engine, KeyCode::Tab, KeyModifiers::NONE)
        .expect("next link");
    assert!(next.redraw);
    assert_eq!(app.focus(), FocusTarget::Link);
    assert_eq!(engine.link_focus(), Some(ids[1]));
    settle(&mut engine, &mut writer, Duration::ZERO);
    assert_focus_style(&engine, ids[0], false);
    assert_focus_style(&engine, ids[1], true);
    assert_eq!(writer.terminal().text(), text_before);

    app.handle_key(&mut engine, KeyCode::BackTab, KeyModifiers::NONE)
        .expect("previous link");
    assert_eq!(engine.link_focus(), Some(ids[0]));
    settle(&mut engine, &mut writer, Duration::ZERO);
    assert_focus_style(&engine, ids[0], true);

    let activation = app
        .handle_key(&mut engine, KeyCode::Enter, KeyModifiers::NONE)
        .expect("activate link");
    assert_eq!(activation.activated, Some(FocusTarget::Link));
    assert_eq!(activation.activated_link, Some(ids[0]));
    assert!(!activation.redraw);
    assert_eq!(
        activation
            .fragment_activation
            .expect("external activation status")
            .status,
        mdtui_tui::fragment::FragmentActivationStatus::External
    );
    assert!(app.history().is_empty());
    assert_terminal_matches_screen(writer.terminal(), engine.runtime().screen());
}

#[test]
fn wrapped_link_is_one_target_and_every_visible_run_is_highlighted() {
    let source = "[abcdefghijklm nopqrstuvwxyz](https://wrapped.example/)\n";
    let mut writer = Vt100Writer::new(10, 16);
    let mut engine = finished_engine(source, 10, 16, &mut writer);
    let table = engine.runtime().link_table().expect("link table");
    assert_eq!(table.len(), 1);
    let id = table.first().expect("wrapped link");
    assert!(table.hits_for(id).count() >= 2);
    let rows: std::collections::BTreeSet<_> = table.hits_for(id).map(|hit| hit.row).collect();
    assert!(rows.len() >= 2);
    let mut app = AppState::default();
    app.handle_key(&mut engine, KeyCode::Tab, KeyModifiers::NONE)
        .expect("enter link");
    settle(&mut engine, &mut writer, Duration::ZERO);
    assert_focus_style(&engine, id, true);

    let same = app
        .handle_key(&mut engine, KeyCode::Tab, KeyModifiers::NONE)
        .expect("single link cycle");
    assert!(!same.redraw);
    assert_eq!(engine.link_focus(), Some(id));
}

#[test]
fn wide_link_hits_use_display_cell_widths() {
    let source = "[中🙂x](https://wide.example/)\n";
    let mut writer = Vt100Writer::new(8, 80);
    let mut engine = finished_engine(source, 8, 80, &mut writer);
    let table = engine.runtime().link_table().expect("link table");
    let hit = table.hits().first().copied().expect("wide link hit");
    assert_eq!(table.links(), &[hit.id]);
    assert_eq!(hit.row, 0);
    assert_eq!(hit.width(), 5);
    assert_eq!(table.hits().len(), 1);
    let mut app = AppState::default();
    app.handle_key(&mut engine, KeyCode::Tab, KeyModifiers::NONE)
        .expect("focus wide link");
    settle(&mut engine, &mut writer, Duration::ZERO);
    assert_focus_style(&engine, hit.id, true);
}

#[test]
fn table_links_are_absent_from_the_hit_table_and_focus_cycle() {
    let source = "| Link | Value |\n| --- | --- |\n| [table](https://table.example/) | x |\n";
    let mut writer = Vt100Writer::new(10, 80);
    let mut engine = finished_engine(source, 10, 80, &mut writer);
    assert!(
        engine
            .runtime()
            .link_table()
            .expect("link table")
            .is_empty()
    );
    for row in 0..10 {
        for column in 0..80 {
            assert_eq!(
                engine
                    .runtime()
                    .screen()
                    .front_cell(row, column)
                    .expect("cell")
                    .link_id(),
                None
            );
        }
    }
    let mut app = AppState::default();
    let tab = app
        .handle_key(&mut engine, KeyCode::Tab, KeyModifiers::NONE)
        .expect("no table links");
    assert!(!tab.redraw);
    assert_eq!(app.focus(), FocusTarget::Body);
    assert_eq!(engine.link_focus(), None);
}

#[test]
fn table_link_is_skipped_when_a_body_link_is_focusable() {
    let source = "Body [outside](https://outside.example/).\n\n| Link | Value |\n| --- | --- |\n| [table](https://table.example/) | x |\n";
    let mut writer = Vt100Writer::new(16, 80);
    let mut engine = finished_engine(source, 16, 80, &mut writer);
    let table = engine.runtime().link_table().expect("link table");
    assert_eq!(table.len(), 1);
    let id = table.first().expect("outside link");
    assert_eq!(
        engine.runtime().document().link_destination(id),
        Some("https://outside.example/")
    );
    let mut app = AppState::default();
    app.handle_key(&mut engine, KeyCode::Tab, KeyModifiers::NONE)
        .expect("focus body link");
    assert_eq!(engine.link_focus(), Some(id));
    settle(&mut engine, &mut writer, Duration::ZERO);
    assert_focus_style(&engine, id, true);
}

#[test]
fn sidebar_toggle_rebuilds_body_only_hits_and_keeps_visible_focus() {
    let source = "# Heading\n\nBefore [body link](https://body.example/) after.\n";
    let mut writer = Vt100Writer::new(20, 120);
    let mut engine = finished_engine(source, 20, 120, &mut writer);
    let mut app = AppState::default();
    app.handle_key(&mut engine, KeyCode::Tab, KeyModifiers::NONE)
        .expect("enter link");
    settle(&mut engine, &mut writer, Duration::ZERO);
    let focused = engine.link_focus().expect("body link focus");

    app.handle_key(&mut engine, KeyCode::Char('s'), KeyModifiers::NONE)
        .expect("open sidebar");
    assert!(engine.link_table().is_none());
    settle(&mut engine, &mut writer, Duration::ZERO);
    let table = engine.link_table().expect("rebuilt link table");
    assert_eq!(engine.link_focus(), Some(focused));
    assert!(!table.hits().is_empty());
    for hit in table.hits() {
        assert!(hit.x0 >= engine.runtime().content_x());
        assert!(hit.x1 <= engine.runtime().content_x() + engine.runtime().geometry().content_width);
    }
    assert_focus_style(&engine, focused, true);
    assert_terminal_matches_screen(writer.terminal(), engine.runtime().screen());
}

#[test]
fn scroll_resize_and_append_rebuild_or_clear_stale_link_focus() {
    let mut source = String::from("Top [first](https://first.example/).\n\n");
    for index in 0..80 {
        source.push_str(&format!("paragraph {index:02} with ordinary text\n\n"));
    }
    source.push_str("Bottom [last](https://last.example/).\n");
    let mut writer = Vt100Writer::new(20, 80);
    let mut engine = finished_engine(&source, 20, 80, &mut writer);
    let mut app = AppState::default();
    app.handle_key(&mut engine, KeyCode::Tab, KeyModifiers::NONE)
        .expect("enter link");
    settle(&mut engine, &mut writer, Duration::ZERO);
    assert!(engine.link_focus().is_some());

    app.handle_key(&mut engine, KeyCode::End, KeyModifiers::NONE)
        .expect("scroll links away");
    assert!(engine.link_table().is_none());
    settle(&mut engine, &mut writer, Duration::ZERO);
    assert_eq!(engine.link_focus(), None);
    assert_eq!(app.focus(), FocusTarget::Link);
    let table = engine.runtime().link_table().expect("rebuilt");
    assert!(!table.is_empty());
    assert!(table.contains(table.last().expect("bottom link")));

    let mut runtime = FrameRuntime::from_finished_source(
        "first [one](https://one.example/)\n\nsecond\n\nthird [two](https://two.example/)\n",
        geometry(10, 80),
        LinkMode::Default,
    )
    .expect("runtime");
    let mut output = Vec::new();
    runtime
        .frame_to(&mut output, &mut unlimited())
        .expect("initial frame");
    assert!(runtime.enter_link_focus(false).is_some());
    let stale = runtime.link_focus().expect("bottom focus");
    assert!(runtime.resize(2, 80).expect("resize").height_changed);
    assert!(runtime.link_table().is_none());
    runtime
        .frame_to(&mut output, &mut unlimited())
        .expect("resize frame");
    assert_eq!(runtime.link_focus(), None);
    assert!(!runtime.link_table().expect("resize table").contains(stale));

    let mut engine =
        StreamEngine::new(FrameRuntime::new(geometry(12, 80), LinkMode::Default).expect("runtime"));
    let mut writer = Vt100Writer::new(12, 80);
    assert_eq!(
        engine
            .ingest_data(
                b"Before [stable](https://stable.example/).\n",
                Duration::ZERO
            )
            .expect("first ingest"),
        IngestResult::FirstFrame
    );
    commit_due(&mut engine, &mut writer, Duration::ZERO);
    let stable = engine
        .runtime()
        .link_table()
        .expect("table")
        .first()
        .unwrap();
    let mut app = AppState::default();
    app.handle_key(&mut engine, KeyCode::Tab, KeyModifiers::NONE)
        .expect("focus stable link");
    settle(&mut engine, &mut writer, Duration::ZERO);
    let at = Duration::from_millis(100);
    assert_eq!(
        engine
            .ingest_data(b"\nappended text\n", at)
            .expect("append"),
        IngestResult::Applied
    );
    assert!(engine.link_table().is_none());
    settle(&mut engine, &mut writer, at + MAX_BATCH_DELAY);
    assert!(engine.link_table().expect("append table").contains(stable));
    assert_eq!(engine.link_focus(), Some(stable));
}
