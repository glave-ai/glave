use proptest::prelude::*;

use super::*;

#[test]
fn an_error_shows_where_a_url_is_and_nothing_after() {
    for (url, said) in [
        ("http://localhost:4318", "http://localhost:4318"),
        ("https://glave.example.com/", "https://glave.example.com"),
        ("http://[::1]:4318/v1/traces", "http://[::1]:4318/…"),
        (
            "https://mcp.zapier.com/api/mcp/s/s3cr3t/mcp",
            "https://mcp.zapier.com/…",
        ),
        ("https://host/sse?key=v#frag", "https://host/…?…#…"),
        ("https://host?email=a@b.com&token=private", "***"),
        (
            "postgres://glave:s3cret@db.example.com:5432/glave",
            "postgres://***@db.example.com:5432/…",
        ),
        (
            "https://ghp_t0ken:x-oauth-basic@github.com/o/r",
            "https://***@github.com/…",
        ),
        ("https://ghp_t0ken@github.com", "https://***@github.com"),
        ("git+ssh://git@github.com/o/r", "git+ssh://***@github.com/…"),
        ("https://me%40mail.example:pw@host/x", "https://***@host/…"),
        (
            "http://me:pa ss@collector:4318",
            "http://***@collector:4318",
        ),
        ("postgres://glave:a@b@db/glave", "postgres://***@db/…"),
        // An `@` past the host, where a name and a password nobody encoded may end: none of it
        // is shown.
        ("postgres://u:p@ss/word@db/x", "***"),
        ("postgres://glave:s3cret@db/glave?to=a@b", "***"),
        ("postgres://glave:ab/cd@db/glave", "***"),
        ("postgres://glave:ab?cd@db/glave", "***"),
        ("postgres://glave:ab#cd@db/glave", "***"),
        ("https://server.example/@owner/mcp", "***"),
        // A name or a password with a `/`, `?`, `#` or `\` that ends the host too soon.
        ("postgres://glave:2024/Summer@db/glave", "***"),
        ("postgres://glave:/s3cretpass@db/glave", "***"),
        ("https://AB/CD+ef@host/x", "***"),
        ("http://tok\\en@host/x", "***"),
        ("host=db user=glave password=s3cret", "***"),
        ("postgres:/glave:s3cret@db/glave", "***"),
        ("mailto:me@example.com", "***"),
    ] {
        assert_eq!(shown(url), said, "{url}");
    }
}

#[test]
fn explain_shows_the_path_too_and_nothing_a_query_carries() {
    assert_eq!(
        shown_with_path("postgres://glave:s3cret@db.example.com:5432/glave?sslmode=require"),
        "postgres://***@db.example.com:5432/glave?…"
    );
    assert_eq!(
        shown_with_path("https://mcp.example.com/sse?abc123&key=v#frag"),
        "https://mcp.example.com/sse?…#…"
    );
    assert_eq!(shown_with_path("s3://glave-blobs"), "s3://glave-blobs");
    assert_eq!(shown_with_path("not a url"), "***");
    assert_eq!(
        shown_with_path("postgres://glave:/s3cretpass@db/glave"),
        "***"
    );
}

#[test]
fn each_address_in_a_sentence_is_shown_as_it_may_be() {
    for (said, kept) in [
        (
            "could not reach https://mcp.example.com/mcp?key=private: refused",
            "could not reach https://mcp.example.com/…?…: refused",
        ),
        (
            "error sending request for url (http://me:private@host:8/a#private)",
            "error sending request for url (http://***@host:8/…#…)",
        ),
        (
            "it did not answer within 30 s",
            "it did not answer within 30 s",
        ),
        (
            "at http://host, then http://b/?x=private.",
            "at http://host, then http://b?….",
        ),
        (
            "see https://host/x?token=ab)cd now",
            "see https://host/…?… now",
        ),
        ("(at http://[::1]:8080).", "(at http://[::1]:8080)."),
        (
            "\"url\": \"https://h/x?k=private\",",
            "\"url\": \"https://h/…?…\",",
        ),
        ("错误：https://x/private", "错误：https://x/…"),
        ("éhttps://x", "éhttps://x"),
        ("1https://x/private", "1https://x/…"),
        ("a ://x/private b", "a ://x/private b"),
        // A password with a space in it, cut there: hidden through its `@`.
        ("to http://me:pa ss@collector:4318 now", "to *** now"),
        ("to http://me:p a s s@collector:4318/x).", "to ***)."),
        ("not a url: https://host:port/x", "not a url: ***"),
        ("http://me:1234\n5678@host now", "*** now"),
        // An `@` later, before the next address, may end a name: too much hidden is safe.
        (
            "could not reach http://h/x for me@example.com, then http://b",
            "could not reach ***, then http://b",
        ),
    ] {
        assert_eq!(urls_shown(said), kept, "{said}");
    }
}

