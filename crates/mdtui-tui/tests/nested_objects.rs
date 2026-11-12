use mdtui_tui::render::first_frame::{FirstFrame, FirstFrameConfig, FirstFrameStatus};
use mdtui_tui::render::objects::SharedLinks;
use mdtui_tui::render::viewport::{
    ViewportConfig, ViewportFrame, ViewportFrameStatus, ViewportState,
};
use mdtui_tui::screen::{ContentArea, Screen};
use mdtui_tui::static_render::render_static;
use mdtui3::{Budget, Document, Span, TileId, WorkLimit};

fn doc(source: &str) -> Document {
    let mut d = Document::new();
    d.append(source).unwrap();
    d
}
fn ready(state: &mut ViewportState, doc: &Document) -> ViewportFrame {
    for _ in 0..100_000 {
        let mut budget = Budget::new(mdtui_tui::stream::FRAME_WORK);
        let frame = state.frame(doc, &mut budget).unwrap();
        assert!(budget.used().queries <= mdtui_tui::stream::FRAME_WORK.queries);
        if matches!(
            frame.status,
            ViewportFrameStatus::Ready | ViewportFrameStatus::EndOfDocument
        ) {
            return frame;
        }
    }
    panic!("frame did not converge");
}
fn text(frame: &ViewportFrame) -> String {
    frame
        .lines
        .iter()
        .flat_map(|l| &l.cells)
        .map(|c| c.text.as_str())
        .collect()
}

#[test]
fn nested_alt_is_flattened_and_hit_targets_are_independent_of_raw_allocation_order() {
    for (source, expected, target) in [
        ("![outer ![](/inner)](/outer)\n", "outer", "/outer"),
        (
            "![outer ![][missing]](/outer)\n",
            "outer ![][missing]",
            "/outer",
        ),
        (
            "![outer ![alt][missing]](/outer)\n",
            "outer ![alt][missing]",
            "/outer",
        ),
        ("![outer [missing]](/outer)\n", "outer [missing]", "/outer"),
        ("![![](/inner)](/outer)\n", "[image]", "/outer"),
        ("![outer [text](/link)](/outer)\n", "outer text", "/outer"),
        ("[![alt](/img)](/page)\n", "alt", "/page"),
        ("[![](/img)](/page)\n", "[image]", "/page"),
        ("![![alt](/inner)](/outer)\n", "alt", "/outer"),
        (
            "![outer ![inner](/inner)](/outer)\n",
            "outer inner",
            "/outer",
        ),
    ] {
        let d = doc(source);
        let mut state = ViewportState::new(&d, ViewportConfig::default()).unwrap();
        let frame = ready(&mut state, &d);
        assert_eq!(text(&frame).trim_end(), expected, "{source}");
        for cell in frame
            .lines
            .iter()
            .flat_map(|l| &l.cells)
            .filter(|c| !c.text.trim().is_empty())
        {
            assert_eq!(
                cell.link.and_then(|id| d.link_destination(id)),
                Some(target),
                "{source} {cell:?}"
            );
            assert!(cell.source.is_some());
        }
        let static_frame = render_static(&d, 80, 24).unwrap();
        let static_text: String = (0..80)
            .map(|x| static_frame.screen.cell(0, x).unwrap().text())
            .collect();
        assert_eq!(static_text.trim_end(), expected);
        let ansi = mdtui_tui::static_render::render_once_ansi(
            &d,
            80,
            24,
            mdtui_tui::cli::LinkMode::Default,
        )
        .unwrap();
        let mut terminal = vt100::Parser::new(
            std::num::NonZeroU16::new(24).unwrap(),
            std::num::NonZeroU16::new(80).unwrap(),
            0,
        );
        terminal.process(&ansi);
        assert_eq!(terminal.screen().contents().trim_end(), expected);
        let osc = format!("\x1b]8;;{target}\x1b\\");
        assert!(
            ansi.windows(osc.len())
                .any(|window| window == osc.as_bytes())
        );
        assert!(!d.is_finished());
    }
}

