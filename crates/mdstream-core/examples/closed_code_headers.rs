//! Isolate one inherited code closure until an inert suffix becomes a table.
use mdstream::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    for success in [false, true] {
        let mut doc = Document::new();
        doc.append("> - [prior](u) ` old\r\n>   [head](v) &amp; end` ")?;
        let original = *doc.link_info(LinkId(0)).unwrap();
        for _ in 0..1000 {
            doc.append(&"x".repeat(1000))?;
        }
        assert_eq!(doc.links().len(), 1);
        assert!(
            !doc.runs_in(Span::new(0, 27))
                .unwrap()
                .any(|r| r.style.contains(Style::CODE))
        );
        let before = doc.work();
        doc.append("|[tail](w)\r\n")?;
        assert_eq!(doc.work().inline_bytes, before.inline_bytes);
        doc.append(if success {
            ">   :--|--:\n"
        } else {
            ">   wrong\n"
        })?;
        assert_eq!(*doc.link_info(LinkId(0)).unwrap(), original);
        assert_eq!(doc.tables().len(), usize::from(success));
        assert_eq!(doc.links().len(), if success { 3 } else { 2 });
        doc.finish();
        if !success {
            assert!(
                doc.runs_in(Span::new(0, 27))
                    .unwrap()
                    .any(|r| r.style.contains(Style::CODE))
            );
            assert!(doc.work().inline_bytes <= doc.source().len() as u64);
            println!(
                "fallback: original code whitespace mapping committed once, old LinkId retained"
            );
            continue;
        }
        assert!(doc.work().inline_bytes <= doc.source().len() as u64 + doc.work().table_bytes);
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
            "table: isolated code discarded, cell objects published once; preview frames={frames}, charged bytes={charged}"
        );
    }
    Ok(())
}
