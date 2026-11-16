//! A settings file that doesn't read, said as where and why: the file, its line and column, and
//! what toml says is wrong and in which key, any value it quotes left out
//! ([`glave_ports::settings::unquoted`]). Never the line itself, which toml's own error repeats
//! and which may hold a secret (`[store] url`'s password), on its way to a terminal or a log
//! store.

use std::path::Path;

use glave_ports::settings::unquoted;

/// The error for the TOML `text` at `path`, which `error` refused.
pub(super) fn unreadable(path: &Path, text: &str, error: &toml::de::Error) -> crate::Error {
    let at = error
        .span()
        .and_then(|span| text.get(..span.start))
        .map(|before| {
            let line = before.matches('\n').count() + 1;
            let last = before.rsplit('\n').next().unwrap_or_default();
            format!(", line {line}, column {}", last.chars().count() + 1)
        })
        .unwrap_or_default();
    // Without the text it read, toml says what is wrong and in which key, and not the line.
    let mut bare = error.clone();
    bare.set_input(None);
    let why = unquoted(bare.to_string().trim()).replace('\n', ", ");
    crate::Error::Invalid(format!("{}{at}: {why}", path.display()))
}

#[cfg(test)]
mod tests {
    use glave_ports::testing::any_secret;

    use super::*;

    /// What reading `text` as a config says.
    fn said(text: &str) -> String {
        crate::policy::config(text, Path::new("/home/glave/config.toml"))
            .err()
            .map(|error| error.to_string())
            .unwrap_or_default()
    }

    #[test]
    fn a_line_that_does_not_read_is_named_by_where_it_is_never_by_what_it_holds() {
        let secret = any_secret();
        let text = format!("model = \"m\"\n[store]\nurl = \"postgres://glave:{secret}@db/glave\n");
        let error = said(&text);
        // First, so what the others print holds no secret.
        assert!(!error.contains(&secret), "the line itself said");
        assert!(!error.contains("postgres://"), "the line itself said");
        assert!(
            error.starts_with("/home/glave/config.toml, line 3, column "),
            "{error}"
        );
    }

    #[test]
    fn a_value_of_the_wrong_kind_is_said_without_the_value() {
        // Quotes, a backslash and a backtick in it, which a quoted value shows escaped.
        let tail = "q\\b`t";
        let secret = format!("{}\"{tail}", any_secret());
        let error = said(&format!("store = {secret:?}\n"));
        assert!(
            !error.contains(&secret) && !error.contains(tail),
            "the value said"
        );
        assert!(error.contains("line 1, column 9"), "{error}");
        assert!(error.contains("in `store`"), "{error}");
    }

    #[test]
    fn what_is_wrong_is_said_whole() {
        let missing = said("[store]\nurl = \"postgres://db\n");
        assert!(missing.contains("expected `\"`"), "{missing}");
        let precision = said("[router_floor]\nprecision = \"eight\"\n");
        assert!(
            precision.contains("`eight` is not a precision: name one such as `fp8` ("),
            "{precision}"
        );
        let country = said("[router_floor]\nbased_in = [\"Germany\"]\n");
        assert!(
            country.contains("`Germany` is not a country's two-letter code, such as `US`"),
            "{country}"
        );
    }
}
