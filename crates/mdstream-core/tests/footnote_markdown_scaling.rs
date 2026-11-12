use mdstream::*;
fn unlimited() -> Budget {
    Budget::new(WorkLimit::unlimited())
}
#[test]
fn million_single_byte_body_appends_preserve_prefix_mapping_and_actual_work_bounds() {
    let prefix = ">".repeat(1000);
    let mut d = Document::new();
    d.append(&format!(
        "{prefix} [^n]: first\n{prefix}     {}",
        "x".repeat(2048)
    ))
    .unwrap();
    let before = d.footnote_body_work();
    let root = d.work();
    let entries = d.footnote_body(FootnoteId(0)).unwrap().mapping_count();
    for _ in 0..1_000_000 {
        d.append("x").unwrap();
    }
    let after = d.footnote_body_work();
    assert_eq!(after.source_bytes - before.source_bytes, 1_000_000);
    assert_eq!(after.projected_bytes - before.projected_bytes, 1_000_000);
    assert_eq!(
        after.engine.input_bytes - before.engine.input_bytes,
        1_000_000
    );
    assert_eq!(
        after.engine.inline_bytes - before.engine.inline_bytes,
        1_000_000
    );
    assert_eq!(
        after.engine.classified_bytes,
        before.engine.classified_bytes
    );
    assert_eq!(after.publication_steps, before.publication_steps);
    assert_eq!(after.address_steps, before.address_steps);
    assert_eq!(d.work().container_bytes, root.container_bytes);
    assert_eq!(d.work().container_steps, root.container_steps);
    let v = d.footnote_body(FootnoteId(0)).unwrap();
    assert_eq!(v.mapping_count(), entries);
    assert_eq!(v.block_count(), 1);
    assert_eq!(v.storage().binding_entries, 0);
    assert!(v.storage().projection_capacity <= 2 * v.storage().projection_bytes);
    assert!(v.storage().mapping_capacity_bytes < 1024);
    println!(
        "million body: {:?}; fixed parser={}B",
        after,
        v.storage().parser_fixed_bytes
    );
}
#[test]
fn late_body_definition_does_not_visit_twenty_thousand_old_root_uses() {
    let mut d = Document::new();
    d.append(&format!("{}\n\n", "[^n] [h][r] ".repeat(20_000)))
        .unwrap();
    let old_links = d.links().to_vec();
    let old_refs = d.footnote_references().to_vec();
    let work = d.work();
    let update = d.append("[^n]: [r]: /late\nend\n").unwrap();
    assert_eq!(d.links(), old_links);
    assert_eq!(d.footnote_references(), old_refs);
    assert_eq!(update.new_links, old_links.len()..old_links.len());
    // changed_links is conservative metadata, not a work counter.
    let delta = d.work();
    assert!(delta.link_index_steps - work.link_index_steps < 128);
    assert!(delta.link_gate_steps - work.link_gate_steps < 16);
    assert!(delta.inline_bytes - work.inline_bytes < 128);
    assert!(delta.tree_visits - work.tree_visits < 4096);
    assert_eq!(d.work().footnote_steps - work.footnote_steps, 3);
    assert_eq!(d.link_destination(LinkId(0)), Some("/late"));
    let body = d.footnote_body_work();
    assert!(body.publication_steps < 20 && body.address_steps < 100);
    assert!(body.engine.input_bytes < 100);
    assert_eq!(
        d.footnote_body(FootnoteId(0))
            .unwrap()
            .storage()
            .binding_entries,
        1
    );
}
#[test]
fn twenty_thousand_sealed_notes_do_not_receive_late_global_definition_updates() {
    let mut d = Document::new();
    let mut source = String::new();
    for i in 0..20_000 {
        source.push_str(&format!("[^n{i}]: hi\n"));
    }
    source.push_str("outside\n\n");
    d.append(&source).unwrap();
    assert_eq!(d.footnote_body_count(), 20_000);
    let before = d.footnote_body_work();
    d.append("[late]: /url\n").unwrap();
    assert_eq!(d.footnote_body_work(), before);
    let v = d.footnote_body(FootnoteId(19_999)).unwrap();
    assert_eq!(v.storage().binding_entries, 0);
    let mut c = v.source_cursor(v.range()).unwrap();
    assert_eq!(c.advance(&mut unlimited()).unwrap().unwrap().text, "h");
    println!(
        "20k notes fixed child bytes={} projection={}",
        20_000 * v.storage().parser_fixed_bytes,
        before.projected_bytes
    );
}
#[test]
fn host_document_retains_send_and_sync() {
    fn check<T: Send + Sync>() {}
    check::<Document>();
}
