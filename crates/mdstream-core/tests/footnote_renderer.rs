use mdstream::*;

fn view(d: &Document) -> BodyView<'_> {
    d.footnote_body(FootnoteId(0)).unwrap()
}

#[test]
fn hosted_read_rejects_wrong_body_document_and_revision_before_callback() {
    let mut d = Document::new();
    d.append("[^a]: first\n[^b]: second\n").unwrap();
    let a = view(&d);
    let b = d.footnote_body(FootnoteId(1)).unwrap();
    assert_eq!(
        a.with_renderer(b.key(), d.revision(), |_| panic!("foreign body read")),
        Err::<(), _>(LayoutError::InvalidRange)
    );
    let mut other = Document::new();
    other.append("[^a]: first\n").unwrap();
    assert_eq!(
        a.with_renderer(view(&other).key(), d.revision(), |_| panic!(
            "foreign host read"
        )),
        Err::<(), _>(LayoutError::WrongDocument)
    );
    assert_eq!(
        a.with_renderer(a.key(), d.revision() - 1, |_| panic!("stale read")),
        Err::<(), _>(LayoutError::StaleContinuation)
    );
    assert_eq!(
        a.with_renderer(a.key(), d.revision(), |native| native.source().to_owned())
            .unwrap(),
        "first"
    );
}

#[test]
fn renderer_bridge_uses_actual_local_facts_and_retains_stable_height() {
    let mut d = Document::new();
    d.enable_display_shape_effects().unwrap();
    d.append(&format!("[^a]: {}", "x".repeat(40000))).unwrap();
    let a = view(&d);
    let owner = a.key();
    let mut clock = BodyRendererClock::new(a);
    let mut heights = HeightIndex::new(1).unwrap();
    a.with_renderer(owner, d.revision(), |native| {
        heights.rebuild(native).unwrap()
    })
    .unwrap();
    let ticket = heights.ticket(TileId(0)).unwrap();
    heights.measure(ticket, 99).unwrap();
    let before = heights.ticket(TileId(0)).unwrap();
    let mut update = d.append("tail").unwrap();
    update.merge(d.append("more").unwrap()).unwrap();
    let a = view(&d);
    let translated = clock.apply(a, &update).unwrap().unwrap();
    assert_eq!(translated.body, owner);
    assert_eq!(
        translated.value.display_shape,
        update.body_updates[0].facts().value.display_shape
    );
    a.with_renderer(owner, d.revision(), |native| {
        heights.apply(native, &translated.value).unwrap()
    })
    .unwrap();
    assert_eq!(heights.base_height(TileId(0)), Some(99));
    heights.measure(before, 99).unwrap();
    assert!(clock.apply(a, &update).unwrap().is_none());
}

#[test]
fn no_body_delta_host_definition_still_expires_the_native_view_ticket() {
    let mut d = Document::new();
    d.enable_display_shape_effects().unwrap();
    d.append("[^a]: [L][r]\n# outside\n").unwrap();
    let a = view(&d);
    let owner = a.key();
    let mut clock = BodyRendererClock::new(a);
    let mut h = HeightIndex::new(1).unwrap();
    a.with_renderer(owner, d.revision(), |n| h.rebuild(n).unwrap())
        .unwrap();
    let t = h.ticket(TileId(0)).unwrap();
    h.measure(t, 99).unwrap();
    let old = h.ticket(TileId(0)).unwrap();
    let update = d.append("\n[r]: /late\n").unwrap();
    assert!(update.body_updates.is_empty());
    let a = view(&d);
    let u = clock.apply(a, &update).unwrap().unwrap();
    a.with_renderer(owner, d.revision(), |n| h.apply(n, &u.value).unwrap())
        .unwrap();
    assert_eq!(h.base_height(TileId(0)), Some(99));
    assert!(h.measure(old, 7).is_err());
}