#[test]
fn image_in_link_keeps_outer_hit_in_heading_list_quote_and_table() {
    for source in [
        "# [![alt](/img)](/page)\n",
        "- [![alt](/img)](/page)\n",
        "> [![alt](/img)](/page)\n",
        "| image |\n| --- |\n| [![alt](/img)](/page) |\n",
    ] {
        let d = doc(source);
        let mut state = ViewportState::new(&d, ViewportConfig::default()).unwrap();
        let frame = ready(&mut state, &d);
        assert!(text(&frame).contains("alt"), "{source}");
        let linked: Vec<_> = frame
            .lines
            .iter()
            .flat_map(|l| &l.cells)
            .filter(|c| c.link.is_some())
            .collect();
        assert_eq!(linked.len(), 3, "{source}");
        assert!(
            linked
                .iter()
                .all(|c| c.link.and_then(|id| d.link_destination(id)) == Some("/page"))
        );
    }
}

#[test]
fn late_child_definition_suppresses_parent_hit_but_keeps_the_real_child_image() {
    let mut d = doc("[![alt](/img) [r]](/page)\n\n");
    let mut state = ViewportState::new(&d, ViewportConfig::default()).unwrap();
    let before = ready(&mut state, &d);
    assert!(
        before
            .lines
            .iter()
            .flat_map(|l| &l.cells)
            .any(|c| c.link.and_then(|id| d.link_destination(id)) == Some("/page"))
    );
    let update = d.append("[r]: /inner\n").unwrap();
    state.apply_update(&d, &update).unwrap();
    let after = ready(&mut state, &d);
    assert!(!before.matches(&d));
    let targets: Vec<_> = after
        .lines
        .iter()
        .flat_map(|l| &l.cells)
        .filter_map(|c| c.link.and_then(|id| d.link_destination(id)))
        .collect();
    assert!(targets.contains(&"/img"));
    assert!(targets.contains(&"/inner"));
    assert!(!targets.contains(&"/page"));
}

#[test]
fn deep_cross_tile_alt_and_revision_cancel_share_and_retire_one_memo_under_micro_budget() {
    let depth = 512;
    let source = format!("{}alt{}\n", "![".repeat(depth), "](/image)".repeat(depth));
    let mut d = doc(&source);
    let links = SharedLinks::new(&d);
    let mut first = FirstFrame::new(
        &d,
        FirstFrameConfig {
            top_tile: d.tile_at(depth * 2).unwrap(),
            tile_limit: 4,
            width: 80,
            row_limit: 40,
        },
    )
    .unwrap();
    first.set_links(links.clone());
    let limit = WorkLimit {
        queries: 1,
        objects: 1,
        bytes: 6,
        tiles: 1,
        recovery: 1,
        dependencies: 1,
    };
    let mut steps = 0;
    loop {
        steps += 1;
        assert!(steps < 100_000);
        let mut budget = Budget::new(limit);
        let status = first.advance(&d, &mut budget).unwrap().status;
        assert!(
            budget.used().queries <= 1 && budget.used().objects <= 1 && budget.used().bytes <= 6
        );
        if matches!(
            status,
            FirstFrameStatus::Covered | FirstFrameStatus::EndOfDocument
        ) {
            break;
        }
    }
    let mut screen = Screen::new(40, 80, ContentArea::new(0, 80)).unwrap();
    first.write_to_screen(&mut screen, 0).unwrap();
    let cells: String = (0..80).map(|x| screen.cell(0, x).unwrap().text()).collect();
    assert_eq!(cells.trim_end(), "alt");
    assert_eq!(
        screen.cell(0, 0).unwrap().link_id(),
        d.links_in(Span::new(0, 1)).next().map(|(id, _)| id)
    );
    let before = links.stats();
    assert!(before.memo_entries >= depth);
    drop(first);
    assert_eq!(links.stats(), before);
    for _ in 0..100 {
        d.append("x").unwrap();
    }
    let mut zero = Budget::new(WorkLimit::default());
    assert!(!links.prepare(&d, &mut zero).unwrap());
    assert_eq!(links.stats(), before);
    let mut calls = 0;
    loop {
        calls += 1;
        let mut budget = Budget::new(limit);
        if links.prepare(&d, &mut budget).unwrap() {
            break;
        }
        assert_eq!(budget.used().queries, 1);
        assert_eq!(budget.used().objects, 1);
    }
    let after = links.stats();
    assert_eq!(after.memo_entries, 0);
    assert_eq!(after.presentation_entries, 0);
    assert_eq!(
        after.resolution.retired_entries - before.resolution.retired_entries,
        before.memo_entries as u64
    );
    assert_eq!(
        after.retired_presentations - before.retired_presentations,
        before.presentation_entries as u64
    );
    assert_eq!(calls, before.memo_entries + before.presentation_entries);
    println!("nested budget steps={steps} before={before:?} after={after:?} retire_frames={calls}");
}

