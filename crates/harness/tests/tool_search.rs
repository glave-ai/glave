//! Connected tools past a tenth of the window wait behind `tool_search`: nothing changes below
//! that, what the model loads stays on the end of its list for the rest of the thread, and a
//! loaded tool asks and runs as any other.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use glave_event::{Arguments, Body, CallId, StopReason, ThreadId};
use glave_harness::{
    Approver, Cancel, Decision, Harness, HarnessError, Live, NoOneToAsk, Settings, Thread,
};
use glave_ports::{
    BoxFuture, Caller, Cancellation, Completion, Effect, EventLog, InMemoryBlobs, InMemoryLog,
    InMemoryTools, Message, ModelRequest, ToolError, ToolInfo, ToolOutput, ToolSource, ToolSpec,
};
use serde_json::json;

use crate::network::InTheLog;
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
struct Running<A: Approver = NoOneToAsk> {
    harness: Harness<ScriptedModel, InMemoryLog, InMemoryBlobs, A>,
    seen: Seen,
    source: Arc<InMemoryTools>,
    thread: Thread,
}

async fn running_with<A: Approver>(
    dir: &std::path::Path,
    answers: Vec<Completion>,
    settings: Settings,
    source: InMemoryTools,
    approver: A,
) -> Running<A> {
    let source = Arc::new(source);
    let log = InMemoryLog::new();
    let (harness, seen) = harness_seeing(dir, answers, approver, log, 1, settings).await;
    let harness = harness.with_tools(vec![Arc::<InMemoryTools>::clone(&source)]);
    let thread = harness.start_thread(scope()).await.unwrap();
    Running {
        harness,
        seen,
        source,
        thread,
    }
}

async fn running(
    dir: &std::path::Path,
    answers: Vec<Completion>,
    settings: Settings,
    source: InMemoryTools,
) -> Running {
    running_with(dir, answers, settings, source, NoOneToAsk).await
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

        let seen = on.seen.lock().unwrap().clone();
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

        let seen = on.seen.lock().unwrap().clone();
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
        let seen = on.seen.lock().unwrap().clone();
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

        let seen = on.seen.lock().unwrap().clone();
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

/// The call `thread`'s log asks the person about.
async fn asked<L: EventLog>(log: &L, thread: ThreadId) -> CallId {
    let recorded = events(log, thread).await;
    let asked = recorded.iter().find_map(|event| match &event.body {
        Body::ApprovalRequested(asked) => Some(asked.call_id.clone()),
        _ => None,
    });
    asked.unwrap()
}

/// Runs a turn that stops at a question for the person, answers yes, and goes on.
async fn stops_then_goes_on(on: &mut Running<InTheLog>, prompt: &str) {
    let cancel = Cancel::new();
    let mut started = None;
    let mut live = |update: Live| {
        if let Live::SubagentStarted { thread, .. } = update {
            started = Some(thread);
        }
    };
    let stopped = on
        .harness
        .run_turn(&mut on.thread, prompt, &cancel, &mut live)
        .await;
    assert!(
        matches!(stopped, Err(HarnessError::Waits(_))),
        "{stopped:?}"
    );
    let asking = started.unwrap_or(on.thread.id);
    let call = asked(on.harness.log(), asking).await;
    on.harness
        .decide(scope(), asking, &call, Decision::AllowOnce, None)
        .await
        .unwrap();
    on.thread = on
        .harness
        .resume_thread(scope(), on.thread.id)
        .await
        .unwrap();
    on.harness
        .go_on(&mut on.thread, &Caller::default(), &cancel, &mut |_| {})
        .await
        .unwrap();
}

#[test]
fn a_loaded_tool_waiting_on_the_person_goes_on_after_a_resume() {
    let dir = folder("tool-search-waits");
    block_on!(async {
        let answers = vec![searching(CREATE), calling(CREATE), done()];
        let source = InMemoryTools::new(tools()).as_connected();
        let mut on = running_with(&dir, answers, narrow(asking()), source, InTheLog).await;
        stops_then_goes_on(&mut on, "file it").await;
        assert_eq!(on.source.calls().len(), 1);
        let seen = on.seen.lock().unwrap().clone();
        assert_eq!(names(&seen[2]).last().map(String::as_str), Some(CREATE));
    });
}

#[test]
fn a_child_that_loaded_a_tool_has_it_still_when_it_goes_on_from_a_wait() {
    let dir = folder("tool-search-child-waits");
    block_on!(async {
        let agent =
            json!({"description": "file", "prompt": "File it.", "subagent_type": "general"});
        let answers = vec![
            answer(None, vec![("agent", agent)], StopReason::ToolUse),
            searching(CREATE),
            calling(CREATE),
            answer(Some("Filed."), vec![], StopReason::EndTurn),
            done(),
        ];
        let mut settings = narrow(asking());
        settings.subagents = true;
        let source = InMemoryTools::new(tools()).as_connected();
        let mut on = running_with(&dir, answers, settings, source, InTheLog).await;
        stops_then_goes_on(&mut on, "file it").await;
        assert_eq!(on.source.calls().len(), 1);
        let seen = on.seen.lock().unwrap().clone();
        assert_eq!(
            names(&seen[3]).last().map(String::as_str),
            Some(CREATE),
            "the child's next call, after the wait"
        );
    });
}

/// Connected tools that grow while a thread waits: only `create_issue` until `grown`.
struct Grows {
    tools: InMemoryTools,
    grown: AtomicBool,
}

impl ToolSource for Grows {
    fn tools(&self) -> BoxFuture<'_, Result<Vec<ToolInfo>, ToolError>> {
        let grown = self.grown.load(Ordering::SeqCst);
        Box::pin(async move {
            let mut tools = self.tools.tools().await?;
            tools.retain(|tool| grown || tool.spec.name == CREATE);
            Ok(tools)
        })
    }

    fn connected(&self) -> bool {
        true
    }

    fn call<'a>(
        &'a self,
        name: &'a str,
        arguments: &'a Arguments,
        cancel: &'a dyn Cancellation,
    ) -> BoxFuture<'a, ToolOutput> {
        self.tools.call(name, arguments, cancel)
    }
}

