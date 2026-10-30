//! Delay a closed shortcut's object and symbol effects until a late header resolves.
use mdtui3::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    for success in [false, true] {
        let mut doc = Document::new();
        doc.append("> - [prior](u) ![old\r\n>   **head** end] ")?;
        let original = *doc.link_info(LinkId(0)).unwrap();
        let references = doc.references().to_vec();
        for _ in 0..1000 {
            doc.append(&"x".repeat(1000))?;
        }
        assert_eq!(doc.links().len(), 1);
        assert_eq!(doc.references(), references);
        let before = doc.work();
        doc.append("|[tail](v)\r\n")?;
        assert_eq!(doc.work().inline_bytes, before.inline_bytes);
        let update = doc.append(if success {
            ">   :--|--:\n"
        } else {
            ">   wrong\n"
        })?;
        assert_eq!(update.new_links, 1..if success { 2 } else { 3 });
        assert_eq!(*doc.link_info(LinkId(0)).unwrap(), original);
        assert_eq!(doc.tables().len(), usize::from(success));
        // Both paths reuse the prefix's live semantics: no full-row reparse.
        assert!(doc.work().inline_bytes - before.inline_bytes < 30);
        if !success {
            let image = *doc.link_info(LinkId(1)).unwrap();
            assert_eq!(image.kind, LinkKind::Image);
            assert_eq!(doc.link_destination(LinkId(1)), None);
            doc.append("\n[old **head** end]: /late 'title'\n")?;
            assert_eq!(*doc.link_info(LinkId(1)).unwrap(), image);
            assert_eq!(doc.link_destination(LinkId(1)), Some("/late"));
            println!("fallback: shortcut committed once; late definition retains the image ID");
            doc.finish();
            continue;
        }
        doc.finish();
        assert!(
            !doc.references()
                .iter()
                .any(|r| r.label == "old **head** end")
        );
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
                break;
            }
            assert!(frames < 100);
        }
        println!(
            "table: no discarded shortcut symbol or ID; preview frames={frames}, charged bytes={charged}"
        );
    }
    Ok(())
}
