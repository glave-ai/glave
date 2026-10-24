//! The person's server over HTTP: the session API, on loopback at 127.0.0.1:4180 unless
//! `--http` says where. Programs on this computer sign in with a local token made at every
//! start, kept in `GLAVE_HOME/serve.token` (readable only by you, with the server's address
//! beside it in `serve.url`). A browser here signs in through a link whose fragment carries a
//! one-time code, never the token, which it trades for a browser session kept in the store as
//! hashes alone, so it stays signed in across restarts. Callers
//! on other computers sign in with OIDC (`--issuer`, `--audience`). It answers beyond this
//! computer only once HTTPS is in place: a tunnel or a proxy in front of it serving its public
//! origin (`[serve] origin`), through which it is reached on loopback too, or HTTPS of its own
//! for a domain (`[serve] domain`, `https`).
//! The web app is served at `/` beside the API: the build in `--web`, else the app built into
//! glave (the `web` feature, which release builds turn on), else a `web` folder beside the
//! binary.

use std::io::IsTerminal as _;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context as _, bail};
use glave_config::policy::Choices;
use glave_event::PrincipalId;
use glave_event::Secret;
use glave_ports::SystemClock;
use glave_ports::host::{PublicOrigin, is_loopback};
use glave_session::http::{Files, Listeners, Site};
use glave_session::protocol::{Reply, Request, SignInWay};
use glave_session::signin::{Browsers, LOCAL_BYTES, LocalToken, SignIn, Trust};
use tokio::net::TcpListener;

use crate::args::Args;
use crate::client::Glave;
use crate::data::Data;
use crate::system::random_uuid;

mod https;

pub(crate) use self::https::{Reach, keep as keep_https, reach};

/// The file the local token is kept in, in glave's folder.
pub(crate) const TOKEN_FILE: &str = "serve.token";

/// The folder beside the binary a built web app may be in.
const WEB_FOLDER: &str = "web";

/// The web app built into glave (the `web` feature, `build.rs`), if it is.
static BUILT_IN: Option<Files> = include!(concat!(env!("OUT_DIR"), "/app.rs"));

/// The HTTP listener, bound, with its address and token written for programs here to find,
/// and the listener for HTTPS of glave's own beside it, with what it serves HTTPS with.
pub(crate) struct Http {
    /// One address, or 127.0.0.1 and ::1 at one port.
    listeners: Vec<TcpListener>,
    secure: Option<(TcpListener, Arc<glave_acme::Https<crate::data::DataLog>>)>,
    signin: SignIn,
    site: Option<Site>,
    token: LocalToken,
    /// glave's folder, where `serve.token` and `serve.url` are kept.
    folder: PathBuf,
}

/// Listens on `address` for the server `args` choose, reached from elsewhere as `reach` says,
/// and writes its address and a new local token where programs here find them.
pub(crate) async fn open(args: &Args, address: SocketAddr, reach: &Reach) -> anyhow::Result<Http> {
    within_reach(address, reach.behind())?;
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
    let listener = match TcpListener::bind(address).await {
        Ok(listener) => listener,
        // The usual port, free when it was looked at and taken since, as by a second server
        // starting beside this one (which then finds the lock held): any free one, as when it
        // was taken before.
        Err(error)
            if error.kind() == std::io::ErrorKind::AddrInUse
                && address.ip().is_loopback()
                && address.port() == crate::server::PORT =>
        {
            let any = SocketAddr::new(address.ip(), 0);
            TcpListener::bind(any)
                .await
                .with_context(|| format!("could not listen on {any}"))?
        }
        Err(error) => return Err(error).with_context(|| format!("could not listen on {address}")),
    };
    let bound = listener.local_addr()?;
    let beside = beside(bound).await;
    let folder = crate::home::private().await?;
    keep(&folder.join(TOKEN_FILE), token.reveal())?;
    // `localhost`, where passkeys work and this server alone answers; else its address.
    let reached = match beside {
        Some(_) => format!("http://localhost:{}", bound.port()),
        None => format!("http://{}", here(bound)),
    };
    keep(&folder.join(crate::app::URL_FILE), &reached)?;
    eprintln!("glave: serving the session API on http://{bound}");
    if let Some(origin) = reach.behind() {
        eprintln!(
            "glave: reached from elsewhere at {origin}, through the tunnel or proxy that serves \
             HTTPS for it"
        );
    }
    let secure = match (&reach.https, &reach.origin) {
        (Some(own), Some(origin)) => {
            let listener = https::listen(args).await?;
            let bound = listener.local_addr()?;
            if own.ready() {
                eprintln!("glave: serving HTTPS at {origin} on {bound}");
            } else {
                eprintln!(
                    "glave: serving HTTPS at {origin} on {bound} once its certificate is in hand, \
                     which glave is getting now"
                );
            }
            Some((listener, Arc::clone(own)))
        }
        _ => None,
    };
    if site.is_none() {
        eprintln!(
            "glave: no web app beside glave; build it (npm run build in web/) and pass --web <folder>"
        );
    }
    Ok(Http {
        listeners: std::iter::once(listener).chain(beside).collect(),
        secure,
        signin,
        site,
        token,
        folder,
    })
}

