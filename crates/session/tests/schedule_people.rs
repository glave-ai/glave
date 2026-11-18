//! Everyone's schedules, whoever made them: a run acts as its maker, with what they may do and
//! spend, never as the person on the server's computer; a server started since fires them though
//! their maker asked it nothing, from the store's index of where schedules are, each
//! organization's in its own scope; a maker removed from the organization meanwhile has their run
//! refused, and their waiting run not adopted, as is one whose people can't be read to tell; and
//! schedules are offered only where a server fires them.

use glave_event::{AuthMethod, Body, PersonRemoved, Principal, Skipped, ThreadId, decode};
use glave_ports::stream::Reserved;
use glave_ports::testing::{FaultyLog, ManualClock};
use std::sync::Arc;

use glave_ports::{
    Caller, EventLog as _, InMemoryLog, InMemoryProjects, OrgId, ProjectId, Projects, Scope,
};
use glave_session::protocol::{Reply, Request, ScheduleInfo, code};

use crate::schedule_firing::{append, daily, ledger_of, made_by, on_a_store, runs_in, set, setup};
use crate::support::*;

#[test]
fn a_server_started_since_fires_everyone_s_schedules_each_as_whoever_made_it() {
    block_on!(async {
        let log = Shared::default();
        // One store: its log, and its projects.
        let (server, projects, now, repo, _dir) = on_a_store("people-fire", &log).await;
        let grace = elsewhere();
        let theirs = daily(&server, &grace).await.unwrap();
        let mine = daily(&server, &Caller::default()).await.unwrap();
        let own = server.person().unwrap();
        server.shutdown().await;
        drop(server);
        // Neither has asked the new server anything; its person is the one on this computer.
        let mut later = setup();
        later.clock = now.clone();
        later.settings.person = Some(own);
        later.projects = Some(projects);
        let (restarted, _) = later.build_on(&repo, log.clone()).await;
        set(&now, "2026-10-10T11:00:00Z");
        let mut fired = restarted.tick().await.unwrap();
        settle(&restarted).await;
        settle_for(&restarted, &grace).await;
        fired.sort();
        let mut chats: Vec<ThreadId> = [&theirs, &mine]
            .iter()
            .filter_map(|made| made.thread)
            .collect();
        chats.sort();
        assert_eq!(fired, chats, "both, though neither asked it anything");
        let grace_id = grace.person.unwrap();
        assert_eq!(
            runs_in(&log, theirs.project).await,
            vec![(Some(Principal::person(grace_id)), theirs.schedule)],
            "as her"
        );
        assert_eq!(
            runs_in(&log, mine.project).await,
            vec![(Some(Principal::person(own)), mine.schedule)],
            "as the person on this computer, named"
        );
        // Her run is signed in by no one, never in the form of the person on this computer, so
        // nothing only they may do is hers.
        let hers = theirs.thread.unwrap();
        let chat = Scope::glaves_own(scope().org(), theirs.project);
        let auth = (log
            .read(&chat, hers, None, usize::MAX)
            .await
            .unwrap()
            .into_iter())
        .find_map(|raw| match decode(raw).unwrap().body {
            Body::TurnStarted(started) => Some(started.auth),
            _ => None,
        })
        .flatten()
        .unwrap();
        assert_eq!(auth.method, AuthMethod::OnBehalf);
        let run = Caller {
            person: grace.person,
            auth: Some(auth),
            org: Some(scope().org()),
            ..Caller::default()
        };
        let only_here = [
            Request::FolderList { path: None },
            Request::SearchSet { on: false },
        ];
        for asked in only_here {
            let refused = restarted.request_as(&run, asked.clone()).await;
            assert_eq!(
                refused.map_err(|error| error.code).err(),
                Some(code::NOT_ALLOWED),
                "{asked:?}"
            );
        }
    });
}

#[test]
fn where_schedules_are_is_kept_in_each_organization_s_own_scope_while_they_are() {
    block_on!(async {
        let log = Shared::default();
        let (server, _, _, _dir) = setup()
            .build_in_repository("people-index", log.clone())
            .await;
        let third = OrgId::from_uuid(uuid::Uuid::from_u128(3));
        let grace = Caller {
            org: Some(third),
            ..elsewhere()
        };
        let hers = daily(&server, &grace).await.unwrap();
        let index = |org| index_of(&log, org);
        let own = index(scope().org()).await;
        assert!(
            own.is_empty(),
            "the server's own lists no one else's: {own:?}"
        );
        let theirs = index(third).await;
        assert!(
            matches!(theirs.as_slice(), [Body::SchedulePlaced(placed)] if placed.project == hers.project.as_uuid()),
            "her project, in her organization's own: {theirs:?}"
        );
        // Her last schedule there gone, the index lists her project no more.
        let deleted = Request::ScheduleDelete {
            schedule: hers.schedule,
        };
        server.request_as(&grace, deleted).await.unwrap();
        let theirs = index(third).await;
        assert!(
            matches!(theirs.as_slice(), [Body::SchedulePlaced(_), Body::ScheduleUnplaced(gone)] if gone.project == hers.project.as_uuid()),
            "unlisted as its last schedule went: {theirs:?}"
        );
    });
}

