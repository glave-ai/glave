//! A thread's prefix kept through what reads it again or goes on with it
//! (`specs/invariants.md`, 27): a thread read again from the log, a turn going on after a
//! wait, and a subagent going on after its parent was read again.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use glave_event::StopReason;
use glave_harness::{AllowAll, Cancel};
use glave_ports::computer::{CredentialHost, Forge, GitCredential};
use glave_ports::testing::{FixedGitCredentials, fake_secret};
use glave_ports::{Caller, Completion, InMemoryLog, ModelRequest, Purpose, ToolSource};

use crate::network::InTheLog;
use crate::prefix::Flaky;
use crate::project_folders::tool;
use crate::subagents::{a_child_writes, asking_with_subagents, until_the_child_is_allowed};
use crate::support::*;

fn done(text: &str) -> Completion {
    answer(Some(text), vec![], StopReason::EndTurn)
}

/// What a request opens with, as the provider's prompt cache reads it.
fn prefix(request: &ModelRequest) -> (String, Vec<u8>) {
    (
        request.system.clone(),
        serde_json::to_vec(&request.tools).unwrap(),
    )
}

#[test]
fn a_thread_read_again_from_the_log_keeps_its_prefix_and_hears_what_changed() {
    let dir = folder("prefix-reread");
    let credential = GitCredential::new("x-access-token", fake_secret("ghp_")).unwrap();
    let credentials = Arc::new(FixedGitCredentials::new(Vec::new(), credential));
    block_on!(async {
        let workspace = workspace(&dir)
            .await
            .with_credentials(Arc::clone(&credentials) as _);
        let answers = vec![done("Hello."), done("Signed in now."), done("Fresh.")];
        let log = InMemoryLog::new();
        let (harness, seen) = harness_with(workspace, answers, AllowAll, log, 1, settings());
        let mut thread = harness.start_thread(scope()).await.unwrap();
        turn(&harness, &mut thread, "hello").await;
        credentials.set_hosts(vec![CredentialHost {
            url: "https://github.com".into(),
            forge: Forge::Github,
        }]);
        // As after a restart, or a thread let go of while idle.
        let mut again = harness.resume_thread(scope(), thread.id).await.unwrap();
        turn(&harness, &mut again, "and now?").await;
        let mut fresh = harness.start_thread(scope()).await.unwrap();
        turn(&harness, &mut fresh, "hi").await;

        let requests = seen.lock().unwrap().clone();
        assert!(
            prefix(&requests[1]) == prefix(&requests[0]),
            "the same bytes"
        );
        let message = texts(requests[1].messages.last().unwrap()).join("\n");
        assert!(
            message.contains("Accounts the person connected"),
            "{message}"
        );
        assert!(requests[2].system.contains("Accounts the person connected"));
    });
}

#[test]
fn a_subagent_going_on_after_a_wait_and_a_restart_keeps_its_own_first_prefix() {
    let dir = folder("prefix-child");
    block_on!(async {
        let (answers, settings) = (a_child_writes(), asking_with_subagents());
        let log = InMemoryLog::new();
        let (harness, seen) = harness_seeing(&dir, answers, InTheLog, log, 1, settings).await;
        let tracker = Arc::new(Flaky {
            down: AtomicBool::new(false),
            tools: tool("mcp__tracker__search", "Issue 7."),
        });
        let harness = harness.with_tools(vec![Arc::clone(&tracker) as Arc<dyn ToolSource>]);
        let mut thread = harness.start_thread(scope()).await.unwrap();
        until_the_child_is_allowed(&harness, &mut thread).await;
        // Read again, as after a restart, with the tracker down: the child is read from its log
        // too, and lists what it listed before.
        tracker.down.store(true, Ordering::SeqCst);
        let mut again = harness.resume_thread(scope(), thread.id).await.unwrap();
        harness
            .go_on(&mut again, &Caller::default(), &Cancel::new(), &mut |_| {})
            .await
            .unwrap();

        let requests = seen.lock().unwrap().clone();
        let (children, parents): (Vec<_>, Vec<_>) =
            (requests.iter()).partition(|request| request.purpose == Purpose::Subagent);
        assert_eq!((children.len(), parents.len()), (2, 2));
        let lists = |request: &ModelRequest| {
            (request.tools.iter()).any(|spec| spec.name == "mcp__tracker__search")
        };
        assert!(lists(children[0]) && lists(children[1]));
        assert!(
            prefix(children[1]) == prefix(children[0]),
            "the child's own"
        );
        assert!(prefix(parents[1]) == prefix(parents[0]), "the parent's own");
        assert!(dir.join("x.txt").exists());
    });
}
