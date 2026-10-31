//! What a glave account's person is called (`specs/model.md`, "Identity"): the name they give
//! with their first passkey, which they can change. It is kept in the people ledger of each
//! organization they are in, with what the organization knows of them (`person.joined`,
//! recorded again when it changes), so each reads it as it reads the rest, and erasing the
//! organization takes it too. A company login's people are named by its token, and the person
//! on the server's computer, whom no ledger records, by no one.

use glave_event::PrincipalId;
use glave_identity::signin::Accounts;
use glave_ports::{BlobStore, Caller, EventLog, ModelProvider};

use crate::organizations::with_account;
use crate::protocol::{ApiError, Reply, code};
use crate::server::Server;
use crate::words::Sentence;

/// The most characters a name may have.
const LONGEST: usize = 100;

/// Whether `letter` is one a name can't hold: what breaks its line, what turns the text around
/// it (which could dress one name as another) or what can't be seen. The joiners that some
/// scripts need are not among them.
fn refused(letter: char) -> bool {
    letter.is_control()
        || matches!(
            letter,
            '\u{200B}'
                | '\u{200E}'
                | '\u{200F}'
                | '\u{2028}'..='\u{202E}'
                | '\u{2060}'..='\u{206F}'
                | '\u{FEFF}'
        )
}

/// `name` as glave keeps it, without the space around it; refused when nothing can be seen of
/// it, it is too long or it holds what [`refused`] says.
pub(crate) fn given_name(name: &str) -> Result<String, ApiError> {
    let name = name.trim();
    let seen = name
        .chars()
        .any(|letter| !letter.is_whitespace() && !matches!(letter, '\u{200C}' | '\u{200D}'));
    if !seen || name.chars().count() > LONGEST || name.chars().any(refused) {
        return Err(ApiError::new(code::INVALID_PARAMS, Sentence::NotAName));
    }
    Ok(name.to_owned())
}

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// Who `caller` is and the accounts that keep them, when they have a glave account here.
    pub(crate) fn account_of(&self, caller: &Caller) -> Option<(PrincipalId, &Accounts)> {
        let accounts = self.inner.accounts.as_deref()?;
        with_account(caller).then_some(())?;
        Some((caller.person?, accounts))
    }

    /// The name `caller` goes by, for someone with a glave account who gave one: the first of
    /// their organizations' that knows it.
    pub(crate) async fn account_name(&self, caller: &Caller) -> Result<Option<String>, ApiError> {
        let Some((person, accounts)) = self.account_of(caller) else {
            return Ok(None);
        };
        for org in accounts.organizations(person).await? {
            self.catch_up_people(org).await?;
            let name = self.people(org, |people| people.info(person).and_then(|info| info.name));
            if name.is_some() {
                return Ok(name);
            }
        }
        Ok(None)
    }

    /// Answers `person/name`: the name goes to every organization the caller is in, recorded
    /// where it is new; asked again, what failed partway is finished.
    pub(crate) async fn name_person(&self, caller: &Caller, name: &str) -> Result<Reply, ApiError> {
        let name = given_name(name)?;
        let Some((person, accounts)) = self.account_of(caller) else {
            return Err(ApiError::new(code::NOT_ALLOWED, Sentence::NameNotOurs));
        };
        for org in accounts.organizations(person).await? {
            self.record_person(org, |people| {
                Ok(people.joined(person).and_then(|joined| {
                    let new = joined.profile.name.as_deref() != Some(name.as_str());
                    new.then(|| {
                        let mut joined = joined.clone();
                        joined.profile.name = Some(name.clone());
                        joined
                    })
                }))
            })
            .await?;
        }
        Ok(Reply::Done {})
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_is_one_line_that_can_be_seen_of_at_most_a_hundred_characters() {
        assert_eq!(given_name("  Ada Lovelace ").unwrap(), "Ada Lovelace");
        assert_eq!(given_name("李雷").unwrap(), "李雷");
        assert_eq!(given_name("می\u{200C}خواهم").unwrap(), "می\u{200C}خواهم");
        let wrong = [
            "",
            "   ",
            "Ada\nLovelace",
            "Ada\u{2028}Lovelace",
            "Ada\u{202E}ecalevoL",
            "\u{2066}Ada",
            "\u{200B}",
            "\u{200D}",
        ];
        for wrong in wrong {
            assert!(given_name(wrong).is_err(), "{wrong:?}");
        }
        assert!(given_name(&"a".repeat(101)).is_err());
        assert!(given_name(&"a".repeat(100)).is_ok());
    }
}
