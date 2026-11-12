use mdstream::{BlockId, BlockKind, Budget, Completion, Document, WorkLimit};

#[test]
fn content_predecessor_matches_independent_blocks_at_every_source_cut() {
    for source in [
        "",
        "plain\n\n[a]: /a\n\n[^n]: note\n",
        "[later]:\n /url\n",
        "```rust\n```\n\n[a]: /url\n",
        "> a\n> b\n> ===\n",
    ] {
        for split in 0..=source.len() {
            let mut doc = Document::new();
            doc.append(&source[..split]).unwrap();
            doc.append(&source[split..]).unwrap();
            for finished in [false, true] {
                if finished {
                    doc.finish();
                }
                for end in 0..=source.len() {
                    let expected = doc
                        .blocks()
                        .iter()
                        .enumerate()
                        .rev()
                        .find(|(_, block)| {
                            block.source.start < end
                                && !matches!(
                                    block.kind,
                                    BlockKind::Blank
                                        | BlockKind::Definition { .. }
                                        | BlockKind::FootnoteDefinition { .. }
                                )
                        })
                        .map(|(id, _)| BlockId(id));
                    let mut budget = Budget::new(WorkLimit::unlimited());
                    assert_eq!(
                        doc.previous_content_block(end, &mut budget),
                        Ok(expected),
                        "{source:?} split {split} cut {end} finish {finished}"
                    );
                    assert_eq!(budget.used().bytes, 0);
                    let mut zero = Budget::new(WorkLimit::default());
                    assert_eq!(
                        doc.previous_content_block(end, &mut zero),
                        Err(Completion::BudgetExhausted)
                    );
                }
            }
            let stats = doc.content_block_index_stats();
            let expected = doc
                .blocks()
                .iter()
                .filter(|block| {
                    !matches!(
                        block.kind,
                        BlockKind::Blank
                            | BlockKind::Definition { .. }
                            | BlockKind::FootnoteDefinition { .. }
                    )
                })
                .count();
            assert_eq!(stats.entries, expected);
            assert_eq!(stats.key_bytes, expected * std::mem::size_of::<BlockId>());
            assert!(stats.mutations <= doc.blocks().len() as u64 * 2);
        }
    }
}
