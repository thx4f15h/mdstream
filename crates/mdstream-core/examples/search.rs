use mdstream::{Budget, Completion, DisplaySearch, Document, WorkLimit};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut doc = Document::new();
    doc.append("# Search\n\na**b**&amp;c and ` ab&c `\n")?;
    doc.finish();
    let query = "ab&c";
    let mut search = DisplaySearch::new(&doc, query)?;
    let mut matches = Vec::new();
    let mut frames = 0;
    loop {
        frames += 1;
        let mut budget = Budget::new(WorkLimit {
            bytes: 16,
            queries: 2,
            objects: 2,
            ..Default::default()
        });
        let completion = search.advance(&doc, &mut budget, &mut matches)?;
        for hit in matches.drain(..) {
            println!(
                "{query:?}: display {:?}, source {:?} = {:?}",
                hit.display,
                hit.source.range(),
                doc.text(hit.source).unwrap()
            );
        }
        if completion == Completion::EndOfDocument {
            println!(
                "searched revision {} in {frames} bounded steps",
                search.revision()
            );
            return Ok(());
        }
    }
}
