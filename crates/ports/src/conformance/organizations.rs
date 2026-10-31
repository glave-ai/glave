//! Many organizations in one store (`specs/storage.md`, "Many organizations in one store"): two
//! organizations that happen to use the same project id and the same thread id never see each
//! other's threads, events, search results, threads view, blobs, projects or own streams, and
//! neither one's writes touch the other's. Ids are never shared on purpose; using the same ones
//! here proves that every row is told apart by its organization, not by the luck of fresh ids.

use glave_event::{Role, ThreadId};

use super::{Verdict, ensure, events, seqs, thread, uuid_v7};
use crate::blob::{BlobStore, Pieces};
use crate::log::EventLog;
use crate::projects::{FolderKey, ProjectKind, Projects};
use crate::scope::{Located, OrgId, ProjectId, Scope};
use crate::stream::Reserved;
use crate::view::{Standing, ThreadQuery};

/// The two organizations, and the one project id both use.
fn places() -> (Scope, Scope) {
    let project = ProjectId::from_uuid(uuid_v7(2_900));
    (
        Scope::glaves_own(OrgId::from_uuid(uuid_v7(1_901)), project),
        Scope::glaves_own(OrgId::from_uuid(uuid_v7(1_902)), project),
    )
}

/// The suite, on one store's log, blobs and projects. The store must be empty.
pub async fn organizations<L: EventLog, B: BlobStore>(
    log: &L,
    blobs: &B,
    projects: &dyn Projects,
) -> Verdict {
    let (ours, theirs) = places();
    let t = thread(900)?;
    threads(log, ours, theirs, t).await?;
    own_streams(log, ours, theirs).await?;
    kept_blobs(blobs, ours, theirs, t).await?;
    kept_projects(projects, ours, theirs).await
}

/// What one organization writes to a thread is nowhere in the other's, which may keep a thread
/// of the same id in a project of the same id, numbered on its own.
async fn threads<L: EventLog>(log: &L, ours: Scope, theirs: Scope, t: ThreadId) -> Verdict {
    let started = super::threads::Writer::new(t).next(1, &super::threads::started()?)?;
    let mut written = vec![started];
    written.push(super::search::said(
        t,
        2,
        Role::User,
        "Plant the marigolds by the gate.",
    )?);
    log.append(&ours, &written)
        .await
        .map_err(|e| e.to_string())?;
    unseen(log, ours, theirs, t).await?;
    let view = log.thread_view(&ours, t).await.map_err(|e| e.to_string())?;
    ensure(view.is_some(), "a thread started must have a view")?;
    let read = |scope: Scope| async move { log.read(&scope, t, None, usize::MAX).await };
    log.append(&theirs, &events(t, &[1])?).await.map_err(|e| {
        format!("another organization's thread of the same id must start at 1: {e}")
    })?;
    ensure(
        seqs(&read(ours).await.map_err(|e| e.to_string())?) == [1, 2],
        "another organization's writes must not touch a thread",
    )?;
    ensure(
        log.thread_view(&ours, t).await.map_err(|e| e.to_string())? == view,
        "another organization's writes must not touch a thread's view",
    )?;
    let found = log
        .search(&ours, "marigold", 10)
        .await
        .map_err(|e| e.to_string())?;
    ensure(
        found.len() == 1,
        "an organization must still find its own words",
    )
}

/// Nothing of a thread `ours` wrote is anywhere in `theirs`: not its events, where it is, its
/// words or its view.
async fn unseen<L: EventLog>(log: &L, ours: Scope, theirs: Scope, t: ThreadId) -> Verdict {
    let read = log
        .read(&theirs, t, None, usize::MAX)
        .await
        .map_err(|e| e.to_string())?;
    ensure(
        read.is_empty(),
        "another organization must not read a thread of the same id",
    )?;
    ensure(
        log.locate(theirs.org(), t)
            .await
            .map_err(|e| e.to_string())?
            .is_none(),
        "locate must not find another organization's thread",
    )?;
    ensure(
        log.locate(ours.org(), t).await.map_err(|e| e.to_string())?
            == Some(Located {
                org: ours.org(),
                project: ours.project(),
            }),
        "locate must find a thread in its own organization",
    )?;
    ensure(
        log.search(&theirs, "marigold", 10)
            .await
            .map_err(|e| e.to_string())?
            .is_empty(),
        "search must not find another organization's words",
    )?;
    ensure(
        log.latest_of(&theirs, t, &["message.added"])
            .await
            .map_err(|e| e.to_string())?
            .is_none(),
        "latest_of must not find another organization's events",
    )?;
    let query = ThreadQuery {
        standing: Standing::Active,
        limit: 10,
        after: None,
        open: false,
        parent: None,
    };
    let views = log
        .thread_views(&theirs, &query)
        .await
        .map_err(|e| e.to_string())?;
    let view = log
        .thread_view(&theirs, t)
        .await
        .map_err(|e| e.to_string())?;
    ensure(
        views.is_empty() && view.is_none(),
        "another organization must not see a thread's view",
    )
}

