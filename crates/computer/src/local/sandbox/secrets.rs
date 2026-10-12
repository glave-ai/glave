//! What the sandbox hides from commands, the same on every platform: credential stores in the
//! home folder, by location, and files anywhere in the project named like secrets, by the rule
//! the read tool uses (`glave_ports::secrets::is_secret_file`). Each platform's sandbox turns
//! these into its own rules.

/// Env files, by their whole name...
pub(super) const ENV_FILES: &str = r"(\.env|\.env\.[^/]*|[^/]*\.env)";
/// ...unless a word in the name says it is a template.
pub(super) const TEMPLATE_WORDS: &str = "(example|sample|template|dist|defaults)";
/// Keys and credential files, by their whole name. `secrets.` only with a data file's
/// extension: `secrets.rs` and `secrets.py` are source modules. A `.pem` only when its name
/// says it is private: most are public certificates.
pub(super) const OTHER_SECRET_FILES: &str = r"(\.envrc|\.netrc|\.npmrc|\.pypirc|\.pgpass|\.git-credentials|\.htpasswd|credentials|credentials\.json|id_rsa|id_dsa|id_ecdsa|id_ed25519|secrets\.(yaml|yml|json|toml|env|txt|properties)|[^/]*\.(key|p12|pfx)|privkey\.pem|[^/]*private[^/]*\.pem|[^/]*-key\.pem|[^/]*\.key\.pem)";

/// Under the home folder, on every platform: where credentials live. Hidden whole, whether
/// they exist yet or not (on Linux, from the next command once they do).
pub(super) const HOME_SECRETS: &[&str] = &[
    // Keys and cloud credentials.
    ".ssh",
    ".gnupg",
    ".aws",
    ".azure",
    ".config/gcloud",
    ".kube",
    ".minikube",
    ".docker/config.json",
    ".docker/contexts",
    ".config/containers/auth.json",
    ".oci",
    // Package registries and forges.
    ".netrc",
    ".npmrc",
    ".pypirc",
    ".pgpass",
    ".git-credentials",
    ".cargo/credentials",
    ".cargo/credentials.toml",
    ".gem/credentials",
    ".m2/settings.xml",
    ".gradle/gradle.properties",
    ".bundle/config",
    ".composer/auth.json",
    ".config/gh",
    ".config/hub",
    ".config/glab-cli",
    // Infrastructure and secret managers.
    ".terraform.d",
    ".terraformrc",
    ".vault-token",
    ".config/op",
    ".op",
    ".password-store",
    ".config/sops",
    ".pulumi",
    ".config/pulumi",
    ".config/configstore",
    ".config/rclone",
    ".config/ngrok",
    ".wrangler",
    ".vercel",
    ".config/netlify",
    ".fly",
    ".cache/huggingface/token",
    // Other agents' and model tools' sign-ins.
    ".claude/.credentials.json",
    ".codex/auth.json",
    ".gemini",
    ".ollama/id_ed25519",
    // Shell and REPL histories, where pasted tokens end up.
    ".bash_history",
    ".zsh_history",
    ".zsh_sessions",
    ".local/share/fish/fish_history",
    ".python_history",
    ".node_repl_history",
    ".psql_history",
    ".mysql_history",
];

#[cfg(test)]
mod tests {
    use std::path::Path;

    use glave_ports::secrets::is_secret_file;
    use regex::Regex;

    use super::*;

    /// The sandbox's patterns hide exactly what the read tool asks about.
    #[test]
    fn the_sandbox_hides_what_the_read_tool_asks_about() {
        let env = Regex::new(&format!("(?i)^{ENV_FILES}$")).unwrap();
        let template = Regex::new(&format!("(?i){TEMPLATE_WORDS}")).unwrap();
        let other = Regex::new(&format!("(?i)^{OTHER_SECRET_FILES}$")).unwrap();
        let hidden =
            |name: &str| other.is_match(name) || (env.is_match(name) && !template.is_match(name));
        for name in [
            ".env",
            ".ENV.local",
            "prod.env",
            ".env.example",
            ".env.sample",
            ".envrc",
            ".netrc",
            "credentials",
            "credentials.json",
            "id_ed25519",
            "id_ed25519.pub",
            "tls.key",
            "client.p12",
            "cert.pfx",
            "privkey.pem",
            "server-private.pem",
            "localhost+2-key.pem",
            "tls.key.pem",
            "cert.pem",
            "fullchain.pem",
            "cacert.pem",
            "roots.pem",
            "server.pem",
            "debug.keystore",
            "release.jks",
            "secrets.yaml",
            "SECRETS.json",
            "secrets.rs",
            "secrets.py",
            "keys.rs",
            "notes.md",
            "environment.ts",
        ] {
            assert_eq!(hidden(name), is_secret_file(Path::new(name)), "{name}");
        }
    }
}
