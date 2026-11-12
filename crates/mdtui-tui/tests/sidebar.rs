use mdtui_tui::render::text::display_width;
use mdtui_tui::sidebar::{SidebarModel, SidebarStatus, default_excerpt_limits};
use mdtui3::{Budget, Document, ExcerptLimits, LayoutError, WorkLimit};

fn parse(source: &str) -> Document {
    let mut document = Document::new();
    document.append(source).unwrap();
    document.finish();
    document
}

fn unlimited() -> Budget {
    Budget::new(WorkLimit::unlimited())
}

fn advance_until_complete(model: &mut SidebarModel, document: &Document) {
    loop {
        let mut budget = unlimited();
        match model.advance(document, &mut budget).unwrap() {
            SidebarStatus::Complete => break,
            SidebarStatus::BudgetExhausted => continue,
        }
    }
}

#[test]
fn ten_thousand_headings_only_load_the_visible_window() {
    let mut source = String::new();
    for index in 0..10_000 {
        let level = 1 + index % 3;
        source.push_str(&format!("{} H{index:05}\n\n", "#".repeat(level)));
    }
    let document = parse(&source);
    assert_eq!(document.headings().entries().len(), 10_000);

    let mut model = SidebarModel::new(&document, 9_997, 3, 12);
    assert_eq!(model.entries().len(), 3);
    assert_eq!(model.limits(), default_excerpt_limits(12));
    assert_eq!(model.limits().lines, 1);
    assert_eq!(model.stats().headings_considered, 3);
    assert_eq!(model.stats().excerpts_created, 0);
    let mut budget = unlimited();
    assert_eq!(
        model.advance(&document, &mut budget).unwrap(),
        SidebarStatus::Complete
    );

    let entries: Vec<_> = model.entries().collect();
    assert_eq!(entries.len(), 3);
    for (offset, entry) in entries.iter().enumerate() {
        assert_eq!(entry.id.0, 9_997 + offset);
        assert_eq!(entry.text, format!("H{:05}", 9_997 + offset));
        assert!(!entry.pending);
        assert!(!entry.truncated);
    }
    let stats = model.stats();
    let limits = model.limits();
    assert_eq!(stats.headings_considered, 3);
    assert_eq!(stats.excerpts_created, 3);
    assert!(budget.used().queries > 0);
    assert!(budget.used().queries <= stats.excerpt_steps);
    assert!(stats.source_bytes_read <= 3 * limits.source_bytes);
    assert!(stats.display_bytes_emitted <= 3 * limits.display_bytes);
    assert_eq!(model.pending_entries(), 0);
}

#[test]
fn ten_thousand_heading_update_stays_window_bound() {
    let mut document = Document::new();
    let mut source = String::new();
    for index in 0..10_000 {
        source.push_str(&format!("# H{index:05}\n\n"));
    }
    document.append(&source).unwrap();
    let mut model = SidebarModel::new(&document, 9_998, 2, 12);
    advance_until_complete(&mut model, &document);
    let before = model.stats();

    let update = document.append("# Tail\n\n").unwrap();
    model.apply_update(&document, &update, 9_999, 2).unwrap();
    let stats = model.stats();
    assert_eq!(stats.headings_considered, 2);
    assert_eq!(stats.reused_entries, before.reused_entries + 1);
    assert_eq!(stats.rebuilt_entries, before.rebuilt_entries + 1);
    assert_eq!(stats.excerpts_created, before.excerpts_created);
    assert_eq!(stats.source_bytes_read, before.source_bytes_read);
    let entries: Vec<_> = model.entries().collect();
    assert_eq!(entries[0].text, "H09999");
    assert_eq!(entries[1].text, "…");

    advance_until_complete(&mut model, &document);
    assert_eq!(model.stats().excerpts_created, before.excerpts_created + 1);
    assert_eq!(model.entry(1).unwrap().text, "Tail");
}

#[test]
fn huge_setext_is_clipped_to_sidebar_width_and_limits() {
    let source = include_str!("fixtures/sidebar/huge-setext.md");
    let document = parse(source);
    assert_eq!(document.headings().entries().len(), 1);
    let heading = &document.headings().entries()[0];
    let limits = ExcerptLimits {
        source_bytes: 64,
        display_bytes: 64,
        lines: 1,
    };
    let mut model = SidebarModel::with_limits(&document, 0, 1, 20, limits);
    assert_eq!(limits.lines, 1);
    assert!(heading.content.len() > limits.source_bytes);

    advance_until_complete(&mut model, &document);
    let entry = model.entry(0).expect("visible heading");
    assert!(entry.truncated);
    assert!(!entry.pending);
    assert!(entry.text.ends_with('…'));
    assert!(display_width(&entry.text) <= 20);
    assert!(model.stats().source_bytes_read <= limits.source_bytes);
    assert!(model.stats().source_bytes_read < heading.content.len());
    assert!(model.stats().display_bytes_emitted <= limits.display_bytes);
}

