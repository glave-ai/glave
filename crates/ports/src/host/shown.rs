//! A URL as it may be shown, in an error, a log line or `policy explain`: where it points kept
//! (scheme, host, port, path), what it carries hidden. A password in it is `***`, and a name
//! before the host with no password after it too, since that is where a token goes
//! (`https://<token>@host`); each query parameter's value is `***`, since a query often carries
//! a token (`?key=`, `?token=`); a fragment's is too. The one place glave makes a URL safe to
//! show, alone or in a sentence that names one (an error a library wrote).

/// What stands in for what is hidden.
const HIDDEN: &str = "***";

/// `text` with each address in it as it may be shown ([`shown`]): an error a library wrote
/// names the address it failed at, as it was asked to reach it.
pub fn urls_shown(text: &str) -> String {
    let mut kept = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find("://") {
        let (before, after) = rest.split_at(at);
        let start = before
            .rfind(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.')))
            .map_or(0, |at| at + 1);
        let (sentence, scheme) = before.split_at(start);
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

/// `url` as it may be shown.
pub fn shown(url: &str) -> String {
    let url = url.trim();
    let (url, fragment) = match url.split_once('#') {
        Some((url, _)) => (url, format!("#{HIDDEN}")),
        None => (url, String::new()),
    };
    let (scheme, rest) = match url.split_once("://") {
        Some((scheme, rest)) => (format!("{scheme}://"), rest),
        None => (String::new(), url),
    };
    let end = rest.find(['/', '?']).unwrap_or(rest.len());
    let (authority, tail) = rest.split_at(end);
    let authority = match authority.rsplit_once('@') {
        Some((user, host)) => match user.split_once(':') {
            Some((name, _)) => format!("{name}:{HIDDEN}@{host}"),
            None => format!("{HIDDEN}@{host}"),
        },
        None => authority.to_owned(),
    };
    let tail = match tail.split_once('?') {
        Some((path, query)) => {
            let query: Vec<String> = query
                .split('&')
                .map(|pair| match pair.split_once('=') {
                    Some((key, _)) => format!("{key}={HIDDEN}"),
                    None if pair.is_empty() => String::new(),
                    None => HIDDEN.to_owned(),
                })
                .collect();
            format!("{path}?{}", query.join("&"))
        }
        None => tail.to_owned(),
    };
    format!("{scheme}{authority}{tail}{fragment}")
}

/// Whether `url` holds a password: after a name before its host, or as a `password` parameter.
pub fn has_password(url: &str) -> bool {
    let url = url.trim();
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    let end = rest.find(['/', '?']).unwrap_or(rest.len());
    let (authority, tail) = rest.split_at(end);
    let in_user = authority
        .rsplit_once('@')
        .is_some_and(|(user, _)| user.contains(':'));
    let in_query = tail.split_once('?').is_some_and(|(_, query)| {
        query.split('&').any(|pair| {
            pair.split_once('=')
                .is_some_and(|(key, _)| key.eq_ignore_ascii_case("password"))
        })
    });
    in_user || in_query
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    #[test]
    fn where_it_points_is_kept_and_what_it_carries_hidden() {
        assert_eq!(
            shown("postgres://glave:s3cret@db.example.com:5432/glave?sslmode=require"),
            "postgres://glave:***@db.example.com:5432/glave?sslmode=***"
        );
        assert_eq!(
            shown("postgres://db.example.com/glave?user=glave&password=s3cret"),
            "postgres://db.example.com/glave?user=***&password=***"
        );
        assert_eq!(
            shown("https://ghp_t0ken@github.com/glave-ai/glave"),
            "https://***@github.com/glave-ai/glave"
        );
        assert_eq!(
            shown("https://mcp.example.com/sse?abc123&key=v#frag"),
            "https://mcp.example.com/sse?***&key=***#***"
        );
        assert_eq!(shown("postgres://a:b:c@h/d"), "postgres://a:***@h/d");
        assert_eq!(shown("s3://glave-blobs"), "s3://glave-blobs");
        assert_eq!(shown(" http://localhost:4318 "), "http://localhost:4318");
    }

    #[test]
    fn each_address_in_a_sentence_is_shown_as_it_may_be() {
        for (said, kept) in [
            (
                "could not reach https://mcp.example.com/mcp?key=private: refused",
                "could not reach https://mcp.example.com/mcp?key=***: refused",
            ),
            (
                "error sending request for url (http://me:private@host:8/a#private)",
                "error sending request for url (http://me:***@host:8/a#***)",
            ),
            (
                "it did not answer within 30 s",
                "it did not answer within 30 s",
            ),
            (
                "at http://host/a, then http://b/?x=private.",
                "at http://host/a, then http://b/?x=***.",
            ),
        ] {
            assert_eq!(urls_shown(said), kept);
        }
    }

    #[test]
    fn a_password_is_found_where_it_would_be_hidden() {
        assert!(has_password("postgres://glave:s3cret@db/glave"));
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
            secret in "[A-Za-z0-9_-]{12,20}",
            host in "[a-z]{1,10}",
            key in "[a-z]{1,8}",
        ) {
            for url in [
                format!("postgres://{user}:{secret}@{host}/db?password={secret}"),
                format!("https://{secret}@{host}/path"),
                format!("https://{host}/mcp?{key}={secret}&{secret}#{secret}"),
            ] {
                let shown = shown(&url);
                prop_assert!(!shown.contains(&secret), "{url}");
                prop_assert!(shown.contains(&host), "{shown}");
            }
        }
    }
}
