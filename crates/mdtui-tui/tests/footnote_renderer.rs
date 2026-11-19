use mdtui_tui::render::first_frame::{FirstFrame, FirstFrameConfig, FirstFrameStatus};
use mdtui3::*;

#[test]
fn runtime_number_is_a_complete_mapped_display_object_before_and_after_definition() {
    use mdtui_tui::{
        cli::LinkMode,
        render::frame::{FrameGeometry, FrameRuntime},
    };
    for source in ["a[^n]z\n", "a[^n]z\n\n[^n]: note\n"] {
        let mut runtime = FrameRuntime::from_finished_source(
            source,
            FrameGeometry {
                rows: 6,
                cols: 24,
                content_width: 24,
            },
            LinkMode::NoOsc8,
        )
        .unwrap();
        let mut wire = vec![];
        for i in 0..10000 {
            if runtime
                .frame_to(&mut wire, &mut Budget::new(WorkLimit::unlimited()))
                .unwrap()
                .committed
            {
                break;
            }
            assert!(i < 9999);
        }
        let row: String = runtime
            .screen()
            .front_row(0)
            .unwrap()
            .iter()
            .map(|c| c.text().collect_text())
            .collect();
        assert_eq!(row.trim_end(), "a¹z");
        assert_eq!(
            runtime.screen().front_cell(0, 1).unwrap().source(),
            Some(Span::new(1, 5))
        );
        assert!(wire.windows("a¹z".len()).any(|w| w == "a¹z".as_bytes()));
    }
}

#[test]
fn real_body_native_renderer_preserves_markdown_math_and_full_cluster() {
    let cluster = format!("👩{}\u{200d}💻", "\u{301}".repeat(5000));
    let mut d = Document::new();
    d.enable_display_shape_effects().unwrap();
    d.append(&format!("[^n]: # H\n\n    - **one**\n    - two\n\n    A \\(\\frac{{a}}{{b}}\\) Z\n\n    {cluster}\n# outside\n")).unwrap();
    d.finish();
    let body = d.footnote_body(FootnoteId(0)).unwrap();
    let owner = body.key();
    let revision = d.revision();
    let mut first = body
        .with_renderer(owner, revision, |native| {
            FirstFrame::new(
                native,
                FirstFrameConfig {
                    top_tile: TileId(0),
                    tile_limit: native.tile_count(),
                    width: 40,
                    row_limit: 100,
                },
            )
            .unwrap()
        })
        .unwrap();
    for frame in 0..1000 {
        let result = body
            .with_renderer(owner, revision, |native| {
                first
                    .advance(native, &mut Budget::new(WorkLimit::unlimited()))
                    .unwrap()
            })
            .unwrap();
        if matches!(
            result.status,
            FirstFrameStatus::Covered | FirstFrameStatus::EndOfDocument
        ) {
            break;
        }
        assert!(frame < 999);
    }
    let rows: Vec<String> = first
        .lines()
        .iter()
        .map(|line| {
            line.cells
                .iter()
                .map(|cell| cell.text.collect_text())
                .collect::<String>()
                .trim_end()
                .to_owned()
        })
        .collect();
    assert_eq!(&rows[..4], &["H", "", "• one", "• two"]);
    assert!(rows.iter().any(|row| row.contains('─')));
    assert!(first.sink_stats().unwrap().math_object_callbacks > 0);
    let glyph = first
        .lines()
        .iter()
        .flat_map(|line| &line.cells)
        .find(|cell| cell.text.len() == cluster.len())
        .unwrap();
    assert!(glyph.text.is_indexed());
    assert_eq!(glyph.text.collect_text(), cluster);
    assert_eq!(glyph.width, 2);
}
