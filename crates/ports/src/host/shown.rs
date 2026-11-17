//! A URL as it may be shown, with what it carries hidden. In an error or a log line, only where
//! it is: its scheme, host and port, then `/…` for anything after, since a path can hold a
//! token as well as a query can (an MCP server's address often does). In `policy explain`, to
//! the person whose settings they are, its path too, each query value `***`. Either way a name
//! and a password before the host are `***@`, the place a token goes, and a fragment is hidden.
//! Text that is not a URL at all shows as `***`. The one place glave makes a URL safe to show,
//! alone or in a sentence that names one (an error a library wrote).

/// What stands in for what is hidden.
const HIDDEN: &str = "***";

/// A URL's parts, as far as showing it needs them.
struct Parts<'a> {
    scheme: &'a str,
    /// Whether a name, or a name and a password, came before the host.
    user: bool,
    /// Whether a password came: a name and a `:` before the host.
    password: bool,
    host: &'a str,
    path: &'a str,
    query: Option<&'a str>,
    fragment: bool,
}

impl<'a> Parts<'a> {
    /// `url`'s parts; none when it has no `scheme://`.
    fn of(url: &'a str) -> Option<Self> {
        let (scheme, rest) = url.trim().split_once("://")?;
        // A password may hold `/`, `?`, `#` or `@`, as a database's driver reads it: when what
        // comes before the first `@` has a `:`, the name and password run to the last `@` before
        // any `?` or `#` after it (a query's value may hold an `@` too). Without one, a name (a
        // token's place, which may hold `?` or `#`) runs to the last `@` before the first `/`.
        // Too much hidden (a path's `@`) is safe; too little is not.
        let (user, password, rest) = match rest.split_once('@') {
            Some((before, after)) if before.contains(':') => {
                let bound = after.find(['?', '#']).unwrap_or(after.len());
                let within = after.get(..bound).unwrap_or_default();
                let host = within.rfind('@').map_or(0, |at| at + 1);
                (true, true, after.get(host..).unwrap_or_default())
            }
            _ => match rest.split('/').next().unwrap_or_default().rsplit_once('@') {
                Some((name, _)) => (true, false, rest.get(name.len() + 1..).unwrap_or_default()),
                None => (false, false, rest),
            },
        };
        let (rest, fragment) = match rest.split_once('#') {
            Some((rest, _)) => (rest, true),
            None => (rest, false),
        };
        let end = rest.find(['/', '?']).unwrap_or(rest.len());
        let (host, tail) = rest.split_at(end);
        let (path, query) = match tail.split_once('?') {
            Some((path, query)) => (path, Some(query)),
            None => (tail, None),
        };
        Some(Self {
            scheme,
            user,
            password,
            host,
            path,
            query,
            fragment,
        })
    }

    /// `scheme://` and, past a name and password hidden, the host and port.
    fn place(&self) -> String {
        let user = if self.user { "***@" } else { "" };
        format!("{}://{user}{}", self.scheme, self.host)
    }
}

/// `url` as it may be shown in an error or a log line: where it is, `/…` for anything after.
pub fn shown(url: &str) -> String {
    let Some(parts) = Parts::of(url) else {
        return HIDDEN.to_owned();
    };
    let bare = matches!(parts.path, "" | "/") && parts.query.is_none() && !parts.fragment;
    if bare {
        format!("{}{}", parts.place(), parts.path)
    } else {
        format!("{}/…", parts.place())
    }
}

/// `url` as `policy explain` shows it to the person whose settings it is: its path too, each
/// query value hidden, and a path's `@` hidden through, where a name meant to come before the
/// host may have landed.
pub fn shown_with_path(url: &str) -> String {
    let Some(parts) = Parts::of(url) else {
        return HIDDEN.to_owned();
    };
    let path = match parts.path.rsplit_once('@') {
        Some((_, after)) => format!("/{HIDDEN}@{after}"),
        None => parts.path.to_owned(),
    };
    let query = parts.query.map_or_else(String::new, |query| {
        let hidden: Vec<String> = query
            .split('&')
            .map(|pair| match pair.split_once('=') {
                Some((key, _)) => format!("{key}={HIDDEN}"),
                None if pair.is_empty() => String::new(),
                None => HIDDEN.to_owned(),
            })
            .collect();
        format!("?{}", hidden.join("&"))
    });
    let fragment = if parts.fragment { "#***" } else { "" };
    format!("{}{path}{query}{fragment}", parts.place())
}

/// Whether `url` holds a password: after a name before its host, or as a `password` parameter.
pub fn has_password(url: &str) -> bool {
    Parts::of(url).is_some_and(|parts| {
        parts.password
            || parts.query.is_some_and(|query| {
                query.split('&').any(|pair| {
                    pair.split_once('=')
                        .is_some_and(|(key, _)| key.eq_ignore_ascii_case("password"))
                })
            })
    })
}

