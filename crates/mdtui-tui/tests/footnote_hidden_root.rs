use mdtui_tui::{
    cli::LinkMode,
    render::frame::{FrameGeometry, FrameRuntime},
};
use mdtui3::{Budget, WorkLimit};

#[test]
fn a_hidden_window_advances_to_a_visible_root_without_committing_a_blank_frame() {
    use mdtui_tui::render::viewport::{ViewportConfig, ViewportFrameStatus, ViewportState};
    let mut document = mdtui3::Document::new();
    document
        .append(&format!("[^n]: {}\n# visible root\n", "x".repeat(2048)))
        .unwrap();
    let mut viewport = ViewportState::new(
        &document,
        ViewportConfig {
            width: 40,
            rows: 4,
            max_tiles: 1,
            ..Default::default()
        },
    )
    .unwrap();
    let mut zero = Budget::new(WorkLimit::default());
    assert!(matches!(
        viewport.frame(&document, &mut zero).unwrap().status,
        ViewportFrameStatus::BudgetExhausted | ViewportFrameStatus::PendingScan
    ));
    assert_eq!(zero.used(), WorkLimit::default());
    let mut completed = None;
    for _ in 0..10_000 {
        let frame = viewport
            .frame(
                &document,
                &mut Budget::new(WorkLimit {
                    queries: 1,
                    objects: 1,
                    bytes: 6,
                    tiles: 1,
                    recovery: 1,
                    dependencies: 1,
                }),
            )
            .expect("bounded hidden window");
        if matches!(
            frame.status,
            ViewportFrameStatus::Ready | ViewportFrameStatus::EndOfDocument
        ) {
            completed = Some(frame);
            break;
        }
    }
    let frame = completed.expect("micro-budget hidden range must make progress");
    assert_eq!(
        frame.lines[0]
            .cells
            .iter()
            .map(|c| c.text.collect_text())
            .collect::<String>()
            .trim_end(),
        "visible root"
    );
}

fn ready(runtime: &mut FrameRuntime) {
    for _ in 0..20_000 {
        if runtime
            .frame_to(
                &mut vec![],
                &mut Budget::new(WorkLimit {
                    queries: 1,
                    objects: 1,
                    bytes: 6,
                    tiles: 1,
                    recovery: 1,
                    dependencies: 1,
                }),
            )
            .expect("live root frame")
            .committed
        {
            return;
        }
    }
    panic!("root frame pending");
}

#[test]
fn a_live_definition_only_root_can_commit_empty_then_show_a_following_root_block() {
    let mut parsed = mdtui3::Utf8Document::new();
    parsed.enable_footnote_continuation().unwrap();
    parsed
        .append(format!("[^n]: {}", "x".repeat(20_000)).as_bytes())
        .unwrap();
    let mut runtime = FrameRuntime::from_utf8_document(
        parsed,
        FrameGeometry {
            rows: 6,
            cols: 40,
            content_width: 40,
        },
        LinkMode::NoOsc8,
    )
    .unwrap();
    let mut wire = vec![];
    assert!(
        !runtime
            .frame_to(&mut wire, &mut Budget::new(WorkLimit::default()))
            .unwrap()
            .committed
    );
    assert!(wire.is_empty());
    while !runtime
        .initialize(&mut Budget::new(WorkLimit::unlimited()))
        .unwrap()
    {}
    ready(&mut runtime);
    assert_eq!(
        runtime
            .screen()
            .front_cell(0, 0)
            .unwrap()
            .text()
            .collect_text(),
        " "
    );
    let update = runtime
        .append_bytes(b"\n# visible root\n")
        .unwrap()
        .unwrap();
    runtime.apply_update(&update).unwrap();
    ready(&mut runtime);
    assert_eq!(
        runtime
            .screen()
            .front_cell(0, 0)
            .unwrap()
            .text()
            .collect_text(),
        "v"
    );
}
