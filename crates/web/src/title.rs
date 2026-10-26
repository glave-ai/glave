//! The title an HTML page gives itself, read from its `<title>` as a person would see it in a
//! tab: its character references read, its spaces made single, cut short when long.

/// The most characters of a title kept.
const MOST: usize = 200;

/// The title of `html`, from the first `<title>` before its `<body>`; `None` when it has none,
/// or one with no words.
pub(crate) fn title(html: &str) -> Option<String> {
    // Lowercasing ASCII keeps every byte where it was, so its offsets hold in `html`.
    let lower = html.to_ascii_lowercase();
    let head = lower.find("<body").unwrap_or(lower.len());
    let start = opened(&lower)?;
    if start > head {
        return None;
    }
    let end = start.saturating_add(lower.get(start..)?.find("</title")?);
    let text: String = references(html.get(start..end)?)
        .chars()
        .filter_map(|c| match c {
            c if c.is_whitespace() => Some(' '),
            c if hidden(c) => None,
            c => Some(c),
        })
        .collect();
    let title = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if title.is_empty() {
        return None;
    }
    if title.chars().count() > MOST {
        let kept: String = title.chars().take(MOST.saturating_sub(1)).collect();
        return Some(format!("{}…", kept.trim_end()));
    }
    Some(title)
}

/// Whether `c` shows nothing of its own: a control character, or one that only changes how the
/// text around it is laid out (the marks and overrides of bidirectional text, and the other
/// invisible format characters), which could make an address shown beside the title read as
/// another.
fn hidden(c: char) -> bool {
    c.is_control()
        || matches!(
            c,
            '\u{00AD}'
                | '\u{061C}'
                | '\u{180E}'
                | '\u{200B}'..='\u{200F}'
                | '\u{202A}'..='\u{202E}'
                | '\u{2060}'..='\u{206F}'
                | '\u{FEFF}'
                | '\u{FFF9}'..='\u{FFFB}'
        )
}

/// Where the text of the first `<title>` tag in `lower` starts: just after its `>`.
fn opened(lower: &str) -> Option<usize> {
    let mut from = 0_usize;
    loop {
        let at = from.saturating_add(lower.get(from..)?.find("<title")?);
        let after = at.saturating_add("<title".len());
        match lower.as_bytes().get(after) {
            Some(b'>') => return Some(after.saturating_add(1)),
            Some(c) if c.is_ascii_whitespace() => {
                let close = lower.get(after..)?.find('>')?;
                return Some(after.saturating_add(close).saturating_add(1));
            }
            // `<titles>` or the like: another tag.
            _ => from = after,
        }
    }
}

/// `text` with its character references read: numeric ones, and the named ones titles use.
/// Any other `&` stays as it is.
fn references(text: &str) -> String {
    let mut pieces = text.split('&');
    let mut out = pieces.next().unwrap_or_default().to_owned();
    for piece in pieces {
        let read = piece
            .split_once(';')
            .and_then(|(name, rest)| Some((character(name)?, rest)));
        if let Some((c, rest)) = read {
            out.push(c);
            out.push_str(rest);
        } else {
            out.push('&');
            out.push_str(piece);
        }
    }
    out
}

/// The character a reference names, without its `&` and `;`.
fn character(name: &str) -> Option<char> {
    if let Some(number) = name.strip_prefix('#') {
        let code = match number.strip_prefix(['x', 'X']) {
            Some(hex) => u32::from_str_radix(hex, 16).ok()?,
            None => number.parse().ok()?,
        };
        return char::from_u32(code);
    }
    Some(match name {
        "amp" => '&',
        "lt" => '<',
        "gt" => '>',
        "quot" => '"',
        "apos" => '\'',
        "nbsp" => ' ',
        "ndash" => '–',
        "mdash" => '—',
        "lsquo" => '‘',
        "rsquo" => '’',
        "ldquo" => '“',
        "rdquo" => '”',
        "hellip" => '…',
        "middot" => '·',
        "bull" => '•',
        "laquo" => '«',
        "raquo" => '»',
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_title_reads_as_a_tab_shows_it() {
        let html = "<!doctype html><HTML><head><meta charset=utf-8>\
            <Title lang=en>\n  Rust &amp; Go &mdash; a&#39;s &#x2014; notes\t</title></head>\
            <body><h1>Other</h1></body></html>";
        assert_eq!(title(html).as_deref(), Some("Rust & Go — a's — notes"));
    }

    #[test]
    fn a_page_without_one_has_none() {
        assert_eq!(title("<html><body><p>No title</p></body></html>"), None);
        assert_eq!(title("<html><head><title>  </title></head></html>"), None);
        assert_eq!(title("<title>never closed"), None);
        // An SVG's title in the body names a picture, not the page.
        let svg = "<html><body><svg><title>A chart</title></svg></body></html>";
        assert_eq!(title(svg), None);
    }

    #[test]
    fn another_tag_that_starts_alike_is_not_it() {
        let html = "<head><titles>x</titles><title>The page</title></head>";
        assert_eq!(title(html).as_deref(), Some("The page"));
    }

    #[test]
    fn unknown_references_stay_and_a_long_title_is_cut() {
        let html = "<title>R&D &bogus; &#xZZ; &#1114112; 5 & 6</title>";
        assert_eq!(
            title(html).as_deref(),
            Some("R&D &bogus; &#xZZ; &#1114112; 5 & 6")
        );
        let long = format!("<title>{}</title>", "word ".repeat(100));
        let cut = title(&long).unwrap_or_default();
        assert_eq!(cut.chars().count(), 200);
        assert!(cut.ends_with("word…"), "{cut}");
    }

    #[test]
    fn control_and_layout_characters_never_reach_the_title() {
        let html = "<title>Safe\u{7}\u{1b}[31m page</title>";
        assert_eq!(title(html).as_deref(), Some("Safe[31m page"));
        // An override left open would turn the address shown after it round.
        let turned = "<title>Bank \u{202E}moc.elpmaxe\u{2066} \u{200B} login&#x202C;</title>";
        assert_eq!(title(turned).as_deref(), Some("Bank moc.elpmaxe login"));
    }
}