/// What `org`'s index of where schedules are holds, in order.
async fn index_of(log: &Shared, org: OrgId) -> Vec<Body> {
    let scope = Scope::of_organization(org);
    let read = log.read(&scope, Reserved::Scheduled.id(), None, usize::MAX);
    (read.await.unwrap().into_iter())
        .map(|raw| decode(raw).unwrap().body)
        .collect()
}

#[test]
fn a_schedule_made_as_its_project_s_last_one_goes_lists_the_project_again() {
    block_on!(async {
        let log = Shared::default();
        let faulty = FaultyLog::over(Arc::clone(&log.0));
        let (server, _, _, _dir) = setup()
            .build_in_repository("people-relisted", faulty.clone())
            .await;
        let grace = elsewhere();
        let first = daily(&server, &grace).await.unwrap();
        // Her last schedule there is deleted, and its project's removal from the index held as
        // it is written, while she makes another there, which finds the project still listed.
        faulty.hold("schedule.unplaced");
        let deleted = Request::ScheduleDelete {
            schedule: first.schedule,
        };
        let deleting = server.request_as(&grace, deleted);
        let making = async {
            faulty.held().await;
            let made = daily(&server, &grace).await.unwrap();
            faulty.release();
            made
        };
        let (deleted, made) = tokio::join!(deleting, making);
        deleted.unwrap();
        assert_eq!(made.project, first.project);
        let listed = index_of(&log, scope().org()).await;
        assert!(
            matches!(listed.as_slice(), [.., Body::ScheduleUnplaced(_), Body::SchedulePlaced(again)] if again.project == first.project.as_uuid()),
            "listed again once the removal landed: {listed:?}"
        );
    });
}

#[test]
fn a_run_of_someone_removed_meanwhile_is_refused() {
    block_on!(async {
        let log = Shared::default();
        let first = setup();
        let now = first.clock.clone();
        let (server, _, _, _dir) = first
            .build_in_repository("people-removed", log.clone())
            .await;
        let grace = elsewhere();
        let theirs = daily(&server, &grace).await.unwrap();
        // Removed on another server, which has yet to delete her schedules.
        removed_elsewhere(&log, &grace).await;
        set(&now, "2026-10-10T11:00:00Z");
        assert_eq!(server.tick().await.unwrap(), Vec::new());
        assert_eq!(runs_in(&log, theirs.project).await, Vec::new());
        let refused = refusal_in(&log, theirs.project).await;
        assert_eq!(refused, "MAKER_REMOVED");
    });
}

#[test]
fn a_run_of_someone_removed_is_refused_though_the_server_forgets_people_ledgers_meanwhile() {
    block_on!(async {
        let log = Shared::default();
        let faulty = FaultyLog::over(Arc::clone(&log.0));
        let first = setup();
        let now = first.clock.clone();
        let (server, _, _, _dir) = first
            .build_in_repository("people-forgotten", faulty.clone())
            .await;
        let grace = elsewhere();
        let theirs = daily(&server, &grace).await.unwrap();
        // Her organization's people ledger runs past one page of a read: 500 others leave, then
        // she is removed, on its second.
        let people = Scope::of_organization(scope().org());
        let ledger = Reserved::People.id();
        let before = log.read(&people, ledger, None, usize::MAX).await.unwrap();
        let next = u64::try_from(before.len()).unwrap();
        for n in 1..=500u64 {
            let left = PersonRemoved {
                person: id(&format!("other-{n}")),
                by: Principal::person(id("ada")),
            };
            append(&log, &people, ledger, next + n, &left).await;
        }
        removed_elsewhere(&log, &grace).await;
        // The tick's read of who is there is held at its second page while this server reads
        // more organizations' people than it keeps, forgetting hers.
        faulty.hold_read("person.removed");
        set(&now, "2026-10-10T11:00:00Z");
        let forgetting = async {
            faulty.held().await;
            for n in 1..=300u128 {
                let org = uuid::Uuid::from_u128(0x0192_f5c4_0000_7000_8000_0000_0000_0000 + n);
                let caller = Caller {
                    org: Some(OrgId::from_uuid(org)),
                    ..signed_in("ada")
                };
                // Refused or not, it read that organization's people.
                let _ = server.request_as(&caller, Request::Discover {}).await;
            }
            faulty.release();
        };
        let (ticked, ()) = tokio::join!(server.tick(), forgetting);
        assert_eq!(ticked.unwrap(), Vec::new());
        assert_eq!(runs_in(&log, theirs.project).await, Vec::new());
        let refused = refusal_in(&log, theirs.project).await;
        assert_eq!(
            refused, "MAKER_REMOVED",
            "read whole, apart from what it forgot"
        );
    });
}

