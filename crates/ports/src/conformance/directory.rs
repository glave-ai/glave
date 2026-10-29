//! What every store of the directory must do: link an identity to one person for good, find
//! it again, keep identities apart, and keep each person's organizations in the order they
//! joined, once each, apart from anyone else's.

use glave_event::PrincipalId;

use super::racing::{WRITERS, together};
use super::{Verdict, ensure, uuid_v7};
use crate::directory::{Directory, IdentityKey};
use crate::scope::OrgId;

fn key(n: u8) -> Result<IdentityKey, String> {
    let hex: String = std::iter::repeat_n(format!("{n:02x}"), 32).collect();
    IdentityKey::new(&hex).map_err(|e| e.to_string())
}

fn person(n: u64) -> Result<PrincipalId, String> {
    PrincipalId::try_from(uuid_v7(7_000 + n)).map_err(|e| e.to_string())
}

fn org(n: u64) -> OrgId {
    OrgId::from_uuid(uuid_v7(8_000 + n))
}

/// Every [`Directory`] must link identities once and keep memberships in order.
pub async fn directory<D: Directory + ?Sized>(directory: &D) -> Verdict {
    linking(directory).await?;
    joining(directory).await?;
    racing(directory).await
}

async fn linking<D: Directory + ?Sized>(directory: &D) -> Verdict {
    let (ada, alan) = (person(1)?, person(2)?);
    let (one, two) = (key(1)?, key(2)?);
    let found = directory.find(&one).await.map_err(|e| e.to_string())?;
    ensure(found.is_none(), "an identity no one linked leads to no one")?;
    let linked = directory.link(&one, ada).await.map_err(|e| e.to_string())?;
    ensure(linked == ada, "a new link leads to its person")?;
    let again = directory
        .link(&one, alan)
        .await
        .map_err(|e| e.to_string())?;
    ensure(again == ada, "an identity linked already keeps its person")?;
    let found = directory.find(&one).await.map_err(|e| e.to_string())?;
    ensure(found == Some(ada), "a linked identity is found")?;
    let other = directory.find(&two).await.map_err(|e| e.to_string())?;
    ensure(other.is_none(), "another identity leads to no one")?;
    ensure(
        IdentityKey::new("not a digest").is_err() && IdentityKey::new(&"A".repeat(64)).is_err(),
        "a key is a SHA-256 in lowercase hex",
    )
}

async fn joining<D: Directory + ?Sized>(directory: &D) -> Verdict {
    let (grace, linus) = (person(3)?, person(4)?);
    let none = directory
        .organizations(grace)
        .await
        .map_err(|e| e.to_string())?;
    ensure(
        none.is_empty(),
        "someone who joined nothing belongs nowhere",
    )?;
    // Joined out of their ids' order, so a store that sorts by id is caught.
    for joined in [org(2), org(1), org(2)] {
        directory
            .join(grace, joined)
            .await
            .map_err(|e| e.to_string())?;
    }
    directory
        .join(linus, org(3))
        .await
        .map_err(|e| e.to_string())?;
    let orgs = directory
        .organizations(grace)
        .await
        .map_err(|e| e.to_string())?;
    ensure(
        orgs == vec![org(2), org(1)],
        "a person's organizations are theirs, once each, the first joined first",
    )?;
    for (from, place) in [(grace, org(2)), (grace, org(9))] {
        directory
            .leave(from, place)
            .await
            .map_err(|e| e.to_string())?;
    }
    let left = directory
        .organizations(grace)
        .await
        .map_err(|e| e.to_string())?;
    ensure(
        left == vec![org(1)],
        "leaving takes one organization out, and no other",
    )?;
    directory
        .join(grace, org(2))
        .await
        .map_err(|e| e.to_string())?;
    let back = directory
        .organizations(grace)
        .await
        .map_err(|e| e.to_string())?;
    ensure(
        back == vec![org(1), org(2)],
        "joining again comes after the rest",
    )?;
    let theirs = directory
        .organizations(linus)
        .await
        .map_err(|e| e.to_string())?;
    ensure(
        theirs == vec![org(3)],
        "another person's organizations are theirs alone",
    )
}

/// Of people racing to link one identity, one wins and all are told the same.
async fn racing<D: Directory + ?Sized>(directory: &D) -> Verdict {
    let identity = key(9)?;
    let people: Vec<PrincipalId> = (0..WRITERS)
        .map(|n| person(100 + n))
        .collect::<Result<_, _>>()?;
    let calls = people
        .iter()
        .map(|person| directory.link(&identity, *person));
    let linked = together(calls.collect()).await;
    let linked: Vec<PrincipalId> = linked
        .into_iter()
        .collect::<Result<_, _>>()
        .map_err(|e| e.to_string())?;
    let winner = linked.first().copied();
    ensure(
        linked.iter().all(|person| Some(*person) == winner),
        "racing links of one identity all lead to one person",
    )?;
    let found = directory.find(&identity).await.map_err(|e| e.to_string())?;
    ensure(found == winner, "and it is the one found")
}
