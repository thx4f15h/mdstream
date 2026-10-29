use mdtui3::{BlockKind, Document, HeightIndex, Span, Style, TileId};

#[test]
fn one_character_appends_of_a_giant_line_have_linear_scanning_work() {
    for size in [8_192, 16_384, 32_768] {
        let mut doc = Document::new();
        for _ in 0..size {
            doc.append("x").unwrap();
        }
        doc.finish();
        let work = doc.work();
        assert_eq!(work.input_bytes, size);
        assert_eq!(work.inline_bytes, size);
        assert!(work.classified_bytes <= 1027);
        assert!(work.tree_visits < 200);
        assert_eq!(doc.blocks().len(), 1);
    }
}

#[test]
fn adversarial_delimiters_have_linear_lifetimes() {
    for count in [8_192, 16_384, 32_768] {
        let source = format!("*a {}z*\n", "_b ".repeat(count));
        let mut doc = Document::new();
        doc.append(&source).unwrap();
        doc.finish();
        let work = doc.work();
        assert_eq!(work.inline_bytes, source.len() as u64);
        assert!(work.delimiter_steps <= (count * 3 + 10) as u64);
        assert!(work.tree_visits < 500);
        let middle = doc.tile_at(source.len() / 2).unwrap();
        assert!(
            doc.runs(middle)
                .unwrap()
                .all(|run| run.style.contains(Style::EMPHASIS))
        );
    }
}

#[test]
fn definition_cost_is_independent_of_prior_reference_uses() {
    for uses in [100, 10_000, 30_000] {
        let mut doc = Document::new();
        doc.append(&format!("{}\n\n", "[key] ".repeat(uses)))
            .unwrap();
        let before = doc.work();
        let update = doc.append("[key]: /destination\n").unwrap();
        let after = doc.work();
        assert_eq!(update.references_changed.len(), 1);
        assert_eq!(after.label_bytes - before.label_bytes, 3);
        assert!(after.tree_visits - before.tree_visits < 200);
        assert!(update.dirty.len() < 8);
        assert_eq!(
            doc.link_destination(mdtui3::LinkId(0)),
            Some("/destination")
        );
        assert_eq!(
            doc.link_destination(mdtui3::LinkId(uses - 1)),
            Some("/destination")
        );
    }
}

#[test]
fn setext_conversion_does_not_visit_the_old_paragraph() {
    let mut doc = Document::new();
    doc.append(&format!("{}\n", "text ".repeat(200_000)))
        .unwrap();
    let key = doc.tile_key(TileId(0)).unwrap();
    let before = doc.work();
    let update = doc.append("===\n").unwrap();
    assert!(matches!(
        doc.blocks()[0].kind,
        BlockKind::Heading { level: 1 }
    ));
    assert_eq!(doc.work().inline_bytes, before.inline_bytes);
    assert!(doc.work().tree_visits - before.tree_visits < 200);
    assert_ne!(doc.tile_key(TileId(0)).unwrap(), key);
    let mut heights = HeightIndex::default();
    heights.rebuild(&doc).unwrap();
    let before = heights.mutation_visits();
    for span in update.dirty {
        heights.invalidate(doc.tiles_overlapping(span)).unwrap();
    }
    assert!(heights.mutation_visits() - before < 500);
}

#[test]
fn visible_queries_do_not_parse_or_allocate_document_state() {
    let mut doc = Document::new();
    doc.append(&format!("**{}**\n", "word ".repeat(200_000)))
        .unwrap();
    doc.finish();
    let before = doc.work();
    let nodes = doc.annotation_node_count();
    let mut heights = HeightIndex::default();
    heights.rebuild(&doc).unwrap();
    let page = heights.viewport(2_000..2_024, 2, 32);
    assert_eq!(page.len(), 28);
    let mut bytes = 0;
    for tile in page {
        for run in doc.runs(tile.tile).unwrap() {
            bytes += run.text.len();
        }
    }
    assert!(bytes <= 28 * mdtui3::TILE_BYTES);
    assert_eq!(before, doc.work());
    assert_eq!(nodes, doc.annotation_node_count());
}

#[test]
fn dense_markup_tree_memory_and_work_are_bounded() {
    let mut previous = None;
    for repetitions in [2_000, 4_000, 8_000] {
        let source = "*a* **b** ~~c~~ [d](e) `f`\n".repeat(repetitions);
        let mut doc = Document::new();
        doc.append(&source).unwrap();
        doc.finish();
        assert!(doc.annotation_node_count() <= 4 * source.len());
        assert!(doc.work().inline_bytes <= source.len() as u64);
        assert!(doc.work().classified_bytes <= source.len() as u64);
        let visits = doc.work().tree_visits;
        if let Some(last) = previous {
            assert!(visits < last * 3);
        }
        previous = Some(visits);
        assert!(
            doc.runs_in(Span::new(0, 3))
                .unwrap()
                .any(|run| run.text == "a")
        );
    }
}

#[test]
fn huge_hidden_regions_are_skipped_without_visiting_tiles() {
    let mut doc = Document::new();
    let source = format!("```lang {}\nvisible\n```\n", "x".repeat(1_000_000));
    doc.append(&source).unwrap();
    doc.finish();
    let at = doc.next_visible(0).unwrap();
    assert_eq!(&doc.source()[at..at + 7], "visible");
    let first = doc.tile_at(at).unwrap();
    let mut heights = HeightIndex::default();
    heights.rebuild(&doc).unwrap();
    let before = heights.mutation_visits();
    heights.suppress(0..first.0).unwrap();
    assert!(heights.mutation_visits() - before < 200);
    assert_eq!(heights.viewport(0..1, 0, 1)[0].tile, first);
    let hidden = doc.source().rfind("```").unwrap();
    assert_eq!(doc.next_visible(hidden), None);
}

#[test]
fn semantic_queries_visit_intervals_instead_of_searching_per_character() {
    let mut plain = Document::new();
    plain.append(&"x".repeat(1_000_000)).unwrap();
    plain.finish();
    let mut runs = plain.runs_in(Span::new(0, plain.source().len())).unwrap();
    assert_eq!(runs.by_ref().count(), 1);
    assert!(runs.annotation_visits() < 32);

    let mut dense = Document::new();
    dense
        .append(&"*é* **🙂** [中](url)\n".repeat(10_000))
        .unwrap();
    dense.finish();
    let tile = dense.tile_at(dense.source().len() / 2).unwrap();
    let span = dense.tile(tile).unwrap();
    let mut runs = dense.runs(tile).unwrap();
    let mut cursor = span.start;
    for run in runs.by_ref() {
        assert_eq!(run.source.start, cursor);
        assert_eq!(dense.text(run.source), Some(run.text));
        cursor = run.source.end;
    }
    assert_eq!(cursor, span.end);
    assert!(runs.annotation_visits() <= 2 * span.len() as u64 + 64);
}
