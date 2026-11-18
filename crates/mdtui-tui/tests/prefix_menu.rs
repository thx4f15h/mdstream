use crossterm::event::{KeyCode, KeyModifiers};
use mdtui_tui::{
    cli::LinkMode,
    render::frame::{FrameGeometry, FrameRuntime},
};
use mdtui3::{Budget, FoldId, WorkLimit};
fn draw(r: &mut FrameRuntime, p: &mut vt100::Parser) -> String {
    let mut bytes = Vec::new();
    for _ in 0..10000 {
        let commit = r
            .frame_to(&mut bytes, &mut Budget::new(WorkLimit::unlimited()))
            .unwrap();
        if commit.committed && !r.fold_pending() {
            p.process(&bytes);
            return p.screen().contents();
        }
    }
    panic!("prefix UI did not settle")
}
fn action(r: &mut FrameRuntime, p: &mut vt100::Parser, page: usize, label: &str) -> String {
    r.open_fold_menu(false);
    draw(r, p);
    for _ in 0..page {
        assert!(r.fold_menu_key(KeyCode::Right));
    }
    assert!(r.fold_menu_key(KeyCode::Home));
    let index = match (page, label) {
        (2, "paragraph: 0 body lines") => 10,
        (2, "paragraph: 1 logical line") => 11,
        (2, "paragraph: 1 display row") => 13,
        (0, "bounded summary") => 6,
        _ => panic!("missing independent menu index {page} {label}"),
    };
    for _ in 0..index {
        assert!(r.fold_menu_key(KeyCode::Down));
    }
    draw(r, p);
    assert!(r.fold_menu_key(KeyCode::Enter));
    draw(r, p)
}
fn runtime(source: &str, width: u16) -> (FrameRuntime, vt100::Parser) {
    (
        FrameRuntime::from_finished_source(
            source,
            FrameGeometry {
                rows: 24,
                cols: 80,
                content_width: width,
            },
            LinkMode::Default,
        )
        .unwrap(),
        vt100::Parser::new(24.try_into().unwrap(), 80.try_into().unwrap(), 0),
    )
}
#[test]
fn real_menu_selects_body_rows_and_resize_reuses_the_same_view_and_source_id() {
    let (mut r, mut p) = runtime("abcdefghijklmnopqrstuvwxyz\n", 4);
    draw(&mut r, &mut p);
    let screen = action(&mut r, &mut p, 2, "paragraph: 1 display row");
    let c = r
        .viewport()
        .folds()
        .source_fold_at(r.document(), 0)
        .unwrap()
        .unwrap();
    assert_eq!(c.hidden.start, 4, "{screen}");
    assert!(r.viewport().folds().is_collapsed(c.id));
    assert!(screen.starts_with("abcd"), "{screen}");
    assert!(!screen.contains("efgh"), "{screen}");
    assert!(r.fold_menu_key(KeyCode::Esc));
    draw(&mut r, &mut p);
    r.set_content_width(8).unwrap();
    let screen = draw(&mut r, &mut p);
    let now = r
        .viewport()
        .folds()
        .source_fold_at(r.document(), 0)
        .unwrap()
        .unwrap();
    assert_eq!(now.id, c.id);
    assert_eq!(now.hidden.start, 8, "{screen}");
    assert!(screen.starts_with("abcdefgh"), "{screen}");
    assert!(!screen.contains("ijkl"), "{screen}");
    assert!(r.enter_object_focus(true));
    r.open_fold_menu(false);
    draw(&mut r, &mut p);
    for _ in 0..4 {
        r.fold_menu_key(KeyCode::Down);
    }
    r.fold_menu_key(KeyCode::Enter);
    let preview = draw(&mut r, &mut p);
    assert!(preview.contains("ijkl"), "{preview}");
    assert_eq!(r.viewport().folds().preview(), Some(c.id));
    assert!(r.fold_menu_key(KeyCode::Esc));
    draw(&mut r, &mut p);
    r.reading_key(KeyCode::Esc, KeyModifiers::NONE, false)
        .unwrap();
    let restored = draw(&mut r, &mut p);
    assert!(!restored.contains("ijkl"), "{restored}");
    assert!(r.viewport().folds().is_collapsed(c.id));
}
#[test]
fn logical_and_zero_prefix_commands_are_actual_body_visibility() {
    let (mut r, mut p) = runtime("ab\r\ncd\nef\n", 4);
    draw(&mut r, &mut p);
    let text = action(&mut r, &mut p, 2, "paragraph: 1 logical line");
    assert_eq!(
        r.viewport()
            .folds()
            .source_fold_at(r.document(), 0)
            .unwrap()
            .unwrap()
            .hidden
            .start,
        4,
        "{text}"
    );
    r.fold_menu_key(KeyCode::Esc);
    draw(&mut r, &mut p);
    let text = action(&mut r, &mut p, 2, "paragraph: 0 body lines");
    assert_eq!(
        r.viewport()
            .folds()
            .source_fold_at(r.document(), 0)
            .unwrap()
            .unwrap()
            .hidden
            .start,
        0,
        "{text}"
    );
    assert!(!text.starts_with("ab"), "{text}");
    assert!(
        r.fold_hits()
            .iter()
            .any(|h| matches!(h.id, FoldId::Source(_))),
        "{text}"
    );
}
#[test]
fn explicit_media_details_show_existing_fields_without_hidden_prose() {
    let (mut r, mut p) = runtime(
        "![cat &amp; dog](https://example.test/p.png) SECRET_PROSE\n",
        80,
    );
    draw(&mut r, &mut p);
    action(&mut r, &mut p, 2, "paragraph: 0 body lines");
    r.fold_menu_key(KeyCode::Esc);
    draw(&mut r, &mut p);
    assert!(r.enter_object_focus(true));
    let text = action(&mut r, &mut p, 0, "bounded summary");
    assert!(
        text.contains("image #0 alt=cat & dog")
            && text.contains("target=https://example.test/p.png"),
        "{text}"
    );
    assert!(!text.contains("SECRET_PROSE"), "{text}");
}
fn append(r: &mut FrameRuntime, bytes: &[u8]) {
    for _ in 0..10000 {
        match r.append_bytes(bytes) {
            Ok(update) => {
                if let Some(update) = update {
                    r.apply_update(&update).unwrap();
                }
                return;
            }
            Err(mdtui_tui::render::frame::FrameError::InitializationPending) => {
                r.initialize(&mut Budget::new(WorkLimit::unlimited()))
                    .unwrap();
            }
            Err(e) => panic!("append failed: {e}"),
        }
    }
    panic!("initialization did not finish")
}
#[test]
fn real_default_display_prefix_follows_append_without_requerying_the_old_prefix() {
    let mut r = FrameRuntime::new(
        FrameGeometry {
            rows: 24,
            cols: 80,
            content_width: 20,
        },
        LinkMode::Default,
    )
    .unwrap();
    let mut p = vt100::Parser::new(24.try_into().unwrap(), 80.try_into().unwrap(), 0);
    draw(&mut r, &mut p);
    r.open_fold_menu(false);
    draw(&mut r, &mut p);
    for _ in 0..5 {
        r.fold_menu_key(KeyCode::Right);
    }
    for _ in 0..19 {
        r.fold_menu_key(KeyCode::Down);
    }
    r.fold_menu_key(KeyCode::Enter);
    draw(&mut r, &mut p);
    r.fold_menu_key(KeyCode::Esc);
    draw(&mut r, &mut p);
    append(&mut r, format!("{}\n", "a".repeat(5000)).as_bytes());
    draw(&mut r, &mut p);
    let c = r
        .viewport()
        .folds()
        .source_fold_at(r.document(), 0)
        .unwrap()
        .unwrap();
    assert_eq!(c.hidden.start, 20);
    let starts = r.prefix_query_starts();
    for _ in 0..20 {
        append(&mut r, b"tail\n");
        draw(&mut r, &mut p);
        let current = r
            .viewport()
            .folds()
            .source_fold_at(r.document(), 0)
            .unwrap()
            .unwrap();
        assert_eq!(current.id, c.id);
        assert_eq!(current.hidden.start, 20);
    }
    assert_eq!(
        r.prefix_query_starts(),
        starts,
        "ordinary append replayed an established prefix"
    );
}
#[test]
fn logical_open_single_line_keeps_its_paid_cursor_until_a_lf_arrives() {
    let mut r = FrameRuntime::new(
        FrameGeometry {
            rows: 24,
            cols: 80,
            content_width: 20,
        },
        LinkMode::Default,
    )
    .unwrap();
    let mut p = vt100::Parser::new(24.try_into().unwrap(), 80.try_into().unwrap(), 0);
    draw(&mut r, &mut p);
    append(
        &mut r,
        format!("first long line {}", "x".repeat(5000)).as_bytes(),
    );
    draw(&mut r, &mut p);
    action(&mut r, &mut p, 2, "paragraph: 1 logical line");
    r.fold_menu_key(KeyCode::Esc);
    draw(&mut r, &mut p);
    let starts = r.prefix_query_starts();
    assert_eq!(starts, 1, "test must exercise a classified open paragraph");
    for _ in 0..20 {
        append(&mut r, b"extra ");
        draw(&mut r, &mut p);
    }
    assert_eq!(
        r.prefix_query_starts(),
        starts,
        "logical prefix rescanned a long old line"
    );
    append(&mut r, b"\nnext hidden\n");
    draw(&mut r, &mut p);
    let cut = r.document().source().find('\n').unwrap() + 1;
    let c = r
        .viewport()
        .folds()
        .source_fold_at(r.document(), 0)
        .unwrap()
        .unwrap();
    assert_eq!(c.hidden.start, cut);
}
#[test]
fn appending_to_the_current_single_egc_does_not_duplicate_its_old_placement() {
    let mut r = FrameRuntime::new(
        FrameGeometry {
            rows: 24,
            cols: 80,
            content_width: 1,
        },
        LinkMode::Default,
    )
    .unwrap();
    let mut p = vt100::Parser::new(24.try_into().unwrap(), 80.try_into().unwrap(), 0);
    draw(&mut r, &mut p);
    append(&mut r, format!("a{}", "\u{301}".repeat(2500)).as_bytes());
    draw(&mut r, &mut p);
    action(&mut r, &mut p, 2, "paragraph: 1 display row");
    r.fold_menu_key(KeyCode::Esc);
    draw(&mut r, &mut p);
    let starts = r.prefix_query_starts();
    assert_eq!(starts, 1);
    for _ in 0..5 {
        append(&mut r, "\u{301}".as_bytes());
        draw(&mut r, &mut p);
        assert_eq!(
            r.viewport().folds().source_fold_count(),
            0,
            "a continued EGC was laid out twice"
        );
    }
    assert_eq!(r.prefix_query_starts(), starts);
}
#[test]
fn append_after_a_native_object_keeps_the_native_and_plain_cluster_barrier() {
    let mut r = FrameRuntime::new(
        FrameGeometry {
            rows: 24,
            cols: 80,
            content_width: 2,
        },
        LinkMode::Default,
    )
    .unwrap();
    let mut p = vt100::Parser::new(24.try_into().unwrap(), 80.try_into().unwrap(), 0);
    draw(&mut r, &mut p);
    append(
        &mut r,
        format!("a{}\\(x\\)", "\u{301}".repeat(2500)).as_bytes(),
    );
    draw(&mut r, &mut p);
    action(&mut r, &mut p, 2, "paragraph: 1 display row");
    r.fold_menu_key(KeyCode::Esc);
    draw(&mut r, &mut p);
    assert_eq!(
        r.document().math_object_stats().active,
        1,
        "fixture must have a confirmed native object at its append frontier"
    );
    assert_eq!(
        r.viewport().folds().source_fold_count(),
        0,
        "initial native object should fit the requested body line"
    );
    let starts = r.prefix_query_starts();
    assert_eq!(starts, 1);
    for _ in 0..5 {
        append(&mut r, "\u{301}".as_bytes());
        draw(&mut r, &mut p);
        assert_eq!(
            r.viewport().folds().source_fold_count(),
            0,
            "a native atom was reintroduced as raw Markdown"
        );
    }
    assert_eq!(r.prefix_query_starts(), starts);
}
fn clipboard(r: &mut FrameRuntime, key: char) -> String {
    r.reading_key(KeyCode::Char(key), KeyModifiers::NONE, true)
        .unwrap();
    let mut wire = Vec::new();
    for _ in 0..100_000 {
        r.advance_reading(
            &mut wire,
            &mut Budget::new(WorkLimit {
                queries: 1,
                objects: 1,
                bytes: 6,
                tiles: 1,
                recovery: 1,
                dependencies: 1,
            }),
        );
        if !r.reading().copy_pending() {
            break;
        }
    }
    assert!(!r.reading().copy_pending());
    assert!(wire.starts_with(b"\x1b]52;c;"));
    assert!(wire.ends_with(b"\x07"));
    let mut out = Vec::new();
    let mut value = 0u32;
    let mut bits = 0;
    for b in wire[7..wire.len() - 1]
        .iter()
        .copied()
        .filter(|b| *b != b'=')
    {
        let n = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/"
            .iter()
            .position(|&c| c == b)
            .unwrap() as u32;
        value = (value << 6) | n;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((value >> bits) as u8);
        }
    }
    String::from_utf8(out).unwrap()
}
#[test]
fn source_selection_visible_and_raw_copy_and_reveal_share_the_prefix_fold_view() {
    let (mut r, mut p) = runtime("abcdEFGH\r\n", 4);
    draw(&mut r, &mut p);
    action(&mut r, &mut p, 2, "paragraph: 1 display row");
    r.fold_menu_key(KeyCode::Esc);
    draw(&mut r, &mut p);
    let c = r
        .viewport()
        .folds()
        .source_fold_at(r.document(), 0)
        .unwrap()
        .unwrap();
    assert_eq!(c.hidden, mdtui3::Span::new(4, 10));
    assert_eq!(r.viewport().heights().total_rows(), 2);
    assert!(r.enter_object_focus(true));
    action(&mut r, &mut p, 0, "bounded summary");
    r.fold_menu_key(KeyCode::Esc);
    draw(&mut r, &mut p);
    r.clear_link_focus();
    r.reading_key(KeyCode::Char('v'), KeyModifiers::NONE, true)
        .unwrap();
    r.reading_key(KeyCode::Down, KeyModifiers::SHIFT, true)
        .unwrap();
    draw(&mut r, &mut p);
    assert_eq!(
        r.reading().selection.range(),
        Some(mdtui3::Span::new(0, 10))
    );
    let FoldId::Source(id) = c.id else { panic!() };
    assert_eq!(
        clipboard(&mut r, 'y'),
        format!("abcd[+ source {} 0..10]", id.index())
    );
    assert_eq!(clipboard(&mut r, 'Y'), "abcdEFGH\r\n");
    r.reading_key(KeyCode::Esc, KeyModifiers::NONE, true)
        .unwrap();
    draw(&mut r, &mut p);
    let saved = r.navigation_position().unwrap();
    r.reading_key(KeyCode::Char('/'), KeyModifiers::NONE, true)
        .unwrap();
    for ch in "EFGH".chars() {
        r.reading_key(KeyCode::Char(ch), KeyModifiers::NONE, true)
            .unwrap();
    }
    r.reading_key(KeyCode::Enter, KeyModifiers::NONE, true)
        .unwrap();
    for _ in 0..10_000 {
        r.advance_reading(&mut Vec::new(), &mut Budget::new(WorkLimit::unlimited()));
        let mut bytes = Vec::new();
        r.frame_to(&mut bytes, &mut Budget::new(WorkLimit::unlimited()))
            .unwrap();
        p.process(&bytes);
        if !r.has_pending_jump() && !r.reading().pending() {
            break;
        }
    }
    let text = draw(&mut r, &mut p);
    assert!(text.contains("EFGH"), "{text}");
    assert!(!r.viewport().folds().is_collapsed(c.id));
    assert_eq!(
        r.reading().current_match().unwrap().source,
        mdtui3::Span::new(4, 8)
    );
    r.restore_navigation_position(saved, &mut Budget::new(WorkLimit::unlimited()))
        .unwrap();
    draw(&mut r, &mut p);
    assert_eq!(r.top_source(), Some(saved.source));
}
#[test]
fn resize_through_an_empty_hidden_suffix_keeps_one_id_and_exact_height() {
    let (mut r, mut p) = runtime("abcdEFGH\r\n", 4);
    draw(&mut r, &mut p);
    action(&mut r, &mut p, 2, "paragraph: 1 display row");
    r.fold_menu_key(KeyCode::Esc);
    draw(&mut r, &mut p);
    let id = r
        .viewport()
        .folds()
        .source_fold_at(r.document(), 0)
        .unwrap()
        .unwrap()
        .id;
    for (width, cut, rows) in [(10, 10, 1), (4, 4, 2)] {
        r.set_content_width(width).unwrap();
        draw(&mut r, &mut p);
        let c = r
            .viewport()
            .folds()
            .fold_candidate(r.document(), id)
            .unwrap()
            .unwrap();
        assert_eq!(c.hidden.start, cut);
        assert_eq!(r.viewport().heights().total_rows(), rows);
        assert!(r.viewport().folds().is_collapsed(id));
        assert_eq!(r.viewport().folds().source_fold_count(), 1);
    }
}
#[test]
fn resize_validation_cannot_publish_the_old_row_cut_or_be_cancelled_into_one() {
    let (mut r, mut p) = runtime("abcdefghijklmnopqrstuvwxyz\n", 8);
    draw(&mut r, &mut p);
    action(&mut r, &mut p, 2, "paragraph: 1 display row");
    r.fold_menu_key(KeyCode::Esc);
    draw(&mut r, &mut p);
    let id = r
        .viewport()
        .folds()
        .source_fold_at(r.document(), 0)
        .unwrap()
        .unwrap()
        .id;
    r.set_content_width(4).unwrap();
    r.fold_menu_key(KeyCode::Esc);
    assert!(
        r.fold_pending(),
        "Esc must retain safety validation after width changed"
    );
    let mut committed = false;
    for _ in 0..100_000 {
        let mut bytes = Vec::new();
        let commit = r
            .frame_to(
                &mut bytes,
                &mut Budget::new(WorkLimit {
                    queries: 1,
                    objects: 1,
                    bytes: 6,
                    tiles: 1,
                    recovery: 1,
                    dependencies: 1,
                }),
            )
            .unwrap();
        if commit.committed {
            let c = r
                .viewport()
                .folds()
                .fold_candidate(r.document(), id)
                .unwrap()
                .unwrap();
            assert_eq!(
                c.hidden.start, 4,
                "an old eight-cell prefix was committed at width four"
            );
            committed = true;
            break;
        }
    }
    assert!(committed, "resumable width validation did not finish");
}

