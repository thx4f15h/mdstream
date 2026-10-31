use mdtui3::{Budget, Completion, Document, FootnoteReferenceId, FootnoteTarget, WorkLimit};

fn main() {
    let mut doc = Document::new();
    doc.append("See [^note] and again [^note].\n").unwrap();
    let mut budget = Budget::new(WorkLimit::unlimited());
    let first = doc
        .resolve_footnote(FootnoteReferenceId(0), &mut budget)
        .unwrap()
        .unwrap();
    assert_eq!(first.number, 1);
    assert_eq!(first.target, FootnoteTarget::Unresolved);
    let before = doc.work();
    doc.append("\n[^note]: 原始 **Markdown**，按预算读取。\n")
        .unwrap();
    assert_eq!(doc.work().inline_bytes, before.inline_bytes);
    doc.finish();
    let resolved = doc
        .resolve_footnote(FootnoteReferenceId(1), &mut budget)
        .unwrap()
        .unwrap();
    assert_eq!(first.symbol, resolved.symbol);
    assert_eq!(resolved.number, 1);
    assert!(matches!(resolved.target, FootnoteTarget::Defined(_)));

    let mut content = doc.footnote_content(resolved.symbol).unwrap();
    let mut output = String::new();
    let mut frames = 0;
    let mut bytes = 0;
    loop {
        frames += 1;
        let mut budget = Budget::new(WorkLimit {
            queries: 1,
            objects: 1,
            bytes: 8,
            ..WorkLimit::default()
        });
        loop {
            match content.advance(&mut budget) {
                Ok(Some(chunk)) => output.push_str(chunk.text),
                Ok(None) => {
                    bytes += budget.used().bytes;
                    assert_eq!(output, "原始 **Markdown**，按预算读取。");
                    println!(
                        "footnote #{}: {output}; {frames} frames / {bytes} bytes",
                        resolved.number
                    );
                    return;
                }
                Err(Completion::BudgetExhausted) => break,
                Err(other) => panic!("unexpected completion: {other:?}"),
            }
        }
        bytes += budget.used().bytes;
    }
}
