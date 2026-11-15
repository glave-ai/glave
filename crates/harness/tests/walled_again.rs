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
        let environment = std::env::vars_os()
            .filter(|(name, _)| name == "PATH" || name == "HOME")
            .collect::<Vec<_>>();
        let (computers, info) = LocalComputers::for_here(environment)
            .with_worktrees(&worktrees)
            .open_here(&repo)
            .await
            .unwrap();
        let sandbox = Sandbox {
            level: SandboxLevel::Project,
            network: SandboxNetwork::when(false),
            home: Some(base.to_path_buf()),
            glave_home: Some(base.join(".glave")),
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
        let workspace = Workspace::new(computers, info)
            .with_sandbox(&sandbox)
            .await
            .unwrap()
            .hiding(known)
            .with_credentials(fixed);
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

        // A cleanup in another glave removes it and records so.
        let removing = format!("git worktree remove --force '{}'", root.display());
        sh(&repo, &removing);
        let removed = WorkspaceRemoved {
            root: root.to_string_lossy().into_owned(),
            branch: None,
            reason: RemovedReason::Pruned,
            trash: None,
        };
        record_removed(
            harness.log(),
            &scope(),
            thread.id,
            SystemClock.now(),
            [1; 10],
            &removed,
        )
        .await
        .unwrap();

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
