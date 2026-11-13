use mdtui_tui::{
    cli::LinkMode,
    render::frame::{FrameGeometry, FrameRuntime},
};
use mdtui3::{Budget, WorkLimit};

fn unlimited() -> Budget {
    Budget::new(WorkLimit::unlimited())
}

#[test]
fn actual_frame_publishes_and_writes_complete_giant_cluster() {
    let cluster = format!("👩{}\u{200d}💻", "\u{301}".repeat(10_000));
    let mut runtime = FrameRuntime::from_finished_source(
        &format!("{cluster}\n\nnext\n"),
        FrameGeometry {
            rows: 5,
            cols: 24,
            content_width: 24,
        },
        LinkMode::NoOsc8,
    )
    .unwrap();
    let mut wire = Vec::new();
    for _ in 0..1000 {
        if runtime
            .frame_to(&mut wire, &mut unlimited())
            .unwrap()
            .committed
        {
            break;
        }
    }
    let cell = runtime.screen().front_cell(0, 0).unwrap();
    assert_eq!(cell.width(), 2);
    assert!(cell.text().is_indexed());
    assert_eq!(cell.text().len(), cluster.len());
    assert_eq!(cell.text().collect_text(), cluster);
    assert!(wire.windows(cluster.len()).any(|w| w == cluster.as_bytes()));
}

#[test]
fn invalidated_native_fence_checkpoint_restores_the_same_blank_and_text_rows() {
    use mdtui_tui::render::{
        first_frame::{FirstFrame, FirstFrameConfig, FirstFrameStatus},
        viewport::{ViewportConfig, ViewportFrameStatus, ViewportState},
    };
    use mdtui3::{Document, TileId};
    fn ready(v: &mut ViewportState, d: &Document) -> Vec<String> {
        for _ in 0..10000 {
            let f = v.frame(d, &mut unlimited()).unwrap();
            if matches!(
                f.status,
                ViewportFrameStatus::Ready | ViewportFrameStatus::EndOfDocument
            ) {
                return f
                    .lines
                    .iter()
                    .map(|l| l.cells.iter().map(|c| c.text.collect_text()).collect())
                    .collect();
            }
        }
        panic!("no frame")
    }
    let s = format!(
        "```\n{}last\n```\n",
        (0..900)
            .map(|i| format!("row{i:04}\n\n"))
            .collect::<String>()
    );
    let mut d = Document::new();
    d.append(&s).unwrap();
    d.finish();
    assert_eq!(d.blocks().len(), 1);
    let mut first = FirstFrame::new(
        &d,
        FirstFrameConfig {
            top_tile: TileId(0),
            tile_limit: d.tile_count(),
            width: 40,
            row_limit: 100000,
        },
    )
    .unwrap();
    for _ in 0..10000 {
        if matches!(
            first.advance(&d, &mut unlimited()).unwrap().status,
            FirstFrameStatus::Covered | FirstFrameStatus::EndOfDocument
        ) {
            break;
        }
    }
    let whole = first
        .lines()
        .iter()
        .map(|l| {
            l.cells
                .iter()
                .map(|c| c.text.collect_text())
                .collect::<String>()
        })
        .collect::<Vec<_>>();
    let mut v = ViewportState::from_first_frame(
        &d,
        first,
        ViewportConfig {
            width: 40,
            rows: 5,
            overscan_rows: 2,
            max_tiles: 64,
            max_replay_bytes: 256 * 1024,
            row_slack: 32,
        },
    )
    .unwrap();
    v.scroll_down(&d, 700).unwrap();
    let before = ready(&mut v, &d);
    assert_eq!(before, whole[700..705]);
    let a = v.top_anchor().unwrap();
    v.checkpoints_mut()
        .invalidate(a.tile.0..a.tile.0 + 1)
        .unwrap();
    assert_eq!(ready(&mut v, &d), before);
}

#[derive(Default)]
struct MicroWriter {
    wire: Vec<u8>,
    calls: usize,
    max_write: usize,
    block_next: bool,
}
impl std::io::Write for MicroWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        assert!(
            bytes.len() <= 6,
            "normal write exceeded actual byte credit: {}",
            bytes.len()
        );
        self.calls += 1;
        self.max_write = self.max_write.max(bytes.len());
        if std::mem::take(&mut self.block_next) {
            return Err(std::io::Error::from(std::io::ErrorKind::WouldBlock));
        }
        let n = bytes.len().min(2);
        self.wire.extend_from_slice(&bytes[..n]);
        Ok(n)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
