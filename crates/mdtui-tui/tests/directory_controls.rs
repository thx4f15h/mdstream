use mdtui_tui::render::objects::SharedLinks;
use mdtui_tui::sidebar::directory::{Directory, DirectoryEvent as E, NameStatus, QUERY_BYTES};
use mdtui3::{Budget, Document, HeadingId, WorkLimit};

fn doc(text: &str) -> Document {
    let mut doc = Document::new();
    doc.append(text).unwrap();
    doc.finish();
    doc
}
fn limit() -> WorkLimit {
    WorkLimit {
        queries: 1000,
        objects: 1000,
        bytes: 16000,
        tiles: 1000,
        recovery: 1000,
        dependencies: 1000,
    }
}
fn settle(d: &mut Directory, doc: &Document, source: usize, rows: usize, width: usize) {
    for _ in 0..100_000 {
        d.advance(doc, source, rows, width, &mut Budget::new(limit()))
            .unwrap();
        if !d.pending() {
            return;
        }
    }
    panic!("directory failed to settle {} {:?}", d.notice(), d.work());
}
fn ids(d: &Directory) -> Vec<usize> {
    d.entries().map(|r| r.id.0).collect()
}
fn event(d: &mut Directory, doc: &Document, e: E, rows: usize) {
    d.event(e);
    settle(d, doc, 0, rows, 36);
}
fn query(d: &mut Directory, doc: &Document, query: &str, rows: usize) {
    d.begin_edit();
    while d.delete() {}
    for ch in query.chars() {
        assert!(d.insert(ch));
    }
    d.submit();
    settle(d, doc, 0, rows, 36);
}

#[test]
fn scope_tree_collapse_expand_parent_and_level_are_distinct() {
    let doc = doc("# Root\n\n### Gap\n\n> # Inner\n>\n> ## Child\n\n# Next\n\n## Tail\n");
    assert_eq!(
        doc.headings().outline_parent(HeadingId(2)),
        Some(HeadingId(1))
    );
    assert_eq!(doc.headings().parent(HeadingId(2)), None);
    let mut d = Directory::new(&doc, SharedLinks::new(&doc));
    settle(&mut d, &doc, 0, 12, 36);
    assert_eq!(ids(&d), [0, 1, 2, 3, 4, 5]);
    event(&mut d, &doc, E::Right, 12);
    assert_eq!(d.selection(), Some(HeadingId(1)));
    event(&mut d, &doc, E::Left, 12);
    assert!(d.collapsed(HeadingId(1)));
    assert_eq!(ids(&d), [0, 1, 4, 5]);
    event(&mut d, &doc, E::Right, 12);
    assert_eq!(ids(&d), [0, 1, 2, 3, 4, 5]);
    event(&mut d, &doc, E::Right, 12);
    assert_eq!(d.selection(), Some(HeadingId(2)));
    event(&mut d, &doc, E::Left, 12);
    assert_eq!(d.selection(), Some(HeadingId(2))); // collapse own child first
    event(&mut d, &doc, E::Left, 12);
    assert_eq!(d.selection(), Some(HeadingId(1)));
    assert!(d.set_level(1));
    settle(&mut d, &doc, 0, 12, 36);
    assert_eq!(ids(&d), [0, 2, 4]);
    assert_eq!(d.selection(), Some(HeadingId(0)));
    assert!(d.set_level(0));
    settle(&mut d, &doc, 0, 12, 36);
    assert_eq!(ids(&d), [0, 1, 2, 4, 5]); // child wish survived filtering
}

#[test]
fn literal_names_are_bounded_unique_heading_pages_with_exact_previous() {
    let doc = doc(
        "# aaA\n\n# &amp; aaa\n\n# ![aaa](img)\n\n# AAA\n\n# 中文aaa\n\n# aaa aaa\n\n# last aaa\n",
    );
    let mut d = Directory::new(&doc, SharedLinks::new(&doc));
    settle(&mut d, &doc, 0, 5, 36);
    query(&mut d, &doc, "aa", 5);
    assert_eq!(ids(&d), [0, 1, 2]);
    assert_eq!(d.status, NameStatus::Page);
    event(&mut d, &doc, E::Move(3), 5);
    assert_eq!(ids(&d), [4, 5, 6]);
    event(&mut d, &doc, E::Move(-3), 5);
    assert_eq!(ids(&d), [0, 1, 2]);
    event(&mut d, &doc, E::Move(3), 5);
    assert_eq!(ids(&d), [4, 5, 6]);
    event(&mut d, &doc, E::End, 5);
    assert_eq!(ids(&d), [4, 5, 6]);
    assert_eq!(d.status, NameStatus::Complete);
    query(&mut d, &doc, "AAA", 5);
    assert_eq!(ids(&d), [3]);
    query(&mut d, &doc, "&", 5);
    assert_eq!(ids(&d), [1]);
    query(&mut d, &doc, "missing", 5);
    assert!(ids(&d).is_empty());
    assert_eq!(d.selection(), None);
}