#[test]
fn cancelling_remote_guard_mid_resolution_retires_actual_paid_entries_without_revision_queue() {
    let source = format!(
        "[![alt](/img) {}](/outer)\n\n",
        (0..1024).map(|n| format!("[r{n}] ")).collect::<String>()
    );
    let mut d = doc(&source);
    let links = SharedLinks::new(&d);
    let mut first = FirstFrame::new(
        &d,
        FirstFrameConfig {
            top_tile: TileId(0),
            tile_limit: 2,
            width: 80,
            row_limit: 24,
        },
    )
    .unwrap();
    first.set_links(links.clone());
    let limit = WorkLimit {
        queries: 1,
        objects: 1,
        bytes: 6,
        tiles: 1,
        recovery: 1,
        dependencies: 1,
    };
    for _ in 0..20_000 {
        let step = first.advance(&d, &mut Budget::new(limit)).unwrap();
        if links.stats().memo_entries >= 100 {
            assert!(!matches!(
                step.status,
                FirstFrameStatus::Covered | FirstFrameStatus::EndOfDocument
            ));
            break;
        }
    }
    let before = links.stats();
    assert!(before.resolution.guard_steps > 100);
    assert!(before.resolution.target_reads > 0);
    assert!(before.memo_entries >= 100);
    links.request_retirement();
    let mut same_revision_zero = Budget::new(WorkLimit::default());
    assert!(links.prepare(&d, &mut same_revision_zero).unwrap());
    assert_eq!(links.stats(), before); // the pending task prevents token reset
    assert_eq!(same_revision_zero.used(), WorkLimit::default());
    drop(first);
    for _ in 0..100 {
        d.append("x").unwrap();
    }
    assert!(
        !links
            .prepare(&d, &mut Budget::new(WorkLimit::default()))
            .unwrap()
    );
    assert_eq!(links.stats(), before);
    let mut paid = 0;
    loop {
        let mut budget = Budget::new(limit);
        let done = links.prepare(&d, &mut budget).unwrap();
        paid += budget.used().objects;
        assert!(
            budget.used().queries <= 1 && budget.used().objects <= 1 && budget.used().bytes == 0
        );
        if done {
            break;
        }
    }
    let after = links.stats();
    assert_eq!(paid, before.memo_entries + before.presentation_entries);
    assert_eq!(
        after.resolution.retired_entries - before.resolution.retired_entries,
        before.memo_entries as u64
    );
    assert_eq!(after.presentation_entries, 0);
    assert_eq!(after.memo_entries, 0);
    println!("mid-guard cancellation before={before:?} after={after:?} paid_retirement={paid}");
}

#[test]
fn empty_code_constructor_and_zero_budget_do_not_probe_following_remote_guards() {
    let d = doc(&format!(
        "```\n```\n\n[a {}](/outer)\n\n",
        (0..4096).map(|n| format!("[r{n}] ")).collect::<String>()
    ));
    let mut first = FirstFrame::new(
        &d,
        FirstFrameConfig {
            top_tile: TileId(0),
            tile_limit: 1,
            width: 80,
            row_limit: 24,
        },
    )
    .unwrap();
    let (reads, work) = first.code_prefix_work();
    assert!(reads <= 3);
    assert_eq!(work, mdtui3::LinkResolutionWork::default());
    assert_eq!(first.plan().unbudgeted_drawable_queries, 0);
    let links = first.shared_links();
    assert_eq!(links.stats().memo_entries, 0);
    let mut zero = Budget::new(WorkLimit::default());
    assert_eq!(
        first.advance(&d, &mut zero).unwrap().status,
        FirstFrameStatus::PendingScan
    );
    assert_eq!(zero.used(), WorkLimit::default());
    assert_eq!(links.stats().resolution, work);
    println!("empty-code bounded metadata_reads={reads} following-guard work={work:?}");
}

