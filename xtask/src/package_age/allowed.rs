//! Package versions let in before they are old enough ([`super::OLD_ENOUGH`]): a security fix
//! that can't wait. Each entry is the registry, the package, the version, when its registry
//! says it was published (crates.io's `created_at`, npm's `time`, as written there), and why it
//! can't wait, such as the advisory it fixes:
//!
//! ```text
//! Entry {
//!     registry: Registry::Crates,
//!     name: "tokio",
//!     version: "1.48.1",
//!     published: "2026-10-24T14:02:11.069149Z",
//!     why: "RUSTSEC-2026-0001: a fix the advisory says can't wait",
//! },
//! ```
//!
//! Once the version is old enough by the time written here, the check fails until its entry
//! goes, so the list holds only what still needs it, and no registry is asked to say so. A
//! change that brings the version in asks its registry, which must say the same time.

use super::locked::Registry;

/// A version let in early.
#[derive(Debug, Clone, Copy)]
pub(super) struct Entry {
    /// The registry it comes from.
    pub(super) registry: Registry,
    /// The package.
    pub(super) name: &'static str,
    /// Its version.
    pub(super) version: &'static str,
    /// When its registry says it was published, in RFC 3339.
    pub(super) published: &'static str,
    /// Why it can't wait.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "written for its reader; only the test reads it")
    )]
    pub(super) why: &'static str,
}

/// The versions let in early.
pub(super) const ALLOWED: &[Entry] = &[
    Entry {
        registry: Registry::Crates,
        name: "lettre",
        version: "0.11.24",
        published: "2026-11-17T16:58:40.930952Z",
        why: "RUSTSEC-2026-0560 (STARTTLS response injection lets a man-in-the-middle forge \
              SMTP replies), RUSTSEC-2026-0561 (missing dot-stuffing after a bare LF or CR \
              allows SMTP command injection), RUSTSEC-2026-0562 (panic on an EHLO reply line \
              of only white space), RUSTSEC-2026-0563 (an address with a quoted local part is \
              sent to a different address), RUSTSEC-2026-0564 (panic on a mailbox display \
              name containing CR or LF) and RUSTSEC-2026-0565 (unbounded buffering of an SMTP \
              reply), all fixed first in 0.11.24: a security fix that can't wait, and cargo \
              deny fails on every branch until it comes in",
    },
    Entry {
        registry: Registry::Crates,
        name: "url",
        version: "2.5.9",
        published: "2026-11-20T23:03:07.148548Z",
        why: "RUSTSEC-2026-0592 (Url::origin() recurses once per nested `blob:` prefix, so \
              one long URL overflows the stack and aborts the process), fixed first in 2.5.9: \
              a security fix that can't wait, and cargo deny fails on every branch until it \
              comes in",
    },
    Entry {
        registry: Registry::Crates,
        name: "form_urlencoded",
        version: "1.2.3",
        published: "2026-11-20T23:02:49.484853Z",
        why: "url 2.5.9, the fix for RUSTSEC-2026-0592, requires form_urlencoded ^1.2.3, \
              published with it: the advisory's fix can't come in without it",
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_entry_names_a_version_once_says_when_it_was_published_and_why() {
        for (at, entry) in ALLOWED.iter().enumerate() {
            let Entry {
                registry,
                name,
                version,
                published,
                why,
            } = *entry;
            assert!(
                !name.is_empty() && !version.is_empty(),
                "entry {at} names no package"
            );
            assert!(why.len() > 20, "{name} {version} needs its reason");
            assert!(
                published.parse::<jiff::Timestamp>().is_ok(),
                "{name} {version}: `{published}` is not a time"
            );
            let listed = ALLOWED
                .iter()
                .filter(|e| (e.registry, e.name, e.version) == (registry, name, version))
                .count();
            assert_eq!(listed, 1, "{name} {version} is listed {listed} times");
        }
    }
}
