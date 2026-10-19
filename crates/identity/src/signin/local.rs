//! Local sign-in (`specs/session.md`, "Signing in"): programs on the person's own computer
//! sign in with a random token `glave serve` makes when it starts and keeps in a file only the
//! person can read. It is accepted only from a loopback address, and it is the local person.

use std::fmt;
use std::net::IpAddr;

use sha2::{Digest as _, Sha256};

/// What every local token starts with, so it is never mistaken for an OIDC token.
pub const LOCAL_PREFIX: &str = "glave_local_";

/// The random bytes a local token is made from.
pub const LOCAL_BYTES: usize = 32;

/// A server's local token. Shown only by [`LocalToken::reveal`], never by `Debug`.
#[derive(Clone)]
pub struct LocalToken {
    text: String,
    digest: [u8; 32],
}

impl LocalToken {
    /// The token made from `random`, which must come from the operating system's random
    /// source: `glave_local_` and 64 hexadecimal digits.
    pub fn new(random: [u8; LOCAL_BYTES]) -> Self {
        let text = format!("{LOCAL_PREFIX}{}", hex(&random));
        let digest = digest(&text);
        Self { text, digest }
    }

    /// The token itself, to write to the person's token file and print once for them.
    pub fn reveal(&self) -> &str {
        &self.text
    }

    /// Whether `presented` is this token. Compares hashes in constant time, so how long it
    /// takes says nothing about the token.
    pub fn matches(&self, presented: &str) -> bool {
        same(&digest(presented), &self.digest)
    }
}

impl fmt::Debug for LocalToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("LocalToken(..)")
    }
}

/// `bytes` in lowercase hexadecimal.
pub(super) fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut text, byte| {
        let _ = write!(text, "{byte:02x}");
        text
    })
}

/// The SHA-256 of `text`: what is kept of a token, never the token.
pub(super) fn digest(text: &str) -> [u8; 32] {
    Sha256::digest(text.as_bytes()).into()
}

/// Whether two digests are the same, in a time that does not depend on where they differ.
pub(super) fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |differ, (x, y)| differ | (x ^ y)) == 0
}

/// Whether `peer` is this computer: `127.0.0.0/8`, `::1`, or `127.0.0.0/8` mapped into IPv6.
pub fn is_loopback(peer: IpAddr) -> bool {
    match peer {
        IpAddr::V4(v4) => v4.is_loopback(),
        IpAddr::V6(v6) => v6
            .to_ipv4_mapped()
            .map_or_else(|| v6.is_loopback(), |v4| v4.is_loopback()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_token_is_its_prefix_and_its_bytes_in_hex_and_never_debug_printed() {
        let token = LocalToken::new([0xab; LOCAL_BYTES]);
        assert_eq!(token.reveal(), format!("glave_local_{}", "ab".repeat(32)));
        assert!(token.matches(token.reveal()));
        assert!(!token.matches("glave_local_"));
        assert!(!format!("{token:?}").contains("abab"));
    }

    #[test]
    fn digests_are_compared_whole() {
        assert!(same(&digest("a"), &digest("a")));
        assert!(!same(&digest("a"), &digest("b")));
    }

    #[test]
    fn loopback_is_this_computer_only() {
        for here in ["127.0.0.1", "127.1.2.3", "::1", "::ffff:127.0.0.1"] {
            assert!(is_loopback(here.parse().unwrap()), "{here}");
        }
        for there in [
            "10.0.0.1",
            "192.168.1.2",
            "::ffff:10.0.0.1",
            "2001:db8::1",
            "0.0.0.0",
        ] {
            assert!(!is_loopback(there.parse().unwrap()), "{there}");
        }
    }
}
