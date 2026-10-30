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

use std::io::{ErrorKind, IsTerminal as _};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context as _, bail};
use glave_config::policy::{Choices, EmailFile};
use glave_event::PrincipalId;
use glave_event::Secret;
use glave_mail::{Smtp, SmtpSettings};
use glave_ports::host::{PublicOrigin, is_loopback};
use glave_ports::{Clock, Mailer, SystemClock};
use glave_session::http::{Files, Listeners, Site};
use glave_session::protocol::{Reply, Request, SignInWay};
use glave_session::signin::{Accounts, Browsers, LOCAL_BYTES, LocalToken, SignIn, Trust};
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

/// Serves HTTP on `listener` for the server `args` choose, reached from elsewhere as `reach`
/// says, and writes its address and a new local token where programs here find them.
pub(crate) async fn open(
    args: &Args,
    listener: TcpListener,
    reach: &Reach,
) -> anyhow::Result<Http> {
    let bound = listener.local_addr()?;
    within_reach(bound, reach.behind())?;
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
    // Whether `localhost` is the origin people share this computer by.
    let shared = reach
        .origin
        .as_ref()
        .is_some_and(|origin| origin.here().is_some());
    served_at(bound, reach.origin.as_ref())?;
    let beside = localhost_beside(bound, shared).await?;
    let folder = crate::home::private().await?;
    keep(&folder.join(TOKEN_FILE), token.reveal())?;
    // `localhost`, where passkeys work and this server alone answers; else its address. Where
    // `localhost` is the origin people share this computer by, the person on it signs in at
    // its address, the door that is theirs alone.
    let reached = match beside {
        Some(_) if !shared => format!("http://localhost:{}", bound.port()),
        _ => format!("http://{}", here(bound)),
    };
    keep(&folder.join(crate::app::URL_FILE), &reached)?;
    eprintln!("glave: serving the session API on http://{bound}");
    if let Some(origin) = reach.origin.as_ref().filter(|_| shared) {
        eprintln!(
            "glave: people who share this computer reach it at {origin}, each in a browser of \
             their own"
        );
    } else if let Some(origin) = reach.behind() {
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
    if origin.is_some_and(|origin| origin.here().is_some())
        && address.ip() != IpAddr::V4(Ipv4Addr::LOCALHOST)
    {
        bail!(
            "glave would listen on {address}, though its origin is this computer's localhost, \
             which a browser reaches at 127.0.0.1 and ::1 alone: listen on 127.0.0.1, which \
             glave holds at ::1 too, or set an https origin people reach from elsewhere"
        );
    }
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

/// Refuses to serve at `bound` when the origin people reach it at is this computer's
/// `localhost` at another port: a browser there would reach another program, or nothing.
fn served_at(bound: SocketAddr, origin: Option<&PublicOrigin>) -> anyhow::Result<()> {
    match origin.and_then(PublicOrigin::here) {
        Some(port) if port != bound.port() => bail!(
            "glave serves at port {}, but its origin under [serve] in config.toml is \
             http://localhost:{port}: serve at that port (--http 127.0.0.1:{port}) once nothing \
             else listens there, or set the origin to http://localhost, which follows the port \
             glave serves at",
            bound.port()
        ),
        _ => Ok(()),
    }
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

/// The glave accounts a server keeps for people other than the local person (`specs/model.md`,
/// "Identity"): where it serves over HTTP, is reached from elsewhere at a public origin, and
/// its organization signs no one in through a company login, since an organization has one
/// identity source.
pub(crate) async fn accounts(
    args: &Args,
    choices: &Choices,
    store: &Data,
) -> anyhow::Result<Option<Arc<Accounts>>> {
    let origin = choices.origin.as_ref().map(|origin| &origin.value);
    let (true, Some(origin), true) = (args.http.is_some(), origin, args.issuers.is_empty()) else {
        return Ok(None);
    };
    let server = store.server(random_uuid()?).await?.to_string();
    let clock: Arc<dyn Clock> = Arc::new(SystemClock);
    let accounts = Accounts::new(store.log(), store.directory(), server, origin, clock)?;
    Ok(Some(Arc::new(accounts)))
}

/// Where the server's email goes out: the mail server `[email]` names, signing in with
/// `GLAVE_SMTP_PASSWORD` when that is set; none without one.
pub(crate) fn mail(choices: &Choices) -> anyhow::Result<Option<Arc<dyn Mailer>>> {
    let Some(email) = choices.email.as_ref().map(|chosen| &chosen.value) else {
        return Ok(None);
    };
    let password = std::env::var("GLAVE_SMTP_PASSWORD").ok().map(Secret::new);
    let settings = SmtpSettings {
        host: email.host.trim().to_owned(),
        port: email.port.unwrap_or(EmailFile::SUBMISSION),
        from: email.from.clone(),
        user: email.user.clone(),
        password,
    };
    Ok(Some(Arc::new(Smtp::new(settings)?)))
}

/// The browsers of `person`, the local person, in `store`, signed in by the server whose id the
/// store keeps, reached from elsewhere at `origin` if anywhere, their passkeys keeping the
/// handle the root derives for them.
async fn open_browsers(
    store: &Data,
    person: PrincipalId,
    origin: Option<&PublicOrigin>,
) -> anyhow::Result<Browsers> {
    let server = store.server(random_uuid()?).await?.to_string();
    let handle = crate::keys::local_passkey_handle(store).await?;
    let scope = crate::projects::personal(store, person).await?;
    let clock = Box::new(SystemClock);
    Ok(Browsers::open(store.log(), scope, server, handle, origin, clock).await?)
}

/// A new one-time code for a sign-in link, from `server`.
async fn link(server: &Glave) -> anyhow::Result<Secret> {
    match server.request(Request::SigninLink {}).await? {
        Reply::Link { code } => Ok(code),
        other => anyhow::bail!("asked for a sign-in link, the server answered {other:?}"),
    }
}

/// A setup link for a device elsewhere, from `server`, when it is reached from there and no
/// passkey signs anyone in yet; none where people share this computer at `localhost`, where the
/// person on it signs in at its address.
async fn first_setup(server: &Glave) -> anyhow::Result<Option<String>> {
    let Reply::Server(info) = server.request(Request::Discover {}).await? else {
        return Ok(None);
    };
    let elsewhere =
        |origin: &String| PublicOrigin::parse(origin).is_some_and(|origin| origin.here().is_none());
    let Some(origin) = info
        .origin
        .filter(|origin| elsewhere(origin) && info.signin.contains(&SignInWay::Passkey))
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

/// A listener on `address`. Where it is the usual port, free when it was looked at and taken
/// since, as by a second server starting beside this one (which then finds the lock held): any
/// free one, as when it was taken before. An origin naming that port then refuses to start
/// ([`served_at`]); one of `localhost` without a port follows it.
pub(crate) async fn listen(address: SocketAddr) -> anyhow::Result<TcpListener> {
    match TcpListener::bind(address).await {
        Ok(listener) => Ok(listener),
        Err(error)
            if error.kind() == ErrorKind::AddrInUse
                && address.ip().is_loopback()
                && address.port() == crate::server::PORT =>
        {
            let any = SocketAddr::new(address.ip(), 0);
            TcpListener::bind(any)
                .await
                .with_context(|| format!("could not listen on {any}"))
        }
        Err(error) => Err(error).with_context(|| format!("could not listen on {address}")),
    }
}

/// The listener at ::1 beside one `bound` at 127.0.0.1, so `localhost`, which stands for
/// both, reaches this server alone; none for any other address, or when it can't be had. Where
/// `localhost` is the origin people share this computer by (`shared`), glave must hold it, so
/// no other program answers a browser there, unless this computer has no ::1 at all.
async fn localhost_beside(bound: SocketAddr, shared: bool) -> anyhow::Result<Option<TcpListener>> {
    match beside(bound).await {
        Ok(beside) => Ok(beside),
        Err(error) if shared && error.kind() != ErrorKind::AddrNotAvailable => Err(error)
            .with_context(|| {
                format!(
                    "could not listen on [::1]:{port}, where a browser may reach \
                     localhost:{port} too, so another program would answer people there: stop \
                     what listens there, or serve at another port and set the origin to it",
                    port = bound.port()
                )
            }),
        Err(_) => Ok(None),
    }
}

/// A listener at ::1 beside one `bound` at 127.0.0.1, at the same port; none for any other.
async fn beside(bound: SocketAddr) -> std::io::Result<Option<TcpListener>> {
    if bound.ip() != IpAddr::V4(Ipv4Addr::LOCALHOST) {
        return Ok(None);
    }
    let v6 = SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), bound.port());
    TcpListener::bind(v6).await.map(Some)
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
    fn a_loopback_listener_holds_its_port_at_both_addresses_so_localhost_is_this_server() {
        glave_ports::block_on!(async {
            let v4 = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let bound = v4.local_addr().unwrap();
            let v6 = localhost_beside(bound, false).await.unwrap().unwrap();
            let expected = SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), bound.port());
            assert_eq!(v6.local_addr().unwrap(), expected);
            let taken = localhost_beside(bound, false).await.unwrap();
            assert!(taken.is_none(), "taken there: no localhost");
            let refused = localhost_beside(bound, true).await.unwrap_err();
            assert!(
                format!("{refused:#}").contains("another program"),
                "shared, it must be glave's: {refused:#}"
            );
            let any = "0.0.0.0:4180".parse().unwrap();
            assert!(localhost_beside(any, true).await.unwrap().is_none());
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
    fn the_person_s_passkey_handle_outlives_a_fresh_store_and_is_each_root_s_own() {
        glave_ports::block_on!(async {
            let origin = PublicOrigin::parse("https://glave.example.com").unwrap();
            let person = PrincipalId::new(SystemClock.now(), [1; 10]).unwrap();
            let handle = async |root: &Arc<dyn glave_keys::Root>| {
                let keys = glave_keys::KeySettings::new(Arc::clone(root));
                let store = Data::Sqlite(glave_store_sqlite::Store::open_in_memory(keys).unwrap());
                let browsers = open_browsers(&store, person, Some(&origin)).await.unwrap();
                let code = browsers.setup().unwrap();
                let proof = glave_identity::signin::Proof::Code(code.expose());
                let asked = browsers.create_options(proof).unwrap();
                asked.options["publicKey"]["user"]["id"].clone()
            };
            let root: Arc<dyn glave_keys::Root> =
                Arc::new(glave_keys::InMemoryRoot::new().unwrap());
            let first = handle(&root).await;
            assert!(first.is_string(), "the handle is text");
            assert_eq!(handle(&root).await, first, "a fresh store, the same root");
            let theirs: Arc<dyn glave_keys::Root> =
                Arc::new(glave_keys::InMemoryRoot::new().unwrap());
            assert_ne!(handle(&theirs).await, first, "another person's root");
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
    fn a_localhost_origin_is_served_on_loopback_alone_and_at_its_own_port() {
        let at = |text: &str| text.parse::<SocketAddr>().unwrap();
        let shared = PublicOrigin::parse("http://localhost:4180").unwrap();
        assert!(within_reach(at("127.0.0.1:4180"), Some(&shared)).is_ok());
        let others = ["[::1]:4180", "127.0.0.2:4180", "0.0.0.0:4180", "[::]:4180"];
        for beyond in others.into_iter().chain(["192.168.1.2:4180"]) {
            let refused = within_reach(at(beyond), Some(&shared))
                .unwrap_err()
                .to_string();
            assert!(refused.contains("listen on 127.0.0.1"), "{refused}");
        }
        assert!(served_at(at("127.0.0.1:4180"), Some(&shared)).is_ok());
        let refused = served_at(at("127.0.0.1:4181"), Some(&shared))
            .unwrap_err()
            .to_string();
        assert!(refused.contains("port 4181"), "{refused}");
        let elsewhere = PublicOrigin::parse("https://glave.example.com").unwrap();
        assert!(served_at(at("127.0.0.1:4181"), Some(&elsewhere)).is_ok());
    }

    #[test]
    fn a_link_points_at_this_computer() {
        let at = |text: &str| here(text.parse().unwrap()).to_string();
        assert_eq!(at("0.0.0.0:8080"), "127.0.0.1:8080");
        assert_eq!(at("[::]:8080"), "[::1]:8080");
        assert_eq!(at("127.0.0.1:9"), "127.0.0.1:9");
    }
}
