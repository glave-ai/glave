//! What every [`Projects`] must do: keep whole records apart by organization, refuse a taken id
//! or a second personal project, keep each owner's project names apart, list a person's projects
//! in id order, change only what may change and never lose an update to another, and lead every
//! folder key to one project, found without making one; and all of it with writers racing each other.

use glave_event::{At, Principal, PrincipalId};

use super::{Verdict, ensure, uuid_v7};

mod racing;
use crate::projects::{FolderKey, Project, ProjectError, ProjectKind, Projects};
use crate::scope::{OrgId, ProjectId};

fn person(n: u64) -> Result<Principal, String> {
    PrincipalId::try_from(uuid_v7(4_000 + n))
        .map(Principal::person)
        .map_err(|e| e.to_string())
}

fn at(text: &str) -> Result<At, String> {
    At::try_from(text.to_owned()).map_err(|e| e.to_string())
}

pub(super) fn record(n: u64, kind: ProjectKind, owner: &Principal) -> Result<Project, String> {
    let created = at("2026-10-11T09:00:00.000Z")?;
    Ok(Project {
        id: ProjectId::from_uuid(uuid_v7(5_000 + n)),
        kind,
        name: format!("project {n}"),
        owner: owner.clone(),
        members: Vec::new(),
        created_at: created,
        updated_at: created,
        revision: 0,
    })
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "taken by value to pass straight to map_err"
)]
pub(super) fn failed(error: ProjectError) -> String {
    error.to_string()
}

/// The projects suite.
pub async fn projects(store: &dyn Projects) -> Verdict {
    let org = OrgId::from_uuid(uuid_v7(1_101));
    let other_org = OrgId::from_uuid(uuid_v7(1_102));
    let (ada, grace, linus) = (person(1)?, person(2)?, person(3)?);

    let mut project = record(1, ProjectKind::Project, &ada)?;
    project.members = vec![grace.clone()];
    ensure(
        store.get(org, project.id).await.map_err(failed)?.is_none(),
        "a project never kept must read as none",
    )?;
    ensure(
        store.create(org, &project).await.map_err(failed)? == project,
        "a project with a name of its own is kept as it is",
    )?;
    ensure(
        store.get(org, project.id).await.map_err(failed)? == Some(project.clone()),
        "a project must read back exactly as kept",
    )?;
    ensure(
        matches!(
            store.create(org, &project).await,
            Err(ProjectError::Conflict(_))
        ),
        "a taken id must be refused",
    )?;
    let mut nameless = record(2, ProjectKind::Project, &ada)?;
    nameless.name = "  ".into();
    ensure(
        matches!(
            store.create(org, &nameless).await,
            Err(ProjectError::Invalid(said)) if said.starts_with("a project's name")
        ),
        "a name of only spaces must be refused, saying what a project's name may be",
    )?;
    ensure(
        store
            .get(other_org, project.id)
            .await
            .map_err(failed)?
            .is_none(),
        "another organization must not see the project",
    )?;
    listing(store, org, other_org, &project, [&ada, &grace, &linus]).await?;
    updating(store, org, &project, &linus).await?;
    personal(store, org, other_org, &ada).await?;
    folders(store, org, other_org, &ada).await?;
    names(store, org, [&ada, &grace]).await?;
    racing::racing(store, OrgId::from_uuid(uuid_v7(1_103)), &linus).await
}

