//! Projects with writers racing each other, as two processes on one store race: renames to one
//! name, people's first personal project, and new projects under one name.

use glave_event::Principal;

use super::super::racing::{WRITERS, together};
use super::super::{Verdict, ensure};
use super::{failed, record};
use crate::projects::{Project, ProjectError, ProjectKind, Projects};
use crate::scope::OrgId;

/// The racing cases, in `org`, a fresh organization, for `owner`.
pub(super) async fn racing(store: &dyn Projects, org: OrgId, owner: &Principal) -> Verdict {
    renames(store, org, owner).await?;
    personal(store, org, owner).await?;
    same_name(store, org, owner).await
}

/// One owner's projects renamed to one name at once: one takes it and every other is refused
/// as taken, so no two of an owner's projects ever share a name.
async fn renames(store: &dyn Projects, org: OrgId, owner: &Principal) -> Verdict {
    let mut made = Vec::new();
    for writer in 0..WRITERS {
        let new = record(40 + writer, ProjectKind::Project, owner)?;
        made.push(store.create(org, &new).await.map_err(failed)?);
    }
    let renames = made
        .iter()
        .map(|project| store.rename(org, project.id, "raced", project.updated_at));
    let results = together(renames.collect()).await;
    let kept = results.iter().filter(|result| result.is_ok()).count();
    ensure(
        kept == 1,
        &format!("exactly one of renames to one name must take it, not {kept}"),
    )?;
    ensure(
        results
            .iter()
            .all(|result| result.is_ok() || matches!(result, Err(ProjectError::Conflict(_)))),
        "every other rename to the name must be refused as taken",
    )?;
    let listed = store.list(org, owner, 100).await.map_err(failed)?;
    let named = listed
        .iter()
        .filter(|project| Project::same_name(&project.name, "raced"))
        .count();
    ensure(
        named == 1,
        &format!("the owner must have one project called that, not {named}"),
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
