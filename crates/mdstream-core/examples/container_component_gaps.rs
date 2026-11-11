//! Container component gaps keep single-line metadata and the original image ID.
use mdstream::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut d = Document::new();
    d.append("> ![图](\r\n")?;
    let original = *d.link_info(LinkId(0)).unwrap();
    d.append(">\t<u&amp;>\n> 'title'\r\n> )\n")?;
    d.finish();
    assert_eq!(d.links().len(), 1);
    assert_eq!(d.link_info(LinkId(0)).unwrap().label, original.label);
    assert_eq!(d.link_destination(LinkId(0)), Some("u&amp;"));
    assert_eq!(d.link_title(LinkId(0)), Some("title"));
    println!("{:?}", d.link_info(LinkId(0)).unwrap());

    let mut failed = Document::new();
    failed.append("> [a](u\n> 'a&amp;'&amp;)\n")?;
    failed.finish();
    let visible: String = failed
        .display_runs_in(Span::new(0, failed.source().len()))
        .unwrap()
        .filter(|r| !r.style.contains(Style::HIDDEN))
        .map(|r| r.text)
        .collect();
    assert_eq!(visible, "a(u\n'a&amp;'&)\n");
    println!("opaque failure: {visible:?}");
    Ok(())
}
