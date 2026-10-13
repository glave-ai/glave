//! `glave serve --http`: the session API over HTTP. A browser on this computer signs in with a
//! local token made at every start, kept in `GLAVE_HOME/serve.token` (readable only by you,
//! with the server's address beside it in `serve.url`, so `glave app` finds it) and given once
//! as a link whose fragment carries it, so it never reaches a server log or a query string.
//! Callers on other computers sign in with OIDC (`--issuer`, `--audience`).
//! The web app is served at `/` beside the API: the build in `--web`, else the `web` folder
//! beside the binary, as a release ships them.

use std::io::IsTerminal as _;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::path::{Path, PathBuf};

use anyhow::Context as _;
use glave_harness::SystemClock;
use glave_session::http::Site;
use glave_session::signin::{LOCAL_BYTES, LocalToken, SignIn, Trust};
use tokio::net::TcpListener;

use crate::args::Args;
use crate::client::Glave;

/// The file the local token is kept in, in glave's folder.
pub const TOKEN_FILE: &str = "serve.token";

/// The folder beside the binary a release puts the built web app in.
const WEB_FOLDER: &str = "web";

/// Serves `server` on `address` until Ctrl-C.
pub async fn serve_http(args: &Args, server: Glave, address: SocketAddr) -> anyhow::Result<()> {
    let token = LocalToken::new(random()?);
    let signin = if args.issuers.is_empty() {
        SignIn::local(token.clone(), Box::new(SystemClock))
    } else {
        let trust = Trust {
            issuers: args.issuers.clone(),
            audience: args.audience.clone().unwrap_or_default(),
        };
        SignIn::new(trust, Box::new(SystemClock))?.with_local(token.clone())
    };
    let site = site(args)?;
    let listener = TcpListener::bind(address)
        .await
        .with_context(|| format!("could not listen on {address}"))?;
    let bound = listener.local_addr()?;
    let path = crate::home::database().await?.with_file_name(TOKEN_FILE);
    let url = format!("http://{}", here(bound));
    keep(&path, token.reveal())?;
    keep(&path.with_file_name(crate::app::URL_FILE), &url)?;
    eprintln!("glave: serving the session API on http://{bound}");
    if site.is_none() {
        eprintln!(
            "glave: no web app beside glave; build it (npm run build in web/) and pass --web <folder>"
        );
    }
    let opened = if args.open {
        let folder = path.parent().unwrap_or(&path);
        crate::app::show(folder, &url, token.reveal()).await
    } else {
        Err(anyhow::anyhow!("not asked to open"))
    };
    if opened.is_ok() {
        eprintln!("glave: glave is open in your browser; Ctrl-C here stops it");
    } else if std::io::stderr().is_terminal() {
        // Shown only to a person at a terminal: a service's error output often ends up in a
        // log.
        eprintln!(
            "glave: open http://{}/#token={} in a browser on this computer",
            here(bound),
            token.reveal()
        );
    } else {
        eprintln!(
            "glave: a browser on this computer signs in with the token in {}",
            path.display()
        );
    }
    let stop = async {
        // Without Ctrl-C, the server runs until it is stopped some other way.
        if tokio::signal::ctrl_c().await.is_err() {
            std::future::pending::<()>().await;
        }
    };
    let ended = glave_session::http::serve(server, signin, site, listener, stop).await;
    // The token is good only while this server runs; the next start makes another. A newer
    // server's files are left alone.
    if std::fs::read_to_string(&path).is_ok_and(|kept| kept == token.reveal()) {
        for file in [TOKEN_FILE, crate::app::URL_FILE, crate::app::OPEN_FILE] {
            let _ = std::fs::remove_file(path.with_file_name(file));
        }
    }
    ended?;
    Ok(())
}

/// The web app to serve: `--web`, which must hold a build, else the one beside the binary.
fn site(args: &Args) -> anyhow::Result<Option<Site>> {
    if let Some(folder) = &args.web {
        return Ok(Some(Site::folder(folder.clone())?));
    }
    let beside = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join(WEB_FOLDER)));
    Ok(beside.and_then(|folder| Site::folder(folder).ok()))
}

/// Random bytes for the local token, from the operating system.
fn random() -> anyhow::Result<[u8; LOCAL_BYTES]> {
    let mut bytes = [0u8; LOCAL_BYTES];
    getrandom::fill(&mut bytes)
        .map_err(|e| anyhow::anyhow!("could not get random bytes for the local token: {e}"))?;
    Ok(bytes)
}

/// Writes `token` to `path`, readable and writable only by you, replacing any earlier one.
pub fn keep(path: &Path, token: &str) -> anyhow::Result<()> {
    let fresh: PathBuf = path.with_extension("token.new");
    let _ = std::fs::remove_file(&fresh);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = options
        .open(&fresh)
        .with_context(|| format!("could not write {}", fresh.display()))?;
    std::io::Write::write_all(&mut file, token.as_bytes())
        .with_context(|| format!("could not write {}", fresh.display()))?;
    drop(file);
    std::fs::rename(&fresh, path).with_context(|| format!("could not write {}", path.display()))
}

/// The address a browser on this computer reaches `bound` at: itself, or loopback for a
/// listener on every address.
fn here(bound: SocketAddr) -> SocketAddr {
    let ip = match bound.ip() {
        IpAddr::V4(v4) if v4.is_unspecified() => IpAddr::V4(Ipv4Addr::LOCALHOST),
        IpAddr::V6(v6) if v6.is_unspecified() => IpAddr::V6(Ipv6Addr::LOCALHOST),
        ip => ip,
    };
    SocketAddr::new(ip, bound.port())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_token_file_is_private_and_replaced() {
        let dir = std::env::temp_dir().join(format!("glave-serve-token-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(TOKEN_FILE);
        keep(&path, "first").unwrap();
        keep(&path, "second").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "second");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_link_points_at_this_computer() {
        let at = |text: &str| here(text.parse().unwrap()).to_string();
        assert_eq!(at("0.0.0.0:8080"), "127.0.0.1:8080");
        assert_eq!(at("[::]:8080"), "[::1]:8080");
        assert_eq!(at("127.0.0.1:9"), "127.0.0.1:9");
    }
}
