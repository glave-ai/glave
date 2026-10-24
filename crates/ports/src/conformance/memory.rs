//! What every [`Memory`] must do about scope, which it enforces itself since the model never
//! names a project: a project's memory is read and changed only from that project, a person's
//! own only by them and never from another organization, and a memory someone may not reach is
//! answered exactly as one that does not exist, so not even its existence leaks. A result kept
//! by reference holds no memory's text, and reads again only what the thread reading it may.

use glave_event::{Arguments, MemoryId, MemoryScope, PrincipalId, ThreadId};

use super::{Verdict, ensure, uuid_v7};
use crate::memories::{Access, Control, Memory, MemoryContext, RECALL, Remembered};
use crate::scope::Scope;

/// Where the memory suite works. The implementation's rules must let memory be shared (the
/// person's and the project's) in each project, and keep `person`'s own memory in the
/// organization of `here` and `next_door`; `abroad` is in another organization.
#[derive(Debug, Clone, Copy)]
pub struct MemoryPlaces {
    pub here: Scope,
    /// Another project of the same organization.
    pub next_door: Scope,
    /// A project of another organization.
    pub abroad: Scope,
    /// The person whose memory it is.
    pub person: PrincipalId,
    /// Someone else in the organization of `here`.
    pub stranger: PrincipalId,
}

fn access(person: PrincipalId, scope: Scope) -> Access {
    Access { person, scope }
}

/// What a control answers, with `id` written as `@memory`.
async fn answered(memory: &dyn Memory, by: &Access, control: Control, id: MemoryId) -> String {
    let shown = match memory.control(by, control).await {
        Ok(memories) => format!("Ok({memories:?})"),
        Err(error) => format!("Err({error:?})"),
    };
    shown.replace(&id.as_uuid().to_string(), "@memory")
}

/// Every control naming `id`, by `by`, must answer as it does for a memory nobody has.
async fn as_if_missing(memory: &dyn Memory, by: &Access, id: MemoryId, who: &str) -> Verdict {
    let nobodys = MemoryId::try_from(uuid_v7(9_901)).map_err(|e| e.to_string())?;
    let controls = |id: MemoryId| {
        [
            Control::Edit {
                memory: id,
                text: "changed".to_owned(),
            },
            Control::Move {
                memory: id,
                scope: MemoryScope::Project,
            },
            Control::Move {
                memory: id,
                scope: MemoryScope::Personal,
            },
            Control::Forget { memory: id },
            Control::Restore { memory: id },
        ]
    };
    for (theirs, missing) in controls(id).into_iter().zip(controls(nobodys)) {
        let what = format!("{theirs:?}");
        let theirs = answered(memory, by, theirs, id).await;
        let missing = answered(memory, by, missing, nobodys).await;
        ensure(
            theirs == missing,
            &format!(
                "{who}: {what} must be answered as for a memory nobody has, not {theirs} \
                 (nobody's: {missing})"
            ),
        )?;
    }
    Ok(())
}

fn ids(listed: &[Remembered]) -> Vec<MemoryId> {
    listed.iter().map(|memory| memory.memory).collect()
}

/// What a thread of `person` in `scope` reads, and what their controls list there.
async fn seen(
    memory: &dyn Memory,
    person: PrincipalId,
    scope: Scope,
) -> Result<Vec<MemoryId>, String> {
    let on = thread_of(person, scope)?;
    let mut seen = ids(&memory.list(&on).await.map_err(|e| e.to_string())?);
    let listed = memory
        .control(&access(person, scope), Control::List { scope: None })
        .await
        .map_err(|e| e.to_string())?;
    seen.extend(ids(&listed));
    Ok(seen)
}

/// Saves one memory of `which` by the person in `here`.
async fn save(
    memory: &dyn Memory,
    at: &MemoryPlaces,
    which: MemoryScope,
) -> Result<Remembered, String> {
    let text = format!("the person's {which:?} note");
    let saved = memory
        .control(
            &access(at.person, at.here),
            Control::Save {
                text: text.clone(),
                scope: which,
            },
        )
        .await
        .map_err(|e| e.to_string())?;
    saved
        .into_iter()
        .find(|memory| memory.text == text)
        .ok_or_else(|| "a saved memory must be answered back".to_owned())
}

