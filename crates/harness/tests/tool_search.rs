//! Connected tools past what the model's list holds (#247): each connector is one tool and
//! `tool_search` finds the rest, on a prefix that never changes but once, when a connection is
//! turned on (`specs/invariants.md`, 27). Calls name the real tool by the time they are asked
//! about, run and recorded.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use glave_event::{Arguments, Body, StopReason};
use glave_harness::{AllowAll, Decision, OwnTools};
use glave_ports::{
    BoxFuture, Cancellation, Completion, Effect, InMemoryLog, InMemoryTools, ModelRequest,
    ToolError, ToolInfo, ToolOutput, ToolSource, ToolSpec,
};
use serde_json::{Value, json};

use crate::support::*;

/// `count` tools of `connector`, as a service someone connected offers them: each takes a
/// `number`, the first only reads, and `tool_7` answers with what it found.
fn connection(connector: &str, count: usize) -> Arc<InMemoryTools> {
    let tools = (0..count).map(|n| {
        let spec = ToolSpec {
            name: format!("mcp__{connector}__tool_{n}"),
            description: format!("Tool {n} of {connector}, which does its own thing."),
            parameters: json!({"type": "object", "properties": {
                "number": {"type": "integer"}
            }, "required": ["number"]}),
        };
        let effect = if n == 0 { Effect::Reads } else { Effect::Runs };
        let info = ToolInfo {
            spec,
            effect,
            parallel: false,
        };
        (
            info,
            ToolOutput::text(false, format!("{connector} {n} answered")),
        )
    });
    Arc::new(InMemoryTools::new(tools.collect()).as_connected())
}

fn done(text: &str) -> Completion {
    answer(Some(text), vec![], StopReason::EndTurn)
}

fn calls(calls: Vec<(&str, Value)>) -> Completion {
    answer(None, calls, StopReason::ToolUse)
}

/// What a request opens with, as the provider's prompt cache reads it.
fn prefix(request: &ModelRequest) -> (String, Vec<u8>) {
    let tools = serde_json::to_vec(&request.tools).unwrap();
    (request.system.clone(), tools)
}

fn names(request: &ModelRequest) -> Vec<&str> {
    request
        .tools
        .iter()
        .map(|spec| spec.name.as_str())
        .collect()
}

/// The text of the last message the person's side sent in `request`.
fn last_said(request: &ModelRequest) -> String {
    texts(request.messages.last().unwrap()).join("\n")
}

/// The results of `thread`'s calls, in order.
async fn outputs(log: &InMemoryLog, thread: glave_event::ThreadId) -> Vec<String> {
    let events = events(log, thread).await;
    let done = events.iter().filter_map(|event| match &event.body {
        Body::ToolCallCompleted(done) => Some(format!("{:?}", done.output)),
        _ => None,
    });
    done.collect()
}

/// The thread's calls opened alike until the connection came, then alike again.
fn one_prefix_made_again_once(requests: &[ModelRequest]) {
    assert_eq!(requests.len(), 6);
    let first = &requests[0];
    let listed = names(first);
    let at = listed
        .iter()
        .position(|name| *name == "tool_search")
        .unwrap();
    assert_eq!(listed[at + 1], "mcp__github");
    assert!(!listed.iter().any(|name| name.starts_with("mcp__github__")));
    assert!(
        requests[..3].iter().all(|r| prefix(r) == prefix(first)),
        "one prefix"
    );
    let rebuilt = &requests[3];
    assert!(prefix(rebuilt) != prefix(first), "made again for jira");
    assert!(
        names(rebuilt).contains(&"mcp__jira"),
        "{:?}",
        names(rebuilt)
    );
    assert!(
        requests[3..].iter().all(|r| prefix(r) == prefix(rebuilt)),
        "and kept"
    );
    let news = last_said(rebuilt);
    assert!(news.contains("your tool list has them now"), "{news}");
    assert!(
        news.contains("`mcp__jira` (3 tools, which tool_search finds)"),
        "{news}"
    );
}