/// Records on `log` that `caller` was removed from the organization, as another server would.
async fn removed_elsewhere(log: &Shared, caller: &Caller) {
    let people = Scope::of_organization(scope().org());
    let ledger = Reserved::People.id();
    let before = log.read(&people, ledger, None, usize::MAX).await.unwrap();
    let next = u64::try_from(before.len()).unwrap() + 1;
    let removed = PersonRemoved {
        person: caller.person.unwrap(),
        by: Principal::person(id("ada")),
    };
    append(log, &people, ledger, next, &removed).await;
}

/// Why `project`'s latest run was refused, as its ledger records it: its reason.
pub(crate) async fn refusal_in(log: &Shared, project: ProjectId) -> String {
    let refused = (ledger_of(log, project).await.into_iter().rev()).find_map(|body| match body {
        Body::ScheduleSkipped(skipped) => Some(skipped.why),
        _ => None,
    });
    match refused {
        Some(Skipped::Refused { reason }) => reason,
        other => panic!("a refusal, not {other:?}"),
    }
}

#[test]
fn a_run_whose_maker_can_t_be_read_still_there_is_held_back_and_recorded_nowhere() {
    block_on!(async {
        let log = Shared::default();
        let faulty = FaultyLog::over(Arc::clone(&log.0));
        let first = setup();
        let now = first.clock.clone();
        let (server, _, _, _dir) = first
            .build_in_repository("people-unread", faulty.clone())
            .await;
        let grace = elsewhere();
        let theirs = daily(&server, &grace).await.unwrap();
        removed_elsewhere(&log, &grace).await;
        faulty.fail_with("person.removed");
        set(&now, "2026-10-10T11:00:00Z");
        assert_eq!(server.tick().await.unwrap(), Vec::new());
        assert_eq!(
            ledger_of(&log, theirs.project).await.len(),
            1,
            "its making alone: the run was neither run, claimed nor refused"
        );
        // Read at its next time, she is found removed: refused, never run as her.
        set(&now, "2026-10-11T11:00:00Z");
        assert_eq!(server.tick().await.unwrap(), Vec::new());
        assert_eq!(runs_in(&log, theirs.project).await, Vec::new());
        let refused = refusal_in(&log, theirs.project).await;
        assert_eq!(refused, "MAKER_REMOVED");
    });
}

#[test]
fn a_run_of_someone_the_people_ledger_does_not_list_is_held_back() {
    block_on!(async {
        let log = Shared::default();
        let first = setup();
        let now = first.clock.clone();
        let (server, _, _, _dir) = first
            .build_in_repository("people-unlisted", log.clone())
            .await;
        // A schedule of someone no person.joined names, as a ledger this server forgot meanwhile
        // reads: never taken for still there.
        let chat = start(&server, false).await;
        let created = made_by(0x42, chat, Some(Principal::person(id("nobody"))));
        append(&log, &scope(), Reserved::Tasks.id(), 1, &created).await;
        set(&now, "2026-10-10T11:00:00Z");
        assert_eq!(server.tick().await.unwrap(), Vec::new());
        assert_eq!(
            ledger_of(&log, scope().project()).await.len(),
            1,
            "its making alone: neither run, claimed nor refused"
        );
    });
}

/// A run of `grace`'s that waits for her, as her run's write asks, left open by its server,
/// which stopped: the log, the projects and the folder the next server works on, its clock,
/// and the schedule.
async fn left_waiting(
    name: &str,
    grace: &Caller,
) -> (
    Shared,
    Arc<dyn Projects>,
    std::path::PathBuf,
    glave_ports::testing::TempDir,
    ManualClock,
    ScheduleInfo,
) {
    let log = Shared::default();
    let projects: Arc<dyn Projects> = Arc::new(InMemoryProjects::new());
    let write = serde_json::json!({"path": "note.md", "content": "hi"});
    let mut first = Setup::new(vec![answer(None, Some(("write", write)))]);
    first.clock = setup().clock;
    first.projects = Some(Arc::clone(&projects));
    let now = first.clock.clone();
    let (server, _, repo, dir) = first.build_in_repository(name, log.clone()).await;
    let theirs = daily(&server, grace).await.unwrap();
    set(&now, "2026-10-10T11:00:00Z");
    assert_eq!(server.tick().await.unwrap(), vec![theirs.thread.unwrap()]);
    asked_in_as(&server, grace, theirs.thread.unwrap()).await;
    server.shutdown().await;
    drop(server);
    (log, projects, repo, dir, now, theirs)
}

