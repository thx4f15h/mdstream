//! Resolve a late table header while an inherited math candidate is still opaque.
use mdstream::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    for success in [false, true] {
        let mut doc = Document::new();
        doc.append("$prior$ [prior](u) $$ old\r\n[head](v) $中🙂$ &amp; ")?;
        let original = *doc.math(MathId(0)).unwrap();
        for _ in 0..1000 {
            doc.append(&"x".repeat(1000))?;
        }
        assert_eq!(doc.maths().len(), 1); // Still inside the old math candidate.
        let before = doc.work();
        doc.append("|$tail$ [tail](w)\r\n")?;
        assert_eq!(doc.work().inline_bytes, before.inline_bytes);
        doc.append(if success { ":--|--:\n" } else { "wrong\n" })?;
        assert_eq!(*doc.math(MathId(0)).unwrap(), original);
        assert_eq!(doc.tables().len(), usize::from(success));
        assert_eq!(doc.maths().len(), if success { 3 } else { 1 });
        if !success {
            doc.append("end$$ [after](z)\n")?;
            assert_eq!(doc.maths().len(), 2);
            assert_eq!(doc.links().len(), 2);
            doc.finish();
            assert!(doc.work().inline_bytes <= doc.source().len() as u64);
            println!("fallback: old math closes, original MathId retained");
            continue;
        }
        doc.finish();
        assert!(doc.work().inline_bytes <= doc.source().len() as u64 + doc.work().table_bytes);
        let mut content = doc.math_content(MathId(1)).unwrap();
        let mut budget = Budget::new(WorkLimit {
            queries: 1,
            objects: 1,
            bytes: 7,
            ..Default::default()
        });
        assert_eq!(content.advance(&mut budget).unwrap().unwrap().text, "中🙂");
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
            "table: cell MathIds published once, preview frames={frames}, charged bytes={charged}"
        );
    }
    Ok(())
}
