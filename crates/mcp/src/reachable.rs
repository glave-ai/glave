//! Which addresses glave's own client reaches for a remote server, checked at each connection
//! after its name is looked up, so a name cannot resolve to a public address when checked and a
//! private one when connected. An address in the URL itself is looked up by no one, so it is
//! checked apart, with this computer's own name ([`Reachable::refuses`]). The rule is glave's one list
//! ([`glave_ports::host::reachable`]), which `web_fetch` holds to too; a name resolved through a
//! proxy is not seen here, so where only public addresses will do the client takes no proxy.

use std::fmt;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use reqwest::dns::{Addrs, Name, Resolve, Resolving};

/// The addresses a remote server may be at. Cloud metadata is out of reach in both, as a tool
/// server is never there.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Reachable {
    /// Private addresses as well as public ones: for the person on their own computer, whose
    /// own network it is, and for servers an operator's file names.
    #[default]
    AlsoPrivate,
    /// Public ones alone: where people sign in from elsewhere, so that no address a person or a
    /// repository names reaches the server's own network.
    OnlyPublic,
}

impl Reachable {
    fn allows(self, ip: IpAddr) -> bool {
        glave_ports::host::reachable(ip, self == Self::AlsoPrivate)
    }

    /// Why `url` is refused, when its host is an address that may not be reached, or this
    /// computer by name where it may not be. Any other name is checked as it is looked up.
    pub(crate) fn refuses(self, url: &str) -> Option<Refused> {
        // The URL parser has already read decimal, octal and hex spellings of addresses.
        let url = reqwest::Url::parse(url).ok()?;
        let host = url.host_str()?;
        let bare = host
            .strip_prefix('[')
            .and_then(|inside| inside.strip_suffix(']'))
            .unwrap_or(host);
        let refused = match bare.parse::<IpAddr>() {
            Ok(ip) => !self.allows(ip),
            Err(_) => self == Self::OnlyPublic && glave_ports::host::is_this_computer(host),
        };
        refused.then(|| Refused {
            host: host.to_owned(),
            reachable: self,
        })
    }

    /// A client whose names are looked up under this rule.
    pub(crate) fn client(self) -> reqwest::ClientBuilder {
        let client = reqwest::Client::builder().dns_resolver(Arc::new(Guarded { reachable: self }));
        match self {
            Self::AlsoPrivate => client,
            Self::OnlyPublic => client.no_proxy(),
        }
    }
}

/// A host that leads only to addresses that may not be reached.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Refused {
    host: String,
    reachable: Reachable,
}

impl fmt::Display for Refused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let host = &self.host;
        match self.reachable {
            Reachable::AlsoPrivate => write!(
                f,
                "{host} is where a cloud serves its machines' credentials, which glave does not \
                 reach for a tool server"
            ),
            Reachable::OnlyPublic => write!(
                f,
                "{host} is a private address, or only leads to them (this computer, a local \
                 network or cloud metadata), which glave does not reach where people sign in \
                 from elsewhere"
            ),
        }
    }
}

impl std::error::Error for Refused {}

/// Looks names up with the system resolver, keeping only the addresses allowed.
#[derive(Clone, Copy)]
struct Guarded {
    reachable: Reachable,
}

impl Guarded {
    /// Of `found`, the addresses allowed, or why there are none.
    fn kept(
        self,
        host: &str,
        found: impl Iterator<Item = SocketAddr>,
    ) -> Result<Vec<SocketAddr>, Refused> {
        let kept: Vec<SocketAddr> = found
            .filter(|addr| self.reachable.allows(addr.ip()))
            .collect();
        if kept.is_empty() {
            return Err(Refused {
                host: host.to_owned(),
                reachable: self.reachable,
            });
        }
        Ok(kept)
    }
}