/// One owner's projects never share a name, ignoring case: a new one takes the next free
/// name, a rename to a taken one is refused. Another owner's may share it.
async fn names(store: &dyn Projects, org: OrgId, [ada, grace]: [&Principal; 2]) -> Verdict {
    let named = |n: u64, name: &str, owner: &Principal| {
        record(n, ProjectKind::Project, owner).map(|mut made| {
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
            Err(ProjectError::Conflict(_))
        ),
        "renaming a project to a name its owner has taken must be refused",
    )?;
    let mut own = again.clone();
    own.name = "web 2".into();
    let kept = store.update(org, &own).await.map_err(failed)?;
    ensure(
        kept.name == own.name && store.get(org, again.id).await.map_err(failed)? == Some(kept),
        "a project may take its own name in another case",
    )
}

async fn listing(
    store: &dyn Projects,
    org: OrgId,
    other_org: OrgId,
    project: &Project,
    [ada, grace, linus]: [&Principal; 3],
) -> Verdict {
    let later = record(3, ProjectKind::Project, ada)?;
    store.create(org, &later).await.map_err(failed)?;
    let ids = |projects: Vec<Project>| projects.into_iter().map(|s| s.id).collect::<Vec<_>>();
    ensure(
        ids(store.list(org, ada, 10).await.map_err(failed)?) == [project.id, later.id],
        "the owner's projects must list in id order",
    )?;
    ensure(
        ids(store.list(org, ada, 1).await.map_err(failed)?) == [project.id],
        "a listing must keep to its limit",
    )?;
    ensure(
        ids(store.list(org, grace, 10).await.map_err(failed)?) == [project.id],
        "a member must see the projects they are a member of",
    )?;
    ensure(
        store.list(org, linus, 10).await.map_err(failed)?.is_empty(),
        "someone neither owner nor member must see no project",
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

async fn updating(
    store: &dyn Projects,
    org: OrgId,
    project: &Project,
    linus: &Principal,
) -> Verdict {
    let mut changed = project.clone();
    changed.name = "renamed".into();
    changed.members = vec![linus.clone(), project.owner.clone()];
    changed.updated_at = at("2026-10-12T09:00:00.000Z")?;
    let mut tampered = changed.clone();
    tampered.kind = ProjectKind::Personal;
    tampered.owner = linus.clone();
    tampered.created_at = changed.updated_at;
    let kept = store.update(org, &tampered).await.map_err(failed)?;
    changed.revision = 1;
    ensure(
        kept == changed && store.get(org, project.id).await.map_err(failed)? == Some(kept),
        "an update must change the name, members and updated_at, count a revision, and nothing else",
    )?;
    ensure(
        matches!(
            store.update(org, &tampered).await,
            Err(ProjectError::Changed(_))
        ),
        "an update naming a revision another update came after must be refused",
    )?;
    ensure(
        store
            .list(org, linus, 10)
            .await
            .map_err(failed)?
            .iter()
            .any(|listed| listed.id == project.id),
        "a member added by an update must see the project",
    )?;
    let missing = record(9, ProjectKind::Project, linus)?;
    ensure(
        matches!(
            store.update(org, &missing).await,
            Err(ProjectError::NotFound(_))
        ),
        "updating a project never kept must be refused",
    )
}

async fn personal(store: &dyn Projects, org: OrgId, other_org: OrgId, ada: &Principal) -> Verdict {
    let first = record(10, ProjectKind::Personal, ada)?;
    let kept = store.personal(org, &first).await.map_err(failed)?;
    ensure(
        kept == first,
        "a person's first personal project must be kept",
    )?;
    let second = record(11, ProjectKind::Personal, ada)?;
    ensure(
        store.personal(org, &second).await.map_err(failed)? == first,
        "a person must keep exactly one personal project",
    )?;
    ensure(
        matches!(
            store.create(org, &second).await,
            Err(ProjectError::Conflict(_))
        ),
        "a second personal project must be refused",
    )?;
    ensure(
        store.personal(other_org, &second).await.map_err(failed)? == second,
        "each organization must give a person a personal project of its own",
    )
}

async fn folders(store: &dyn Projects, org: OrgId, other_org: OrgId, ada: &Principal) -> Verdict {
    let path = |p: &str| FolderKey::Path(p.into());
    let remote = FolderKey::Remote("git@example.com:glave/app.git".into());
    let first = record(20, ProjectKind::Project, ada)?;
    let unused = record(21, ProjectKind::Project, ada)?;
    let looked = store
        .find_folder(org, &[path("/code/app"), remote.clone()])
        .await
        .map_err(failed)?;
    ensure(looked.is_none(), "a folder no key leads to must find none")?;
    let found = store
        .folder(org, &[path("/code/app"), remote.clone()], &first)
        .await
        .map_err(failed)?;
    ensure(
        found == first,
        "a folder no key leads to must get the new project",
    )?;
    let looked = store
        .find_folder(org, &[path("/nowhere/app"), remote.clone()])
        .await
        .map_err(failed)?;
    ensure(
        looked.as_ref() == Some(&first),
        "finding a folder must find the project its first known key leads to",
    )?;
    ensure(
        store
            .find_folder(org, &[path("/nowhere/app")])
            .await
            .map_err(failed)?
            .is_none(),
        "finding a folder must make no key lead anywhere",
    )?;
    ensure(
        store
            .find_folder(other_org, &[path("/code/app")])
            .await
            .map_err(failed)?
            .is_none(),
        "finding a folder must not lead across organizations",
    )?;
    let clone = store
        .folder(org, &[path("/elsewhere/app"), remote.clone()], &unused)
        .await
        .map_err(failed)?;
    ensure(
        clone.id == first.id,
        "another checkout of the same remote must find the same project",
    )?;
    let moved = store
        .folder(org, &[path("/elsewhere/app")], &unused)
        .await
        .map_err(failed)?;
    ensure(
        moved.id == first.id,
        "a path met through its remote must lead to the project from then on",
    )?;
    ensure(
        store.get(org, unused.id).await.map_err(failed)?.is_none(),
        "a project not needed must not be kept",
    )?;
    let fresh = store
        .folder(org, &[path("/code/other")], &unused)
        .await
        .map_err(failed)?;
    ensure(
        fresh == unused,
        "an unknown folder must get the new project",
    )?;
    let other = record(22, ProjectKind::Project, ada)?;
    let there = store
        .folder(other_org, &[path("/code/app")], &other)
        .await
        .map_err(failed)?;
    ensure(
        there == other,
        "folder keys must not lead across organizations",
    )
}
