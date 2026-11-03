//! Connected tools past a tenth of the window wait behind `tool_search`: nothing changes below
//! that, what the model loads stays on the end of its list for the rest of the thread, and a
//! loaded tool asks and runs as any other.

use std::sync::Arc;

use glave_event::{Body, StopReason};
use glave_harness::{Harness, NoOneToAsk, Settings, Thread};
use glave_ports::{
    Completion, Effect, InMemoryBlobs, InMemoryLog, InMemoryTools, Message, ModelRequest, ToolInfo,
    ToolOutput, ToolSpec,
};
use serde_json::json;

use crate::support::*;

const SEARCH: &str = "mcp__docs__search";
const CREATE: &str = "mcp__tracker__create_issue";

fn tool(name: &str, effect: Effect, about: &str, answer: &str) -> (ToolInfo, ToolOutput) {
    let info = ToolInfo {
        spec: ToolSpec {
            name: name.into(),
            description: format!("{about}. {}", "It has a long description. ".repeat(400)),
            parameters: json!({"type": "object"}),
        },
        effect,
        parallel: effect == Effect::Reads,
    };
    (info, ToolOutput::text(false, answer))
}

fn tools() -> Vec<(ToolInfo, ToolOutput)> {
    vec![
        tool(
            SEARCH,
            Effect::Reads,
            "Search the documents",
            "The setup guide.",
        ),
        tool(
            CREATE,
            Effect::Runs,
            "File an issue in the tracker",
            "Issue 7.",
        ),
    ]
}

/// Settings whose window these tools crowd, and nothing else: a tenth of it is about 8,000
/// characters, and the conversation stays too short to compact.
fn narrow(mut settings: Settings) -> Settings {
    settings.compaction.window_tokens = 20_000;
    settings
}

/// A thread where the model answers `answers`, offered `source`.
struct Running {
    harness: Harness<ScriptedModel, InMemoryLog, InMemoryBlobs, NoOneToAsk>,
    seen: Seen,
    source: Arc<InMemoryTools>,
    thread: Thread,
}

async fn running(
    dir: &std::path::Path,
    answers: Vec<Completion>,
    settings: Settings,
    source: InMemoryTools,
) -> Running {
    let source = Arc::new(source);
    let log = InMemoryLog::new();
    let (harness, seen) = harness_seeing(dir, answers, NoOneToAsk, log, 1, settings).await;
    let harness = harness.with_tools(vec![Arc::<InMemoryTools>::clone(&source)]);
    let thread = harness.start_thread(scope()).await.unwrap();
    Running {
        harness,
        seen,
        source,
        thread,
    }
}

/// A thread whose connected tools crowd its window.
async fn crowded(dir: &std::path::Path, answers: Vec<Completion>, settings: Settings) -> Running {
    let source = InMemoryTools::new(tools()).as_connected();
    running(dir, answers, narrow(settings), source).await
}

fn names(request: &ModelRequest) -> Vec<String> {
    request.tools.iter().map(|tool| tool.name.clone()).collect()
}

fn listing(request: &ModelRequest) -> String {
    let search = request.tools.iter().find(|tool| tool.name == "tool_search");
    search.unwrap().description.clone()
}

fn searching(query: &str) -> Completion {
    let call = ("tool_search", json!({"query": query}));
    answer(None, vec![call], StopReason::ToolUse)
}

fn calling(name: &str) -> Completion {
    answer(None, vec![(name, json!({}))], StopReason::ToolUse)
}

fn done() -> Completion {
    answer(Some("Done."), vec![], StopReason::EndTurn)
}

#[test]
fn while_connected_tools_fit_nothing_changes() {
    let dir = folder("tool-search-fit");
    block_on!(async {
        let source = InMemoryTools::new(tools()).as_connected();
        let mut on = running(&dir, vec![done()], settings(), source).await;
        turn(&on.harness, &mut on.thread, "hello").await;
        let offered = names(&on.seen.lock().unwrap()[0]);
        assert!(offered.contains(&SEARCH.to_owned()), "{offered:?}");
        assert!(!offered.contains(&"tool_search".to_owned()), "{offered:?}");
    });
}