/// The server that starts next on what [`left_waiting`] left, through `log`: how many tasks
/// left open it recorded interrupted.
async fn recovered_by<L: glave_ports::EventLog + 'static>(
    log: L,
    projects: Arc<dyn Projects>,
    repo: &std::path::Path,
    now: &ManualClock,
) -> usize {
    let mut later = setup();
    later.clock = now.clone();
    later.projects = Some(projects);
    later.worktrees = repo.parent().map(|dir| dir.join("worktrees"));
    let (restarted, _) = later.build_on(repo, log).await;
    restarted.recover_tasks().await.unwrap().closed
}

#[test]
fn a_waiting_run_of_someone_removed_is_not_adopted() {
    block_on!(async {
        let grace = elsewhere();
        let (log, projects, repo, _dir, now, _) =
            left_waiting("people-adopt-removed", &grace).await;
        removed_elsewhere(&log, &grace).await;
        assert_eq!(
            recovered_by(log, projects, &repo, &now).await,
            1,
            "recorded interrupted"
        );
    });
}

#[test]
fn a_waiting_run_whose_maker_can_t_be_read_still_there_is_not_adopted() {
    block_on!(async {
        let grace = elsewhere();
        let (log, projects, repo, _dir, now, _) = left_waiting("people-adopt-unread", &grace).await;
        let faulty = FaultyLog::over(Arc::clone(&log.0));
        faulty.fail_with("person.joined");
        assert_eq!(recovered_by(faulty, projects, &repo, &now).await, 1);
    });
}

#[test]
fn schedules_are_offered_only_where_a_server_fires_them() {
    block_on!(async {
        let mut setup = setup();
        setup.schedules = false;
        let (server, _, _, _dir) = setup
            .build_in_repository("people-none", InMemoryLog::new())
            .await;
        let Reply::Server(info) = server.request(Request::Discover {}).await.unwrap() else {
            panic!("server info");
        };
        assert!(!info.schedules);
        assert_eq!(
            daily(&server, &Caller::default()).await,
            Err(code::NOT_AVAILABLE)
        );
        let (here, _, _, _dir) = self::setup()
            .build_in_repository("people-here", InMemoryLog::new())
            .await;
        let Reply::Server(info) = here.request(Request::Discover {}).await.unwrap() else {
            panic!("server info");
        };
        assert!(info.schedules);
    });
}

#[test]
fn removing_someone_deletes_every_schedule_of_theirs_though_a_tick_forgets_ledgers_meanwhile() {
    block_on!(async {
        let log = Shared::default();
        let faulty = FaultyLog::over(Arc::clone(&log.0));
        let (server, _, _, _dir) = setup()
            .build_in_repository("people-remove-race", faulty.clone())
            .await;
        let grace = elsewhere();
        let first = daily(&server, &grace).await.unwrap();
        let chat = first.thread.unwrap();
        // Her project's ledger runs past one page of a read, its last on the second.
        let ledger = Scope::glaves_own(scope().org(), first.project);
        for n in 2..=500u64 {
            let created = made_by(n.into(), chat, grace.person.map(Principal::person));
            append(&log, &ledger, Reserved::Tasks.id(), n, &created).await;
        }
        let paused = glave_event::SchedulePaused {
            schedule: first.schedule,
            paused: true,
            by: None,
        };
        append(&log, &ledger, Reserved::Tasks.id(), 501, &paused).await;
        // More projects with schedules than a server keeps ledgers of, which a tick reads.
        let index = (
            Scope::of_organization(scope().org()),
            Reserved::Scheduled.id(),
        );
        let listed = log.read(&index.0, index.1, None, usize::MAX).await.unwrap();
        let next = u64::try_from(listed.len()).unwrap();
        for n in 1..=1_030u64 {
            let project =
                uuid::Uuid::from_u128(0x0192_f5c3_0000_7000_8000_0000_0000_0000 + u128::from(n));
            let placed = glave_event::SchedulePlaced { project };
            append(&log, &index.0, index.1, next + n, &placed).await;
        }
        // The removal's read of her ledger is held at its second page while a tick forgets
        // every ledger this server kept.
        faulty.hold_read("schedule.paused");
        let removal = server.request(Request::PersonRemove {
            person: grace.person.unwrap(),
        });
        let ticking = async {
            faulty.held().await;
            let ticked = server.tick().await;
            faulty.release();
            ticked
        };
        let (removed, ticked) = tokio::join!(removal, ticking);
        removed.unwrap();
        ticked.unwrap();
        let deleted = (ledger_of(&log, first.project).await.into_iter())
            .filter(|body| matches!(body, Body::ScheduleDeleted(_)))
            .count();
        assert_eq!(deleted, 500, "every schedule of hers, read whole");
    });
}
