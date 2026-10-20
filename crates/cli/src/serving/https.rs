//! Where people reach the person's server from elsewhere (`specs/one-server.md`, "HTTPS of its
//! own"): through a tunnel or a proxy that serves HTTPS for its origin, or at a domain glave
//! serves HTTPS for itself, its certificate from an ACME authority (glave-acme), kept in the
//! certificates ledger of the person's personal project. HTTPS listens on port 443 of every
//! address unless `--https` says where; until a certificate is in hand it answers only the
//! authority's challenge.

use std::future::pending;
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;

use anyhow::{Context as _, bail};
use glave_acme::{Attempt, Https};
use glave_config::policy::Choices;
use glave_event::PrincipalId;
use glave_ports::host::PublicOrigin;
use glave_ports::{OsEntropy, SystemClock};
use tokio::net::TcpListener;

use crate::args::Args;
use crate::data::{Data, DataLog};

/// The port HTTPS of glave's own listens on unless `--https` says.
const HTTPS_PORT: u16 = 443;

/// How people reach the server from elsewhere, if they do.
#[derive(Default)]
pub(crate) struct Reach {
    /// Its public origin.
    pub origin: Option<PublicOrigin>,
    /// HTTPS of its own for that origin, when glave serves it itself.
    pub https: Option<Arc<Https<DataLog>>>,
}

impl Reach {
    /// The origin a tunnel or a proxy serves HTTPS for, in front of glave: none when glave
    /// serves it itself, or no one does.
    pub(crate) fn behind(&self) -> Option<&PublicOrigin> {
        self.origin.as_ref().filter(|_| self.https.is_none())
    }
}

/// How people reach the server `args` serve from elsewhere, as `choices` say: its HTTPS of its
/// own opened on `store`, in `person`'s personal project, when it serves that itself over HTTP.
pub(crate) async fn reach(
    args: &Args,
    choices: &Choices,
    store: &Data,
    person: PrincipalId,
) -> anyhow::Result<Reach> {
    let origin = choices.origin.as_ref().map(|origin| origin.value.clone());
    let directory = choices.acme.as_ref().map(|acme| acme.value.as_str());
    if args.https.is_some() && directory.is_none() {
        bail!(
            "--https is where glave serves HTTPS of its own: set domain under [serve] in \
             config.toml, the name it serves HTTPS for"
        );
    }
    let (Some(origin), Some(directory), Some(_)) = (&origin, directory, args.http) else {
        return Ok(Reach {
            origin,
            https: None,
        });
    };
    let scope = crate::projects::personal(store, person).await?;
    let name = origin.authority();
    let https = Https::open(
        store.log(),
        scope,
        name,
        directory,
        Box::new(SystemClock),
        Box::new(OsEntropy),
    )
    .await
    .with_context(|| format!("could not open what glave serves HTTPS for {name} with"))?;
    Ok(Reach {
        origin: Some(origin.clone()),
        https: Some(Arc::new(https)),
    })
}

/// The listener for HTTPS of glave's own: at `--https`, else port 443 of every address.
pub(crate) async fn listen(args: &Args) -> anyhow::Result<TcpListener> {
    let addresses = match args.https {
        Some(address) => vec![address],
        // Both address families where the system has IPv6, IPv4 alone where it has none.
        None => vec![
            SocketAddr::from((Ipv6Addr::UNSPECIFIED, HTTPS_PORT)),
            SocketAddr::from((Ipv4Addr::UNSPECIFIED, HTTPS_PORT)),
        ],
    };
    let mut failed = None;
    for address in addresses {
        match TcpListener::bind(address).await {
            Ok(listener) => return Ok(listener),
            Err(error) => failed = Some((address, error)),
        }
    }
    let Some((address, error)) = failed else {
        bail!("glave has no address to serve HTTPS on");
    };
    if error.kind() == std::io::ErrorKind::PermissionDenied {
        bail!(
            "glave may not listen on {address} for HTTPS: a port below 1024 needs the right to \
             (the service glave's install sets up has it), or choose another with --https"
        );
    }
    Err(error).with_context(|| format!("could not listen on {address} for HTTPS"))
}

/// Keeps the certificate of `reach`'s HTTPS current until dropped, saying what became of each
/// order; never ends where glave serves no HTTPS of its own.
pub(crate) async fn keep(reach: &Reach) {
    let (Some(https), Some(origin)) = (&reach.https, &reach.origin) else {
        return pending().await;
    };
    https
        .keep(|attempt| match attempt {
            Attempt::Issued(issued) => eprintln!(
                "glave: serving HTTPS at {origin}, its certificate good until {}",
                issued.not_after.strftime("%Y-%m-%d")
            ),
            Attempt::Failed { error, again } => eprintln!(
                "glave: could not get a certificate for {}: {error}; trying again in {} minutes",
                origin.authority(),
                again.as_secs() / 60
            ),
        })
        .await;
}