fn micro() -> Budget {
    Budget::new(WorkLimit {
        queries: 1,
        objects: 1,
        bytes: 6,
        tiles: 1,
        ..Default::default()
    })
}
#[test]
fn real_frame_stream_append_is_one_egc_transaction_with_micro_writes_and_eagain() {
    for batches in [8, 64, 256] {
        let mut runtime = FrameRuntime::new(
            FrameGeometry {
                rows: 4,
                cols: 24,
                content_width: 24,
            },
            LinkMode::NoOsc8,
        )
        .unwrap();
        let mut writer = MicroWriter::default();
        let mut expected = String::new();
        for batch in 0..=batches {
            let chunk = if batch == 0 {
                "a".to_owned()
            } else {
                "\u{301}".repeat(1024)
            };
            expected.push_str(&chunk);
            let update = runtime.append_bytes(chunk.as_bytes()).unwrap().unwrap();
            runtime.apply_update(&update).unwrap();
            let before_marks = writer.wire.iter().filter(|&&b| b == 0xcc).count();
            writer.block_next = true;
            let mut acknowledged = false;
            let mut last = None;
            for _ in 0..100_000 {
                let mut budget = micro();
                let commit = runtime.frame_to(&mut writer, &mut budget).unwrap();
                assert!(budget.used().bytes <= 6);
                last = Some((commit.status, budget.used()));
                if commit.committed {
                    acknowledged = true;
                    break;
                }
            }
            assert!(
                acknowledged,
                "batch {batch} failed to advance: {last:?}, writes {}, bytes {}",
                writer.calls,
                writer.wire.len()
            );
            let added_marks = writer.wire.iter().filter(|&&b| b == 0xcc).count() - before_marks;
            assert_eq!(
                added_marks,
                if batch == 0 { 0 } else { 1024 },
                "batch {batch}, effects {:?}",
                update.display_shape.events()
            );
        }
        assert_eq!(
            runtime.screen().front_cell(0, 0).unwrap().text().len(),
            expected.len()
        );
        assert!(
            writer
                .wire
                .windows(expected.len())
                .any(|w| w == expected.as_bytes())
        );
        assert_eq!(
            writer
                .wire
                .windows(expected.len())
                .filter(|w| *w == expected.as_bytes())
                .count(),
            1
        );
        assert_eq!(
            writer.wire.iter().filter(|&&b| b == 0xcc).count(),
            batches * 1024
        );
        assert!(writer.max_write <= 6);
        use crossterm::event::{KeyCode, KeyModifiers};
        runtime
            .reading_key(KeyCode::Char('/'), KeyModifiers::NONE, true)
            .unwrap();
        let before_status = writer.wire.len();
        for _ in 0..32 {
            runtime
                .reading_key(KeyCode::Char('z'), KeyModifiers::NONE, true)
                .unwrap();
            let mut done = false;
            for _ in 0..10_000 {
                if runtime
                    .frame_to(&mut writer, &mut micro())
                    .unwrap()
                    .committed
                {
                    done = true;
                    break;
                }
            }
            assert!(done);
        }
        assert_eq!(
            writer.wire.len(),
            before_status,
            "status must wait outside the same open EGC"
        );
        let shape_work = runtime.viewport().shared_links().shape().unwrap().work();
        assert_eq!(shape_work.source_decode_bytes, expected.len());
        // One read per emitted scalar plus the fixed first-scalar publication
        // frontier per batch. A suffix reader must not revisit the old EGC.
        assert!(shape_work.reader_atom_reads <= batches * 1024 + 1 + 4 * batches + 64);
        eprintln!(
            "production batches={batches} payload_bytes={} normal_write_max={} normal_write_calls={} status_updates_deferred=32 shape={:?} retained_capacity={}",
            expected.len(),
            writer.max_write,
            writer.calls,
            runtime.viewport().shared_links().shape().unwrap().work(),
            runtime
                .viewport()
                .shared_links()
                .shape()
                .unwrap()
                .capacity_bytes()
        );
        let update = runtime.finish_input().unwrap().unwrap();
        runtime.apply_update(&update).unwrap();
        let mut finished = false;
        for _ in 0..100_000 {
            if runtime
                .frame_to(&mut writer, &mut micro())
                .unwrap()
                .committed
            {
                finished = true;
                break;
            }
        }
        assert!(finished);
        assert_eq!(
            writer.wire.iter().filter(|&&b| b == 0xcc).count(),
            batches * 1024,
            "EOF must close the same EGC without resending its prefix; effects {:?}",
            update.display_shape.events()
        );
    }
}

#[test]
fn already_parsed_input_initializes_with_budget_then_continues_the_real_updates() {
    let cluster = format!("a{}", "\u{301}".repeat(10_000));
    let mut input = mdtui3::Utf8Document::new();
    input.append(cluster.as_bytes()).unwrap();
    let mut runtime = FrameRuntime::from_utf8_document(
        input,
        FrameGeometry {
            rows: 4,
            cols: 24,
            content_width: 24,
        },
        LinkMode::NoOsc8,
    )
    .unwrap();
    assert!(
        !runtime
            .initialize(&mut Budget::new(WorkLimit::default()))
            .unwrap()
    );
    assert!(matches!(
        runtime.append_bytes(b"x"),
        Err(mdtui_tui::render::frame::FrameError::InitializationPending)
    ));
    let mut ready = false;
    for _ in 0..100_000 {
        if runtime.initialize(&mut micro()).unwrap() {
            ready = true;
            break;
        }
    }
    assert!(ready);
    let update = runtime.append_bytes("\u{301}".as_bytes()).unwrap().unwrap();
    runtime.apply_update(&update).unwrap();
    let mut wire = Vec::new();
    for _ in 0..1000 {
        if runtime
            .frame_to(&mut wire, &mut unlimited())
            .unwrap()
            .committed
        {
            break;
        }
    }
    assert_eq!(
        runtime
            .screen()
            .front_cell(0, 0)
            .unwrap()
            .text()
            .collect_text(),
        format!("{cluster}\u{301}")
    );
}

