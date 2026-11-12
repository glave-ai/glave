//! What the proxy does with a host: the mode and the hosts allowed decide by the name alone,
//! before anything is looked up, and then the addresses it resolves to are kept or refused by
//! glave's one rule ([`reachable`]). Pure: no lookups, no sockets.

use std::fmt;
use std::net::IpAddr;

use glave_ports::computer::NetworkMode;
use glave_ports::host::{Host, HostPattern, reachable};

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
    mode: NetworkMode,
    allowed: Vec<HostPattern>,
}

impl Rules {
    /// Rules of `mode`, allowing `allowed` in [`NetworkMode::Guarded`].
    pub fn new(mode: NetworkMode, allowed: impl IntoIterator<Item = HostPattern>) -> Self {
        Self {
            mode,
            allowed: allowed.into_iter().collect(),
        }
    }

    /// What to do with `host` at `port`, by its name alone: nothing is looked up before this
    /// says to go. In [`NetworkMode::Guarded`] a host no entry admits is held.
    pub(crate) fn step(&self, host: &Host, port: u16) -> Step {
        match self.mode {
            NetworkMode::Offline => Step::Refuse(Refusal::Off),
            NetworkMode::Open => Step::Go,
            NetworkMode::Guarded => {
                let named = host.to_string();
                if self.allowed.iter().any(|a| a.admits(&named, port)) {
                    Step::Go
                } else {
                    Step::Ask
                }
            }
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