#[test]
fn zero_budget_reads_nothing_micro_budget_progresses_and_input_has_scalar_bound() {
    let doc = doc("# 汉字q\n\n## Child\n");
    let mut d = Directory::new(&doc, SharedLinks::new(&doc));
    assert_eq!(d.work().metadata, 0);
    assert_eq!(d.stats().source_bytes_read, 0);
    d.advance(&doc, 0, 8, 36, &mut Budget::new(WorkLimit::default()))
        .unwrap();
    assert_eq!(d.work().metadata, 0);
    assert_eq!(d.stats().source_bytes_read, 0);
    assert!(d.begin_edit());
    assert!(d.insert('q'));
    assert!(d.insert('汉'));
    assert!(d.delete());
    assert_eq!(d.editing(), Some("q"));
    while d.insert('界') {}
    assert!(d.editing().unwrap().len() <= QUERY_BYTES);
    assert!(d.cancel_edit());
    assert!(d.query().is_empty());
    let micro = WorkLimit {
        queries: 1,
        objects: 1,
        bytes: 6,
        tiles: 1,
        recovery: 1,
        dependencies: 1,
    };
    let mut frames = 0;
    while d.pending() {
        let mut b = Budget::new(micro);
        d.advance(&doc, 0, 8, 36, &mut b).unwrap();
        assert!(b.used().queries <= 1 && b.used().objects <= 1 && b.used().bytes <= 6);
        frames += 1;
        assert!(frames < 10000);
    }
    assert_eq!(ids(&d), [0, 1]);
    assert!(frames > 2);
    d.begin_edit();
    d.insert('q');
    d.submit();
    while d.pending() {
        d.advance(&doc, 0, 8, 36, &mut Budget::new(micro)).unwrap();
        frames += 1;
        assert!(frames < 10000);
    }
    assert_eq!(ids(&d), [0]);
}

#[test]
fn append_stales_name_task_until_explicit_resubmit_and_preserves_ids() {
    let mut doc = Document::new();
    doc.append("# [late][r]\n\n## Child\n\n").unwrap();
    let mut d = Directory::new(&doc, SharedLinks::new(&doc));
    settle(&mut d, &doc, 0, 8, 36);
    query(&mut d, &doc, "[r]", 8);
    assert_eq!(ids(&d), [0]);
    let selected = d.selection();
    let update = doc.append("[r]: /url\n\n# Tail\n").unwrap();
    d.apply_update(&doc, &update);
    settle(&mut d, &doc, 0, 8, 36);
    assert_eq!(d.status, NameStatus::Stale);
    assert!(ids(&d).is_empty());
    assert_eq!(d.selection(), selected);
    let work = d.work();
    settle(&mut d, &doc, 0, 8, 36);
    assert_eq!(d.work(), work);
    d.begin_edit();
    d.submit();
    settle(&mut d, &doc, 0, 8, 36);
    assert!(ids(&d).is_empty());
    assert_eq!(d.status, NameStatus::Complete);
    query(&mut d, &doc, "late", 8);
    assert_eq!(ids(&d), [0]);
    d.clear_query();
    settle(&mut d, &doc, 0, 8, 36);
    assert_eq!(ids(&d), [0, 1, 2]);
}

