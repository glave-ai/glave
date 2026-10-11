//! `glave tasks` and `glave schedule`: the folder's tasks (threads that run with no one
//! attached) and the schedules that start them, through the task host. And, at the start of
//! every session here, a word about tasks that finished since the last one.

mod show;

use std::path::Path;
use std::process::ExitCode;

use anyhow::{Context as _, bail};
use glave_event::{At, ScheduleId, ThreadId, When};
use glave_harness::DEFAULT_BUDGET;
use glave_session::protocol::{Reply, Request};
use jiff::Timestamp;
use uuid::Uuid;

use crate::args::Args;
use crate::client::Glave;
use crate::daemon::Host;

pub use show::finished_since;

pub const TASKS_USAGE: &str = "\
glave tasks: what runs here with no one attached

Usage:
  glave tasks                 list the tasks running and recently finished: kind, model,
                              tokens and cost so far, elapsed time, status
  glave tasks stop <thread>   stop a running task
  glave --background <prompt>, or -b
                              start one: the turn runs in this folder's task host (on a
                              worker of its own when [workers] has a backend), so you can
                              close the terminal. With --goal it works toward the goal;
                              --budget <tokens> stops the turn at that many tokens

Read a task's thread with `glave threads`, or continue it in a session with --continue.";

pub const SCHEDULE_USAGE: &str = "\
glave schedule: run a prompt or a goal as a task at set times

Usage:
  glave schedule add \"<cron>\" <prompt> [--goal] [--budget <tokens>] [--worktree]
                              whenever the five-field cron expression (minute hour day
                              month weekday, in UTC) matches; --goal works toward the
                              prompt as a goal across turns
  glave schedule add --at <time> <prompt> [...]
                              once, at an RFC 3339 time such as 2026-10-11T09:00:00Z
  glave schedule list         the schedules, with when each runs next
  glave schedule rm <id>      delete a schedule; no more runs start

Every run has a token budget: --budget, or 1000000. Runs happen in this folder's task host,
which glave starts when needed. Runs missed while it was not running run once, as soon as
it is, never once for each.";

/// `glave tasks ...`.
pub fn tasks_main(words: &[String]) -> ExitCode {
    run_with(words, TASKS_USAGE, |words| async move {
        let folder = std::env::current_dir()?;
        match words.as_slice() {
            [] => list_tasks(&folder).await,
            [stop, thread] if stop == "stop" => stop_task(&folder, thread_id(thread)?).await,
            [first, ..] => bail!("unexpected {first:?}"),
        }
    })
}

/// `glave schedule ...`.
pub fn schedule_main(words: &[String]) -> ExitCode {
    run_with(words, SCHEDULE_USAGE, |words| async move {
        let folder = std::env::current_dir()?;
        match words.split_first() {
            Some((add, rest)) if add == "add" => {
                let request = schedule_request(rest)?;
                add_schedule(&folder, request).await
            }
            Some((list, [])) if list == "list" => list_schedules(&folder).await,
            Some((rm, [id])) if rm == "rm" => remove_schedule(&folder, schedule_id(id)?).await,
            Some((first, _)) => bail!("unexpected {first:?}"),
            None => list_schedules(&folder).await,
        }
    })
}

fn run_with<F, R>(words: &[String], usage: &str, run: F) -> ExitCode
where
    F: FnOnce(Vec<String>) -> R,
    R: Future<Output = anyhow::Result<()>>,
{
    if words.iter().any(|word| word == "--help" || word == "-h") {
        println!("{usage}");
        return ExitCode::SUCCESS;
    }
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("glave: could not start: {error}");
            return ExitCode::FAILURE;
        }
    };
    match runtime.block_on(run(words.to_vec())) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("glave: {error:#}");
            ExitCode::FAILURE
        }
    }
}

async fn list_tasks(folder: &Path) -> anyhow::Result<()> {
    let mut host = Host::connect_or_start(folder, &Args::default()).await?;
    let Reply::Tasks { tasks } = host.request(Request::TasksList { limit: None }).await? else {
        bail!("the task host answered tasks/list with something else");
    };
    if tasks.is_empty() {
        println!("No tasks here. Start one with glave --background <prompt>.");
    }
    for task in &tasks {
        println!("{}", show::task_line(task));
    }
    Ok(())
}

async fn stop_task(folder: &Path, thread: ThreadId) -> anyhow::Result<()> {
    let Some(mut host) = Host::connect(folder).await? else {
        bail!("no task is running here: the task host is not running");
    };
    host.request(Request::TasksStop { thread }).await?;
    println!("stopped the task in {thread}");
    Ok(())
}

/// Starts `prompt` (or `args.goal`) as a task in a new thread, for `glave --background`.
pub async fn background(args: &Args) -> anyhow::Result<ExitCode> {
    let folder = std::env::current_dir()?;
    let mut host = Host::connect_or_start(&folder, args).await?;
    let started = Request::ThreadStart {
        worktree: args.worktree.then_some(true),
        background: true,
    };
    let Reply::Thread { thread } = host.request(started).await? else {
        bail!("the task host answered thread/start with something else");
    };
    let thread = thread.id;
    let request = match &args.goal {
        Some(objective) => Request::GoalStart {
            thread,
            objective: Some(objective.clone()),
            budget: args.goal_budget,
            background: true,
        },
        None => Request::TurnStart {
            thread,
            prompt: args.prompt.clone(),
            background: true,
            budget: args.budget,
            meta: glave_session::protocol::Meta::default(),
        },
    };
    let Reply::Task { task } = host.request(request).await? else {
        bail!("the task host did not start a task");
    };
    println!("{}", show::task_line(&task));
    println!("glave tasks to follow it; glave tasks stop {thread} to stop it");
    Ok(ExitCode::SUCCESS)
}

