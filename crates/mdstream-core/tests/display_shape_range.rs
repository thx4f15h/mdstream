use mdstream::{
    Budget, Completion, DisplayShapeDomain, DisplayShapeEffectKind, DisplayShapeRangeSeed,
    DisplayShapeReplacement, DisplayShapeSeedItem as Item, Document, LayoutError, Span, Style,
    WorkLimit,
};

fn micro() -> Budget {
    Budget::new(WorkLimit {
        queries: 1,
        objects: 1,
        bytes: 6,
        tiles: 1,
        recovery: 1,
        dependencies: 1,
    })
}
fn collect(document: &Document, seed: &mut DisplayShapeRangeSeed) -> Vec<Item> {
    let mut items = Vec::new();
    for _ in 0..4096 {
        let mut budget = micro();
        let next = seed.advance(document, &mut budget).unwrap();
        let used = budget.used();
        assert!(used.queries <= 1 && used.objects <= 1);
        assert_eq!(used.bytes, 0);
        assert_eq!((used.tiles, used.recovery, used.dependencies), (0, 0, 0));
        match next {
            Ok(Some(item)) => items.push(item),
            Ok(None) => return items,
            Err(Completion::BudgetExhausted) => {}
            other => panic!("unexpected completion: {other:?}"),
        }
    }
    panic!("range cursor did not finish");
}
fn current_marks(document: &Document) -> Vec<Item> {
    let mut seed = document.display_shape_seed();
    let mut result = Vec::new();
    loop {
        match seed
            .advance(document, &mut Budget::new(WorkLimit::unlimited()))
            .unwrap()
            .unwrap()
        {
            Some(item) => result.push(item),
            None => return result,
        }
    }
}
fn mark_at(items: &[Item], at: usize) -> (Style, bool, bool, usize, usize) {
    let mut answer = (Style::default(), false, false, 0, 0);
    for item in items {
        if let Item::Mark {
            source,
            mark,
            baseline: false,
        } = item
            && source.start <= at
            && at < source.end
        {
            answer.0 = answer.0.union(mark.style);
            answer.1 |= mark.base_hidden;
            answer.2 |= mark.delimiter_hidden;
            answer.3 += mark.emphasis_depth;
            answer.4 += mark.strong_depth;
        }
    }
    answer
}
#[test]
fn empty_zero_invalid_and_foreign_ranges_preserve_work() {
    let mut document = Document::new();
    document.append("é &amp; **text**\n").unwrap();
    assert!(matches!(
        document.display_shape_range_seed(Span::new(1, 2)),
        Err(LayoutError::InvalidRange)
    ));
    assert!(matches!(
        document.display_shape_range_seed(Span::new(0, 1000)),
        Err(LayoutError::InvalidRange)
    ));
    for source in [Span::new(4, 4), Span::new(0, document.source().len())] {
        let mut seed = document.display_shape_range_seed(source).unwrap();
        let before = seed.work();
        let mut zero = Budget::new(WorkLimit::default());
        let step = seed.advance(&document, &mut zero).unwrap();
        assert!(if source.is_empty() {
            matches!(step, Ok(None))
        } else {
            matches!(step, Err(Completion::BudgetExhausted))
        });
        assert_eq!(seed.work(), before);
        assert_eq!(zero.used(), WorkLimit::default());
        let mut foreign = Document::new();
        foreign.append(document.source()).unwrap();
        assert!(matches!(
            seed.advance(&foreign, &mut zero),
            Err(LayoutError::WrongDocument)
        ));
        assert_eq!(seed.work(), before);
        assert_eq!(zero.used(), WorkLimit::default());
    }
}
#[test]
fn suffix_ranges_keep_native_marks_in_bounds() {
    let mut document = Document::new();
    document.append("# header\n\n").unwrap();
    let start = document.source().len() + 256;
    document.append(&"x".repeat(1024)).unwrap();
    let source = Span::new(start, start + 4);
    let items = collect(
        &document,
        &mut document.display_shape_range_seed(source).unwrap(),
    );
    for item in &items {
        if let Item::Mark { source: marked, .. } = item {
            assert!(
                source.start <= marked.start
                    && marked.start < marked.end
                    && marked.end <= source.end
            );
        }
    }
    let full = current_marks(&document);
    for at in source.range() {
        assert_eq!(mark_at(&items, at), mark_at(&full, at));
    }
}
#[test]
fn clipped_current_marks_match_the_native_full_seed() {
    let mut document = Document::new();
    document.append("# **abcdefgh** &amp; [x](/p)\n").unwrap();
    document.finish();
    let full = current_marks(&document);
    for source in [
        Span::new(0, 4),
        Span::new(2, 12),
        Span::new(6, 8),
        Span::new(0, document.source().len()),
    ] {
        let items = collect(
            &document,
            &mut document.display_shape_range_seed(source).unwrap(),
        );
        for item in &items {
            if let Item::Mark {
                source: marked,
                baseline,
                ..
            } = item
            {
                assert!(!baseline);
                assert!(
                    source.start <= marked.start
                        && marked.start < marked.end
                        && marked.end <= source.end
                );
            }
        }
        for at in source.range() {
            assert_eq!(mark_at(&items, at), mark_at(&full, at), "at={at}");
        }
    }
}
#[test]
fn enclosing_objects_replacements_and_footnotes_keep_full_extents() {
    for text in [
        "[abcdefgh](https://e.test)\n",
        "![abcdefgh](image)\n",
        "`abcdefgh`\n",
        "abc[^abcdefgh]def\n\n[^abcdefgh]: note\n",
    ] {
        let mut document = Document::new();
        document.append(text).unwrap();
        document.finish();
        let start = text.find("abcdefgh").unwrap() + 2;
        let source = Span::new(start, start + 4);
        let items = collect(
            &document,
            &mut document.display_shape_range_seed(source).unwrap(),
        );
        let enclosing = items.iter().any(|item| match item {
            Item::Link {
                info, active: true, ..
            } => info.source.start < source.start && info.source.end > source.end,
            Item::Replacement {
                source: original,
                replacement: DisplayShapeReplacement::Code { .. },
            }
            | Item::FootnoteReference {
                source: original, ..
            } => original.start < source.start && original.end > source.end,
            _ => false,
        });
        assert!(enclosing, "{text:?}: {items:?}");
    }
}
#[test]
fn real_parser_rollback_and_finish_stale_old_readonly_cursors() {
    let mut document = Document::new();
    document.enable_display_shape_effects().unwrap();
    // Same inherited-scope/late-pipe fixture as support/annotation_split.rs;
    // a leading pipe or an unparsed Setext title does not prove rollback.
    document.append("*prior\n").unwrap();
    let start = document.source().len();
    document
        .append(&format!("end* [a](b) {}|b\n", "x".repeat(2048)))
        .unwrap();
    let source = Span::new(start, start + 4);
    let mut seed = document.display_shape_range_seed(source).unwrap();
    seed.advance(&document, &mut micro()).unwrap().ok();
    let update = document.append("-|-\n\n").unwrap();
    assert!(update.display_shape.events().iter().any(|e| matches!(
        e.kind,
        DisplayShapeEffectKind::Rollback(DisplayShapeDomain::Annotations)
    )));
    let before = seed.work();
    let mut zero = Budget::new(WorkLimit::default());
    assert!(matches!(
        seed.advance(&document, &mut zero),
        Err(LayoutError::StaleContinuation)
    ));
    assert_eq!(seed.work(), before);
    assert_eq!(zero.used(), WorkLimit::default());
    let mut current = document.display_shape_range_seed(source).unwrap();
    let actual = collect(&document, &mut current);
    let expected = current_marks(&document);
    for at in source.range() {
        assert_eq!(mark_at(&actual, at), mark_at(&expected, at));
    }
    document.finish();
    assert!(matches!(
        current.advance(&document, &mut zero),
        Err(LayoutError::StaleContinuation)
    ));
    let actual = collect(
        &document,
        &mut document.display_shape_range_seed(source).unwrap(),
    );
    let expected = current_marks(&document);
    for at in source.range() {
        assert_eq!(mark_at(&actual, at), mark_at(&expected, at));
    }
}
#[test]
fn input_html_lookup_handles_empty_ranges_and_preceding_openers() {
    let mut document = Document::new();
    document
        .append("<span title='abcdefgh'>x</span>\n\n`<span title='ijklmnop'>x</span>`\n")
        .unwrap();
    document.finish();
    let source = document.display_source();
    let start = document.source().find("abcdefgh").unwrap();
    let span = source.first_html_in(Span::new(start, start + 4)).unwrap();
    assert_eq!(span.start, 0);
    assert_eq!(source.first_html_in(Span::new(start, start)), None);
    let mut zero = Budget::new(WorkLimit::default());
    assert_eq!(
        document
            .display_shape_html_qualified(span.start, &mut zero)
            .unwrap(),
        Err(Completion::BudgetExhausted)
    );
    assert_eq!(zero.used(), WorkLimit::default());
    assert_eq!(
        document
            .display_shape_html_qualified(span.start, &mut micro())
            .unwrap(),
        Ok(true)
    );
    let start = document.source().find("ijklmnop").unwrap();
    let span = source.first_html_in(Span::new(start, start + 4)).unwrap();
    assert_eq!(
        document
            .display_shape_html_qualified(span.start, &mut micro())
            .unwrap(),
        Ok(false)
    );
}