#[test]
fn independent_views_setext_and_open_title_growth_refresh_only_window() {
    let mut doc = Document::new();
    doc.append("# Root\n\n## Child\n\nparagraph").unwrap();
    let links = SharedLinks::new(&doc);
    let mut a = Directory::new(&doc, links.clone());
    let mut b = Directory::new(&doc, links);
    settle(&mut a, &doc, 0, 8, 36);
    settle(&mut b, &doc, 0, 8, 36);
    event(&mut a, &doc, E::Left, 8);
    assert_eq!(ids(&a), [0]);
    assert_eq!(ids(&b), [0, 1]);
    let u = doc.append("\n===\n").unwrap();
    a.apply_update(&doc, &u);
    b.apply_update(&doc, &u);
    settle(&mut a, &doc, 0, 8, 36);
    settle(&mut b, &doc, 0, 8, 36);
    assert_eq!(ids(&a), [0, 2]);
    assert_eq!(ids(&b), [0, 1, 2]);
    let before = b.stats().source_bytes_read;
    let u = doc.append("\n# growing").unwrap();
    a.apply_update(&doc, &u);
    b.apply_update(&doc, &u);
    settle(&mut b, &doc, 0, 8, 36);
    let u = doc.append(" title\n").unwrap();
    a.apply_update(&doc, &u);
    b.apply_update(&doc, &u);
    settle(&mut b, &doc, 0, 8, 36);
    assert_eq!(b.entries().last().unwrap().text, "growing title");
    assert!(b.stats().source_bytes_read - before < 80);
    assert_eq!(a.selection(), Some(HeadingId(0)));
}

#[test]
fn hundred_thousand_hidden_descendants_are_skipped_and_append_does_not_walk_them() {
    let mut source = String::from("# Root\n\n");
    for i in 0..100_000 {
        source.push_str(&format!("## H{i}\n\n"));
    }
    source.push_str("# End\n");
    let mut doc = Document::new();
    doc.append(&source).unwrap();
    let mut d = Directory::new(&doc, SharedLinks::new(&doc));
    settle(&mut d, &doc, 0, 8, 36);
    let before = d.work();
    event(&mut d, &doc, E::Left, 8);
    assert_eq!(ids(&d), [0, 100001]);
    assert!(d.work().metadata - before.metadata < 40);
    event(&mut d, &doc, E::End, 8);
    let before = d.work();
    let bytes = d.stats().source_bytes_read;
    let update = doc.append("\nordinary body\n").unwrap();
    d.apply_update(&doc, &update);
    settle(&mut d, &doc, 0, 8, 36);
    assert!(d.work().metadata - before.metadata < 40);
    assert_eq!(d.stats().source_bytes_read, bytes);
    assert!(d.memory().rows <= 6 && d.memory().collapsed_keys == 1);
    eprintln!(
        "hidden=100000 metadata_delta={} memory={:?}",
        d.work().metadata - before.metadata,
        d.memory()
    );
}

#[test]
fn deep_outline_breadcrumb_is_bounded_but_parent_selection_pays_for_ancestors() {
    let mut source = String::from("# Root\n\n");
    for depth in 1..=1000 {
        source.push_str(&format!("{} # H{depth}\n", ">".repeat(depth)));
    }
    let doc = doc(&source);
    let source = doc.headings().entries().last().unwrap().source.start;
    let mut d = Directory::new(&doc, SharedLinks::new(&doc));
    let micro = WorkLimit {
        queries: 1,
        objects: 1,
        bytes: 6,
        tiles: 1,
        recovery: 1,
        dependencies: 1,
    };
    let mut frames = 0;
    loop {
        d.advance(&doc, source, 5, 36, &mut Budget::new(micro))
            .unwrap();
        frames += 1;
        if !d.pending() {
            break;
        }
        assert!(frames < 100_000);
    }
    assert_eq!(d.selection(), Some(HeadingId(1000)));
    assert!(d.breadcrumb_text().starts_with("…/"));
    assert!(d.breadcrumb_text().len() < 128);
    assert!(d.work().ancestor_steps >= 1000);
    assert!(d.memory().rows <= 3);
    eprintln!(
        "deep=1000 frames={frames} work={:?} memory={:?}",
        d.work(),
        d.memory()
    );
}

#[test]
fn giant_title_excerpts_have_source_display_and_column_limits() {
    let doc = doc(&format!(
        "# {}\n\n# ![&#x754c;&amp;](u)\n",
        "**long** ".repeat(120_000)
    ));
    let mut d = Directory::new(&doc, SharedLinks::new(&doc));
    settle(&mut d, &doc, 0, 6, 12);
    assert!(d.stats().source_bytes_read < 5000);
    for entry in d.entries() {
        assert!(entry.text.len() < 256);
        assert!(mdtui_tui::render::text::display_width(&entry.text) <= 12);
    }
    assert!(d.memory().title_capacity_bytes < 32_000);
    assert!(d.entries().next().unwrap().truncated);
    query(&mut d, &doc, "long", 6);
    assert_eq!(ids(&d), [0]);
    assert!(d.memory().query_capacity_bytes < 8192);
}