#[test]
fn a_password_is_found_where_it_would_be_hidden() {
    for url in [
        "postgres://glave:s3cret@db/glave",
        "postgres://db/glave?user=glave&password=s3cret",
        "postgres://db/glave?PWD=s3cret",
        "postgres://db/glave?pass=s3cret",
        "postgres://glave:ab/cd@db/glave",
        "postgres://glave:/s3cretpass@db/glave",
        "postgres://db:5432/x?application_name=a@b",
        "password=x https://h",
    ] {
        assert!(has_password(url), "{url}");
    }
    for url in [
        " postgres://glave@db/glave?sslmode=require ",
        "postgres://db/glave",
    ] {
        assert!(!has_password(url), "{url}");
    }
}

/// The first four characters in a row of `secret` that `said` still holds, if any, in any case:
/// a host is said in small letters.
fn survives(secret: &str, said: &str) -> Option<String> {
    let said = said.to_lowercase();
    let chars: Vec<char> = secret.to_lowercase().chars().collect();
    chars
        .windows(4)
        .map(|four| four.iter().collect::<String>())
        .find(|four| said.contains(four.as_str()))
}

/// A secret in a query, a fragment or a path, which may hold `@ / ? #`, drawn from characters
/// that never run four in a row, in any case, in what is said around it: its letters are none
/// of the template's (`a` to `i`, and the schemes' and words' own).
const CARRIED: &str = "[JLNUZ0-9@/?#]{12,20}";

/// A secret as a name or a password, which may hold spaces, line breaks, quotes, `)`, `@` and
/// what ends a host too soon (`/ ? # \`), drawn as [`CARRIED`] is.
const NAMED: &str = "[JLNUZ0-9 \n\"')@/?#\\\\]{12,20}";

/// What closes a sentence's word after an address.
fn closing() -> impl Strategy<Value = &'static str> {
    prop::sample::select(vec!["", ")", ").", "\",", ":", "`", "]", "'.", ";", ">"])
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(4096))]

    #[test]
    fn what_a_url_carries_never_survives(
        carried in CARRIED,
        named in NAMED,
        user in "[a-i]{1,8}",
        host in "[a-i]{1,10}",
        key in "[a-i]{1,8}",
        tail in prop::sample::select(vec!["", "/x", "?q=1", "#f", ":81/x?y=1"]),
    ) {
        let both = format!("postgres://{user}:{named}@{host}/db?password={carried}");
        let hidden = [
            (format!("https://{host}?{key}={carried}"), &carried),
            (format!("https://{host}#{carried}"), &carried),
            (format!("https://{host}:81?{key}={carried}#{carried}"), &carried),
            (format!("http://{user}:{named}@{host}{tail}"), &named),
            (format!("http://{named}@{host}{tail}"), &named),
            (format!("postgres://{user}:{named}@{host}{tail}"), &named),
            (both.clone(), &named),
            (both, &carried),
        ];
        for (url, secret) in &hidden {
            for said in [shown(url), shown_with_path(url)] {
                prop_assert!(survives(secret, &said).is_none(), "{url:?}: {said:?}");
            }
        }
        // A token in the path: hidden in errors and logs; `policy explain` keeps the path.
        let in_path = format!("https://{host}/mcp/{carried}");
        prop_assert!(survives(&carried, &shown(&in_path)).is_none(), "{in_path}");
    }

    #[test]
    fn an_address_mid_sentence_keeps_the_sentence_and_nothing_it_carries(
        carried in CARRIED,
        named in NAMED,
        user in "[a-i]{1,8}",
        host in "[a-i]{1,10}",
        lead in "[a-i]{0,12}",
        trail in "[a-i]{0,12}",
        closing in closing(),
    ) {
        for (url, secret) in [
            (format!("https://{host}/mcp/{carried}"), &carried),
            (format!("https://{host}?k={carried}#{carried}"), &carried),
            (format!("http://{user}:{named}@{host}:81/v1"), &named),
            (format!("http://{named}@{host}"), &named),
        ] {
            let text = format!("{lead} {url}{closing} {trail}");
            let said = urls_shown(&text);
            prop_assert!(survives(secret, &said).is_none(), "{text:?}: {said:?}");
            prop_assert!(said.starts_with(&format!("{lead} ")), "{text:?}: {said:?}");
            prop_assert!(said.ends_with(&format!("{closing} {trail}")), "{text:?}: {said:?}");
        }
    }

    #[test]
    fn any_sentence_around_an_address_is_kept_and_never_breaks(
        before in any::<String>().prop_filter("no address or word of its own", |before| {
            !before.contains("://") && !before.contains("private")
        }),
        closing in closing(),
        trail in "[a-z ]{0,8}",
    ) {
        let after = format!("{closing} {trail}");
        let text = format!("{before}https://host/private{after}");
        let said = urls_shown(&text);
        prop_assert!(!said.contains("private"), "{said}");
        prop_assert!(said.ends_with(&after), "{said}");
    }
}