#[test]
fn work_steps_count_atomic_charges_without_double_counting_link_steps() {
    let mut document = Document::new();
    document.append("![outer [abcdefgh](/b)](/a)\n").unwrap();
    document.finish();
    let start = document.source().find("abcdefgh").unwrap() + 2;
    let source = Span::new(start, start + 4);
    let mut range = document.display_shape_range_seed(source).unwrap();
    let mut range_units = 0;
    let mut complete = false;
    for _ in 0..4096 {
        let mut budget = micro();
        let step = range.advance(&document, &mut budget).unwrap();
        range_units += budget.used().queries + budget.used().objects;
        if matches!(step, Ok(None)) {
            complete = true;
            break;
        }
    }
    assert!(complete);
    let mut links = document.link_window(source);
    let mut link_steps = 0;
    complete = false;
    for _ in 0..4096 {
        let mut budget = micro();
        let step = links.advance(&document, &mut budget).unwrap();
        // Native LinkWindow charges one query OR one object per logical step.
        link_steps += budget.used().queries + budget.used().objects;
        if matches!(step, mdstream::LinkStep::Complete) {
            complete = true;
            break;
        }
    }
    assert!(complete);
    assert!(link_steps > 1, "exercise the enclosing-link traversal");
    // Other range steps charge query AND object together, once per step.
    assert_eq!(2 * range.work().steps, range_units + link_steps);
}
