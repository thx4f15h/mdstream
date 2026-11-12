//! Owned physical sources awaiting either confirmation or ordinary replay.
//! No text, historical containers, or published inline objects are copied here.
use crate::{BlockId, ContainerId, MAX_LABEL_BYTES, MAX_LINK_SUFFIX_BYTES, Span};

// Each open label/title source contributes at least one real byte to its
// phase's bound. Add the closing-label, target and final physical sources.
const MAX_DEFERRED_SOURCES: usize = MAX_LABEL_BYTES + MAX_LINK_SUFFIX_BYTES + 4;

// Spans alone do not authorize consumption. This non-Clone right moves from a
// header to a definition ledger and is retired by exactly one consuming path.
pub(crate) struct SourceOwner(());

impl SourceOwner {
    pub fn new() -> Self {
        Self(())
    }
}

pub(crate) struct DeferredFragment {
    pub source: Span,
    // Always excludes consumed prefix bytes, including the partially used tab.
    pub raw: Span,
    pub virtual_spaces: u8,
    pub owner: SourceOwner,
}

impl DeferredFragment {
    pub fn from_payload(
        source: Span,
        payload: Span,
        virtual_spaces: u8,
        owner: SourceOwner,
    ) -> Self {
        Self {
            source,
            raw: Span::new(payload.start + usize::from(virtual_spaces > 0), payload.end),
            virtual_spaces,
            owner,
        }
    }
}

pub(crate) struct DeferredSources {
    pub block: BlockId,
    pub scope: Option<ContainerId>,
    fragments: Vec<DeferredFragment>,
}

impl DeferredSources {
    pub fn new(block: BlockId, scope: Option<ContainerId>, first: DeferredFragment) -> Self {
        let mut sources = Self {
            block,
            scope,
            fragments: Vec::new(),
        };
        assert!(
            sources.push(first).is_ok(),
            "first deferred physical source is valid"
        );
        sources
    }

    // O(1) admission: never revisit any previous payload or active ancestor.
    pub fn push(&mut self, fragment: DeferredFragment) -> Result<(), DeferredFragment> {
        let ordered = self
            .fragments
            .last()
            .is_none_or(|last| last.source.end == fragment.source.start);
        if !ordered
            || fragment.source.start >= fragment.source.end
            || fragment.raw.start < fragment.source.start
            || fragment.raw.end != fragment.source.end
            || fragment.raw.start > fragment.raw.end
            || fragment.virtual_spaces > 3
            || (fragment.virtual_spaces > 0 && fragment.raw.start == fragment.source.start)
            || self.fragments.len() >= MAX_DEFERRED_SOURCES
        {
            return Err(fragment);
        }
        self.fragments.push(fragment);
        Ok(())
    }

    pub fn start(&self) -> usize {
        self.fragments[0].source.start
    }

    // Confirmation consumes the entire ledger without feeding ordinary Inline.
    pub fn confirm(self) -> Span {
        Span::new(self.start(), self.fragments.last().unwrap().source.end)
    }

