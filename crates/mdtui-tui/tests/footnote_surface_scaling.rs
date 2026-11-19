use mdtui_tui::{
    cli::LinkMode,
    render::frame::{FrameGeometry, FrameRuntime},
};
use mdtui3::{Budget, WorkLimit};
use std::time::Instant;

#[test]
fn twenty_thousand_hidden_bodies_do_not_get_a_renderer_or_input_walk_for_a_late_definition() {
    let mut source = String::from("# root\n\n");
    for i in 0..20_000 {
        source.push_str(&format!("[^n{i}]: [L][r]\n"));
    }
    source.push_str("# stop\n\n");
    let started = Instant::now();
    let mut r = FrameRuntime::new(
        FrameGeometry {
            rows: 8,
            cols: 40,
            content_width: 40,
        },
        LinkMode::NoOsc8,
    )
    .unwrap();
    let update = r.append_bytes(source.as_bytes()).unwrap().unwrap();
    r.apply_update(&update).unwrap();
    let input_time = started.elapsed();
    let before = r.footnote_stats().unwrap();
    let core = r.document().footnote_body_work();
    assert_eq!(before.note_creations, 0);
    assert!(before.active_owner.is_none());
    let started = Instant::now();
    let update = r
        .append_bytes(b"[r]: https://late.test/\n")
        .unwrap()
        .unwrap();
    assert!(update.body_updates.is_empty());
    r.apply_update(&update).unwrap();
    let late_time = started.elapsed();
    let after = r.footnote_stats().unwrap();
    assert_eq!(core, r.document().footnote_body_work());
    assert!(after.input_events - before.input_events < 32);
    assert_eq!(after.input_mapping, before.input_mapping);
    assert_eq!(after.note_creations, 0);
    let mut wire = vec![];
    let started = Instant::now();
    for _ in 0..10000 {
        if r.frame_to(&mut wire, &mut Budget::new(WorkLimit::unlimited()))
            .unwrap()
            .committed
        {
            break;
        }
    }
    let draw_time = started.elapsed();
    assert!(r.footnote_stats().unwrap().active_owner.is_none());
    println!(
        "ui20k input={input_time:?} late={late_time:?} root_draw={draw_time:?} before={before:?} after={after:?} wire={} root_shape={:?}",
        wire.len(),
        r.viewport().shared_links().shape().unwrap().work()
    );
}

#[test]
fn a_million_byte_cold_body_append_keeps_the_native_source_index_and_active_owner() {
    let mut parsed = mdtui3::Utf8Document::new();
    parsed.enable_footnote_continuation().unwrap();
    parsed
        .append(format!("[^n]: {}", "x".repeat(1_000_000)).as_bytes())
        .unwrap();
    let mut r = FrameRuntime::from_utf8_document(
        parsed,
        FrameGeometry {
            rows: 6,
            cols: 40,
            content_width: 40,
        },
        LinkMode::NoOsc8,
    )
    .unwrap();
    while !r
        .initialize(&mut Budget::new(WorkLimit::unlimited()))
        .unwrap()
    {}
    let key = |r: &mut FrameRuntime, k| {
        assert!(
            r.footnote_key(k, crossterm::event::KeyModifiers::NONE, true)
                .unwrap()
        );
    };
    let ready = |r: &mut FrameRuntime| {
        for _ in 0..10000 {
            if r.frame_to(&mut vec![], &mut Budget::new(WorkLimit::unlimited()))
                .unwrap()
                .committed
            {
                return;
            }
        }
        panic!("body frame pending");
    };
    ready(&mut r);
    key(&mut r, crossterm::event::KeyCode::Char('f'));
    ready(&mut r);
    key(&mut r, crossterm::event::KeyCode::Enter);
    ready(&mut r);
    let before = r.footnote_stats().unwrap();
    let work = r.document().footnote_body_work();
    let started = Instant::now();
    let update = r.append_bytes(b"x").unwrap().unwrap();
    r.apply_update(&update).unwrap();
    let input_time = started.elapsed();
    ready(&mut r);
    let after = r.footnote_stats().unwrap();
    assert_eq!(before.active_owner, after.active_owner);
    assert_eq!(after.note_creations, 1);
    assert_eq!(after.shape.source_decode_bytes, 0);
    assert_eq!(after.bootstrap, before.bootstrap);
    assert!(after.shape.input_node_visits - before.shape.input_node_visits < 1024);
    assert_eq!(
        r.document().footnote_body_work().projected_bytes - work.projected_bytes,
        1
    );
    assert_eq!(
        r.screen().front_cell(0, 0).unwrap().text().collect_text(),
        "x"
    );
    println!("million cold active append input={input_time:?} before={before:?} after={after:?}");
}
