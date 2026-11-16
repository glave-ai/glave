//! Someone else's worktree on the harness's computer, removed by a cleanup while the harness
//! holds their thread, is made again walled as it was (`specs/sandbox.md`, "Whose computer"):
//! its commands still get none of the person's secrets or git sign-ins.

use std::process::Command;

use glave_computer::LocalComputers;
use glave_event::{Forge, RemovedReason, Secret, StopReason, WorkspaceRemoved};
use glave_harness::{AllowAll, Place, Workspace, record_removed};
use glave_ports::computer::{CredentialHost, GitCredential, Sandbox, SandboxLevel, SandboxNetwork};
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
        apart: false,
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

/// Whether the sandbox runs here, as it must where `GLAVE_REQUIRE_SANDBOX` says.
fn sandboxed_here() -> bool {
    let available = glave_computer::sandbox_available();
    assert!(available || std::env::var_os("GLAVE_REQUIRE_SANDBOX").is_none());
    available
}

/// A repository with one commit, in `base/app`.
fn repository(base: &std::path::Path) -> std::path::PathBuf {
    let repo = base.join("app");
    std::fs::create_dir_all(&repo).unwrap();
    sh(
        &repo,
        "git init -q --initial-branch=main && echo hi > a.txt && git add a.txt && \
         git -c user.name=t -c user.email=t@example.com commit -q -m one",
    );
    repo
}

/// Someone other than the person the harness works for.
fn someone_else() -> Caller {
    let someone = glave_event::PrincipalId::new(jiff::Timestamp::UNIX_EPOCH, [9; 10]);
    Caller {
        person: Some(someone.unwrap()),
        ..Caller::default()
    }
}

/// A thread of `caller`'s in a worktree of `repo`, after a first turn.
async fn said_hi<A: glave_harness::Approver>(
    harness: &glave_harness::Harness<ScriptedModel, InMemoryLog, glave_ports::InMemoryBlobs, A>,
    caller: &Caller,
    repo: &std::path::Path,
    cancel: &glave_harness::Cancel,
) -> glave_harness::Thread {
    let place = Place::Worktree(repo.to_string_lossy().into_owned());
    let mut thread = harness
        .start_thread_in(scope(), caller, place)
        .await
        .unwrap();
    harness
        .run_turn_as(&mut thread, "hi", caller, cancel, &mut |_| {})
        .await
        .unwrap();
    thread
}

/// Someone else's commands on the harness's computer share no package cache with its person's
/// (`specs/sandbox.md`, "Whose computer"): they read nothing the person's commands keep there,
/// and what theirs leave in a cache, a tool to run later, the person's never find in their
/// own, whether left through the cache they are given or written where the person's is.
#[test]
fn someone_else_s_commands_share_no_cache_with_the_person_s() {
    if !sandboxed_here() {
        return;
    }
    let base = TempDir::outside_temp("walled-cache").unwrap();
    let repo = repository(&base);
    let worktrees = base.join("worktrees");
    // glave's folder is there, as on any computer glave runs on: its sandbox cache is kept in it.
    std::fs::create_dir_all(base.join(".glave")).unwrap();
    block_on!(async {
        let workspace = persons_workspace(&base, &repo, &worktrees).await;
        let keep = "mkdir -p \"$XDG_CACHE_HOME\" && echo kept-private > \"$XDG_CACHE_HOME/own\"";
        // Theirs reads the person's cache, then leaves a tool there and in the one it is given.
        let cache = std::fs::canonicalize(base.join(".glave")).unwrap();
        let plant = format!(
            "find '{cache}' -type f 2>/dev/null | xargs cat 2>/dev/null; \
             for kept in '{cache}'/*/; do \
             echo planted > \"$kept/tool\" 2>/dev/null && echo wrote-theirs; done; \
             mkdir -p \"$XDG_CACHE_HOME\" && echo planted > \"$XDG_CACHE_HOME/tool\" && \
             echo own-cache-works",
            cache = cache.join("sandbox-cache").display()
        );
        let look = "cat \"$XDG_CACHE_HOME/tool\" 2>/dev/null || echo nothing-there";
        let ran = |command: &str| {
            let call = vec![("bash", json!({"command": command}))];
            [
                answer(None, call, StopReason::ToolUse),
                answer(Some("Done."), vec![], StopReason::EndTurn),
            ]
        };
        let answers = [ran(keep), ran(&plant), ran(look)];
        let answers = answers.into_iter().flatten().collect();
        let (harness, _) = harness_with(
            workspace,
            answers,
            AllowAll,
            InMemoryLog::new(),
            1,
            in_the_sandbox(),
        );
        let cancel = glave_harness::Cancel::new();
        let person = Caller::default();
        said_hi(&harness, &person, &repo, &cancel).await;
        let theirs = said_hi(&harness, &someone_else(), &repo, &cancel).await;
        let said = results(theirs.messages()).join("\n");
        assert!(said.contains("own-cache-works"), "{said}");
        assert!(!said.contains("kept-private"), "they read it: {said}");
        assert!(!said.contains("wrote-theirs"), "they wrote it: {said}");
        let own = said_hi(&harness, &person, &repo, &cancel).await;
        let said = results(own.messages()).join("\n");
        assert!(
            said.contains("nothing-there"),
            "the person's command found what someone else's left: {said}"
        );
    });
}

#[test]
fn someone_else_s_worktree_made_again_gets_none_of_the_person_s_secrets_or_sign_ins() {
    if !sandboxed_here() {
        return;
    }
    let base = TempDir::outside_temp("walled-again").unwrap();
    let repo = repository(&base);
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
        let grace = someone_else();
        let cancel = glave_harness::Cancel::new();
        let mut thread = said_hi(&harness, &grace, &repo, &cancel).await;
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