#[test]
#[ignore = "manual million-heading complexity acceptance"]
fn manual_million_outline_window_reads_only_local_metadata() {
    let mut source = String::with_capacity(12_000_000);
    for i in 0..1_000_000 {
        source.push_str(&format!("# H{i}\n\n"));
    }
    let doc = doc(&source);
    assert_eq!(doc.headings().entries().len(), 1_000_000);
    let source = doc.headings().entries()[999_995].source.start;
    let mut d = Directory::new(&doc, SharedLinks::new(&doc));
    settle(&mut d, &doc, source, 10, 24);
    assert_eq!(d.selection(), Some(HeadingId(999_995)));
    assert_eq!(ids(&d), (999_991..999_999).collect::<Vec<_>>());
    assert!(d.work().metadata < 40 && d.stats().source_bytes_read < 256);
    assert!(d.memory().rows <= 8 && d.memory().row_capacity_bytes < 8192);
    eprintln!(
        "million headings work={:?} sidebar={:?} memory={:?}",
        d.work(),
        d.stats(),
        d.memory()
    );
}

#[test]
fn query_cancel_retires_the_only_shared_memo_under_micro_budget() {
    let text = format!(
        "before heading\n\n# [![alt](/img) {}](/outer)\n",
        (0..1024).map(|i| format!("[r{i}] ")).collect::<String>()
    );
    let doc = doc(&text);
    let links = SharedLinks::new(&doc);
    let mut d = Directory::new(&doc, links.clone());
    d.begin_edit();
    for c in "missing".chars() {
        d.insert(c);
    }
    d.submit();
    let micro = WorkLimit {
        queries: 1,
        objects: 1,
        bytes: 6,
        tiles: 1,
        recovery: 1,
        dependencies: 1,
    };
    for _ in 0..100_000 {
        d.advance(&doc, 0, 6, 12, &mut Budget::new(micro)).unwrap();
        if links.stats().memo_entries >= 100 {
            break;
        }
    }
    assert_eq!(d.status, NameStatus::Searching);
    let before = links.stats().memo_entries;
    assert!(before > 10, "memo {before}");
    assert!(d.memory().query_capacity_bytes >= "missing".len() * 25);
    assert_eq!(d.stats().excerpts_created, 0);
    d.clear_query();
    d.advance(&doc, 0, 6, 12, &mut Budget::new(WorkLimit::default()))
        .unwrap();
    assert_eq!(links.stats().memo_entries, before);
    assert!(d.memory().query_capacity_bytes <= QUERY_BYTES); // bounded String, KMP task gone
    let micro = WorkLimit {
        queries: 1,
        objects: 1,
        bytes: 6,
        tiles: 1,
        recovery: 1,
        dependencies: 1,
    };
    let retired = links.stats().resolution.retired_entries;
    let mut peak = before;
    for _ in 0..100_000 {
        let old = links.stats().resolution.retired_entries;
        let complete = links.prepare(&doc, &mut Budget::new(micro)).unwrap();
        assert!(links.stats().resolution.retired_entries - old <= 1);
        peak = peak.max(links.stats().memo_entries);
        if complete {
            break;
        }
    }
    assert_eq!(links.stats().memo_entries, 0);
    assert!(links.stats().resolution.retired_entries - retired >= before as u64);
    assert!(peak <= before);
    eprintln!(
        "cancel memo_before={before} retired={} peak={peak}",
        links.stats().resolution.retired_entries - retired
    );
}

