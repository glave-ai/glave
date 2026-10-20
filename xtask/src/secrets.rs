//! `cargo xtask secrets`: no tracked file holds a secret-shaped literal: a provider's key or
//! token, an AWS key id, a JWT or a private key block. Tests make their fake secrets at run
//! time (`glave_ports::testing::fake_secret`), so the source holds a provider's prefix at most,
//! and no scanner, nor reader, mistakes a test for a leak. The files on [`ALLOWED`] are passed
//! over, each for its reason.

use std::io::ErrorKind;

use glave_ports::secrets::TOKEN_PREFIXES;

use crate::{cmd, workspace};

/// Tracked files that may hold a secret's shape, and why.
const ALLOWED: &[(&str, &str)] = &[(
    "crates/google/src/jwt/rfc7515-a2.pem",
    "the example RSA key RFC 7515 publishes (appendix A.2), whose RS256 signature the tests \
     check byte for byte",
)];

/// The first line of a PEM private key, around the kind of key (`RSA `, `OPENSSH `, none).
const KEY_OPENS: &str = "-----BEGIN ";
const KEY_NAMED: &str = "PRIVATE KEY-----";

/// The shortest part of a JWT, as gitleaks reads one: `ey` and at least 17 more.
const JWT_PART: usize = 19;

/// Fails on a secret-shaped literal in a tracked file, or on an allowed file that no longer
/// holds one.
pub(crate) fn check() -> Result<(), String> {
    let root = workspace::root()?;
    let listed = cmd::output("git", &["ls-files", "-z"])?;
    let mut problems = Vec::new();
    let mut scanned = 0_usize;
    for path in listed.split('\0').filter(|path| !path.is_empty()) {
        let text = match std::fs::read_to_string(root.join(path)) {
            Ok(text) => text,
            // Binary files, folders and files deleted but not yet staged hold no text.
            Err(err)
                if matches!(
                    err.kind(),
                    ErrorKind::InvalidData | ErrorKind::NotFound | ErrorKind::IsADirectory
                ) =>
            {
                continue;
            }
            Err(err) => return Err(format!("could not read {path}: {err}")),
        };
        scanned += 1;
        let found = findings(&text);
        if allowed(path) {
            if found.is_empty() {
                problems.push(format!("{path}: allowed, but holds no secret's shape now"));
            }
        } else {
            problems.extend(
                found
                    .into_iter()
                    .map(|(n, what)| format!("{path}:{n}: {what}")),
            );
        }
    }
    if problems.is_empty() {
        println!(
            "xtask: no secret-shaped literal in {scanned} tracked files ({} allowed)",
            ALLOWED.len()
        );
        Ok(())
    } else {
        Err(format!(
            "{} secret-shaped literal(s); make a fake at run time with \
             glave_ports::testing::fake_secret(\"<prefix>\"), or use FAKE_SECRET where any \
             secret will do:\n  {}",
            problems.len(),
            problems.join("\n  ")
        ))
    }
}

fn allowed(path: &str) -> bool {
    ALLOWED.iter().any(|(allowed, _)| *allowed == path)
}

/// What in `text` has a secret's shape, by line number from 1: what it is, never the secret.
fn findings(text: &str) -> Vec<(usize, String)> {
    let lines: Vec<&str> = text.lines().collect();
    let mut found = Vec::new();
    for (n, line) in lines.iter().enumerate() {
        let next = lines.get(n + 1).copied().unwrap_or_default();
        found.extend(in_line(line, next).into_iter().map(|what| (n + 1, what)));
    }
    found
}

/// The secrets' shapes in `line`, the next line showing whether a key block's first line opens
/// a block.
fn in_line(line: &str, next: &str) -> Vec<String> {
    let mut found = Vec::new();
    for (at, _) in line.char_indices().filter(|(at, _)| starts_word(line, *at)) {
        let rest = line.get(at..).unwrap_or_default();
        if let Some(prefix) = provider_token(rest) {
            found.push(format!("a key or token after `{prefix}`"));
        } else if is_jwt(rest) {
            found.push("a JWT".to_owned());
        }
    }
    if private_key(line, next) {
        found.push("a private key".to_owned());
    }
    found
}

/// The characters keys and tokens are made of.
fn is_token(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

/// Whether a word starts at `at`: not inside another word, a path or base64.
fn starts_word(line: &str, at: usize) -> bool {
    line.get(..at)
        .and_then(|before| before.chars().next_back())
        .is_none_or(|c| !(is_token(c) || "+/.".contains(c)))
}

/// The prefix `rest` starts with, the longest one, when enough token characters follow it.
fn provider_token(rest: &str) -> Option<&'static str> {
    let known = TOKEN_PREFIXES
        .iter()
        .filter(|known| rest.starts_with(known.prefix))
        .max_by_key(|known| known.prefix.len())?;
    let body = rest
        .get(known.prefix.len()..)?
        .chars()
        .take_while(|c| is_token(*c));
    (body.count() >= known.in_source).then_some(known.prefix)
}

