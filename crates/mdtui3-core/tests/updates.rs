use mdtui3::*;

#[test]
fn identity_order_replay_and_atomic_consumption() {
    let mut a = Document::new();
    let mut b = Document::new();
    let first = a.append("first\n").unwrap();
    let foreign = b.append("other\n").unwrap();
    let mut heights = HeightIndex::default();
    let mut checkpoints = CheckpointIndex::new();
    apply_layout_update(&a, &first, &mut heights, &mut checkpoints).unwrap();
    heights
        .measure(heights.ticket(TileId(0)).unwrap(), 7)
        .unwrap();
    checkpoints.insert(TileId(0), 42).unwrap();
    apply_layout_update(&a, &first, &mut heights, &mut checkpoints).unwrap();
    assert_eq!(heights.height(TileId(0)), Some(7));
    assert_eq!(checkpoints.get(TileId(0)), Some(&42));
    assert_eq!(heights.apply(&a, &foreign), Err(LayoutError::WrongDocument));
    assert_ne!(a.tile_key(TileId(0)), b.tile_key(TileId(0)));
    let mut other = HeightIndex::default();
    other.rebuild(&a).unwrap();
    assert_eq!(
        other.measure(heights.ticket(TileId(0)).unwrap(), 2),
        Err(LayoutError::WrongView)
    );
    checkpoints.rebuild(&b);
    let second = a.append("second\n").unwrap();
    assert_eq!(
        apply_layout_update(&a, &second, &mut heights, &mut checkpoints),
        Err(LayoutError::WrongDocument)
    );
    assert_eq!(heights.height(TileId(0)), Some(7));
    // An empty update at a missed revision cannot stand in for the missed input.
    let empty = a.append("").unwrap();
    assert_eq!(
        heights.apply(&a, &empty),
        Err(LayoutError::OutOfOrderUpdate)
    );
}

#[test]
fn batch_and_reused_updates_match_immediate_consumption() {
    let mut doc = Document::new();
    let mut immediate = HeightIndex::default();
    let mut batch = Update::default();
    let mut scratch = Update::default();
    for chunk in [
        "# heading\n",
        "[x] **long",
        " text**\n\n",
        "[x]: /url\n",
        "| a | b |\n| - | - |\n",
        "| longer | c |\n",
    ] {
        doc.append_into(chunk, &mut scratch).unwrap();
        immediate.apply(&doc, &scratch).unwrap();
        batch.merge(scratch.clone()).unwrap();
    }
    doc.finish_into(&mut scratch);
    immediate.apply(&doc, &scratch).unwrap();
    batch.merge(scratch).unwrap();
    let mut delayed = HeightIndex::default();
    delayed.apply(&doc, &batch).unwrap();
    assert_eq!(
        immediate.viewport(0..100, 0, 100),
        delayed.viewport(0..100, 0, 100)
    );
    assert_eq!(batch.base_revision, 0);
    assert_eq!(batch.new_links, 0..1);
    assert!(batch.rows_changed.iter().any(|r| r.rows.contains(&1)));
    assert!(doc.blocks().iter().all(|b| b.state == SyntaxState::Sealed));
    assert!(doc.link_info(LinkId(0)).unwrap().state == SyntaxState::Sealed);
}

#[test]
fn update_and_viewport_buffers_retain_capacity() {
    let mut doc = Document::new();
    let mut update = Update::default();
    doc.append_into(&"a".repeat(2048), &mut update).unwrap();
    let capacity = update.dirty.capacity();
    for _ in 0..100 {
        doc.append_into("a", &mut update).unwrap();
        assert_eq!(update.dirty.capacity(), capacity);
    }
    let mut heights = HeightIndex::default();
    heights.rebuild(&doc).unwrap();
    let mut out = Vec::new();
    heights.viewport_into(0..5, 0, 5, &mut out);
    let ptr = out.as_ptr();
    for _ in 0..20 {
        heights.viewport_into(0..5, 0, 5, &mut out);
        assert_eq!(out.as_ptr(), ptr);
    }
}

#[test]
fn empty_attachment_binds_identity_and_repeated_eof_preserves_measurements() {
    let mut doc = Document::new();
    let mut foreign = Document::new();
    let empty = doc.append("").unwrap();
    let mut heights = HeightIndex::default();
    heights.apply(&doc, &empty).unwrap();
    let other = foreign.append("").unwrap();
    assert_eq!(
        heights.apply(&foreign, &other),
        Err(LayoutError::WrongDocument)
    );
    let update = doc.append("x").unwrap();
    heights.apply(&doc, &update).unwrap();
    let update = doc.finish();
    heights.apply(&doc, &update).unwrap();
    heights
        .measure(heights.ticket(TileId(0)).unwrap(), 9)
        .unwrap();
    heights.apply(&doc, &update).unwrap();
    let update = doc.finish();
    heights.apply(&doc, &update).unwrap();
    assert_eq!(heights.height(TileId(0)), Some(9));
    assert_eq!(heights.height_state(TileId(0)), Some(HeightState::Measured));
}

#[test]
fn object_delta_ranges_cover_every_existing_object_mutation() {
    let source = format!(
        "# {}\n\n[old](<url> 'title')\n\n| a | b |\n| - | - |\n| {} | y |\n\n",
        "title".repeat(300),
        "cell".repeat(400)
    );
    let mut doc = Document::new();
    for chunk in source.as_bytes().chunks(97) {
        let blocks = doc.blocks().to_vec();
        let headings = doc.headings().entries().to_vec();
        let links: Vec<_> = (0..doc.links().len())
            .map(|i| *doc.link_info(LinkId(i)).unwrap())
            .collect();
        let tables = doc.tables().to_vec();
        let update = doc.append(std::str::from_utf8(chunk).unwrap()).unwrap();
        for (i, old) in blocks.iter().enumerate() {
            if old != &doc.blocks()[i] {
                assert!(update.changed_blocks.contains(&i));
            }
        }
        for (i, old) in headings.iter().enumerate() {
            if old != &doc.headings().entries()[i] {
                assert!(update.changed_headings.contains(&i));
            }
        }
        for (i, old) in links.iter().enumerate() {
            if old != doc.link_info(LinkId(i)).unwrap() {
                assert!(update.changed_links.contains(&i));
            }
        }
        for (i, old) in tables.iter().enumerate() {
            for (row, old_row) in old.rows().iter().enumerate() {
                if old_row != doc.tables()[i].row(row).unwrap() {
                    assert!(
                        update
                            .rows_changed
                            .iter()
                            .any(|change| change.table == TableId(i) && change.rows.contains(&row))
                    );
                }
            }
        }
    }
}
