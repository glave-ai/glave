//! A thread's prefix (`specs/invariants.md`, 27): every model call of a thread opens with the
//! same system prompt and tools, byte for byte, whatever changes between its turns; what
//! changed reaches the model at the end, in a note. Only a compaction, another model and a
//! connection of the person's turned on (`connection_places.rs`) make a new prefix: a server of
//! their own settings waits.

use std::future::{Future, ready};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use glave_event::{Arguments, Body, Permissions, StopReason};
use glave_harness::{
    AllowAll, ApprovalRequest, Approver, Cancel, Decision, Defaults, Harness, OwnTools, Skills,
    Thread, find_skills, skill_folders,
};
use glave_ports::computer::{CredentialHost, Forge, GitCredential};
use glave_ports::settings::Protections;
use glave_ports::testing::{FixedGitCredentials, fake_secret};
use glave_ports::{
    BoxFuture, Caller, Cancellation, Completion, InMemoryBlobs, InMemoryLog, InMemoryTools,
    Message, ModelRequest, Purpose, ToolError, ToolInfo, ToolOutput, ToolSource,
};
use serde_json::json;

use crate::project_folders::{Project, in_project, tool};
use crate::support::*;

type Held = Harness<ScriptedModel, InMemoryLog, InMemoryBlobs, AllowAll>;

/// A tool server that stops answering while `down`, and comes back.
pub(crate) struct Flaky {
    pub(crate) down: AtomicBool,
    pub(crate) tools: InMemoryTools,
}