#[test]
fn actual_middle_source_windows_publish_the_same_complete_cluster_without_prefix_decoding() {
    use mdtui_tui::render::{
        display::{DisplayScanner, ScanStatus},
        refine::{RefineConfig, RefineProgress, SourceRefiner},
    };
    use mdtui3::Span;
    let prefix = "x".repeat(2 * 1024 * 1024);
    let cluster = format!("a{}", "\u{301}".repeat(100_000));
    let source = format!("{prefix}{cluster} tail\n\n");
    let mut runtime = FrameRuntime::from_finished_source(
        &source,
        FrameGeometry {
            rows: 4,
            cols: 80,
            content_width: 80,
        },
        LinkMode::NoOsc8,
    )
    .unwrap();
    let (document, viewport) = runtime.document_and_viewport_mut();
    let links = viewport.shared_links();
    let shape = links.shape().unwrap();
    let old = shape.work();
    let at = prefix.len() + 1 + 2 * 50_000;
    let mut maximum = 0;
    let mut cold = 0;
    let mut cold_refs = 0;
    let mut max_refs = 0;
    for i in 0..1000 {
        let query_before = shape.work();
        let start = at + (i % 100) * 2;
        let mut scanner = DisplayScanner::new(document, Span::new(start, start + 2)).unwrap();
        scanner.set_links(links.clone());
        assert_eq!(
            scanner.advance(document, &mut unlimited()).unwrap(),
            ScanStatus::Complete
        );
        assert_eq!(scanner.segments().len(), 1);
        let segment = &scanner.segments()[0];
        assert_eq!(
            segment.source,
            Span::new(prefix.len(), prefix.len() + cluster.len())
        );
        assert_eq!(segment.text.len(), cluster.len());
        assert_eq!(scanner.stats().omitted_edge_clusters, 0);
        let properties = shape.work().query_properties - query_before.query_properties;
        if i == 0 {
            cold = properties;
        }
        maximum = maximum.max(properties);
        let references =
            properties + shape.work().reader_atom_reads - query_before.reader_atom_reads;
        if i == 0 {
            cold_refs = references;
        }
        max_refs = max_refs.max(references);
        assert!(references <= 64 * (8 * (source.len().ilog2() as usize + 1) + 24));
        assert!(
            properties < 2048,
            "query {i} read {properties} old properties; cold={cold} max={maximum}"
        );
    }
    let after = shape.work();
    assert_eq!(after.source_decode_bytes, old.source_decode_bytes);
    assert!(
        after.query_properties - old.query_properties < 2_048_000,
        "actual fixed-leaf property reads {}",
        after.query_properties - old.query_properties
    );
    let mut refiner = SourceRefiner::new(
        document,
        viewport.heights(),
        viewport.checkpoints(),
        at,
        RefineConfig {
            width: 80,
            max_tiles: 8,
            max_replay_bytes: 2048,
            row_slack: 8,
        },
    )
    .unwrap();
    refiner.set_links(links);
    let mut refined = None;
    for _ in 0..100_000 {
        let (heights, checkpoints) = viewport.layout_parts_mut();
        if let RefineProgress::Ready(target) = refiner
            .advance(document, heights, checkpoints, &mut micro())
            .unwrap()
        {
            refined = Some(target);
            break;
        }
    }
    let target = refined.expect("micro-budget source refinement must finish");
    assert_eq!(target.source, prefix.len());
    assert_eq!(target.line_start, prefix.len());
    assert_eq!(shape.work().source_decode_bytes, old.source_decode_bytes);
    println!(
        "actual 2MiB-prefix middle queries=1000 property_reads={} decoded_new_bytes={} query_nodes={} capacity={} cold_properties={cold} max_properties={maximum} cold_refs={cold_refs} max_refs={max_refs}",
        after.query_properties - old.query_properties,
        after.source_decode_bytes - old.source_decode_bytes,
        after.query_nodes - old.query_nodes,
        shape.capacity_bytes()
    );
}

#[test]
fn logical_page_down_during_first_wire_frame_keeps_the_new_view() {
    use crossterm::event::{KeyCode, KeyModifiers};
    use mdtui_tui::{
        app::AppState,
        stream::{FRAME_WORK, StreamEngine},
    };
    use std::time::Duration;
    let source = format!(
        "# TOP_MARK\n\nWRAP_MARK {}\n\n{}",
        "word ".repeat(60),
        "filler paragraph abcdefghijklmnopqrstuvwxyz\n\n".repeat(100)
    );
    let runtime = FrameRuntime::new(
        FrameGeometry {
            rows: 24,
            cols: 80,
            content_width: 80,
        },
        LinkMode::NoOsc8,
    )
    .unwrap();
    let mut engine = StreamEngine::new(runtime);
    let mut wire = Vec::new();
    let mut now = Duration::ZERO;
    engine.ingest_data(source.as_bytes(), now).unwrap();
    for _ in 0..10_000 {
        engine
            .render_due(&mut wire, &mut Budget::new(FRAME_WORK), now)
            .unwrap();
        if engine.first_frame_done() {
            break;
        }
        now += Duration::from_millis(1);
    }
    assert!(engine.first_frame_done());
    engine.finish_input(now).unwrap();
    let mut app = AppState::default();
    app.handle_key(&mut engine, KeyCode::PageDown, KeyModifiers::NONE)
        .unwrap();
    let expected = engine.runtime().top_source();
    let expected_row = engine.runtime().viewport().top_row();
    for _ in 0..10_000 {
        if engine.calibration_pending() && !engine.due(now) {
            engine
                .advance_calibration(&mut Budget::new(FRAME_WORK))
                .unwrap();
        }
        if engine.due(now) {
            engine
                .render_due(&mut wire, &mut Budget::new(FRAME_WORK), now)
                .unwrap();
        }
        now += Duration::from_millis(1);
        if !engine.calibration_pending() && !engine.due(now) {
            break;
        }
    }
    let actual = engine.runtime().top_source();
    let actual_row = engine.runtime().viewport().top_row();
    let text = (0..24)
        .map(|row| {
            (0..80)
                .map(|column| {
                    engine
                        .runtime()
                        .screen()
                        .front_cell(row, column)
                        .unwrap()
                        .text()
                        .collect_text()
                })
                .collect::<String>()
        })
        .collect::<Vec<_>>();
    assert!(
        !text.iter().any(|row| row.contains("TOP_MARK")),
        "source expected {expected:?} row {expected_row:?} actual {actual:?} row {actual_row:?}: {text:?}"
    );
}

#[test]
fn unrelated_definitions_and_offscreen_links_never_resend_an_acked_literal_cluster() {
    let cluster = format!("a{}", "\u{301}".repeat(10_000));
    let mut runtime = FrameRuntime::new(
        FrameGeometry {
            rows: 4,
            cols: 24,
            content_width: 24,
        },
        LinkMode::NoOsc8,
    )
    .unwrap();
    let update = runtime
        .append_bytes(format!("{cluster}\n\n{}", "offscreen\n\n".repeat(30)).as_bytes())
        .unwrap()
        .unwrap();
    runtime.apply_update(&update).unwrap();
    let mut wire = Vec::new();
    for _ in 0..1000 {
        if runtime
            .frame_to(&mut wire, &mut unlimited())
            .unwrap()
            .committed
        {
            break;
        }
    }
    assert_eq!(wire.iter().filter(|&&b| b == 0xcc).count(), 10_000);
    let old = runtime.screen().front_cell(0, 0).unwrap().text().clone();
    for i in 0..20 {
        let update = runtime
            .append_bytes(format!("[unused{i}]: /u\n\n[new{i}](/v)\n\n").as_bytes())
            .unwrap()
            .unwrap();
        runtime.apply_update(&update).unwrap();
        let before = wire.len();
        let mut ready = false;
        for _ in 0..1000 {
            if runtime
                .frame_to(&mut wire, &mut unlimited())
                .unwrap()
                .committed
            {
                ready = true;
                break;
            }
        }
        assert!(ready);
        assert_eq!(
            wire[before..].iter().filter(|&&b| b == 0xcc).count(),
            0,
            "unused definition {i} resent the giant prefix"
        );
        assert_eq!(old.collect_text(), cluster);
    }
}

