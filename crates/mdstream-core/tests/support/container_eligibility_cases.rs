pub fn cases() -> Vec<String> {
    let mut cases: Vec<String> = [
        "> **a\n> b**\n",
        "> *a\n>\n> b*\n",
        "> [a](u\n>\n> 't')\n",
        "> *a\n> > child\n> b*\n",
        "> *a\n> > # child\n> b*\n",
        "> [a](u\n> > # child\n> 't')\n",
        "> [a](u\n 't')\n",
        "> [a](u\n    't')\n",
        "> [a](u\n>    't')\n",
        "> [a](u\n>     't')\n",
        "- [a](u\n  't')\n",
        "- [a](u\n- 't')\n",
        "> - > [a](u\n> 't')\n",
        "> [a](u\n>\t't')\n",
        ">\t[a](u 't')\n",
        "> [a](u 'x\n> y')\n",
        "> [r\n> not a definition\n> [a](u 't')\n",
        "> [r]: /u 'open\n> not closed\n> [a](u 't')\n",
        "> [r]: /u\n>\n> [r]\n",
        "> [r\n> label]: /u\n>\n> [r label]\n",
        "> a\n> ===\n> b\n",
        "> *a\n> # head\n> b*\n",
        "> *a\n> ```\n> b*\n> ```\n",
        "> *a\n> $$\n> b*\n> $$\n",
        "> *a\n> [^f]: note\n> b*\n",
        "> 中🙂 &amp; \\* _a_ ~~b~~ [Straße][] ![图](<u> 't')\n> tail\n",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    for prefix in [">\t", " >\t", "  >\t", "- ", "- >\t"] {
        cases.push(format!("{prefix}*中\n{prefix}🙂* &amp; \\*\n"));
        cases.push(format!("{prefix}[r\n{prefix}bad\n"));
        cases.push(format!("{prefix}[r]: /u 'open\n{prefix}bad\n"));
    }
    for width in [1023, 1024, 1025, 1100] {
        for prior in [
            "",
            "> *prior [a](u\n",
            "> [a](u\n",
            "> `prior\n",
            "> $prior\n",
            "> [prior\n",
        ] {
            for (tail, decision) in [("", ""), (" | b", "> -|-\n"), (" | b", "> wrong\n")] {
                cases.push(format!(
                    "{prior}>\tend*and{}{}\n{decision}",
                    "x".repeat(width),
                    tail
                ));
            }
        }
    }
    for prior in ["[a](u\n", "[a](u\n 't')\n", "[a](u\nbar)\n"] {
        for marker in ["> ", "- ", ">\t"] {
            cases.push(format!("{prior}{marker}{}\n", "x".repeat(1100)));
        }
    }
    for next in ["\nroot ", "- sibling ", "> > child "] {
        cases.push(format!("> [a](u\n{next}{}\n", "x".repeat(1100)));
    }
    // Preserve the known structural-prefix suffix-limit counterexample, too.
    cases.push(format!(
        "> [a]({}{}\n> '{}' )\n",
        " ".repeat(59),
        "d".repeat(4096),
        "t".repeat(4096)
    ));
    let crlf = cases
        .iter()
        .map(|s| s.replace('\n', "\r\n"))
        .collect::<Vec<_>>();
    cases.extend(crlf);
    cases
}

pub fn ends(source: &str) -> Vec<usize> {
    if source.len() < 180 {
        (0..=source.len()).collect()
    } else {
        let mut ends = vec![0, 1, 3, 1023, 1024, 1025, source.len() - 1, source.len()];
        ends.retain(|&end| end <= source.len());
        ends.sort_unstable();
        ends.dedup();
        ends
    }
}

pub fn hash(text: &str) -> u64 {
    text.bytes().fold(0xcbf29ce484222325, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x100000001b3)
    })
}

pub fn old_work(work: mdstream::Work) -> String {
    legacy_work(work, false)
}
#[allow(dead_code)]
pub fn full_work(work: mdstream::Work) -> String {
    legacy_work(work, true)
}
fn legacy_work(work: mdstream::Work, observers: bool) -> String {
    // New publication counters are tested independently; preserve this legacy ledger.
    let raw = format!("{work:?}");
    let raw = format!(
        "Work {{ {}",
        raw.split_once("paragraph_closures:")
            .map(|(_, rest)| format!("paragraph_closures:{rest}"))
            .unwrap()
    );
    raw.split(", ")
        .filter(|field| {
            (observers || !field.starts_with("container_eligibility_"))
                && !field.starts_with("code_fallback_")
                && !field.starts_with("fence_candidate_")
                && !field.starts_with("structural_prefix_")
                && !field.starts_with("link_title_gap_steps:")
                && !field.starts_with("link_fallback_")
                && !field.starts_with("bare_")
                && !field.starts_with("link_label_steps:")
                && !field.starts_with("link_index_steps:")
                && !field.starts_with("link_gate_steps:")
        })
        .collect::<Vec<_>>()
        .join(", ")
}
