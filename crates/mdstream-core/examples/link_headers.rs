//! Keep live objects while resolving a late header after an unclosed image label.
use mdstream::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    for success in [false, true] {
        let mut doc = Document::new();
        doc.append("[prior](u) ![old  \r\n**local** &amp; ")?;
        let original = *doc.link_info(LinkId(0)).unwrap();
        let chunk = "x".repeat(1000);
        for _ in 0..1000 {
            doc.append(&chunk)?;
        }
        assert_eq!(doc.links().len(), 1);
        let before = doc.work();
        doc.append("|end](image) [tail](w)\r\n")?;
        assert_eq!(doc.work().inline_bytes, before.inline_bytes);
        doc.append(if success { ":--|--:\n" } else { "wrong\n" })?;
        assert_eq!(*doc.link_info(LinkId(0)).unwrap(), original);
        assert_eq!(doc.tables().len(), usize::from(success));
        assert_eq!(doc.links().len(), if success { 2 } else { 3 });
        doc.finish();
        assert!(doc.work().inline_bytes <= doc.source().len() as u64);
        if !success {
            let image = doc.link_info(LinkId(1)).unwrap();
            assert_eq!(image.kind, LinkKind::Image);
            assert_eq!(
                doc.text(Span::new(image.source.start, image.source.start + 2)),
                Some("![")
            );
            assert_eq!(doc.link_destination(LinkId(1)), Some("image"));
            println!("fallback: original image label closes, prior LinkId retained");
            continue;
        }
        assert_eq!(doc.link_destination(LinkId(1)), Some("w"));
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
            "table: inherited label discarded, preview frames={frames}, charged bytes={charged}"
        );
    }
    Ok(())
}