impl Http {
    /// Serves `server` until `stop` completes, then cancels its running turns. The token and
    /// address go with it.
    pub(crate) async fn serve(
        self,
        server: Glave,
        stop: impl Future<Output = ()>,
    ) -> anyhow::Result<()> {
        if std::io::stderr().is_terminal() {
            // Shown only to a person at a terminal: a service's error output often ends up in
            // a log. The code is good once, for five minutes, so the link is no key left behind.
            let url = std::fs::read_to_string(self.folder.join(crate::app::URL_FILE))?;
            let code = link(&server).await?;
            eprintln!(
                "glave: open {url}/#code={} in a browser on this computer (the link works once, \
                 for five minutes; glave opens another)",
                code.expose()
            );
            if let Some(setup) = first_setup(&server).await? {
                eprintln!(
                    "glave: to sign in from elsewhere, open {setup} on that device: it works \
                     once, for ten minutes, and makes a passkey there"
                );
            }
        } else {
            eprintln!("glave: run glave on this computer to open glave in a browser");
        }
        let listeners = Listeners {
            plain: self.listeners,
            secure: self.secure.map(|(listener, https)| (listener, https.tls())),
        };
        let ended =
            glave_session::http::serve(server, self.signin, self.site, listeners, stop).await;
        // The token is good only while this server runs; the next start makes another. A newer
        // server's files are left alone.
        let kept = std::fs::read_to_string(self.folder.join(TOKEN_FILE));
        if kept.is_ok_and(|kept| kept == self.token.reveal()) {
            for file in [TOKEN_FILE, crate::app::URL_FILE, crate::app::OPEN_FILE] {
                let _ = std::fs::remove_file(self.folder.join(file));
            }
        }
        ended?;
        Ok(())
    }
}

/// Refuses a listener at `address` beyond this computer with nothing serving HTTPS in front of
/// it, `origin`: sign-ins would cross a network in the clear. glave answers on loopback alone
/// until HTTPS is in place.
fn within_reach(address: SocketAddr, origin: Option<&PublicOrigin>) -> anyhow::Result<()> {
    if is_loopback(address.ip()) || origin.is_some() {
        return Ok(());
    }
    bail!(
        "glave would listen on {address}, beyond this computer, with nothing serving HTTPS in \
         front of it, so sign-ins would cross the network in the clear. Put a tunnel or a proxy \
         that serves HTTPS in front of glave, and set the address people reach it at as origin \
         under [serve] in config.toml (such as origin = \"https://glave.example.com\"); or \
         listen on 127.0.0.1"
    )
}

/// The web app to serve: `--web`, which must hold a build, else the one built into glave, else
/// the one beside the binary.
fn site(args: &Args) -> anyhow::Result<Option<Site>> {
    if let Some(folder) = &args.web {
        return Ok(Some(Site::folder(folder.clone())?));
    }
    if let Some(files) = BUILT_IN {
        return Ok(Some(Site::built_in(files)?));
    }
    let beside = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join(WEB_FOLDER)));
    Ok(beside.and_then(|folder| Site::folder(folder).ok()))
}

/// The browsers a server signs in, when it serves over HTTP: kept in the store, in the
/// browsers ledger of `person`'s personal project, so a browser's session outlives a restart;
/// with passkeys at the public origin where `choices` say it is reached from elsewhere, else
/// at `localhost` once it listens there.
pub(crate) async fn browsers(
    args: &Args,
    choices: &Choices,
    store: &Data,
    person: PrincipalId,
) -> anyhow::Result<Option<Arc<Browsers>>> {
    if args.http.is_none() {
        return Ok(None);
    }
    let origin = choices.origin.as_ref().map(|origin| &origin.value);
    open_browsers(store, person, origin)
        .await
        .map(|browsers| Some(Arc::new(browsers)))
}

/// The browsers of `person` in `store`, signed in by the server whose id the store keeps,
/// reached from elsewhere at `origin` if anywhere.
async fn open_browsers(
    store: &Data,
    person: PrincipalId,
    origin: Option<&PublicOrigin>,
) -> anyhow::Result<Browsers> {
    let server = store.server(random_uuid()?).await?.to_string();
    let scope = crate::projects::personal(store, person).await?;
    let clock = Box::new(SystemClock);
    Ok(Browsers::open(store.log(), scope, server, origin, clock).await?)
}

/// A new one-time code for a sign-in link, from `server`.
async fn link(server: &Glave) -> anyhow::Result<Secret> {
    match server.request(Request::SigninLink {}).await? {
        Reply::Link { code } => Ok(code),
        other => anyhow::bail!("asked for a sign-in link, the server answered {other:?}"),
    }
}

