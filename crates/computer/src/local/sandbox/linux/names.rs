//! File names that mark secrets, the same rule as the macOS profile and the read tool, in any
//! case (`.ENV.local` is an env file even where the file system tells cases apart).

use regex::Regex;

use super::super::secrets::{CERTIFICATE_BUNDLES, ENV_FILES, OTHER_SECRET_FILES, TEMPLATE_WORDS};

#[derive(Debug)]
pub(super) struct SecretNames {
    env: Regex,
    template: Regex,
    other: Regex,
    bundle: Regex,
}

impl SecretNames {
    pub(super) fn new() -> Result<Self, regex::Error> {
        Ok(Self {
            env: Regex::new(&format!("(?i)^{ENV_FILES}$"))?,
            template: Regex::new(&format!("(?i){TEMPLATE_WORDS}"))?,
            other: Regex::new(&format!("(?i)^{OTHER_SECRET_FILES}$"))?,
            bundle: Regex::new(&format!("(?i)^{CERTIFICATE_BUNDLES}$"))?,
        })
    }

    /// Whether a file named `name` holds secrets.
    pub(super) fn is_secret(&self, name: &str) -> bool {
        (self.other.is_match(name) && !self.bundle.is_match(name))
            || (self.env.is_match(name) && !self.template.is_match(name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_names_match_the_read_tool_s_rule() {
        let names = SecretNames::new().unwrap();
        for secret in [
            ".env",
            ".ENV.local",
            "prod.env",
            "server.pem",
            "id_ed25519",
            "secrets.yaml",
            "SECRETS.json",
            "secrets.properties",
        ] {
            assert!(names.is_secret(secret), "{secret}");
        }
        for open in [
            ".env.example",
            ".env.sample",
            "notes.md",
            "environment.ts",
            "keys.rs",
            "secrets.rs",
            "secrets.py",
            "secrets.ts",
            "secrets.go",
            "cacert.pem",
            "roots.pem",
        ] {
            assert!(!names.is_secret(open), "{open}");
        }
    }
}
