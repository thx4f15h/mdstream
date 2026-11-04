//! Offline scoring of CommonMark 0.31.2 Fenced code blocks (119–147).
//! Project public semantics only; no Markdown reparsing, trimming, or example IDs.
use mdstream::{BlockId, BlockKind, ContainerKind, Document, Span, Style};

#[path = "support/snapshot.rs"]
mod support;
use support::snapshot;

include!("fixtures/commonmark_fenced_code.rs");

fn parse(source: &str) -> Document {
    let mut doc = Document::new();
    doc.append(source).unwrap();
    doc.finish();
    doc
}

fn escape(ch: char, out: &mut String) {
    match ch {
        '&' => out.push_str("&amp;"),
        '<' => out.push_str("&lt;"),
        '>' => out.push_str("&gt;"),
        '"' => out.push_str("&quot;"),
        _ => out.push(ch),
    }
}

// A narrow test adapter: quote containers, top-level headings, paragraphs and
// code blocks/spans. Unknown structures/styles fail rather than disappear.
fn project(doc: &Document) -> String {
    assert!(doc.links().is_empty());
    assert!(doc.maths().is_empty());
    assert!(doc.tables().is_empty());
    let mut html = String::new();
    let mut stack = Vec::new();
    let mut seen = vec![false; doc.container_count()];
    let mut headings = doc.headings().entries().iter();
    for (index, block) in doc.blocks().iter().enumerate() {
        let mut path = Vec::new();
        let mut parent = block.container;
        while let Some(id) = parent {
            path.push(id);
            let container = doc.container(id).unwrap();
            assert_eq!(container.kind, ContainerKind::Quote);
            parent = container.parent;
        }
        path.reverse();
        let shared = stack.iter().zip(&path).take_while(|(a, b)| a == b).count();
        while stack.len() > shared {
            stack.pop();
            html.push_str("</blockquote>\n");
        }
        for &id in &path[shared..] {
            assert!(!seen[id.0], "reopened sealed container");
            seen[id.0] = true;
            html.push_str("<blockquote>\n");
            stack.push(id);
        }
        let (open, close, literal) = match block.kind {
            BlockKind::Blank => continue,
            BlockKind::Paragraph => {
                assert!(block.container.is_none(), "unscored container prose");
                ("<p>".to_owned(), "</p>\n".to_owned(), false)
            }
            BlockKind::Heading { level } => {
                assert!(block.container.is_none(), "unscored container heading");
                let heading = headings.next().expect("missing indexed heading");
                assert_eq!(heading.block, BlockId(index));
                assert_eq!(heading.level, level);
                assert_eq!(heading.source, block.source);
                assert_eq!(heading.container, block.container);
                (format!("<h{level}>"), format!("</h{level}>\n"), false)
            }
            BlockKind::Code { info } => {
                let mut open = "<pre><code".to_owned();
                if let Some(info) = info {
                    assert!(block.source.start <= info.start && info.end <= block.source.end);
                    // HTML language classes use the first word of the public
                    // raw info span; no Markdown syntax is inferred here.
                    let raw = doc.text(info).unwrap();
                    let language = raw.split_ascii_whitespace().next().unwrap();
                    // Entity/escape decoding of info is outside this chapter.
                    assert!(!raw.contains(['&', '\\']));
                    open.push_str(" class=\"language-");
                    for ch in language.chars() {
                        escape(ch, &mut open);
                    }
                    open.push('"');
                }
                open.push('>');
                (open, "</code></pre>\n".to_owned(), true)
            }
            ref other => panic!("block outside fenced code projection: {other:?}"),
        };
        let mut chars = Vec::new();
        for run in doc.display_runs_in(block.source).unwrap() {
            if run.style.contains(Style::HIDDEN) {
                continue;
            }
            assert!(run.link.is_none());
            for unsupported in [Style::EMPHASIS, Style::STRONG, Style::STRIKE, Style::MATH] {
                assert!(!run.style.contains(unsupported), "unscored inline style");
            }
            assert!(!literal || run.style.contains(Style::CODE));
            chars.extend(run.text.chars().map(|ch| (ch, run.style)));
        }
        // Only a physical prose terminator is structural. Code LFs, blank
        // lines, indentation, trailing spaces and internal tabs stay literal.
        if !literal && chars.last().is_some_and(|(ch, _)| *ch == '\n') {
            let (_, style) = chars.pop().unwrap();
            assert!(!style.contains(Style::HARD_BREAK));
            assert!(!style.contains(Style::SOFT_BREAK));
            assert!(!style.contains(Style::CODE));
        }
        html.push_str(&open);
        let mut active_code = false;
        for (ch, style) in chars {
            let code = style.contains(Style::CODE) && !literal;
            if active_code && !code {
                html.push_str("</code>");
            }
            if code && !active_code {
                html.push_str("<code>");
            }
            active_code = code;
            if ch == '\n' && style.contains(Style::HARD_BREAK) && !literal && !code {
                html.push_str("<br />");
            }
            escape(ch, &mut html);
        }
        if active_code {
            html.push_str("</code>");
        }
        html.push_str(&close);
    }
    for _ in stack {
        html.push_str("</blockquote>\n");
    }
    assert!(seen.into_iter().all(|seen| seen), "unscored container");
    assert!(headings.next().is_none(), "extra indexed heading");
    html
}

