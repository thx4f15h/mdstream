#[path = "support/code_replay_changes.rs"]
mod code_changes;
#[path = "support/style_confirmation_changes.rs"]
mod style_changes;
use mdstream::*;
#[path = "support/container_eligibility_cases.rs"]
mod corpus;
#[path = "support/nested_changes.rs"]
mod nested_changes;
#[allow(dead_code)]
#[path = "support/reference_boundary_hashes.rs"]
mod reference_hashes;
#[allow(dead_code)]
#[path = "support/shortcut_hashes.rs"]
mod shortcut_hashes;
#[path = "support/snapshot.rs"]
mod support;

include!("fixtures/container_eligibility_baseline.rs");
include!("fixtures/container_component_gap_changes.rs");
include!("fixtures/container_title_legacy_changes.rs");
#[path = "support/component_gap_model.rs"]
mod component_model;
#[path = "support/container_title_changes.rs"]
mod title_model;

#[test]
fn container_eligibility_preserves_head_snapshots_and_every_old_work_counter() {
    let mut index = 0;
    for (case, source) in corpus::cases().iter().enumerate() {
        for end in corpus::ends(source) {
            for bytewise in [false, true] {
                let mut d = Utf8Document::new();
                let bytes = &source.as_bytes()[..end];
                for chunk in bytes.chunks(if bytewise { 1 } else { bytes.len().max(1) }) {
                    d.append(chunk).unwrap();
                }
                d.finish();
                let exact_change = COMPONENT_GAP_CHANGES
                    .iter()
                    .find(|entry| entry.0.as_bytes() == bytes);
                let title_change = LEGACY_TITLE_CHANGES
                    .iter()
                    .find(|entry| entry.0.as_bytes() == bytes);
                let expected = if let Some((_, changes, _)) = title_change {
                    let doc = d.document();
                    let cr = usize::from(doc.source().contains('\r'));
                    let title = Span::new(9, 14 + cr);
                    assert_eq!(
                        doc.text(title),
                        Some(if cr == 0 { "x\n> y" } else { "x\r\n> y" })
                    );
                    title_model::assert_plain(
                        doc,
                        LinkId(0),
                        Span::new(3, 4),
                        Span::new(2, 16 + cr),
                        Span::new(6, 7),
                        title,
                        &[Span::new(11 + cr, 13 + cr)],
                    );
                    changes[usize::from(bytewise)]
                } else if let Some((_, changes, public)) = exact_change {
                    if *public {
                        component_model::assert_direct(d.document(), "a", 0);
                    } else {
                        assert_eq!(
                            corpus::hash(&format!("{:?}", support::snapshot(d.document()))),
                            shortcut_hashes::container(case, end, bytewise, BASELINE[index].0)
                        );
                        assert_eq!(d.document().link_destination(LinkId(0)), None);
                    }
                    changes[usize::from(bytewise)]
                } else {
                    BASELINE[index]
                };
                let expected = (
                    reference_hashes::container(
                        bytes,
                        case,
                        end,
                        bytewise,
                        shortcut_hashes::source(
                            d.document().source(),
                            bytewise,
                            shortcut_hashes::container(case, end, bytewise, expected.0),
                        ),
                    ),
                    nested_changes::work(d.document().source(), expected.1),
                );
                let expected = style_changes::pair(
                    d.document().source(),
                    code_changes::pair(d.document().source(), expected),
                );
                assert_eq!(
                    (
                        corpus::hash(&format!("{:?}", support::snapshot(d.document()))),
                        corpus::hash(&corpus::old_work(d.document().work()))
                    ),
                    expected,
                    "case={case} end={end} bytewise={bytewise}"
                );
                index += 1;
            }
        }
    }
    assert_eq!(index, BASELINE.len());
}

