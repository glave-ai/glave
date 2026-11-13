//! A host pattern: which hosts an entry of a list names, read one way for every list of hosts
//! glave keeps (`[web]`'s allow and deny).

use std::fmt;
use std::net::{IpAddr, Ipv6Addr};

use thiserror::Error;

use super::Host;

/// A host pattern: one host exactly (`example.com`, or an address, `127.0.0.1`, `[::1]`), any
/// name under one (`*.example.com`, never `example.com` itself), or every host (`*`); at any
/// port, or at the one port after it (`example.com:8443`). Names compare in any case, with or
/// without a final dot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostPattern {
    hosts: Hosts,
    port: Option<u16>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Hosts {
    Every,
    Exactly(Host),
    /// Any name under this one, which it is not.
    Under(String),
}

/// A pattern glave cannot read, and why.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{pattern:?} is not a host pattern: {why}")]
pub struct BadHostPattern {
    pattern: String,
    why: &'static str,
}

impl HostPattern {
    /// The pattern `text` is, spaces around it aside.
    pub fn parse(text: &str) -> Result<Self, BadHostPattern> {
        let bad = |why| BadHostPattern {
            pattern: text.to_owned(),
            why,
        };
        let not_a_host = || {
            bad(
                "a host is a name of letters, digits, dots, dashes and underscores, or an \
                 address, with *. before it for any name under it, such as *.example.com",
            )
        };
        let text = text.trim();
        if text.contains('/') {
            return Err(bad("name the host only, such as docs.rs or *.example.com"));
        }
        // A bare IPv6 address takes no port: its colons are its own.
        if let Ok(v6) = text.parse::<Ipv6Addr>() {
            return Ok(Self {
                hosts: Hosts::Exactly(Host::Address(IpAddr::V6(v6))),
                port: None,
            });
        }
        let (host, port) = match text.rsplit_once(':') {
            Some((host, port)) if !text.ends_with(']') => {
                let port = port.parse::<u16>().ok().filter(|port| *port != 0);
                (host, Some(port.ok_or_else(|| bad("a port is 1 to 65535"))?))
            }
            _ => (text, None),
        };
        let hosts = match (host, host.strip_prefix("*.").map(Host::parse)) {
            ("*", _) => Hosts::Every,
            (_, Some(Some(Host::Name(name)))) => Hosts::Under(name),
            (_, Some(_)) => return Err(not_a_host()),
            (_, None) => Hosts::Exactly(Host::parse(host).ok_or_else(not_a_host)?),
        };
        Ok(Self { hosts, port })
    }

    /// Whether this pattern names `host` at `port`.
    pub fn admits(&self, host: &Host, port: u16) -> bool {
        self.port.is_none_or(|own| own == port) && self.names(host)
    }

    /// Whether this pattern names `host`, at whatever port.
    pub fn names(&self, host: &Host) -> bool {
        match (&self.hosts, host) {
            (Hosts::Every, _) => true,
            // An IPv4 address written as IPv6 (`::ffff:a.b.c.d`) is that address.
            (Hosts::Exactly(Host::Address(own)), Host::Address(ip)) => {
                own.to_canonical() == ip.to_canonical()
            }
            (Hosts::Exactly(Host::Name(own)), Host::Name(name)) => own == name,
            (Hosts::Under(own), Host::Name(name)) => name
                .strip_suffix(own.as_str())
                .and_then(|before| before.strip_suffix('.'))
                .is_some_and(|label| !label.is_empty()),
            _ => false,
        }
    }

    /// Whether it names one host, with no `*`.
    pub fn is_exact(&self) -> bool {
        matches!(self.hosts, Hosts::Exactly(_))
    }
}

impl fmt::Display for HostPattern {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.hosts {
            Hosts::Every => f.write_str("*")?,
            Hosts::Exactly(host) => write!(f, "{host}")?,
            Hosts::Under(name) => write!(f, "*.{name}")?,
        }
        match self.port {
            Some(port) => write!(f, ":{port}"),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests;
