//! Failed direct suffixes recover closed atoms without replaying inline grammar.
use mdstream::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut doc = Document::new();
    doc.append("> [a](u 'x\n")?;
    let original = *doc.link_info(LinkId(0)).unwrap();
    doc.append(">\t&amp;\\*'x)\n")?;
    doc.finish();
    let shown: String = doc
        .display_runs_in(Span::new(0, doc.source().len()))
        .unwrap()
        .filter(|run| !run.style.contains(Style::HIDDEN))
        .map(|run| run.text)
        .collect();
    assert_eq!(shown, "[a](u 'x\n  &*'x)\n");
    assert_eq!(doc.link_info(LinkId(0)).unwrap().source, original.source);
    assert_eq!(doc.link_destination(LinkId(0)), None);
    println!("{shown}");
    println!("recovery visits: {}", doc.work().link_fallback_bytes);
    Ok(())
}
