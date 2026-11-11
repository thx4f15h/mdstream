//! Narrow HTML output adapter shared by two chapter scores. Markdown decoding
//! belongs entirely to the public core cursors; this module only encodes output.
use mdstream::{Budget, Completion, Document, LinkId, WorkLimit};

pub const SIX_BYTES: WorkLimit = WorkLimit {
    bytes: 6,
    queries: 1,
    objects: 1,
    tiles: 0,
    recovery: 0,
    dependencies: 0,
};

fn escape(ch: char, html: &mut String) {
    match ch {
        '&' => html.push_str("&amp;"),
        '<' => html.push_str("&lt;"),
        '>' => html.push_str("&gt;"),
        '"' => html.push_str("&quot;"),
        _ => html.push(ch),
    }
}

fn url(ch: char, html: &mut String) {
    // A decoded U+0020 has the same URI spelling for every destination.
    if ch == ' ' {
        html.push_str("%20");
        return;
    }
    // Keep the chapters' other narrow URL rules: UTF-8 and backslash encoding.
    // Existing percent escapes and URI punctuation pass unchanged.
    assert!(!ch.is_ascii_control() && !ch.is_ascii_whitespace());
    assert!(
        !matches!(ch, '`' | '<' | '>' | '"'),
        "unscored URL encoding"
    );
    if !ch.is_ascii() || ch == '\\' {
        const HEX: &[u8] = b"0123456789ABCDEF";
        for byte in ch.encode_utf8(&mut [0; 4]).bytes() {
            html.push('%');
            html.push(char::from(HEX[usize::from(byte >> 4)]));
            html.push(char::from(HEX[usize::from(byte & 15)]));
        }
    } else {
        escape(ch, html);
    }
}

pub fn opening_tag(doc: &Document, id: LinkId, limit: WorkLimit) -> String {
    let mut html = String::from("<a href=\"");
    // One shared frame budget for destination and title. Exhaustion preserves
    // the cursor (including entity probe state); only the budget is renewed.
    let mut budget = Budget::new(limit);
    let mut cursor = doc.decoded_link_destination(id).unwrap();
    let start = cursor.source_position();
    let mut end = start;
    let raw_len = doc.link_destination(id).unwrap().len();
    let mut steps = 0;
    loop {
        steps += 1;
        assert!(steps <= 4 * raw_len + 2, "destination did not progress");
        match cursor.advance(&mut budget) {
            Ok(Some(chunk)) => {
                assert_eq!(chunk.source.start, end);
                assert!(doc.text(chunk.source).is_some());
                end = chunk.source.end;
                for ch in chunk.text.chars() {
                    url(ch, &mut html);
                }
            }
            Ok(None) => break,
            Err(Completion::BudgetExhausted) => {
                assert_ne!(budget.used(), WorkLimit::default());
                budget = Budget::new(limit);
            }
            Err(other) => panic!("unexpected {other:?}"),
        }
        assert_eq!(cursor.source_position(), end);
    }
    assert_eq!(end - start, raw_len);
    html.push('"');
    if let Some(mut cursor) = doc.decoded_link_title(id) {
        html.push_str(" title=\"");
        let start = cursor.source_position();
        let mut end = start;
        let raw_len = doc.link_title(id).unwrap().len();
        let mut steps = 0;
        loop {
            steps += 1;
            assert!(steps <= 4 * raw_len + 3, "title did not progress");
            match cursor.advance(&mut budget) {
                Ok(Some(chunk)) => {
                    assert_eq!(chunk.source.start, end);
                    assert!(doc.text(chunk.source).is_some());
                    end = chunk.source.end;
                    for ch in chunk.text.chars() {
                        escape(ch, &mut html);
                    }
                }
                Ok(None) => break,
                Err(Completion::BudgetExhausted) => {
                    assert_ne!(budget.used(), WorkLimit::default());
                    budget = Budget::new(limit);
                }
                Err(other) => panic!("unexpected {other:?}"),
            }
            assert_eq!(cursor.source_position(), end);
        }
        assert_eq!(end - start, raw_len);
        html.push('"');
    }
    html.push('>');
    html
}

pub fn snapshot(doc: &Document) -> Vec<Option<String>> {
    (0..doc.links().len())
        .map(|index| {
            let id = LinkId(index);
            doc.link_destination(id)?;
            let tiny = opening_tag(doc, id, SIX_BYTES);
            assert_eq!(tiny, opening_tag(doc, id, WorkLimit::unlimited()));
            Some(tiny)
        })
        .collect()
}