/// An organization's own streams (its people ledger) are its alone.
async fn own_streams<L: EventLog>(log: &L, ours: Scope, theirs: Scope) -> Verdict {
    let people = Reserved::People.id();
    log.append(&Scope::of_organization(ours.org()), &events(people, &[1])?)
        .await
        .map_err(|e| e.to_string())?;
    let unseen = log
        .read(
            &Scope::of_organization(theirs.org()),
            people,
            None,
            usize::MAX,
        )
        .await
        .map_err(|e| e.to_string())?;
    ensure(
        unseen.is_empty(),
        "another organization must not read an organization's people ledger",
    )
}

/// The same content kept by both is two blobs: neither reads the other's, and one's release
/// leaves the other's in place through both organizations' sweeps.
async fn kept_blobs<B: BlobStore>(blobs: &B, ours: Scope, theirs: Scope, t: ThreadId) -> Verdict {
    let bytes = b"the gate's plan".to_vec();
    let reference = super::blobs::put(blobs, &ours, t, Pieces::from(bytes.clone())).await?;
    ensure(
        super::blobs::read(blobs, &theirs, &reference.sha256)
            .await?
            .is_none(),
        "another organization must not read a blob of the same content",
    )?;
    super::blobs::put(blobs, &theirs, t, Pieces::from(bytes.clone())).await?;
    blobs.release(&ours, t).await.map_err(|e| e.to_string())?;
    for org in [ours.org(), theirs.org()] {
        blobs.sweep(org).await.map_err(|e| e.to_string())?;
    }
    ensure(
        super::blobs::read(blobs, &ours, &reference.sha256)
            .await?
            .is_none(),
        "a blob released and swept must be gone",
    )?;
    ensure(
        super::blobs::read(blobs, &theirs, &reference.sha256).await?
            == Some((reference.clone(), bytes)),
        "one organization's release and sweep must leave another's blob in place",
    )
}

/// A project id and a folder kept by both are each organization's own: a rename in one leaves
/// the other's as it was, and a folder leads each to its own project.
async fn kept_projects(projects: &dyn Projects, ours: Scope, theirs: Scope) -> Verdict {
    let owner = super::threads::person()?;
    let mut project = super::projects::record(900, ProjectKind::Project, &owner)?;
    project.id = ours.project();
    let failed = |e: crate::projects::ProjectError| e.to_string();
    projects
        .create(ours.org(), &project)
        .await
        .map_err(failed)?;
    projects
        .create(theirs.org(), &project)
        .await
        .map_err(|e| format!("another organization may keep a project of the same id: {e}"))?;
    let at = project.created_at;
    projects
        .rename(ours.org(), project.id, "garden", at)
        .await
        .map_err(failed)?;
    ensure(
        projects
            .get(theirs.org(), project.id)
            .await
            .map_err(failed)?
            == Some(project.clone()),
        "a rename in one organization must leave another's project of the same id as it was",
    )?;
    let folder = [FolderKey::Path("/code/garden".into())];
    let (first, second) = (
        super::projects::record(901, ProjectKind::Project, &owner)?,
        super::projects::record(902, ProjectKind::Project, &owner)?,
    );
    let mine = projects
        .folder(ours.org(), &folder, &first)
        .await
        .map_err(failed)?;
    let yours = projects
        .folder(theirs.org(), &folder, &second)
        .await
        .map_err(failed)?;
    ensure(
        mine.id == first.id && yours.id == second.id,
        "the same folder must lead each organization to its own project",
    )
}
