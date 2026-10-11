//! TLS to the database: rustls on aws-lc-rs, as the rest of glave, with the operating
//! system's certificate verifier, so a server's certificate is always checked against the
//! trusted roots and its name. A database URL that does not say `sslmode` gets `require` when
//! the database is on another computer: an organization's history never crosses a network in
//! the clear unless someone wrote `sslmode=disable`.

use std::sync::Arc;

use rustls::ClientConfig;
use rustls_platform_verifier::BuilderVerifierExt as _;
use tokio_postgres::Config;
use tokio_postgres::config::{Host, SslMode};
use tokio_postgres_rustls::MakeRustlsConnect;

use crate::store::StoreError;

/// The connector every connection uses: TLS when the server and `sslmode` agree to it.
pub(crate) fn connector() -> Result<MakeRustlsConnect, StoreError> {
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let failed = |e: rustls::Error| StoreError::Connect(format!("could not set up TLS: {e}"));
    let config = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(failed)?
        .with_platform_verifier()
        .map_err(failed)?
        .with_no_client_auth();
    Ok(MakeRustlsConnect::new(config))
}

/// Requires TLS for a database on another computer when the URL chose nothing stricter or
/// looser than the default (`prefer`, which would fall back to the clear).
pub(crate) fn require_off_this_computer(config: &mut Config) {
    let remote = config.get_hosts().iter().any(|host| match host {
        Host::Tcp(name) => !matches!(name.as_str(), "localhost" | "127.0.0.1" | "::1" | "[::1]"),
        #[cfg(unix)]
        Host::Unix(_) => false,
    });
    if remote && config.get_ssl_mode() == SslMode::Prefer {
        config.ssl_mode(SslMode::Require);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mode(url: &str) -> SslMode {
        let mut config: Config = url.parse().unwrap();
        require_off_this_computer(&mut config);
        config.get_ssl_mode()
    }

    #[test]
    fn another_computer_needs_tls_unless_the_url_says_otherwise() {
        assert_eq!(mode("postgres://u@db.example.com/glave"), SslMode::Require);
        assert_eq!(mode("postgres://u@10.0.0.5:5432/glave"), SslMode::Require);
        assert_eq!(
            mode("postgres://u@db.example.com/glave?sslmode=disable"),
            SslMode::Disable
        );
        assert_eq!(mode("postgres://u@localhost/glave"), SslMode::Prefer);
        assert_eq!(mode("postgres://u@127.0.0.1:5432/glave"), SslMode::Prefer);
    }

    #[test]
    fn the_connector_builds() {
        assert!(connector().is_ok());
    }
}
