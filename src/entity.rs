// Semicolon-terminated references only, following CommonMark 0.31.2 §2.5.
// The generated named table is fixed at build time; parsing never uses a network.
mod names {
    include!("entity_names.rs");
}

pub(crate) const MAX_ENTITY_BODY: usize = 31;

pub(crate) fn decode(body: &str) -> Option<String> {
    if let Some(numeric) = body.strip_prefix('#') {
        let (digits, radix, max) = match numeric.strip_prefix(['x', 'X']) {
            Some(digits) => (digits, 16, 6),
            None => (numeric, 10, 7),
        };
        if digits.is_empty()
            || digits.len() > max
            || !digits.bytes().all(|b| {
                if radix == 16 {
                    b.is_ascii_hexdigit()
                } else {
                    b.is_ascii_digit()
                }
            })
        {
            return None;
        }
        let value = u32::from_str_radix(digits, radix).ok()?;
        let scalar = char::from_u32(value)
            .filter(|c| *c != '\0')
            .unwrap_or('\u{fffd}');
        Some(scalar.to_string())
    } else {
        let index = names::NAMES
            .binary_search_by_key(&body, |(name, _)| *name)
            .ok()?;
        Some(names::NAMES[index].1.to_owned())
    }
}