#[test]
fn leading_zero_width_cluster_is_published_written_and_selected_without_a_base() {
    use crossterm::event::{KeyCode, KeyModifiers};
    let cluster = "\u{301}".repeat(10_000);
    let mut runtime = FrameRuntime::from_finished_source(
        &format!("{cluster}x\n\n"),
        FrameGeometry {
            rows: 4,
            cols: 24,
            content_width: 24,
        },
        LinkMode::NoOsc8,
    )
    .unwrap();
    let mut wire = Vec::new();
    let mut done = false;
    for _ in 0..1000 {
        if runtime
            .frame_to(&mut wire, &mut unlimited())
            .unwrap()
            .committed
        {
            done = true;
            break;
        }
    }
    assert!(done);
    let zero = runtime
        .screen()
        .front_zero_cell(0, 0)
        .expect("complete orphan EGC has its own screen record");
    assert_eq!(zero.width(), 0);
    assert_eq!(zero.source(), Some(mdtui3::Span::new(0, cluster.len())));
    assert_eq!(zero.text().collect_text(), cluster);
    assert_eq!(runtime.screen().front_cell(0, 0).unwrap().text(), "x");
    assert!(wire.windows(cluster.len()).any(|w| w == cluster.as_bytes()));
    runtime
        .reading_key(KeyCode::Char('v'), KeyModifiers::NONE, true)
        .unwrap();
    assert_eq!(
        runtime.reading().selection.cursor(),
        Some(mdtui3::Span::new(0, cluster.len()))
    );
}

#[test]
fn display_normalizes_tab_and_skipped_control_before_unicode_segmentation() {
    let marks = "\u{301}".repeat(1000);
    for (source, expected) in [
        (format!("a\t{marks}x\n\n"), format!(" {marks}")),
        (format!("a\u{1}{marks}x\n\n"), format!("a{marks}")),
    ] {
        let mut runtime = FrameRuntime::from_finished_source(
            &source,
            FrameGeometry {
                rows: 4,
                cols: 24,
                content_width: 24,
            },
            LinkMode::NoOsc8,
        )
        .unwrap();
        let mut wire = Vec::new();
        let mut done = false;
        for _ in 0..1000 {
            if runtime
                .frame_to(&mut wire, &mut unlimited())
                .unwrap()
                .committed
            {
                done = true;
                break;
            }
        }
        assert!(done);
        assert!((0..24).any(|col|runtime.screen().front_cell(0,col).unwrap().text()==expected.as_str()),"expected full normalized EGC, source bytes {}",source.len());
        assert!(
            wire.windows(expected.len())
                .any(|w| w == expected.as_bytes())
        );
    }
}

#[test]
fn inline_html_is_removed_before_gcb_but_code_and_autolinks_keep_their_content() {
    let marks = "\u{301}".repeat(1000);
    for (source, expected) in [
        (format!("a<b>{marks}x\n\n"), format!("a{marks}")),
        (format!("a<!-- c -->{marks}x\n\n"), format!("a{marks}")),
        (format!("`a<b>{marks}`x\n\n"), format!(">{marks}")),
        (
            format!("<https://example.test>{marks}x\n\n"),
            format!("t{marks}"),
        ),
    ] {
        let mut runtime = FrameRuntime::from_finished_source(
            &source,
            FrameGeometry {
                rows: 4,
                cols: 80,
                content_width: 80,
            },
            LinkMode::NoOsc8,
        )
        .unwrap();
        let mut wire = Vec::new();
        let mut done = false;
        for _ in 0..1000 {
            if runtime
                .frame_to(&mut wire, &mut unlimited())
                .unwrap()
                .committed
            {
                done = true;
                break;
            }
        }
        assert!(done);
        assert!((0..80).any(|col|runtime.screen().front_cell(0,col).unwrap().text()==expected.as_str()),"missing expected source-mapped cluster, source bytes {}",source.len());
        assert!(
            wire.windows(expected.len())
                .any(|w| w == expected.as_bytes())
        );
    }
}

#[test]
fn frozen_end_does_not_mix_future_combining_append_into_the_saved_source_cut() {
    let initial = format!("a{}", "\u{301}".repeat(5000));
    let mut runtime = FrameRuntime::new(
        FrameGeometry {
            rows: 4,
            cols: 24,
            content_width: 24,
        },
        LinkMode::NoOsc8,
    )
    .unwrap();
    let update = runtime.append_bytes(initial.as_bytes()).unwrap().unwrap();
    runtime.apply_update(&update).unwrap();
    runtime.seek_end(false).unwrap();
    let mut wire = Vec::new();
    for _ in 0..1000 {
        runtime.frame_to(&mut wire, &mut unlimited()).unwrap();
        if runtime.screen().front_cell(0, 0).unwrap().text().len() == initial.len() {
            break;
        }
    }
    assert_eq!(
        runtime
            .screen()
            .front_cell(0, 0)
            .unwrap()
            .text()
            .collect_text(),
        initial
    );
    let update = runtime
        .append_bytes("\u{301}".repeat(5000).as_bytes())
        .unwrap()
        .unwrap();
    runtime.apply_update(&update).unwrap();
    let mut ready = false;
    for _ in 0..1000 {
        if runtime
            .frame_to(&mut wire, &mut unlimited())
            .unwrap()
            .committed
        {
            ready = true;
            break;
        }
    }
    assert!(ready);
    assert_eq!(
        runtime.screen().front_cell(0, 0).unwrap().text().len(),
        initial.len(),
        "frozen source view must retain its own EGC boundary"
    );
    assert!(runtime.screen().front_cell(0, 0).unwrap().text() == initial.as_str());
    runtime.seek_end(true).unwrap();
    for _ in 0..1000 {
        runtime.frame_to(&mut wire, &mut unlimited()).unwrap();
        if runtime.screen().front_cell(0, 0).unwrap().text().len() == initial.len() + 10_000 {
            break;
        }
    }
    assert_eq!(
        runtime.screen().front_cell(0, 0).unwrap().text().len(),
        initial.len() + 10_000
    );
}

