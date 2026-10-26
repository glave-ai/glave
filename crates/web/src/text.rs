//! A response body as text the model can read: HTML as Markdown (scripts, styles and other
//! parts a reader never sees left out), other text as it is, and anything else refused.

/// Tags whose content is never part of what a reader sees.
const SKIPPED: [&str; 9] = [
    "script", "style", "noscript", "template", "svg", "canvas", "iframe", "object", "head",
];

/// What a body holds, by its content type, or by a look at it when there is none.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Html,
    Text,
}

/// The kind of a body with `content_type`, or `None` when it is not text.
pub(crate) fn kind(content_type: Option<&str>, body: &[u8]) -> Option<Kind> {
    let Some(content_type) = content_type else {
        let text = std::str::from_utf8(body).ok()?;
        let start: String = text.trim_start().chars().take(15).collect();
        let start = start.to_ascii_lowercase();
        return Some(
            if start.starts_with("<!doctype html") || start.starts_with("<html") {
                Kind::Html
            } else {
                Kind::Text
            },
        );
    };
    let essence = content_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    match essence.as_str() {
        "text/html" | "application/xhtml+xml" => Some(Kind::Html),
        "application/json"
        | "application/xml"
        | "application/javascript"
        | "application/x-yaml"
        | "application/yaml"
        | "application/toml" => Some(Kind::Text),
        other
            if other.starts_with("text/")
                || other.ends_with("+json")
                || other.ends_with("+xml") =>
        {
            Some(Kind::Text)
        }
        _ => None,
    }
}

/// The text of a body of `kind`. Bytes that are not UTF-8 are replaced.
pub(crate) fn readable(kind: Kind, body: &[u8]) -> String {
    let text = String::from_utf8_lossy(body);
    match kind {
        Kind::Text => text.into_owned(),
        Kind::Html => markdown(&text),
    }
}

/// The title a body of `kind` gives itself: an HTML page's, when it gives one.
pub(crate) fn title(kind: Kind, body: &[u8]) -> Option<String> {
    match kind {
        Kind::Html => crate::title::title(&String::from_utf8_lossy(body)),
        Kind::Text => None,
    }
}

/// HTML as Markdown, or as its text with tags dropped when it cannot be converted.
fn markdown(html: &str) -> String {
    let converter = htmd::HtmlToMarkdown::builder()
        .skip_tags(SKIPPED.to_vec())
        .build();
    match converter.convert(html) {
        Ok(markdown) => markdown,
        Err(_) => strip_tags(html),
    }
}

/// `html` with everything between `<` and `>` dropped: a last resort.
fn strip_tags(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut inside = false;
    for c in html.chars() {
        match c {
            '<' => inside = true,
            '>' => inside = false,
            c if !inside => out.push(c),
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html_becomes_markdown_without_scripts_or_styles() {
        let html = r#"<!doctype html><html><head><title>T</title><style>p{color:red}</style>
            </head><body><h1>Release notes</h1><p>The codename is <b>Juniper</b>.</p>
            <script>alert("hi")</script><ul><li>one</li><li>two</li></ul>
            <a href="https://example.com/more">more</a></body></html>"#;
        let text = readable(Kind::Html, html.as_bytes());
        assert!(text.contains("# Release notes"), "{text}");
        assert!(text.contains("**Juniper**"), "{text}");
        assert!(text.contains("one") && text.contains("two"), "{text}");
        assert!(text.contains("(https://example.com/more)"), "{text}");
        assert!(
            !text.contains("alert") && !text.contains("color:red"),
            "{text}"
        );
    }

    #[test]
    fn kinds_follow_the_content_type_or_a_look_at_the_body() {
        assert_eq!(
            kind(Some("text/html; charset=utf-8"), b""),
            Some(Kind::Html)
        );
        assert_eq!(kind(Some("text/plain"), b""), Some(Kind::Text));
        assert_eq!(kind(Some("application/ld+json"), b""), Some(Kind::Text));
        assert_eq!(kind(Some("image/png"), b""), None);
        assert_eq!(kind(Some("application/octet-stream"), b"x"), None);
        assert_eq!(kind(None, b"  <!DOCTYPE html><p>x"), Some(Kind::Html));
        assert_eq!(kind(None, b"plain words"), Some(Kind::Text));
        assert_eq!(kind(None, &[0xff, 0xfe, 0x00]), None);
    }

    #[test]
    fn the_last_resort_drops_tags() {
        assert_eq!(strip_tags("<p>a <b>b</b></p>"), "a b");
    }
}
