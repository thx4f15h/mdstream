use mdtui3::{HeightIndex, Utf8Document};

#[test]
fn every_byte_split_and_invalid_sequence_matches_lossy_decoding() {
    let inputs: &[&[u8]] = &[
        "# 中文🙂\n**é**".as_bytes(),
        b"a\xf0\x9f\x92\xa9z",
        b"\xe0\x80a\xed\xa0\x80\xff\xc2",
        b"\xf0\x9f\x92",
        b"\xc2a\xf4\x90\x80\x80",
        b"",
    ];
    for input in inputs {
        for step in 1..=input.len().max(1) {
            let mut stream = Utf8Document::new();
            let mut heights = HeightIndex::default();
            for chunk in input.chunks(step) {
                let update = stream.append(chunk).unwrap();
                heights.apply(stream.document(), &update).unwrap();
                assert!(stream.pending_bytes().len() <= 3);
            }
            let update = stream.finish();
            heights.apply(stream.document(), &update).unwrap();
            assert_eq!(stream.document().source(), String::from_utf8_lossy(input));
            assert!(stream.pending_bytes().is_empty());
            assert!(stream.append(b"x").is_err());
            let update = stream.finish();
            heights.apply(stream.document(), &update).unwrap();
        }
    }
}