impl ToolSource for Flaky {
    fn tools(&self) -> BoxFuture<'_, Result<Vec<ToolInfo>, ToolError>> {
        if self.down.load(Ordering::SeqCst) {
            return Box::pin(ready(Err(ToolError {
                source_name: "tracker".into(),
                message: "it did not answer within 30 seconds".into(),
            })));
        }
        self.tools.tools()
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

/// What the test changes between turns.
struct World {
    project: Arc<Project>,
    credentials: Arc<FixedGitCredentials>,
    tracker: Arc<Flaky>,
}

/// Writes a skill called `name` into the project at `project_dir`.
fn install_skill(project_dir: &Path, name: &str) {
    let skill = project_dir.join(".glave/skills").join(name);
    std::fs::create_dir_all(&skill).unwrap();
    let text = format!("---\nname: {name}\ndescription: How {name} is done here.\n---\nDo it.\n");
    std::fs::write(skill.join("SKILL.md"), text).unwrap();
}

/// What a request opens with, as the provider's prompt cache reads it.
fn prefix(request: &ModelRequest) -> (String, Vec<u8>) {
    let tools = serde_json::to_vec(&request.tools).unwrap();
    (request.system.clone(), tools)
}

/// The text of the last message the person's side sent in `request`.
fn last_said(request: &ModelRequest) -> String {
    let said = request
        .messages
        .iter()
        .rev()
        .find(|message| matches!(message, Message::User { .. }));
    texts(said.unwrap()).join("\n")
}

fn done(text: &str) -> Completion {
    answer(Some(text), vec![], StopReason::EndTurn)
}

/// A harness with plans on, a skill, the tracker and git credentials with no host yet, and a
/// thread in its project at `project_dir`.
async fn set_up(
    project_dir: &Path,
    world: &World,
    answers: Vec<Completion>,
) -> (Held, Thread, Seen) {
    let mut settings = settings();
    settings.plans = true;
    let credentials = Arc::clone(&world.credentials);
    let workspace = workspace(project_dir).await.with_credentials(credentials);
    let log = InMemoryLog::new();
    let (harness, seen) = harness_with(workspace, answers, AllowAll, log, 1, settings);
    let found = find_skills(&skill_folders(project_dir, &[])).await.skills;
    let skills = Skills::found_again(skill_folders(project_dir, &[]), Vec::new(), found);
    let harness = harness
        .with_tools(vec![Arc::clone(&world.tracker) as Arc<dyn ToolSource>])
        .with_skills(Some(skills));
    let (harness, thread) = in_project(harness, &world.project, project_dir).await;
    (harness, thread, seen)
}

/// Between turns: a git host connected, plan mode on, the project trusted (its tools and
/// hooks read again), a tool source of the person's own that is no connection (one turned on
/// makes the prefix again, `tool_search.rs`), the settings read again with `web_fetch`, a skill
/// installed, and the tracker no longer answering.
async fn change_everything(harness: &Held, thread: &mut Thread, world: &World, project_dir: &Path) {
    world.credentials.set_hosts(vec![CredentialHost {
        url: "https://github.com".into(),
        forge: Forge::Github,
    }]);
    let caller = Caller::default();
    harness
        .choose_permissions(thread, &caller, Permissions::Plan)
        .await
        .unwrap();
    world.project.trust();
    harness.set_own_tools(OwnTools::everywhere(vec![Arc::new(tool(
        "mcp__mail__send",
        "Sent.",
    ))]));
    let web = Arc::new(tool("web_fetch", "A page.")) as Arc<dyn ToolSource>;
    harness.set_defaults(Defaults {
        web: Some(web),
        ..Defaults::clone(&harness.defaults())
    });
    install_skill(project_dir, "release-notes");
    world.tracker.down.store(true, Ordering::SeqCst);
}

/// Every call opened with the first's prefix until the compaction, which took what is on
/// offer then, and kept it.
fn opened_alike(requests: &[ModelRequest]) {
    assert_eq!(requests.len(), 8, "four turns, a summary, three turns");
    let first = prefix(&requests[0]);
    for (at, request) in requests[..5].iter().enumerate() {
        assert!(prefix(request) == first, "call {at} opens as the first did");
    }
    // The summary is sent the conversation as the call before it was, then that call's answer
    // and the instruction, and reasons as it did, so the provider's cache holds all of it.
    let (built, summary) = (&requests[3], &requests[4]);
    assert_eq!(summary.purpose, Purpose::Summary);
    assert!(summary.messages.starts_with(&built.messages));
    assert_eq!(summary.messages.len(), built.messages.len() + 2);
    assert_eq!(summary.effort, built.effort);
    let compacted = prefix(&requests[5]);
    assert!(
        compacted != first,
        "the compaction opened with what is on offer now"
    );
    assert!(prefix(&requests[6]) == compacted, "and kept it");
    // Another model gets its own: the prompt names it, the tools are those on offer.
    assert!(
        requests[7].system.contains("another-model"),
        "{}",
        requests[7].system
    );
    assert_eq!(requests[7].tools, requests[6].tools);
    let offered =
        |request: &ModelRequest, name: &str| request.tools.iter().any(|spec| spec.name == name);
    for name in ["mcp__docs__search", "mcp__mail__send", "web_fetch"] {
        assert!(
            !offered(&requests[4], name),
            "{name} waited for the compaction"
        );
        assert!(offered(&requests[5], name), "{name} came with it");
    }
    assert!(
        offered(&requests[4], "mcp__tracker__search"),
        "listed while down"
    );
}

/// What changed reached the model at the end, each once.
fn told_at_the_end(requests: &[ModelRequest]) {
    let news = last_said(&requests[1]);
    assert!(news.starts_with("<system-reminder>\n"), "{news}");
    for told in [
        "Accounts the person connected",
        "Plan mode:",
        "`mcp__mail__send`",
        "`mcp__docs__search`",
        "`web_fetch`",
        "release-notes",
    ] {
        assert!(news.contains(told), "{told}: {news}");
    }
    assert!(news.ends_with("look up the issue"), "{news}");
    let off = last_said(&requests[3]);
    assert!(off.contains("Plan mode is off"), "{off}");
    assert!(
        !off.contains("Accounts") && !off.contains("tracker"),
        "{off}"
    );
    assert!(!last_said(&requests[6]).contains("<system-reminder>"));
}

#[test]
fn every_call_opens_with_the_same_prompt_and_tools_whatever_changes_mid_thread() {
    let dir = folder("prefix-frozen");
    let project_dir = dir.join("project");
    std::fs::create_dir_all(&project_dir).unwrap();
    install_skill(&project_dir, "house-style");
    let credential = GitCredential::new("x-access-token", fake_secret("ghp_")).unwrap();
    let world = World {
        project: Project::new(Protections::default()),
        credentials: Arc::new(FixedGitCredentials::new(Vec::new(), credential)),
        tracker: Arc::new(Flaky {
            down: AtomicBool::new(false),
            tools: tool("mcp__tracker__search", "Issue 7."),
        }),
    };
    block_on!(async {
        let search = vec![("mcp__tracker__search", json!({}))];
        let answers = vec![
            done("Hello."),
            answer(None, search, StopReason::ToolUse),
            done("The tracker is not answering."),
            done("Built."),
            done("Summary: said hello, the tracker was down, then built."),
            done("Still here."),
            done("Again."),
            done("From another model."),
        ];
        let (harness, mut thread, seen) = set_up(&project_dir, &world, answers).await;
        let caller = Caller::default();
        turn(&harness, &mut thread, "hello").await;
        change_everything(&harness, &mut thread, &world, &project_dir).await;
        turn(&harness, &mut thread, "look up the issue").await;
        // Plan mode off, and the tracker back.
        harness
            .choose_permissions(&mut thread, &caller, Permissions::Auto)
            .await
            .unwrap();
        world.tracker.down.store(false, Ordering::SeqCst);
        turn(&harness, &mut thread, "build it").await;
        harness
            .compact(&mut thread, &Cancel::new(), &mut |_| {})
            .await
            .unwrap();
        turn(&harness, &mut thread, "still there?").await;
        turn(&harness, &mut thread, "and now?").await;
        harness
            .choose_model(&mut thread, &caller, "another-model")
            .await
            .unwrap();
        turn(&harness, &mut thread, "who are you?").await;

        let requests = seen.lock().unwrap().clone();
        opened_alike(&requests);
        told_at_the_end(&requests);

        // The tracker's tool, called while down, said so, and the turn recorded why.
        let events = events(harness.log(), thread.id).await;
        let output = events.iter().find_map(|event| match &event.body {
            Body::ToolCallCompleted(done) => Some(format!("{:?}", done.output)),
            _ => None,
        });
        let output = output.unwrap();
        assert!(output.contains("is not answering"), "{output}");
        let missing = events.iter().filter_map(|event| match &event.body {
            Body::TurnStarted(started) => Some(started.missing.len()),
            _ => None,
        });
        assert_eq!(missing.collect::<Vec<_>>(), [0, 1, 0, 0, 0, 0]);
    });
}

/// Someone answers until they leave, as a thread's later turn run as a task.
struct Leaves(AtomicBool);

impl Approver for Leaves {
    fn decide(&self, _request: &ApprovalRequest) -> impl Future<Output = Decision> + Send {
        ready(Decision::Unavailable)
    }

    fn can_answer(&self, _thread: glave_event::ThreadId) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

#[test]
fn a_question_listed_from_the_first_call_is_told_when_no_one_answers_now() {
    let dir = folder("prefix-question");
    block_on!(async {
        let ask = json!({"questions": [{"question": "Tabs?", "options": []}]});
        let answers = vec![
            done("Hello."),
            answer(None, vec![("question", ask)], StopReason::ToolUse),
            done("I went with tabs."),
        ];
        let approver = Leaves(AtomicBool::new(true));
        let log = InMemoryLog::new();
        let (harness, seen) = harness_seeing(&dir, answers, approver, log, 1, settings()).await;
        let mut thread = harness.start_thread(scope()).await.unwrap();
        turn(&harness, &mut thread, "hello").await;
        harness.approver().0.store(false, Ordering::SeqCst);
        turn(&harness, &mut thread, "tabs or spaces?").await;
        let requests = seen.lock().unwrap().clone();
        assert!(
            requests
                .iter()
                .all(|request| prefix(request) == prefix(&requests[0]))
        );
        assert_eq!(
            results(thread.messages()),
            [
                "No one is there to answer questions in this turn. Go on with your best judgement, \
              and say in your answer what you assumed."
            ]
        );
    });
}