#[test]
fn owned_body_target_decoder_keeps_original_and_projected_addresses_under_micro_budgets() {
    let mut d = Document::new();
    d.append("[^a]: [L][r]\n[^b]: [L](https://local.test/a&amp;b \"local\")\n# root\n\n[r]: https://host.test/x&amp;y \"host title\"\n").unwrap();
    d.finish();
    let a = view(&d);
    let b = d.footnote_body(FootnoteId(1)).unwrap();
    let mut task = a.link_destination_task(a.qualify(LinkId(0))).unwrap();
    let mut zero = Budget::new(WorkLimit::default());
    assert!(matches!(
        task.advance(a, &mut zero).unwrap(),
        Err(Completion::BudgetExhausted)
    ));
    assert_eq!(zero.used(), WorkLimit::default());
    assert_eq!(
        task.advance(b, &mut zero).err(),
        Some(LayoutError::InvalidRange)
    );
    assert_eq!(zero.used(), WorkLimit::default());
    let mut other = Document::new();
    other.append("[^a]: x\n").unwrap();
    assert_eq!(
        task.advance(view(&other), &mut zero).err(),
        Some(LayoutError::WrongDocument)
    );
    let mut decoded = String::new();
    let mut charges = WorkLimit::default();
    for _ in 0..1000 {
        let mut budget = Budget::new(WorkLimit {
            queries: 1,
            objects: 1,
            dependencies: 1,
            bytes: 6,
            ..Default::default()
        });
        let step = task.advance(a, &mut budget).unwrap();
        let done = match step {
            Ok(Some(chunk)) => {
                assert!(
                    matches!(chunk.source,BodyAddress::Original {document,..} if document==d.id())
                );
                decoded.push_str(chunk.text);
                false
            }
            Ok(None) => true,
            Err(_) => false,
        };
        charges.bytes += budget.used().bytes;
        charges.objects += budget.used().objects;
        if done {
            break;
        }
    }
    assert_eq!(decoded, "https://host.test/x&y");
    assert!(charges.bytes >= decoded.len());
    assert!(
        matches!(a.link_title(a.qualify(LinkId(0))).unwrap(),Some(BodyAddress::Original {document,..}) if document==d.id())
    );
    assert!(
        matches!(b.link_title(b.qualify(LinkId(0))).unwrap(),Some(BodyAddress::Projected(span)) if span.body==b.key())
    );
    let mut local = b.link_destination_task(b.qualify(LinkId(0))).unwrap();
    let mut decoded = String::new();
    let mut budget = Budget::new(WorkLimit::unlimited());
    while let Ok(Some(chunk)) = local.advance(b, &mut budget).unwrap() {
        assert!(matches!(chunk.source,BodyAddress::Projected(span) if span.body==b.key()));
        decoded.push_str(chunk.text);
    }
    assert_eq!(decoded, "https://local.test/a&b");
}

#[test]
fn owned_fragment_uses_the_existing_decoder_and_retires_each_real_index_record() {
    let mut d = Document::new();
    d.append("# First\n# 中🙂\n").unwrap();
    d.finish();
    let mut index = HeadingAnchors::new(&d);
    index
        .advance(&d, &mut Budget::new(WorkLimit::unlimited()))
        .unwrap();
    let mut request = index
        .fragment_owned(std::sync::Arc::new("#%E4%B8%AD".to_owned()))
        .unwrap();
    let mut result = FragmentStep::BudgetExhausted;
    for _ in 0..100 {
        result = request
            .advance(
                &index,
                &d,
                &mut Budget::new(WorkLimit {
                    queries: 1,
                    objects: 1,
                    bytes: 3,
                    ..Default::default()
                }),
            )
            .unwrap();
        if result != FragmentStep::BudgetExhausted {
            break;
        }
    }
    assert!(matches!(
        result,
        FragmentStep::Resolved(AnchorTarget { source: 8, .. })
    ));
    let mut other = Document::new();
    other.finish();
    let foreign = HeadingAnchors::new(&other);
    let mut zero = Budget::new(WorkLimit::default());
    assert_eq!(
        request.advance(&foreign, &other, &mut zero),
        Err(LayoutError::WrongDocument)
    );
    assert_eq!(zero.used(), WorkLimit::default());
    assert_eq!(index.retire(&mut zero), Completion::BudgetExhausted);
    assert_eq!(index.indexed_headings(), 2);
    let mut steps = 0;
    loop {
        let mut budget = Budget::new(WorkLimit {
            queries: 1,
            objects: 1,
            ..Default::default()
        });
        let done = index.retire(&mut budget);
        steps += budget.used().objects;
        if done == Completion::Covered {
            break;
        }
        assert!(steps < 100);
    }
    assert!(steps > 10);
    assert_eq!(index.indexed_headings(), 0);
}
