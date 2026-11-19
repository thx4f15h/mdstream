use crossterm::event::{KeyCode, KeyModifiers};
use mdtui_tui::{
    cli::LinkMode,
    render::frame::{FrameGeometry, FrameRuntime},
};
use mdtui3::{Budget, WorkLimit};

fn ready(runtime: &mut FrameRuntime) -> Vec<String> {
    let mut wire = vec![];
    for i in 0..10000 {
        if runtime
            .frame_to(&mut wire, &mut Budget::new(WorkLimit::unlimited()))
            .unwrap()
            .committed
        {
            return (0..runtime.screen().rows())
                .map(|r| {
                    runtime
                        .screen()
                        .front_row(r)
                        .unwrap()
                        .iter()
                        .map(|c| c.text().collect_text())
                        .collect::<String>()
                        .trim_end()
                        .to_owned()
                })
                .collect();
        }
        assert!(i < 9999, "no published note frame");
    }
    unreachable!()
}
fn key(r: &mut FrameRuntime, code: KeyCode) {
    assert!(r.footnote_key(code, KeyModifiers::NONE, true).unwrap());
}

#[test]
fn cold_adoption_bootstraps_root_and_body_backlinks_from_existing_indexes() {
    let source = "a[^n]\n\n[^n]: body [^other]\n[^other]: other\n";
    let geometry = FrameGeometry {
        rows: 8,
        cols: 40,
        content_width: 40,
    };
    let mut parsed = mdtui3::Utf8Document::new();
    parsed.append(source.as_bytes()).unwrap();
    parsed.finish();
    let mut cold = FrameRuntime::from_utf8_document(parsed, geometry, LinkMode::NoOsc8).unwrap();
    let mut warm = FrameRuntime::from_finished_source(source, geometry, LinkMode::NoOsc8).unwrap();
    for runtime in [&mut cold, &mut warm] {
        let work = runtime.document().footnote_body_work();
        let mut wire = vec![];
        runtime
            .frame_to(&mut wire, &mut Budget::new(WorkLimit::default()))
            .unwrap();
        assert!(wire.is_empty());
        assert_eq!(work, runtime.document().footnote_body_work());
        ready(runtime);
        key(runtime, KeyCode::Char('f'));
        ready(runtime);
        key(runtime, KeyCode::Enter);
        ready(runtime);
        key(runtime, KeyCode::Char('b'));
        assert_eq!(ready(runtime)[1], "main @1");
        key(runtime, KeyCode::Char('f'));
        ready(runtime);
        key(runtime, KeyCode::Down);
        ready(runtime);
        key(runtime, KeyCode::Enter);
        ready(runtime);
        key(runtime, KeyCode::Char('b'));
        assert_eq!(ready(runtime)[1], "note ¹ @5");
    }
}

#[test]
fn cold_live_body_subscribes_without_replaying_source_before_late_host_binding() {
    let mut parsed = mdtui3::Utf8Document::new();
    parsed.append(b"a[^n]\n\n[^n]: [L][r]\n# root\n").unwrap();
    let mut r = FrameRuntime::from_utf8_document(
        parsed,
        FrameGeometry {
            rows: 8,
            cols: 40,
            content_width: 40,
        },
        LinkMode::NoOsc8,
    )
    .unwrap();
    let work = r.document().footnote_body_work();
    for _ in 0..1000 {
        if r.initialize(&mut Budget::new(WorkLimit {
            queries: 1,
            objects: 1,
            dependencies: 1,
            bytes: 6,
            ..Default::default()
        }))
        .unwrap()
        {
            break;
        }
    }
    assert_eq!(r.document().footnote_body_work(), work);
    ready(&mut r);
    key(&mut r, KeyCode::Char('f'));
    ready(&mut r);
    key(&mut r, KeyCode::Enter);
    assert_eq!(ready(&mut r)[0], "[L][r]");
    let update = r.append_bytes(b"\n[r]: /late\n").unwrap().unwrap();
    assert!(update.body_updates.is_empty());
    r.apply_update(&update).unwrap();
    assert_eq!(ready(&mut r)[0], "L");
    assert_eq!(r.document().footnote_body_work(), work);
}

