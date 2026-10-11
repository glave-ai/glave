//! The conformance suite for memory servers: what any [`Memory`] must do, glave's own or one a
//! company plugs in (`[memory] server`). It speaks only through the trait, so through MCP with
//! who and where in `_meta`: scope kept apart by the context, never by the model's arguments,
//! the near-duplicate check, the budget refusal, forgetting that leaves nothing to read, and
//! `recall` that finds by keyword only what the thread may read.
//! It returns the first violation it finds.

use glave_event::{Arguments, MemoryId};
use glave_harness::{
    EDIT_MEMORY, FORGET, Memory, MemoryChange, MemoryContext, RECALL, REMEMBER, Shown,
};
use serde_json::json;

type Verdict = Result<(), String>;

/// The threads the suite works from. All are the same person's, in one organization, and each
/// space starts with no memories.
#[derive(Debug, Clone, Copy)]
pub struct Fixture {
    /// A thread in a space whose setting is `shared`.
    pub here: MemoryContext,
    /// A thread in another `shared` space.
    pub elsewhere: MemoryContext,
    /// A thread in an `isolated` space.
    pub isolated: MemoryContext,
}

fn ensure(condition: bool, violation: &str) -> Verdict {
    if condition {
        Ok(())
    } else {
        Err(violation.to_owned())
    }
}

fn arguments(value: &serde_json::Value) -> Arguments {
    Arguments::Json(value.as_object().cloned().unwrap_or_default())
}

/// Saves `text` to `scope` from `on`, returning the memory saved, if one was.
async fn remember(
    memory: &dyn Memory,
    on: &MemoryContext,
    text: &str,
    scope: &str,
    force: bool,
) -> Option<MemoryId> {
    let call = json!({"text": text, "scope": scope, "force": force});
    match memory.call(on, REMEMBER, &arguments(&call)).await.change {
        Some(MemoryChange::Saved { memory, .. }) => Some(memory.memory),
        _ => None,
    }
}

async fn texts(memory: &dyn Memory, on: &MemoryContext) -> Result<Vec<String>, String> {
    let listed = memory.list(on).await.map_err(|e| e.to_string())?;
    Ok(listed.into_iter().map(|m| m.text).collect())
}

/// Runs the whole suite.
pub async fn memory(memory: &dyn Memory, fixture: &Fixture) -> Verdict {
    scopes(memory, fixture).await?;
    duplicates(memory, fixture).await?;
    forgetting(memory, fixture).await?;
    recalling(memory, fixture).await?;
    budget(memory, fixture).await
}

async fn scopes(memory: &dyn Memory, f: &Fixture) -> Verdict {
    remember(
        memory,
        &f.here,
        "This project's tests need make db first",
        "space",
        false,
    )
    .await
    .ok_or("a space memory saves")?;
    remember(
        memory,
        &f.here,
        "Ada prefers short answers without headings",
        "personal",
        false,
    )
    .await
    .ok_or("a personal memory saves in a shared space")?;
    let here = texts(memory, &f.here).await?;
    let elsewhere = texts(memory, &f.elsewhere).await?;
    let isolated = texts(memory, &f.isolated).await?;
    ensure(
        here.len() == 2,
        "a shared space reads its own and the person's",
    )?;
    ensure(
        elsewhere == ["Ada prefers short answers without headings"],
        "another space reads the person's memory, never this space's",
    )?;
    ensure(isolated.is_empty(), "an isolated space reads neither")?;
    let refused = remember(
        memory,
        &f.isolated,
        "The client is BLUEHERON",
        "personal",
        true,
    )
    .await;
    ensure(
        refused.is_none(),
        "an isolated space never writes personal memory, whatever the model asks",
    )?;
    ensure(
        texts(memory, &f.elsewhere).await?.len() == 1,
        "a refused save leaves nothing behind",
    )
}

async fn duplicates(memory: &dyn Memory, f: &Fixture) -> Verdict {
    let first = "Use tabs, not spaces, in this project's Python";
    remember(memory, &f.here, first, "space", false)
        .await
        .ok_or("a new memory saves")?;
    let again = remember(
        memory,
        &f.here,
        "This project's Python uses tabs instead of spaces",
        "space",
        false,
    )
    .await;
    ensure(
        again.is_none(),
        "a near duplicate comes back instead of a save",
    )?;
    let forced = remember(
        memory,
        &f.here,
        "This project's Python uses tabs, not spaces",
        "space",
        true,
    )
    .await;
    ensure(forced.is_some(), "`force` saves despite a near duplicate")
}

async fn forgetting(memory: &dyn Memory, f: &Fixture) -> Verdict {
    let gone = remember(memory, &f.here, "The code word is marigold", "space", false)
        .await
        .ok_or("a memory to forget saves")?;
    let edit = json!({"memory": gone.to_string(), "text": "The code word is marigold, still"});
    let edited = memory.call(&f.here, EDIT_MEMORY, &arguments(&edit)).await;
    ensure(
        matches!(edited.change, Some(MemoryChange::Edited(_))),
        "an edit changes the memory",
    )?;
    let request = json!({"memory": gone.to_string()});
    let forgot = memory.call(&f.here, FORGET, &arguments(&request)).await;
    ensure(
        forgot.change == Some(MemoryChange::Forgotten(gone)),
        "forget forgets",
    )?;
    ensure(
        !texts(memory, &f.here)
            .await?
            .iter()
            .any(|t| t.contains("marigold")),
        "a forgotten memory is no longer listed",
    )?;
    let versions = [1, 2].map(|version| Shown {
        memory: gone,
        version,
    });
    let left = memory
        .versions(&f.here, &versions)
        .await
        .map_err(|e| e.to_string())?;
    ensure(
        left.is_empty(),
        "no version of a forgotten memory can be read",
    )
}

/// What `recall` of `query` from `on` says, or why it failed.
async fn recall(memory: &dyn Memory, on: &MemoryContext, query: &str) -> Result<String, String> {
    let call = json!({"query": query});
    let done = memory.call(on, RECALL, &arguments(&call)).await;
    let text = format!("{:?}", done.output.parts);
    if done.output.is_error {
        Err(text)
    } else {
        Ok(text)
    }
}

async fn recalling(memory: &dyn Memory, f: &Fixture) -> Verdict {
    let found = recall(memory, &f.here, "tests make db").await?;
    ensure(
        found.contains("make db first"),
        "recall finds a memory by keyword",
    )?;
    let elsewhere = recall(memory, &f.elsewhere, "tests make db").await?;
    ensure(
        !elsewhere.contains("make db first"),
        "recall never finds another space's memory",
    )?;
    let isolated = recall(memory, &f.isolated, "short answers headings").await?;
    ensure(
        !isolated.contains("Ada prefers"),
        "recall in an isolated space never finds personal memory",
    )
}

async fn budget(memory: &dyn Memory, f: &Fixture) -> Verdict {
    for n in 0..200 {
        let text = format!(
            "Fact {n} about the isolated space: {}",
            "quite distinct words ".repeat(1 + n % 4)
        );
        if remember(memory, &f.isolated, &text, "space", true)
            .await
            .is_none()
        {
            return ensure(n > 3, "a few memories fit before the budget runs out");
        }
    }
    Err("saves past the budget (about 1,000 tokens a scope) must be refused".into())
}