#[test]
fn container_eligibility_charges_only_consumed_payload_and_virtual_visits() {
    for (source, bytes) in [
        ("> a\n> b\n", 4),
        (">\ta\n>\tb\n", 8),
        (" >\ta\n", 3),
        ("  >\ta\n", 2),
        ("- a\n  b\n", 4),
        ("> [r\n> bad\n", 7),
        ("> [r]: /u\n", 0),
        ("> [r\n> label]: /u\n", 0),
        ("> # head\n", 0),
        ("> ```\n> text\n> ```\n", 0),
        ("> $$\n> text\n> $$\n", 0),
        ("> [^f]: text\n", 0),
        ("> a|b\n> -|-\n", 0),
    ] {
        let mut d = Document::new();
        for ch in source.chars() {
            d.append(ch.encode_utf8(&mut [0; 4])).unwrap();
        }
        d.finish();
        assert_eq!(d.work().container_eligibility_bytes, bytes, "{source:?}");
        assert_eq!(d.work().root_eligibility_bytes, 0, "{source:?}");
    }
}

#[test]
fn container_eligibility_appends_do_not_revisit_body_scope_or_old_references() {
    let mut d = Document::new();
    for i in 0..20_000 {
        d.append(&format!("[r{i}]: /u\n")).unwrap();
    }
    d.append("\n").unwrap();
    let references = d.references().to_vec();
    d.append(&format!(
        "{}plain `x``{}",
        "> ".repeat(500),
        "a".repeat(1_000_000)
    ))
    .unwrap();
    let before = d.work();
    for _ in 0..1000 {
        d.append("b").unwrap();
    }
    let after = d.work();
    assert_eq!(
        after.container_eligibility_bytes - before.container_eligibility_bytes,
        1000
    );
    assert_eq!(
        after.container_eligibility_steps - before.container_eligibility_steps,
        1000
    );
    assert_eq!(after.inline_bytes - before.inline_bytes, 1000);
    assert_eq!(after.root_eligibility_bytes, before.root_eligibility_bytes);
    assert_eq!(after.tree_visits, before.tree_visits);
    assert_eq!(after.classified_bytes, before.classified_bytes);
    assert_eq!(after.container_steps, before.container_steps);
    assert_eq!(d.references(), references);
    d.finish();
    assert_eq!(d.references(), references);
}

#[test]
fn container_observation_preserves_public_logical_paragraph_first_leaf_and_mapping() {
    let mut d = Document::new();
    d.append("> *中\n").unwrap();
    let first = d.block_at(0).unwrap();
    let scope = d.block(first).unwrap().container;
    d.append("> 🙂* [x](u)\n").unwrap();
    let end = d.source().len();
    let mut active = ParagraphFoldCandidates::new(&d, Span::new(0, end), 0).unwrap();
    let mut open = Vec::new();
    assert_eq!(
        active.advance(&d, &mut Budget::new(WorkLimit::unlimited()), &mut open),
        Ok(Completion::Covered)
    );
    assert_eq!(open.len(), 1);
    assert_eq!(open[0].block, first);
    assert_eq!(open[0].source, Span::new(0, end));
    assert!(open[0].open);
    d.append(">\n> new\n").unwrap();
    d.finish();
    let mut candidates =
        ParagraphFoldCandidates::new(&d, Span::new(0, d.source().len()), 0).unwrap();
    let mut out = Vec::new();
    assert_eq!(
        candidates.advance(&d, &mut Budget::new(WorkLimit::unlimited()), &mut out),
        Ok(Completion::Covered)
    );
    assert_eq!(out.len(), 2);
    assert_eq!(out[0].block, first);
    assert_eq!(out[0].container, scope);
    assert_eq!(out[0].source, Span::new(0, end));
    assert!(!out[0].open);
    assert_ne!(out[1].block, first);
    assert_eq!(out[1].container, scope);
    assert_eq!(d.link_info(LinkId(0)).unwrap().label, Span::new(16, 17));
    for (at, ch) in d.source().char_indices() {
        assert_eq!(
            d.runs_in(Span::new(at, at + ch.len_utf8()))
                .unwrap()
                .map(|r| r.text)
                .collect::<String>(),
            ch.to_string()
        );
    }
}
