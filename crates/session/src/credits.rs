//! The organization's credits (`specs/gateway.md`, "Credits"), where the server's own
//! configuration keeps them: `credits/get` answers as much of them as the caller may see, its
//! Owners and Admins the whole account and everyone's spending, anyone else their own;
//! `credits/limit` sets the month's limit, for its Owners alone (`Permit::Billing`, checked
//! with the other admin methods); and `credits/add` is the operation money comes in by, for
//! whoever runs the server today and the billing service that calls it later.

use std::cmp::Reverse;

use glave_event::{At, Currency, Money, OrgRole, Principal, PrincipalId};
use glave_gateway::{Account, Credits};
use glave_ports::{BlobStore, Caller, EventLog, ModelProvider, OrgId};

use crate::people::People;
use crate::protocol::{ApiError, CreditsAccount, CreditsInfo, PersonSpent, Reply, code};
use crate::providers::local;
use crate::server::Server;
use crate::words::Sentence;

/// The longest reference of a payment, in bytes.
const LONGEST_REFERENCE: usize = 256;

/// How much of an account a caller sees: the whole of it, or their own spending alone; and
/// whether they may set its limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Seen {
    whole: bool,
    limit: bool,
}

impl Seen {
    /// What `role` sees: an Owner or an Admin, the whole account; an Owner sets its limit.
    fn by(role: OrgRole) -> Self {
        Self {
            whole: matches!(role, OrgRole::Owner | OrgRole::Admin),
            limit: role == OrgRole::Owner,
        }
    }
}

