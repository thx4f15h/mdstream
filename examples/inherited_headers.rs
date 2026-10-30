//! Isolate a pending paragraph break while a late-pipe header streams.
use mdtui3::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    for success in [false, true] {
        let mut doc = Document::new();
        let prior = "> - **prior**  \r\n";
        doc.append(prior)?;
        doc.append(">   **name** [link](url) ")?;
        for _ in 0..1000 {
            doc.append(&"x".repeat(1000))?;
        }
        let original = *doc.link_info(LinkId(0)).unwrap();
        let newline = Span::new(prior.len() - 1, prior.len());
        assert!(
            !doc.runs_in(newline)
                .unwrap()
                .any(|run| run.style.contains(Style::HARD_BREAK))
        );
        let before = doc.work();
        doc.append("|**second** [next](target)\r\n")?;
        assert_eq!(doc.work().inline_bytes, before.inline_bytes);
        doc.append(if success {
            ">   :--|--:\n"
        } else {
            ">   wrong\n"
        })?;
        assert_eq!(*doc.link_info(LinkId(0)).unwrap(), original);
        assert_eq!(doc.links().len(), 2);
        assert_eq!(doc.tables().len(), usize::from(success));
        assert_eq!(
            doc.runs_in(newline)
                .unwrap()
                .any(|run| run.style.contains(Style::HARD_BREAK)),
            !success
        );
        doc.finish();
        assert!(doc.work().inline_bytes <= doc.source().len() as u64);
        if !success {
            println!(
                "fallback: restored hard break, stable link, inline bytes={}",
                doc.work().inline_bytes
            );
            continue;
        }
        let mut preview = TablePreview::new(
            &doc,
            TableId(0),
            TablePreviewLimits {
                rows: 1,
                source_bytes: 64,
                display_bytes: 64,
            },
        )?;
        let mut frames = 0;
        let mut charged = 0;
        loop {
            frames += 1;
            let mut budget = Budget::new(WorkLimit {
                queries: 1,
                objects: 1,
                bytes: 8,
                ..Default::default()
            });
            let step = preview.next(&doc, &mut budget)?;
            charged += budget.used().bytes;
            if let TablePreviewStep::Complete(end) = step {
                assert_eq!(end, TablePreviewEnd::SourceLimit);
                assert!(charged < 128);
                break;
            }
            assert!(frames < 100);
        }
        println!(
            "table: isolated prior break, stable link, preview frames={frames}, charged bytes={charged}"
        );
    }
    Ok(())
}