#[test]
fn million_scalar_combining_zwj_and_ri_middle_queries_use_classified_frontiers() {
    use mdtui_tui::render::display::{DisplayScanner, ScanStatus};
    use mdtui3::Span;
    use unicode_segmentation::UnicodeSegmentation;
    use unicode_width::UnicodeWidthStr;
    let fixtures = [
        ("combining", format!("a{}", "\u{301}".repeat(1_000_000))),
        ("zwj", format!("{}💻", "👩\u{301}\u{200d}".repeat(333_333))),
        ("ri", "🇨🇳".repeat(500_000)),
    ];
    for (name, text) in fixtures {
        let expected: Vec<_> = text
            .grapheme_indices(true)
            .map(|(at, g)| (at, g.len(), g.width().min(2)))
            .collect();
        let prefix = "p".repeat(128 * 1024);
        let source = format!("{prefix}{text}\n\n");
        let mut runtime = FrameRuntime::from_finished_source(
            &source,
            FrameGeometry {
                rows: 4,
                cols: 80,
                content_width: 80,
            },
            LinkMode::NoOsc8,
        )
        .unwrap();
        let (document, viewport) = runtime.document_and_viewport_mut();
        let links = viewport.shared_links();
        let shape = links.shape().unwrap();
        let old = shape.work();
        let mut max_reads = 0;
        let mut cold = 0;
        for fraction in [0, 1, 2, 3, 4, 5, 6, 7, 8, 9] {
            let mut relative = text.len() * fraction / 10;
            while !text.is_char_boundary(relative) {
                relative += 1;
            }
            let at = prefix.len() + relative;
            let next = text[relative..].chars().next().unwrap().len_utf8();
            let oracle = expected[expected.partition_point(|(start, _, _)| *start <= relative) - 1];
            let before = shape.work();
            let mut scanner = DisplayScanner::new(document, Span::new(at, at + next)).unwrap();
            scanner.set_links(links.clone());
            assert!(
                !scanner
                    .initialize(document, &mut Budget::new(WorkLimit::default()))
                    .is_err()
            );
            let mut done = false;
            for _ in 0..100_000 {
                let mut b = micro();
                if scanner.advance(document, &mut b).unwrap() == ScanStatus::Complete {
                    done = true;
                    break;
                }
            }
            assert!(done, "{name} at {relative}");
            let segment = &scanner.segments()[0];
            assert_eq!(
                segment.source,
                Span::new(prefix.len() + oracle.0, prefix.len() + oracle.0 + oracle.1)
            );
            assert_eq!(segment.text.len(), oracle.1);
            assert_eq!(segment.text.width(), oracle.2);
            let after = shape.work();
            let reads = after.query_properties - before.query_properties + after.reader_atom_reads
                - before.reader_atom_reads;
            let height = (source.len() + 1).ilog2() as usize + 1;
            assert!(
                reads <= 64 * (8 * height + 24),
                "{name}: {reads} property references exceed logarithmic fixed-leaf frontier"
            );
            if fraction == 0 {
                cold = reads;
            }
            max_reads = max_reads.max(reads);
            assert_eq!(after.source_decode_bytes, old.source_decode_bytes);
        }
        let after = shape.work();
        println!(
            "actual million {name} source_bytes={} scalars={} cold_property_refs={cold} max_property_refs={max_reads} new_unicode_decode={} summary_hits={} reader_leaf_visits={} tracked_owner_capacity={} input_width_transitions={}",
            source.len(),
            text.chars().count(),
            after.source_decode_bytes - old.source_decode_bytes,
            after.summary_hits - old.summary_hits,
            after.reader_leaf_visits - old.reader_leaf_visits,
            shape.capacity_bytes(),
            old.width_state_transitions
        );
    }
}

