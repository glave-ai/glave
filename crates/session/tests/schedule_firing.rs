//! Firing every run once: past the ledgers a server keeps in memory, every project's runs are
//! still looked at, each found right after its ledger is read; two servers on one store that
//! both believe they fire start a run once, the one whose claim lands; a schedule made by no
//! one glave can act for is refused and recorded; and a run is never someone else's in more than
//! their own sign-in would be: no sign-in of the owner's, and no Full access, fired or adopted.

use std::sync::Arc;

use glave_event::{
    Body, Forge, Principal, ScheduleCreated, ScheduleId, ScheduleOrganization, SchedulePlaced,
    ThreadId, When, decode,
};
use glave_ports::computer::{CredentialHost, GitCredential};
use glave_ports::stream::Reserved;
use glave_ports::testing::{FaultyLog, FixedGitCredentials, any_secret};
use glave_ports::{InMemoryProjects, ProjectId, Projects, Scope};
use glave_session::protocol::{Answer, Request};
use serde_json::json;

use crate::schedule_people::{append, daily, ledger_of, refusal_in, runs_in, set, setup};
use crate::support::*;

/// The `n`th id of the kind `kind` stands for, a UUID v7.
fn v7(kind: u128, n: u128) -> uuid::Uuid {
    uuid::Uuid::from_u128(0x0192_f5c0_0000_7000_8000_0000_0000_0000 + (kind << 80) + n)
}

/// A chat no one made: the `n`th.
fn missing_chat(n: u128) -> ThreadId {
    ThreadId::try_from(v7(4, n)).unwrap()
}

/// The `n`th schedule, made at 10:00, that runs at 11:00 every day into `thread`, made by `by`.
fn made_by(n: u128, thread: ThreadId, by: Option<Principal>) -> ScheduleCreated {
    ScheduleCreated {
        schedule: ScheduleId::try_from(v7(1, n)).unwrap(),
        name: "Daily check".into(),
        when: When::Cron("0 11 * * *".into()),
        zone: "UTC".into(),
        prompt: "check the build".into(),
        budget: 5_000,
        thread: Some(thread),
        from: None,
        by,
    }
}

#[test]
fn past_the_ledgers_a_server_keeps_every_project_s_runs_are_still_looked_at() {
    block_on!(async {
        let log = Shared::default();
        let first = setup();
        let now = first.clock.clone();
        let (server, _, _, _dir) = first.build_in_repository("firing-many", log.clone()).await;
        // More projects with schedules than a server keeps ledgers of, written straight into
        // the store, each schedule's chat missing so each run is refused and recorded.
        let org = scope().org();
        let (index, ledger) = (Reserved::Scheduled.id(), Reserved::Tasks.id());
        let listed = ScheduleOrganization { org: org.as_uuid() };
        append(&log, &Scope::of_organization(org), index, 1, &listed).await;
        let projects: Vec<ProjectId> = (0..1_030).map(|n| ProjectId::from_uuid(v7(3, n))).collect();
        for (n, project) in (0u64..).zip(&projects) {
            let placed = SchedulePlaced {
                project: project.as_uuid(),
            };
            append(&log, &Scope::of_organization(org), index, n + 2, &placed).await;
            let created = made_by(n.into(), missing_chat(n.into()), None);
            append(&log, &Scope::glaves_own(org, *project), ledger, 1, &created).await;
        }
        set(&now, "2026-10-10T11:00:00Z");
        assert_eq!(server.tick().await.unwrap(), Vec::new());
        for project in &projects {
            let refused = refusal_in(&log, *project).await;
            assert!(!refused.is_empty(), "{project}'s run was never looked at");
        }
    });
}

#[test]
fn two_servers_that_both_fire_start_someone_else_s_run_once() {
    block_on!(async {
        let log = Shared::default();
        let faulty = FaultyLog::over(Arc::clone(&log.0));
        // One store: its log, and its projects; each server keeps leases of its own, so both
        // believe they fire the organization's schedules.
        let projects: Arc<dyn Projects> = Arc::new(InMemoryProjects::new());
        let mut first = setup();
        first.projects = Some(Arc::clone(&projects));
        let now = first.clock.clone();
        let (one, _, repo, _dir) = first
            .build_in_repository("firing-twice", faulty.clone())
            .await;
        let grace = elsewhere();
        let theirs = daily(&one, &grace).await.unwrap();
        let mut second = setup();
        second.clock = now.clone();
        second.projects = Some(projects);
        second.seed = 2;
        let (two, _) = second.build_on(&repo, log.clone()).await;
        set(&now, "2026-10-10T11:00:00Z");
        // The first's claim is held as it is written, while the second ticks through.
        faulty.hold("schedule.fired");
        let racing = async {
            faulty.held().await;
            let fired = two.tick().await;
            faulty.release();
            fired
        };
        let (from_one, from_two) = tokio::join!(one.tick(), racing);
        let mut started = from_one.unwrap();
        started.extend(from_two.unwrap());
        assert_eq!(started, vec![theirs.thread.unwrap()], "started once");
        settle_for(&one, &grace).await;
        settle_for(&two, &grace).await;
        assert_eq!(runs_in(&log, theirs.project).await.len(), 1);
        let claims = (ledger_of(&log, theirs.project).await.into_iter())
            .filter(|body| matches!(body, Body::ScheduleFired(_)))
            .count();
        assert_eq!(
            claims, 1,
            "one claim landed; the other read it and wrote none"
        );
    });
}

