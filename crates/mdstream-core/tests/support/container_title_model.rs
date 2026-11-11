use mdstream::*;

pub struct Model {
    pub source: String,
    pub target: Span,
    pub info: LinkInfo,
    pub chunks: Vec<(Span, String)>,
    pub gaps: Vec<Span>,
}

fn token(source: &mut String, chunks: &mut Vec<(Span, String)>, raw: &str, text: &str) {
    let start = source.len();
    source.push_str(raw);
    chunks.push((Span::new(start, source.len()), text.to_owned()));
}

// Construct expected metadata and scalar/entity maps alongside the input. No
// parser output, projection, offset-gap inference or snapshot supplies the model.
pub fn successes() -> Vec<Model> {
    let mut cases = Vec::new();
    for (first, prefix, indent) in [
        ("> ", "> ", ""),
        ("- ", "  ", ""),
        ("> - > ", ">   > ", ""),
        ("> ", "", ""),       // actual lazy continuation
        ("> ", "", " "),      // real lazy indentation
        ("> ", "", "\t"),     // real lazy tab
        ("> ", ">\t", " "),   // two virtual columns, one real space
        ("> ", " >\t", " "),  // one virtual column
        ("> ", "   >\t", ""), // three virtual columns
        ("> ", "  >\t", ""),  // whole tab
        ("> ", "> ", "  "),   // actual payload indentation
    ] {
        for ending in ["\n", "\r\n"] {
            for (open, close, escape, decoded) in [
                ("'", "'", "\\'", "'"),
                ("\"", "\"", "\\\"", "\""),
                ("(", ")", "\\(", "("),
            ] {
                for image in ["", "!"] {
                    let mut source = format!("{first}{image}[中&amp;](<p&amp;中> {open}");
                    let start = source.len();
                    let target_start = first.len() + image.len() + "[中&amp;](<".len();
                    let target = Span::new(target_start, target_start + "p&amp;中".len());
                    let mut chunks = Vec::new();
                    let mut gaps = Vec::new();
                    token(&mut source, &mut chunks, "a", "a");
                    token(&mut source, &mut chunks, ending, "\n");
                    for line in 0..2 {
                        let at = source.len();
                        source.push_str(prefix);
                        if !prefix.is_empty() {
                            gaps.push(Span::new(at, source.len()));
                        }
                        for ch in indent.chars() {
                            let text = ch.to_string();
                            token(&mut source, &mut chunks, &text, &text);
                        }
                        if line == 0 {
                            token(&mut source, &mut chunks, "中", "中");
                            token(&mut source, &mut chunks, "\t", "\t");
                            token(&mut source, &mut chunks, "🙂", "🙂");
                            token(&mut source, &mut chunks, "&nGt;", "≫\u{20d2}");
                            token(&mut source, &mut chunks, escape, decoded);
                            token(&mut source, &mut chunks, ending, "\n");
                        }
                    }
                    let end = source.len();
                    source.push_str(close);
                    source.push(')');
                    let info = LinkInfo {
                        kind: if image.is_empty() {
                            LinkKind::Link
                        } else {
                            LinkKind::Image
                        },
                        label: Span::new(
                            first.len() + image.len() + 1,
                            first.len() + image.len() + 1 + "中&amp;".len(),
                        ),
                        source: Span::new(first.len(), source.len()),
                        title: Some(Span::new(start, end)),
                        state: SyntaxState::Sealed,
                    };
                    source.push_str(ending);
                    cases.push(Model {
                        source,
                        target,
                        info,
                        chunks,
                        gaps,
                    });
                }
            }
        }
    }
    cases
}

pub fn controls() -> Vec<String> {
    [
        "[a](u 'a\n  b')\n", // root
        "> [a](u\n> 't')\n", // B
        "> [r]: /u 'a\n> b'\n>\n> [r]\n",
        "> [a](u 'a\n>\n> b')\n",
        "> [a](u 'a\n>     b')\n",
        "> [a](u 'a\n> > b')\n",
        "- [a](u 'a\n- b')\n",
        "> [a](<a\n> b> 't')\n",
        "> [a](u 'a\rb')\n",
        "> # [a](u 'a\n> b')\n",
        "> `[a](u 'a\n> b')`\n",
        "> $[a](u 'a\n> b')$\n",
        "> [^f]: [a](u 'a\n> b')\n",
        "> [a](u (a\n> (b))\n",
        "> [a](u 'a\n> # b')\n",
        "> [a](u 'a\n> ```\n> b')\n> ```\n",
        "> [a](u 'a\n> $$\n> b')\n> $$\n",
        "> [a](u 'a\n> b')|c\n> -|-\n",
        "> [a](u 'a\n> b')|c\n> wrong\n",
        "> [a](u 'a\n> [r]: /v\n> b')\n",
        "> [a](u 'a\n> [r]: bad title\n> b')\n",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

pub fn parse(source: &str) -> Document {
    let mut d = Document::new();
    d.append(source).unwrap();
    d.finish();
    d
}

pub fn chunks(d: &Document, id: LinkId) -> Option<Vec<(Span, String)>> {
    let mut c = d.decoded_link_title(id)?;
    let mut out = Vec::new();
    while let Some(chunk) = c.advance(&mut Budget::new(WorkLimit::unlimited())).unwrap() {
        out.push((chunk.source, chunk.text.to_owned()));
    }
    Some(out)
}

pub fn assert_model(d: &Document, m: &Model) {
    assert_eq!(d.links(), &[LinkTarget::Direct(m.target)], "{:?}", m.source);
    assert_eq!(d.link_info(LinkId(0)), Some(&m.info));
    assert_eq!(d.link_title(LinkId(0)), d.text(m.info.title.unwrap()));
    assert_eq!(
        chunks(d, LinkId(0)),
        Some(m.chunks.clone()),
        "{:?}",
        m.source
    );
    let text: String = d
        .display_runs_in(Span::new(0, d.source().len()))
        .unwrap()
        .filter(|r| !r.style.contains(Style::HIDDEN))
        .map(|r| r.text)
        .collect();
    assert_eq!(text, "中&\n");
}

pub fn table_cases() -> Vec<String> {
    let mut out = Vec::new();
    for width in [0, 1023, 1024, 1025, 1100] {
        for prior in ["", "*before ", "`before ", "$before ", "[outer "] {
            for decision in ["> -|-\n", "> wrong\n"] {
                out.push(format!(
                    "> {prior}[a](u 'a\n> b')end*and{} | c\n{decision}",
                    "x".repeat(width)
                ));
            }
        }
    }
    out
}

pub fn table_shape(d: &Document) -> String {
    format!(
        "{:?}",
        (
            d.blocks(),
            (0..d.container_count())
                .map(|i| d.container(ContainerId(i)).unwrap())
                .collect::<Vec<_>>(),
            d.tables(),
            d.references()
        )
    )
}
