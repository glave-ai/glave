//! Package versions let in before they are old enough ([`super::OLD_ENOUGH`]): a security fix
//! that can't wait. Each entry is the registry, the package, the version and why it can't wait,
//! such as the advisory it fixes:
//!
//! ```text
//! (Registry::Crates, "tokio", "1.48.1", "RUSTSEC-2026-0001: a fix the advisory says can't wait"),
//! ```
//!
//! Once the version is old enough the check fails until its entry goes, so the list holds only
//! what still needs it.

use super::locked::Registry;

/// An entry: the registry, the package, its version, and why it can't wait.
pub(super) type Entry = (Registry, &'static str, &'static str, &'static str);

/// The versions let in early.
pub(super) const ALLOWED: &[Entry] = &[(
    Registry::Crates,
    "lettre",
    "0.11.24",
    "RUSTSEC-2026-0560 (STARTTLS response injection lets a man-in-the-middle forge SMTP \
     replies), RUSTSEC-2026-0561 (missing dot-stuffing after a bare LF or CR allows SMTP \
     command injection), RUSTSEC-2026-0562 (panic on an EHLO reply line of only white space), \
     RUSTSEC-2026-0563 (an address with a quoted local part is sent to a different address), \
     RUSTSEC-2026-0564 (panic on a mailbox display name containing CR or LF) and \
     RUSTSEC-2026-0565 (unbounded buffering of an SMTP reply), all fixed first in 0.11.24: a \
     security fix that can't wait, and cargo deny fails on every branch until it comes in",
)];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_entry_names_a_version_once_and_says_why() {
        for (at, (registry, name, version, why)) in ALLOWED.iter().enumerate() {
            assert!(
                !name.is_empty() && !version.is_empty(),
                "entry {at} names no package"
            );
            assert!(why.len() > 20, "{name} {version} needs its reason");
            let listed = ALLOWED
                .iter()
                .filter(|(r, n, v, _)| (r, n, v) == (registry, name, version))
                .count();
            assert_eq!(listed, 1, "{name} {version} is listed {listed} times");
        }
    }
}
