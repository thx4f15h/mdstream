use std::borrow::Cow;

use mdtui3::{Document, LinkId, LinkTarget};

use crate::cli::LinkMode;
use crate::screen::StyleRole;
use crate::theme::MONOKAI;

pub const MAX_OSC8_DESTINATION_BYTES: usize = 4096;

const OSC8_CLOSE: &str = "\x1b]8;;\x1b\\";

/// Source bytes inspected when preparing one bounded terminal target.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DestinationStats {
    pub source_bytes: usize,
    pub too_long: usize,
}

pub fn osc8_destination(
    document: &Document,
    id: LinkId,
) -> (Option<Cow<'_, str>>, DestinationStats) {
    let Some(raw) = document.link_destination(id) else {
        return (None, DestinationStats::default());
    };
    let www = matches!(document.links().get(id.0), Some(LinkTarget::Www(_)));
    if raw.len().saturating_add(if www { 7 } else { 0 }) > MAX_OSC8_DESTINATION_BYTES {
        return (
            None,
            DestinationStats {
                source_bytes: 0,
                too_long: 1,
            },
        );
    }
    let stats = DestinationStats {
        source_bytes: raw.len(),
        too_long: 0,
    };
    if raw.is_empty() || raw.chars().any(char::is_control) {
        return (None, stats);
    }
    let target = if www {
        Cow::Owned(format!("http://{raw}"))
    } else {
        Cow::Borrowed(raw)
    };
    (Some(target), stats)
}
pub(crate) fn osc8_body_destination(
    body: mdtui3::BodyView<'_>,
    id: mdtui3::BodyId<LinkId>,
) -> (Option<String>, DestinationStats) {
    let Ok(Some(raw)) = body.raw_link_destination(id) else {
        return (None, DestinationStats::default());
    };
    let www = body
        .with_renderer(id.body(), body.host_revision(), |native| {
            matches!(native.links().get(id.local().0), Some(LinkTarget::Www(_)))
        })
        .unwrap_or(false);
    if raw.text.len().saturating_add(if www { 7 } else { 0 }) > MAX_OSC8_DESTINATION_BYTES {
        return (
            None,
            DestinationStats {
                source_bytes: 0,
                too_long: 1,
            },
        );
    }
    let stats = DestinationStats {
        source_bytes: raw.text.len(),
        too_long: 0,
    };
    if raw.text.is_empty() || raw.text.chars().any(char::is_control) {
        return (None, stats);
    }
    (
        Some(if www {
            format!("http://{}", raw.text)
        } else {
            raw.text.to_owned()
        }),
        stats,
    )
}