#[test]
fn first_child_can_restart_level_inside_a_container_and_filter_does_not_jump_to_sibling() {
    for source in [
        "# Outer\n\n> # Restart\n>\n> ### Deep\n\n# After\n",
        "# Outer\n\n- # Restart\n  - ### Deep\n\n# After\n",
    ] {
        let doc = doc(source);
        assert_eq!(doc.headings().first_child(HeadingId(0)), None);
        let mut d = Directory::new(&doc, SharedLinks::new(&doc));
        settle(&mut d, &doc, 0, 8, 36);
        event(&mut d, &doc, E::Right, 8);
        assert_eq!(d.selection(), Some(HeadingId(1)));
        d.set_level(1);
        settle(&mut d, &doc, 0, 8, 36);
        event(&mut d, &doc, E::Right, 8);
        assert_eq!(d.selection(), Some(HeadingId(1)));
        let work = d.work();
        assert!(!d.event(E::Right));
        settle(&mut d, &doc, 0, 8, 36);
        assert_eq!(d.work(), work);
        event(&mut d, &doc, E::Left, 8);
        assert_eq!(ids(&d), [0, 1, 3]);
        event(&mut d, &doc, E::Left, 8);
        assert_eq!(d.selection(), Some(HeadingId(0)));
    }
}

#[test]
fn filtered_selection_current_and_query_boundaries_remain_independent() {
    let doc = doc("### Hidden\n\n# Alpha\n\n## Betaaa\n\n# Zeta\n");
    let mut d = Directory::new(&doc, SharedLinks::new(&doc));
    settle(&mut d, &doc, 0, 5, 36);
    d.set_level(1);
    settle(&mut d, &doc, 0, 5, 36);
    assert_eq!(d.current(), Some(HeadingId(0)));
    assert_eq!(d.selection(), Some(HeadingId(1)));
    assert_eq!(d.visible_current(), None);
    assert_eq!(ids(&d), [1, 3]);
    d.set_level(0);
    settle(&mut d, &doc, 0, 5, 36);
    query(&mut d, &doc, "a", 5);
    assert_eq!(ids(&d), [1, 2, 3]);
    let work = d.work();
    assert!(!d.event(E::Home));
    assert!(!d.event(E::Move(-1)));
    settle(&mut d, &doc, 0, 5, 36);
    assert_eq!(d.work(), work);
    event(&mut d, &doc, E::End, 5);
    let work = d.work();
    assert!(!d.event(E::End));
    assert!(!d.event(E::Move(1)));
    settle(&mut d, &doc, 0, 5, 36);
    assert_eq!(d.work(), work);
    event(&mut d, &doc, E::Home, 5);
    assert_eq!(ids(&d), [1, 2, 3]);
    assert_eq!(d.selection(), Some(HeadingId(1)));
}

#[test]
fn append_inside_open_collapsed_ancestor_does_not_scan_old_descendants_or_change_body_wishes() {
    let mut doc = Document::new();
    let mut text = String::from("# Root\n\n");
    for i in 0..100_000 {
        text.push_str(&format!("## H{i}\n\n"));
    }
    doc.append(&text).unwrap();
    let mut body = mdtui3::FoldView::<()>::new(&doc, 1).unwrap();
    body.set_collapsed(&doc, mdtui3::FoldId::Heading(HeadingId(1)), true)
        .unwrap();
    let mut d = Directory::new(&doc, SharedLinks::new(&doc));
    settle(&mut d, &doc, 0, 8, 36);
    event(&mut d, &doc, E::Left, 8);
    assert_eq!(ids(&d), [0]);
    let before = d.work();
    let bytes = d.stats().source_bytes_read;
    for _ in 0..100 {
        let update = doc.append("new ordinary body\n").unwrap();
        d.apply_update(&doc, &update);
        body.apply(&doc, &update).unwrap();
    }
    settle(&mut d, &doc, 0, 8, 36);
    assert!(d.work().metadata - before.metadata < 20);
    assert_eq!(d.stats().source_bytes_read, bytes);
    assert!(body.is_collapsed(mdtui3::FoldId::Heading(HeadingId(1))));
    event(&mut d, &doc, E::Right, 8);
    assert!(body.is_collapsed(mdtui3::FoldId::Heading(HeadingId(1))));
    assert!(!body.is_collapsed(mdtui3::FoldId::Heading(HeadingId(0))));
    assert_eq!(d.selection(), Some(HeadingId(0)));
}

#[test]
fn name_page_resize_restarts_at_retained_window_and_keeps_selected_id() {
    let mut source = String::new();
    for i in 0..20000 {
        source.push_str(&format!("# Match{i}\n\n"));
    }
    let doc = doc(&source);
    let mut d = Directory::new(&doc, SharedLinks::new(&doc));
    settle(&mut d, &doc, 0, 8, 36);
    query(&mut d, &doc, "Match", 8);
    event(&mut d, &doc, E::End, 8);
    let selected = d.selection();
    assert_eq!(selected, Some(HeadingId(19999)));
    let before = d.work().search_steps;
    settle(&mut d, &doc, 0, 5, 24);
    assert_eq!(d.selection(), selected);
    assert_eq!(ids(&d), [19998, 19999]);
    assert!(d.work().search_steps - before < 10);
    let before = d.work();
    assert!(!d.event(E::End));
    settle(&mut d, &doc, 0, 5, 24);
    assert_eq!(d.work(), before);
}

