use std::borrow::Cow;

use mdtui3::AnchorTarget;

pub const MAX_FRAGMENT_DESTINATION_BYTES: usize = mdtui3::MAX_DESTINATION_BYTES;
const MAX_ENTITY_BODY_BYTES: usize = 31;

// Reuse the core's generated CommonMark HTML entity table without changing it.
mod entity_names {
    include!("../../mdstream-core/src/entity_names.rs");
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FragmentDecodeError {
    NotInternal,
    TooLong,
}

/// Decode only the CommonMark destination layer before percent/UTF-8 fragment
/// decoding in the core. The returned text is bounded by the raw destination.
pub fn decode_fragment_destination(destination: &str) -> Result<Cow<'_, str>, FragmentDecodeError> {
    if !destination.starts_with('#') {
        return Err(FragmentDecodeError::NotInternal);
    }
    if destination.len() > MAX_FRAGMENT_DESTINATION_BYTES {
        return Err(FragmentDecodeError::TooLong);
    }
    if !destination
        .as_bytes()
        .iter()
        .any(|byte| matches!(byte, b'\\' | b'&'))
    {
        return Ok(Cow::Borrowed(destination));
    }

    let mut decoded = String::with_capacity(destination.len());
    let mut offset = 0;
    while offset < destination.len() {
        let rest = &destination[offset..];
        let byte = rest.as_bytes()[0];
        if byte == b'\\' && rest.as_bytes().get(1).is_some_and(u8::is_ascii_punctuation) {
            decoded.push(char::from(rest.as_bytes()[1]));
            offset += 2;
            continue;
        }
        if byte == b'&'
            && let Some(consumed) = push_entity(rest, &mut decoded)
        {
            offset += consumed;
            continue;
        }
        let ch = rest.chars().next().expect("non-empty UTF-8 suffix");
        decoded.push(ch);
        offset += ch.len_utf8();
    }
    debug_assert!(decoded.len() <= destination.len());
    Ok(Cow::Owned(decoded))
}

fn push_entity(input: &str, out: &mut String) -> Option<usize> {
    let semicolon = input.get(1..)?.find(';')? + 1;
    if semicolon == 1 || semicolon - 1 > MAX_ENTITY_BODY_BYTES {
        return None;
    }
    let body = &input[1..semicolon];
    if let Some(numeric) = body.strip_prefix('#') {
        let (digits, radix, max_digits) = match numeric.strip_prefix(['x', 'X']) {
            Some(digits) => (digits, 16, 6),
            None => (numeric, 10, 7),
        };
        if digits.is_empty()
            || digits.len() > max_digits
            || !digits.bytes().all(|byte| {
                if radix == 16 {
                    byte.is_ascii_hexdigit()
                } else {
                    byte.is_ascii_digit()
                }
            })
        {
            return None;
        }
        let value = u32::from_str_radix(digits, radix).ok()?;
        let scalar = char::from_u32(value)
            .filter(|scalar| *scalar != '\0')
            .unwrap_or('\u{fffd}');
        out.push(scalar);
        return Some(semicolon + 1);
    }
    let index = entity_names::NAMES
        .binary_search_by_key(&body, |(name, _)| *name)
        .ok()?;
    out.push_str(entity_names::NAMES[index].1);
    Some(semicolon + 1)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnchorBuildStatus {
    PendingDocument,
    BudgetExhausted,
    Complete,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FragmentActivationStatus {
    Moved,
    NoLink,
    PendingDocument,
    PendingIndex,
    BudgetExhausted,
    NotFound,
    External,
    Invalid,
    Failed,
}

impl FragmentActivationStatus {
    #[must_use]
    pub const fn is_success(self) -> bool {
        matches!(self, Self::Moved)
    }

    #[must_use]
    pub const fn shows_notice(self) -> bool {
        matches!(
            self,
            Self::PendingDocument
                | Self::PendingIndex
                | Self::BudgetExhausted
                | Self::NotFound
                | Self::Invalid
                | Self::Failed
        )
    }

    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::Moved => "已跳转",
            Self::NoLink => "无选中链接",
            Self::PendingDocument => "待文档完成",
            Self::PendingIndex => "正在建立锚点索引",
            Self::BudgetExhausted => "待锚点索引完成",
            Self::NotFound => "未找到标题锚点",
            Self::External => "外部链接",
            Self::Invalid => "无效的内部链接",
            Self::Failed => "跳转失败",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FragmentActivation {
    pub status: FragmentActivationStatus,
    pub target: Option<AnchorTarget>,
    pub moved: bool,
}

impl FragmentActivation {
    #[must_use]
    pub const fn status(status: FragmentActivationStatus) -> Self {
        Self {
            status,
            target: None,
            moved: false,
        }
    }

    #[must_use]
    pub const fn moved(target: AnchorTarget, moved: bool) -> Self {
        Self {
            status: FragmentActivationStatus::Moved,
            target: Some(target),
            moved,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode(destination: &str) -> String {
        decode_fragment_destination(destination)
            .unwrap()
            .into_owned()
    }

    #[test]
    fn decodes_backslash_punctuation_entities_and_preserves_fragment_prefix() {
        assert_eq!(decode("#a\\_b"), "#a_b");
        assert_eq!(decode("#&#x61;"), "#a");
        assert_eq!(decode("#a\\&b"), "#a&b");
        assert_eq!(decode("#&#x41;"), "#A");
        assert_eq!(decode("#a&amp;b"), "#a&b");
        assert_eq!(decode("#%E4%B8%AD"), "#%E4%B8%AD");
        assert_eq!(decode("#"), "#");
    }

    #[test]
    fn malformed_entities_stay_literal_and_limits_are_explicit() {
        assert_eq!(decode("#&amp"), "#&amp");
        assert_eq!(decode("#&not-an-entity;"), "#&not-an-entity;");
        assert_eq!(decode("#a\\q"), "#a\\q");
        assert_eq!(
            decode_fragment_destination("https://example.com/"),
            Err(FragmentDecodeError::NotInternal)
        );
        assert_eq!(
            decode_fragment_destination(""),
            Err(FragmentDecodeError::NotInternal)
        );
        let too_long = format!("#{}", "a".repeat(MAX_FRAGMENT_DESTINATION_BYTES));
        assert_eq!(
            decode_fragment_destination(&too_long),
            Err(FragmentDecodeError::TooLong)
        );
    }
}
