//! Projects with writers racing each other, as two processes on one store race: updates from
//! one read, people's first personal project, and new projects under one name.

use glave_event::Principal;

use super::super::racing::{WRITERS, together};
use super::super::{Verdict, ensure};
use super::{failed, record};
use crate::projects::{Project, ProjectError, ProjectKind, Projects};
use crate::scope::OrgId;

/// The racing cases, in `org`, a fresh organization, for `owner`.
pub(super) async fn racing(store: &dyn Projects, org: OrgId, owner: &Principal) -> Verdict {
    updates(store, org, owner).await?;
    personal(store, org, owner).await?;
    same_name(store, org, owner).await
}

/// Updates made from one read: exactly one is kept and every other is told the project
/// changed, so none is lost without its writer knowing.
async fn updates(store: &dyn Projects, org: OrgId, owner: &Principal) -> Verdict {
    let read = store
        .create(org, &record(40, ProjectKind::Project, owner)?)
        .await
        .map_err(failed)?;
    let writers = (0..WRITERS).map(|writer| {
        let mut changed = read.clone();
        changed.name = format!("renamed by {writer}");
        async move { store.update(org, &changed).await }
    });
    let results = together(writers.collect()).await;
    let kept: Vec<&Project> = results
        .iter()
        .filter_map(|result| result.as_ref().ok())
        .collect();
    ensure(
        kept.len() == 1,
        &format!(
            "exactly one of updates from one read must be kept, not {}",
            kept.len()
        ),
    )?;
    ensure(
        results
            .iter()
            .all(|result| result.is_ok() || matches!(result, Err(ProjectError::Changed(_)))),
        "an update another came before must be told the project changed",
    )?;
    let stored = store.get(org, read.id).await.map_err(failed)?;
    ensure(
        stored.as_ref() == kept.first().copied() && stored.is_some_and(|p| p.revision == 1),
        "the project must be the one update kept, one revision on",
    )
}

/// People asking for their personal project at once all get the same one.
async fn personal(store: &dyn Projects, org: OrgId, owner: &Principal) -> Verdict {
    let mut asks = Vec::new();
    for writer in 0..WRITERS {
        asks.push(record(50 + writer, ProjectKind::Personal, owner)?);
    }
    let results = together(asks.iter().map(|new| store.personal(org, new)).collect()).await;
    let mut ids = Vec::new();
    for result in results {
        ids.push(result.map_err(failed)?.id);
    }
    ids.dedup();
    ensure(
        ids.len() == 1,
        "people asking at once for their personal project must all get the same one",
    )
}

/// New projects of one owner under one name, made at once, each get a name of their own.
async fn same_name(store: &dyn Projects, org: OrgId, owner: &Principal) -> Verdict {
    let mut news = Vec::new();
    for writer in 0..WRITERS {
        let mut new = record(60 + writer, ProjectKind::Project, owner)?;
        new.name = "race".into();
        news.push(new);
    }
    let results = together(news.iter().map(|new| store.create(org, new)).collect()).await;
    let mut names = Vec::new();
    for result in results {
        names.push(result.map_err(failed)?.name.to_lowercase());
    }
    names.sort();
    let mut expected: Vec<String> = (1..=WRITERS)
        .map(|n| match n {
            1 => "race".to_owned(),
            n => format!("race {n}"),
        })
        .collect();
    expected.sort();
    ensure(
        names == expected,
        &format!("projects named at once must each get a free name: {names:?}"),
    )
}
