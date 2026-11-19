use mdstream::{Budget, Completion, UriEncoder, UriWork, WorkLimit};
fn tiny() -> Budget {
    Budget::new(WorkLimit {
        bytes: 6,
        queries: 1,
        objects: 1,
        ..WorkLimit::default()
    })
}
fn encode(input: &str) -> String {
    let mut cursor = UriEncoder::new(input);
    let mut out = String::new();
    for _ in 0..10000 {
        let mut b = tiny();
        match cursor.advance(&mut b) {
            Ok(Some(chunk)) => out.push_str(chunk.text),
            Ok(None) => return out,
            Err(Completion::BudgetExhausted) => assert_ne!(b.used(), WorkLimit::default()),
            Err(e) => panic!("{e:?}"),
        }
    }
    panic!("no completion")
}
#[test]
fn independent_reference_vectors_and_existing_percent_spelling() {
    for (raw, expected) in [
        (
            "https://example.org/路径?q=é&x=%2f#片",
            "https://example.org/%E8%B7%AF%E5%BE%84?q=%C3%A9&x=%2f#%E7%89%87",
        ),
        ("../a b\\c?x=1&y=2", "../a%20b%5Cc?x=1&y=2"),
        ("a%20b%zz%", "a%20b%25zz%25"),
        ("\t\n\0\u{85}", "%09%0A%00%C2%85"),
        ("mailto:a+b@example.org", "mailto:a+b@example.org"),
        ("data:image/png;base64,AA==", "data:image/png;base64,AA=="),
        ("#é", "#%C3%A9"),
        ("", ""),
    ] {
        assert_eq!(encode(raw), expected)
    }
}
#[test]
fn no_probe_before_admission_and_percent_probe_is_resumable() {
    let input = String::from("%2f🙂");
    let mut e = UriEncoder::new(input);
    for _ in 0..8 {
        assert!(matches!(
            e.advance(&mut Budget::new(WorkLimit::default())),
            Err(Completion::BudgetExhausted)
        ));
        assert_eq!(e.work(), UriWork::default());
        assert_eq!(e.input_position(), 0);
    }
    let mut out = String::new();
    let mut probes = 0;
    loop {
        let mut b = tiny();
        match e.advance(&mut b) {
            Ok(Some(c)) => out.push_str(c.text),
            Ok(None) => break,
            Err(_) => {}
        }
        probes += b.used().bytes;
        assert!(b.used().queries <= 1 && b.used().objects <= 1 && b.used().bytes <= 6);
    }
    assert_eq!(out, "%2f%F0%9F%99%82");
    assert_eq!(e.work().input_bytes, 9);
    assert_eq!(e.work().output_bytes, out.len());
    assert_eq!(probes, 9 + out.len());
    assert_eq!(e.input(), "%2f🙂");
}
#[test]
fn all_ascii_bytes_have_independent_expected_escaping() {
    for byte in 0..=127u8 {
        let s = char::from(byte).to_string();
        let expected = if byte.is_ascii_alphanumeric() || b"-._~:/?#[]@!$&'()*+,;=".contains(&byte)
        {
            s.clone()
        } else {
            format!("%{byte:02X}")
        };
        assert_eq!(encode(&s), expected, "byte {byte}");
    }
}

#[test]
fn owned_and_borrowed_inputs_keep_actual_bytes_and_ranges_across_yields() {
    use std::borrow::Cow;
    let borrowed = String::from("a%2f🙂");
    for input in [
        Cow::Borrowed(borrowed.as_str()),
        Cow::Owned(borrowed.clone()),
    ] {
        let original = input.as_ptr();
        let mut encoder = UriEncoder::new(input);
        assert_eq!(
            encoder.input().as_ptr(),
            original,
            "constructor must move or borrow, not copy"
        );
        let mut out = String::new();
        let mut ranges = Vec::new();
        for _ in 0..1000 {
            let before = (encoder.input_position(), encoder.work());
            assert!(matches!(
                encoder.advance(&mut Budget::new(WorkLimit::default())),
                Err(Completion::BudgetExhausted)
            ));
            assert_eq!((encoder.input_position(), encoder.work()), before);
            assert_eq!(encoder.input(), "a%2f🙂");
            match encoder.advance(&mut tiny()) {
                Ok(Some(chunk)) => {
                    out.push_str(chunk.text);
                    ranges.push(chunk.input)
                }
                Ok(None) => break,
                Err(Completion::BudgetExhausted) => {}
                Err(e) => panic!("{e:?}"),
            }
            if out == "a%2f%F0%9F%99%82" {
                break;
            }
        }
        assert_eq!(out, "a%2f%F0%9F%99%82");
        assert_eq!(
            ranges,
            (0..borrowed.len()).map(|i| i..i + 1).collect::<Vec<_>>()
        );
        assert!(
            encoder
                .advance(&mut Budget::new(WorkLimit::default()))
                .unwrap()
                .is_none()
        );
    }
}