#[test]
fn table_width_remote_dependencies_resume_and_late_definition_is_a_real_cache_miss() {
    use mdtui_tui::render::input::DisplayInput;
    use mdtui_tui::render::table_widths::TableWidthCache;
    use mdtui_tui::render::text::display_width;
    let mut d = doc(&format!(
        "| H |\n| --- |\n| [a {}](/outer) |\n\n",
        (0..128).map(|n| format!("[r{n}] ")).collect::<String>()
    ));
    assert_eq!(d.tables().len(), 1);
    let id = mdtui3::TableId(0);
    let links = SharedLinks::new(&d);
    let mut cache = TableWidthCache::new(&d);
    cache.set_links(links.clone());
    let limit = WorkLimit {
        queries: 1,
        objects: 1,
        bytes: 6,
        tiles: 1,
        recovery: 1,
        dependencies: 1,
    };
    let mut widths = Vec::new();
    for revision in 0..2 {
        for _ in 0..100_000 {
            let mut budget = Budget::new(limit);
            let step = cache.advance(&d, id, 2000, &mut budget).unwrap();
            assert!(
                budget.used().queries <= 1
                    && budget.used().objects <= 1
                    && budget.used().bytes <= 6
            );
            if step.completion == mdtui3::Completion::Covered {
                break;
            }
        }
        let snapshot = cache.snapshot(&d, id).unwrap();
        assert!(!snapshot.provisional);
        let cell = d.tables()[0].cell(1, 0).unwrap().source;
        assert_eq!(
            snapshot.widths[0],
            display_width(
                &DisplayInput::from_document(&d, cell)
                    .unwrap()
                    .visible_text()
            )
            .max(1)
        );
        widths.push(snapshot.widths[0]);
        if revision == 0 {
            let before = links.stats();
            assert!(before.resolution.guard_steps > 128);
            let update = d.append("[r0]: /inner\n").unwrap();
            assert!(cache.apply_update(&d, &update).unwrap());
            assert!(cache.snapshot(&d, id).unwrap().provisional);
            assert!(
                !links
                    .prepare(&d, &mut Budget::new(WorkLimit::default()))
                    .unwrap()
            );
            assert_eq!(links.stats(), before);
        }
    }
    assert_ne!(widths[0], widths[1]);
    println!(
        "nested table widths={widths:?} shared={:?} cache={:?}",
        links.stats(),
        cache.memory_stats()
    );
}

#[test]
fn focused_parent_is_revalidated_before_activation_after_a_late_definition() {
    use mdtui_tui::cli::LinkMode;
    use mdtui_tui::fragment::FragmentActivationStatus;
    use mdtui_tui::render::frame::{FrameGeometry, FrameRuntime};
    let mut runtime = FrameRuntime::new(
        FrameGeometry {
            rows: 24,
            cols: 80,
            content_width: 80,
        },
        LinkMode::Default,
    )
    .unwrap();
    let update = runtime
        .append_bytes(b"[![alt](/img) [r]](/page)\n\n")
        .unwrap()
        .unwrap();
    runtime.apply_update(&update).unwrap();
    for _ in 0..100 {
        if runtime
            .frame_to(
                &mut Vec::new(),
                &mut Budget::new(mdtui_tui::stream::FRAME_WORK),
            )
            .unwrap()
            .committed
        {
            break;
        }
    }
    let focused = runtime.enter_link_focus(true).unwrap();
    assert_eq!(runtime.document().link_destination(focused), Some("/page"));
    let update = runtime.append_bytes(b"[r]: /inner\n").unwrap().unwrap();
    runtime.apply_update(&update).unwrap();
    // The source ID and raw destination still exist; neither is an active hit.
    assert_eq!(runtime.document().link_destination(focused), Some("/page"));
    let limit = WorkLimit {
        queries: 1,
        objects: 1,
        bytes: 6,
        tiles: 1,
        recovery: 1,
        dependencies: 1,
    };
    for _ in 0..10_000 {
        let mut budget = Budget::new(limit);
        let result = runtime.activate_focused_link(&mut budget).unwrap();
        assert!(budget.used().queries <= 1 && budget.used().objects <= 1);
        if result.status != FragmentActivationStatus::BudgetExhausted {
            assert_eq!(result.status, FragmentActivationStatus::Invalid);
            return;
        }
    }
    panic!("activation validation did not finish");
}