#[test]
fn a_crowd_of_connected_tools_is_searched_and_called_through_its_connector_on_one_prefix() {
    let dir = folder("tool-search");
    let github = connection("github", 150);
    block_on!(async {
        let found = json!({"query": "tool 7", "connector": "github"});
        let called = json!({"tool": "tool_7", "arguments": {"number": 4120}});
        let answers = vec![
            calls(vec![("tool_search", found)]),
            calls(vec![("mcp__github", called)]),
            done("Tool 7 answered."),
            done("Jira is here."),
            calls(vec![(
                "mcp__jira",
                json!({"tool": "tool_1", "arguments": {"number": 1}}),
            )]),
            done("Jira is off."),
        ];
        let approver = Answers::new(&[Decision::AllowOnce]);
        let log = InMemoryLog::new();
        let (harness, seen) = harness_seeing(&dir, answers, approver, log, 1, asking()).await;
        harness.set_own_tools(OwnTools::everywhere(vec![
            Arc::clone(&github) as Arc<dyn ToolSource>
        ]));
        let mut thread = harness.start_thread(scope()).await.unwrap();
        turn(&harness, &mut thread, "what does tool 7 say about 4120?").await;
        // A connection turned on: the prefix is made again, once, and the model told.
        let jira = connection("jira", 3);
        harness.set_own_tools(OwnTools::everywhere(vec![
            Arc::clone(&github) as Arc<dyn ToolSource>,
            Arc::clone(&jira) as Arc<dyn ToolSource>,
        ]));
        turn(&harness, &mut thread, "and jira?").await;
        // Turned off: its calls are refused at once, the prefix as it was.
        harness.set_own_tools(OwnTools::everywhere(vec![
            Arc::clone(&github) as Arc<dyn ToolSource>
        ]));
        turn(&harness, &mut thread, "jira again").await;

        one_prefix_made_again_once(&seen.lock().unwrap());

        // The search showed the tool's parameters, the call ran the real tool, asked about by
        // its own name, and the call jira's tool made once it was off ran nothing.
        let outputs = outputs(harness.log(), thread.id).await;
        assert!(
            outputs[0].contains("\\\"tool\\\":\\\"tool_7\\\""),
            "{}",
            outputs[0]
        );
        assert!(outputs[1].contains("github 7 answered"), "{}", outputs[1]);
        assert!(
            outputs[2].contains("is not available now"),
            "{}",
            outputs[2]
        );
        let mut number = serde_json::Map::new();
        number.insert("number".into(), json!(4120));
        assert_eq!(
            github.calls(),
            [("mcp__github__tool_7".to_owned(), Arguments::Json(number))]
        );
        assert_eq!(jira.calls().len(), 0, "jira ran nothing");
        let asked = harness.approver().asked();
        assert_eq!(asked.len(), 1);
        assert_eq!(asked[0].tool, "mcp__github__tool_7");
        assert!(
            asked[0].summary.starts_with("mcp__github__tool_7"),
            "{}",
            asked[0].summary
        );
    });
}

#[test]
fn a_call_that_does_not_fit_the_real_tool_is_shown_its_parameters_and_runs_nothing() {
    let dir = folder("tool-search-fits");
    let github = connection("github", 150);
    block_on!(async {
        let guessed = json!({"tool": "tool_7", "arguments": {"issue": 4120}});
        let answers = vec![
            calls(vec![("mcp__github", guessed)]),
            calls(vec![("mcp__github__tool_7", json!({"number": 4120}))]),
            done("Done."),
        ];
        let log = InMemoryLog::new();
        let (harness, _) = harness_seeing(&dir, answers, AllowAll, log, 1, settings()).await;
        harness.set_own_tools(OwnTools::everywhere(vec![
            Arc::clone(&github) as Arc<dyn ToolSource>
        ]));
        let mut thread = harness.start_thread(scope()).await.unwrap();
        turn(&harness, &mut thread, "look up 4120").await;
        let outputs = outputs(harness.log(), thread.id).await;
        assert!(outputs[0].contains("Nothing ran"), "{}", outputs[0]);
        assert!(outputs[0].contains("number"), "{}", outputs[0]);
        // Called by its own name, which the list does not hold, it still runs.
        assert!(outputs[1].contains("github 7 answered"), "{}", outputs[1]);
        assert_eq!(github.calls().len(), 1);
    });
}

