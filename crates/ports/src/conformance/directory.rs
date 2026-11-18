//! What every store of the directory must do: link an identity to one person, letting a
//! person's emails go only while they belong to no organization, find it again, keep
//! identities apart, kinds too, and keep each person's organizations in the order they joined,
//! once each, apart from anyone else's.

use glave_event::PrincipalId;

use super::racing::{WRITERS, together};
use super::{Verdict, ensure, uuid_v7};
use crate::directory::{Directory, IdentityKey, IdentityKind};
use crate::scope::OrgId;

/// The key of an identity of `kind`, `n` telling it apart from others of its kind.
fn kind_of(kind: IdentityKind, n: u8) -> Result<IdentityKey, String> {
    let hex: String = std::iter::repeat_n(format!("{n:02x}"), 32).collect();
    IdentityKey::new(kind, &hex).map_err(|e| e.to_string())
}

/// An email's key.
fn key(n: u8) -> Result<IdentityKey, String> {
    kind_of(IdentityKind::Email, n)
}

/// A company login's key, of the same digest as [`key`]'s for `n`.
fn login(n: u8) -> Result<IdentityKey, String> {
    kind_of(IdentityKind::CompanyLogin, n)
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
    unlinking(directory).await?;
    joining(directory).await?;
    racing(directory).await
}

/// A person's emails, and no other kind of identity of theirs, are unlinked only while they
/// belong to no organization; then anyone may link them.
async fn unlinking<D: Directory + ?Sized>(directory: &D) -> Verdict {
    let (grace, linus) = (person(5)?, person(6)?);
    let (hers, also, company, his) = (key(5)?, key(6)?, login(5)?, key(7)?);
    let linked = [
        (&hers, grace),
        (&also, grace),
        (&company, grace),
        (&his, linus),
    ];
    for (identity, whose) in linked {
        directory
            .link(identity, whose)
            .await
            .map_err(|e| e.to_string())?;
    }
    directory
        .join(grace, org(5))
        .await
        .map_err(|e| e.to_string())?;
    directory
        .unlink_email_if_alone(grace)
        .await
        .map_err(|e| e.to_string())?;
    let kept = directory.find(&hers).await.map_err(|e| e.to_string())?;
    ensure(
        kept == Some(grace),
        "someone still in an organization keeps their emails",
    )?;
    directory
        .leave(grace, org(5))
        .await
        .map_err(|e| e.to_string())?;
    directory
        .unlink_email_if_alone(grace)
        .await
        .map_err(|e| e.to_string())?;
    for email in [&hers, &also] {
        let freed = directory.find(email).await.map_err(|e| e.to_string())?;
        ensure(
            freed.is_none(),
            "the emails of someone in no organization lead to no one",
        )?;
    }
    let stays = directory.find(&company).await.map_err(|e| e.to_string())?;
    ensure(
        stays == Some(grace),
        "an identity of another kind is never unlinked",
    )?;
    let theirs = directory.find(&his).await.map_err(|e| e.to_string())?;
    ensure(theirs == Some(linus), "another person's email stays theirs")?;
    let relinked = directory
        .link(&hers, linus)
        .await
        .map_err(|e| e.to_string())?;
    ensure(
        relinked == linus,
        "a freed email may be linked to someone else",
    )
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
    let company = directory
        .link(&login(1)?, alan)
        .await
        .map_err(|e| e.to_string())?;
    let still = directory.find(&one).await.map_err(|e| e.to_string())?;
    ensure(
        company == alan && still == Some(ada),
        "one digest of two kinds is two identities",
    )?;
    let email = IdentityKind::Email;
    ensure(
        IdentityKey::new(email, "not a digest").is_err()
            && IdentityKey::new(email, &"A".repeat(64)).is_err(),
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
