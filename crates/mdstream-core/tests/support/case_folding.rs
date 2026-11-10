//! Independent oracle: read upstream records, never the generated Rust table or
//! the host Unicode case mapping. Keep parsing/normalization separate from core.
use std::collections::BTreeMap;

pub fn mappings() -> BTreeMap<char, String> {
    let raw = include_str!("../../../../data/unicode/17.0.0/CaseFolding.txt");
    assert!(raw.starts_with("# CaseFolding-17.0.0.txt\n"));
    let mut mappings = BTreeMap::new();
    for record in raw.lines() {
        let body = record.split('#').next().unwrap().trim();
        if body.is_empty() {
            continue;
        }
        let fields: Vec<_> = body.split(';').map(str::trim).collect();
        if matches!(fields[1], "C" | "F") {
            let scalar = |s| char::from_u32(u32::from_str_radix(s, 16).unwrap()).unwrap();
            let value = fields[2].split_whitespace().map(scalar).collect();
            assert!(mappings.insert(scalar(fields[0]), value).is_none());
        }
    }
    assert_eq!(mappings.len(), 1585);
    mappings
}

pub fn normalize(label: &str, mappings: &BTreeMap<char, String>) -> Option<String> {
    if label.len() > 999 {
        return None;
    }
    let words: Vec<String> = label
        .split_whitespace()
        .map(|word| {
            word.chars()
                .map(|c| mappings.get(&c).cloned().unwrap_or_else(|| c.to_string()))
                .collect()
        })
        .collect();
    (!words.is_empty()).then(|| words.join(" "))
}
