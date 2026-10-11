//! Schedules through the session API, on a clock the test moves: one-off and cron schedules
//! fire at the right times and once each; missed runs fire once, never a burst, including in
//! a fresh server; deleting a schedule stops its runs; every run has a budget; and policy
//! limits how many schedules are kept.

#[cfg(test)]
#[path = "support/model.rs"]
#[expect(dead_code, reason = "shared helpers; this file uses some of them")]
mod support;

#[cfg(test)]
#[path = "support/tasks.rs"]
#[expect(dead_code, reason = "shared helpers; this file uses some of them")]
mod helpers;

#[cfg(test)]
mod schedules {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicI64, Ordering};

    use glave_event::{At, ScheduleId, When};
    use glave_session::protocol::{Reply, Request, ScheduleInfo, code};
    use jiff::Timestamp;

    use super::helpers::*;
    use super::support::*;

    fn at(text: &str) -> i64 {
        text.parse::<Timestamp>().unwrap().as_millisecond()
    }

    fn set(now: &Arc<AtomicI64>, text: &str) {
        now.store(at(text), Ordering::SeqCst);
    }

    /// Plenty of short answers: each run takes one.
    fn answers() -> Vec<glave_ports::Completion> {
        (0..20).map(|_| answer(Some("Done."), None)).collect()
    }

    async fn create(server: &Tested, when: When, budget: u64) -> Result<ScheduleInfo, i64> {
        let request = Request::ScheduleCreate {
            when,
            prompt: "check the build".into(),
            goal: false,
            budget,
            worktree: None,
        };
        match server.request(request).await {
            Ok(Reply::Schedule { schedule }) => Ok(schedule),
            Ok(other) => panic!("a schedule, not {other:?}"),
            Err(error) => Err(error.code),
        }
    }

    async fn list(server: &Tested) -> Vec<ScheduleInfo> {
        match server.request(Request::ScheduleList {}).await.unwrap() {
            Reply::Schedules { schedules } => schedules,
            other => panic!("schedules, not {other:?}"),
        }
    }

    async fn tick(server: &Tested) -> usize {
        let fired = server.tick().await.unwrap();
        settle(server).await;
        fired.len()
    }

    fn cron(text: &str) -> When {
        When::Cron(text.into())
    }

    #[test]
    fn a_cron_schedule_fires_when_due_once_each_time_and_missed_runs_fire_once() {
        let dir = folder("schedule-cron");
        block_on(async {
            let setup = Setup::new(answers());
            let now = Arc::clone(&setup.now);
            let (server, _) = setup.build(&dir).await;
            let made = create(&server, cron("*/10 * * * *"), 10_000).await.unwrap();
            assert_eq!(
                made.next,
                Some(At::from(
                    "2026-10-10T10:10:00Z".parse::<Timestamp>().unwrap()
                ))
            );
            assert_eq!(tick(&server).await, 0, "not due at 10:03");
            set(&now, "2026-10-10T10:09:59Z");
            assert_eq!(tick(&server).await, 0);
            set(&now, "2026-10-10T10:10:00Z");
            assert_eq!(tick(&server).await, 1, "due at 10:10");
            assert_eq!(tick(&server).await, 0, "once");
            // 10:20 to 10:50 are missed; one run, never four.
            set(&now, "2026-10-10T10:55:00Z");
            assert_eq!(tick(&server).await, 1);
            assert_eq!(tick(&server).await, 0);
            let listed = list(&server).await;
            assert_eq!(
                listed[0].last.map(|a| a.to_string()).as_deref(),
                Some("2026-10-10T10:50:00.000Z")
            );
            assert_eq!(
                listed[0].next.map(|a| a.to_string()).as_deref(),
                Some("2026-10-10T11:00:00.000Z")
            );
            let ran = tasks(&server).await;
            assert_eq!(ran.len(), 2);
            assert!(ran.iter().all(|task| task.schedule == Some(made.schedule)));
            assert!(ran.iter().all(|task| task.budget == Some(10_000)));
            // Deleted: no more runs.
            server
                .request(Request::ScheduleDelete {
                    schedule: made.schedule,
                })
                .await
                .unwrap();
            set(&now, "2026-10-10T12:00:00Z");
            assert_eq!(tick(&server).await, 0);
            assert_eq!(list(&server).await, Vec::new());
            let again = server
                .request(Request::ScheduleDelete {
                    schedule: made.schedule,
                })
                .await
                .unwrap_err();
            assert_eq!(again.code, code::NO_SCHEDULE);
        });
    }

    #[test]
    fn a_one_off_schedule_fires_once_at_its_time() {
        let dir = folder("schedule-once");
        block_on(async {
            let setup = Setup::new(answers());
            let now = Arc::clone(&setup.now);
            let (server, _) = setup.build(&dir).await;
            let when = When::At(At::from(
                "2026-10-10T11:00:00Z".parse::<Timestamp>().unwrap(),
            ));
            create(&server, when, 5_000).await.unwrap();
            set(&now, "2026-10-10T10:59:59Z");
            assert_eq!(tick(&server).await, 0);
            set(&now, "2026-10-10T11:00:00Z");
            assert_eq!(tick(&server).await, 1);
            set(&now, "2026-10-11T11:00:00Z");
            assert_eq!(tick(&server).await, 0);
            let listed = list(&server).await;
            assert_eq!(listed[0].next, None, "it has run");
            assert_eq!(server.next_due(), None);
        });
    }

    #[test]
    fn a_fresh_server_runs_what_was_missed_while_none_ran_once() {
        let dir = folder("schedule-restart");
        block_on(async {
            let log = Shared::default();
            let mut setup = Setup::new(answers());
            setup.log = log.clone();
            let now = Arc::clone(&setup.now);
            let (server, _) = setup.build(&dir).await;
            create(&server, cron("0 * * * *"), 10_000).await.unwrap();
            server.shutdown().await;
            drop(server);
            // Down for a day: 24 runs missed.
            let mut later = Setup::new(answers());
            later.log = log;
            later.now = Arc::clone(&now);
            set(&now, "2026-10-11T10:30:00Z");
            let (restarted, _) = later.build(&dir).await;
            assert_eq!(restarted.recover_tasks().await.unwrap(), 0);
            assert_eq!(tick(&restarted).await, 1);
            assert_eq!(tick(&restarted).await, 0);
            set(&now, "2026-10-11T11:00:00Z");
            assert_eq!(tick(&restarted).await, 1);
        });
    }

    #[test]
    fn schedules_need_a_budget_a_valid_time_and_room_under_policy() {
        let dir = folder("schedule-limits");
        block_on(async {
            let mut setup = Setup::new(answers());
            setup.limits.max_schedules = Some(1);
            let (server, _) = setup.build(&dir).await;
            assert_eq!(
                create(&server, cron("* * * * *"), 0).await.unwrap_err(),
                code::INVALID_PARAMS
            );
            assert_eq!(
                create(&server, cron("every day"), 10).await.unwrap_err(),
                code::INVALID_PARAMS
            );
            create(&server, cron("0 9 * * 1-5"), 10).await.unwrap();
            assert_eq!(
                create(&server, cron("0 10 * * *"), 10).await.unwrap_err(),
                code::LIMIT
            );
            let unknown = ScheduleId::new(Timestamp::UNIX_EPOCH, [7; 10]).unwrap();
            let missing = server
                .request(Request::ScheduleDelete { schedule: unknown })
                .await
                .unwrap_err();
            assert_eq!(missing.code, code::NO_SCHEDULE);
        });
    }

    #[test]
    fn the_task_ledger_is_never_listed_or_read_as_a_thread() {
        let dir = folder("schedule-ledger");
        block_on(async {
            let (server, _) = Setup::new(answers()).build(&dir).await;
            create(&server, cron("0 9 * * *"), 10).await.unwrap();
            let Reply::Threads { threads, .. } = server
                .request(Request::ThreadList {
                    space: None,
                    limit: None,
                    show: None,
                    cursor: None,
                })
                .await
                .unwrap()
            else {
                panic!("threads");
            };
            assert_eq!(threads, Vec::new());
            let ledger = glave_event::ThreadId::new(Timestamp::UNIX_EPOCH, [0; 10]).unwrap();
            let read = server
                .request(Request::ThreadEvents {
                    thread: ledger,
                    after: None,
                    limit: None,
                })
                .await
                .unwrap_err();
            assert_eq!(read.code, code::NO_THREAD);
        });
    }
}
