use mdstream::{BlockId, Budget, CodeInfoStep, Document, LayoutError, WorkLimit};
fn micro() -> Budget {
    Budget::new(WorkLimit {
        queries: 1,
        objects: 1,
        bytes: 6,
        ..Default::default()
    })
}
fn text(doc: &Document, cap: usize) -> String {
    let mut cursor = doc.code_info_prefix(BlockId(0), cap).unwrap();
    let mut out = String::new();
    for _ in 0..10000 {
        let position = cursor.source_position();
        let mut zero = Budget::new(WorkLimit::default());
        let step = cursor.next(doc, &mut zero).unwrap();
        assert_eq!(cursor.source_position(), position);
        assert_eq!(zero.used(), WorkLimit::default());
        if step == CodeInfoStep::Complete {
            return out;
        }
        assert_eq!(step, CodeInfoStep::BudgetExhausted);
        match cursor.next(doc, &mut micro()).unwrap() {
            CodeInfoStep::Atom { utf8, len, source } => {
                assert!(source.end <= doc.block(BlockId(0)).unwrap().source.end);
                out.push_str(std::str::from_utf8(&utf8[..usize::from(len)]).unwrap());
            }
            CodeInfoStep::Complete => return out,
            CodeInfoStep::BudgetExhausted => {}
        }
    }
    panic!("owned code info did not progress")
}
#[test]
fn zero_micro_entity_probe_and_utf8_source_prefix_caps_use_the_original_decoder() {
    let mut doc = Document::new();
    doc.append("~~~ a\\+&ouml;中\nBODY\n~~~\n").unwrap();
    doc.finish();
    for (cap, expected) in [
        (0, ""),
        (1, "a"),
        (2, "a\\"),
        (3, "a+"),
        (4, "a+&"),
        (9, "a+ö"),
        (10, "a+ö"),
        (11, "a+ö"),
        (12, "a+ö中"),
        (256, "a+ö中"),
    ] {
        assert_eq!(text(&doc, cap), expected, "source cap {cap}");
    }
}
#[test]
fn wrong_and_stale_document_reject_before_budget_and_do_not_advance_entity_probe() {
    let mut doc = Document::new();
    doc.append("~~~ &ouml;\n").unwrap();
    let mut cursor = doc.code_info_prefix(BlockId(0), 256).unwrap();
    let mut one = Budget::new(WorkLimit {
        queries: 1,
        objects: 1,
        bytes: 1,
        ..Default::default()
    });
    assert_eq!(
        cursor.next(&doc, &mut one).unwrap(),
        CodeInfoStep::BudgetExhausted
    );
    let position = cursor.source_position();
    let other = Document::new();
    let mut budget = micro();
    assert_eq!(
        cursor.next(&other, &mut budget),
        Err(LayoutError::WrongDocument)
    );
    assert_eq!(budget.used(), WorkLimit::default());
    assert_eq!(cursor.source_position(), position);
    doc.append("body").unwrap();
    assert_eq!(
        cursor.next(&doc, &mut budget),
        Err(LayoutError::StaleContinuation)
    );
    assert_eq!(budget.used(), WorkLimit::default());
    assert_eq!(cursor.source_position(), position);
}