#[test]
fn connected_tools_that_fit_stay_listed_and_one_turned_on_is_listed_at_once() {
    let dir = folder("tool-search-fit");
    let github = connection("github", 3);
    block_on!(async {
        let answers = vec![
            done("Hello."),
            calls(vec![("mcp__slack__tool_1", json!({"number": 2}))]),
            done("Posted."),
            done("Still here."),
        ];
        let log = InMemoryLog::new();
        let (harness, seen) = harness_seeing(&dir, answers, AllowAll, log, 1, settings()).await;
        harness.set_own_tools(OwnTools::everywhere(vec![
            Arc::clone(&github) as Arc<dyn ToolSource>
        ]));
        let mut thread = harness.start_thread(scope()).await.unwrap();
        turn(&harness, &mut thread, "hello").await;
        let slack = connection("slack", 2);
        harness.set_own_tools(OwnTools::everywhere(vec![
            Arc::clone(&github) as Arc<dyn ToolSource>,
            Arc::clone(&slack) as Arc<dyn ToolSource>,
        ]));
        turn(&harness, &mut thread, "post it").await;
        // As after a restart: the prefix the log kept, and no note again.
        let mut again = harness.resume_thread(scope(), thread.id).await.unwrap();
        turn(&harness, &mut again, "there?").await;

        let requests = seen.lock().unwrap().clone();
        assert!(!names(&requests[0]).contains(&"tool_search"));
        assert!(names(&requests[0]).contains(&"mcp__github__tool_2"));
        assert!(names(&requests[1]).contains(&"mcp__slack__tool_1"));
        assert!(prefix(&requests[1]) == prefix(&requests[2]), "kept");
        assert!(
            prefix(&requests[3]) == prefix(&requests[1]),
            "kept through the restart"
        );
        assert!(!last_said(&requests[3]).contains("<system-reminder>"));
        let news = last_said(&requests[1]);
        assert!(
            news.contains("`mcp__slack__tool_0`, `mcp__slack__tool_1`"),
            "{news}"
        );
        assert_eq!(slack.calls().len(), 1);
    });
}

#[test]
fn a_thread_read_again_keeps_its_connectors_and_calls_through_them() {
    let dir = folder("tool-search-kept");
    let github = connection("github", 150);
    block_on!(async {
        let called = json!({"tool": "tool_0", "arguments": {"number": 1}});
        let answers = vec![
            done("Hello."),
            calls(vec![("mcp__github", called)]),
            done("Read it."),
        ];
        let log = InMemoryLog::new();
        let (harness, seen) = harness_seeing(&dir, answers, AllowAll, log, 1, settings()).await;
        harness.set_own_tools(OwnTools::everywhere(vec![
            Arc::clone(&github) as Arc<dyn ToolSource>
        ]));
        let thread = {
            let mut thread = harness.start_thread(scope()).await.unwrap();
            turn(&harness, &mut thread, "hello").await;
            thread
        };
        let mut again = harness.resume_thread(scope(), thread.id).await.unwrap();
        turn(&harness, &mut again, "read tool 0").await;
        let requests = seen.lock().unwrap().clone();
        assert!(requests.iter().all(|r| prefix(r) == prefix(&requests[0])));
        assert!(
            !last_said(&requests[1]).contains("<system-reminder>"),
            "nothing came"
        );
        assert_eq!(github.calls().len(), 1);
    });
}

/// Each turn's tools, as the harness was given them before it: `own[n]` before turn `n`.
async fn turns_with(
    dir: &std::path::Path,
    own: Vec<Vec<Arc<dyn ToolSource>>>,
) -> Vec<ModelRequest> {
    let answers = own.iter().map(|_| done("Fine.")).collect();
    let log = InMemoryLog::new();
    let (harness, seen) = harness_seeing(dir, answers, AllowAll, log, 1, settings()).await;
    let mut thread = None;
    for tools in own {
        harness.set_own_tools(OwnTools::everywhere(tools));
        let thread = match &mut thread {
            Some(thread) => thread,
            None => thread.insert(harness.start_thread(scope()).await.unwrap()),
        };
        turn(&harness, thread, "go on").await;
    }
    seen.lock().unwrap().clone()
}