#[test]
fn unresolved_parent_does_not_hide_literal_syntax_or_override_real_child_targets() {
    for (source, expected, target) in [
        (
            "![outer [text](/link)][missing]\n",
            "![outer text][missing]",
            "/link",
        ),
        (
            "![outer ![](/inner)][missing]\n",
            "![outer [image]][missing]",
            "/inner",
        ),
        (
            "![outer ![inner](/img)][missing]\n",
            "![outer inner][missing]",
            "/img",
        ),
        ("[![alt](/img)][missing]\n", "[alt][missing]", "/img"),
    ] {
        let d = doc(source);
        let mut state = ViewportState::new(&d, ViewportConfig::default()).unwrap();
        let frame = ready(&mut state, &d);
        assert_eq!(text(&frame), expected, "{source}");
        let targets: Vec<_> = frame
            .lines
            .iter()
            .flat_map(|l| &l.cells)
            .filter_map(|c| c.link.and_then(|id| d.link_destination(id)))
            .collect();
        assert!(!targets.is_empty());
        assert!(targets.iter().all(|&actual| actual == target));
        let static_frame = render_static(&d, 80, 24).unwrap();
        let static_text: String = (0..80)
            .map(|x| static_frame.screen.cell(0, x).unwrap().text())
            .collect();
        assert_eq!(static_text.trim_end(), expected);
    }
}

#[test]
fn unresolved_empty_images_are_literal_and_composite_empty_alt_has_one_control_in_all_blocks() {
    for source in [
        "# ![][missing]\n",
        "- ![][missing]\n",
        "> ![][missing]\n",
        "| H |\n| --- |\n| ![][missing] |\n",
    ] {
        let d = doc(source);
        let mut state = ViewportState::new(&d, ViewportConfig::default()).unwrap();
        let frame = ready(&mut state, &d);
        assert!(
            text(&frame).contains("![][missing]"),
            "{source} text={:?} blocks={:?} input={:?}",
            text(&frame),
            d.blocks(),
            mdtui_tui::render::input::DisplayInput::from_document(
                &d,
                Span::new(0, d.source().len())
            )
        );
        assert!(
            frame
                .lines
                .iter()
                .flat_map(|l| &l.cells)
                .all(|c| c.link.is_none())
        );
    }
    for source in [
        "# [![![](/inner)](/outer)](/page)\n",
        "- [![![](/inner)](/outer)](/page)\n",
        "> [![![](/inner)](/outer)](/page)\n",
        "| H |\n| --- |\n| [![![](/inner)](/outer)](/page) |\n",
    ] {
        let d = doc(source);
        let mut state = ViewportState::new(&d, ViewportConfig::default()).unwrap();
        let frame = ready(&mut state, &d);
        assert_eq!(text(&frame).matches("[image]").count(), 1, "{source}");
        let linked: Vec<_> = frame
            .lines
            .iter()
            .flat_map(|l| &l.cells)
            .filter(|c| c.link.is_some())
            .collect();
        assert_eq!(linked.len(), 7);
        assert!(
            linked
                .iter()
                .all(|c| c.link.and_then(|id| d.link_destination(id)) == Some("/page"))
        );
    }

    for source in [
        "# [![alt][missing]](/page)\n",
        "- [![alt][missing]](/page)\n",
        "> [![alt][missing]](/page)\n",
        "| H |\n| --- |\n| [![alt][missing]](/page) |\n",
    ] {
        let d = doc(source);
        let mut state = ViewportState::new(&d, ViewportConfig::default()).unwrap();
        let frame = ready(&mut state, &d);
        assert!(
            text(&frame).contains("![alt][missing]"),
            "{source} {:?}",
            text(&frame)
        );
        let linked: Vec<_> = frame
            .lines
            .iter()
            .flat_map(|l| &l.cells)
            .filter_map(|c| c.link.and_then(|id| d.link_destination(id)))
            .collect();
        assert!(!linked.is_empty());
        assert!(linked.iter().all(|&target| target == "/page"));
    }
}