// All 29 examples match upstream after bounded fence indentation support.
// The upstream fixture and projection remain independent of this assessment.
fn check_case(number: usize, markdown: &str, expected: &str, doc: &Document) {
    assert_eq!(project(doc), expected, "example {number}");
    assert_eq!(doc.source(), markdown);
    let mut end = 0;
    for run in doc.display_runs_in(Span::new(0, markdown.len())).unwrap() {
        assert_eq!(run.source.start, end);
        assert!(run.source.start < run.source.end);
        assert!(markdown.is_char_boundary(run.source.start));
        assert!(markdown.is_char_boundary(run.source.end));
        if let Some(original) = run.replacement {
            assert!(original.start <= run.source.start && run.source.end <= original.end);
            assert!(markdown.is_char_boundary(original.start));
            assert!(markdown.is_char_boundary(original.end));
            // All replacements in this chapter are code-space normalization.
            assert!(run.style.contains(Style::CODE));
            let raw = &markdown[original.start..original.end];
            match run.text {
                "" => assert!(raw == " " || raw == "\n"),
                " " => assert_eq!(raw, "\n"),
                other => panic!("unexpected code replacement {other:?}"),
            }
        } else {
            assert_eq!(run.text, &markdown[run.source.start..run.source.end]);
        }
        end = run.source.end;
    }
    assert_eq!(end, markdown.len());
}

#[test]
fn pinned_commonmark_fenced_code_report() {
    assert_eq!(FENCED_CODE_CASES.len(), 29);
    for (index, &(number, markdown, expected)) in FENCED_CODE_CASES.iter().enumerate() {
        assert_eq!(number, 119 + index);
        check_case(number, markdown, expected, &parse(markdown));
        eprintln!("example {number}: supported — matches upstream");
    }
    eprintln!(
        "CommonMark 0.31.2 Fenced code blocks (119–147): 29 supported, 0 deliberate differences, 0 unsupported; other sections not scored by this test"
    );
}

#[test]
fn pinned_commonmark_fenced_code_every_split_and_scalar_prefix() {
    let mut splits = 0;
    let mut prefixes = 0;
    for &(number, markdown, expected) in FENCED_CODE_CASES {
        let whole = parse(markdown);
        for split in (0..=markdown.len()).filter(|&split| markdown.is_char_boundary(split)) {
            let mut doc = Document::new();
            doc.append(&markdown[..split]).unwrap();
            doc.append(&markdown[split..]).unwrap();
            doc.finish();
            assert_eq!(
                snapshot(&doc),
                snapshot(&whole),
                "example {number}, split {split}"
            );
            assert_eq!(doc.pending(), whole.pending());
            check_case(number, markdown, expected, &doc);
            splits += 1;
        }
        let mut doc = Document::new();
        for (offset, ch) in markdown.char_indices() {
            doc.append(ch.encode_utf8(&mut [0; 4])).unwrap();
            let mut prefix = Document::new();
            prefix.append(&markdown[..offset + ch.len_utf8()]).unwrap();
            assert_eq!(
                snapshot(&doc),
                snapshot(&prefix),
                "example {number}, prefix {offset}"
            );
            assert_eq!(doc.pending(), prefix.pending());
            prefixes += 1;
        }
        doc.finish();
        check_case(number, markdown, expected, &doc);
        assert_eq!(snapshot(&doc), snapshot(&whole));
    }
    assert_eq!((splits, prefixes), (585, 556));
    eprintln!(
        "29 examples: 585 UTF-8 single splits and 556 scalar prefixes preserve classified semantics and source metadata"
    );
}

