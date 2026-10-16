//! What every [`Spaces`] must do: keep whole records apart by organization, refuse a taken id
//! or a second personal space, keep each owner's project names apart, list a person's spaces
//! in id order, change only what may change, and lead every folder key to one space.

use glave_event::{At, Principal, PrincipalId};

use super::{Verdict, ensure, uuid_v7};
use crate::scope::{OrgId, SpaceId};
use crate::spaces::{FolderKey, Space, SpaceError, SpaceKind, Spaces};

fn person(n: u64) -> Result<Principal, String> {
    PrincipalId::try_from(uuid_v7(4_000 + n))
        .map(Principal::person)
        .map_err(|e| e.to_string())
}

fn at(text: &str) -> Result<At, String> {
    At::try_from(text.to_owned()).map_err(|e| e.to_string())
}

fn space(n: u64, kind: SpaceKind, owner: &Principal) -> Result<Space, String> {
    let created = at("2026-10-11T09:00:00.000Z")?;
    Ok(Space {
        id: SpaceId::from_uuid(uuid_v7(5_000 + n)),
        kind,
        name: format!("space {n}"),
        owner: owner.clone(),
        members: Vec::new(),
        created_at: created,
        updated_at: created,
    })
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "taken by value to pass straight to map_err"
)]
fn failed(error: SpaceError) -> String {
    error.to_string()
}

/// The spaces suite.
pub async fn spaces(store: &dyn Spaces) -> Verdict {
    let org = OrgId::from_uuid(uuid_v7(1_101));
    let other_org = OrgId::from_uuid(uuid_v7(1_102));
    let (ada, grace, linus) = (person(1)?, person(2)?, person(3)?);

    let mut project = space(1, SpaceKind::Project, &ada)?;
    project.members = vec![grace.clone()];
    ensure(
        store.get(org, project.id).await.map_err(failed)?.is_none(),
        "a space never kept must read as none",
    )?;
    ensure(
        store.create(org, &project).await.map_err(failed)? == project,
        "a space with a name of its own is kept as it is",
    )?;
    ensure(
        store.get(org, project.id).await.map_err(failed)? == Some(project.clone()),
        "a space must read back exactly as kept",
    )?;
    ensure(
        matches!(
            store.create(org, &project).await,
            Err(SpaceError::Conflict(_))
        ),
        "a taken id must be refused",
    )?;
    let mut nameless = space(2, SpaceKind::Project, &ada)?;
    nameless.name = "  ".into();
    ensure(
        matches!(
            store.create(org, &nameless).await,
            Err(SpaceError::Invalid(_))
        ),
        "a name of only spaces must be refused",
    )?;
    ensure(
        store
            .get(other_org, project.id)
            .await
            .map_err(failed)?
            .is_none(),
        "another organization must not see the space",
    )?;
    listing(store, org, other_org, &project, [&ada, &grace, &linus]).await?;
    updating(store, org, &project, &linus).await?;
    personal(store, org, other_org, &ada).await?;
    folders(store, org, other_org, &ada).await?;
    names(store, org, [&ada, &grace]).await
}

/// One owner's projects never share a name, ignoring case: a new one takes the next free
/// name, a rename to a taken one is refused. Another owner's may share it.
async fn names(store: &dyn Spaces, org: OrgId, [ada, grace]: [&Principal; 2]) -> Verdict {
    let named = |n: u64, name: &str, owner: &Principal| {
        space(n, SpaceKind::Project, owner).map(|mut made| {
            made.name = name.into();
            made
        })
    };
    let web = store
        .create(org, &named(30, "web", ada)?)
        .await
        .map_err(failed)?;
    let again = store
        .create(org, &named(31, "Web", ada)?)
        .await
        .map_err(failed)?;
    let third = store
        .folder(
            org,
            &[FolderKey::Path("/code/web".into())],
            &named(32, "WEB", ada)?,
        )
        .await
        .map_err(failed)?;
    ensure(
        web.name == "web" && again.name == "Web 2" && third.name == "WEB 3",
        &format!(
            "a project named as another of its owner's takes the next free name: {}, {}, {}",
            web.name, again.name, third.name
        ),
    )?;
    ensure(
        store.get(org, again.id).await.map_err(failed)? == Some(again.clone()),
        "the free name is the one kept",
    )?;
    let theirs = store
        .create(org, &named(33, "web", grace)?)
        .await
        .map_err(failed)?;
    ensure(
        theirs.name == "web",
        "another owner's project may have the same name",
    )?;
    let mut taken = again.clone();
    taken.name = "WEB".into();
    ensure(
        matches!(
            store.update(org, &taken).await,
            Err(SpaceError::Conflict(_))
        ),
        "renaming a project to a name its owner has taken must be refused",
    )?;
    let mut own = again.clone();
    own.name = "web 2".into();
    store.update(org, &own).await.map_err(failed)?;
    ensure(
        store.get(org, again.id).await.map_err(failed)? == Some(own),
        "a project may take its own name in another case",
    )
}

