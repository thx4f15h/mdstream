use mdstream::*;
fn v(d: &Document) -> BodyView<'_> {
    d.footnote_body(FootnoteId(0)).unwrap()
}
fn seed(d: &Document) -> BodyHeights {
    let view = v(d);
    let mut h = BodyHeights::new(view, 1).unwrap();
    let t = h.ticket(view, view.qualify(TileId(0))).unwrap().unwrap();
    h.measure(view, t, 99).unwrap();
    h
}
#[test]
fn streaming_body_deltas_retain_stable_prefix_heights_tickets_and_checkpoints() {
    let mut d = Document::new();
    d.append(&format!("[^n]: {}", "x".repeat(40_000))).unwrap();
    let mut h = seed(&d);
    let mut checkpoints = BodyCheckpoints::new(v(&d));
    checkpoints
        .insert(v(&d), v(&d).qualify(TileId(0)), "stable")
        .unwrap();
    let before = h.ticket(v(&d), v(&d).qualify(TileId(0))).unwrap().unwrap();
    let from = h
        .ticket_from(
            v(&d),
            v(&d).qualify(TileId(0)),
            &checkpoints,
            v(&d).qualify(TileId(0)),
        )
        .unwrap()
        .unwrap();
    let update = d.append("tail").unwrap();
    assert_eq!(update.body_updates.len(), 1);
    assert_eq!(update.body_updates[0].body(), v(&d).key());
    h.apply(v(&d), &update).unwrap();
    checkpoints.apply(v(&d), &update).unwrap();
    assert_eq!(
        h.base_height(v(&d), v(&d).qualify(TileId(0))).unwrap(),
        Some(99)
    );
    h.measure(v(&d), before, 99).unwrap();
    h.measure_from(v(&d), from, 99, &checkpoints).unwrap();
    assert_eq!(
        checkpoints.get(v(&d), v(&d).qualify(TileId(0))).unwrap(),
        Some(&"stable")
    );
    let update = d.append("\noutside\n").unwrap();
    h.apply(v(&d), &update).unwrap();
    checkpoints.apply(v(&d), &update).unwrap();
    let before = h.ticket(v(&d), v(&d).qualify(TileId(0))).unwrap().unwrap();
    let update = d.append("plain outside\n").unwrap();
    assert!(update.body_updates.is_empty());
    h.apply(v(&d), &update).unwrap();
    checkpoints.apply(v(&d), &update).unwrap();
    h.measure(v(&d), before, 99).unwrap();
    assert_eq!(
        checkpoints.get(v(&d), v(&d).qualify(TileId(0))).unwrap(),
        Some(&"stable")
    );
}
#[test]
fn split_and_merged_host_updates_apply_the_same_body_facts_without_revision_aliasing() {
    let mut d = Document::new();
    d.append(&format!("[^n]: {}", "x".repeat(40_000))).unwrap();
    let mut split = seed(&d);
    let mut merged = seed(&d);
    let a = d.append("\n    second\n").unwrap();
    split.apply(v(&d), &a).unwrap();
    let b = d.append("    third\n").unwrap();
    split.apply(v(&d), &b).unwrap();
    let mut batch = a;
    batch.merge(b).unwrap();
    assert_eq!(batch.body_updates.len(), 1);
    merged.apply(v(&d), &batch).unwrap();
    for i in 0..v(&d).tile_count() {
        let id = v(&d).qualify(TileId(i));
        assert_eq!(
            split.base_height(v(&d), id).unwrap(),
            merged.base_height(v(&d), id).unwrap()
        );
        assert_eq!(
            split.height_state(v(&d), id).unwrap(),
            merged.height_state(v(&d), id).unwrap()
        );
    }
    let rows = merged.total_rows();
    merged.apply(v(&d), &batch).unwrap();
    assert_eq!(merged.total_rows(), rows);
    let bad = Document::new().append("x").unwrap();
    assert_eq!(merged.apply(v(&d), &bad), Err(LayoutError::WrongDocument));
    assert_eq!(merged.total_rows(), rows);
    let a = d.append("    fourth\n").unwrap();
    let b = d.append("    fifth\n").unwrap();
    let rows = merged.total_rows();
    assert_eq!(merged.apply(v(&d), &b), Err(LayoutError::OutOfOrderUpdate));
    assert_eq!(merged.total_rows(), rows);
    let mut batch = a;
    batch.merge(b).unwrap();
    merged.apply(v(&d), &batch).unwrap();
}
#[test]
fn host_late_binding_expires_measurement_but_keeps_old_nonzero_height() {
    let mut d = Document::new();
    d.append(&format!(
        "[^n]: [label][r] {}\noutside\n",
        "x".repeat(40_000)
    ))
    .unwrap();
    let mut h = seed(&d);
    let mut checkpoints = BodyCheckpoints::new(v(&d));
    checkpoints
        .insert(v(&d), v(&d).qualify(TileId(0)), 7)
        .unwrap();
    let old = h.ticket(v(&d), v(&d).qualify(TileId(0))).unwrap().unwrap();
    let update = d.append("\n[r]: /late\n").unwrap();
    assert!(update.body_updates.is_empty());
    h.apply(v(&d), &update).unwrap();
    checkpoints.apply(v(&d), &update).unwrap();
    assert_eq!(
        h.base_height(v(&d), v(&d).qualify(TileId(0))).unwrap(),
        Some(99)
    );
    assert!(h.measure(v(&d), old, 3).is_err());
    assert!(
        checkpoints
            .get(v(&d), v(&d).qualify(TileId(0)))
            .unwrap()
            .is_none()
    );
    let update = d.finish();
    h.apply(v(&d), &update).unwrap();
    checkpoints.apply(v(&d), &update).unwrap();
    assert_eq!(
        h.base_height(v(&d), v(&d).qualify(TileId(0))).unwrap(),
        Some(99)
    );
}
#[test]
fn host_eof_missing_and_wrong_body_updates_do_not_rebuild_or_mutate_before_validation() {
    let mut d = Document::new();
    d.append("[^n]: [^missing]\noutside\n\n[^m]: another\nend\n")
        .unwrap();
    let mut h = seed(&d);
    let old = h.ticket(v(&d), v(&d).qualify(TileId(0))).unwrap().unwrap();
    let update = d.finish();
    let other = d.footnote_body(FootnoteId(2)).unwrap();
    let rows = h.total_rows();
    assert_eq!(h.apply(other, &update), Err(LayoutError::InvalidRange));
    assert_eq!(h.total_rows(), rows);
    h.apply_with_policy(
        v(&d),
        &update,
        LayoutPolicy {
            style_affects_size: false,
            ..LayoutPolicy::default()
        },
    )
    .unwrap();
    assert_eq!(
        h.base_height(v(&d), v(&d).qualify(TileId(0))).unwrap(),
        Some(99)
    );
    assert!(h.measure(v(&d), old, 2).is_err());
}
#[test]
fn subscribed_body_forwards_actual_shape_updates_and_global_binding_effects() {
    let mut d = Document::new();
    d.enable_display_shape_effects().unwrap();
    let first = d.append("[^n]: *em* [label][r] [^c]\noutside\n").unwrap();
    assert_eq!(first.body_updates.len(), 1);
    let body = v(&d);
    let facts = first.body_updates[0].facts();
    assert_eq!(facts.body, body.key());
    assert!(facts.value.display_shape.enabled());
    let events = facts.value.display_shape.events().to_vec();
    assert!(!events.is_empty());
    assert!(events.windows(2).all(|p| p[0].sequence < p[1].sequence));
    assert!(
        events
            .iter()
            .any(|e| matches!(e.kind, DisplayShapeEffectKind::SourceAppend(_)))
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e.kind, DisplayShapeEffectKind::ScopeEvent { .. }))
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e.kind, DisplayShapeEffectKind::FootnoteReference { .. }))
    );
    assert!(first.display_shape.events().iter().any(|e| matches!(
        e.kind,
        DisplayShapeEffectKind::Footnote { id: FootnoteId(1) }
    )));
    assert_eq!(
        body.canonical_footnote_symbol(body.qualify(FootnoteId(0)))
            .unwrap(),
        Some(FootnoteId(1))
    );
    let epoch = d.display_shape_stats().binding_epoch;
    let local = body.display_shape_stats();
    assert_eq!(local.body, body.key());
    assert!(local.value.emitted > 0);
    let late = d.append("\n[r]: /late\n").unwrap();
    assert!(late.body_updates.is_empty());
    assert!(d.display_shape_stats().binding_epoch > epoch);
    assert!(
        late.display_shape
            .events()
            .iter()
            .any(|e| matches!(e.kind, DisplayShapeEffectKind::Reference { .. }))
    );
    // Local publication is immutable; canonical nonlocal changes are consumed
    // from the enclosing host protocol, without manufacturing local history.
    assert_eq!(v(&d).display_shape_stats().value, local.value);
    let mut joined = first;
    joined.merge(late).unwrap();
    assert_eq!(
        joined.body_updates[0].facts().value.display_shape.events(),
        events.as_slice()
    );
}
#[test]
fn body_style_marker_bridge_uses_paid_canonical_late_target_and_stale_revision() {
    let mut d = Document::new();
    d.enable_display_shape_effects().unwrap();
    d.append("[^n]: [*a][r] z*\noutside\n").unwrap();
    let body = v(&d);
    let mut cursor = body.style_markers(body.range()).unwrap();
    let mut resolver = body.link_resolver();
    let before = cursor.source_position();
    assert_eq!(
        cursor
            .advance(body, &mut resolver, &mut Budget::new(WorkLimit::default()))
            .unwrap(),
        BodyStyleMarkerStep::BudgetExhausted
    );
    assert_eq!(cursor.source_position(), before);
    let mut steps = 0;
    loop {
        steps += 1;
        assert!(steps < 2000);
        let mut budget = Budget::new(WorkLimit {
            queries: 1,
            objects: 1,
            bytes: 6,
            dependencies: 1,
            recovery: 1,
            tiles: 1,
        });
        match cursor.advance(body, &mut resolver, &mut budget).unwrap() {
            BodyStyleMarkerStep::Span(s) => {
                assert_eq!(s.body, body.key());
                assert!(s.value.source.end > s.value.source.start);
            }
            BodyStyleMarkerStep::BudgetExhausted => {}
            BodyStyleMarkerStep::Complete => break,
        }
    }
    let update = d.append("\n[r]: /ok\n").unwrap();
    assert!(update.body_updates.is_empty());
    assert_eq!(
        cursor.advance(
            v(&d),
            &mut resolver,
            &mut Budget::new(WorkLimit::unlimited())
        ),
        Err(LayoutError::StaleContinuation)
    );
    let body = v(&d);
    let mut cursor = body.style_markers(body.range()).unwrap();
    let mut resolver = body.link_resolver();
    let mut spans = 0;
    loop {
        let mut budget = Budget::new(WorkLimit {
            queries: 1,
            objects: 1,
            bytes: 6,
            dependencies: 1,
            recovery: 1,
            tiles: 1,
        });
        match cursor.advance(body, &mut resolver, &mut budget).unwrap() {
            BodyStyleMarkerStep::Span(s) => {
                assert_eq!(s.body, body.key());
                spans += 1;
            }
            BodyStyleMarkerStep::BudgetExhausted => {}
            BodyStyleMarkerStep::Complete => break,
        }
    }
    assert!(spans > 0);
}