/// A setup link for a device elsewhere, from `server`, when it is reached from there and no
/// passkey signs anyone in yet.
async fn first_setup(server: &Glave) -> anyhow::Result<Option<String>> {
    let Reply::Server(info) = server.request(Request::Discover {}).await? else {
        return Ok(None);
    };
    let Some(origin) = info
        .origin
        .filter(|_| info.signin.contains(&SignInWay::Passkey))
    else {
        return Ok(None);
    };
    let Reply::Passkeys { passkeys } = server.request(Request::PasskeyList {}).await? else {
        return Ok(None);
    };
    if !passkeys.is_empty() {
        return Ok(None);
    }
    let Reply::Link { code } = server.request(Request::SigninSetup {}).await? else {
        return Ok(None);
    };
    Ok(Some(format!("{origin}/#setup={}", code.expose())))
}

/// Random bytes for the local token, from the operating system.
fn random() -> anyhow::Result<[u8; LOCAL_BYTES]> {
    let mut bytes = [0u8; LOCAL_BYTES];
    getrandom::fill(&mut bytes)
        .map_err(|e| anyhow::anyhow!("could not get random bytes for the local token: {e}"))?;
    Ok(bytes)
}

/// Writes `contents` to `path`, readable and writable only by you, replacing any earlier one
/// whole.
pub(crate) fn keep(path: &Path, contents: &str) -> anyhow::Result<()> {
    glave_ports::file::replace(path, contents.as_bytes())
        .with_context(|| format!("could not write {}", path.display()))
}

/// A listener at ::1 beside one `bound` at 127.0.0.1, at the same port, so `localhost`, which
/// stands for both, reaches this server alone; none for any other, or when that is taken.
async fn beside(bound: SocketAddr) -> Option<TcpListener> {
    if bound.ip() != IpAddr::V4(Ipv4Addr::LOCALHOST) {
        return None;
    }
    TcpListener::bind(SocketAddr::new(
        IpAddr::V6(Ipv6Addr::LOCALHOST),
        bound.port(),
    ))
    .await
    .ok()
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
    use glave_ports::Clock as _;

    use super::*;

    #[test]
    fn a_loopback_listener_holds_its_port_at_both_addresses_so_localhost_is_this_server() {
        glave_ports::block_on!(async {
            let v4 = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let bound = v4.local_addr().unwrap();
            let v6 = beside(bound).await.unwrap();
            let expected = SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), bound.port());
            assert_eq!(v6.local_addr().unwrap(), expected);
            assert!(beside(bound).await.is_none(), "taken there: no localhost");
            assert!(beside("0.0.0.0:4180".parse().unwrap()).await.is_none());
        });
    }

    #[test]
    fn the_server_and_its_sessions_are_kept_in_the_store_across_opens() {
        glave_ports::block_on!(async {
            let root = glave_keys::InMemoryRoot::new().unwrap();
            let keys = glave_keys::KeySettings::new(Arc::new(root));
            let store = Data::Sqlite(glave_store_sqlite::Store::open_in_memory(keys).unwrap());
            let person = PrincipalId::new(SystemClock.now(), [1; 10]).unwrap();
            let first = open_browsers(&store, person, None).await.unwrap();
            let code = first.link().unwrap();
            let session = first.redeem(code.expose()).await.unwrap();
            let again = open_browsers(&store, person, None).await.unwrap();
            assert_eq!(again.server(), first.server());
            assert!(again.verify(session.token.expose(), true).await.is_ok());
        });
    }

    #[test]
    fn beyond_this_computer_glave_listens_only_behind_https() {
        let at = |text: &str| text.parse::<SocketAddr>().unwrap();
        let origin = PublicOrigin::parse("https://glave.example.com").unwrap();
        for here in ["127.0.0.1:4180", "[::1]:4180"] {
            assert!(within_reach(at(here), None).is_ok(), "{here}");
            assert!(within_reach(at(here), Some(&origin)).is_ok(), "{here}");
        }
        for beyond in ["0.0.0.0:4180", "[::]:4180", "192.168.1.2:4180"] {
            let refused = within_reach(at(beyond), None).unwrap_err().to_string();
            assert!(refused.contains("HTTPS"), "{refused}");
            assert!(refused.contains("origin under [serve]"), "{refused}");
            assert!(within_reach(at(beyond), Some(&origin)).is_ok(), "{beyond}");
        }
    }

    #[test]
    fn a_link_points_at_this_computer() {
        let at = |text: &str| here(text.parse().unwrap()).to_string();
        assert_eq!(at("0.0.0.0:8080"), "127.0.0.1:8080");
        assert_eq!(at("[::]:8080"), "[::1]:8080");
        assert_eq!(at("127.0.0.1:9"), "127.0.0.1:9");
    }
}
