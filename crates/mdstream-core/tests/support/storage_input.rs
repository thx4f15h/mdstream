// Shared deterministic inputs; keep historical storage measurements reproducible.
pub fn fingerprint(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325, |h, b| {
        (h ^ u64::from(*b)).wrapping_mul(0x100000001b3)
    })
}

pub fn source(case: &str, size: usize) -> String {
    match case {
        "plain" => "x".repeat(size),
        "dense" => "*a* **b** ~~c~~ [d](e) `f`\n".repeat(size.div_ceil(26)),
        "containers" => "> - **bold** text\n\n".repeat(size.div_ceil(19)),
        _ => panic!("unknown storage case"),
    }
}
