//! Isolate a closed inherited formula without allocating a tentative MathId.
use mdstream::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    for success in [false, true] {
        let mut doc = Document::new();
        doc.append("> - $prior$ $$ old\r\n>   $head$ end$$ ")?;
        let original = *doc.math(MathId(0)).unwrap();
        for _ in 0..1000 {
            doc.append(&"x".repeat(1000))?;
        }
        assert_eq!(doc.maths(), &[original]);
        let before = doc.work();
        doc.append("|$tail$\r\n")?;
        assert_eq!(doc.work().inline_bytes, before.inline_bytes);
        let update = doc.append(if success {
            ">   :--|--:\n"
        } else {
            ">   wrong\n"
        })?;
        assert_eq!(update.new_maths, 1..3);
        assert_eq!(*doc.math(MathId(0)).unwrap(), original);
        assert_eq!(doc.tables().len(), usize::from(success));
        assert_eq!(doc.text(doc.math(MathId(2)).unwrap().content), Some("tail"));
        doc.finish();
        if !success {
            let mut reader = doc.math_payload(MathId(1)).unwrap();
            let mut budget = Budget::new(WorkLimit::unlimited());
            let mut text = String::new();
            while let Some(chunk) = reader.advance(&mut budget).unwrap() {
                text.push_str(chunk.text);
            }
            assert_eq!(text, " old\r\n$head$ end");
            assert!(doc.work().inline_bytes <= doc.source().len() as u64);
            println!(
                "fallback: old formula committed once, mapped payload and stable MathId retained"
            );
            continue;
        }
        assert_eq!(doc.text(doc.math(MathId(1)).unwrap().content), Some("head"));
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
            "table: old formula discarded, cell objects published once; preview frames={frames}, charged bytes={charged}"
        );
    }
    Ok(())
}