fn clipboard(runtime: &mut FrameRuntime) -> Vec<u8> {
    let mut wire = vec![];
    for _ in 0..100000 {
        runtime
            .frame_to(
                &mut wire,
                &mut Budget::new(WorkLimit {
                    queries: 2,
                    objects: 2,
                    bytes: 6,
                    tiles: 1,
                    recovery: 1,
                    dependencies: 1,
                }),
            )
            .unwrap();
        if runtime.footnote_reading().unwrap().message() == "clipboard request sent"
            && !runtime.footnote_reading().unwrap().copy_pending()
        {
            break;
        }
    }
    let prefix = b"\x1b]52;c;";
    let at = wire
        .windows(prefix.len())
        .position(|w| w == prefix)
        .expect("no OSC52");
    let data = &wire[at + prefix.len()..];
    let end = data.iter().position(|&b| b == 7).expect("unclosed OSC52");
    let mut output = vec![];
    let mut bits = 0u32;
    let mut count = 0;
    for &byte in &data[..end] {
        if byte == b'=' {
            break;
        }
        let value = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/"
            .iter()
            .position(|&b| b == byte)
            .unwrap() as u32;
        bits = (bits << 6) | value;
        count += 6;
        if count >= 8 {
            count -= 8;
            output.push((bits >> count) as u8);
        }
    }
    output
}

#[test]
fn body_visible_and_original_copy_keep_their_own_address_and_crlf_prefix_contracts() {
    let mut r = FrameRuntime::from_finished_source(
        "> [^n]: one\r\n>     two\r\n# outside\r\n",
        FrameGeometry {
            rows: 8,
            cols: 40,
            content_width: 40,
        },
        LinkMode::NoOsc8,
    )
    .unwrap();
    ready(&mut r);
    key(&mut r, KeyCode::Char('f'));
    ready(&mut r);
    key(&mut r, KeyCode::Enter);
    assert_eq!(ready(&mut r)[0], "one two");
    key(&mut r, KeyCode::Char('v'));
    ready(&mut r);
    key(&mut r, KeyCode::Char(' '));
    for _ in 0..6 {
        assert!(
            r.footnote_key(KeyCode::Right, KeyModifiers::SHIFT, true)
                .unwrap()
        );
        ready(&mut r);
    }
    key(&mut r, KeyCode::Char('Y'));
    assert_eq!(clipboard(&mut r), b"one\r\n>     two");
    key(&mut r, KeyCode::Char('y'));
    assert_eq!(clipboard(&mut r), b"one two");
}

#[test]
fn switching_notes_closes_a_partly_sent_visible_or_original_clipboard_before_new_paint() {
    struct ShortWriter<'a> {
        wire: &'a mut Vec<u8>,
        remaining: usize,
    }
    impl std::io::Write for ShortWriter<'_> {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if self.remaining == 0 {
                return Err(std::io::ErrorKind::WouldBlock.into());
            }
            let n = bytes.len().min(self.remaining);
            self.wire.extend_from_slice(&bytes[..n]);
            self.remaining -= n;
            Ok(n)
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    for copy in ['y', 'Y'] {
        let mut r = FrameRuntime::from_finished_source(
            "[^a]: one two three four five six\r\n[^b]: other note\r\n",
            FrameGeometry {
                rows: 8,
                cols: 40,
                content_width: 40,
            },
            LinkMode::NoOsc8,
        )
        .unwrap();
        ready(&mut r);
        key(&mut r, KeyCode::Char('f'));
        ready(&mut r);
        key(&mut r, KeyCode::Enter);
        assert_eq!(ready(&mut r)[0], "one two three four five six");
        key(&mut r, KeyCode::Char('v'));
        ready(&mut r);
        key(&mut r, KeyCode::Char(' '));
        for _ in 0..20 {
            key(&mut r, KeyCode::Right);
            ready(&mut r);
        }
        key(&mut r, KeyCode::Char(copy));
        let mut wire = vec![];
        let prefix = b"\x1b]52;c;";
        let micro = WorkLimit {
            queries: 2,
            objects: 2,
            bytes: 6,
            tiles: 1,
            recovery: 1,
            dependencies: 1,
        };
        let mut start = None;
        for _ in 0..10000 {
            r.frame_to(
                &mut ShortWriter {
                    wire: &mut wire,
                    remaining: 1,
                },
                &mut Budget::new(micro),
            )
            .unwrap();
            if let Some(at) = wire.windows(prefix.len()).position(|w| w == prefix) {
                start = Some(at + prefix.len());
                break;
            }
        }
        let start = start.expect("clipboard did not start");
        assert!(!wire[start..].contains(&7), "copy already completed");
        key(&mut r, KeyCode::Char('f'));
        for _ in 0..10000 {
            r.frame_to(&mut wire, &mut Budget::new(micro)).unwrap();
            if wire[start..].windows(3).any(|w| w == b"\x18\x1b\\") {
                break;
            }
        }
        let end = wire[start..]
            .windows(3)
            .position(|w| w == b"\x18\x1b\\")
            .expect("cancelled clipboard stayed open");
        assert!(
            wire[start..start + end]
                .iter()
                .all(|b| { b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'=') })
        );
        ready(&mut r);
        key(&mut r, KeyCode::Down);
        ready(&mut r);
        key(&mut r, KeyCode::Enter);
        assert_eq!(ready(&mut r)[0], "other note");
        let stats = r.footnote_stats().unwrap();
        assert_eq!(stats.note_creations - stats.note_retirements, 1);
    }
}