#[test]
fn an_answer_still_decides_a_call_whose_tool_was_held_back_while_it_waited() {
    let dir = folder("tool-search-grew");
    block_on!(async {
        let answers = vec![calling(CREATE), done()];
        let mut settings = asking();
        // Room for one of these tools, not for both.
        settings.compaction.window_tokens = 30_000;
        let log = InMemoryLog::new();
        let (harness, seen) = harness_seeing(&dir, answers, InTheLog, log, 1, settings).await;
        let source = Arc::new(Grows {
            tools: InMemoryTools::new(tools()),
            grown: AtomicBool::new(false),
        });
        let harness = harness.with_tools(vec![Arc::<Grows>::clone(&source)]);
        let mut thread = harness.start_thread(scope()).await.unwrap();
        let cancel = Cancel::new();
        let stopped = harness
            .run_turn(&mut thread, "file it", &cancel, &mut |_| {})
            .await;
        assert!(
            matches!(stopped, Err(HarnessError::Waits(_))),
            "{stopped:?}"
        );
        assert!(names(&seen.lock().unwrap().clone()[0]).contains(&CREATE.to_owned()));
        source.grown.store(true, Ordering::SeqCst);
        let call = asked(harness.log(), thread.id).await;
        harness
            .decide(scope(), thread.id, &call, Decision::AllowOnce, None)
            .await
            .unwrap();
        harness
            .go_on(&mut thread, &Caller::default(), &cancel, &mut |_| {})
            .await
            .unwrap();
        assert_eq!(source.tools.calls().len(), 1);
        assert_eq!(results(thread.messages())[0], "Issue 7.");
        let after = names(&seen.lock().unwrap().clone()[1]);
        assert!(after.contains(&"tool_search".to_owned()), "{after:?}");
    });
}