/// Encode one visible link label as an independently closed, single-line ANSI segment.
///
/// The destination is taken verbatim from [`Document::link_destination`], so email
/// autolinks do not gain a `mailto:` prefix and link titles are never emitted.
#[must_use]
pub fn encode_link(
    document: &Document,
    link_id: LinkId,
    visible_text: &str,
    style: StyleRole,
    link_mode: LinkMode,
    allow_osc8: bool,
) -> Option<String> {
    document.link_info(link_id)?;
    let styled = MONOKAI.encode_segment(style.with(StyleRole::LINK), visible_text);
    if !allow_osc8 || link_mode == LinkMode::NoOsc8 {
        return Some(styled);
    }
    let (destination, _) = osc8_destination(document, link_id);
    let Some(destination) = destination else {
        return Some(styled);
    };
    Some(format!("\x1b]8;;{destination}\x1b\\{styled}{OSC8_CLOSE}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(source: &str) -> Document {
        let mut document = Document::new();
        document.append(source).unwrap();
        document.finish();
        document
    }

    fn encode(
        document: &Document,
        link_id: LinkId,
        text: &str,
        style: StyleRole,
        mode: LinkMode,
        allow_osc8: bool,
    ) -> String {
        encode_link(document, link_id, text, style, mode, allow_osc8).expect("known link")
    }

    fn link_style(text: &str) -> String {
        MONOKAI.encode_segment(StyleRole::LINK, text)
    }

    #[test]
    fn http_and_https_emit_osc8_outside_self_contained_sgr() {
        let document = parse("[http](http://example.com) [https](https://example.com)\n");
        let http = encode(
            &document,
            LinkId(0),
            "http",
            StyleRole::DEFAULT,
            LinkMode::Default,
            true,
        );
        let https = encode(
            &document,
            LinkId(1),
            "https",
            StyleRole::DEFAULT,
            LinkMode::Default,
            true,
        );

        assert_eq!(
            http,
            format!(
                "\x1b]8;;http://example.com\x1b\\{}{OSC8_CLOSE}",
                link_style("http")
            )
        );
        assert_eq!(
            https,
            format!(
                "\x1b]8;;https://example.com\x1b\\{}{OSC8_CLOSE}",
                link_style("https")
            )
        );
        assert!(http.contains("\x1b[4m\x1b[38;2;139;233;253mhttp\x1b[0m"));
        assert_eq!(
            encode(
                &document,
                LinkId(0),
                "http",
                StyleRole::DEFAULT,
                LinkMode::OpenExternal,
                true,
            ),
            http
        );
    }

    #[test]
    fn email_uses_raw_destination_while_link_url_adds_mailto() {
        let document = parse("<user@example.com>\n");
        let link_id = LinkId(0);
        assert_eq!(document.link_destination(link_id), Some("user@example.com"));
        assert_eq!(
            document.link_url(link_id).as_deref(),
            Some("mailto:user@example.com")
        );

        let bytes = encode(
            &document,
            link_id,
            "user@example.com",
            StyleRole::DEFAULT,
            LinkMode::Default,
            true,
        );
        assert!(bytes.starts_with("\x1b]8;;user@example.com\x1b\\"));
        assert!(bytes.ends_with(OSC8_CLOSE));
        assert!(!bytes.contains("mailto:"));
    }

    #[test]
    fn empty_destination_has_no_osc8() {
        let document = parse("[empty]()\n");
        let link_id = LinkId(0);
        assert_eq!(document.link_destination(link_id), Some(""));

        let bytes = encode(
            &document,
            link_id,
            "empty",
            StyleRole::DEFAULT,
            LinkMode::Default,
            true,
        );
        assert_eq!(bytes, link_style("empty"));
        assert!(!bytes.contains("\x1b]8;"));
    }

    #[test]
    fn title_is_never_emitted_and_destination_stays_raw() {
        let document = parse("[entity](https://example.com/?a=&amp;b \"Ignored title\")\n");
        let link_id = LinkId(0);
        assert_eq!(document.link_title(link_id), Some("Ignored title"));
        assert_eq!(
            document.link_destination(link_id),
            Some("https://example.com/?a=&amp;b")
        );

        let bytes = encode(
            &document,
            link_id,
            "entity",
            StyleRole::DEFAULT,
            LinkMode::Default,
            true,
        );
        assert!(bytes.contains("\x1b]8;;https://example.com/?a=&amp;b\x1b\\"));
        assert!(!bytes.contains("Ignored title"));
    }

    #[test]
    fn inline_reference_collapsed_and_email_forms_encode_identically() {
        let inline = parse("[site](https://example.com)\n");
        let full_reference = parse("[site][r]\n\n[r]: https://example.com\n");
        let collapsed = parse("[site][]\n\n[site]: https://example.com\n");
        let expected = encode(
            &inline,
            LinkId(0),
            "site",
            StyleRole::DEFAULT,
            LinkMode::Default,
            true,
        );
        assert_eq!(
            encode(
                &full_reference,
                LinkId(0),
                "site",
                StyleRole::DEFAULT,
                LinkMode::Default,
                true,
            ),
            expected
        );
        assert_eq!(
            encode(
                &collapsed,
                LinkId(0),
                "site",
                StyleRole::DEFAULT,
                LinkMode::Default,
                true,
            ),
            expected
        );

        let direct_email = parse("[mail](user@example.com)\n");
        let automatic_email = parse("<user@example.com>\n");
        let direct_bytes = encode(
            &direct_email,
            LinkId(0),
            "user@example.com",
            StyleRole::DEFAULT,
            LinkMode::Default,
            true,
        );
        assert_eq!(
            encode(
                &automatic_email,
                LinkId(0),
                "user@example.com",
                StyleRole::DEFAULT,
                LinkMode::Default,
                true,
            ),
            direct_bytes
        );
    }

    #[test]
    fn adjacent_links_open_and_close_independently() {
        let document = parse("[one](https://one.example) and [two](https://two.example)\n");
        let first = encode(
            &document,
            LinkId(0),
            "one",
            StyleRole::DEFAULT,
            LinkMode::Default,
            true,
        );
        let second = encode(
            &document,
            LinkId(1),
            "two",
            StyleRole::DEFAULT,
            LinkMode::Default,
            true,
        );
        assert!(first.contains("https://one.example"));
        assert!(!first.contains("https://two.example"));
        assert!(second.contains("https://two.example"));
        assert!(!second.contains("https://one.example"));
        assert_eq!(first.matches(OSC8_CLOSE).count(), 1);
        assert_eq!(second.matches(OSC8_CLOSE).count(), 1);

        let joined = format!("{first} {second}");
        assert_eq!(joined.matches("\x1b]8;;https").count(), 2);
        assert_eq!(joined.matches(OSC8_CLOSE).count(), 2);
    }

    #[test]
    fn disabled_osc8_keeps_style_bytes_identical() {
        let document = parse("[site](https://example.com)\n");
        let styled = link_style("site");
        let by_mode = encode(
            &document,
            LinkId(0),
            "site",
            StyleRole::DEFAULT,
            LinkMode::NoOsc8,
            true,
        );
        let by_allow = encode(
            &document,
            LinkId(0),
            "site",
            StyleRole::DEFAULT,
            LinkMode::Default,
            false,
        );
        assert_eq!(by_mode, styled);
        assert_eq!(by_allow, styled);
        assert!(!by_mode.contains("\x1b]8;"));
        assert!(!by_allow.contains("\x1b]8;"));
    }

    #[test]
    fn inline_attributes_keep_p05_order_inside_osc8() {
        let document = parse("[link](https://example.com)\n");
        let style = StyleRole::STRONG
            .with(StyleRole::EMPHASIS)
            .with(StyleRole::STRIKE)
            .with(StyleRole::MATH);
        let bytes = encode(&document, LinkId(0), "link", style, LinkMode::Default, true);
        assert_eq!(
            bytes,
            concat!(
                "\x1b]8;;https://example.com\x1b\\",
                "\x1b[1m\x1b[3m\x1b[4m\x1b[9m\x1b[38;2;139;233;253m",
                "link",
                "\x1b[0m",
                "\x1b]8;;\x1b\\"
            )
        );
        assert_eq!(bytes.matches("\x1b[0m").count(), 1);
        assert_eq!(bytes.matches(OSC8_CLOSE).count(), 1);
    }

    #[test]
    fn unknown_link_returns_none() {
        let document = parse("[site](https://example.com)\n");
        assert!(
            encode_link(
                &document,
                LinkId(99),
                "site",
                StyleRole::DEFAULT,
                LinkMode::Default,
                true,
            )
            .is_none()
        );
    }
}