#[test]
fn failed_code_release_updates_real_cells_entities_styles_images_and_click_mapping() {
    use mdtui_tui::screen::StyleRole;
    let source = "`**bold** &amp; ![![alt](/inner)](/outer)\n";
    let mut d = doc(source);
    let mut state = ViewportState::new(&d, ViewportConfig::default()).unwrap();
    let before = ready(&mut state, &d);
    assert!(text(&before).contains("&amp;"));
    assert!(
        before
            .lines
            .iter()
            .flat_map(|l| &l.cells)
            .all(|c| c.link.is_none())
    );
    let ticket = state.heights().ticket(TileId(0)).unwrap();
    let update = d.finish();
    state.apply_update(&d, &update).unwrap();
    assert!(!before.matches(&d));
    assert_eq!(
        state.heights_mut().measure(ticket, 1),
        Err(mdtui3::LayoutError::StaleMeasurement)
    );
    let after = ready(&mut state, &d);
    assert_eq!(text(&after), "`bold & alt");
    let cells = &after.lines[0].cells;
    assert_eq!(cells[0].source, Some(Span::new(0, 1)));
    assert!(!cells[0].style.contains(StyleRole::CODE));
    assert!(
        cells[1..5]
            .iter()
            .all(|c| c.style.contains(StyleRole::STRONG))
    );
    let amp = cells.iter().find(|c| c.text == "&").unwrap();
    assert_eq!(amp.source.map(|s| d.text(s).unwrap()), Some("&amp;"));
    let image: Vec<_> = cells.iter().filter(|c| c.link.is_some()).collect();
    assert_eq!(image.len(), 3);
    assert!(
        image
            .iter()
            .all(|c| c.link.and_then(|id| d.link_destination(id)) == Some("/outer"))
    );
    assert_eq!(
        image[0].source,
        Some(Span::new(
            source.find("alt").unwrap(),
            source.find("alt").unwrap() + 1
        ))
    );
    let ansi =
        mdtui_tui::static_render::render_once_ansi(&d, 80, 24, mdtui_tui::cli::LinkMode::Default)
            .unwrap();
    let mut terminal = vt100::Parser::new(
        std::num::NonZeroU16::new(24).unwrap(),
        std::num::NonZeroU16::new(80).unwrap(),
        0,
    );
    terminal.process(&ansi);
    assert_eq!(terminal.screen().contents().trim_end(), "`bold & alt");
}

#[test]
fn million_byte_code_failure_replay_does_not_turn_visible_image_layout_into_a_prefix_scan() {
    let source = format!("`{} &amp; ![](/img)\n", "word ".repeat(200_000));
    let mut d = doc(&source);
    assert_eq!(d.work().code_fallback_bytes, 0);
    d.finish();
    assert!(d.work().code_fallback_bytes > 900_000);
    assert!(d.work().code_fallback_bytes <= 2 * source.len() as u64);
    let at = source.find("![]").unwrap();
    let top = d.tile_at(at).unwrap();
    let mut first = FirstFrame::new(
        &d,
        FirstFrameConfig {
            top_tile: top,
            tile_limit: 8,
            width: 80,
            row_limit: 100,
        },
    )
    .unwrap();
    for _ in 0..1000 {
        let mut budget = Budget::new(mdtui_tui::stream::FRAME_WORK);
        let status = first.advance(&d, &mut budget).unwrap().status;
        assert!(budget.used().bytes <= mdtui_tui::stream::FRAME_WORK.bytes);
        if matches!(
            status,
            FirstFrameStatus::Covered | FirstFrameStatus::EndOfDocument
        ) {
            break;
        }
    }
    let text: String = first
        .lines()
        .iter()
        .flat_map(|l| &l.cells)
        .map(|c| c.text.as_str())
        .collect();
    assert!(text.contains("[image]"));
    let linked: Vec<_> = first
        .lines()
        .iter()
        .flat_map(|l| &l.cells)
        .filter(|c| c.link.is_some())
        .collect();
    assert_eq!(linked.len(), 7);
    assert!(
        linked
            .iter()
            .all(|c| c.link.and_then(|id| d.link_destination(id)) == Some("/img"))
    );
    assert!(first.scanner().stats().scanned_bytes <= 8 * mdtui3::TILE_BYTES + 4);
    assert_eq!(first.sink_stats().unwrap().preparation_source_bytes, 0);
    println!(
        "code-release source={} replay_bytes={} scanner={:?} layout={:?}",
        source.len(),
        d.work().code_fallback_bytes,
        first.scanner().stats(),
        first.sink_stats().unwrap()
    );
}