#[test]
fn fenced_code_projection_preserves_literals_whitespace_info_and_structure() {
    for (source, expected) in [
        ("", ""),
        ("```\n", "<pre><code></code></pre>\n"),
        ("```\n```\n", "<pre><code></code></pre>\n"),
        ("```\na", "<pre><code>a</code></pre>\n"),
        ("```\na\n", "<pre><code>a\n</code></pre>\n"),
        (
            "~~~\n a  \n\n\t\n~~~\n",
            "<pre><code> a  \n\n\t\n</code></pre>\n",
        ),
        ("  ```\n  a\n ```\n", "<pre><code>a\n</code></pre>\n"),
        (
            "```rust\nx\n```\n",
            "<pre><code class=\"language-rust\">x\n</code></pre>\n",
        ),
        (
            "~~~ python other\nx\n~~~\n",
            "<pre><code class=\"language-python\">x\n</code></pre>\n",
        ),
        (
            "~~~ a\"<b>\nx\n~~~\n",
            "<pre><code class=\"language-a&quot;&lt;b&gt;\">x\n</code></pre>\n",
        ),
        (
            "```\n*hi* `x` &amp; <a/> $x$\n```\n",
            "<pre><code>*hi* `x` &amp;amp; &lt;a/&gt; $x$\n</code></pre>\n",
        ),
        ("```\nα\t中🙂\n```\n", "<pre><code>α\t中🙂\n</code></pre>\n"),
        ("    raw\n", "<pre><code>raw\n</code></pre>\n"),
        ("a  b\n", "<p>a  b</p>\n"),
        ("a\tb\n", "<p>a\tb</p>\n"),
        ("a &#32;\n", "<p>a  </p>\n"),
        ("a\nb\n", "<p>a\nb</p>\n"),
        ("a  \nb\n", "<p>a<br />\nb</p>\n"),
        ("`a`\n", "<p><code>a</code></p>\n"),
        ("`  a  `\n", "<p><code> a </code></p>\n"),
        ("`a\nb`\n", "<p><code>a b</code></p>\n"),
        ("# a\n", "<h1>a</h1>\n"),
        ("a\n---\n", "<h2>a</h2>\n"),
        (
            "a\n\n```\nx\n```\n\nb\n",
            "<p>a</p>\n<pre><code>x\n</code></pre>\n<p>b</p>\n",
        ),
        (
            "> ```\n> x\n> ```\n",
            "<blockquote>\n<pre><code>x\n</code></pre>\n</blockquote>\n",
        ),
        (
            "> > ```\n> > x\n> > ```\n",
            "<blockquote>\n<blockquote>\n<pre><code>x\n</code></pre>\n</blockquote>\n</blockquote>\n",
        ),
        (
            "> ```\n> x\n\ny\n",
            "<blockquote>\n<pre><code>x\n</code></pre>\n</blockquote>\n<p>y</p>\n",
        ),
        (
            "```\nx\n```\n```\ny\n```\n",
            "<pre><code>x\n</code></pre>\n<pre><code>y\n</code></pre>\n",
        ),
    ] {
        assert_eq!(project(&parse(source)), expected, "input {source:?}");
    }
    // Selecting a language class in the adapter must not hide mutation of the
    // core's full raw info span, including the remaining words and punctuation.
    for (source, expected) in [
        (
            "~~~    ruby startline=3 $%@#$\ncode\n~~~\n",
            "ruby startline=3 $%@#$",
        ),
        ("```  rust other \t\n```\n", "rust other"),
    ] {
        let doc = parse(source);
        let BlockKind::Code { info: Some(info) } = doc.blocks()[0].kind else {
            panic!("missing raw info");
        };
        assert_eq!(doc.text(info), Some(expected));
        assert_eq!(&source[info.start..info.end], expected);
    }
}

#[test]
fn fenced_code_projection_rejects_unscored_structure_and_styles() {
    for source in [
        "> prose\n",
        "> # heading\n",
        "- list\n",
        "- ```\n  x\n  ```\n",
        "***\n",
        "a|b\n--|--\n",
        "*em*\n",
        "**strong**\n",
        "~~strike~~\n",
        "$math$\n",
        "[link](url)\n",
        "![image](url)\n",
        "~~~ &amp;\nx\n~~~\n",
        "~~~ a\\*b\nx\n~~~\n",
    ] {
        assert!(
            std::panic::catch_unwind(|| project(&parse(source))).is_err(),
            "accepted unscored input {source:?}"
        );
    }
}
