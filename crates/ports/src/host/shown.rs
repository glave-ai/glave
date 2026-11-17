//! A URL as it may be shown, with what it carries hidden, read as browsers and HTTP clients read
//! one (WHATWG's rules, through the `url` crate). In an error or a log line, only where it is: its
//! scheme, host and port, then `/…` for a path, since a path can hold a token as well as a query
//! can (an MCP server's address often does). In `policy explain`, to the person whose settings
//! they are, its path too. Either way a name or a password before the host is `***@`, the place
//! a token goes, and a query or a fragment is `?…` or `#…`, nothing of what it holds. Text that
//! does not read as a URL with a host shows as `***`, and so does a URL with an `@` past its
//! host: a name or a password nobody encoded may hold a `/`, `?`, `#` or `\` that ends the host
//! too soon, and run on to an `@` later, as Postgres's driver reads a name and password up to the
//! first `@`. The one place glave makes a URL safe to show, alone or in a sentence that names one
//! (an error a library wrote).

#[cfg(test)]
mod tests;

use url::Url;

/// What stands in for what is hidden.
const HIDDEN: &str = "***";

/// A query's keys whose value is a password.
const PASSWORD_KEYS: [&str; 3] = ["password", "pass", "pwd"];

/// What may close a sentence's word after an address: the sentence's, not the address's.
const CLOSING: [char; 11] = [')', ']', '}', '"', '\'', '>', ',', '.', ';', ':', '`'];

/// `url` as it may be shown in an error or a log line: where it is, `/…` for a path.
pub fn shown(url: &str) -> String {
    said(url, false)
}

/// `url` as `policy explain` shows it to the person whose settings it is: its path too.
pub fn shown_with_path(url: &str) -> String {
    said(url, true)
}

/// `text` read as a URL and said with what it carries hidden, its path too when `with_path`.
fn said(text: &str, with_path: bool) -> String {
    let Ok(url) = Url::parse(text) else {
        return HIDDEN.to_owned();
    };
    let Some(host) = url.host_str() else {
        return HIDDEN.to_owned();
    };
    if past_host(&url) {
        return HIDDEN.to_owned();
    }
    let named = !url.username().is_empty() || url.password().is_some();
    let user = if named { "***@" } else { "" };
    let port = url
        .port()
        .map(|port| format!(":{port}"))
        .unwrap_or_default();
    let path = match url.path() {
        "" | "/" => "",
        path if with_path => path,
        _ => "/…",
    };
    let query = if url.query().is_some() { "?…" } else { "" };
    let fragment = if url.fragment().is_some() { "#…" } else { "" };
    format!(
        "{}://{user}{host}{port}{path}{query}{fragment}",
        url.scheme()
    )
}

/// Whether an `@` comes past `url`'s host, in its path, query or fragment, where a name and a
/// password nobody encoded may end.
fn past_host(url: &Url) -> bool {
    [
        url.path(),
        url.query().unwrap_or_default(),
        url.fragment().unwrap_or_default(),
    ]
    .iter()
    .any(|part| part.contains('@'))
}

/// Whether `url` holds a password: after a name before its host, or as a query's `password`,
/// `pass` or `pwd`. Text that does not read as a URL may hold one anywhere, and so may a URL
/// with an `@` past its host.
pub fn has_password(url: &str) -> bool {
    Url::parse(url).map_or(true, |url| {
        url.password().is_some()
            || past_host(&url)
            || url.query_pairs().any(|(key, _)| {
                PASSWORD_KEYS
                    .iter()
                    .any(|password| key.eq_ignore_ascii_case(password))
            })
    })
}

/// `text` with each address in it as it may be shown in an error ([`shown`]): an error a
/// library wrote names the address it failed at, as it was asked to reach it. An address is a
/// scheme (a letter, then letters, digits, `+`, `-` or `.`) and `://`, up to the next space, less
/// the punctuation that closes the sentence's word, which is kept as it was. When an `@` comes
/// after it, before the next address, the address and all to the end of that `@`'s word are
/// `***`, since a name or a password with a space in it may run on to there.
pub fn urls_shown(text: &str) -> String {
    let mut kept = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find("://") {
        let (before, after) = rest.split_at(at);
        let Some(start) = scheme_start(before) else {
            // `://` with no scheme before it is no address.
            kept.push_str(before);
            kept.push_str("://");
            rest = after.get(3..).unwrap_or_default();
            continue;
        };
        let (sentence, address) = rest.split_at(start);
        kept.push_str(sentence);
        let (said, others) = address_said(address);
        kept.push_str(&said);
        rest = others;
    }
    kept.push_str(rest);
    kept
}

/// Where the scheme that `before` ends with starts; none when it ends with none. A character
/// before it, of any width, belongs to the sentence.
fn scheme_start(before: &str) -> Option<usize> {
    let sentence = before
        .trim_end_matches(|c: char| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
    let token = before.get(sentence.len()..).unwrap_or_default();
    let letter = token.find(|c: char| c.is_ascii_alphabetic())?;
    Some(sentence.len() + letter)
}

/// The address `text` opens with, as it may be shown, and the text after it.
fn address_said(text: &str) -> (String, &str) {
    let end = text.find(char::is_whitespace).unwrap_or(text.len());
    let (word, others) = text.split_at(end);
    let url = without_closing(word);
    let closing = word.get(url.len()..).unwrap_or_default();
    // A name or a password with a space or a line break in it is cut there, and an `@` after
    // the cut, before the next address, may end it: all of it, through the word that `@` is in,
    // is hidden.
    if let Some(through) = name_ends(others) {
        let (named, others) = others.split_at(through);
        let closing = named
            .get(without_closing(named).len()..)
            .unwrap_or_default();
        return (format!("{HIDDEN}{closing}"), others);
    }
    (format!("{}{closing}", shown(url)), others)
}

/// In `others`, the text after an address cut at a space, where the word ends that holds the
/// last `@` before the next address; none when there is no such `@`.
fn name_ends(others: &str) -> Option<usize> {
    let next = others.find("://").unwrap_or(others.len());
    let at = others.get(..next)?.rfind('@')?;
    let word = others.get(at..)?;
    Some(at + word.find(char::is_whitespace).unwrap_or(word.len()))
}

/// `word` less the punctuation at its end that closes the sentence's word: a `)`, `]` or `}`
/// that closes an opening one in it is the address's own, as in `[::1]`.
fn without_closing(word: &str) -> &str {
    let mut url = word;
    while let Some(last) = url.chars().next_back() {
        let opening = match last {
            ')' => Some('('),
            ']' => Some('['),
            '}' => Some('{'),
            _ => None,
        };
        let own = opening
            .is_some_and(|opening| url.matches(opening).count() >= url.matches(last).count());
        match url.strip_suffix(last) {
            Some(less) if CLOSING.contains(&last) && !own => url = less,
            _ => break,
        }
    }
    url
}
