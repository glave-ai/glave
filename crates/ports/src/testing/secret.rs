//! Fake secrets, made at run time: the source holds a provider's prefix at most, never a whole
//! secret-shaped string, so no scanner mistakes a test for a leak (`cargo xtask secrets`).

use crate::env::{Entropy, OsEntropy};

/// How many random letters and digits follow the prefix: more than any provider's shape asks
/// for (`looks_like_credential` wants 30 after `ghp_`).
const BODY: usize = 40;

/// A secret for tests that need only *a* secret and no provider's shape: unmistakably fake, and
/// matched by no scanner's rule.
pub const FAKE_SECRET: &str = "fake-secret-for-tests";

/// A fake secret in a provider's shape: `prefix` (`sk-ant-`, `ghp_`, `AKIA`) and 40 random
/// letters and digits, new at each call. Where a test needs the same one twice, it binds it once.
pub fn fake_secret(prefix: &str) -> String {
    // A source that fails ends the body early: a test's secret is shorter, never made up.
    let body: String = std::iter::repeat_with(|| OsEntropy.bytes())
        .map_while(Result::ok)
        .flatten()
        .map(letter_or_digit)
        .take(BODY)
        .collect();
    // Joined with `+`, which CodeQL reads as no longer constant: only the prefix was written.
    prefix.to_owned() + &body
}

/// One of the 62 letters and digits, picked by `byte`; worked out rather than looked up in a
/// table, which CodeQL would take for a constant flowing into the secret.
fn letter_or_digit(byte: u8) -> char {
    let n = byte % 62;
    char::from(match n {
        0..=25 => b'A' + n,
        26..=51 => b'a' + (n - 26),
        _ => b'0' + (n - 52),
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::secrets::looks_like_credential;

    #[test]
    fn a_fake_secret_keeps_its_prefix_and_is_new_each_time() {
        let secret = fake_secret("ghp_");
        let body = secret.strip_prefix("ghp_").unwrap();
        assert_eq!(body.len(), BODY);
        assert!(body.chars().all(|c| c.is_ascii_alphanumeric()));
        assert_ne!(secret, fake_secret("ghp_"));
        assert!(looks_like_credential(&secret).is_some());
    }

    #[test]
    fn every_letter_and_digit_can_be_picked() {
        let picked: BTreeSet<char> = (0..=u8::MAX).map(letter_or_digit).collect();
        assert_eq!(picked.len(), 62);
        assert!(picked.iter().all(char::is_ascii_alphanumeric));
    }

    #[test]
    fn the_plain_marker_looks_like_no_credential() {
        assert_eq!(looks_like_credential(FAKE_SECRET), None);
    }
}