/// `text` with each address in it as it may be shown in an error ([`shown`]): an error a
/// library wrote names the address it failed at, as it was asked to reach it.
pub fn urls_shown(text: &str) -> String {
    let mut kept = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find("://") {
        let (before, after) = rest.split_at(at);
        // The scheme runs back from `://` while its characters do; a character before it, of
        // any width, belongs to the sentence.
        let scheme = before
            .trim_end_matches(|c: char| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
        let (sentence, scheme) = before.split_at(scheme.len());
        let end = after
            .find(|c: char| c.is_whitespace() || matches!(c, ')' | '"' | '\'' | '>'))
            .unwrap_or(after.len());
        let (address, others) = after.split_at(end);
        // The sentence's own colon or comma, after an address it follows.
        let url = address.trim_end_matches([':', ',', '.']);
        let trailing = address.get(url.len()..).unwrap_or_default();
        kept.push_str(sentence);
        kept.push_str(&shown(&format!("{scheme}{url}")));
        kept.push_str(trailing);
        rest = others;
    }
    kept.push_str(rest);
    kept
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    #[test]
    fn an_error_shows_where_a_url_is_and_nothing_after() {
        for (url, said) in [
            ("http://localhost:4318", "http://localhost:4318"),
            ("https://glave.example.com/", "https://glave.example.com/"),
            (
                "https://mcp.zapier.com/api/mcp/s/s3cr3t/mcp",
                "https://mcp.zapier.com/…",
            ),
            ("https://host/sse?key=v#frag", "https://host/…"),
            (
                "postgres://glave:s3cret@db.example.com:5432/glave",
                "postgres://***@db.example.com:5432/…",
            ),
            (
                "https://ghp_t0ken:x-oauth-basic@github.com/o/r",
                "https://***@github.com/…",
            ),
            ("https://ghp_t0ken@github.com", "https://***@github.com"),
            ("postgres://glave:ab/cd@db/glave", "postgres://***@db/…"),
            ("postgres://glave:ab?cd@db/glave", "postgres://***@db/…"),
            ("postgres://glave:ab#cd@db/glave", "postgres://***@db/…"),
            ("postgres://glave:a@b@db/glave", "postgres://***@db/…"),
            ("host=db user=glave password=s3cret", "***"),
            ("postgres:/glave:s3cret@db/glave", "***"),
        ] {
            assert_eq!(shown(url), said, "{url}");
        }
    }

    #[test]
    fn explain_shows_the_path_too_and_hides_what_a_query_carries() {
        assert_eq!(
            shown_with_path("postgres://glave:s3cret@db.example.com:5432/glave?sslmode=require"),
            "postgres://***@db.example.com:5432/glave?sslmode=***"
        );
        assert_eq!(
            shown_with_path("https://mcp.example.com/sse?abc123&key=v#frag"),
            "https://mcp.example.com/sse?***&key=***#***"
        );
        assert_eq!(shown_with_path("s3://glave-blobs"), "s3://glave-blobs");
        assert_eq!(shown_with_path("not a url"), "***");
    }

    #[test]
    fn each_address_in_a_sentence_is_shown_as_it_may_be() {
        for (said, kept) in [
            (
                "could not reach https://mcp.example.com/mcp?key=private: refused",
                "could not reach https://mcp.example.com/…: refused",
            ),
            (
                "error sending request for url (http://me:private@host:8/a#private)",
                "error sending request for url (http://***@host:8/…)",
            ),
            (
                "it did not answer within 30 s",
                "it did not answer within 30 s",
            ),
            (
                "at http://host, then http://b/?x=private.",
                "at http://host, then http://b/….",
            ),
            ("错误：https://x/private", "错误：https://x/…"),
            ("éhttps://x", "éhttps://x"),
        ] {
            assert_eq!(urls_shown(said), kept);
        }
    }

    #[test]
    fn a_password_is_found_where_it_would_be_hidden() {
        assert!(has_password("postgres://glave:s3cret@db/glave"));
        assert!(has_password("postgres://glave:ab/cd@db/glave"));
        assert!(has_password(
            "postgres://db/glave?user=glave&password=s3cret"
        ));
        assert!(!has_password(" postgres://glave@db/glave?sslmode=require "));
        assert!(!has_password("postgres://db/glave"));
    }

    proptest! {
        #[test]
        fn what_a_url_carries_never_survives(
            user in "[a-z]{1,8}",
            secret in "[A-Za-z0-9_/?#@:%-]{12,20}",
            host in "[a-z]{1,10}",
            key in "[a-z]{1,8}",
        ) {
            let carried = [
                format!("postgres://{user}:{secret}@{host}/db?password={secret}"),
                format!("https://{secret}x@{host}/path"),
            ];
            for url in &carried {
                for said in [shown(url), shown_with_path(url)] {
                    prop_assert!(!said.contains(&secret), "{url}: {said}");
                }
            }
            // A token in the path: hidden in errors and logs; `policy explain` keeps the path.
            let in_path = format!("https://{host}/mcp/{secret}?{key}={secret}&{secret}#{secret}");
            prop_assert!(!shown(&in_path).contains(&secret), "{in_path}");
        }

        #[test]
        fn any_sentence_around_an_address_is_kept_and_never_breaks(
            before in any::<String>().prop_filter("no address or word of its own", |before| {
                !before.contains("://") && !before.contains("private")
            }),
            after in "[ )][a-z ]{0,8}",
        ) {
            let text = format!("{before}https://host/private{after}");
            let said = urls_shown(&text);
            prop_assert!(!said.contains("private"), "{said}");
            prop_assert!(said.ends_with(&after), "{said}");
        }
    }
}
