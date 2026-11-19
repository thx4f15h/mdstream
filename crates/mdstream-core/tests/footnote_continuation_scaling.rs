use mdstream::*;

#[test]
fn omitted_list_ancestors_are_not_revisited_for_each_lazy_row() {
    for depth in [1, 16, 64, 256, 1024] {
        let mut d = Document::new();
        d.enable_footnote_continuation().unwrap();
        d.append(&format!("{}[^n]: body\n", "- ".repeat(depth)))
            .unwrap();
        let before = d.work();
        for _ in 0..1000 {
            d.append("continuation\n").unwrap();
        }
        let work = d.work();
        let steps = work.container_steps - before.container_steps;
        println!("depth={depth} bytes={} steps={steps}", d.source().len());
        assert_eq!(d.footnote_body_count(), 1);
        assert_eq!(
            d.footnotes()[0].definition.unwrap().content.end,
            d.source().len() - 1
        );
        assert!(steps <= 3000, "depth={depth}, container steps={steps}");
    }
}

#[test]
fn omitted_list_segments_still_require_every_interleaved_quote_in_source_order() {
    let mut d = Document::new();
    d.enable_footnote_continuation().unwrap();
    let path = format!("{}> ", "- ".repeat(64)).repeat(16);
    d.append(&format!("{path}[^n]: body\n")).unwrap();
    let before = d.work();
    for _ in 0..1000 {
        d.append(&format!("{}continuation\n", "> ".repeat(16)))
            .unwrap();
    }
    let work = d.work();
    assert!(work.container_steps - before.container_steps <= 20_000);
    let end = d.footnotes()[0].definition.unwrap().content.end;
    assert_eq!(end, d.source().len() - 1);
    // Omit the final mandatory Quote while retaining all preceding ones.
    d.append(&format!("{}outside\n", "> ".repeat(15))).unwrap();
    assert_eq!(d.footnotes()[0].definition.unwrap().content.end, end);
    d.finish();
    println!(
        "interleaved input={} steps={} quote_bytes={}",
        d.source().len(),
        work.container_steps - before.container_steps,
        work.container_bytes - before.container_bytes
    );
}

#[test]
fn extended_mode_million_byte_appends_do_not_reclassify_a_historical_line_or_mapping() {
    let mut d = Document::new();
    d.enable_footnote_continuation().unwrap();
    d.append(&format!("> [^n]: first\n>     {}", "x".repeat(2048)))
        .unwrap();
    let before = d.footnote_body_work();
    let root = d.work();
    let pieces = d.footnote_body(FootnoteId(0)).unwrap().mapping_count();
    for _ in 0..1_000_000 {
        d.append("x").unwrap();
    }
    let after = d.footnote_body_work();
    let work = d.work();
    assert_eq!(after.source_bytes - before.source_bytes, 1_000_000);
    assert_eq!(after.projected_bytes - before.projected_bytes, 1_000_000);
    assert_eq!(
        after.engine.input_bytes - before.engine.input_bytes,
        1_000_000
    );
    assert_eq!(
        after.engine.classified_bytes,
        before.engine.classified_bytes
    );
    assert_eq!(work.classified_bytes, root.classified_bytes);
    assert_eq!(work.container_steps, root.container_steps);
    assert_eq!(after.address_steps, before.address_steps);
    assert_eq!(after.publication_steps, before.publication_steps);
    let body = d.footnote_body(FootnoteId(0)).unwrap();
    assert_eq!(body.mapping_count(), pieces);
    assert!(body.storage().projection_capacity < 2 * body.storage().projection_bytes);
    println!(
        "extended million before={before:?} after={after:?} root={work:?} storage={:?}",
        body.storage()
    );
    d.finish();
    assert!(d.work().classified_bytes - work.classified_bytes < 6_000_000);
}

#[test]
fn extended_twenty_thousand_sealed_bodies_share_late_bindings_without_old_body_visits() {
    let mut d = Document::new();
    d.enable_footnote_continuation().unwrap();
    d.enable_display_shape_effects().unwrap();
    let mut source = String::new();
    for i in 0..20_000 {
        source.push_str(&format!("[^n{i}]: [L][r]\n"));
    }
    source.push_str("# outside\n\n");
    d.append(&source).unwrap();
    let before = d.footnote_body_work();
    let root = d.work();
    let update = d.append("[r]: https://late.test/\n").unwrap();
    assert!(update.body_updates.is_empty());
    assert_eq!(before, d.footnote_body_work());
    assert!(d.work().inline_bytes - root.inline_bytes < 100);
    // The genuine bracket parser interns both the provisional L shortcut and r.
    let last = d.footnote_body(FootnoteId(19_999)).unwrap();
    assert_eq!(last.storage().binding_entries, 2);
    let destination = last
        .raw_link_destination(last.qualify(LinkId(0)))
        .unwrap()
        .unwrap();
    assert_eq!(destination.text, "https://late.test/");
    assert!(matches!(destination.source,BodyAddress::Original {document,..} if document==d.id()));
    println!(
        "extended 20k input={} body={before:?} root_before={root:?} root_after={:?} last={:?}",
        source.len(),
        d.work(),
        last.storage()
    );
}
