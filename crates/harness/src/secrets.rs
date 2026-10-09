//! What counts as a secret: environment variables and files whose contents must not reach the
//! model without the person's say-so. Judged by name only, so checking costs nothing and never
//! reads the secret itself.

use std::ffi::{OsStr, OsString};
use std::path::Path;

/// Words that mark an environment variable as secret anywhere in its name, ignoring case:
/// `OPENROUTER_API_KEY`, `GITHUB_TOKEN`, `DB_PASSWORD`.
const SECRET_WORDS: &[&str] = &["KEY", "SECRET", "TOKEN", "PASSWORD", "PASSWD", "CREDENTIAL"];

/// Whether an environment variable's name marks it as a secret.
pub(crate) fn is_secret_variable(name: &OsStr) -> bool {
    let name = name.to_string_lossy().to_ascii_uppercase();
    SECRET_WORDS.iter().any(|word| name.contains(word))
}

/// `vars` without the secrets, for the commands the model runs.
pub(crate) fn without_secrets(
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

/// Files that hold secrets, by extension: private keys and key stores.
const SECRET_EXTENSIONS: &[&str] = &["pem", "key", "p12", "pfx", "jks", "keystore"];

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
        || name.starts_with("secrets.")
        || extension.is_some_and(|extension| SECRET_EXTENSIONS.contains(&extension))
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
        let vars = [
            ("PATH", "/bin"),
            ("GLAVE_API_KEY", "sk-test"),
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
            "certs/server.pem",
            "tls.key",
            "id_ed25519",
            ".npmrc",
            "secrets.yaml",
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
            "environment.md",
            ".",
        ] {
            assert!(!is_secret_file(Path::new(path)), "{path}");
        }
    }
}