/// Two parts of base64url JSON (`eyJ`), each followed by a dot.
fn is_jwt(rest: &str) -> bool {
    let mut parts = rest.splitn(3, '.');
    let (Some(header), Some(claims), Some(_)) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    [header, claims]
        .iter()
        .all(|part| part.starts_with("eyJ") && part.len() >= JWT_PART && part.chars().all(is_token))
}

/// A private key block's first line with its base64 after it: on the next line, or after an
/// escaped line break in a string. A first line alone, as a parser looks for it, is no key.
fn private_key(line: &str, next: &str) -> bool {
    line.match_indices(KEY_OPENS).any(|(at, _)| {
        let rest = line.get(at + KEY_OPENS.len()..).unwrap_or_default();
        let Some(end) = rest.find(KEY_NAMED) else {
            return false;
        };
        let kind = rest.get(..end).unwrap_or_default();
        if !kind.chars().all(|c| c.is_ascii_uppercase() || c == ' ') {
            return false;
        }
        let after = rest.get(end + KEY_NAMED.len()..).unwrap_or_default();
        let after = after.strip_prefix("\\r").unwrap_or(after);
        let body = match after.strip_prefix("\\n") {
            Some(escaped) => escaped,
            None if after.trim().is_empty() => next,
            None => return false,
        };
        body.trim_start()
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphanumeric() || "+/=".contains(c))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `prefix` and `length` token characters, as a string in a source file holds them.
    fn literal(prefix: &str, length: usize) -> String {
        format!("let key = \"{prefix}{}\";", "Z".repeat(length))
    }

    #[test]
    fn every_family_is_found_once_it_has_its_length() {
        for known in TOKEN_PREFIXES {
            let (prefix, least) = (known.prefix, known.in_source);
            let found = findings(&literal(prefix, least));
            assert_eq!(found.len(), 1, "{prefix} with {least}: {found:?}");
            assert!(findings(&literal(prefix, least - 1)).is_empty(), "{prefix}");
        }
    }

    #[test]
    fn a_secret_is_found_wherever_it_starts_a_word() {
        let token = format!("ghp_{}", "Z".repeat(36));
        for line in [
            format!("Authorization: Bearer {token}"),
            format!("refresh_token={token}&grant_type=refresh_token"),
            format!("https://ada:{token}@github.com/acme/app"),
            format!("{{\"access_token\": \"{token}\"}}"),
        ] {
            assert_eq!(findings(&line).len(), 1, "{line}");
        }
        let found = findings(&format!("one\ntwo\n{token}\n"));
        assert_eq!(found, [(3, "a key or token after `ghp_`".to_owned())]);
    }

    #[test]
    fn a_prefix_alone_or_inside_a_word_is_no_secret() {
        for line in [
            "    (\"sk-ant-\", 20),",
            "token.expose().starts_with(\"ghu_\")",
            "format!(\"sk-{}\", body)",
            "fake_secret(\"github_pat_\")",
            "keys look like `sk-…` and `ghp_…`",
            "let dir = folder(\"task-runs\");",
            "a risk-free disk-1 in xtask-web",
            "AKIA plus sixteen letters",
            "sha512-AbC/sk-abc+ghp_xyz",
        ] {
            assert_eq!(findings(line), [], "{line}");
        }
    }

    #[test]
    fn a_jwt_is_found_and_a_lone_part_is_not() {
        let part = format!("eyJ{}", "a".repeat(JWT_PART - 3));
        assert_eq!(findings(&format!("auth {part}.{part}.sig")).len(), 1);
        assert_eq!(findings(&format!("auth {part}.{part}.")).len(), 1);
        assert_eq!(findings(&format!("\"{part}\\")), []);
        assert_eq!(findings("isLocalToken('eyJhbGciOi.an-access-token')"), []);
    }

    #[test]
    fn a_key_block_is_found_but_not_its_first_line_alone() {
        let first = [KEY_OPENS, "RSA ", KEY_NAMED].concat();
        assert_eq!(findings(&format!("{first}\nMIIEvQIBADANBg\n")).len(), 1);
        assert_eq!(
            findings(&format!("\"{first}\\nMIIEvQIBADANBg\\n\"")).len(),
            1
        );
        assert_eq!(findings(&format!("const BEGIN: &str = \"{first}\";")), []);
        assert_eq!(
            findings(&format!("format!(\"{first}\\n{{}}\\n\", body)")),
            []
        );
        assert_eq!(findings(&format!("{first}\n\n")), []);
    }

    #[test]
    fn only_the_listed_files_are_allowed() {
        assert!(allowed("crates/google/src/jwt/rfc7515-a2.pem"));
        assert!(!allowed("crates/google/src/jwt.rs"));
        assert!(ALLOWED.iter().all(|(_, why)| !why.is_empty()));
    }
}
