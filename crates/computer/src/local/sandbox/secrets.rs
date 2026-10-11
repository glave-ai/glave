//! Files named like secrets, the rule the read tool uses: env files unless they are templates,
//! keys and credential files. Each platform's sandbox turns these into its own rules.

/// Env files, by their whole name...
pub(super) const ENV_FILES: &str = r"(\.env|\.env\.[^/]*|[^/]*\.env)";
/// ...unless a word in the name says it is a template.
pub(super) const TEMPLATE_WORDS: &str = "(example|sample|template|dist|defaults)";
/// Certificate bundles, which are public, though named like keys: certifi's `cacert.pem`, in
/// every Python environment that makes HTTPS requests, and gRPC's `roots.pem`.
pub(super) const CERTIFICATE_BUNDLES: &str =
    r"(cacerts?\.pem|ca-bundle\.pem|ca-certificates\.pem|roots\.pem)";
/// Keys and credential files, by their whole name. `secrets.` only with a data file's
/// extension: `secrets.rs` and `secrets.py` are source modules.
pub(super) const OTHER_SECRET_FILES: &str = r"(\.envrc|\.netrc|\.npmrc|\.pypirc|\.pgpass|\.git-credentials|\.htpasswd|credentials|credentials\.json|id_rsa|id_dsa|id_ecdsa|id_ed25519|secrets\.(yaml|yml|json|toml|env|txt|properties)|[^/]*\.(pem|key|p12|pfx|jks|keystore))";