#[test]
fn a_schedule_made_by_no_one_glave_can_act_for_is_refused_and_recorded() {
    block_on!(async {
        let log = Shared::default();
        let first = setup();
        let now = first.clock.clone();
        let (server, _, _, _dir) = first
            .build_in_repository("firing-no-one", log.clone())
            .await;
        let project = scope().project();
        let created = made_by(1, missing_chat(1), Some(Principal::glave()));
        append(&log, &scope(), Reserved::Tasks.id(), 1, &created).await;
        set(&now, "2026-10-10T11:00:00Z");
        assert_eq!(server.tick().await.unwrap(), Vec::new());
        let refused = refusal_in(&log, project).await;
        assert!(refused.contains("can't tell who made it"), "{refused}");
    });
}

/// Whether the model was last asked with the owner's GitHub said signed in.
fn told_signed_in(seen: &Seen) -> bool {
    let asked = seen.lock().unwrap();
    let system = &asked.last().unwrap().system;
    serde_json::to_string(system)
        .unwrap()
        .contains("`gh` is signed in for github.com")
}

/// How many times `thread` asked the person to approve something.
async fn asked<L: glave_ports::EventLog + 'static>(server: &Tested<L>, thread: ThreadId) -> usize {
    (events_as(server, &elsewhere(), thread).await.into_iter())
        .filter(|raw| {
            matches!(
                decode(raw.clone()).unwrap().body,
                Body::ApprovalRequested(_)
            )
        })
        .count()
}

#[test]
fn someone_else_s_run_fired_or_adopted_has_none_of_the_owner_s_sign_ins_nor_full_access() {
    block_on!(async {
        let log = Shared::default();
        let projects: Arc<dyn Projects> = Arc::new(InMemoryProjects::new());
        // The owner's GitHub is signed in on the server's computer, whose mode is Full access.
        let owner_s = || {
            let host = CredentialHost {
                url: "https://github.com".into(),
                forge: Forge::Github,
            };
            let credential = GitCredential::new("x-access-token", any_secret()).unwrap();
            Arc::new(FixedGitCredentials::new(vec![host], credential))
        };
        let write =
            |path: &str| answer(None, Some(("write", json!({"path": path, "content": "x"}))));
        let mut first = Setup::new(vec![write("a.md")]);
        first.clock = setup().clock;
        first.projects = Some(Arc::clone(&projects));
        first.credentials = Some(owner_s());
        let now = first.clock.clone();
        let (server, seen, repo, _dir) = first
            .build_in_repository("firing-sandboxed", log.clone())
            .await;
        let grace = elsewhere();
        let chat = daily(&server, &grace).await.unwrap().thread.unwrap();
        set(&now, "2026-10-10T11:00:00Z");
        assert_eq!(server.tick().await.unwrap(), vec![chat]);
        let approval = asked_in_as(&server, &grace, chat).await;
        assert!(
            !told_signed_in(&seen),
            "fired: none of the owner's sign-ins"
        );
        assert_eq!(
            asked(&server, chat).await,
            1,
            "fired: its write asks, as no Full access"
        );
        server.shutdown().await;
        drop(server);
        // The next server adopts the waiting run and goes on with it once she answers.
        let mut later = Setup::new(vec![write("b.md"), answer(Some("Done."), None)]);
        later.clock = now.clone();
        later.projects = Some(projects);
        later.credentials = Some(owner_s());
        later.worktrees = repo.parent().map(|dir| dir.join("worktrees"));
        let (restarted, seen) = later.build_on(&repo, log).await;
        assert_eq!(
            restarted.recover_tasks().await.unwrap().closed,
            0,
            "adopted"
        );
        let decided = Request::ApprovalDecide {
            thread: chat,
            approval,
            decision: Answer::AllowOnce,
        };
        restarted.request_as(&grace, decided).await.unwrap();
        for _ in 0..100_000 {
            if asked(&restarted, chat).await == 2 {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert_eq!(
            asked(&restarted, chat).await,
            2,
            "adopted: its next write asks too"
        );
        assert!(
            !told_signed_in(&seen),
            "adopted: none of the owner's sign-ins"
        );
        restarted.shutdown().await;
    });
}
