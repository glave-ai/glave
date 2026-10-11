//! Local sign-in (`specs/session.md`, "Signing in"): a browser on the person's own computer
//! signs in with a random token `glave serve` makes when it starts and keeps in a file only the
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
        let mut text = String::with_capacity(LOCAL_PREFIX.len() + 2 * LOCAL_BYTES);
        text.push_str(LOCAL_PREFIX);
        for byte in random {
            text.push(hex(byte >> 4));
            text.push(hex(byte & 0x0f));
        }
        let digest = digest(&text);
        Self { text, digest }
    }

    /// The token itself, to write to the person's token file and print once for them.
    pub fn reveal(&self) -> &str {
        &self.text
    }

    /// Whether `presented` is this token. Compares hashes, so how long it takes says nothing
    /// about the token.
    pub fn matches(&self, presented: &str) -> bool {
        digest(presented) == self.digest
    }
}

impl fmt::Debug for LocalToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("LocalToken(..)")
    }
}

fn hex(nibble: u8) -> char {
    char::from(
        b"0123456789abcdef"
            .get(usize::from(nibble))
            .copied()
            .unwrap_or(b'0'),
    )
}

fn digest(text: &str) -> [u8; 32] {
    Sha256::digest(text.as_bytes()).into()
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