#[test]
fn body_global_link_wire_metadata_uses_the_original_host_definition() {
    let mut r = FrameRuntime::from_finished_source(
        "[^n]: [L][r]\n# outside\n\n[r]: https://example.test/note\n",
        FrameGeometry {
            rows: 8,
            cols: 40,
            content_width: 40,
        },
        LinkMode::Default,
    )
    .unwrap();
    ready(&mut r);
    key(&mut r, KeyCode::Char('f'));
    ready(&mut r);
    key(&mut r, KeyCode::Enter);
    let mut wire = vec![];
    for _ in 0..10000 {
        if r.frame_to(&mut wire, &mut Budget::new(WorkLimit::unlimited()))
            .unwrap()
            .committed
        {
            break;
        }
    }
    assert!(
        wire.windows(b"\x1b]8;;https://example.test/note\x1b\\".len())
            .any(|w| w == b"\x1b]8;;https://example.test/note\x1b\\")
    );
}

#[test]
fn cycles_are_finite_user_navigation_and_back_returns_to_exact_body_context() {
    let mut r = FrameRuntime::from_finished_source(
        "See [^a].\n\n[^a]: A [^b]\n[^b]: B [^a]\n",
        FrameGeometry {
            rows: 8,
            cols: 40,
            content_width: 40,
        },
        LinkMode::NoOsc8,
    )
    .unwrap();
    ready(&mut r);
    assert!(r.enter_object_focus(true));
    key(&mut r, KeyCode::Enter);
    assert_eq!(ready(&mut r)[0], "A ²");
    let a = r.footnote_surface().unwrap();
    assert!(r.enter_object_focus(true));
    key(&mut r, KeyCode::Enter);
    assert_eq!(ready(&mut r)[0], "B ¹");
    let b = r.footnote_surface().unwrap();
    assert_ne!(a, b);
    assert!(r.enter_object_focus(true));
    key(&mut r, KeyCode::Enter);
    assert_eq!(ready(&mut r)[0], "A ²");
    key(&mut r, KeyCode::Char('b'));
    assert_eq!(ready(&mut r)[0], "B ¹");
    assert_eq!(r.footnote_surface(), Some(b));
}

#[test]
fn chooser_opens_real_markdown_body_and_returns_to_root() {
    let mut r = FrameRuntime::from_finished_source(
        "See [^n].\n\n[^n]: **Note** body.\n\n    - first\n    - second\n# root tail\n",
        FrameGeometry {
            rows: 12,
            cols: 40,
            content_width: 40,
        },
        LinkMode::NoOsc8,
    )
    .unwrap();
    assert_eq!(ready(&mut r)[0], "See ¹.");
    key(&mut r, KeyCode::Char('f'));
    let chooser = ready(&mut r);
    assert_eq!(chooser[0], "Footnotes");
    assert_eq!(chooser[1], "¹ n [Defined]");
    key(&mut r, KeyCode::Enter);
    let note = ready(&mut r);
    assert_eq!(note[0], "Note body.");
    assert!(note.iter().any(|s| s == "• first"));
    let source = r.footnote_source_at(0, 0).unwrap();
    assert_eq!(source.body, r.footnote_surface().unwrap());
    assert!(source.span.end < r.document().source().find("# root tail").unwrap());
    key(&mut r, KeyCode::Char('b'));
    assert_eq!(ready(&mut r)[0], "Footnote sources");
    key(&mut r, KeyCode::Enter);
    assert_eq!(ready(&mut r)[0], "See ¹.");
    assert!(r.footnote_surface().is_none());
}

