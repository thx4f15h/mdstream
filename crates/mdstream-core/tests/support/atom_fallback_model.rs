use mdstream::*;

pub const TARGETS: &[(&str, &str)] = &[
    ("[a](&amp;", "a(&"),
    ("[a](&amp; x)", "a(& x)"),
    ("[a](\\*", "a(*"),
    ("[a](u 'x\n&amp;\\*", "a(u 'x\n&*"),
    ("[a](u 'x\n&amp;'x)", "a(u 'x\n&'x)"),
    ("> [a](u 'x\n> &amp;\\*", "a(u 'x\n&*"),
    ("> [a](u 'x\n>\t&amp;\\*", "a(u 'x\n  &*"),
    ("[a](u '&amp;'\\*)", "a(u '&'*)"),
    ("[a](u '&amp;'[b](v)", "a(u '&'b"),
    ("[a](&#10; x)", "a(\n x)"),
    ("| [a](u '&amp;'x) | b |\n| - | - |\n", "a(u '&'x)b\n"),
];

pub fn parse(source: &str) -> Document {
    let mut d = Document::new();
    d.append(source).unwrap();
    d.finish();
    d
}

pub fn shown(d: &Document) -> String {
    d.display_runs_in(Span::new(0, d.source().len()))
        .unwrap()
        .filter(|r| !r.style.contains(Style::HIDDEN))
        .map(|r| r.text)
        .collect()
}

// Actual predecessor snapshots cover every disqualifying syntax byte, incomplete
// atoms, successful grammar and existing first-cell/opaque routing. No test writes
// expectations. The external predecessor probe froze these before implementation.
pub fn controls() -> Vec<String> {
    let mut cases = Vec::new();
    for prefix in ["", "> ", "- ", "# "] {
        for tail in [
            "<x&amp;",
            "[b]&amp;",
            "]&amp;",
            "!&amp;",
            "*b*&amp;",
            "_b_&amp;",
            "~b~&amp;",
            "`b&amp;`",
            "$b&amp;$",
            "&amp;&half",
            "&amp;\\",
            "u '&amp;\\\nnext",
            "u '&amp;\\\r\nnext",
        ] {
            cases.push(format!("{prefix}[a]({tail}"));
        }
        for body in [
            "[a][&amp;",
            "[a",
            "[a]",
            "[a](u '&amp;\\*')\n",
            "`[a](&amp;`\n",
            "$[a](&amp;$\n",
            "[r]: /u '&amp;\\*'\n\n[r]\n",
        ] {
            cases.push(format!("{prefix}{body}"));
        }
    }
    for prior in ["[old\n", "`old\n", "$old\n", "*old\n"] {
        for row in ["end](u 't')", "end](u '&amp;')", "end`", "end$"] {
            for decision in ["-|-", "wrong"] {
                cases.push(format!("{prior}{row} | b\n{decision}\n"));
                cases.push(format!("> {prior}> {row} | b\n> {decision}\n"));
            }
        }
    }
    for width in [1023, 1024, 1025] {
        for next in ["> > ", "- ", "> # ", "> ===\n> ", "> [r]: /v\n> "] {
            cases.push(format!(
                "> [a](u '*&amp;\n{next}{} | b\n> -|-\n",
                "x".repeat(width)
            ));
        }
    }
    cases
}

pub fn hash(text: &str) -> u64 {
    text.bytes().fold(0xcbf29ce484222325, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x100000001b3)
    })
}
