//! Files named like secrets, the rule the read tool uses: env files unless they are templates,
//! keys and credential files. Each platform's sandbox turns these into its own rules.

/// Env files, by their whole name...
pub(super) const ENV_FILES: &str = r"(\.env|\.env\.[^/]*|[^/]*\.env)";
/// ...unless a word in the name says it is a template.
pub(super) const TEMPLATE_WORDS: &str = "(example|sample|template|dist|defaults)";
/// Keys and credential files, by their whole name.
pub(super) const OTHER_SECRET_FILES: &str = r"(\.envrc|\.netrc|\.npmrc|\.pypirc|\.pgpass|\.git-credentials|\.htpasswd|credentials|credentials\.json|id_rsa|id_dsa|id_ecdsa|id_ed25519|secrets\.[^/]*|[^/]*\.(pem|key|p12|pfx|jks|keystore))";
