use mdtui_tui::{
    image::{
        cache::ResourceId,
        layout::{AnchorQuery, Band, ImageKey, Surface},
        worker::Geometry,
    },
    render::{
        first_frame::{FirstFrame, FirstFrameConfig, FirstFrameStatus},
        objects::SharedLinks,
    },
};
use mdtui3::{Budget, Document, LinkId, Span, TileId, WorkLimit};
fn full() -> Budget {
    Budget::new(WorkLimit::unlimited())
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
fn prepared(d: &Document, links: &SharedLinks) {
    for _ in 0..10000 {
        if links.prepare_shape(d, &mut full()).unwrap() {
            return;
        }
    }
    panic!("shape seed")
}
fn band(d: &Document, links: &SharedLinks, id: LinkId, rows: u32) -> Band {
    prepared(d, links);
    let info = d.link_info(id).unwrap();
    let mut query = AnchorQuery::new(info.source, info.label);
    let mut answer = None;
    assert_eq!(
        query
            .advance(d, links, &mut Budget::new(WorkLimit::default()))
            .unwrap(),
        None
    );
    for _ in 0..100000 {
        let mut b = micro();
        answer = query.advance(d, links, &mut b).unwrap();
        assert!(b.used().queries <= 1 && b.used().objects <= 1 && b.used().bytes <= 6);
        if answer.is_some() {
            break;
        }
    }
    Band {
        key: ImageKey {
            surface: links.image_bands().surface(),
            object: id,
            source_start: info.source.start,
        },
        source: info.source,
        label: info.label,
        hit: id,
        anchor: answer.unwrap(),
        resource: ResourceId {
            slot: 0,
            generation: 1,
        },
        geometry: Geometry { columns: 3, rows },
        generation: 1,
    }
}
fn layout(d: &Document, links: &SharedLinks) -> FirstFrame {
    let mut f = FirstFrame::new(
        d,
        FirstFrameConfig {
            top_tile: TileId(0),
            tile_limit: d.tile_count(),
            width: 24,
            row_limit: 100,
        },
    )
    .unwrap();
    f.set_links(links.clone());
    let z = f
        .advance(d, &mut Budget::new(WorkLimit::default()))
        .unwrap();
    assert_eq!(z.processed_tiles, 0);
    for _ in 0..100000 {
        let mut b = micro();
        let s = f.advance(d, &mut b).unwrap();
        assert!(
            b.used().queries <= 1
                && b.used().objects <= 1
                && b.used().bytes <= 6
                && b.used().tiles <= 1
        );
        if matches!(
            s.status,
            FirstFrameStatus::Covered | FirstFrameStatus::EndOfDocument
        ) {
            return f;
        }
    }
    panic!("band/native layout did not finish")
}
#[test]
fn actual_native_root_caption_anchor_height_and_decorative_cells() {
    let source = "![**a** 😀](oracle.png)\n\nTAIL\n";
    let mut d = Document::new();
    d.append(source).unwrap();
    d.finish();
    let links = SharedLinks::new(&d);
    let before = layout(&d, &links);
    let b = band(&d, &links, LinkId(0), 3);
    assert_eq!(b.anchor, source.find('😀').unwrap());
    assert!(
        !links
            .image_bands()
            .publish(&d, b, &mut Budget::new(WorkLimit::default()))
            .unwrap()
    );
    assert!(links.image_bands().is_empty());
    assert!(links.image_bands().publish(&d, b, &mut micro()).unwrap());
    let after = layout(&d, &links);
    assert_eq!(after.lines().len(), before.lines().len() + 3);
    let rows: Vec<_> = after
        .lines()
        .iter()
        .filter(|l| !l.images.is_empty())
        .collect();
    assert_eq!(rows.len(), 3);
    for (row, l) in rows.iter().enumerate() {
        let image = l.images[0];
        for (column, cell) in l.cells.iter().enumerate() {
            let owned = column >= usize::from(image.column)
                && column < usize::from(image.column + image.width);
            assert_eq!(cell.link, owned.then_some(image.band.hit));
            assert_eq!(cell.source, owned.then_some(image.band.source));
        }
        assert_eq!(l.source.unwrap().source, b.source);
        assert_eq!(l.images[0].row, row as u32);
        assert_eq!(l.images[0].band.key.surface, Surface::Root(d.id()));
    }
    assert_eq!(
        after
            .lines()
            .iter()
            .filter(|l| l.images.is_empty())
            .cloned()
            .collect::<Vec<_>>(),
        before.lines()
    );
}
#[test]
fn actual_table_images_share_row_height_and_retain_cell_columns_and_borders() {
    let mut d = Document::new();
    d.append("| A | B |\n|---|---|\n| ![x](a.png) | ![y](b.png) |\n")
        .unwrap();
    d.finish();
    let links = SharedLinks::new(&d);
    let before = layout(&d, &links);
    for (id, rows) in [(LinkId(0), 2), (LinkId(1), 3)] {
        let b = band(&d, &links, id, rows);
        links.image_bands().publish(&d, b, &mut full()).unwrap();
    }
    let after = layout(&d, &links);
    assert_eq!(
        after.lines().len(),
        before.lines().len() + 3,
        "two cells use max height rather than summing"
    );
    let rows: Vec<_> = after
        .lines()
        .iter()
        .filter(|l| !l.images.is_empty())
        .collect();
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0].images.len(), 2);
    assert_ne!(rows[0].images[0].column, rows[0].images[1].column);
    assert_eq!(rows[2].images.len(), 1);
    for l in rows {
        assert!(l.cells.iter().any(|c| c.text.contiguous() == Some("│")));
        for (column, cell) in l.cells.iter().enumerate() {
            if cell.text.contiguous() == Some("│") {
                assert_eq!(cell.link, None, "table border is not owned by pixels");
                assert_eq!(cell.source, None);
                continue;
            }
            let image = l.images.iter().find(|image| {
                column >= usize::from(image.column)
                    && column < usize::from(image.column + image.width)
            });
            assert_eq!(
                cell.link,
                image.map(|i| i.band.hit),
                "only actual image columns acquire object hit identity"
            );
            assert_eq!(
                cell.source,
                image.map(|i| i.band.source),
                "owned cells use the native object envelope, no fabricated byte positions"
            );
        }
    }
}
#[test]
fn body_key_namespace_and_zero_micro_band_query_cut_contract() {
    let mut d = Document::new();
    d.append("note[^n]\n\n[^n]: ![图](oracle.png)\n").unwrap();
    d.finish();
    let body = d.footnote_body(mdtui3::FootnoteId(0)).unwrap();
    let key = body.key();
    body.with_renderer(key, body.host_revision(), |native| {
        let links = SharedLinks::new(native);
        links.image_bands().bind_note(key).unwrap();
        let b = band(native, &links, LinkId(0), 2);
        assert_eq!(b.key.surface, Surface::Note(key));
        links.image_bands().publish(native, b, &mut full()).unwrap();
        let frame = layout(native, &links);
        assert_eq!(
            frame
                .lines()
                .iter()
                .filter(|l| !l.images.is_empty())
                .count(),
            2
        );
        let mut cut = links
            .image_bands()
            .query(Span::new(0, native.source().len()), b.source.end - 1);
        assert!(
            !cut.advance(native, None, &mut Budget::new(WorkLimit::default()))
                .unwrap()
        );
        for _ in 0..1000 {
            if cut.advance(native, None, &mut micro()).unwrap() {
                break;
            }
        }
        assert!(cut.take().unwrap().is_empty());
    })
    .unwrap();
}
#[test]
fn viewport_can_resume_inside_sparse_band_and_remeasure_current_geometry() {
    use mdtui_tui::render::viewport::{ViewportConfig, ViewportFrameStatus, ViewportState};
    let mut d = Document::new();
    d.append("![x](oracle.png)\n\nTAIL\n").unwrap();
    d.finish();
    let links = SharedLinks::new(&d);
    let mut b = band(&d, &links, LinkId(0), 300);
    links.image_bands().publish(&d, b, &mut full()).unwrap();
    let first = layout(&d, &links);
    let mut view = ViewportState::from_first_frame(
        &d,
        first,
        ViewportConfig {
            width: 24,
            rows: 2,
            overscan_rows: 2,
            ..Default::default()
        },
    )
    .unwrap();
    assert!(view.scroll_down(&d, 120).unwrap());
    fn ready(v: &mut ViewportState, d: &Document) -> mdtui_tui::render::viewport::ViewportFrame {
        for _ in 0..10000 {
            let f = v.frame(d, &mut full()).unwrap();
            if matches!(
                f.status,
                ViewportFrameStatus::Ready | ViewportFrameStatus::EndOfDocument
            ) {
                return f;
            }
        }
        panic!("sparse viewport")
    }
    let f = ready(&mut view, &d);
    assert_eq!(f.lines.len(), 2);
    assert!(f.lines.iter().all(|l| !l.images.is_empty()));
    assert_eq!(f.lines[0].images[0].row, 119);
    b.geometry.rows = 5;
    b.generation = 2;
    links.image_bands().publish(&d, b, &mut full()).unwrap();
    view.image_geometry_changed(&d, b.anchor).unwrap();
    let f = ready(&mut view, &d);
    assert!(
        !f.lines.is_empty(),
        "geometry shrink must retain a visible source position"
    );
    assert!(
        f.lines[0]
            .images
            .iter()
            .any(|i| i.band.key == b.key && i.row == 4),
        "shrink must clamp the same source image intrinsic row, not move into following text"
    );
    assert!(
        f.lines.iter().all(|l| l.images.iter().all(|i| i.row < 5)),
        "old band row cannot survive current geometry"
    );
}
#[test]
fn current_object_replacement_keeps_one_entry_and_old_queries_stale() {
    let mut d = Document::new();
    d.append("![abcd](oracle.png)\n").unwrap();
    d.finish();
    let links = SharedLinks::new(&d);
    let mut b = band(&d, &links, LinkId(0), 2);
    let shared = links.image_bands();
    shared.publish(&d, b, &mut full()).unwrap();
    let mut old = shared.query(Span::new(0, d.source().len()), d.source().len());
    for generation in 2..100 {
        b.generation = generation;
        b.anchor = b.label.start + (generation as usize) % 4;
        shared.publish(&d, b, &mut micro()).unwrap();
        assert_eq!(shared.len(), 1)
    }
    assert_eq!(
        old.advance(&d, None, &mut full()),
        Err(mdtui3::LayoutError::StaleContinuation)
    );
}
fn draw_runtime(r: &mut mdtui_tui::render::frame::FrameRuntime) -> Vec<u8> {
    let mut bytes = Vec::new();
    for _ in 0..100000 {
        if r.frame_to(
            &mut bytes,
            &mut Budget::new(WorkLimit {
                recovery: 1,
                dependencies: 1,
                ..micro().remaining()
            }),
        )
        .unwrap()
        .committed
            && !r.reading_pending()
        {
            return bytes;
        }
    }
    panic!(
        "real image surface did not settle: note={:?}, reading={}, fold={}, jump={}, stats={:?}",
        r.footnote_surface(),
        r.reading_pending(),
        r.fold_pending(),
        r.has_pending_jump(),
        r.footnote_stats()
    )
}
#[test]
fn real_runtime_band_publication_visible_copy_and_raw_source() {
    use base64::{Engine, engine::general_purpose::STANDARD};
    use crossterm::event::{KeyCode, KeyModifiers};
    use mdtui_tui::{
        cli::LinkMode,
        render::frame::{FrameGeometry, FrameRuntime},
    };
    let source = "![**a**b界](oracle.png)\n\nTAIL\n";
    let mut r = FrameRuntime::from_finished_source(
        source,
        FrameGeometry {
            rows: 8,
            cols: 24,
            content_width: 24,
        },
        LinkMode::NoOsc8,
    )
    .unwrap();
    draw_runtime(&mut r);
    let links = r.viewport().shared_links();
    let b = band(r.document(), &links, LinkId(0), 3);
    assert!(r.publish_image_band(b, &mut full()).unwrap());
    draw_runtime(&mut r);
    let image_hits = r
        .link_table()
        .unwrap()
        .hits_for(b.hit)
        .filter(|h| h.row > 0)
        .copied()
        .collect::<Vec<_>>();
    assert!(
        image_hits.len() >= 3,
        "physical image rows need native click hit ranges"
    );
    let image_hit = image_hits.last().unwrap();
    assert!(r.focus_link_at(image_hit.row, image_hit.x0));
    r.clear_link_focus();
    for key in [
        KeyCode::Char('v'),
        KeyCode::Char(' '),
        KeyCode::Right,
        KeyCode::Right,
    ] {
        assert!(r.reading_key(key, KeyModifiers::NONE, true).unwrap());
        draw_runtime(&mut r);
    }
    let span = r.reading().selection.range().unwrap();
    assert!(span.start < span.end);
    let raw = &source[span.start..span.end];
    assert_eq!(raw, "a**b界");
    for (key, expected) in [(KeyCode::Char('y'), "ab界"), (KeyCode::Char('Y'), raw)] {
        assert!(r.reading_key(key, KeyModifiers::NONE, true).unwrap());
        let bytes = draw_runtime(&mut r);
        let prefix = b"\x1b]52;c;";
        let start = bytes
            .windows(prefix.len())
            .position(|p| p == prefix)
            .expect("actual native copy packet")
            + prefix.len();
        let end = start + bytes[start..].iter().position(|p| *p == 7).unwrap();
        assert_eq!(
            STANDARD.decode(&bytes[start..end]).unwrap(),
            expected.as_bytes()
        );
    }
}
#[test]
fn actual_note_runtime_receives_body_qualified_band_without_root_rows() {
    use crossterm::event::{KeyCode, KeyModifiers};
    use mdtui_tui::{
        cli::LinkMode,
        render::frame::{FrameGeometry, FrameRuntime},
    };
    let mut r = FrameRuntime::from_finished_source(
        "root[^n]\n\n[^n]: ![abc](oracle.png)\n\n    TAIL\n",
        FrameGeometry {
            rows: 12,
            cols: 32,
            content_width: 32,
        },
        LinkMode::NoOsc8,
    )
    .unwrap();
    draw_runtime(&mut r);
    assert!(
        r.footnote_key(KeyCode::Char('f'), KeyModifiers::NONE, true)
            .unwrap()
    );
    draw_runtime(&mut r);
    assert!(
        r.footnote_key(KeyCode::Enter, KeyModifiers::NONE, true)
            .unwrap()
    );
    draw_runtime(&mut r);
    let key = r.footnote_surface().unwrap();
    let baseline = (0..r.screen().rows())
        .map(|row| {
            r.screen()
                .front_row(row)
                .unwrap()
                .iter()
                .map(|c| c.text().collect_text())
                .collect::<String>()
        })
        .collect::<Vec<_>>();
    let baseline_gap = baseline.iter().position(|l| l.contains("TAIL")).unwrap()
        - baseline.iter().position(|l| l.contains("abc")).unwrap();
    let body = r.document().footnote_body(key.symbol()).unwrap();
    let b = body
        .with_renderer(key, body.host_revision(), |native| {
            let links = SharedLinks::new(native);
            links.image_bands().bind_note(key).unwrap();
            band(native, &links, LinkId(0), 3)
        })
        .unwrap();
    assert!(r.publish_image_band(b, &mut full()).unwrap());
    eprintln!("published note band; entering real frame");
    draw_runtime(&mut r);
    let text = (0..r.screen().rows())
        .map(|row| {
            r.screen()
                .front_row(row)
                .unwrap()
                .iter()
                .map(|c| c.text().collect_text())
                .collect::<String>()
        })
        .collect::<Vec<_>>();
    let caption = text.iter().position(|l| l.contains("abc")).unwrap();
    let tail = text.iter().position(|l| l.contains("TAIL")).unwrap();
    assert_eq!(
        tail - caption,
        baseline_gap + 3,
        "note image band must contribute exactly its three physical rows: {text:?}"
    );
    let pixel_row = caption as u16 + 1;
    assert_eq!(
        r.screen().front_cell(pixel_row, 0).unwrap().source(),
        Some(b.source)
    );
    assert!(
        r.focus_footnote_at(pixel_row, 0),
        "actual BodyKey note click path must include physical image cells"
    );
    assert_eq!(r.footnote_surface(), Some(key));
}
#[test]
fn native_thousand_row_geometry_adopts_only_visible_rows_with_paid_micro_steps() {
    let mut d = Document::new();
    d.append("![x](oracle.png)\n").unwrap();
    d.finish();
    let links = SharedLinks::new(&d);
    let b = band(&d, &links, LinkId(0), 1000);
    links.image_bands().publish(&d, b, &mut full()).unwrap();
    let mut f = FirstFrame::new(
        &d,
        FirstFrameConfig {
            top_tile: TileId(0),
            tile_limit: d.tile_count(),
            width: 24,
            row_limit: 100,
        },
    )
    .unwrap();
    f.set_links(links);
    let mut saw_pause = false;
    let mut finished = false;
    for _ in 0..100000 {
        let before = f.sink().map_or(0, |s| s.stats().image_rows_adopted);
        let mut credit = micro();
        let step = f.advance(&d, &mut credit).unwrap();
        let after = f.sink().map_or(0, |s| s.stats().image_rows_adopted);
        assert!(
            after - before <= credit.used().queries as u64,
            "image row handles cannot all move in one unmetered core end_tile callback"
        );
        if after > 0 && after < 99 {
            saw_pause = true;
            let before = f.sink().unwrap().stats();
            f.advance(&d, &mut Budget::new(WorkLimit::default()))
                .unwrap();
            assert_eq!(
                f.sink().unwrap().stats().image_rows_adopted,
                before.image_rows_adopted
            )
        }
        if matches!(
            step.status,
            FirstFrameStatus::Covered | FirstFrameStatus::EndOfDocument
        ) {
            finished = true;
            break;
        }
    }
    assert!(finished && saw_pause);
    let sink = f.sink().unwrap();
    assert_eq!(sink.stats().image_rows_adopted, 99);
    assert_eq!(sink.tile_rows(TileId(0)), Some(1001));
    assert_eq!(
        f.lines().iter().filter(|l| !l.images.is_empty()).count(),
        99
    );
}
#[test]
fn actual_fold_visibility_and_cut_exclude_whole_images_without_removing_objects() {
    use mdtui3::{FoldId, FoldView, HeadingId};
    let mut d = Document::new();
    d.append("# A\n\n![abc](oracle.png)\n\n# B\nTAIL\n")
        .unwrap();
    d.finish();
    let links = SharedLinks::new(&d);
    let b = band(&d, &links, LinkId(0), 4);
    let shared = links.image_bands();
    shared.publish(&d, b, &mut full()).unwrap();
    let mut folds =
        FoldView::<mdtui_tui::render::checkpoint::RenderCheckpoint>::with_state(&d, 1).unwrap();
    for collapsed in [false, true, false] {
        folds
            .set_collapsed(&d, FoldId::Heading(HeadingId(0)), collapsed)
            .unwrap();
        let mut q = shared.query(Span::new(0, d.source().len()), d.source().len());
        let mut done = false;
        for _ in 0..10000 {
            if q.advance(&d, Some(&folds), &mut micro()).unwrap() {
                done = true;
                break;
            }
        }
        assert!(done);
        assert_eq!(q.take().unwrap().len(), usize::from(!collapsed));
        assert_eq!(
            shared.len(),
            1,
            "fold visibility is a view, not source object deletion"
        );
    }
    let mut q = shared.query(Span::new(0, d.source().len()), b.source.end - 1);
    while !q.advance(&d, Some(&folds), &mut micro()).unwrap() {}
    assert!(q.take().unwrap().is_empty());
}
#[test]
fn withdrawing_current_band_restores_its_caption_instead_of_following_text() {
    use mdtui_tui::render::viewport::{ViewportConfig, ViewportFrameStatus, ViewportState};
    let mut d = Document::new();
    d.append("![x](oracle.png)\n\nTAIL\n").unwrap();
    d.finish();
    let links = SharedLinks::new(&d);
    let b = band(&d, &links, LinkId(0), 300);
    links.image_bands().publish(&d, b, &mut full()).unwrap();
    let first = layout(&d, &links);
    let mut v = ViewportState::from_first_frame(
        &d,
        first,
        ViewportConfig {
            width: 24,
            rows: 2,
            overscan_rows: 2,
            ..Default::default()
        },
    )
    .unwrap();
    assert!(v.scroll_down(&d, 120).unwrap());
    let mut ready = None;
    for _ in 0..10000 {
        let frame = v.frame(&d, &mut full()).unwrap();
        if matches!(
            frame.status,
            ViewportFrameStatus::Ready | ViewportFrameStatus::EndOfDocument
        ) {
            ready = Some(frame);
            break;
        }
    }
    assert_eq!(ready.unwrap().lines[0].images[0].row, 119);
    assert!(links.image_bands().withdraw(b, &mut micro()));
    v.image_band_removed(&d, b).unwrap();
    let mut ready = None;
    for _ in 0..10000 {
        let frame = v.frame(&d, &mut full()).unwrap();
        if matches!(
            frame.status,
            ViewportFrameStatus::Ready | ViewportFrameStatus::EndOfDocument
        ) {
            ready = Some(frame);
            break;
        }
    }
    let frame = ready.unwrap();
    assert!(frame.lines.iter().all(|l| l.images.is_empty()));
    let top = frame
        .lines
        .first()
        .unwrap()
        .cells
        .iter()
        .map(|c| c.text.collect_text())
        .collect::<String>();
    assert!(
        top.contains('x'),
        "removed image must restore its same caption, not the old intrinsic offset in following text: {top:?}"
    );
}
#[test]
fn stale_geometry_owner_cannot_withdraw_a_new_generation_at_same_coordinates() {
    let mut d = Document::new();
    d.append("![x](oracle.png)\n").unwrap();
    d.finish();
    let links = SharedLinks::new(&d);
    let old = band(&d, &links, LinkId(0), 3);
    let shared = links.image_bands();
    shared.publish(&d, old, &mut micro()).unwrap();
    let mut current = old;
    current.generation += 1;
    current.resource.generation += 1;
    current.geometry.rows = 4;
    shared.publish(&d, current, &mut micro()).unwrap();
    let version = shared.version();
    assert!(!shared.withdraw(current, &mut Budget::new(WorkLimit::default())));
    assert_eq!(shared.version(), version);
    assert!(shared.withdraw(old, &mut micro()));
    assert_eq!(shared.version(), version);
    assert_eq!(shared.len(), 1);
    let mut q = shared.query(Span::new(0, d.source().len()), d.source().len());
    for _ in 0..1000 {
        if q.advance(&d, None, &mut micro()).unwrap() {
            break;
        }
    }
    assert_eq!(q.take().unwrap(), vec![current]);
    assert!(shared.withdraw(current, &mut micro()));
    assert!(shared.is_empty());
    assert!(shared.version() > version);
}
#[test]
fn actual_native_end_uses_last_raster_rows_and_restarts_after_withdrawal() {
    use mdtui_tui::render::viewport::{ViewportConfig, ViewportFrameStatus, ViewportState};
    let mut d = Document::new();
    d.append("![x](oracle.png)\n").unwrap();
    d.finish();
    let links = SharedLinks::new(&d);
    let b = band(&d, &links, LinkId(0), 1000);
    links.image_bands().publish(&d, b, &mut full()).unwrap();
    let first = layout(&d, &links);
    let mut v = ViewportState::from_first_frame(
        &d,
        first,
        ViewportConfig {
            width: 24,
            rows: 4,
            overscan_rows: 2,
            ..Default::default()
        },
    )
    .unwrap();
    v.seek_end(&d, false).unwrap();
    let mut ready = None;
    for _ in 0..100000 {
        let mut credit = Budget::new(WorkLimit {
            recovery: 1,
            dependencies: 1,
            ..micro().remaining()
        });
        let frame = v.frame(&d, &mut credit).unwrap();
        assert!(
            credit.used().queries <= 1 && credit.used().objects <= 1 && credit.used().bytes <= 6
        );
        if matches!(
            frame.status,
            ViewportFrameStatus::Ready | ViewportFrameStatus::EndOfDocument
        ) {
            ready = Some(frame);
            break;
        }
    }
    let frame = ready.expect("native End should complete under the same micro Budget");
    let rows = frame
        .lines
        .iter()
        .flat_map(|l| l.images.iter().map(|i| i.row))
        .collect::<Vec<_>>();
    assert_eq!(
        rows,
        vec![996, 997, 998, 999],
        "End must include the physical final raster rows rather than stop at its caption"
    );
    links.image_bands().withdraw(b, &mut micro());
    v.image_band_removed(&d, b).unwrap();
    let mut ready = None;
    for _ in 0..100000 {
        let mut credit = Budget::new(WorkLimit {
            recovery: 1,
            dependencies: 1,
            ..micro().remaining()
        });
        let frame = v.frame(&d, &mut credit).unwrap();
        if matches!(
            frame.status,
            ViewportFrameStatus::Ready | ViewportFrameStatus::EndOfDocument
        ) {
            ready = Some(frame);
            break;
        }
    }
    let frame = ready.unwrap();
    assert!(frame.lines.iter().all(|l| l.images.is_empty()));
    assert!(
        frame
            .lines
            .iter()
            .flat_map(|l| &l.cells)
            .any(|c| c.text.contiguous() == Some("x"))
    );
}
#[test]
fn immutable_native_frame_capture_reads_only_band_metadata_and_retires_owned_rows() {
    use mdtui_tui::{
        image::scene::{Capture, SceneError},
        render::viewport::{ViewportConfig, ViewportFrameStatus, ViewportState},
    };
    use std::rc::Rc;
    let mut d = Document::new();
    d.append("![图😀](oracle.png)\n\nTAIL\n").unwrap();
    d.finish();
    let links = SharedLinks::new(&d);
    let b = band(&d, &links, LinkId(0), 2);
    links.image_bands().publish(&d, b, &mut micro()).unwrap();
    let first = layout(&d, &links);
    let mut v = ViewportState::from_first_frame(
        &d,
        first,
        ViewportConfig {
            width: 24,
            rows: 6,
            ..Default::default()
        },
    )
    .unwrap();
    let mut ready = None;
    for _ in 0..10000 {
        let f = v.frame(&d, &mut full()).unwrap();
        if matches!(
            f.status,
            ViewportFrameStatus::Ready | ViewportFrameStatus::EndOfDocument
        ) {
            ready = Some(f);
            break;
        }
    }
    let mut frame = Rc::new(ready.unwrap());
    let pointer = Rc::as_ptr(&frame);
    let cell = mdtui_tui::image::worker::CellPixels {
        width: 1,
        height: 1,
    };
    let (error, returned) = Capture::new(
        frame.clone(),
        Surface::Root(d.id()),
        cell,
        0,
        24,
        6,
        &mut Budget::new(WorkLimit::default()),
    )
    .err()
    .unwrap();
    assert_eq!(error, SceneError::Budget);
    assert_eq!(Rc::as_ptr(&returned), pointer);
    drop(returned);
    let mut capture = Capture::new(
        frame.clone(),
        Surface::Root(d.id()),
        cell,
        0,
        24,
        6,
        &mut micro(),
    )
    .ok()
    .unwrap();
    assert!(
        Rc::get_mut(&mut frame).is_none(),
        "the actual fixed frame is bound, not a callback or a whole query-time copy"
    );
    assert!(capture.frame_handle_capacity() > 0);
    assert_eq!(
        capture.advance(&mut Budget::new(WorkLimit::default())),
        Ok(false)
    );
    assert_eq!(capture.work.row_queries, 0);
    let mut done = false;
    for _ in 0..1000 {
        let mut b = micro();
        if capture.advance(&mut b).unwrap() {
            done = true;
            break;
        }
        assert!(b.used().queries <= 1 && b.used().objects <= 1)
    }
    assert!(done);
    assert_eq!(capture.work.records, 2);
    let scene = capture.take().unwrap();
    assert_eq!(scene.len(), 1);
    while !capture.retire(&mut micro()) {}
    assert!(Rc::get_mut(&mut frame).is_some());
    assert_eq!(
        capture.work.retired_cells, 0,
        "shared native owner keeps its cells"
    );
    let mut unique = Capture::new(frame, Surface::Root(d.id()), cell, 0, 24, 6, &mut micro())
        .ok()
        .unwrap();
    assert!(!unique.retire(&mut Budget::new(WorkLimit::default())));
    assert_eq!(unique.work.retired_cells, 0);
    let mut done = false;
    for _ in 0..10000 {
        let before =
            unique.work.retired_cells + unique.work.retired_records + unique.work.retired_rows;
        let mut b = micro();
        done = unique.retire(&mut b);
        let after =
            unique.work.retired_cells + unique.work.retired_records + unique.work.retired_rows;
        assert!(after - before <= b.used().queries as u64);
        if done {
            break;
        }
    }
    assert!(done);
    assert!(unique.work.retired_cells > 0);
    assert_eq!(unique.frame_handle_capacity(), 0);
    assert_eq!(unique.advance(&mut micro()), Err(SceneError::Replaced));
}
#[test]
fn adding_real_image_rows_preserves_native_heading_and_list_link_dialect() {
    let mut d = Document::new();
    d.append("# [heading](https://example.test/h)\n\n- [list](https://example.test/l)\n\nplain [body](https://example.test/b)\n\n![x](oracle.png)\n").unwrap();
    d.finish();
    let links = SharedLinks::new(&d);
    let before = layout(&d, &links);
    let id = (0..8)
        .map(LinkId)
        .find(|id| {
            d.link_info(*id)
                .is_some_and(|i| i.kind == mdtui3::LinkKind::Image)
        })
        .unwrap();
    let b = band(&d, &links, id, 3);
    links.image_bands().publish(&d, b, &mut micro()).unwrap();
    let after = layout(&d, &links);
    let remaining = after
        .lines()
        .iter()
        .filter(|l| l.images.is_empty())
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(
        remaining,
        before.lines(),
        "image bands may add pixel rows but cannot expand the separate native heading/list plain-link dialect"
    );
}
