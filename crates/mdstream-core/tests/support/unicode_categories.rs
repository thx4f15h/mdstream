//! Independent oracle: expand official general-category records into a dense
//! scalar map. Never reads the generated ranges or the host Unicode database.
use std::sync::OnceLock;

pub fn classes() -> &'static [u8] {
    static CLASSES: OnceLock<Vec<u8>> = OnceLock::new();
    CLASSES.get_or_init(|| {
        let raw = include_str!("../../../../data/unicode/17.0.0/DerivedGeneralCategory.txt");
        assert!(raw.starts_with("# DerivedGeneralCategory-17.0.0.txt\n"));
        let mut result = vec![0; 0x110000];
        for record in raw.lines() {
            let body = record.split('#').next().unwrap().trim();
            if body.is_empty() {
                continue;
            }
            let (codes, category) = body.split_once(';').unwrap();
            let value = match category.trim() {
                "Zs" => 1,
                s if s.starts_with('P') || s.starts_with('S') => 2,
                _ => continue,
            };
            let ends: Vec<_> = codes
                .trim()
                .split("..")
                .map(|s| u32::from_str_radix(s, 16).unwrap())
                .collect();
            result[ends[0] as usize..=(*ends.last().unwrap()) as usize].fill(value);
        }
        for ch in ['\t', '\n', '\x0c', '\r'] {
            result[ch as usize] = 1;
        }
        result
    })
}