#[test]
fn tiny_budget_preserves_ellipsis_and_resumes_the_same_excerpt() {
    let document = parse("# This is a deliberately long heading for resume\n");
    let mut model = SidebarModel::new(&document, 0, 1, 12);
    let tiny = WorkLimit {
        queries: 1,
        tiles: 0,
        bytes: 6,
        objects: 1,
        recovery: 0,
        dependencies: 0,
    };
    let mut budget = Budget::new(tiny);
    assert_eq!(
        model.advance(&document, &mut budget).unwrap(),
        SidebarStatus::BudgetExhausted
    );
    let pending = model.entry(0).expect("pending heading");
    assert!(pending.pending);
    assert!(pending.truncated);
    assert!(pending.text.ends_with('…'));
    let pending_prefix = pending.text.strip_suffix('…').unwrap().to_owned();
    let excerpts_created = model.stats().excerpts_created;
    assert_eq!(excerpts_created, 1);
    assert!(model.stats().budget_exhausted > 0);

    advance_until_complete(&mut model, &document);
    let complete = model.entry(0).expect("complete heading");
    assert!(!complete.pending);
    assert!(complete.text.starts_with(&pending_prefix));
    assert!(display_width(&complete.text) <= 12);
    assert_eq!(model.stats().excerpts_created, excerpts_created);
}

#[test]
fn append_and_finish_make_the_model_stale_and_rebuild_is_correct() {
    let mut document = Document::new();
    document.append("# Old\n\n").unwrap();
    let mut model = SidebarModel::new(&document, 0, 4, 12);
    advance_until_complete(&mut model, &document);
    assert_eq!(model.entry(0).unwrap().text, "Old");

    let revision = document.revision();
    document.append("## New\n\n").unwrap();
    assert_ne!(document.revision(), revision);
    assert!(!model.matches(&document));
    assert_eq!(
        model.advance(&document, &mut unlimited()).unwrap_err(),
        LayoutError::StaleContinuation
    );

    let mut rebuilt = SidebarModel::new(&document, 0, 4, 12);
    advance_until_complete(&mut rebuilt, &document);
    let entries: Vec<_> = rebuilt.entries().collect();
    assert_eq!(entries.len(), 2);
    assert_eq!((entries[0].level, entries[0].text.as_str()), (1, "Old"));
    assert_eq!((entries[1].level, entries[1].text.as_str()), (2, "New"));

    let revision = document.revision();
    document.finish();
    assert_ne!(document.revision(), revision);
    assert!(!rebuilt.matches(&document));
    assert_eq!(
        rebuilt.advance(&document, &mut unlimited()).unwrap_err(),
        LayoutError::StaleContinuation
    );

    let mut finished = SidebarModel::new(&document, 0, 4, 12);
    advance_until_complete(&mut finished, &document);
    assert_eq!(finished.entry(1).unwrap().text, "New");

    let other = parse("# Other\n");
    assert_eq!(
        finished.advance(&other, &mut unlimited()).unwrap_err(),
        LayoutError::WrongDocument
    );
}

#[test]
fn empty_cjk_emoji_and_heading_level_order_are_preserved() {
    let document = parse("#\n\n## 你好🙂世界\n\n# Plain\n\n### Tail\n");
    let mut model = SidebarModel::new(&document, 0, 4, 6);
    advance_until_complete(&mut model, &document);
    let entries: Vec<_> = model.entries().collect();
    assert_eq!(entries.len(), 4);
    assert_eq!(
        entries.iter().map(|entry| entry.level).collect::<Vec<_>>(),
        vec![1, 2, 1, 3]
    );
    assert_eq!(entries[0].text, "");
    assert!(!entries[0].truncated);
    assert!(entries[1].truncated);
    assert!(entries[1].text.ends_with('…'));
    assert!(display_width(&entries[1].text) <= 6);
    assert_eq!(entries[2].text, "Plain");
    assert!(!entries[2].truncated);
    assert_eq!(entries[3].text, "Tail");
}

