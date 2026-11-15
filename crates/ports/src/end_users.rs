//! The anonymous id a model call sends its provider for the person it works for
//! (`specs/gateway.md`, "Whose call it is"): a maker that finds a policy broken then acts on
//! that one person, not the whole account.

use std::fmt::{self, Write as _};

use glave_event::PrincipalId;
use hmac::{Hmac, KeyInit as _, Mac as _};
use sha2::Sha256;

use crate::scope::OrgId;

/// What every id starts with, so whoever runs a server knows one when a maker reports it.
const PREFIX: &str = "g_";

/// How many bytes of the MAC an id keeps: 128 bits, in 32 hex digits.
const KEPT: usize = 16;

/// One person's id in one organization, as providers are sent it: `g_` and 32 hex digits, an
/// HMAC-SHA-256 under a secret only the server holds. It says nothing of who they are, and
/// without that secret no one can tell whose it is.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EndUser(String);

impl EndUser {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for EndUser {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Makes each person's [`EndUser`] under the server's secret: the same for the same person in
/// the same organization, for as long as the secret lasts.
#[derive(Clone)]
pub struct EndUsers {
    /// The MAC keyed with the secret, cloned for each id: the secret itself is not kept.
    keyed: Hmac<Sha256>,
}

impl EndUsers {
    /// Ids under `secret`, 32 bytes only the server holds.
    pub fn new(secret: &[u8; 32]) -> Self {
        // HMAC pads a key shorter than its block with zeros, so the secret padded here is the
        // same key, taken without a length to check.
        let mut key = hmac::digest::Key::<Hmac<Sha256>>::default();
        key.iter_mut()
            .zip(secret)
            .for_each(|(to, from)| *to = *from);
        Self {
            keyed: Hmac::<Sha256>::new(&key),
        }
    }

    /// `person`'s id in `org`.
    pub fn id(&self, org: OrgId, person: PrincipalId) -> EndUser {
        let mut mac = self.keyed.clone();
        // Two ids of 16 bytes each: the message has one way to be read.
        mac.update(org.as_uuid().as_bytes());
        mac.update(person.as_uuid().as_bytes());
        let digest = mac.finalize().into_bytes();
        let mut id = String::with_capacity(PREFIX.len() + 2 * KEPT);
        id.push_str(PREFIX);
        for byte in digest.iter().take(KEPT) {
            let _ = write!(id, "{byte:02x}");
        }
        EndUser(id)
    }

    /// Which of `people`, each in their organization, `id` is, if any.
    pub fn find(
        &self,
        id: &str,
        people: impl IntoIterator<Item = (OrgId, PrincipalId)>,
    ) -> Option<(OrgId, PrincipalId)> {
        let id = id.trim();
        people
            .into_iter()
            .find(|(org, person)| self.id(*org, *person).as_str() == id)
    }
}

impl fmt::Debug for EndUsers {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("EndUsers(..)")
    }
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::*;

    fn org(n: u128) -> OrgId {
        OrgId::from_uuid(Uuid::from_u128(n))
    }

    fn person(n: u8) -> PrincipalId {
        PrincipalId::new(jiff::Timestamp::UNIX_EPOCH, [n; 10]).unwrap()
    }

    #[test]
    fn an_id_is_the_same_for_one_person_and_differs_for_anyone_else() {
        let ids = EndUsers::new(&[7; 32]);
        let id = ids.id(org(1), person(2));
        assert_eq!(id, ids.id(org(1), person(2)));
        assert_eq!(id, EndUsers::new(&[7; 32]).id(org(1), person(2)));
        assert_ne!(id, ids.id(org(1), person(3)), "another person");
        assert_ne!(id, ids.id(org(4), person(2)), "another organization");
        assert_ne!(
            id,
            EndUsers::new(&[8; 32]).id(org(1), person(2)),
            "another secret"
        );
        let mut plain = Hmac::<Sha256>::new_from_slice(&[7; 32]).unwrap();
        plain.update(org(1).as_uuid().as_bytes());
        plain.update(person(2).as_uuid().as_bytes());
        let hex: String = plain.finalize().into_bytes()[..16]
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        assert_eq!(
            id.as_str(),
            format!("g_{hex}"),
            "HMAC-SHA-256 under the secret itself"
        );
    }

    #[test]
    fn an_id_holds_nothing_but_its_prefix_and_hex() {
        let ids = EndUsers::new(&[7; 32]);
        let (org, person) = (org(0x1234), person(0x56));
        let id = ids.id(org, person);
        let hex = id.as_str().strip_prefix("g_").unwrap();
        assert_eq!(hex.len(), 32);
        assert!(hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')));
        for raw in [org.to_string(), person.to_string()] {
            let simple = raw.replace('-', "");
            assert!(!id.as_str().contains(&simple) && !id.as_str().contains(&raw));
        }
        assert_eq!(format!("{ids:?}"), "EndUsers(..)");
    }

    #[test]
    fn an_id_is_found_among_the_people_it_could_be() {
        let ids = EndUsers::new(&[7; 32]);
        let people = [
            (org(1), person(2)),
            (org(1), person(3)),
            (org(4), person(3)),
        ];
        for one in people {
            let id = ids.id(one.0, one.1);
            assert_eq!(ids.find(id.as_str(), people), Some(one));
            assert_eq!(ids.find(&format!(" {id}\n"), people), Some(one));
        }
        assert_eq!(ids.find("g_00000000000000000000000000000000", people), None);
        let other = EndUsers::new(&[8; 32]).id(org(1), person(2));
        assert_eq!(ids.find(other.as_str(), people), None);
    }
}
