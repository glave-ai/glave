//! Someone else's worktree on the harness's computer, removed by a cleanup while the harness
//! holds their thread, is made again walled as it was (`specs/sandbox.md`, "Whose computer"):
//! its commands still get none of the person's secrets or git sign-ins.

use std::process::Command;

use glave_computer::LocalComputers;
use glave_event::{Forge, RemovedReason, Secret, StopReason, WorkspaceRemoved};
use glave_harness::{AllowAll, Place, Workspace, record_removed};
use glave_ports::computer::{
    CredentialHost, GitCredential, Sandbox, SandboxCaches, SandboxLevel, SandboxNetwork,
};
use glave_ports::secrets::{Known, KnownSecrets};
use glave_ports::testing::{FixedGitCredentials, TempDir, any_secret};
use glave_ports::{Caller, Clock as _, InMemoryLog, SystemClock};
use serde_json::json;

use crate::support::*;

/// Runs `script` in `dir`, which must succeed.
fn sh(dir: &std::path::Path, script: &str) {
    let ran = Command::new("bash")
        .args(["-c", script])
        .current_dir(dir)
        .status()
        .unwrap();
    assert!(ran.success(), "{script}");
}

/// The repository `repo` on this computer, its worktrees in `worktrees`, its commands in the
/// sandbox (`base` their home), with the person's secret and GitHub sign-in.
async fn persons_workspace(
    base: &std::path::Path,
    repo: &std::path::Path,
    worktrees: &std::path::Path,
) -> Workspace {
    let environment = std::env::vars_os()
        .filter(|(name, _)| name == "PATH" || name == "HOME")
        .collect::<Vec<_>>();
    let (computers, info) = LocalComputers::for_here(environment)
        .with_worktrees(worktrees)
        .open_here(repo)
        .await
        .unwrap();
    let sandbox = Sandbox {
        level: SandboxLevel::Project,
        network: SandboxNetwork::when(false),
        home: Some(base.to_owned()),
        glave_home: Some(base.join(".glave")),
        caches: SandboxCaches::Kept,
    };
    let known = KnownSecrets::default();
    known.set(vec![Known::new(
        "STRIPE_KEY".into(),
        Secret::new(any_secret()),
    )]);
    let host = CredentialHost {
        url: "https://github.com".into(),
        forge: Forge::Github,
    };
    let credential = GitCredential::new("x-access-token", any_secret()).unwrap();
    let fixed = std::sync::Arc::new(FixedGitCredentials::new(vec![host], credential));
    Workspace::new(computers, info)
        .with_sandbox(&sandbox)
        .await
        .unwrap()
        .hiding(known)
        .with_credentials(fixed)
}

/// A cleanup in another glave removes the worktree of `thread` at `root`, a worktree of
/// `repo`, and records so.
async fn removed_outside<A: glave_harness::Approver>(
    harness: &glave_harness::Harness<ScriptedModel, InMemoryLog, glave_ports::InMemoryBlobs, A>,
    thread: &glave_harness::Thread,
    repo: &std::path::Path,
    root: &std::path::Path,
) {
    sh(
        repo,
        &format!("git worktree remove --force '{}'", root.display()),
    );
    let removed = WorkspaceRemoved {
        root: root.to_string_lossy().into_owned(),
        branch: None,
        reason: RemovedReason::Pruned,
        trash: None,
    };
    let now = SystemClock.now();
    record_removed(harness.log(), &scope(), thread.id, now, [1; 10], &removed)
        .await
        .unwrap();
}