#[test]
fn current_source_change_with_zero_budget_stays_pending_and_hides_old_path() {
    let doc = doc("# Alpha\n\n# Beta\n");
    let mut d = Directory::new(&doc, SharedLinks::new(&doc));
    settle(&mut d, &doc, 0, 8, 36);
    let selected = d.selection();
    let source = doc.headings().get(HeadingId(1)).unwrap().source.start;
    d.advance(&doc, source, 8, 36, &mut Budget::new(WorkLimit::default()))
        .unwrap();
    assert!(d.pending());
    assert_eq!(d.current(), None);
    assert_eq!(d.visible_current(), None);
    assert_eq!(d.breadcrumb_text(), "path …");
    assert_eq!(d.selection(), selected);
    settle(&mut d, &doc, source, 8, 36);
    assert_eq!(d.current(), Some(HeadingId(1)));
    assert_eq!(d.breadcrumb_text(), "Beta");
}

#[test]
fn exact_full_last_name_page_does_not_invent_previous_page_or_wrap_selection() {
    let doc = doc("# aaa\n\n# aaa\n\n# aaa\n");
    let mut d = Directory::new(&doc, SharedLinks::new(&doc));
    settle(&mut d, &doc, 0, 5, 36);
    query(&mut d, &doc, "aa", 5);
    event(&mut d, &doc, E::Move(3), 5);
    assert_eq!(ids(&d), [0, 1, 2]);
    assert_eq!(d.selection(), Some(HeadingId(0)));
    assert!(!d.event(E::Move(-1)));
    assert!(!d.event(E::Home));
    event(&mut d, &doc, E::End, 5);
    assert_eq!(d.selection(), Some(HeadingId(2)));
    settle(&mut d, &doc, 0, 4, 36);
    assert_eq!(d.selection(), Some(HeadingId(2)));
    settle(&mut d, &doc, 0, 8, 36);
    event(&mut d, &doc, E::Home, 8);
    assert_eq!(d.selection(), Some(HeadingId(0)));
    assert_eq!(ids(&d), [0, 1, 2]);
}

#[test]
fn leaving_flat_name_results_normalizes_hidden_selection_to_collapsed_ancestor() {
    let doc = doc("# Root\n\n## Child\n\n### Leaf\n\n# Other\n");
    let mut d = Directory::new(&doc, SharedLinks::new(&doc));
    settle(&mut d, &doc, 0, 8, 36);
    event(&mut d, &doc, E::Left, 8);
    query(&mut d, &doc, "Child", 8);
    assert_eq!(d.selection(), Some(HeadingId(1)));
    assert_eq!(ids(&d), [1]);
    assert!(d.collapsed(HeadingId(0)));
    d.clear_query();
    let source = doc.headings().get(HeadingId(2)).unwrap().source.start;
    settle(&mut d, &doc, source, 8, 36);
    assert_eq!(ids(&d), [0, 3]);
    assert_eq!(d.selection(), Some(HeadingId(0)));
    assert_eq!(d.current(), Some(HeadingId(2)));
    assert_eq!(d.visible_current(), None);
    assert_eq!(d.breadcrumb_text(), "Root/Child/Leaf");
}

#[test]
fn suspended_completed_name_page_preserves_selection_and_continues_after_its_last_id() {
    let doc = doc("# aaa0\n\n# aaa1\n\n# aaa2\n\n# aaa3\n\n# aaa4\n");
    let mut d = Directory::new(&doc, SharedLinks::new(&doc));
    settle(&mut d, &doc, 0, 5, 36);
    query(&mut d, &doc, "aaa", 5);
    let selected = d.selection();
    d.suspend();
    settle(&mut d, &doc, 0, 5, 36);
    assert_eq!(ids(&d), [0, 1, 2]);
    assert_eq!(d.selection(), selected);
    event(&mut d, &doc, E::Move(3), 5);
    assert_eq!(ids(&d), [3, 4]);
}