/// `account` as a caller who is `you` sees it, each person by the name `named` gives them.
fn shown(
    account: &Account,
    seen: Seen,
    you: Option<PrincipalId>,
    named: impl Fn(PrincipalId) -> Option<String>,
) -> CreditsInfo {
    let mut spent: Vec<(PrincipalId, Money)> = account
        .people
        .iter()
        .filter(|(person, _)| seen.whole || Some(**person) == you)
        .map(|(person, spent)| (*person, *spent))
        .collect();
    if let Some(you) = you.filter(|_| spent.is_empty() && !seen.whole) {
        let nothing = Money {
            micros: 0,
            currency: Currency::USD,
        };
        spent.push((you, nothing));
    }
    spent.sort_by_key(|(_, spent)| Reverse(spent.micros));
    let people = spent
        .into_iter()
        .map(|(person, spent)| PersonSpent {
            person,
            name: named(person),
            spent,
            you: Some(person) == you,
        })
        .collect();
    CreditsInfo {
        account: seen.whole.then_some(CreditsAccount {
            balance: account.balance,
            limit: account.limit,
            spent: account.spent,
        }),
        people,
        since: At::from(account.since),
        can_set_limit: seen.limit,
    }
}

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// The credits the server keeps, refused where its configuration keeps none.
    fn credits(&self) -> Result<&Credits<L>, ApiError> {
        self.inner
            .credits
            .as_deref()
            .ok_or_else(|| ApiError::new(code::NOT_AVAILABLE, Sentence::NoCredits))
    }

    /// `account` as `caller` sees it, who `seen` says may see what, each person by the name
    /// `people`, their organization's, knows them by.
    fn credits_seen(
        &self,
        caller: &Caller,
        people: &People,
        account: &Account,
        seen: Seen,
    ) -> CreditsInfo {
        let you = self.person_of(caller).ok();
        shown(account, seen, you, |person| people.known_as(person))
    }

    /// `account` of the caller's own organization `org` as they see it.
    async fn own_seen(
        &self,
        caller: &Caller,
        org: OrgId,
        account: &Account,
        seen: Seen,
    ) -> Result<CreditsInfo, ApiError> {
        self.catch_up_people(org).await?;
        Ok(self.people(org, |people| {
            self.credits_seen(caller, people, account, seen)
        }))
    }

    /// Answers `credits/get`.
    pub(crate) async fn get_credits(&self, caller: &Caller) -> Result<Reply, ApiError> {
        let credits = self.credits()?;
        let seen = Seen::by(self.role_of(caller).await?);
        let org = self.org_of(caller);
        let account = credits.account(org).await?;
        let credits = self.own_seen(caller, org, &account, seen).await?;
        Ok(Reply::Credits { credits })
    }

    /// Answers `credits/limit`, once the one check found the caller an Owner.
    pub(crate) async fn set_credits_limit(
        &self,
        caller: &Caller,
        limit: Money,
    ) -> Result<Reply, ApiError> {
        let credits = self.credits()?;
        let by = Principal::person(self.person_of(caller)?);
        let org = self.org_of(caller);
        let account = credits.set_limit(org, limit, by).await?;
        let seen = Seen::by(OrgRole::Owner);
        let credits = self.own_seen(caller, org, &account, seen).await?;
        Ok(Reply::Credits { credits })
    }

    /// Answers `credits/add`: only for whoever runs the server, the person on its computer,
    /// into an organization it serves; answered with the whole account, its limit theirs to
    /// set only in their own.
    pub(crate) async fn add_credits(
        &self,
        caller: &Caller,
        amount: Money,
        reference: &str,
        organization: Option<OrgId>,
    ) -> Result<Reply, ApiError> {
        let credits = self.credits()?;
        if !local(caller) {
            return Err(ApiError::new(
                code::NOT_ALLOWED,
                Sentence::OperatorAddsCredits,
            ));
        }
        let reference = reference.trim();
        if reference.is_empty() || reference.len() > LONGEST_REFERENCE {
            let most = LONGEST_REFERENCE;
            return Err(ApiError::new(
                code::INVALID_PARAMS,
                Sentence::PaymentReference { most },
            ));
        }
        let own = self.org_of(caller);
        let org = organization.unwrap_or(own);
        // Another organization's people are read apart, so one the operator names crowds out
        // none this server keeps.
        let people = if org == own {
            None
        } else {
            Some(self.people_apart(org).await?)
        };
        let unknown = people.as_ref().is_some_and(|people| people.count() == 0);
        if org != self.inner.own && unknown {
            let org = org.to_string();
            return Err(ApiError::new(
                code::NO_ORGANIZATION,
                Sentence::NoSuchOrganization { org },
            ));
        }
        let by = Principal::person(self.person_of(caller)?);
        let account = credits.add(org, amount, reference, by).await?;
        let seen = Seen {
            whole: true,
            limit: org == own,
        };
        let credits = match &people {
            Some(people) => self.credits_seen(caller, people, &account, seen),
            None => self.own_seen(caller, org, &account, seen).await?,
        };
        Ok(Reply::Credits { credits })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use jiff::Timestamp;

    use super::*;

    fn money(micros: i64) -> Money {
        Money {
            micros,
            currency: Currency::USD,
        }
    }

    fn person(n: u8) -> PrincipalId {
        PrincipalId::new(Timestamp::from_second(i64::from(n)).unwrap(), [n; 10]).unwrap()
    }

    fn account() -> Account {
        Account {
            balance: money(4_000_000),
            limit: Some(money(10_000_000)),
            spent: money(6_000_000),
            people: BTreeMap::from([(person(1), money(1_000_000)), (person(2), money(5_000_000))]),
            since: Timestamp::from_second(1_790_812_800).unwrap(),
        }
    }

    #[test]
    fn owners_and_admins_see_the_whole_account_and_anyone_else_their_own_spending() {
        let named = |person| (person == self::person(2)).then(|| "Bo".to_owned());
        let admin = shown(&account(), Seen::by(OrgRole::Admin), Some(person(1)), named);
        let whole = admin.account.unwrap();
        assert_eq!(whole.balance, money(4_000_000));
        assert_eq!(whole.limit, Some(money(10_000_000)));
        let spent: Vec<(PrincipalId, bool, Option<&str>)> = admin
            .people
            .iter()
            .map(|p| (p.person, p.you, p.name.as_deref()))
            .collect();
        let most_first = [(person(2), false, Some("Bo")), (person(1), true, None)];
        assert_eq!(spent, most_first, "most first, by name");
        assert!(!admin.can_set_limit, "only an Owner sets the limit");
        let owner = shown(&account(), Seen::by(OrgRole::Owner), None, |_| None);
        assert!(owner.can_set_limit);
        let unnamed = |_| None;
        let member = shown(
            &account(),
            Seen::by(OrgRole::Member),
            Some(person(1)),
            unnamed,
        );
        assert_eq!(member.account, None);
        assert_eq!(member.people.len(), 1);
        assert_eq!(member.people[0].spent, money(1_000_000));
        let newcomer = shown(
            &account(),
            Seen::by(OrgRole::Member),
            Some(person(3)),
            unnamed,
        );
        assert_eq!(
            (newcomer.people[0].person, newcomer.people[0].spent),
            (person(3), money(0)),
            "someone who spent nothing is told so"
        );
    }
}