#[test]
fn someone_else_s_worktree_made_again_gets_none_of_the_person_s_secrets_or_sign_ins() {
    if !glave_computer::sandbox_available() {
        assert!(std::env::var_os("GLAVE_REQUIRE_SANDBOX").is_none());
        return;
    }
    let base = TempDir::outside_temp("walled-again").unwrap();
    let repo = base.join("app");
    std::fs::create_dir_all(&repo).unwrap();
    sh(
        &repo,
        "git init -q --initial-branch=main && echo hi > a.txt && git add a.txt && \
         git -c user.name=t -c user.email=t@example.com commit -q -m one",
    );
    let worktrees = base.join("worktrees");
    block_on!(async {
        let workspace = persons_workspace(&base, &repo, &worktrees).await;
        // Whether each reached it, never what it is.
        let command = "[ -n \"$STRIPE_KEY\" ] && echo has-key || echo no-key; \
             [ -n \"$GIT_CONFIG_COUNT\" ] && echo has-helper || echo no-helper";
        let answers = vec![
            answer(Some("One."), vec![], StopReason::EndTurn),
            answer(
                None,
                vec![("bash", json!({"command": command}))],
                StopReason::ToolUse,
            ),
            answer(Some("Done."), vec![], StopReason::EndTurn),
        ];
        let (harness, _) = harness_with(
            workspace,
            answers,
            AllowAll,
            InMemoryLog::new(),
            1,
            in_the_sandbox(),
        );
        let someone = glave_event::PrincipalId::new(jiff::Timestamp::UNIX_EPOCH, [9; 10]);
        let grace = Caller {
            person: Some(someone.unwrap()),
            ..Caller::default()
        };
        let place = Place::Worktree(repo.to_string_lossy().into_owned());
        let mut thread = harness
            .start_thread_in(scope(), &grace, place)
            .await
            .unwrap();
        let cancel = glave_harness::Cancel::new();
        harness
            .run_turn_as(&mut thread, "hi", &grace, &cancel, &mut |_| {})
            .await
            .unwrap();
        let root = harness.works_in(&thread).unwrap().root().to_owned();
        assert!(root.join("a.txt").exists());

        removed_outside(&harness, &thread, &repo, &root).await;

        harness
            .run_turn_as(&mut thread, "look", &grace, &cancel, &mut |_| {})
            .await
            .unwrap();
        assert!(root.join("a.txt").exists(), "made again");
        let said = results(thread.messages()).join("\n");
        assert!(said.contains("-key"), "the command ran: {said}");
        assert!(said.contains("no-key"), "no secret of the person's: {said}");
        assert!(
            said.contains("no-helper"),
            "no sign-in of the person's: {said}"
        );
    });
}

/// Someone else's commands on the person's computer share no package cache with the person's
/// (#1469): they read nothing the person's commands kept there, and nothing they leave is
/// found by the person's next command, which would run it as the person.
#[test]
fn someone_else_s_commands_share_no_cache_with_the_person_s() {
    if !glave_computer::sandbox_available() {
        assert!(std::env::var_os("GLAVE_REQUIRE_SANDBOX").is_none());
        return;
    }
    let base = TempDir::outside_temp("walled-caches").unwrap();
    let repo = base.join("app");
    std::fs::create_dir_all(&repo).unwrap();
    sh(&repo, "git init -q --initial-branch=main");
    let worktrees = base.join("worktrees");
    let glave_home = base.join(".glave");
    std::fs::create_dir_all(&glave_home).unwrap();
    let kept = std::fs::canonicalize(&glave_home)
        .unwrap()
        .join("sandbox-cache/xdg");
    let kept = kept.display();
    let runs = |command: String| {
        answer(
            None,
            vec![("bash", json!({ "command": command }))],
            StopReason::ToolUse,
        )
    };
    let answers = vec![
        runs(format!(
            "mkdir -p \"$XDG_CACHE_HOME\" && echo fetched > \"$XDG_CACHE_HOME/kept\" && \
             [ \"$XDG_CACHE_HOME\" = '{kept}' ] && echo kept-there"
        )),
        answer(Some("Kept."), vec![], StopReason::EndTurn),
        runs(format!(
            "cat '{kept}/kept' 2>/dev/null || echo read-nothing; \
             mkdir -p \"$XDG_CACHE_HOME\" && echo planted > \"$XDG_CACHE_HOME/tool\"; \
             echo planted > '{kept}/tool' 2>/dev/null || echo wrote-nothing"
        )),
        answer(Some("Tried."), vec![], StopReason::EndTurn),
        runs(format!(
            "cat '{kept}/tool' 2>/dev/null || echo found-nothing"
        )),
        answer(Some("Looked."), vec![], StopReason::EndTurn),
    ];
    block_on!(async {
        let workspace = persons_workspace(&base, &repo, &worktrees).await;
        let (harness, _) = harness_with(
            workspace,
            answers,
            AllowAll,
            InMemoryLog::new(),
            1,
            in_the_sandbox(),
        );
        let person = Caller::default();
        let someone = glave_event::PrincipalId::new(jiff::Timestamp::UNIX_EPOCH, [9; 10]);
        let grace = Caller {
            person: Some(someone.unwrap()),
            ..Caller::default()
        };
        let place = || Place::Folder(repo.to_string_lossy().into_owned());
        let cancel = glave_harness::Cancel::new();
        let mut said = Vec::new();
        for (caller, asked) in [(&person, "keep"), (&grace, "plant"), (&person, "look")] {
            let mut thread = harness
                .start_thread_in(scope(), caller, place())
                .await
                .unwrap();
            harness
                .run_turn_as(&mut thread, asked, caller, &cancel, &mut |_| {})
                .await
                .unwrap();
            said.push(results(thread.messages()).join("\n"));
        }
        assert!(
            said[0].contains("kept-there"),
            "the person's cache: {}",
            said[0]
        );
        assert!(said[1].contains("read-nothing"), "{}", said[1]);
        assert!(said[1].contains("wrote-nothing"), "{}", said[1]);
        assert!(said[2].contains("found-nothing"), "{}", said[2]);
    });
}
