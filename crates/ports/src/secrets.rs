//! What counts as a secret: environment variables and files whose contents must not reach the
//! model without the person's say-so. Judged by name only, so checking costs nothing and never
//! reads the secret itself. The harness asks before showing such a file, and a computer never
//! opens one while searching, so the rule lives here, where both see it.

use std::ffi::{OsStr, OsString};
use std::path::Path;

mod credential;
mod known;

pub use credential::{TOKEN_PREFIXES, TokenPrefix, looks_like_credential};
pub use known::{Known, KnownSecrets, Masking};

/// Words that mark an environment variable as secret anywhere in its name, ignoring case:
/// `OPENROUTER_API_KEY`, `GITHUB_TOKEN`, `DB_PASSWORD`.
const SECRET_WORDS: &[&str] = &["KEY", "SECRET", "TOKEN", "PASSWORD", "PASSWD", "CREDENTIAL"];

/// Whether an environment variable's name marks it as a secret.
pub fn is_secret_variable(name: &OsStr) -> bool {
    let name = name.to_string_lossy().to_ascii_uppercase();
    SECRET_WORDS.iter().any(|word| name.contains(word))
}

/// `vars` without the secrets, for the commands the model runs.
pub fn without_secrets(
    vars: impl IntoIterator<Item = (OsString, OsString)>,
) -> Vec<(OsString, OsString)> {
    vars.into_iter()
        .filter(|(name, _)| !is_secret_variable(name))
        .collect()
}

/// Files that hold secrets, by exact name.
const SECRET_NAMES: &[&str] = &[
    ".envrc",
    ".netrc",
    ".npmrc",
    ".pypirc",
    ".pgpass",
    ".git-credentials",
    ".htpasswd",
    "credentials",
    "credentials.json",
    "id_rsa",
    "id_dsa",
    "id_ecdsa",
    "id_ed25519",
];

/// Files that hold secrets, by extension: private keys and key stores. Not `.pem` alone, as
/// often a public certificate (`cert.pem`, `fullchain.pem`, certifi's `cacert.pem`), nor Java's
/// `.jks` and `.keystore`, which are password-protected and include Android's shared
/// `debug.keystore`.
const SECRET_EXTENSIONS: &[&str] = &["key", "p12", "pfx"];

/// The extensions of a `secrets.` file that holds data; `secrets.rs` or `secrets.py` is code.
const SECRETS_DATA: &[&str] = &["yaml", "yml", "json", "toml", "env", "txt", "properties"];

/// Words in an env file's name that mark it as a template with no real values.
const TEMPLATE_WORDS: &[&str] = &["example", "sample", "template", "dist", "defaults"];

/// Whether a file usually holds secrets, judged by its name: env files such as `.env` and
/// `.env.local` (but not `.env.example`), private keys, and credential files.
pub fn is_secret_file(path: &Path) -> bool {
    let Some(name) = path.file_name() else {
        return false;
    };
    let name = name.to_string_lossy().to_ascii_lowercase();
    let extension = Path::new(&name).extension().and_then(OsStr::to_str);
    let env_file = name == ".env" || name.starts_with(".env.") || extension == Some("env");
    if env_file {
        return !TEMPLATE_WORDS.iter().any(|word| name.contains(word));
    }
    SECRET_NAMES.contains(&name.as_str())
        || name
            .strip_prefix("secrets.")
            .is_some_and(|rest| SECRETS_DATA.contains(&rest))
        || extension.is_some_and(|extension| SECRET_EXTENSIONS.contains(&extension))
        || private_pem(&name)
}

/// A `.pem` whose name says it holds a private key: `privkey.pem` (Let's Encrypt's),
/// `*private*.pem`, `*-key.pem` (mkcert's, cfssl's) and `*.key.pem`.
fn private_pem(name: &str) -> bool {
    name.strip_suffix(".pem").is_some_and(|stem| {
        stem == "privkey"
            || stem.contains("private")
            || stem
                .strip_suffix("key")
                .is_some_and(|rest| rest.ends_with(['-', '.']))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_variables_are_found_by_name() {
        for name in [
            "OPENROUTER_API_KEY",
            "GLAVE_API_KEY",
            "github_token",
            "AWS_SECRET_ACCESS_KEY",
            "DB_PASSWORD",
            "GOOGLE_APPLICATION_CREDENTIALS",
        ] {
            assert!(is_secret_variable(OsStr::new(name)), "{name}");
        }
        for name in ["PATH", "HOME", "SSH_AUTH_SOCK", "CARGO_TARGET_DIR", "LANG"] {
            assert!(!is_secret_variable(OsStr::new(name)), "{name}");
        }
    }

    #[test]
    fn commands_get_everything_but_the_secrets() {
        let key = crate::testing::any_secret();
        let vars = [
            ("PATH", "/bin"),
            ("GLAVE_API_KEY", key.as_str()),
            ("HOME", "/h"),
        ]
        .map(|(name, value)| (OsString::from(name), OsString::from(value)));
        let kept: Vec<_> = without_secrets(vars)
            .into_iter()
            .map(|(name, _)| name)
            .collect();
        assert_eq!(kept, ["PATH", "HOME"]);
    }

    #[test]
    fn secret_files_are_found_by_name() {
        for path in [
            ".env",
            "app/.env.local",
            ".env.production",
            "prod.env",
            ".envrc",
            "certs/privkey.pem",
            "server-private.pem",
            "localhost+2-key.pem",
            "tls.key.pem",
            "tls.key",
            "client.p12",
            "id_ed25519",
            ".npmrc",
            "secrets.yaml",
            "secrets.json",
            "secrets.properties",
            "config/credentials.json",
        ] {
            assert!(is_secret_file(Path::new(path)), "{path}");
        }
        for path in [
            ".env.example",
            ".env.sample",
            "example.env",
            "id_ed25519.pub",
            "src/main.rs",
            "keys.rs",
            "src/secrets.rs",
            "secrets.py",
            "secrets.ts",
            "secrets.go",
            "secrets.yaml.rs",
            "environment.md",
            ".venv/lib/python3.12/site-packages/certifi/cacert.pem",
            "grpc/_cython/_credentials/roots.pem",
            "cert.pem",
            "fullchain.pem",
            "certs/server.pem",
            "android/debug.keystore",
            "release.jks",
            "keys/public-key.pub",
            ".",
        ] {
            assert!(!is_secret_file(Path::new(path)), "{path}");
        }
    }
}