#[test]
fn conditional_styles_share_tui_memo_retirement_and_far_prefix_cache_invalidation() {
    let mut d = doc(&format!("*{} [x* z]\n\n", "a".repeat(20_000)));
    let links = SharedLinks::new(&d);
    let config = FirstFrameConfig {
        top_tile: TileId(0),
        tile_limit: 1,
        width: 80,
        row_limit: 4,
    };
    let mut first = FirstFrame::new(&d, config).unwrap();
    first.set_links(links.clone());
    let limit = WorkLimit {
        queries: 1,
        objects: 1,
        bytes: 6,
        tiles: 1,
        recovery: 1,
        dependencies: 1,
    };
    let before = links.stats();
    first
        .advance(&d, &mut Budget::new(WorkLimit::default()))
        .unwrap();
    assert_eq!(links.stats(), before);
    for _ in 0..100_000 {
        let mut b = Budget::new(limit);
        let status = first.advance(&d, &mut b).unwrap().status;
        assert!(b.used().queries <= 1 && b.used().objects <= 1 && b.used().bytes <= 6);
        if matches!(
            status,
            FirstFrameStatus::Covered | FirstFrameStatus::EndOfDocument
        ) {
            break;
        }
    }
    assert_eq!(links.stats().styles.events, 4);
    let mut screen = Screen::new(4, 80, ContentArea::new(0, 80)).unwrap();
    first.write_to_screen(&mut screen, 0).unwrap();
    assert_eq!(screen.cell(0, 0).unwrap().text(), "a");
    drop(first);
    let mut state = ViewportState::new(
        &d,
        ViewportConfig {
            rows: 4,
            ..ViewportConfig::default()
        },
    )
    .unwrap();
    let old = ready(&mut state, &d);
    assert!(text(&old).starts_with('a'));
    let update = d.append("[x* z]: /u\n").unwrap();
    state.apply_update(&d, &update).unwrap();
    assert!(!old.matches(&d));
    let new = ready(&mut state, &d);
    assert!(text(&new).starts_with("*a"));
    let before = links.stats();
    assert!(
        !links
            .prepare(&d, &mut Budget::new(WorkLimit::default()))
            .unwrap()
    );
    assert_eq!(links.stats(), before);
    let mut paid = 0;
    loop {
        let mut b = Budget::new(limit);
        let done = links.prepare(&d, &mut b).unwrap();
        paid += b.used().objects;
        if done {
            break;
        }
    }
    assert_eq!(paid, before.memo_entries + before.presentation_entries);
    assert_eq!(links.stats().memo_entries, 0);
    // The official ANSI fixtures remain immutable; this independent conditional
    // example exercises the actual terminal output and decoded cell grid.
    for (source, expected) in [
        ("*foo [bar* baz]\n", "foo [bar baz]"),
        ("*foo [bar* baz]\n\n[bar* baz]: /u\n", "*foo bar* baz"),
    ] {
        let d = doc(source);
        let ansi = mdtui_tui::static_render::render_once_ansi(
            &d,
            80,
            8,
            mdtui_tui::cli::LinkMode::Default,
        )
        .unwrap();
        let mut vt = vt100::Parser::new(
            std::num::NonZeroU16::new(8).unwrap(),
            std::num::NonZeroU16::new(80).unwrap(),
            0,
        );
        vt.process(&ansi);
        assert_eq!(vt.screen().contents().trim_end(), expected);
    }
}

#[test]
fn confirmed_table_displays_withdrawn_footnote_spelling_without_stalling() {
    let d = doc(&format!(
        "$old\nend$ [^n] {}|b\n-|-\nx|y\n\n[^n]: note\n",
        "x".repeat(1100)
    ));
    assert_eq!(d.tables().len(), 1);
    assert!(
        d.footnote_reference_state(mdtui3::FootnoteReferenceId(0))
            .is_none()
    );
    let mut first = FirstFrame::new(
        &d,
        FirstFrameConfig {
            top_tile: TileId(0),
            tile_limit: 8,
            width: 80,
            row_limit: 12,
        },
    )
    .unwrap();
    let mut done = false;
    for _ in 0..1000 {
        let result = first
            .advance(&d, &mut Budget::new(mdtui_tui::stream::FRAME_WORK))
            .unwrap();
        if matches!(
            result.status,
            FirstFrameStatus::Covered | FirstFrameStatus::EndOfDocument
        ) {
            done = true;
            break;
        }
    }
    assert!(
        done,
        "selected table scanner must skip inactive raw arena entries"
    );
    let mut screen = Screen::new(12, 80, ContentArea::new(0, 80)).unwrap();
    first.write_to_screen(&mut screen, 0).unwrap();
    let line = (0..80)
        .map(|x| screen.cell(4, x).unwrap().text())
        .collect::<String>();
    let all = (0..12)
        .flat_map(|y| (0..80).map(move |x| (y, x)))
        .map(|(y, x)| screen.cell(y, x).unwrap().text())
        .collect::<String>();
    assert!(all.contains("[^n]"), "{line} / {all}");
}
