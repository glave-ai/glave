//! Which addresses glave's own client reaches for a remote server, checked at each connection
//! after its name is looked up, so a name cannot resolve to a public address when checked and a
//! private one when connected. An address in the URL itself is looked up by no one, so it is
//! checked apart, with this computer's own name ([`Reach::refuses`]). The rule is glave's one list
//! ([`glave_ports::host::reachable`]), and names are looked up as `web_fetch` looks them up
//! ([`Lookup`]); a name resolved through a proxy is not seen here, so where only public addresses
//! will do the client takes no proxy.

use std::fmt;
use std::net::IpAddr;
use std::sync::Arc;

use glave_ports::host::{Lookup, is_this_computer};

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
    /// Why `url` is refused for a server: [`Reach::refuses`] under this rule.
    pub(crate) fn refuses(self, url: &str) -> Option<Refused> {
        Reach::from(self).refuses(url)
    }

    /// A client for a server, whose names are looked up under this rule.
    pub(crate) fn client(self) -> reqwest::ClientBuilder {
        Reach::from(self).client()
    }

    /// What signing in to the MCP server at `url` reaches. Its metadata names the next address
    /// (its authorization server, then where glave registers and trades for tokens), so a
    /// server on the internet could otherwise send glave's requests into the person's own
    /// network. Where private addresses are reached at all, a server whose own address is one
    /// as its URL writes it (this computer, by name or address, or a private address) signs in
    /// under the same rule; any other reaches a private address only at its own host, which its
    /// calls reach anyway. rmcp's own client decides alike, from the URL as written, since what
    /// a server's own name looks up to is the server's to say.
    pub(crate) fn signing_in_to(self, url: &str) -> Reach {
        let host = reqwest::Url::parse(url)
            .ok()
            .and_then(|url| url.host_str().map(str::to_owned));
        match (self, host) {
            (Self::AlsoPrivate, Some(host)) if !is_private(&host) => Reach::Only(host),
            (Self::AlsoPrivate, Some(_)) => Reach::Anywhere,
            (Self::OnlyPublic, _) | (_, None) => Reach::Nowhere,
        }
    }
}

/// Whether `host`, as a URL writes it, is this computer or a private address.
fn is_private(host: &str) -> bool {
    is_this_computer(host)
        || address(host).is_some_and(|ip| !glave_ports::host::reachable(ip, false))
}

/// The address `host` is, as a URL writes it, bracketed or not.
fn address(host: &str) -> Option<IpAddr> {
    let bare = host
        .strip_prefix('[')
        .and_then(|inside| inside.strip_suffix(']'))
        .unwrap_or(host);
    bare.parse().ok()
}

/// The hosts a client may reach at a private address. Cloud metadata is out of reach at every
/// one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Reach {
    /// Every one ([`Reachable::AlsoPrivate`]).
    Anywhere,
    /// None ([`Reachable::OnlyPublic`]).
    Nowhere,
    /// This one alone: the host of a server on the internet, while signing in to it where
    /// private addresses are reached ([`Reachable::signing_in_to`]).
    Only(String),
}

impl From<Reachable> for Reach {
    fn from(reachable: Reachable) -> Self {
        match reachable {
            Reachable::AlsoPrivate => Self::Anywhere,
            Reachable::OnlyPublic => Self::Nowhere,
        }
    }
}

impl Reach {
    fn private_ok(&self, host: &str) -> bool {
        match self {
            Self::Anywhere => true,
            Self::Nowhere => false,
            Self::Only(own) => own.eq_ignore_ascii_case(host),
        }
    }

    /// Why `url` is refused, when its host is an address that may not be reached, or this
    /// computer by name where it may not be. Any other name is checked as it is looked up.
    pub(crate) fn refuses(&self, url: &str) -> Option<Refused> {
        // The URL parser has already read decimal, octal and hex spellings of addresses.
        let url = reqwest::Url::parse(url).ok()?;
        let host = url.host_str()?;
        let private_ok = self.private_ok(host);
        let refused = match address(host) {
            Some(ip) => !glave_ports::host::reachable(ip, private_ok),
            None => !private_ok && is_this_computer(host),
        };
        refused.then(|| Refused::by(host, self.fence()))
    }

    /// What keeps a host out of reach under this rule.
    fn fence(&self) -> Fence {
        match self {
            Self::Anywhere => Fence::Metadata,
            Self::Nowhere => Fence::Elsewhere,
            Self::Only(_) => Fence::Internet,
        }
    }

    /// The lookup of names under this rule, refusing a name that leads only to addresses out
    /// of its reach in its own words.
    fn lookup(&self) -> Lookup<Refused> {
        let refused: fn(&str) -> Refused = match self.fence() {
            Fence::Metadata => |host| Refused::by(host, Fence::Metadata),
            Fence::Elsewhere => |host| Refused::by(host, Fence::Elsewhere),
            Fence::Internet => |host| Refused::by(host, Fence::Internet),
        };
        let reach = self.clone();
        Lookup::new(move |host| reach.private_ok(host), refused)
    }

