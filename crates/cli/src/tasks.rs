//! `glave tasks` and `glave schedule`: every project's tasks (threads that run with no one
//! attached) and the schedules that start them, through the person's server (`reach.rs`); and
//! `glave --background`, which starts a task there.

mod show;

use std::process::ExitCode;

use anyhow::{Context as _, bail};
use glave_event::{At, ScheduleId, ThreadId, When};
use glave_harness::DEFAULT_BUDGET;
use glave_ports::ProjectId;
use glave_session::protocol::{Reply, Request};
use jiff::Timestamp;
use uuid::Uuid;

use crate::args::Args;
use crate::reach::{self, Reached};

pub(crate) const TASKS_USAGE: &str = "\
glave tasks: what runs here with no one attached

Usage:
  glave tasks                 list the tasks running and recently finished: kind, model,
                              tokens and cost so far, elapsed time, status
  glave tasks stop <thread>   stop a running task
  glave --background <prompt>, or -b
                              start one: the turn runs in glave running for you in the
                              background, on a worktree of its own (so the folder must be a
                              git repository with a commit), or a worker with
                              on = \"cloud\", so you can close the terminal. With --goal it
                              works toward the goal; --budget <tokens> stops the turn at
                              that many tokens

Read a task's thread with `glave threads`, or continue it in a session with --continue.";

pub(crate) const SCHEDULE_USAGE: &str = "\
glave schedule: run a prompt or a goal as a task at set times

Usage:
  glave schedule add \"<cron>\" <prompt> [--goal] [--budget <tokens>]
                              whenever the five-field cron expression (minute hour day
                              month weekday, in UTC) matches; --goal works toward the
                              prompt as a goal across turns
  glave schedule add --at <time> <prompt> [...]
                              once, at an RFC 3339 time such as 2026-10-11T09:00:00Z
  glave schedule list         the schedules, with when each runs next
  glave schedule rm <id>      delete a schedule; no more runs start

Every run has a token budget: --budget, or 1000000. Each run is a new thread in this
folder's project, on a worktree of its own. Runs happen while glave runs for you in the
background, which these commands start from a terminal; runs missed while it was not
running run once, as soon as it is, never once for each.";

/// `glave tasks ...`.
pub(crate) fn tasks_main(words: &[String]) -> ExitCode {
    run_with(words, TASKS_USAGE, |words| async move {
        let stop = match words.as_slice() {
            [] => None,
            [stop, thread] if stop == "stop" => Some(thread_id(thread)?),
            [first, ..] => bail!("unexpected {first:?}"),
        };
        reach::with(async |server| match stop {
            None => list_tasks(server).await,
            Some(thread) => stop_task(server, thread).await,
        })
        .await
    })
}

/// `glave schedule ...`.
pub(crate) fn schedule_main(words: &[String]) -> ExitCode {
    run_with(words, SCHEDULE_USAGE, |words| async move {
        let asked = match words.split_first() {
            Some((add, rest)) if add == "add" => Asked::Add(schedule_request(rest, None)?),
            Some((list, [])) if list == "list" => Asked::List,
            Some((rm, [id])) if rm == "rm" => Asked::Remove(schedule_id(id)?),
            Some((first, _)) => bail!("unexpected {first:?}"),
            None => Asked::List,
        };
        reach::with(async |server| match asked {
            Asked::Add(request) => add_schedule(server, request).await,
            Asked::List => list_schedules(server).await,
            Asked::Remove(schedule) => remove_schedule(server, schedule).await,
        })
        .await
    })
}

/// What `glave schedule` was asked.
enum Asked {
    Add(Request),
    List,
    Remove(ScheduleId),
}

pub(crate) fn run_with<F, R>(words: &[String], usage: &str, run: F) -> ExitCode
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

async fn list_tasks(server: &mut Reached) -> anyhow::Result<()> {
    let Reply::Tasks { tasks } = server.request(Request::TasksList { limit: None }).await? else {
        bail!("glave answered tasks/list with something else");
    };
    if tasks.is_empty() {
        println!("No tasks here. Start one with glave --background <prompt>.");
    }
    for task in &tasks {
        println!("{}", show::task_line(task));
    }
    Ok(())
}

