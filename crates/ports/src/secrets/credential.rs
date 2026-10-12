//! Whether text looks like it holds a credential: the one check everything that must not store
//! secrets reuses, such as memory (`specs/memory.md`, "Never secrets"). It reads only the text
//! it is given and errs toward refusing: a false alarm costs a rephrasing, a miss keeps a key.

/// Prefixes that providers put on their keys and tokens, and how many characters at least
/// follow each in a real one.
const PREFIXES: &[(&str, usize)] = &[
    ("sk-ant-", 20),
    ("sk-proj-", 20),
    ("sk-or-", 20),
    ("sk-", 20),
    ("sk_live_", 16),
    ("sk_test_", 16),
    ("rk_live_", 16),
    ("pk_live_", 16),
    ("ghp_", 30),
    ("gho_", 30),
    ("ghu_", 30),
    ("ghs_", 30),
    ("ghr_", 30),
    ("github_pat_", 30),
    ("glpat-", 16),
    ("xoxb-", 16),
    ("xoxp-", 16),
    ("xoxa-", 16),
    ("xapp-", 16),
    ("AKIA", 16),
    ("ASIA", 16),
    ("AIza", 30),
    ("hf_", 30),
    ("npm_", 30),
    ("pypi-", 30),
    ("SG.", 30),
    ("dop_v1_", 30),
    ("shpat_", 20),
    ("sq0atp-", 20),
];

/// Labels after which a value is a password or key.
const LABELS: &[&str] = &[
    "password",
    "passwd",
    "pwd",
    "passphrase",
    "pin",
    "api key",
    "api_key",
    "apikey",
    "secret",
    "token",
    "access key",
];

/// The shortest run of token characters judged by its randomness.
const LONG_TOKEN: usize = 32;

/// Bits of entropy per character above which a long token reads as random: hex, such as a
/// commit hash or a UUID, stays below it.
const RANDOM_BITS: f64 = 4.3;

/// Why `text` looks like it holds a credential, or `None` when it does not.
pub fn looks_like_credential(text: &str) -> Option<&'static str> {
    if text.contains("-----BEGIN") && text.contains("PRIVATE KEY") {
        return Some("it holds a private key");
    }
    for word in words(text) {
        if has_known_prefix(word) {
            return Some("it holds what looks like a provider's API key or token");
        }
        if looks_like_jwt(word) {
            return Some("it holds what looks like a signed token");
        }
        if looks_random(word) {
            return Some("it holds a long random-looking token");
        }
    }
    if labelled_value(text) {
        return Some("it holds a password or key next to its label");
    }
    None
}

/// Runs of the characters tokens are made of.
fn words(text: &str) -> impl Iterator<Item = &str> {
    text.split(|c: char| !(c.is_ascii_alphanumeric() || "_-+/=.".contains(c)))
        .map(|word| word.trim_matches(|c: char| c == '.' || c == '='))
        .filter(|word| !word.is_empty())
}

fn has_known_prefix(word: &str) -> bool {
    PREFIXES.iter().any(|(prefix, rest)| {
        word.strip_prefix(prefix).is_some_and(|tail| {
            tail.len() >= *rest
                && tail
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "_-".contains(c))
        })
    })
}

/// Three base64url parts, the first a JSON header (`eyJ`).
fn looks_like_jwt(word: &str) -> bool {
    let parts: Vec<&str> = word.split('.').collect();
    parts.len() == 3 && word.starts_with("eyJ") && parts.iter().all(|part| part.len() >= 8)
}

fn looks_random(word: &str) -> bool {
    if word.len() < LONG_TOKEN || word.contains('/') && word.matches('/').count() > 2 {
        return false;
    }
    let letters = word.chars().any(|c| c.is_ascii_alphabetic());
    let digits = word.chars().any(|c| c.is_ascii_digit());
    letters && digits && entropy(word) > RANDOM_BITS
}