#[test]
fn live_note_rebinds_late_host_definition_without_reparsing_or_body_delta() {
    let mut r = FrameRuntime::new(
        FrameGeometry {
            rows: 8,
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
    let update = r
        .append_bytes(b"See [^n].\n\n[^n]: [L][r]\n# outside\n")
        .unwrap()
        .unwrap();
    r.apply_update(&update).unwrap();
    ready(&mut r);
    key(&mut r, KeyCode::Char('f'));
    ready(&mut r);
    key(&mut r, KeyCode::Enter);
    assert_eq!(ready(&mut r)[0], "[L][r]");
    let work = r.document().footnote_body_work();
    let update = r.append_bytes(b"\n[r]: /late\n").unwrap().unwrap();
    assert!(update.body_updates.is_empty());
    assert_eq!(r.document().footnote_body_work(), work);
    r.apply_update(&update).unwrap();
    assert_eq!(ready(&mut r)[0], "L");
    assert_eq!(r.document().footnote_body_work(), work);
}

#[test]
fn the_note_surface_keeps_a_complete_giant_cluster_and_number_for_copy_and_resize() {
    let cluster = format!("a{}", "\u{301}".repeat(5000));
    let source = format!("See [^n].\n\n[^n]: {cluster} [^missing]\n");
    let mut r = FrameRuntime::from_finished_source(
        &source,
        FrameGeometry {
            rows: 8,
            cols: 30,
            content_width: 30,
        },
        LinkMode::NoOsc8,
    )
    .unwrap();
    ready(&mut r);
    key(&mut r, KeyCode::Char('f'));
    ready(&mut r);
    key(&mut r, KeyCode::Enter);
    assert_eq!(ready(&mut r)[0], format!("{cluster} ²"));
    key(&mut r, KeyCode::Char('v'));
    ready(&mut r);
    key(&mut r, KeyCode::Char('y'));
    assert_eq!(clipboard(&mut r), cluster.as_bytes());
    r.resize(6, 1).unwrap();
    assert_eq!(ready(&mut r)[0], cluster);
    key(&mut r, KeyCode::End);
    ready(&mut r);
    assert!(r.footnote_surface().is_some());
}

#[test]
fn body_switch_retires_the_actual_math_and_shape_owners_under_micro_budget() {
    let mut r = FrameRuntime::from_finished_source(
        "See [^a].\n\n[^a]: A $\\frac{1}{2}$ [^b]\n[^b]: B $x^2$ [^a]\n",
        FrameGeometry {
            rows: 8,
            cols: 40,
            content_width: 40,
        },
        LinkMode::NoOsc8,
    )
    .unwrap();
    ready(&mut r);
    assert!(r.enter_object_focus(true));
    key(&mut r, KeyCode::Enter);
    ready(&mut r);
    let before = r.footnote_stats().unwrap();
    assert_eq!(before.note_creations, 1);
    assert!(before.math_owners > 0);
    assert!(r.enter_object_focus(true));
    key(&mut r, KeyCode::Enter);
    let mut wire = vec![];
    r.frame_to(&mut wire, &mut Budget::new(WorkLimit::default()))
        .unwrap();
    let zero = r.footnote_stats().unwrap();
    assert_eq!(zero.retirement, before.retirement);
    assert_eq!(zero.note_creations, 1);
    let mut committed = false;
    for _ in 0..100000 {
        let commit = r
            .frame_to(
                &mut wire,
                &mut Budget::new(WorkLimit {
                    queries: 2,
                    objects: 2,
                    dependencies: 1,
                    bytes: 6,
                    tiles: 1,
                    recovery: 1,
                }),
            )
            .unwrap();
        let stats = r.footnote_stats().unwrap();
        assert!(stats.note_creations - stats.note_retirements <= 1);
        if commit.committed {
            committed = true;
            break;
        }
    }
    assert!(
        committed,
        "micro-budget switch did not converge: {:?}",
        r.footnote_stats().unwrap()
    );
    let after = r.footnote_stats().unwrap();
    assert_eq!(after.note_creations, 2);
    assert_eq!(after.note_retirements, 1);
    assert!(after.retirement.objects > 10);
    assert!(after.math_owners > 0);
    assert_ne!(after.active_owner, before.active_owner);
    println!(
        "note retire before={before:?} after={after:?} wire={}",
        wire.len()
    );
}

#[test]
fn a_note_keeps_its_fold_choice_after_another_note_retires_its_renderer() {
    let mut r = FrameRuntime::from_finished_source(
        "See [^a].\n\n[^a]: # H\n\n    secret\n[^b]: B\n",
        FrameGeometry {
            rows: 8,
            cols: 40,
            content_width: 40,
        },
        LinkMode::NoOsc8,
    )
    .unwrap();
    ready(&mut r);
    key(&mut r, KeyCode::Char('f'));
    ready(&mut r);
    key(&mut r, KeyCode::Enter);
    assert!(ready(&mut r).iter().any(|s| s == "secret"));
    key(&mut r, KeyCode::Char('z'));
    ready(&mut r);
    key(&mut r, KeyCode::Enter);
    ready(&mut r);
    key(&mut r, KeyCode::Esc);
    assert!(!ready(&mut r).iter().any(|s| s == "secret"));
    key(&mut r, KeyCode::Char('f'));
    ready(&mut r);
    key(&mut r, KeyCode::Down);
    ready(&mut r);
    key(&mut r, KeyCode::Enter);
    assert_eq!(ready(&mut r)[0], "B");
    key(&mut r, KeyCode::Char('f'));
    ready(&mut r);
    key(&mut r, KeyCode::Home);
    ready(&mut r);
    key(&mut r, KeyCode::Enter);
    assert!(
        !ready(&mut r).iter().any(|s| s == "secret"),
        "reopening lost the actual fold intent"
    );
    let stats = r.footnote_stats().unwrap();
    assert_eq!(stats.note_creations - stats.note_retirements, 1);
}

#[test]
fn note_preview_and_default_wishes_survive_reopening_and_typed_queries_reject_other_owners() {
    let source = "See [^a].\n\n[^a]: # H\n\n    secret\n[^b]: # B\n\n    other\n";
    let mut r = FrameRuntime::from_finished_source(
        source,
        FrameGeometry {
            rows: 8,
            cols: 40,
            content_width: 40,
        },
        LinkMode::NoOsc8,
    )
    .unwrap();
    ready(&mut r);
    key(&mut r, KeyCode::Char('f'));
    ready(&mut r);
    key(&mut r, KeyCode::Enter);
    ready(&mut r);
    let a = r.footnote_surface().unwrap();
    let local = mdtui3::FoldId::Heading(mdtui3::HeadingId(0));
    let id = r
        .document()
        .footnote_body(a.symbol())
        .unwrap()
        .qualify(local);
    key(&mut r, KeyCode::Char('z'));
    ready(&mut r);
    key(&mut r, KeyCode::Enter);
    ready(&mut r);
    for _ in 0..4 {
        key(&mut r, KeyCode::Down);
        ready(&mut r);
    }
    assert!(!ready(&mut r)[7].contains("end preview"));
    key(&mut r, KeyCode::Enter);
    ready(&mut r);
    for _ in 0..100 {
        if !r.fold_pending() {
            break;
        }
        ready(&mut r);
    }
    key(&mut r, KeyCode::Esc);
    ready(&mut r);
    let state = r
        .footnote_fold_state(
            id,
            r.document().revision(),
            &mut Budget::new(WorkLimit::unlimited()),
        )
        .unwrap()
        .unwrap();
    assert!(state.collapsed);
    assert_eq!(state.preview, Some(id));
    key(&mut r, KeyCode::Char('z'));
    ready(&mut r);
    for _ in 0..5 {
        key(&mut r, KeyCode::Right);
        ready(&mut r);
    }
    key(&mut r, KeyCode::Enter);
    ready(&mut r);
    key(&mut r, KeyCode::Esc);
    ready(&mut r);
    assert!(
        r.footnote_fold_state(
            id,
            r.document().revision(),
            &mut Budget::new(WorkLimit::unlimited())
        )
        .unwrap()
        .unwrap()
        .defaults
        .heading_levels[0]
    );
    key(&mut r, KeyCode::Char('f'));
    ready(&mut r);
    key(&mut r, KeyCode::Down);
    ready(&mut r);
    key(&mut r, KeyCode::Enter);
    ready(&mut r);
    let mut zero = Budget::new(WorkLimit::default());
    assert!(
        r.footnote_fold_state(id, r.document().revision(), &mut zero)
            .is_err()
    );
    assert_eq!(zero.used(), WorkLimit::default());
    key(&mut r, KeyCode::Char('f'));
    ready(&mut r);
    key(&mut r, KeyCode::Home);
    ready(&mut r);
    key(&mut r, KeyCode::Enter);
    ready(&mut r);
    let state = r
        .footnote_fold_state(
            id,
            r.document().revision(),
            &mut Budget::new(WorkLimit::unlimited()),
        )
        .unwrap()
        .unwrap();
    assert!(state.collapsed);
    assert_eq!(state.preview, Some(id));
    assert!(state.defaults.heading_levels[0]);
    assert_eq!(state.source.body, a);
    let other = FrameRuntime::from_finished_source(
        source,
        FrameGeometry {
            rows: 8,
            cols: 40,
            content_width: 40,
        },
        LinkMode::NoOsc8,
    )
    .unwrap();
    assert!(
        other
            .footnote_fold_state(id, other.document().revision(), &mut zero)
            .is_err()
    );
    assert_eq!(zero.used(), WorkLimit::default());
}

#[test]
fn search_and_copy_use_complete_display_numbers_in_the_addressed_note() {
    let mut r = FrameRuntime::from_finished_source(
        "See [^a].\n\n[^a]: x [^missing] y\n",
        FrameGeometry {
            rows: 8,
            cols: 40,
            content_width: 40,
        },
        LinkMode::NoOsc8,
    )
    .unwrap();
    ready(&mut r);
    key(&mut r, KeyCode::Char('f'));
    ready(&mut r);
    key(&mut r, KeyCode::Enter);
    assert_eq!(ready(&mut r)[0], "x ² y");
    key(&mut r, KeyCode::Char('/'));
    key(&mut r, KeyCode::Char('²'));
    key(&mut r, KeyCode::Enter);
    ready(&mut r);
    let hit = r.footnote_reading().unwrap().current_match().unwrap();
    assert_eq!(hit.source.body, r.footnote_surface().unwrap());
    let body = r.document().footnote_body(hit.body.symbol()).unwrap();
    let mut text = body.source_cursor(hit.source).unwrap();
    let mut actual = String::new();
    while let Some(chunk) = text
        .advance(&mut Budget::new(WorkLimit::unlimited()))
        .unwrap()
    {
        actual.push_str(chunk.text);
    }
    assert_eq!(actual, "[^missing]");
    key(&mut r, KeyCode::Esc);
    ready(&mut r);
    key(&mut r, KeyCode::Char('v'));
    ready(&mut r);
    key(&mut r, KeyCode::Right);
    ready(&mut r);
    key(&mut r, KeyCode::Right);
    ready(&mut r);
    key(&mut r, KeyCode::Char('y'));
    assert_eq!(clipboard(&mut r), "²".as_bytes());
    key(&mut r, KeyCode::Char('Y'));
    assert_eq!(clipboard(&mut r), b"[^missing]");
}

#[test]
fn narrow_root_reference_keeps_every_number_digit_and_the_complete_original_marker() {
    let source = (1..=12)
        .map(|n| format!("[^n{n}]"))
        .collect::<Vec<_>>()
        .join(" ")
        + "\n";
    let mut r = FrameRuntime::from_finished_source(
        &source,
        FrameGeometry {
            rows: 40,
            cols: 1,
            content_width: 1,
        },
        LinkMode::NoOsc8,
    )
    .unwrap();
    ready(&mut r);
    let start = source.find("[^n12]").unwrap();
    let span = mdtui3::Span::new(start, start + 6);
    let mut digits = String::new();
    for row in 0..40 {
        let cell = r.screen().front_cell(row, 0).unwrap();
        if cell.source() == Some(span) {
            digits.push_str(&cell.text().collect_text());
        }
    }
    assert_eq!(digits, "¹²");
}

#[test]
fn chooser_converges_with_one_query_per_frame_and_keeps_every_visible_entry() {
    let mut r = FrameRuntime::from_finished_source(
        "a[^a] b[^b] c[^missing]\n\n[^a]: A\n[^b]: B\n",
        FrameGeometry {
            rows: 8,
            cols: 40,
            content_width: 40,
        },
        LinkMode::NoOsc8,
    )
    .unwrap();
    ready(&mut r);
    key(&mut r, KeyCode::Char('f'));
    let mut wire = vec![];
    let mut committed = false;
    for _ in 0..10000 {
        let commit = r
            .frame_to(
                &mut wire,
                &mut Budget::new(WorkLimit {
                    queries: 1,
                    objects: 1,
                    bytes: 6,
                    dependencies: 1,
                    tiles: 1,
                    recovery: 1,
                }),
            )
            .unwrap();
        if commit.committed {
            committed = true;
            break;
        }
    }
    assert!(
        committed,
        "chooser starved its wire under the shared budget"
    );
    let grid = ready(&mut r);
    assert_eq!(grid[1], "¹ a [Defined]");
    assert_eq!(grid[2], "² b [Defined]");
    assert_eq!(grid[3], "³ missing [Missing]");
}

#[test]
fn note_end_follows_only_that_body_and_ctrl_end_and_home_pause_at_their_own_cut() {
    let mut r = FrameRuntime::new(
        FrameGeometry {
            rows: 4,
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
    let update = r
        .append_bytes(b"See [^n].\n\n[^n]: first\n\n    second\n\n    third\n")
        .unwrap()
        .unwrap();
    r.apply_update(&update).unwrap();
    ready(&mut r);
    key(&mut r, KeyCode::Char('f'));
    ready(&mut r);
    key(&mut r, KeyCode::Enter);
    ready(&mut r);
    key(&mut r, KeyCode::End);
    let grid = ready(&mut r);
    assert!(grid[3].starts_with("FOLLOW"));
    let update = r.append_bytes(b"\n    fourth\n").unwrap().unwrap();
    r.apply_update(&update).unwrap();
    assert!(ready(&mut r).iter().any(|s| s == "fourth"));
    r.footnote_key(KeyCode::End, KeyModifiers::CONTROL, true)
        .unwrap();
    ready(&mut r);
    let update = r.append_bytes(b"\n    fifth\n").unwrap().unwrap();
    r.apply_update(&update).unwrap();
    let frozen = ready(&mut r);
    assert!(!frozen.iter().any(|s| s == "fifth"));
    assert!(!frozen[3].starts_with("FOLLOW"));
    key(&mut r, KeyCode::End);
    ready(&mut r);
    key(&mut r, KeyCode::Home);
    assert_eq!(ready(&mut r)[0], "first");
    let update = r
        .append_bytes(b"\n    sixth\n# outside\n")
        .unwrap()
        .unwrap();
    r.apply_update(&update).unwrap();
    assert_eq!(ready(&mut r)[0], "first");
    assert!(!r.viewport().following());
}

#[test]
fn restored_note_fold_masks_math_before_any_engine_or_payload_preparation() {
    let mut r = FrameRuntime::from_finished_source(
        "See [^a].\n\n[^a]: # H\n\n    $\\frac{1}{2}$\n[^b]: B\n",
        FrameGeometry {
            rows: 8,
            cols: 40,
            content_width: 40,
        },
        LinkMode::NoOsc8,
    )
    .unwrap();
    ready(&mut r);
    key(&mut r, KeyCode::Char('f'));
    ready(&mut r);
    key(&mut r, KeyCode::Enter);
    ready(&mut r);
    assert!(r.footnote_stats().unwrap().math_owners > 0);
    key(&mut r, KeyCode::Char('z'));
    ready(&mut r);
    key(&mut r, KeyCode::Enter);
    ready(&mut r);
    key(&mut r, KeyCode::Esc);
    ready(&mut r);
    key(&mut r, KeyCode::Char('f'));
    ready(&mut r);
    key(&mut r, KeyCode::Down);
    ready(&mut r);
    key(&mut r, KeyCode::Enter);
    ready(&mut r);
    key(&mut r, KeyCode::Char('f'));
    ready(&mut r);
    key(&mut r, KeyCode::Home);
    ready(&mut r);
    key(&mut r, KeyCode::Enter);
    ready(&mut r);
    let stats = r.footnote_stats().unwrap();
    assert_eq!(stats.math_owners, 0);
    assert_eq!(stats.shape.source_decode_bytes, 0);
    assert_eq!(stats.note_creations - stats.note_retirements, 1);
}