    pub fn into_fragments(self) -> impl Iterator<Item = DeferredFragment> {
        self.fragments.into_iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::*;

    mod support {
        use crate as mdstream;
        include!("../tests/support/snapshot.rs");
    }

    fn parse(source: &str) -> Document {
        let mut doc = Document::new();
        doc.append(source).unwrap();
        doc.finish();
        doc
    }

    // Only a physical-source producer: no syntax or ordinary parsing is
    // implemented here. Probe, admission, flush and commit are production code.
    fn stage(
        doc: &mut Document,
        text: &str,
        split: usize,
        columns: &mut Option<usize>,
    ) -> (Update, bool) {
        let start = doc.source.len();
        let mut update = Update::default();
        doc.begin_update(start, &mut update);
        doc.revision += 1;
        update.revision = doc.revision;
        for chunk in [&text[..split], &text[split..]] {
            let old = doc.source.len();
            doc.source.push_str(chunk);
            doc.extend_tiles(old);
            doc.work.input_bytes += chunk.len() as u64;
        }
        let span = Span::new(start, doc.source.len());
        update.appended.end = span.end;
        update.dirty.push(span);
        update.change(span, ChangeKind::Text);
        let scope = doc.definition_candidate.as_ref().unwrap().sources.scope;
        let probe = doc.containers.continuation_source(
            &doc.source,
            span,
            scope,
            false,
            *columns,
            &mut doc.work,
        );
        let retained = if let Some((_, raw, virtual_spaces)) = probe {
            let mut summary = table::HeaderSummary::default();
            summary.feed(&doc.source[raw.range()], &mut doc.work);
            *columns = summary.columns();
            let fragment = DeferredFragment {
                source: span,
                raw,
                virtual_spaces,
                owner: SourceOwner::new(),
            };
            doc.definition_candidate
                .as_mut()
                .unwrap()
                .sources
                .push(fragment)
                .is_ok()
        } else {
            false
        };
        if !retained {
            doc.flush_definition_candidate(&mut update);
            doc.commit_line(span, true, true, &mut update);
        }
        doc.line_start = span.end;
        doc.complete_update(&mut update);
        (update, retained)
    }

    fn assert_mapping_and_budget(doc: &Document) {
        let full = Span::new(0, doc.source.len());
        assert_eq!(
            doc.runs_in(full)
                .unwrap()
                .map(|r| r.text)
                .collect::<String>(),
            doc.source
        );
        let visible = doc
            .display_runs_in(full)
            .unwrap()
            .filter(|r| !r.style.contains(Style::HIDDEN))
            .map(|r| r.text)
            .collect::<String>();
        let sliced = doc
            .source
            .char_indices()
            .flat_map(|(at, c)| {
                doc.display_runs_in(Span::new(at, at + c.len_utf8()))
                    .unwrap()
            })
            .filter(|r| !r.style.contains(Style::HIDDEN))
            .map(|r| r.text)
            .collect::<String>();
        assert_eq!(sliced, visible);
        let mut cursor = doc.display_cursor(full).unwrap();
        let mut text = String::new();
        while cursor.source_position() < full.end {
            let at = cursor.source_position();
            assert_eq!(
                cursor
                    .next(doc, &mut Budget::new(WorkLimit::default()))
                    .unwrap(),
                DisplayStep::BudgetExhausted
            );
            assert_eq!(cursor.source_position(), at);
            let mut budget = Budget::new(WorkLimit {
                queries: 1,
                objects: 1,
                bytes: 6,
                ..Default::default()
            });
            let run = match cursor.next(doc, &mut budget).unwrap() {
                DisplayStep::Run(run) => run,
                DisplayStep::BudgetExhausted => {
                    assert_eq!(cursor.source_position(), at);
                    assert!(
                        budget.used().queries > 0,
                        "metadata must make paid progress"
                    );
                    continue;
                }
                DisplayStep::Complete => panic!("source remains"),
            };
            assert_eq!(run.source.start, at);
            if !run.style.contains(Style::HIDDEN) {
                text.push_str(run.text);
            }
            assert!(budget.used().bytes <= 6);
        }
        assert_eq!(text, visible);
    }

    // Independent ordinary control: commit only the first physical line with
    // candidate admission disabled, then use the public append/finish pipeline.
    // Raw source, headers, containers and Inline are otherwise identical.
    fn ordinary_control(source: &str) -> Document {
        let first_end = source.find('\n').map_or(source.len(), |end| end + 1);
        let mut doc = Document::new();
        let mut update = Update::default();
        doc.begin_update(0, &mut update);
        doc.revision += 1;
        update.revision = doc.revision;
        doc.source.push_str(&source[..first_end]);
        doc.extend_tiles(0);
        doc.work.input_bytes += first_end as u64;
        doc.commit_line_mode(Span::new(0, first_end), true, true, false, &mut update);
        doc.line_start = first_end;
        doc.complete_update(&mut update);
        doc.append(&source[first_end..]).unwrap();
        doc.finish();
        doc
    }

    #[test]
    fn multiline_failures_match_actual_ordinary_structures_headers_and_lazy_paths() {
        let mut cases = 0;
        for (first, next) in [
            ("", ""),
            ("> ", "> "),
            ("- ", "  "),
            ("> - ", ">   "),
            ("- > ", "  > "),
            ("- - ", "    "),
            (">\t", ">\t"),
            ("- - ", "\t"),
            ("- > ", "\t>\t"),
            ("> > ", ">\t>\t"),
        ] {
            for head in [
                "[a", "[a|b", "[a\\|b", "[a`x|y", "[a$x|y", "[a*x|y", "[a**x",
            ] {
                for trigger in [
                    "===",
                    "---",
                    "***",
                    "# h",
                    "~~~",
                    "```",
                    "$$",
                    "    code",
                    "\tcode",
                    "",
                    "> new",
                    "- sibling",
                    "[r]: /url",
                    "b[bad]: /url",
                    "b] : /url",
                    "b]: <bad",
                    "b]: /url 'bad",
                    "unclosed",
                ] {
                    let source = format!("{first}{head}\n{next}{trigger}\n{next}ordinary\n");
                    let actual = parse(&source);
                    let control = ordinary_control(&source);
                    assert_eq!(
                        support::snapshot(&actual),
                        support::snapshot(&control),
                        "{source:?}"
                    );
                    cases += 1;
                }
                // A real middle header, a closing-label header without target,
                // and escaped pipes must keep the exact table grammar/priority.
                for row in ["b|c", "b|c]:", "`b|c`", "$b|c$", "*b|c*", "b\\|c"] {
                    for delimiter in ["-|-", " -|-", "  -|-", "   -|-", "    -|-", "\t-|-", "==="] {
                        let source = format!("{first}{head}\n{next}{row}\n{next}{delimiter}\n");
                        let actual = parse(&source);
                        // Some four-column delimiters legally become targets
                        // after b|c]:; those cases test definitions elsewhere.
                        if actual.references().iter().any(|r| r.destination.is_some()) {
                            continue;
                        }
                        assert_eq!(
                            support::snapshot(&actual),
                            support::snapshot(&ordinary_control(&source)),
                            "{source:?}"
                        );
                        assert_mapping_and_budget(&actual);
                        cases += 1;
                    }
                }
                // Missing prefixes retain the real ordinary lazy eligibility.
                for tail in [
                    "b]: /url\n",
                    "plain\n===\n",
                    "> new\n",
                    "- sibling\n",
                    "[r]: /url\n",
                ] {
                    let source = format!("{first}{head}\n{tail}");
                    let actual = parse(&source);
                    if actual.references().iter().any(|r| r.destination.is_some()) {
                        continue;
                    }
                    assert_eq!(
                        support::snapshot(&actual),
                        support::snapshot(&ordinary_control(&source)),
                        "{source:?}"
                    );
                    cases += 1;
                }
            }
            // A >1024-byte delimiter must see the last real middle header,
            // never the first candidate's header or a synthetic whole label.
            let source = format!(
                "{first}[a|b\n{next}x|y|z\n{next}{}|{}|{}\n",
                "-".repeat(1100),
                "-".repeat(1100),
                "-".repeat(1100)
            );
            assert_eq!(
                support::snapshot(&parse(&source)),
                support::snapshot(&ordinary_control(&source)),
                "long delimiter {first:?}/{next:?}"
            );
        }
        eprintln!("multiline ordinary-control comparisons: {cases}");
    }

    include!("../tests/fixtures/commonmark_reference_definitions.rs");
    include!("../tests/fixtures/commonmark_reference_labels.rs");

    #[test]
    fn multiline_official_structure_and_source_maps_change_only_after_confirmation() {
        for (number, source, old_blocks, old_links, new_blocks, new_links) in [
            (
                208,
                REFERENCE_DEFINITION_CASES
                    .iter()
                    .find(|c| c.0 == 208)
                    .unwrap()
                    .1,
                1,
                1,
                2,
                0,
            ),
            (
                541,
                REFERENCE_LABEL_CASES.iter().find(|c| c.0 == 541).unwrap().1,
                3,
                2,
                3,
                1,
            ),
        ] {
            // This is the actual ordinary path used by the former unsupported
            // definition, with only first-line admission disabled. No projection.
            let control = ordinary_control(source);
            let doc = parse(source);
            assert_eq!(
                (control.blocks.len(), control.links.len()),
                (old_blocks, old_links)
            );
            assert_eq!((doc.blocks.len(), doc.links.len()), (new_blocks, new_links));
            assert!(matches!(doc.blocks[0].kind, BlockKind::Definition { .. }));
            assert_eq!(doc.blocks[0].source.start, 0);
            assert_eq!(doc.blocks.last().unwrap().source.end, source.len());
            for at in (0..source.len()).filter(|&at| source.is_char_boundary(at)) {
                let id = doc.block_at(at).unwrap();
                assert!(doc.blocks[id.0].source.range().contains(&at));
                for run in doc
                    .runs_in(Span::new(
                        at,
                        at + source[at..].chars().next().unwrap().len_utf8(),
                    ))
                    .unwrap()
                {
                    assert_eq!(doc.text(run.source), Some(run.text));
                }
            }
            eprintln!(
                "official {number}: ordinary blocks={:?} links={:?}; confirmed blocks={:?} links={:?}",
                control.blocks, control.link_info, doc.blocks, doc.link_info
            );
        }
    }

    #[test]
    fn multiline_pipe_header_context_does_not_hide_real_tab_code() {
        for (first, next) in [("> - ", ">   "), ("- ", "  "), ("- - ", "\t")] {
            let source = format!("{first}[a|b\n{next}\tb]: /url\n");
            let control = ordinary_control(&source);
            if control
                .blocks
                .iter()
                .any(|block| matches!(block.kind, BlockKind::Code { .. }))
            {
                let doc = parse(&source);
                assert!(
                    doc.references
                        .entries
                        .iter()
                        .all(|r| r.destination.is_none()),
                    "{source:?}"
                );
                assert_eq!(support::snapshot(&doc), support::snapshot(&control));
            }
        }
    }

    #[test]
    fn deferred_multiple_sources_replay_through_ordinary_table_path() {
        for (first, next) in [
            ("", ""),
            ("> ", "> "),
            ("- ", "  "),
            (">\t", ">\t"),
            ("- > ", "\t>\t"),
        ] {
            for label in ["r", "r|s"] {
                let head = format!("{first}[{label}]:\n");
                let rows = [format!("{next}<bad 中🙂\n"), format!("{next}b|c\n")];
                let trigger = format!("{next}-|-\n");
                let source = format!("{head}{}{}{trigger}", rows[0], rows[1]);
                let expected = support::snapshot(&parse(&source));
                let mut doc = Document::new();
                doc.append(&head).unwrap();
                let old_block = doc.blocks[0].clone();
                let mut columns = None;
                for row in &rows {
                    assert!(stage(&mut doc, row, 0, &mut columns).1, "{source:?}");
                    assert_eq!(doc.blocks.len(), 1);
                    assert_eq!(doc.blocks[0], old_block);
                    assert!(doc.links.is_empty());
                    assert_eq!(doc.source_fold_limit(), 0);
                }
                assert_eq!(columns, Some(2));
                assert!(!stage(&mut doc, &trigger, 0, &mut columns).1);
                assert!(doc.definition_candidate.is_none());
                doc.finish();
                assert_eq!(doc.tables.len(), 1);
                assert_eq!(
                    doc.tables[0].rows()[0].source.start,
                    head.len() + rows[0].len()
                );
                assert_eq!(support::snapshot(&doc), expected, "{source:?}");
                assert_mapping_and_budget(&doc);
            }
        }
    }

    #[test]
    fn deferred_probe_interruptions_replay_in_original_paragraph_and_scope() {
        for (first, next) in [
            ("", ""),
            ("> ", "> "),
            ("- ", "  "),
            (">\t", ">\t"),
            ("- - ", "\t"),
            ("- > ", "\t>\t"),
        ] {
            for tail in [
                "===\n",
                "# h\n",
                "***\n",
                "~~~\n",
                "$$\n",
                "> child\n",
                "- child\n",
                "1. child\n",
                "\n",
                "    code\n",
                "\tcode\n",
            ] {
                let head = format!("{first}[r]:\n");
                let row = format!("{next}<bad **中🙂**\n");
                let trigger = format!("{next}{tail}");
                let source = format!("{head}{row}{trigger}");
                let mut doc = Document::new();
                doc.append(&head).unwrap();
                let mut columns = None;
                assert!(stage(&mut doc, &row, 0, &mut columns).1);
                let (_, retained) = stage(&mut doc, &trigger, 0, &mut columns);
                let model = parse(&source);
                let leaf = model.blocks.last().unwrap();
                let ordinary = leaf.container == doc.blocks[0].container
                    && matches!(
                        leaf.kind,
                        BlockKind::Paragraph | BlockKind::Quote { .. } | BlockKind::ListItem { .. }
                    )
                    && model.headings.entries().is_empty();
                assert!(!retained || ordinary, "{source:?}");
                // A tab spelling that misses the old path can still become
                // ordinary lazy text. It must terminate the candidate.
                if !trigger.contains('\t') {
                    assert_eq!(retained, ordinary, "{source:?}");
                }
                doc.finish();
                assert_eq!(
                    support::snapshot(&doc),
                    support::snapshot(&model),
                    "{source:?}"
                );
            }
        }
        for (head, row, trigger) in [
            ("> [r]:\n", "> <bad\n", "lazy\n"),
            ("- [r]:\n", "  <bad\n", "- sibling\n"),
            ("> - [r]:\n", ">   <bad\n", "> - sibling\n"),
            ("> - [r]:\n", ">   <bad\n", "missing\n"),
            ("- > [r]:\n", "  > <bad\n", "  > > child\n"),
        ] {
            let source = format!("{head}{row}{trigger}");
            let mut doc = Document::new();
            doc.append(head).unwrap();
            let mut columns = None;
            assert!(stage(&mut doc, row, 0, &mut columns).1);
            assert!(!stage(&mut doc, trigger, 0, &mut columns).1);
            doc.finish();
            assert_eq!(
                support::snapshot(&doc),
                support::snapshot(&parse(&source)),
                "{source:?}"
            );
        }
    }

    #[test]
    fn deferred_probe_matches_real_leaf_and_scope_qualification_matrix() {
        for (first, next) in [
            ("", ""),
            ("> ", "> "),
            ("- ", "  "),
            ("> - ", ">   "),
            ("- > ", "  > "),
            ("- - ", "    "),
            (">\t", ">\t"),
            ("> ", ">\t"),
            ("   > ", "   >\t"),
            ("- ", "\t"),
            ("1. ", "\t"),
            ("- - ", "\t"),
            ("- - - ", "\t\t"),
            ("- > ", "\t>\t"),
            ("> > ", ">\t>\t"),
        ] {
            for indent in [
                "", " ", "  ", "   ", "    ", "     ", "\t", " \t", "\t ", "  \t", "\t  ", "   \t",
                "\t\t",
            ] {
                for text in [
                    "plain",
                    "[s]: /url",
                    "b|c",
                    "-|-",
                    "===",
                    "# h",
                    "***",
                    "~~~",
                    "$$",
                    "> child",
                    "- child",
                    "1. child",
                    "2. child",
                    "+",
                    "-",
                ] {
                    let head = format!("{first}[r]:\n");
                    let row = format!("{next}<bad\n");
                    let tail = format!("{next}{indent}{text}\n");
                    let source = format!("{head}{row}{tail}");
                    let model = parse(&source);
                    let mut doc = Document::new();
                    doc.append(&head).unwrap();
                    let scope = doc.blocks[0].container;
                    let mut columns = None;
                    assert!(stage(&mut doc, &row, 0, &mut columns).1);
                    let retained = stage(&mut doc, &tail, 0, &mut columns).1;
                    let last = model.blocks.last().unwrap();
                    let ordinary = last.container == scope
                        && matches!(
                            last.kind,
                            BlockKind::Paragraph
                                | BlockKind::Quote { .. }
                                | BlockKind::ListItem { .. }
                        )
                        && model.headings.entries().is_empty();
                    assert!(!retained || ordinary, "{source:?}");
                    if !tail.contains('\t') {
                        assert_eq!(retained, ordinary, "{source:?}");
                    }
                    doc.finish();
                    assert_eq!(
                        support::snapshot(&doc),
                        support::snapshot(&model),
                        "{source:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn deferred_raw_virtual_columns_and_target_indentation_are_distinct() {
        // Opening/target lines keep their existing partial-tab capability.
        // Their stored semantic source must neither invent bytes for virtual
        // columns nor lose the first real payload byte.
        for (source, raw) in [
            ("> [r]: /u\n>\t'a\n", "'a\n"),
            ("> [r]:\n>\t/u 'a\n", "/u 'a\n"),
            ("> [r\n>\t中]: /u 'a\n", "中]: /u 'a\n"),
        ] {
            let mut doc = Document::new();
            doc.append(source).unwrap();
            let fragment = doc
                .definition_candidate
                .as_ref()
                .unwrap()
                .sources
                .fragments
                .last()
                .unwrap();
            assert_eq!(&doc.source[fragment.raw.range()], raw);
            assert_eq!(fragment.virtual_spaces, 2);
            doc.append("> b'\n").unwrap();
            assert_eq!(
                doc.references.entries[0].title.and_then(|s| doc.text(s)),
                Some("a\n> b")
            );
        }
        for (head, text, expected_raw, spaces) in [
            (">\t[r]:\n", ">\t中🙂\n", "中🙂\n", 2),
            ("- [r]:\n", "\t中🙂\n", "中🙂\n", 2),
            ("> [r]:\n", ">  中🙂\n", " 中🙂\n", 0),
            ("- - [r]:\n", "\t中🙂\n", "中🙂\n", 0),
        ] {
            let mut doc = Document::new();
            doc.append(head).unwrap();
            let mut columns = None;
            assert!(stage(&mut doc, text, 0, &mut columns).1);
            let fragment = doc
                .definition_candidate
                .as_ref()
                .unwrap()
                .sources
                .fragments
                .last()
                .unwrap();
            assert_eq!(&doc.source[fragment.raw.range()], expected_raw);
            assert_eq!(fragment.virtual_spaces, spaces);
            assert_eq!(&doc.source[fragment.source.range()], text);
        }
        for (head, tail) in [
            ("[r]:\n", "    /url\n"),
            (">\t[r]:\n", ">\t  /url\n"),
            ("- [r]:\n", "\t  /url\n"),
        ] {
            let mut doc = Document::new();
            doc.append(head).unwrap();
            let source = format!("{head}{tail}");
            let span = Span::new(head.len(), source.len());
            let scope = doc.blocks[0].container;
            assert!(
                doc.containers
                    .continuation_source(&source, span, scope, false, None, &mut doc.work)
                    .is_none()
            );
            assert!(
                doc.containers
                    .definition_prefix(&source, span, scope, &mut doc.work)
                    .is_some()
            );
            assert!(
                parse(&source)
                    .blocks
                    .iter()
                    .any(|b| matches!(b.kind, BlockKind::Definition { .. }))
            );
        }
    }

    #[test]
    fn deferred_replay_keeps_prior_objects_and_does_not_restart_definitions() {
        let history = "[old](url) [ref]\n\n";
        for (first, next) in [("", ""), ("> ", "> "), ("- ", "  "), (">\t", ">\t")] {
            let head = format!("{first}[r]:\n");
            let rows = [
                format!("{next}<bad\n"),
                format!("{next}[s]: /must-stay-text\n"),
                format!("{next}[ref] *中🙂*\n"),
            ];
            let source = format!("{history}{head}{}", rows.concat());
            let mut doc = Document::new();
            doc.append(history).unwrap();
            let links = doc.links.clone();
            let info = doc.link_info.clone();
            doc.append(&head).unwrap();
            let block = doc.current.unwrap();
            let scope = doc.blocks[block.0].container;
            let mut columns = None;
            for row in rows {
                assert!(stage(&mut doc, &row, 0, &mut columns).1);
            }
            doc.finish();
            assert_eq!(&doc.links[..links.len()], links);
            assert_eq!(&doc.link_info[..info.len()], info);
            assert_eq!(doc.blocks[block.0].container, scope);
            assert!(
                doc.references
                    .entries
                    .iter()
                    .all(|r| r.destination.is_none())
            );
            assert_eq!(support::snapshot(&doc), support::snapshot(&parse(&source)));
            assert_mapping_and_budget(&doc);
        }
    }

    #[test]
    fn deferred_eof_and_utf8_fragment_splits_match_real_ordinary_snapshots() {
        for (first, next) in [("", ""), (">\t", ">\t"), ("- ", "\t")] {
            let head = format!("{first}[r|s]:\r\n");
            let first_row = format!("{next}<bad\r\n");
            let row = format!("{next}[中🙂] *x* &amp;\r\n");
            for end in row.char_indices().map(|(i, _)| i).chain([row.len()]) {
                let prefix = &row[..end];
                for split in prefix.char_indices().map(|(i, _)| i).chain([prefix.len()]) {
                    let mut doc = Document::new();
                    for c in head.chars() {
                        doc.append(c.encode_utf8(&mut [0; 4])).unwrap();
                    }
                    let mut columns = None;
                    assert!(stage(&mut doc, &first_row, 0, &mut columns).1);
                    if !prefix.is_empty() {
                        stage(&mut doc, prefix, split, &mut columns);
                    }
                    doc.finish();
                    let source = format!("{head}{first_row}{prefix}");
                    assert_eq!(
                        support::snapshot(&doc),
                        support::snapshot(&parse(&source)),
                        "{source:?} {split}"
                    );
                    assert!(doc.finish().changes.is_empty());
                }
            }
        }
    }

    #[test]
    fn deferred_fold_updates_reject_stale_tickets_after_replay() {
        for (first, next) in [("", ""), ("> ", "> "), (">\t", ">\t"), ("- ", "\t")] {
            for tail in ["===\n", "b|c\n", "plain\n"] {
                let mut doc = Document::new();
                doc.append(&format!("{first}[r]:\n")).unwrap();
                let mut view = FoldView::new(&doc, 1).unwrap();
                let ticket = view.heights().ticket(TileId(0)).unwrap();
                view.measure(ticket, 3).unwrap();
                let mut columns = None;
                let (update, _) = stage(&mut doc, &format!("{next}<bad\n"), 0, &mut columns);
                view.apply(&doc, &update).unwrap();
                assert_eq!(doc.source_fold_limit(), 0);
                assert_eq!(doc.blocks[0].state, SyntaxState::Growing);
                assert!(SourceFoldRegistration::new(&doc, &view, doc.blocks[0].source).is_err());
                let (update, _) = stage(&mut doc, &format!("{next}{tail}"), 0, &mut columns);
                view.apply(&doc, &update).unwrap();
                let update = if tail == "b|c\n" {
                    stage(&mut doc, &format!("{next}-|-\n"), 0, &mut columns).0
                } else {
                    doc.finish()
                };
                view.apply(&doc, &update).unwrap();
                assert!(view.measure(ticket, 3).is_err());
                let update = doc.finish();
                view.apply(&doc, &update).unwrap();
                assert_eq!(doc.source_fold_limit(), doc.source.len());
            }
        }
    }

    #[test]
    fn root_title_failures_replay_once_against_real_ordinary_controls() {
        let mut cases = 0;
        for independent in [false, true] {
            for indent in ["", " ", "   ", "    ", "\t", " \t", "        "] {
                for middle in ["plain", "x|y", "x\\|y", "`x|y`", "$x|y$", "[s]: /stay-text"] {
                    for trigger in [
                        "",
                        " \t",
                        "===",
                        "---",
                        "***",
                        "# h",
                        "~~~",
                        "$$",
                        "> quote",
                        "- item",
                        "1. item",
                        "    code",
                        "\tcode",
                        "-|-",
                        "' bad",
                        "unclosed",
                        "[new]: /url",
                        "(bad)",
                    ] {
                        let head = if independent { "[r]: /url\n" } else { "" };
                        let opener = if independent {
                            "'first\n"
                        } else {
                            "[r]: /url 'first\n"
                        };
                        let tail = format!("{opener}{middle}\n{indent}{trigger}\n");
                        let source = format!("{head}{tail}");
                        let actual = parse(&source);
                        // Disable only the initial admission in the production
                        // pipeline. All classification, tables, Inline and IDs
                        // are from the real ordinary parser, not a second model.
                        let control = if independent {
                            let mut doc = Document::new();
                            doc.append(head).unwrap();
                            doc.seal_definition_title(&mut Update::default());
                            doc.append(&tail).unwrap();
                            doc.finish();
                            doc
                        } else {
                            ordinary_control(&source)
                        };
                        assert_eq!(
                            support::snapshot(&actual),
                            support::snapshot(&control),
                            "{source:?}"
                        );
                        assert_mapping_and_budget(&actual);
                        cases += 1;
                    }
                }
            }
        }
        eprintln!("root title actual ordinary controls: {cases}");
        // Replay cannot restart a new definition, even when it looks valid in
        // isolation. The blank trigger itself is still consumed exactly once.
        for independent in [false, true] {
            let head = if independent {
                "[r]: /url\n'first\n"
            } else {
                "[r]: /url 'first\n"
            };
            let doc = parse(&format!("{head}[s]: /must-stay-text\n\n"));
            assert!(!doc.references().iter().any(|r| {
                r.destination
                    .is_some_and(|s| doc.text(s) == Some("/must-stay-text"))
            }));
        }
    }

    #[test]
    fn container_title_failures_match_actual_ordinary_pipeline() {
        let mut cases = 0;
        for (first, next) in [
            ("> ", "> "),
            ("- ", "  "),
            ("> - ", ">   "),
            ("- > ", "  > "),
            ("- - ", "\t"),
            (">\t", ">\t"),
            ("1. ", "\t"),
            ("-    ", "\t\t"),
            ("- > ", "\t>\t"),
            ("> > ", ">\t>\t"),
            ("- - - ", "\t\t"),
            (" > ", " >\t"),
            ("   > ", "   >\t"),
            (" > - ", " >\t\t"),
            ("> - > ", ">\t>\t"),
        ] {
            for independent in [false, true] {
                for middle in [
                    "plain",
                    "a|b",
                    "`a|b`",
                    "[s]: /stay-text",
                    " plain",
                    "  plain",
                    "\tplain",
                    " \tplain",
                ] {
                    for trigger in [
                        "",
                        " \t",
                        "===",
                        "---",
                        "***",
                        "# h",
                        "~~~",
                        "$$",
                        "> quote",
                        "- sibling",
                        "1. item",
                        "    code",
                        "\tcode",
                        "-|-",
                        "' bad",
                        "(bad)",
                    ] {
                        let head = if independent {
                            format!("{first}[r]: /u\n")
                        } else {
                            String::new()
                        };
                        let opener = if independent {
                            format!("{next}'first\n")
                        } else {
                            format!("{first}[r]: /u 'first\n")
                        };
                        let tail = format!("{opener}{next}{middle}\n{next}{trigger}\n");
                        let source = format!("{head}{tail}");
                        let actual = parse(&source);
                        let control = if independent {
                            let mut doc = Document::new();
                            doc.append(&head).unwrap();
                            doc.seal_definition_title(&mut Update::default());
                            doc.append(&tail).unwrap();
                            doc.finish();
                            doc
                        } else {
                            ordinary_control(&source)
                        };
                        assert_eq!(
                            support::snapshot(&actual),
                            support::snapshot(&control),
                            "{source:?}"
                        );
                        assert_mapping_and_budget(&actual);
                        cases += 1;
                    }
                }
            }
        }
        eprintln!("{cases} container title failures against actual ordinary pipeline");
    }

    #[test]
    fn partial_title_sources_keep_exact_prefix_gaps_and_failure_tab_replacements() {
        for (first, next, remainder) in [
            ("> ", ">\t", 2),
            ("- ", "\t", 2),
            ("1. ", "\t", 1),
            ("-    ", "\t\t", 3),
            ("- > ", "\t>\t", 2),
            ("> > ", ">\t>\t", 2),
            ("- - - ", "\t\t", 2),
            (" > ", " >\t", 1),
            ("   > ", "   >\t", 3),
            (" > - ", " >\t\t", 3),
            ("> - > ", ">\t>\t", 2),
        ] {
            for independent in [false, true] {
                let gap = if independent {
                    format!("\n{next}")
                } else {
                    " ".into()
                };
                let head = format!("[r]\n\n{first}[r]: /u{gap}'a\r\n");
                let row = format!("{next}中🙂\t&amp;\r\n");
                let mut doc = Document::new();
                doc.append(&head).unwrap();
                let update = doc.append(&row).unwrap();
                assert!(update.new_links.is_empty());
                let candidate = doc
                    .definition_candidate
                    .as_ref()
                    .unwrap_or_else(|| panic!("{head:?} {row:?}"));
                let fragment = candidate.sources.fragments.last().unwrap();
                assert_eq!(fragment.virtual_spaces, remainder, "{first:?} {next:?}");
                assert_eq!(doc.text(fragment.raw), Some("中🙂\t&amp;\r\n"));
                let prefix = Span::new(fragment.source.start, fragment.raw.start);
                assert_eq!(doc.text(prefix), Some(next));
                assert_eq!(candidate.title.as_ref().unwrap().gaps, [prefix]);
                let tab = Span::new(fragment.raw.start - 1, fragment.raw.start);

                // EOF is a real failure, compared to the independent ordinary
                // pipeline elsewhere. Its partial tab remains one source byte.
                doc.finish();
                let run = doc.display_runs_in(tab).unwrap().next().unwrap();
                assert_eq!(run.source, tab);
                assert_eq!(run.replacement, Some(tab));
                assert_eq!(run.text, " ".repeat(remainder.into()));
                assert!(!run.style.contains(Style::HIDDEN));
                assert_mapping_and_budget(&doc);

                let doc = parse(&format!("{head}{row}{next}'\n"));
                let title = doc.link_title_span(LinkId(0)).unwrap();
                assert_eq!(
                    doc.link_title(LinkId(0)),
                    Some(format!("a\r\n{row}{next}").as_str())
                );
                assert_eq!(
                    doc.references.title_gaps[0],
                    [prefix, Span::new(head.len() + row.len(), title.end)]
                );
                let mut cursor = doc.decoded_link_title(LinkId(0)).unwrap();
                let mut decoded = String::new();
                while let Some(chunk) = cursor
                    .advance(&mut Budget::new(WorkLimit::unlimited()))
                    .unwrap()
                {
                    assert!(doc.text(chunk.source).is_some());
                    decoded.push_str(chunk.text);
                }
                assert_eq!(decoded, "a\n中🙂\t&\n");
                assert_eq!(cursor.source_position(), title.end);
            }
        }
    }

    #[test]
    fn duplicate_container_titles_retain_only_winning_prefix_gaps() {
        for (independent, next) in [false, true]
            .into_iter()
            .flat_map(|independent| ["> ", ">\t"].map(|next| (independent, next)))
        {
            let gap = if independent {
                format!("\n{next}")
            } else {
                " ".into()
            };
            let mut doc = Document::new();
            doc.append(&format!("[r]\n\n> [r]: /u{gap}'a\n{next}b'\n"))
                .unwrap();
            assert_eq!(doc.references.title_gaps.len(), 1);
            let winning = doc.references.title_gaps[0].clone();
            for _ in 0..4096 {
                doc.append(&format!("\n> [r]: /ignored{gap}'c\n{next}d'\n"))
                    .unwrap();
                assert_eq!(doc.references.title_gaps.len(), 1);
                assert_eq!(doc.references.title_gaps[0], winning);
                assert!(doc.definition_candidate.is_none());
            }
            doc.finish();
            assert_eq!(
                doc.references
                    .title_gaps
                    .iter()
                    .map(Vec::len)
                    .sum::<usize>(),
                1
            );
            assert_eq!(
                doc.link_title(LinkId(0)),
                Some(format!("a\n{next}b").as_str())
            );
        }
    }

    #[test]
    fn deferred_ledger_bounds_order_and_constant_admission_work() {
        fn fragment(start: usize, end: usize) -> DeferredFragment {
            DeferredFragment::from_payload(
                Span::new(start, end),
                Span::new(start, end),
                0,
                SourceOwner::new(),
            )
        }
        let mut ledger = DeferredSources::new(BlockId(0), None, fragment(0, 1));
        assert!(ledger.push(fragment(0, 1)).is_err());
        assert!(ledger.push(fragment(2, 3)).is_err());
        let long = DeferredSources::new(BlockId(0), None, fragment(0, BLOCK_LOOKAHEAD + 5));
        assert_eq!(long.confirm(), Span::new(0, BLOCK_LOOKAHEAD + 5));
        for i in 1..MAX_DEFERRED_SOURCES {
            assert!(ledger.push(fragment(i, i + 1)).is_ok());
        }
        assert!(
            ledger
                .push(fragment(MAX_DEFERRED_SOURCES, MAX_DEFERRED_SOURCES + 1))
                .is_err()
        );
        assert_eq!(ledger.confirm(), Span::new(0, MAX_DEFERRED_SOURCES));
        for depth in [0, 32, 128, 970] {
            let prefix = ">".repeat(depth);
            let mut doc = Document::new();
            doc.append(&format!("{prefix}[r]:\n")).unwrap();
            let mut columns = None;
            for _ in 0..50 {
                let text = format!("{prefix}<bad\n");
                let before = doc.work;
                assert!(stage(&mut doc, &text, 0, &mut columns).1);
                assert_eq!(doc.work.inline_bytes, before.inline_bytes);
                assert!(doc.work.container_steps - before.container_steps <= 3 * depth as u64);
                assert!(doc.work.container_bytes - before.container_bytes < 8 * text.len() as u64);
            }
            let before = doc.work;
            doc.flush_definition_candidate(&mut Update::default());
            let n = doc.source.len() as u64;
            assert!(doc.work.inline_bytes - before.inline_bytes <= n + 150);
            assert!(doc.work.classified_bytes < 20 * n);
            assert!(doc.work.container_steps < 10 * n);
            eprintln!(
                "deferred depth {depth}: input={n}, classified={}, inline={}, container_bytes={}, container_steps={}",
                doc.work.classified_bytes,
                doc.work.inline_bytes,
                doc.work.container_bytes,
                doc.work.container_steps
            );
            doc.append(&format!("{prefix}{}", "x".repeat(1_000_000)))
                .unwrap();
            let before = doc.work;
            for _ in 0..1000 {
                doc.append("x").unwrap();
            }
            assert_eq!(doc.work.inline_bytes - before.inline_bytes, 1000);
            assert_eq!(doc.work.classified_bytes, before.classified_bytes);
            assert_eq!(doc.work.container_steps, before.container_steps);
            assert_eq!(doc.work.container_bytes, before.container_bytes);
            eprintln!(
                "deferred depth {depth}: 1000 growing suffix bytes, inline +{}, classified +{}, container bytes +{}, container steps +{}",
                doc.work.inline_bytes - before.inline_bytes,
                doc.work.classified_bytes - before.classified_bytes,
                doc.work.container_bytes - before.container_bytes,
                doc.work.container_steps - before.container_steps
            );
            doc.finish();
        }
    }
}