impl Resolve for Guarded {
    fn resolve(&self, name: Name) -> Resolving {
        let host = name.as_str().to_owned();
        let guarded = *self;
        Box::pin(async move {
            let found = tokio::net::lookup_host((host.as_str(), 0)).await?;
            let kept = guarded.kept(&host, found)?;
            let addrs: Addrs = Box::new(kept.into_iter());
            Ok(addrs)
        })
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use glave_ports::testing::FakeHttp;

    use super::*;

    fn kept(reachable: Reachable, found: &[&str]) -> Result<Vec<String>, Refused> {
        let found = found
            .iter()
            .map(|ip| SocketAddr::new(ip.parse().unwrap(), 0));
        let kept = Guarded { reachable }.kept("a.example.com", found)?;
        Ok(kept.iter().map(|addr| addr.ip().to_string()).collect())
    }

    const METADATA: [&str; 3] = ["169.254.169.254", "::ffff:169.254.169.254", "fd00:ec2::254"];
    const PRIVATE: [&str; 5] = ["127.0.0.1", "::1", "10.0.0.1", "::ffff:10.0.0.1", "fc00::1"];

    #[test]
    fn a_name_keeps_only_the_addresses_allowed() {
        for reachable in [Reachable::AlsoPrivate, Reachable::OnlyPublic] {
            let refused = kept(reachable, &METADATA).unwrap_err();
            assert_eq!(refused.host, "a.example.com");
            let some = kept(reachable, &["169.254.169.254", "93.184.216.34"]).unwrap();
            assert_eq!(
                some,
                ["93.184.216.34"],
                "only the public one is connected to"
            );
        }
        assert!(kept(Reachable::OnlyPublic, &PRIVATE).is_err());
        assert_eq!(
            kept(Reachable::AlsoPrivate, &PRIVATE).unwrap().len(),
            PRIVATE.len()
        );
    }

    #[test]
    fn an_address_in_a_url_is_judged_as_a_name_s_is() {
        let at = |ip: &str| match ip.parse().unwrap() {
            IpAddr::V4(_) => format!("http://{ip}/mcp"),
            IpAddr::V6(_) => format!("http://[{ip}]:8080/mcp"),
        };
        for reachable in [Reachable::AlsoPrivate, Reachable::OnlyPublic] {
            for ip in METADATA {
                assert!(reachable.refuses(&at(ip)).is_some(), "{ip}");
            }
            // 169.254.169.254, spelled as one number and in hex.
            assert!(reachable.refuses("http://2852039166/mcp").is_some());
            assert!(reachable.refuses("http://0xa9fea9fe/mcp").is_some());
            assert!(reachable.refuses("https://93.184.216.34/mcp").is_none());
            assert!(reachable.refuses("https://mcp.example.com/mcp").is_none());
            assert!(
                reachable.refuses("not a url").is_none(),
                "its client says why"
            );
        }
        assert!(
            Reachable::OnlyPublic
                .refuses("http://LocalHost:80/mcp")
                .is_some()
        );
        assert!(
            Reachable::AlsoPrivate
                .refuses("http://localhost:80/mcp")
                .is_none()
        );
        for ip in PRIVATE {
            assert!(Reachable::OnlyPublic.refuses(&at(ip)).is_some(), "{ip}");
            assert!(Reachable::AlsoPrivate.refuses(&at(ip)).is_none(), "{ip}");
        }
    }

    #[test]
    fn the_client_looks_names_up_under_the_rule() {
        let here = FakeHttp::always(200, "here").unwrap();
        let port = here.host.rsplit_once(':').unwrap().1;
        let url = format!("http://localhost:{port}/");
        glave_ports::block_on!(async {
            let get = |reachable: Reachable| {
                let client = reachable.client().build().unwrap();
                client.get(&url).send()
            };
            let error = get(Reachable::OnlyPublic).await.unwrap_err();
            let refused = std::iter::successors(Some(&error as &dyn Error), |&e| e.source())
                .any(|e| e.downcast_ref::<Refused>().is_some());
            assert!(refused, "{error}");
            assert_eq!(get(Reachable::AlsoPrivate).await.unwrap().status(), 200);
        });
        assert_eq!(here.received().len(), 1, "reached only where it may be");
    }

    #[test]
    fn a_name_that_looks_up_to_this_computer_is_refused_only_where_people_sign_in_from_elsewhere() {
        glave_ports::block_on!(async {
            let name = |text: &str| text.parse::<Name>().unwrap();
            let looked_up = |reachable| Guarded { reachable }.resolve(name("localhost"));
            let refused = looked_up(Reachable::OnlyPublic).await.err().unwrap();
            assert!(refused.downcast_ref::<Refused>().is_some(), "{refused}");
            let found: Vec<SocketAddr> = looked_up(Reachable::AlsoPrivate)
                .await
                .ok()
                .unwrap()
                .collect();
            assert!(
                found.iter().all(|addr| addr.ip().is_loopback()),
                "{found:?}"
            );
        });
    }
}