async fn stop_task(server: &mut Reached, thread: ThreadId) -> anyhow::Result<()> {
    server.request(Request::TasksStop { thread }).await?;
    println!("stopped the task in {thread}");
    Ok(())
}

/// Starts `prompt` (or `args.goal`) as a task in a new thread of this folder's project, for
/// `glave --background`: only in the person's server, since a task outlives this command.
pub(crate) async fn background(args: &Args) -> anyhow::Result<ExitCode> {
    let folder = std::env::current_dir()?;
    let Some(server) = crate::server::connect_or_start().await? else {
        bail!(
            "a task runs in glave running for you in the background, which is not running, and \
             only a command run in a terminal starts it: run this in one, or run glave serve"
        );
    };
    let mut server = Reached::Server(server);
    let project = server.project(&folder).await?;
    let started = Request::ThreadStart {
        project: Some(project),
        worktree: None,
        background: true,
        computer: None,
        model: args.model.clone(),
        approval: args.approval,
    };
    let Reply::Thread { thread } = server.request(started).await? else {
        bail!("glave answered thread/start with something else");
    };
    let thread = thread.id;
    let request = match &args.goal {
        Some(objective) => Request::GoalStart {
            thread,
            objective: Some(objective.clone()),
            budget: args.goal_budget,
            background: true,
            unattended: false,
        },
        None => Request::TurnStart {
            thread,
            prompt: args.prompt.clone(),
            background: true,
            budget: args.budget,
            meta: glave_session::protocol::Meta::default(),
            model: None,
            approval: None,
            unattended: false,
        },
    };
    let Reply::Task { task } = server.request(request).await? else {
        bail!("glave did not start a task");
    };
    println!("{}", show::task_line(&task));
    println!("glave tasks to follow it; glave tasks stop {thread} to stop it");
    Ok(ExitCode::SUCCESS)
}

/// Adds the schedule `request` asks for in this folder's project.
async fn add_schedule(server: &mut Reached, mut request: Request) -> anyhow::Result<()> {
    if let Request::ScheduleCreate { project, .. } = &mut request {
        *project = Some(server.project(&std::env::current_dir()?).await?);
    }
    let Reply::Schedule { schedule } = server.request(request).await? else {
        bail!("glave answered schedule/create with something else");
    };
    println!("{}", show::schedule_line(&schedule));
    Ok(())
}

async fn list_schedules(server: &mut Reached) -> anyhow::Result<()> {
    let Reply::Schedules { schedules } = server.request(Request::ScheduleList {}).await? else {
        bail!("glave answered schedule/list with something else");
    };
    if schedules.is_empty() {
        println!("No schedules here. Add one with glave schedule add \"<cron>\" <prompt>.");
    }
    for schedule in &schedules {
        println!("{}", show::schedule_line(schedule));
    }
    Ok(())
}

async fn remove_schedule(server: &mut Reached, schedule: ScheduleId) -> anyhow::Result<()> {
    server.request(Request::ScheduleDelete { schedule }).await?;
    println!("deleted schedule {schedule}");
    Ok(())
}

/// `schedule/create` in `project` from the words after `glave schedule add`.
fn schedule_request(words: &[String], project: Option<ProjectId>) -> anyhow::Result<Request> {
    let mut when = None;
    let mut goal = false;
    let mut budget = DEFAULT_BUDGET;
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
        project,
        when,
        prompt,
        goal,
        budget,
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
            ..
        } = schedule_request(
            &words(&["0 9 * * 1-5", "triage", "issues", "--budget", "5000"]),
            None,
        )
        .unwrap()
        else {
            panic!("schedule/create");
        };
        assert_eq!(when, When::Cron("0 9 * * 1-5".into()));
        assert_eq!(
            (prompt.as_str(), goal, budget),
            ("triage issues", false, 5000)
        );
        let Request::ScheduleCreate {
            when, goal, budget, ..
        } = schedule_request(
            &words(&["--at", "2026-10-11T09:00:00Z", "--goal", "make it pass"]),
            None,
        )
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
            &["0 9 * * *", "x", "--worktree"],
        ] {
            assert!(schedule_request(&words(wrong), None).is_err(), "{wrong:?}");
        }
    }
}
