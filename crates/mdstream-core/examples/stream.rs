use mdstream::{Document, HeightIndex, Style};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut document = Document::new();
    let mut heights = HeightIndex::new(1)?;
    for chunk in [
        "# Stream\n\n",
        "Hello **world** and [guide].\n\n",
        "[guide]: https://example.org\n",
    ] {
        let update = document.append(chunk)?;
        heights.apply(&document, &update)?;
    }
    let update = document.finish();
    heights.apply(&document, &update)?;

    // A renderer obtains only its viewport plus overscan. Actual shaping/wrapping
    // belongs to the caller; this example prints semantic runs, not a TUI.
    for visible in heights.viewport(0..24, 4, 64) {
        let ticket = heights.ticket(visible.tile).unwrap();
        for run in document.runs(visible.tile).unwrap() {
            if !run.style.contains(Style::HIDDEN) {
                println!(
                    "{:?} {:?} {:?} {:?}",
                    run.block,
                    run.style,
                    run.text,
                    run.link.and_then(|id| document.link_destination(id))
                );
            }
        }
        // After real layout, feed back the rows owned by this tile:
        heights.measure(ticket, 1)?;
    }
    println!("work: {:?}", document.work());
    Ok(())
}