#[test]
fn abort_after_one_utf8_byte_only_finishes_the_scalar_then_sends_fixed_cleanup() {
    struct OneByte(Vec<u8>, bool);
    impl std::io::Write for OneByte {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if std::mem::take(&mut self.1) {
                return Err(std::io::ErrorKind::WouldBlock.into());
            }
            self.0.push(bytes[0]);
            if bytes[0] == 0xf0 {
                self.1 = true;
            }
            Ok(1)
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut runtime = FrameRuntime::new(
        FrameGeometry {
            rows: 4,
            cols: 24,
            content_width: 24,
        },
        LinkMode::NoOsc8,
    )
    .unwrap();
    let update = runtime
        .append_bytes(format!("👩{}", "\u{301}".repeat(10_000)).as_bytes())
        .unwrap()
        .unwrap();
    runtime.apply_update(&update).unwrap();
    let mut writer = OneByte(Vec::new(), false);
    let mut partial = false;
    for _ in 0..100_000 {
        let mut b = Budget::new(WorkLimit {
            queries: 64,
            objects: 64,
            bytes: 6,
            tiles: 8,
            ..Default::default()
        });
        runtime.frame_to(&mut writer, &mut b).unwrap();
        if writer.0.last() == Some(&0xf0) {
            partial = true;
            break;
        }
    }
    assert!(partial);
    let before = writer.0.len();
    let mut done = false;
    for _ in 0..1000 {
        if runtime.abort_output(&mut writer, &mut micro()).unwrap() {
            done = true;
            break;
        }
    }
    assert!(done);
    assert_eq!(&writer.0[before..before + 3], &"👩".as_bytes()[1..]);
    assert_eq!(&writer.0[before + 3..], b"\x18\x1b\\\x1b[0m\x1b[?2026l");
    std::str::from_utf8(&writer.0).unwrap();
    assert!(!writer.0.contains(&0xcc));
}

#[test]
fn crlf_softbreak_and_code_normalization_preserve_the_exact_scalar_envelope() {
    use mdtui_tui::render::display::{DisplayScanner, ScanStatus};
    use mdtui3::Span;
    let marks = "\u{301}".repeat(1000);
    for (source, start) in [
        (format!("a\r\n{marks}x\n\n"), 1),
        (format!("`a\r\n{marks}`x\n\n"), 2),
    ] {
        let mut runtime = FrameRuntime::from_finished_source(
            &source,
            FrameGeometry {
                rows: 4,
                cols: 80,
                content_width: 80,
            },
            LinkMode::NoOsc8,
        )
        .unwrap();
        let (document, viewport) = runtime.document_and_viewport_mut();
        let links = viewport.shared_links();
        let mut scanner = DisplayScanner::new(document, Span::new(0, source.len())).unwrap();
        scanner.set_links(links);
        assert_eq!(
            scanner.advance(document, &mut unlimited()).unwrap(),
            ScanStatus::Complete
        );
        let expected = format!(" {marks}");
        let glyph = scanner
            .segments()
            .iter()
            .find(|s| s.text == expected)
            .expect("one normalized CRLF space joins the marks");
        assert_eq!(glyph.source, Span::new(start, start + 2 + marks.len()));
        assert_eq!(
            scanner.segments().iter().filter(|s| s.text == " ").count(),
            0,
            "CRLF normalizes to one display space"
        );
    }
}

#[test]
fn every_actual_block_renderer_preserves_an_orphan_giant_egc() {
    let cluster = "\u{301}".repeat(5000);
    for source in [
        format!("# {cluster}x\n"),
        format!("- {cluster}x\n"),
        format!("> {cluster}x\n"),
        format!("```\n{cluster}x\n```\n"),
        format!("|h|\n|---|\n|{cluster}x|\n"),
    ] {
        let mut runtime = FrameRuntime::from_finished_source(
            &source,
            FrameGeometry {
                rows: 12,
                cols: 40,
                content_width: 40,
            },
            LinkMode::NoOsc8,
        )
        .unwrap();
        let mut wire = Vec::new();
        let mut done = false;
        for _ in 0..1000 {
            if runtime
                .frame_to(&mut wire, &mut unlimited())
                .unwrap()
                .committed
            {
                done = true;
                break;
            }
        }
        assert!(done);
        let cell = (0..12)
            .flat_map(|row| (0..40).map(move |col| (row, col)))
            .find_map(|(row, col)| {
                runtime
                    .screen()
                    .front_zero_cell(row, col)
                    .filter(|c| c.text().len() == cluster.len())
            });
        assert!(
            cell.is_some(),
            "block {:?} lost the zero-width source owner",
            &source[..source.len().min(3)]
        );
        assert!(wire.windows(cluster.len()).any(|w| w == cluster.as_bytes()));
    }
}

#[test]
fn relevant_late_definition_cancels_old_wire_once_and_repaints_the_new_binding() {
    use mdtui_tui::render::payload::PayloadStep;
    let cluster = format!("a{}", "\u{301}".repeat(5000));
    let mut runtime = FrameRuntime::new(
        FrameGeometry {
            rows: 4,
            cols: 24,
            content_width: 24,
        },
        LinkMode::NoOsc8,
    )
    .unwrap();
    let update = runtime
        .append_bytes(format!("[{cluster}][late]\n\n").as_bytes())
        .unwrap()
        .unwrap();
    runtime.apply_update(&update).unwrap();
    let mut writer = MicroWriter::default();
    let mut old = None;
    for _ in 0..100_000 {
        runtime.frame_to(&mut writer, &mut micro()).unwrap();
        if writer.wire.iter().filter(|&&b| b == 0xcc).count() >= 3 {
            old = Some(runtime.screen().cell(0, 1).unwrap().text().clone());
            break;
        }
    }
    let old = old.expect("the real old payload must be in flight");
    let partial = writer.wire.iter().filter(|&&b| b == 0xcc).count();
    assert!(partial < 5000);
    let update = runtime.append_bytes(b"[late]: /u\n").unwrap().unwrap();
    runtime.apply_update(&update).unwrap();
    assert!(matches!(
        old.reader().next(&mut unlimited()),
        Err(mdtui3::LayoutError::StaleContinuation)
    ));
    let mut done = false;
    for _ in 0..200_000 {
        if runtime
            .frame_to(&mut writer, &mut micro())
            .unwrap()
            .committed
        {
            done = true;
            break;
        }
    }
    assert!(done);
    let cell = runtime.screen().front_cell(0, 0).unwrap();
    assert_eq!(cell.text().len(), cluster.len());
    assert!(cell.link_id().is_some());
    assert_eq!(
        writer.wire.iter().filter(|&&b| b == 0xcc).count(),
        partial + 5000
    );
    let cleanup = b"\x18\x1b\\\x1b[0m\x1b[?2026l";
    assert_eq!(
        writer
            .wire
            .windows(cleanup.len())
            .filter(|w| *w == cleanup)
            .count(),
        1
    );
    std::str::from_utf8(&writer.wire).unwrap();
    assert!(matches!(
        cell.text().reader().next(&mut unlimited()).unwrap(),
        PayloadStep::Bytes { .. }
    ));
}

#[test]
fn inflight_giant_resize_and_late_header_each_cancel_once_without_splitting_utf8() {
    use mdtui_tui::screen::StyleRole;
    let cluster = format!("👩{}\u{200d}💻", "\u{301}".repeat(5000));
    for header in [false, true] {
        let mut runtime = FrameRuntime::new(
            FrameGeometry {
                rows: 5,
                cols: 32,
                content_width: 32,
            },
            LinkMode::NoOsc8,
        )
        .unwrap();
        let update = runtime
            .append_bytes(format!("{cluster}\n").as_bytes())
            .unwrap()
            .unwrap();
        runtime.apply_update(&update).unwrap();
        let mut writer = MicroWriter::default();
        for _ in 0..100_000 {
            runtime.frame_to(&mut writer, &mut micro()).unwrap();
            if writer.wire.iter().filter(|&&b| b == 0xcc).count() >= 3 {
                break;
            }
        }
        let partial = writer.wire.iter().filter(|&&b| b == 0xcc).count();
        assert!((3..5000).contains(&partial));
        if header {
            let update = runtime.append_bytes(b"===\n\n").unwrap().unwrap();
            runtime.apply_update(&update).unwrap();
        } else {
            runtime.resize(6, 25).unwrap();
        }
        let mut done = false;
        for i in 0..300_000 {
            writer.block_next = i % 17 == 0;
            if runtime
                .frame_to(&mut writer, &mut micro())
                .unwrap()
                .committed
            {
                done = true;
                break;
            }
        }
        assert!(done, "header={header}");
        assert_eq!(
            writer.wire.iter().filter(|&&b| b == 0xcc).count(),
            partial + 5000
        );
        let cleanup = b"\x18\x1b\\\x1b[0m\x1b[?2026l";
        assert_eq!(
            writer
                .wire
                .windows(cleanup.len())
                .filter(|w| *w == cleanup)
                .count(),
            1
        );
        std::str::from_utf8(&writer.wire).unwrap();
        let cell = (0..runtime.screen().rows())
            .flat_map(|r| (0..runtime.screen().cols()).map(move |c| (r, c)))
            .find_map(|(r, c)| {
                runtime
                    .screen()
                    .front_cell(r, c)
                    .filter(|c| c.text().len() == cluster.len())
            })
            .unwrap();
        assert_eq!(cell.text().collect_text(), cluster);
        assert_eq!(cell.style().contains(StyleRole::HEADING), header);
    }
}

#[test]
fn a_conditional_scope_binding_updates_the_style_of_its_plain_giant_text() {
    use mdtui_tui::screen::StyleRole;
    let cluster = format!("a{}", "\u{301}".repeat(5000));
    let mut runtime = FrameRuntime::new(
        FrameGeometry {
            rows: 4,
            cols: 40,
            content_width: 40,
        },
        LinkMode::NoOsc8,
    )
    .unwrap();
    let update = runtime
        .append_bytes(format!("*{cluster} [x* z]\n\n").as_bytes())
        .unwrap()
        .unwrap();
    runtime.apply_update(&update).unwrap();
    let mut wire = Vec::new();
    for _ in 0..1000 {
        if runtime
            .frame_to(&mut wire, &mut unlimited())
            .unwrap()
            .committed
        {
            break;
        }
    }
    let first = (0..40)
        .find_map(|col| {
            runtime
                .screen()
                .front_cell(0, col)
                .filter(|c| c.text().len() == cluster.len())
        })
        .unwrap();
    assert!(first.style().contains(StyleRole::EMPHASIS));
    let old = first.text().clone();
    let update = runtime.append_bytes(b"[x* z]: /u\n").unwrap().unwrap();
    runtime.apply_update(&update).unwrap();
    for _ in 0..1000 {
        if runtime
            .frame_to(&mut wire, &mut unlimited())
            .unwrap()
            .committed
        {
            break;
        }
    }
    let current = (0..40)
        .find_map(|col| {
            runtime
                .screen()
                .front_cell(0, col)
                .filter(|c| c.text().len() == cluster.len())
        })
        .unwrap();
    assert!(
        !current.style().contains(StyleRole::EMPHASIS),
        "the giant text depends on conditional scope recovery even though its own raw scalars carry no delimiters or link ID"
    );
    assert!(matches!(
        old.reader().next(&mut unlimited()),
        Err(mdtui3::LayoutError::StaleContinuation)
    ));
}

#[test]
fn unused_definitions_preserve_a_conditional_scope_and_its_acked_giant_payload() {
    let cluster = format!("a{}", "\u{301}".repeat(5000));
    let mut runtime = FrameRuntime::new(
        FrameGeometry {
            rows: 4,
            cols: 40,
            content_width: 40,
        },
        LinkMode::NoOsc8,
    )
    .unwrap();
    let mut wire = Vec::new();
    let update = runtime
        .append_bytes(format!("*{cluster} [x* z]\n\n{}", "offscreen\n\n".repeat(30)).as_bytes())
        .unwrap()
        .unwrap();
    runtime.apply_update(&update).unwrap();
    for _ in 0..1000 {
        if runtime
            .frame_to(&mut wire, &mut unlimited())
            .unwrap()
            .committed
        {
            break;
        }
    }
    assert_eq!(wire.iter().filter(|&&b| b == 0xcc).count(), 5000);
    for i in 0..20 {
        let update = runtime
            .append_bytes(format!("[unused{i}]: /u\n\n").as_bytes())
            .unwrap()
            .unwrap();
        runtime.apply_update(&update).unwrap();
        let before = wire.len();
        let mut done = false;
        for _ in 0..1000 {
            if runtime
                .frame_to(&mut wire, &mut unlimited())
                .unwrap()
                .committed
            {
                done = true;
                break;
            }
        }
        assert!(done);
        assert_eq!(
            wire[before..].iter().filter(|&&b| b == 0xcc).count(),
            0,
            "unused symbol {i} changed no dependency of the conditional scope"
        );
    }
}

#[test]
fn an_orphan_giant_link_keeps_its_keyboard_identity_and_oversize_copy_rejects_metadata_first() {
    use crossterm::event::{KeyCode, KeyModifiers};
    let cluster = "\u{301}".repeat(40_000);
    let source = format!("[{cluster}](/target)\n\n");
    let mut runtime = FrameRuntime::from_finished_source(
        &source,
        FrameGeometry {
            rows: 4,
            cols: 24,
            content_width: 24,
        },
        LinkMode::NoOsc8,
    )
    .unwrap();
    let mut wire = Vec::new();
    for _ in 0..1000 {
        if runtime
            .frame_to(&mut wire, &mut unlimited())
            .unwrap()
            .committed
        {
            break;
        }
    }
    assert_eq!(runtime.link_table().unwrap().links(), &[mdtui3::LinkId(0)]);
    assert_eq!(runtime.enter_link_focus(true), Some(mdtui3::LinkId(0)));
    runtime.clear_link_focus();
    runtime
        .reading_key(KeyCode::Char('v'), KeyModifiers::NONE, true)
        .unwrap();
    let links = runtime.viewport().shared_links();
    let shape = links.shape().unwrap();
    let before = shape.work();
    runtime
        .reading_key(KeyCode::Char('y'), KeyModifiers::NONE, true)
        .unwrap();
    runtime.advance_reading(&mut wire, &mut micro());
    assert_eq!(
        shape.work().reader_atom_reads,
        before.reader_atom_reads,
        "the 64KiB limit must reject the payload length without flattening it"
    );
    assert!(!wire.windows(5).any(|w| w == b"\x1b]52;"));
}

#[test]
fn final_extend_and_eof_or_newline_before_ack_close_the_same_wire_egc() {
    for partial in [false, true] {
        for finish in [false, true] {
            let mut runtime = FrameRuntime::new(
                FrameGeometry {
                    rows: 5,
                    cols: 24,
                    content_width: 24,
                },
                LinkMode::NoOsc8,
            )
            .unwrap();
            let prefix = format!("👩{}", "\u{301}".repeat(1000));
            let update = runtime.append_bytes(prefix.as_bytes()).unwrap().unwrap();
            runtime.apply_update(&update).unwrap();
            let mut writer = MicroWriter::default();
            for _ in 0..100_000 {
                let commit = runtime.frame_to(&mut writer, &mut micro()).unwrap();
                if partial && writer.wire.iter().filter(|&&b| b == 0xcc).count() >= 3
                    || !partial && commit.committed
                {
                    break;
                }
            }
            let suffix = format!("{}\u{200d}💻", "\u{301}".repeat(1000));
            let update = runtime
                .append_bytes(format!("{suffix}\n\nEND\n").as_bytes())
                .unwrap()
                .unwrap();
            runtime.apply_update(&update).unwrap();
            if finish {
                let update = runtime.finish_input().unwrap().unwrap();
                runtime.apply_update(&update).unwrap();
            }
            let mut ready = false;
            for i in 0..100_000 {
                writer.block_next = i % 17 == 0;
                if runtime
                    .frame_to(&mut writer, &mut micro())
                    .unwrap()
                    .committed
                {
                    ready = true;
                    break;
                }
            }
            assert!(ready, "partial={partial} finish={finish}");
            assert_eq!(writer.wire.iter().filter(|&&b| b == 0xcc).count(), 2000);
            assert!(!writer.wire.contains(&24));
            let expected = format!("{prefix}{suffix}");
            assert!(
                writer
                    .wire
                    .windows(expected.len())
                    .any(|w| w == expected.as_bytes())
            );
            assert_eq!(
                runtime.screen().front_cell(0, 0).unwrap().text().len(),
                expected.len()
            );
            std::str::from_utf8(&writer.wire).unwrap();
        }
    }
}

#[test]
fn entity_and_unicode_osc8_abort_finish_only_the_current_scalar_tail() {
    use std::io::{self, Write};
    struct StopAtLead {
        wire: Vec<u8>,
        lead: u8,
        stopped: bool,
    }
    impl Write for StopAtLead {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if self.stopped {
                return Err(io::ErrorKind::WouldBlock.into());
            }
            self.wire.push(bytes[0]);
            self.stopped = bytes[0] == self.lead;
            Ok(1)
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let sources = std::iter::once((
        format!("&NotEqualTilde;{}", "\u{301}".repeat(5000)),
        LinkMode::NoOsc8,
        0xe2,
        vec![0x89, 0x82],
    ))
    .chain((0..8).map(|pad| {
        (
            format!("[label](/{pad}{}💻💻💻)", "x".repeat(pad)),
            LinkMode::Default,
            0xf0,
            vec![0x9f, 0x92, 0xbb],
        )
    }));
    for (source, mode, lead, expected_tail) in sources {
        let mut runtime = FrameRuntime::new(
            FrameGeometry {
                rows: 4,
                cols: 24,
                content_width: 24,
            },
            mode,
        )
        .unwrap();
        let update = runtime.append_bytes(source.as_bytes()).unwrap().unwrap();
        runtime.apply_update(&update).unwrap();
        let update = runtime.finish_input().unwrap().unwrap();
        runtime.apply_update(&update).unwrap();
        let mut writer = StopAtLead {
            wire: Vec::new(),
            lead,
            stopped: false,
        };
        for _ in 0..10000 {
            runtime.frame_to(&mut writer, &mut micro()).unwrap();
            if writer.stopped {
                break;
            }
        }
        assert!(writer.stopped);
        let mut tail = Vec::new();
        for _ in 0..100 {
            if runtime.abort_output(&mut tail, &mut micro()).unwrap() {
                break;
            }
        }
        let cleanup = if mode == LinkMode::NoOsc8 {
            b"\x18\x1b\\\x1b[0m\x1b[?2026l".as_slice()
        } else {
            b"\x18\x1b\\\x1b]8;;\x1b\\\x1b[0m\x1b[?2026l".as_slice()
        };
        assert_eq!(&tail[..expected_tail.len()], expected_tail);
        assert_eq!(&tail[expected_tail.len()..], cleanup);
        assert!(expected_tail.len() <= 3);
        writer.wire.extend_from_slice(&tail);
        std::str::from_utf8(&writer.wire).unwrap();
    }
}