#[test]
fn update_reuses_unaffected_visible_entries_and_only_reads_new_ones() {
    let mut document = Document::new();
    document.append("# Alpha\n\n# Beta\n\n").unwrap();
    let mut model = SidebarModel::new(&document, 0, 2, 20);
    advance_until_complete(&mut model, &document);
    let before = model.stats();
    let alpha_before = model.entry(0).unwrap().clone();
    let beta_before = model.entry(1).unwrap().clone();

    let update = document.append("# Gamma\n\n").unwrap();
    model.apply_update(&document, &update, 0, 3).unwrap();
    let after_update = model.stats();
    assert_eq!(after_update.reused_entries, before.reused_entries + 2);
    assert_eq!(after_update.rebuilt_entries, before.rebuilt_entries + 1);
    assert_eq!(after_update.excerpts_created, before.excerpts_created);
    assert_eq!(after_update.source_bytes_read, before.source_bytes_read);
    assert_eq!(model.entry(0), Some(&alpha_before));
    assert_eq!(model.entry(1), Some(&beta_before));
    assert_eq!(model.entry(2).unwrap().text, "…");
    assert!(model.entry(2).unwrap().pending);

    advance_until_complete(&mut model, &document);
    let after_advance = model.stats();
    assert_eq!(after_advance.excerpts_created, before.excerpts_created + 1);
    assert!(after_advance.source_bytes_read > before.source_bytes_read);
    assert_eq!(model.entry(2).unwrap().text, "Gamma");
    assert!(!model.entry(2).unwrap().pending);
}

#[test]
fn heading_added_outside_the_window_does_not_rebuild_visible_entries() {
    let mut document = Document::new();
    document.append("# Alpha\n\n# Beta\n\n# Gamma\n\n").unwrap();
    let mut model = SidebarModel::new(&document, 0, 2, 20);
    advance_until_complete(&mut model, &document);
    let before = model.stats();
    let visible: Vec<_> = model.entries().cloned().collect();

    let update = document.append("# Delta\n\n").unwrap();
    model.apply_update(&document, &update, 0, 2).unwrap();
    let after = model.stats();
    assert_eq!(after.headings_considered, 2);
    assert_eq!(after.reused_entries, before.reused_entries + 2);
    assert_eq!(after.rebuilt_entries, before.rebuilt_entries);
    assert_eq!(after.excerpts_created, before.excerpts_created);
    assert_eq!(after.source_bytes_read, before.source_bytes_read);
    assert_eq!(model.entries().cloned().collect::<Vec<_>>(), visible);
}

#[test]
fn changed_heading_range_rebuilds_only_the_visible_dirty_entry() {
    let mut document = Document::new();
    document.append("# Alpha\n\n# Beta\n\n").unwrap();
    let mut model = SidebarModel::new(&document, 0, 2, 20);
    advance_until_complete(&mut model, &document);
    let before = model.stats();

    let mut update = document.append("# Gamma\n\n").unwrap();
    update.appended.start = 0;
    update.changed_headings = 0..1;
    model.apply_update(&document, &update, 0, 2).unwrap();
    let stats = model.stats();
    assert_eq!(stats.reused_entries, before.reused_entries + 1);
    assert_eq!(stats.rebuilt_entries, before.rebuilt_entries + 1);
    assert_eq!(model.entry(0).unwrap().text, "…");
    assert!(model.entry(0).unwrap().pending);
    assert_eq!(model.entry(1).unwrap().text, "Beta");

    advance_until_complete(&mut model, &document);
    assert_eq!(model.entry(0).unwrap().text, "Alpha");
}

#[test]
fn moving_the_window_reuses_overlapping_entries() {
    let document = parse("# Alpha\n\n# Beta\n\n# Gamma\n\n");
    let mut model = SidebarModel::new(&document, 0, 2, 20);
    advance_until_complete(&mut model, &document);
    let before = model.stats();
    let beta = model.entry(1).unwrap().clone();

    model.reconfigure(&document, 1, 2).unwrap();
    let stats = model.stats();
    assert_eq!(stats.reused_entries, before.reused_entries + 1);
    assert_eq!(stats.rebuilt_entries, before.rebuilt_entries + 1);
    assert_eq!(stats.excerpts_created, before.excerpts_created);
    assert_eq!(stats.source_bytes_read, before.source_bytes_read);
    assert_eq!(model.entry(0), Some(&beta));
    assert_eq!(model.entry(1).unwrap().text, "…");
}