/// A thread of `person` in `scope`.
fn thread_of(person: PrincipalId, scope: Scope) -> Result<MemoryContext, String> {
    Ok(MemoryContext {
        person: Some(person),
        agent: None,
        scope,
        thread: ThreadId::try_from(uuid_v7(9_903)).map_err(|e| e.to_string())?,
        seq: None,
        untrusted_read: false,
    })
}

/// A result that showed memories is kept without their text, and read again only as the
/// thread reading it may read: never another project's memory, another organization's, or
/// another person's own, whatever the kept result names.
async fn rereads_only_what_may_be_read(
    memory: &dyn Memory,
    at: &MemoryPlaces,
    [project, personal]: [&Remembered; 2],
) -> Verdict {
    let mut query = serde_json::Map::new();
    query.insert("query".to_owned(), "person note".into());
    let found = (memory.call(
        &thread_of(at.person, at.here)?,
        RECALL,
        &Arguments::Json(query),
    ))
    .await;
    let kept = found
        .kept
        .ok_or("a result that showed memories must be kept by reference")?;
    let both = [project.text.as_str(), personal.text.as_str()];
    ensure(
        both.iter().all(|text| !kept.text.contains(text)),
        "the words a result is kept in must hold no memory's text",
    )?;
    let shown = [kept.shown];
    let read = async |person, scope| -> Result<String, String> {
        let again = memory.reread(&thread_of(person, scope)?, &shown).await;
        Ok(again.unwrap_or_default().into_iter().flatten().collect())
    };
    let here = read(at.person, at.here).await?;
    ensure(
        both.iter().all(|text| here.contains(text)),
        "a result read again where it was shown must show what it showed",
    )?;
    for (who, person, scope, theirs) in [
        ("from another project", at.person, at.next_door, &both[..1]),
        ("from another organization", at.person, at.abroad, &both[..]),
        ("by someone else", at.stranger, at.here, &both[1..]),
    ] {
        let again = read(person, scope).await?;
        ensure(
            theirs.iter().all(|text| !again.contains(text)),
            &format!("a result read again {who} must not show what they may not read"),
        )?;
    }
    Ok(())
}

/// The memory scope suite.
pub async fn memory_scopes(memory: &dyn Memory, at: MemoryPlaces) -> Verdict {
    let project = save(memory, &at, MemoryScope::Project).await?;
    let personal = save(memory, &at, MemoryScope::Personal).await?;
    rereads_only_what_may_be_read(memory, &at, [&project, &personal]).await?;
    let (p, q) = (project.memory, personal.memory);

    let next_door = seen(memory, at.person, at.next_door).await?;
    ensure(
        !next_door.contains(&p),
        "a project's memory must not be read from another project",
    )?;
    let abroad = seen(memory, at.person, at.abroad).await?;
    ensure(
        !abroad.contains(&p) && !abroad.contains(&q),
        "no memory may be read from another organization",
    )?;
    let stranger = seen(memory, at.stranger, at.here).await?;
    ensure(
        !stranger.contains(&q),
        "a person's own memory must not be read by anyone else",
    )?;

    let from_next_door = access(at.person, at.next_door);
    as_if_missing(memory, &from_next_door, p, "from another project").await?;
    let from_abroad = access(at.person, at.abroad);
    as_if_missing(memory, &from_abroad, p, "from another organization").await?;
    as_if_missing(memory, &from_abroad, q, "from another organization").await?;
    let by_stranger = access(at.stranger, at.here);
    as_if_missing(memory, &by_stranger, q, "by someone else").await?;

    let after = memory
        .control(&access(at.person, at.here), Control::List { scope: None })
        .await
        .map_err(|e| e.to_string())?;
    ensure(
        after.contains(&project) && after.contains(&personal),
        "what others tried must change none of the person's memories",
    )
}
