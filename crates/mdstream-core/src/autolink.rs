//! Bounded angle-autolink recognition. A candidate is inspected once at closure;
//! failed or unfinished candidates stay literal and are never replayed.
pub const MAX_AUTOLINK_BYTES: usize = 4096;

pub(crate) enum Target {
    Uri,
    Email,
}

pub(crate) fn classify(text: &str) -> Option<Target> {
    if let Some((scheme, _)) = text.split_once(':')
        && (2..=32).contains(&scheme.len())
        && scheme.as_bytes()[0].is_ascii_alphabetic()
        && scheme
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"+.-".contains(&b))
    {
        return Some(Target::Uri);
    }
    let (local, domain) = text.split_once('@')?;
    if local.is_empty()
        || !local
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b".!#$%&'*+/=?^_`{|}~-".contains(&b))
    {
        return None;
    }
    if !domain.split('.').all(|label| {
        !label.is_empty()
            && label.len() <= 63
            && label.as_bytes()[0].is_ascii_alphanumeric()
            && label.as_bytes()[label.len() - 1].is_ascii_alphanumeric()
            && label
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-')
    }) {
        return None;
    }
    Some(Target::Email)
}
