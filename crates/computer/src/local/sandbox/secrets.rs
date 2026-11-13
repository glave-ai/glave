//! What the sandbox hides from commands, the same on every platform: credential stores in the
//! home folder, by location. The project's own files, `.env` among them, stay readable, so a
//! test or a dev server that loads them works. Each platform's sandbox turns these into its own
//! rules.

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
