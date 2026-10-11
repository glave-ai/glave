//! A URL as it may be shown: its password, in the user part or a `password` parameter,
//! replaced by `***`, so `policy explain` and errors never repeat a secret.

/// `url` without its password.
pub fn redacted(url: &str) -> String {
    let url = url.trim();
    let (scheme, rest) = url.split_once("://").unwrap_or(("", url));
    let end = rest.find(['/', '?']).unwrap_or(rest.len());
    let (authority, tail) = rest.split_at(end);
    let authority = match authority.rsplit_once('@') {
        Some((user, host)) => match user.split_once(':') {
            Some((name, _)) => format!("{name}:***@{host}"),
            None => format!("{user}@{host}"),
        },
        None => authority.to_owned(),
    };
    let tail = match tail.split_once('?') {
        Some((path, query)) => {
            let query: Vec<String> = query
                .split('&')
                .map(|pair| match pair.split_once('=') {
                    Some((key, _)) if key.eq_ignore_ascii_case("password") => {
                        format!("{key}=***")
                    }
                    _ => pair.to_owned(),
                })
                .collect();
            format!("{path}?{}", query.join("&"))
        }
        None => tail.to_owned(),
    };
    if scheme.is_empty() {
        format!("{authority}{tail}")
    } else {
        format!("{scheme}://{authority}{tail}")
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    #[test]
    fn passwords_are_hidden_and_the_rest_kept() {
        assert_eq!(
            redacted("postgres://glave:s3cret@db.example.com:5432/glave?sslmode=require"),
            "postgres://glave:***@db.example.com:5432/glave?sslmode=require"
        );
        assert_eq!(
            redacted("postgres://db.example.com/glave?user=glave&password=s3cret"),
            "postgres://db.example.com/glave?user=glave&password=***"
        );
        assert_eq!(
            redacted("postgresql://glave@localhost/glave"),
            "postgresql://glave@localhost/glave"
        );
        assert_eq!(redacted("postgres://a:b:c@h/d"), "postgres://a:***@h/d");
    }

    proptest! {
        #[test]
        fn a_password_never_survives(
            user in "[a-z]{1,8}",
            password in "[A-Za-z0-9]{12,20}",
            host in "[a-z]{1,10}",
        ) {
            let url = format!("postgres://{user}:{password}@{host}/db?password={password}");
            prop_assert!(!redacted(&url).contains(&password));
        }
    }
}