async fn listing(
    store: &dyn Spaces,
    org: OrgId,
    other_org: OrgId,
    project: &Space,
    [ada, grace, linus]: [&Principal; 3],
) -> Verdict {
    let later = space(3, SpaceKind::Project, ada)?;
    store.create(org, &later).await.map_err(failed)?;
    let ids = |spaces: Vec<Space>| spaces.into_iter().map(|s| s.id).collect::<Vec<_>>();
    ensure(
        ids(store.list(org, ada, 10).await.map_err(failed)?) == [project.id, later.id],
        "the owner's spaces must list in id order",
    )?;
    ensure(
        ids(store.list(org, ada, 1).await.map_err(failed)?) == [project.id],
        "a listing must keep to its limit",
    )?;
    ensure(
        ids(store.list(org, grace, 10).await.map_err(failed)?) == [project.id],
        "a member must see the spaces they are a member of",
    )?;
    ensure(
        store.list(org, linus, 10).await.map_err(failed)?.is_empty(),
        "someone neither owner nor member must see no space",
    )?;
    ensure(
        store
            .list(other_org, ada, 10)
            .await
            .map_err(failed)?
            .is_empty(),
        "a listing must not cross organizations",
    )
}

async fn updating(store: &dyn Spaces, org: OrgId, project: &Space, linus: &Principal) -> Verdict {
    let mut changed = project.clone();
    changed.name = "renamed".into();
    changed.members = vec![linus.clone(), project.owner.clone()];
    changed.updated_at = at("2026-10-12T09:00:00.000Z")?;
    let mut tampered = changed.clone();
    tampered.kind = SpaceKind::Personal;
    tampered.owner = linus.clone();
    tampered.created_at = changed.updated_at;
    store.update(org, &tampered).await.map_err(failed)?;
    ensure(
        store.get(org, project.id).await.map_err(failed)? == Some(changed.clone()),
        "an update must change the name, members and updated_at, and nothing else",
    )?;
    ensure(
        store
            .list(org, linus, 10)
            .await
            .map_err(failed)?
            .iter()
            .any(|space| space.id == project.id),
        "a member added by an update must see the space",
    )?;
    let missing = space(9, SpaceKind::Project, linus)?;
    ensure(
        matches!(
            store.update(org, &missing).await,
            Err(SpaceError::NotFound(_))
        ),
        "updating a space never kept must be refused",
    )
}

async fn personal(store: &dyn Spaces, org: OrgId, other_org: OrgId, ada: &Principal) -> Verdict {
    let first = space(10, SpaceKind::Personal, ada)?;
    let kept = store.personal(org, &first).await.map_err(failed)?;
    ensure(
        kept == first,
        "a person's first personal space must be kept",
    )?;
    let second = space(11, SpaceKind::Personal, ada)?;
    ensure(
        store.personal(org, &second).await.map_err(failed)? == first,
        "a person must keep exactly one personal space",
    )?;
    ensure(
        matches!(
            store.create(org, &second).await,
            Err(SpaceError::Conflict(_))
        ),
        "a second personal space must be refused",
    )?;
    ensure(
        store.personal(other_org, &second).await.map_err(failed)? == second,
        "each organization must give a person a personal space of its own",
    )
}

async fn folders(store: &dyn Spaces, org: OrgId, other_org: OrgId, ada: &Principal) -> Verdict {
    let path = |p: &str| FolderKey::Path(p.into());
    let remote = FolderKey::Remote("git@example.com:glave/app.git".into());
    let first = space(20, SpaceKind::Project, ada)?;
    let unused = space(21, SpaceKind::Project, ada)?;
    let found = store
        .folder(org, &[path("/code/app"), remote.clone()], &first)
        .await
        .map_err(failed)?;
    ensure(
        found == first,
        "a folder no key leads to must get the new space",
    )?;
    let clone = store
        .folder(org, &[path("/elsewhere/app"), remote.clone()], &unused)
        .await
        .map_err(failed)?;
    ensure(
        clone.id == first.id,
        "another checkout of the same remote must find the same space",
    )?;
    let moved = store
        .folder(org, &[path("/elsewhere/app")], &unused)
        .await
        .map_err(failed)?;
    ensure(
        moved.id == first.id,
        "a path met through its remote must lead to the space from then on",
    )?;
    ensure(
        store.get(org, unused.id).await.map_err(failed)?.is_none(),
        "a space not needed must not be kept",
    )?;
    let fresh = store
        .folder(org, &[path("/code/other")], &unused)
        .await
        .map_err(failed)?;
    ensure(fresh == unused, "an unknown folder must get the new space")?;
    let other = space(22, SpaceKind::Project, ada)?;
    let there = store
        .folder(other_org, &[path("/code/app")], &other)
        .await
        .map_err(failed)?;
    ensure(
        there == other,
        "folder keys must not lead across organizations",
    )
}