/// Shannon entropy in bits per character.
fn entropy(word: &str) -> f64 {
    let mut counts = [0u32; 128];
    let mut total = 0u32;
    for byte in word.bytes().filter(u8::is_ascii) {
        if let Some(count) = counts.get_mut(usize::from(byte)) {
            *count = count.saturating_add(1);
            total = total.saturating_add(1);
        }
    }
    if total == 0 {
        return 0.0;
    }
    let total = f64::from(total);
    counts
        .iter()
        .filter(|count| **count > 0)
        .map(|count| {
            let p = f64::from(*count) / total;
            -p * p.log2()
        })
        .sum()
}

/// A label, then `:`, `=` or `is`, then a value with a digit or a symbol in it: `password:
/// hunter2`, `API key = abc123def`. A value of plain words ("the password is stored in
/// Vault") is not one.
fn labelled_value(text: &str) -> bool {
    let lower = text.to_lowercase();
    LABELS.iter().any(|label| {
        lower.match_indices(label).any(|(at, _)| {
            let before = lower.get(..at).and_then(|head| head.chars().next_back());
            if before.is_some_and(char::is_alphanumeric) {
                return false;
            }
            let rest = lower
                .get(at.saturating_add(label.len())..)
                .unwrap_or_default();
            value_after(rest).is_some_and(|value| {
                value.chars().count() >= 4
                    && value
                        .chars()
                        .any(|c| c.is_ascii_digit() || (!c.is_alphanumeric() && c != '-'))
            })
        })
    })
}

/// The value after a label's separator, if one follows it.
fn value_after(rest: &str) -> Option<&str> {
    let rest = rest.trim_start_matches(['s', '\'', ' ']);
    let rest = if let Some(after) = rest.strip_prefix("is ") {
        after
    } else {
        rest.strip_prefix(':').or_else(|| rest.strip_prefix('='))?
    };
    let value = rest.trim_start().trim_start_matches(['"', '\'', '`']);
    value
        .split(|c: char| c.is_whitespace() || "\"'`".contains(c))
        .next()
        .map(|value| value.trim_end_matches(['.', ',', ';']))
        .filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credentials_are_refused() {
        for text in [
            "use sk-ant-api03-AbCdEfGhIjKlMnOpQrStUvWxYz0123456789 for the deploy",
            "the key is sk-proj-1234567890abcdefghijklmnopqrstuv",
            "token ghp_aB3dE5gH7jK9mN1pQ3sT5vW7yZ9bC1dE3fG5",
            "AWS: AKIAIOSFODNN7EXAMPLE",
            "-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXk\n-----END OPENSSH PRIVATE KEY-----",
            "auth eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dozjgNryP4J3jVmNHl0w5N_XgL0n3I9PlFUP0THsR8U",
            "the session cookie is 9fK2mQ7xL4pR8vT1wZ6bN3cH5jD0sG2aY7uE",
            "my password: hunter2",
            "Password = correct-horse-battery-9",
            "the staging api key is x7f3k9q2m1",
        ] {
            assert!(looks_like_credential(text).is_some(), "{text}");
        }
    }

    #[test]
    fn ordinary_memories_pass() {
        for text in [
            "Ada prefers short answers without headings",
            "This repository's tests need `make db` first",
            "We chose Postgres over MySQL on 2026-10-02 for JSONB",
            "Commit 3f2a9c1e8b7d6a5f4e3d2c1b0a9f8e7d6c5b4a39 broke the build",
            "Thread 0192f5c0-11aa-7e21-8c3d-5f9b2e4a6c13 has the details",
            "the password is stored in Vault under team/app",
            "rotate the API key every 90 days",
            "use tabs, not spaces, in this project's Python",
            "docs live at https://docs.example.com/guides/getting-started/install",
            "the token budget is generous",
            "we deploy with Render",
        ] {
            assert_eq!(looks_like_credential(text), None, "{text}");
        }
    }

    proptest::proptest! {
        #[test]
        fn any_text_is_judged_without_panicking(text in ".{0,400}") {
            let _ = looks_like_credential(&text);
        }
    }
}