/// Says which tasks finished since the last session here, and starts the task host when a
/// schedule is waiting and none is running (as after a restart).
pub async fn at_start(server: &Glave, args: &Args) {
    let Ok(folder) = std::env::current_dir() else {
        return;
    };
    for line in finished_since(server, &folder).await {
        eprintln!("glave: {line}");
    }
    let waiting = matches!(
        server.request(Request::ScheduleList {}).await,
        Ok(Reply::Schedules { schedules }) if schedules.iter().any(|s| s.next.is_some())
    );
    if waiting
        && matches!(Host::connect(&folder).await, Ok(None))
        && let Err(error) = Host::connect_or_start(&folder, args).await
    {
        eprintln!("glave: schedules wait here, but the task host did not start: {error:#}");
    }
}

async fn add_schedule(folder: &Path, request: Request) -> anyhow::Result<()> {
    let mut host = Host::connect_or_start(folder, &Args::default()).await?;
    let Reply::Schedule { schedule } = host.request(request).await? else {
        bail!("the task host answered schedule/create with something else");
    };
    println!("{}", show::schedule_line(&schedule));
    Ok(())
}

async fn list_schedules(folder: &Path) -> anyhow::Result<()> {
    let mut host = Host::connect_or_start(folder, &Args::default()).await?;
    let Reply::Schedules { schedules } = host.request(Request::ScheduleList {}).await? else {
        bail!("the task host answered schedule/list with something else");
    };
    if schedules.is_empty() {
        println!("No schedules here. Add one with glave schedule add \"<cron>\" <prompt>.");
    }
    for schedule in &schedules {
        println!("{}", show::schedule_line(schedule));
    }
    Ok(())
}

async fn remove_schedule(folder: &Path, schedule: ScheduleId) -> anyhow::Result<()> {
    let mut host = Host::connect_or_start(folder, &Args::default()).await?;
    host.request(Request::ScheduleDelete { schedule }).await?;
    println!("deleted schedule {schedule}");
    Ok(())
}

/// `schedule/create` from the words after `glave schedule add`.
fn schedule_request(words: &[String]) -> anyhow::Result<Request> {
    let mut when = None;
    let mut goal = false;
    let mut budget = DEFAULT_BUDGET;
    let mut worktree = None;
    let mut prompt = Vec::new();
    let mut rest = words.iter();
    while let Some(word) = rest.next() {
        match word.as_str() {
            "--at" => {
                let at = rest
                    .next()
                    .context("--at needs a time, such as 2026-10-11T09:00:00Z")?;
                let at: Timestamp = at
                    .parse()
                    .with_context(|| format!("--at {at:?} is not an RFC 3339 time"))?;
                when = Some(When::At(At::from(at)));
            }
            "--goal" => goal = true,
            "--worktree" => worktree = Some(true),
            "--budget" => {
                let tokens = rest.next().context("--budget needs a number of tokens")?;
                budget = tokens
                    .parse()
                    .ok()
                    .filter(|tokens| *tokens > 0)
                    .with_context(|| format!("--budget {tokens:?} is not a number of tokens"))?;
            }
            flag if flag.starts_with("--") => bail!("unknown option {flag}"),
            _ if when.is_none() => when = Some(When::Cron(word.clone())),
            _ => prompt.push(word.clone()),
        }
    }
    let when = when.context("glave schedule add needs a cron expression or --at <time>")?;
    let prompt = prompt.join(" ");
    if prompt.trim().is_empty() {
        bail!("glave schedule add needs a prompt");
    }
    Ok(Request::ScheduleCreate {
        when,
        prompt,
        goal,
        budget,
        worktree,
    })
}

fn thread_id(text: &str) -> anyhow::Result<ThreadId> {
    let uuid = Uuid::parse_str(text).with_context(|| format!("{text:?} is not a thread id"))?;
    ThreadId::try_from(uuid).with_context(|| format!("{text:?} is not a thread id"))
}

fn schedule_id(text: &str) -> anyhow::Result<ScheduleId> {
    let uuid = Uuid::parse_str(text).with_context(|| format!("{text:?} is not a schedule id"))?;
    ScheduleId::try_from(uuid).with_context(|| format!("{text:?} is not a schedule id"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(text: &[&str]) -> Vec<String> {
        text.iter().map(|word| (*word).to_owned()).collect()
    }

    #[test]
    fn schedule_add_reads_a_time_a_prompt_and_options() {
        let Request::ScheduleCreate {
            when,
            prompt,
            goal,
            budget,
            worktree,
        } = schedule_request(&words(&[
            "0 9 * * 1-5",
            "triage",
            "issues",
            "--budget",
            "5000",
        ]))
        .unwrap()
        else {
            panic!("schedule/create");
        };
        assert_eq!(when, When::Cron("0 9 * * 1-5".into()));
        assert_eq!(
            (prompt.as_str(), goal, budget, worktree),
            ("triage issues", false, 5000, None)
        );
        let Request::ScheduleCreate {
            when, goal, budget, ..
        } = schedule_request(&words(&[
            "--at",
            "2026-10-11T09:00:00Z",
            "--goal",
            "--worktree",
            "make it pass",
        ]))
        .unwrap()
        else {
            panic!("schedule/create");
        };
        assert!(matches!(when, When::At(_)));
        assert_eq!((goal, budget), (true, DEFAULT_BUDGET));
        for wrong in [
            &["0 9 * * *"][..],
            &["--at", "tomorrow", "x"],
            &["0 9 * * *", "x", "--budget", "0"],
            &["0 9 * * *", "x", "--every"],
        ] {
            assert!(schedule_request(&words(wrong)).is_err(), "{wrong:?}");
        }
    }
}
