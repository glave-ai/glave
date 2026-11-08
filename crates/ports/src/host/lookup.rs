//! Looking a host's name up for an HTTP client (reqwest's resolver), keeping only the addresses
//! glave's one rule ([`reachable`]) lets the client reach: every address the name resolves to is
//! checked, and only those allowed are handed to the connection, so a name cannot resolve to a
//! public address when checked and a private one when connected (DNS rebinding). The one
//! definition `web_fetch`, remote tool servers and pushes all look names up with; each says in
//! its own words why a name was refused.

use std::error::Error;
use std::net::SocketAddr;
use std::sync::Arc;

use reqwest::dns::{Addrs, Name, Resolve, Resolving};

use super::reachable;

/// Looks names up with the system's resolver, keeping the addresses a client may reach: public
/// ones, and private ones too for a host `private_ok` names. A name that leads to none is refused
/// with what `refused` makes of it.
pub struct Lookup<R> {
    private_ok: Arc<dyn Fn(&str) -> bool + Send + Sync>,
    refused: fn(&str) -> R,
}

impl<R> Clone for Lookup<R> {
    fn clone(&self) -> Self {
        Self {
            private_ok: Arc::clone(&self.private_ok),
            refused: self.refused,
        }
    }
}

impl<R> Lookup<R> {
    /// A lookup that keeps private addresses for the hosts `private_ok` names, and refuses a
    /// name that leads to none it may reach with `refused`'s error for its host.
    pub fn new(
        private_ok: impl Fn(&str) -> bool + Send + Sync + 'static,
        refused: fn(&str) -> R,
    ) -> Self {
        Self {
            private_ok: Arc::new(private_ok),
            refused,
        }
    }

    /// Of `found`, what `host` resolved to, the addresses a client may reach, or why there are
    /// none.
    pub fn kept(
        &self,
        host: &str,
        found: impl Iterator<Item = SocketAddr>,
    ) -> Result<Vec<SocketAddr>, R> {
        let private_ok = (self.private_ok)(host);
        let kept: Vec<SocketAddr> = found
            .filter(|addr| reachable(addr.ip(), private_ok))
            .collect();
        if kept.is_empty() {
            return Err((self.refused)(host));
        }
        Ok(kept)
    }
}

impl<R: Error + Send + Sync + 'static> Resolve for Lookup<R> {
    fn resolve(&self, name: Name) -> Resolving {
        let host = name.as_str().to_owned();
        let lookup = self.clone();
        Box::pin(async move {
            let found = tokio::net::lookup_host((host.as_str(), 0)).await?;
            let kept = lookup
                .kept(&host, found)
                .map_err(|refused| Box::new(refused) as Box<dyn Error + Send + Sync>)?;
            Ok(Box::new(kept.into_iter()) as Addrs)
        })
    }
}

#[cfg(test)]
mod tests {
    use std::fmt;

    use super::*;

    #[derive(Debug, PartialEq, Eq)]
    struct Refused(String);

    impl fmt::Display for Refused {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "{} is out of reach", self.0)
        }
    }

    impl Error for Refused {}

    fn refused(host: &str) -> Refused {
        Refused(host.to_owned())
    }

    fn found(ips: &[&str]) -> impl Iterator<Item = SocketAddr> {
        ips.iter()
            .map(|ip| SocketAddr::new(ip.parse().unwrap(), 0))
            .collect::<Vec<_>>()
            .into_iter()
    }

    #[test]
    fn only_the_addresses_a_client_may_reach_are_kept() {
        let public = Lookup::new(|_| false, refused);
        let kept = public.kept("a.example", found(&["10.0.0.1", "93.184.216.34"]));
        assert_eq!(kept.unwrap().len(), 1);
        let private = public.kept("a.example", found(&["127.0.0.1", "169.254.169.254"]));
        assert_eq!(private.unwrap_err(), Refused("a.example".into()));
        let here = Lookup::new(|host| host == "here.example", refused);
        assert_eq!(
            here.kept("here.example", found(&["10.0.0.1"]))
                .unwrap()
                .len(),
            1
        );
        assert!(here.kept("there.example", found(&["10.0.0.1"])).is_err());
        let metadata = here.kept("here.example", found(&["169.254.169.254"]));
        assert!(metadata.is_err(), "cloud metadata is never reached");
    }

    #[test]
    fn a_name_that_leads_only_to_this_computer_is_refused_in_its_own_words() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let lookup = Lookup::new(|_| false, refused);
        let looked_up = runtime.block_on(lookup.resolve("localhost".parse().unwrap()));
        let refusal = looked_up.err().unwrap();
        assert_eq!(refusal.to_string(), "localhost is out of reach");
        assert!(refusal.downcast_ref::<Refused>().is_some());
    }
}
