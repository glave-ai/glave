//! A settings file's parser error said without the values it quotes. serde's errors name a value
//! of the wrong kind as it is written (`invalid type: string "…"`), and a settings file's values
//! include secrets: a store's password, a server's token in its headers. Where and what is wrong
//! is enough to mend it.

/// `message` with what it quotes left out: `string "…"`. Names in backticks stay.
pub fn unquoted(message: &str) -> String {
    message
        .split('"')
        .enumerate()
        .map(|(at, part)| if at % 2 == 1 { "…" } else { part })
        .collect::<Vec<_>>()
        .join("\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_is_quoted_is_left_out_and_the_rest_kept() {
        assert_eq!(
            unquoted("invalid type: string \"x\", expected `a` or `b`"),
            "invalid type: string \"…\", expected `a` or `b`"
        );
        assert_eq!(unquoted("no quotes"), "no quotes");
    }
}