#[test]
fn handwritten_container_prefix_grid_consumes_027_spacing_and_source_rows() {
    for (source, width, first, cut, next_row, total) in [
        ("> abcdef\n> ghij\n\nnext\n", 6, "│ abcd", 6, 4, 5),
        ("- abc de fg\n\nnext\n", 6, "• abc", 5, 3, 4),
    ] {
        let (mut r, mut p) = runtime(source, width);
        draw(&mut r, &mut p);
        action(&mut r, &mut p, 2, "paragraph: 1 display row");
        r.fold_menu_key(KeyCode::Esc);
        let text = draw(&mut r, &mut p);
        eprintln!(
            "container prefix grid {text:?}; rows={}",
            r.viewport().heights().total_rows()
        );
        assert_eq!(text.lines().next(), Some(first));
        let candidate = r
            .viewport()
            .folds()
            .source_fold_at(r.document(), 0)
            .unwrap()
            .unwrap();
        assert_eq!(candidate.hidden.start, cut);
        if next_row == 4 {
            assert_eq!(text.lines().nth(3), Some(""));
        }
        assert_eq!(text.lines().nth(next_row), Some("next"));
        assert_eq!(r.viewport().heights().total_rows(), total);
    }
}

#[test]
fn frozen_end_and_follow_end_keep_the_same_streaming_prefix_choice() {
    let mut r = FrameRuntime::new(
        FrameGeometry {
            rows: 8,
            cols: 40,
            content_width: 40,
        },
        LinkMode::Default,
    )
    .unwrap();
    let mut p = vt100::Parser::new(8.try_into().unwrap(), 40.try_into().unwrap(), 0);
    draw(&mut r, &mut p);
    append(
        &mut r,
        format!("{} OLD_TAIL\n", "a".repeat(5000)).as_bytes(),
    );
    draw(&mut r, &mut p);
    action(&mut r, &mut p, 2, "paragraph: 1 display row");
    r.fold_menu_key(KeyCode::Esc);
    draw(&mut r, &mut p);
    let c = r
        .viewport()
        .folds()
        .source_fold_at(r.document(), 0)
        .unwrap()
        .unwrap();
    r.seek_end(false).unwrap();
    let at_end = draw(&mut r, &mut p);
    eprintln!(
        "frozen before append {at_end:?} top={:?} hit={:?}",
        r.top_source(),
        r.fold_hits()
    );
    append(&mut r, b"NEW_TAIL\n");
    let after_append = draw(&mut r, &mut p);
    eprintln!(
        "frozen after append {after_append:?} top={:?} hit={:?} fold={:?}",
        r.top_source(),
        r.fold_hits(),
        r.viewport().folds().fold_candidate(r.document(), c.id)
    );
    assert!(r.enter_object_focus(true));
    assert!(r.activate_focused_fold());
    let frozen = draw(&mut r, &mut p);
    assert!(!r.viewport().folds().is_collapsed(c.id));
    assert!(frozen.contains("OLD_TAIL"), "{frozen}");
    assert!(!frozen.contains("NEW_TAIL"), "{frozen}");
    r.seek_end(true).unwrap();
    let following = draw(&mut r, &mut p);
    assert!(following.contains("NEW_TAIL"), "{following}");
}