#[test]
fn a_source_that_is_not_connected_is_never_deferred_however_large() {
    let dir = folder("tool-search-own");
    block_on!(async {
        let source = InMemoryTools::new(tools());
        let mut on = running(&dir, vec![done()], narrow(settings()), source).await;
        turn(&on.harness, &mut on.thread, "hello").await;
        let offered = names(&on.seen.lock().unwrap()[0]);
        assert!(offered.contains(&SEARCH.to_owned()), "{offered:?}");
        assert!(!offered.contains(&"tool_search".to_owned()), "{offered:?}");
    });
}

#[test]
fn past_a_tenth_they_are_listed_found_loaded_on_the_end_and_called() {
    let dir = folder("tool-search-found");
    block_on!(async {
        let answers = vec![searching("documents"), calling(SEARCH), done()];
        let mut on = crowded(&dir, answers, settings()).await;
        turn(&on.harness, &mut on.thread, "how do I set up?").await;

        let seen = on.seen.lock().unwrap();
        let first = names(&seen[0]);
        assert!(
            first.contains(&"bash".to_owned()),
            "glave's own stay: {first:?}"
        );
        assert!(
            !first.iter().any(|name| name.starts_with("mcp__")),
            "{first:?}"
        );
        let listed = listing(&seen[0]);
        assert!(
            listed.contains(&format!("{SEARCH}: Search the documents\n")),
            "{listed}"
        );
        assert!(
            listed.contains(&format!("{CREATE}: File an issue")),
            "{listed}"
        );
        assert!(
            !listed.contains("long description"),
            "one line each: {listed}"
        );

        // The list before it is untouched, tool_search's description included, for the cache.
        assert_eq!(seen[1].tools[..seen[0].tools.len()], seen[0].tools[..]);
        assert_eq!(names(&seen[1])[first.len()..], [SEARCH]);
        assert_eq!(on.source.calls().len(), 1);
        assert_eq!(results(on.thread.messages())[1], "The setup guide.");

        let recorded = events(on.harness.log(), on.thread.id).await;
        let loaded: Vec<_> = recorded
            .iter()
            .filter_map(|event| match &event.body {
                Body::ToolsLoaded(loaded) => Some(loaded.tools.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(loaded, [[SEARCH.to_owned()]]);
    });
}

#[test]
fn what_was_loaded_stays_through_turns_and_a_resume_and_more_goes_after_it() {
    let dir = folder("tool-search-stays");
    block_on!(async {
        let answers = vec![
            searching(CREATE),
            done(),
            done(),
            searching(SEARCH),
            calling(SEARCH),
            done(),
        ];
        let mut on = crowded(&dir, answers, settings()).await;
        turn(&on.harness, &mut on.thread, "one").await;
        turn(&on.harness, &mut on.thread, "two").await;
        let mut resumed = on
            .harness
            .resume_thread(scope(), on.thread.id)
            .await
            .unwrap();
        turn(&on.harness, &mut resumed, "three").await;

        let seen = on.seen.lock().unwrap();
        let before_any = seen[0].tools.len();
        for later in seen.iter().skip(1) {
            assert_eq!(later.tools[..before_any], seen[0].tools[..]);
        }
        assert_eq!(names(&seen[1])[before_any..], [CREATE], "loaded this turn");
        assert_eq!(
            names(&seen[2])[before_any..],
            [CREATE],
            "kept the next turn"
        );
        assert_eq!(
            names(&seen[3])[before_any..],
            [CREATE],
            "kept after a resume"
        );
        assert_eq!(
            names(&seen[4])[before_any..],
            [CREATE, SEARCH],
            "the next on the end"
        );
    });
}

/// The ids of the calls `thread`'s model made, in order.
fn call_ids(thread: &Thread) -> Vec<String> {
    thread
        .messages()
        .iter()
        .filter_map(|message| match message {
            Message::Assistant { tool_calls, .. } => Some(tool_calls),
            _ => None,
        })
        .flatten()
        .map(|call| call.call_id.as_str().to_owned())
        .collect()
}

#[test]
fn a_resume_numbers_calls_as_the_thread_going_on_would_have() {
    let run = |resume: bool| {
        let dir = folder("tool-search-ids");
        block_on!(async {
            let answers = vec![searching(SEARCH), done(), calling(SEARCH), done()];
            let mut on = crowded(&dir, answers, settings()).await;
            turn(&on.harness, &mut on.thread, "one").await;
            if resume {
                on.thread = on
                    .harness
                    .resume_thread(scope(), on.thread.id)
                    .await
                    .unwrap();
            }
            turn(&on.harness, &mut on.thread, "two").await;
            call_ids(&on.thread)
        })
    };
    assert_eq!(run(true), run(false));
}

#[test]
fn a_tool_not_loaded_yet_is_never_run_on_a_guess() {
    let dir = folder("tool-search-guess");
    block_on!(async {
        let mut on = crowded(&dir, vec![calling(SEARCH), done()], settings()).await;
        turn(&on.harness, &mut on.thread, "guess").await;
        assert!(on.source.calls().is_empty());
        let said = results(on.thread.messages());
        assert!(
            said[0].contains("Call tool_search with its name"),
            "{said:?}"
        );
    });
}

#[test]
fn a_search_that_finds_nothing_loads_nothing() {
    let dir = folder("tool-search-none");
    block_on!(async {
        let mut on = crowded(&dir, vec![searching("kubernetes"), done()], settings()).await;
        turn(&on.harness, &mut on.thread, "find it").await;
        let seen = on.seen.lock().unwrap();
        assert_eq!(seen[1].tools, seen[0].tools);
        let said = results(on.thread.messages());
        assert!(said[0].contains("No connected tool matches"), "{said:?}");
    });
}

#[test]
fn a_loaded_tool_asks_as_it_would_have_in_the_list() {
    let dir = folder("tool-search-asks");
    block_on!(async {
        let answers = vec![searching(CREATE), calling(CREATE), done()];
        let mut on = crowded(&dir, answers, asking()).await;
        turn(&on.harness, &mut on.thread, "file it").await;
        assert!(
            on.source.calls().is_empty(),
            "with no one to ask, it never ran"
        );
        let said = results(on.thread.messages());
        assert!(
            said[1].contains("No one is available to approve"),
            "{said:?}"
        );
    });
}

#[test]
fn an_explorer_searches_only_what_it_may_use_and_loads_for_itself() {
    let dir = folder("tool-search-explorer");
    block_on!(async {
        let agent =
            json!({"description": "look", "prompt": "Look it up.", "subagent_type": "explore"});
        let answers = vec![
            answer(None, vec![("agent", agent)], StopReason::ToolUse),
            searching(SEARCH),
            calling(SEARCH),
            answer(Some("Found it."), vec![], StopReason::EndTurn),
            done(),
        ];
        let mut settings = settings();
        settings.subagents = true;
        let mut on = crowded(&dir, answers, settings).await;
        turn(&on.harness, &mut on.thread, "find out").await;

        let seen = on.seen.lock().unwrap();
        let listed = listing(&seen[1]);
        assert!(listed.contains(SEARCH), "{listed}");
        assert!(!listed.contains(CREATE), "an explorer only reads: {listed}");
        assert_eq!(names(&seen[2]).last().map(String::as_str), Some(SEARCH));
        assert_eq!(on.source.calls().len(), 1);
        assert!(
            !names(&seen[4]).contains(&SEARCH.to_owned()),
            "what a child loaded is its own: {:?}",
            names(&seen[4])
        );
    });
}