    /// A client whose names are looked up under this rule.
    pub(crate) fn client(&self) -> reqwest::ClientBuilder {
        let client = reqwest::Client::builder().dns_resolver(Arc::new(self.lookup()));
        match self {
            Self::Anywhere => client,
            Self::Nowhere | Self::Only(_) => client.no_proxy(),
        }
    }
}

/// What keeps a host out of reach.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Fence {
    /// Cloud metadata alone, which no rule reaches.
    Metadata,
    /// Private addresses too, where people sign in from elsewhere.
    Elsewhere,
    /// Private addresses too, while signing in to a server on the internet.
    Internet,
}

/// A host that leads only to addresses that may not be reached.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Refused {
    host: String,
    fence: Fence,
}

impl Refused {
    fn by(host: &str, fence: Fence) -> Self {
        Self {
            host: host.to_owned(),
            fence,
        }
    }
}

impl fmt::Display for Refused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let host = &self.host;
        let private = "is a private address, or only leads to them (this computer, a local \
                       network or cloud metadata), which glave does not reach";
        match self.fence {
            Fence::Metadata => write!(
                f,
                "{host} is where a cloud serves its machines' credentials, which glave does not \
                 reach for a tool server"
            ),
            Fence::Elsewhere => write!(f, "{host} {private} where people sign in from elsewhere"),
            Fence::Internet => write!(
                f,
                "{host} {private} while signing in to a server on the internet"
            ),
        }
    }
}

impl std::error::Error for Refused {}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::net::SocketAddr;

    use glave_ports::testing::FakeHttp;
    use reqwest::dns::{Name, Resolve as _};

    use super::*;

    /// What `reach` connects to of `found`, what `host` looked up to.
    fn kept_at(reach: &Reach, host: &str, found: &[&str]) -> Result<Vec<String>, Refused> {
        let found = found
            .iter()
            .map(|ip| SocketAddr::new(ip.parse().unwrap(), 0));
        let kept = reach.lookup().kept(host, found)?;
        Ok(kept.iter().map(|addr| addr.ip().to_string()).collect())
    }

    fn kept(reachable: Reachable, found: &[&str]) -> Result<Vec<String>, Refused> {
        kept_at(&reachable.into(), "a.example.com", found)
    }

    /// What signing in to a server on the internet reaches, on the person's own computer.
    fn on_the_internet() -> Reach {
        Reachable::AlsoPrivate.signing_in_to("https://MCP.example.com/mcp")
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
    fn a_sign_in_reaches_private_addresses_only_for_a_server_at_one() {
        for url in [
            "http://127.0.0.1:8080/mcp",
            "http://LocalHost:8080/mcp",
            "http://[::1]:8080/mcp",
            "https://10.0.0.5/mcp",
            "https://100.64.0.5/mcp",
            "https://[fd00::5]/mcp",
        ] {
            let here = Reachable::AlsoPrivate.signing_in_to(url);
            assert_eq!(here, Reach::Anywhere, "{url}");
            let elsewhere = Reachable::OnlyPublic.signing_in_to(url);
            assert_eq!(elsewhere, Reach::Nowhere, "{url}");
        }
        let elsewhere = Reachable::OnlyPublic.signing_in_to("https://mcp.example.com/mcp");
        assert_eq!(elsewhere, Reach::Nowhere);
        let unread = Reachable::AlsoPrivate.signing_in_to("not a url");
        assert_eq!(unread, Reach::Nowhere);
        let internet = on_the_internet();
        // Its own host, which its calls reach anyway, as before.
        let own = kept_at(&internet, "mcp.example.com", &PRIVATE).unwrap();
        assert_eq!(own.len(), PRIVATE.len());
        assert!(kept_at(&internet, "mcp.example.com", &METADATA).is_err());
        // Any other only at its public addresses.
        let refused = kept_at(&internet, "auth.example.com", &PRIVATE).unwrap_err();
        let said = refused.to_string();
        assert!(
            said.contains("signing in to a server on the internet"),
            "{said}"
        );
        let some = kept_at(
            &internet,
            "auth.example.com",
            &["10.0.0.1", "93.184.216.34"],
        );
        assert_eq!(some.unwrap(), ["93.184.216.34"]);
    }

    #[test]
    fn a_sign_in_to_a_server_on_the_internet_judges_an_address_in_a_url_alike() {
        let internet = on_the_internet();
        for url in [
            "http://127.0.0.1:8080/token",
            "http://[::1]/register",
            "http://LOCALHOST/register",
            "http://2130706433/token",
            "https://10.0.0.9/register",
            "https://[::ffff:192.168.1.1]/token",
            "https://100.64.0.1/",
            "http://169.254.169.254/",
        ] {
            assert!(internet.refuses(url).is_some(), "{url}");
        }
        assert!(internet.refuses("https://93.184.216.34/token").is_none());
        let named = internet.refuses("https://auth.example.com/token");
        assert!(named.is_none(), "judged as it is looked up");
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
            let looked_up =
                |reachable: Reachable| Reach::from(reachable).lookup().resolve(name("localhost"));
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