#[test]
fn a_new_tool_of_a_connection_listed_in_full_waits_and_changes_nothing() {
    let dir = folder("tool-search-grew");
    block_on!(async {
        let before = connection("github", 3) as Arc<dyn ToolSource>;
        let after = connection("github", 4) as Arc<dyn ToolSource>;
        let requests = turns_with(&dir, vec![vec![before], vec![after]]).await;
        assert!(
            prefix(&requests[1]) == prefix(&requests[0]),
            "not made again"
        );
        let news = last_said(&requests[1]);
        assert!(
            news.contains("`mcp__github__tool_3`. They are not available here yet"),
            "{news}"
        );
    });
}

#[test]
fn a_connection_that_takes_the_list_past_its_room_lists_every_connector_by_its_tool() {
    let dir = folder("tool-search-flip");
    block_on!(async {
        let github = connection("github", 3) as Arc<dyn ToolSource>;
        let jira = connection("jira", 150) as Arc<dyn ToolSource>;
        let both = vec![Arc::clone(&github), jira];
        let requests = turns_with(&dir, vec![vec![github], both]).await;
        assert!(names(&requests[0]).contains(&"mcp__github__tool_0"));
        let listed = names(&requests[1]);
        assert!(
            listed.contains(&"mcp__github") && listed.contains(&"mcp__jira"),
            "{listed:?}"
        );
        let news = last_said(&requests[1]);
        assert!(
            news.contains("`mcp__jira` (150 tools, which tool_search finds)"),
            "{news}"
        );
        assert!(news.contains("by their connector: `mcp__github`"), "{news}");
    });
}

/// A connection that is not answering while `down`.
struct Down {
    down: AtomicBool,
    tools: Arc<InMemoryTools>,
}

impl ToolSource for Down {
    fn tools(&self) -> BoxFuture<'_, Result<Vec<ToolInfo>, ToolError>> {
        if self.down.load(Ordering::SeqCst) {
            let err = ToolError {
                source_name: "jira".into(),
                message: "it did not answer".into(),
            };
            return Box::pin(std::future::ready(Err(err)));
        }
        self.tools.tools()
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

/// A server that answers again is no connection turned on (#1199): the thread keeps its
/// prefix, read again from the log too, and is told once; its tools come in a new thread.
#[test]
fn a_connection_down_at_the_first_call_comes_back_in_a_new_thread_on_the_prefix_it_had() {
    let dir = folder("tool-search-back");
    block_on!(async {
        let jira = Arc::new(Down {
            down: AtomicBool::new(true),
            tools: connection("jira", 2),
        });
        let own = || vec![Arc::clone(&jira) as Arc<dyn ToolSource>];
        let answers = vec![
            done("Jira is down."),
            done("Jira is back, in a new thread."),
            done("Still."),
            done("Here it is."),
        ];
        let log = InMemoryLog::new();
        let (harness, seen) = harness_seeing(&dir, answers, AllowAll, log, 1, settings()).await;
        harness.set_own_tools(OwnTools::everywhere(own()));
        let mut thread = harness.start_thread(scope()).await.unwrap();
        turn(&harness, &mut thread, "jira?").await;
        jira.down.store(false, Ordering::SeqCst);
        turn(&harness, &mut thread, "jira now?").await;
        // As after a restart, or a thread let go of while idle.
        let mut again = harness.resume_thread(scope(), thread.id).await.unwrap();
        turn(&harness, &mut again, "and now?").await;
        let mut fresh = harness.start_thread(scope()).await.unwrap();
        turn(&harness, &mut fresh, "jira?").await;
        let requests = seen.lock().unwrap().clone();
        assert!(!names(&requests[0]).contains(&"mcp__jira__tool_0"));
        for (at, request) in requests[..3].iter().enumerate() {
            assert!(
                prefix(request) == prefix(&requests[0]),
                "call {at} as the first"
            );
        }
        let news = last_said(&requests[1]);
        assert!(news.contains("answer again: `jira`"), "{news}");
        assert!(news.contains("in a new thread"), "{news}");
        assert!(!news.contains("mcp__jira__tool_0"), "{news}");
        // Read again from the log, the thread knows only what its prefix says: told again.
        assert!(last_said(&requests[2]).contains("answer again: `jira`"));
        assert!(names(&requests[3]).contains(&"mcp__jira__tool_0"));
        assert!(!requests[3].system.contains("is not available"));
    });
}
