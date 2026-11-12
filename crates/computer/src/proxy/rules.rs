//! What the proxy does with a host: the mode and the hosts allowed decide by the name alone,
//! before anything is looked up, and then the addresses it resolves to are kept or refused by
//! glave's one rule ([`reachable`]). Pure: no lookups, no sockets.

use std::fmt;
use std::net::IpAddr;

use glave_ports::host::reachable;

/// How much of the network commands reach through the proxy (`specs/sandbox.md`, "The outbound
/// proxy").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Nothing goes out.
    Offline,
    /// The hosts allowed go out; any other is held for the person.
    Guarded,
    /// Any public host goes out, at any port.
    Open,
}

/// The ports an entry that names none allows: the web's.
const WEB_PORTS: [u16; 2] = [80, 443];

/// A host commands may reach in [`Mode::Guarded`]: one name exactly (`example.com`), or any
/// name under one (`*.example.com`, never `example.com` itself), at the web's ports (80 and
/// 443) or at the one port it names (`example.com:8443`). Names are kept in lower case with
/// no final dot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Allowed {
    name: String,
    under: bool,
    port: Option<u16>,
}

impl Allowed {
    /// The host `text` names, with `*.` before it or not and a port after it or not, or none
    /// when it is not a host name.
    pub fn parse(text: &str) -> Option<Self> {
        let (host, port) = match text.rsplit_once(':') {
            Some((host, port)) => (host, Some(port.parse::<u16>().ok().filter(|p| *p != 0)?)),
            None => (text, None),
        };
        let (under, rest) = match host.strip_prefix("*.") {
            Some(rest) => (true, rest),
            None => (false, host),
        };
        let name = Host::name(rest)?;
        Some(Self { name, under, port })
    }

    /// Whether `name`, in lower case with no final dot, at `port`, is this host or one under
    /// it, at a port it allows.
    fn admits(&self, name: &str, port: u16) -> bool {
        let at = self
            .port
            .map_or(WEB_PORTS.contains(&port), |own| own == port);
        at && self.names(name)
    }

    fn names(&self, name: &str) -> bool {
        if self.under {
            name.strip_suffix(self.name.as_str())
                .and_then(|before| before.strip_suffix('.'))
                .is_some_and(|label| !label.is_empty())
        } else {
            name == self.name
        }
    }
}

/// A host a connection names: a name, kept in lower case with no final dot, or an address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Host {
    /// A name, which the proxy looks up only once it is allowed.
    Name(String),
    /// An address, written as the host (`93.184.216.34`, `[2606:4700::1111]`).
    Address(IpAddr),
}

impl Host {
    /// The host `text` names: an IPv4 address, a bracketed IPv6 one, or a name of letters,
    /// digits, `-` and `_` in labels of 1 to 63 characters, 253 in all, not all of them digits
    /// (which a resolver would read as an address, `127.1`).
    pub fn parse(text: &str) -> Option<Self> {
        if let Some(v6) = text.strip_prefix('[').and_then(|t| t.strip_suffix(']')) {
            return v6.parse().ok().map(Self::Address);
        }
        if let Ok(address) = text.parse::<std::net::Ipv4Addr>() {
            return Some(Self::Address(IpAddr::V4(address)));
        }
        Self::name(text).map(Self::Name)
    }

    fn name(text: &str) -> Option<String> {
        let name = text.strip_suffix('.').unwrap_or(text).to_ascii_lowercase();
        let labels_ok = name.split('.').all(|label| {
            (1..=63).contains(&label.len())
                && label
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        });
        // A name that reads as an IPv4 address is one; it is not a name (`1.2.3.4.`).
        let numeric = name
            .split('.')
            .all(|label| label.chars().all(|c| c.is_ascii_digit()));
        (labels_ok && !numeric && name.len() <= 253).then_some(name)
    }
}

impl fmt::Display for Host {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Name(name) => f.write_str(name),
            Self::Address(IpAddr::V6(v6)) => write!(f, "[{v6}]"),
            Self::Address(IpAddr::V4(v4)) => write!(f, "{v4}"),
        }
    }
}

/// What the proxy does with a host before looking anything up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Step {
    /// Look it up (a name) or check it (an address), then connect to what passes.
    Go,
    /// Hold the connection for the person's yes.
    Ask,
    /// Refuse it.
    Refuse(Refusal),
}

/// Why a connection was refused, as the model is told it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Refusal {
    /// The network is off for commands.
    Off,
    /// The host is not allowed, and no one said yes.
    NotAllowed,
    /// The host is on a private network or serves cloud metadata, withheld on purpose.
    Withheld,
    /// The name was not found.
    NotFound,
}

impl fmt::Display for Refusal {
    /// The one sentence the model is told.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Off => "The network is off for commands here.",
            Self::NotAllowed => {
                "This host is not one commands here may reach, and no one allowed it; ask the \
                 person to allow it."
            }
            Self::Withheld => {
                "This host is on a private network or serves cloud metadata, which is withheld \
                 from commands on purpose; ask the person if it is needed rather than looking \
                 for another way in."
            }
            Self::NotFound => "This host's name was not found.",
        })
    }
}

/// The mode and the hosts allowed: what the proxy decides each connection by.
#[derive(Debug, Clone)]
pub struct Rules {
    mode: Mode,
    allowed: Vec<Allowed>,
}

impl Rules {
    /// Rules of `mode`, allowing `allowed` in [`Mode::Guarded`].
    pub fn new(mode: Mode, allowed: impl IntoIterator<Item = Allowed>) -> Self {
        Self {
            mode,
            allowed: allowed.into_iter().collect(),
        }
    }

    /// What to do with `host` at `port`, by its name alone: nothing is looked up before this
    /// says to go. In [`Mode::Guarded`] an address, which no entry names, is held like any
    /// host not allowed.
    pub(crate) fn step(&self, host: &Host, port: u16) -> Step {
        match (self.mode, host) {
            (Mode::Offline, _) => Step::Refuse(Refusal::Off),
            (Mode::Open, _) => Step::Go,
            (Mode::Guarded, Host::Name(name))
                if self.allowed.iter().any(|a| a.admits(name, port)) =>
            {
                Step::Go
            }
            (Mode::Guarded, _) => Step::Ask,
        }
    }
}

/// Of `found`, the addresses a host resolved to (or the one it is), those a command may reach:
/// public addresses only, never a private network, link-local, loopback or cloud metadata in
/// any form. None left is a refusal: withheld when there were some, not found when there were
/// none.
pub(crate) fn kept(found: &[IpAddr]) -> Result<Vec<IpAddr>, Refusal> {
    if found.is_empty() {
        return Err(Refusal::NotFound);
    }
    let kept: Vec<IpAddr> = found
        .iter()
        .copied()
        .filter(|ip| reachable(*ip, false))
        .collect();
    if kept.is_empty() {
        return Err(Refusal::Withheld);
    }
    Ok(kept)
}

#[cfg(test)]
mod tests;
