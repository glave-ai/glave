//! A turn, a turn continuing the folder's thread (`-c`) and a goal, through the person's server
//! (`specs/one-server.md`, "The CLI as a client"): the folder's project, asked about once when
//! it is new; the thread; what its project asks to be trusted; then the turn, followed with the
//! code that follows a server elsewhere. With no one at the terminal, nothing is asked: the
//! folder becomes a project, and the turn is answered by no one. `glave` alone finds the
//! folder's project the same way (`app.rs`), and so do tasks and schedules, which with no one at
//! the terminal make none (`tasks.rs`); the commands that list find it, making nothing.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::bail;
use glave_harness::{DEFAULT_BUDGET, Goal};
use glave_ports::ProjectId;
use glave_session::protocol::{NewFolder, Reply, Request, ThreadSummary, TrustAsk, TrustChoice};

use crate::args::Args;
use crate::client::{Client, Session};
use crate::input::Input;
use crate::socket::Connection;
use crate::terminal::Printer;

/// What a command that finds no project for its folder says.
pub(crate) const NO_PROJECT: &str = "this folder isn't a glave project";

/// Where the folder's threads go.
pub(crate) enum Here {
    /// Its project.
    Project(ProjectId),
    /// The person's Chats: the home folder, the folders above it, the temporary folder.
    Chats,
    /// No project: the folder leads to none, and none was made, as the person said, or as no
    /// one was asked.
    NoProject,
}

/// Runs the turn or goal `args` ask for through the person's server at `server`; `terminal`
/// says whether someone is at the terminal to ask and to answer.
pub(crate) async fn run(
    args: &Args,
    server: &Connection,
    terminal: bool,
) -> anyhow::Result<ExitCode> {
    let session = Session::Elsewhere(server.remote());
    let input = Input::start();
    let folder = std::env::current_dir()?;
    let project = match here(session, &folder, terminal, &input).await? {
        Here::Project(project) => Some(project),
        Here::Chats => None,
        // The turn was asked for in this folder, and has nowhere else to work.
        Here::NoProject => {
            eprintln!("glave: nothing was made, and nothing ran");
            return Ok(ExitCode::SUCCESS);
        }
    };
    let summary = thread(session, project, args).await?;
    if terminal && let Some(ask) = &summary.trust {
        trust(session, summary.id, ask, &input).await?;
    }
    let thread = summary.id;
    if let Some(working) = crate::worktrees::note(session, thread).await {
        eprintln!("glave: {working}");
    }
    let client = Client {
        server: session,
        thread,
        input: &input,
        endpoint: None,
        asks: terminal,
        servers: None,
        model: args.model.clone(),
        approval: args.approval,
    };
    let model = match &args.model {
        Some(model) => model.clone(),
        None => default_model(session).await,
    };
    let mut printer = Printer::new(&model, format!("approval: {}", summary.approval.name()));
    let code = match &args.goal {
        Some(objective) => {
            let goal = Goal {
                objective: objective.clone(),
                token_budget: args.goal_budget.unwrap_or(DEFAULT_BUDGET),
            };
            Box::pin(client.goal(goal, &mut printer)).await
        }
        None => Box::pin(client.turn(&args.prompt, &mut printer)).await,
    };
    if let Some(path) = &args.events {
        crate::client::export(session, thread, path).await?;
    }
    Ok(code)
}

/// Where `folder`'s threads go, making nothing: the person's Chats where the folder is never a
/// project ([`crate::projects::never_a_project`]); else its project, as the server finds it;
/// else `Here::NoProject`.
pub(crate) async fn found(session: Session<'_>, folder: &Path) -> anyhow::Result<Here> {
    if crate::projects::never_a_project(folder) {
        return Ok(Here::Chats);
    }
    let path = root(folder)?.to_string_lossy().into_owned();
    let found = session.request(Request::ProjectFind { path }).await?;
    if let Reply::Projects { projects, .. } = found
        && let Some(project) = projects.first()
    {
        return Ok(Here::Project(project.id));
    }
    Ok(Here::NoProject)
}

/// Where `folder`'s threads go, as [`found`] finds it; for a folder that leads to no project, a
/// project made now, after asking once when someone is at the terminal, or `Here::NoProject`
/// when they say no, with nothing printed: the caller says what follows.
pub(crate) async fn here(
    session: Session<'_>,
    folder: &Path,
    terminal: bool,
    input: &Input,
) -> anyhow::Result<Here> {
    let found = found(session, folder).await?;
    if !matches!(found, Here::NoProject) {
        return Ok(found);
    }
    let root = root(folder)?;
    if terminal {
        eprint!("Make {} a glave project? [y/N] ", shown(&root));
        crate::terminal::flush();
        if !yes(input.line().await) {
            return Ok(Here::NoProject);
        }
    }
    let path = root.to_string_lossy().into_owned();
    let name = root
        .file_name()
        .map_or_else(|| path.clone(), |name| name.to_string_lossy().into_owned());
    let created = Request::ProjectCreate {
        name,
        members: Vec::new(),
        folder: Some(NewFolder::Path { path }),
    };
    match session.request(created).await? {
        Reply::Project { project, .. } => Ok(Here::Project(project.id)),
        other => bail!("glave answered project/create with {other:?}"),
    }
}

/// The folder `folder`'s project has: the repository around it, or `folder` itself.
fn root(folder: &Path) -> anyhow::Result<PathBuf> {
    let folder = std::fs::canonicalize(folder).unwrap_or_else(|_| folder.to_owned());
    Ok(glave_config::policy::Locations::here(&crate::home::folder()?, &folder).root)
}

/// `folder` with the home folder as `~`, as the person knows it.
fn shown(folder: &Path) -> String {
    std::env::var_os("HOME")
        .and_then(|home| folder.strip_prefix(home).ok())
        .map_or_else(
            || folder.display().to_string(),
            |rest| format!("~/{}", rest.display()),
        )
}

/// Whether a typed line says yes: `y` or `yes`; anything else, the end of input too, is no.
fn yes(line: Option<String>) -> bool {
    line.is_some_and(|line| matches!(line.trim().to_ascii_lowercase().as_str(), "y" | "yes"))
}

/// The thread to use in `project` (Chats when none): the newest one there with `--continue`,
/// otherwise a new one, choosing the model, approval mode and worktree `args` name.
async fn thread(
    session: Session<'_>,
    project: Option<ProjectId>,
    args: &Args,
) -> anyhow::Result<ThreadSummary> {
    if args.resume {
        let listed = Request::ThreadList {
            project,
            limit: Some(1),
            show: None,
            cursor: None,
        };
        if let Reply::Threads { threads, .. } = session.request(listed).await?
            && let Some(latest) = threads.first()
        {
            return summary(
                session
                    .request(Request::ThreadGet { thread: latest.id })
                    .await?,
            );
        }
    }
    let started = Request::ThreadStart {
        project,
        background: false,
        worktree: args.worktree.then_some(true),
        computer: None,
        model: args.model.clone(),
        approval: args.approval,
        effort: None,
    };
    summary(session.request(started).await?)
}

fn summary(reply: Reply) -> anyhow::Result<ThreadSummary> {
    match reply {
        Reply::Thread { thread } => Ok(thread),
        other => bail!("glave answered with {other:?} where it gives a thread"),
    }
}

/// Puts what the thread's project asks to be trusted to the person, and sends their answer.
async fn trust(
    session: Session<'_>,
    thread: glave_event::ThreadId,
    ask: &TrustAsk,
    input: &Input,
) -> anyhow::Result<()> {
    eprintln!("\nThis project can run commands on your computer:");
    for item in &ask.items {
        match &item.detail {
            Some(detail) => eprintln!("    {}: {detail}", item.text),
            None => eprintln!("    {}", item.text),
        }
    }
    eprint!("Trust them? glave asks once, and remembers the answer. [y/N] ");
    crate::terminal::flush();
    let answer = if yes(input.line().await) {
        TrustChoice::Trust
    } else {
        TrustChoice::NotNow
    };
    let answered = Request::TrustAnswer {
        thread,
        ask: ask.ask.clone(),
        answer,
    };
    session.request(answered).await?;
    Ok(())
}

/// The name of the model threads that choose none go to, for the answers' label.
async fn default_model(session: Session<'_>) -> String {
    match session.request(Request::ModelList {}).await {
        Ok(Reply::Models { models }) => models.into_iter().find(|model| model.default).map_or_else(
            || "glave".to_owned(),
            |model| model.name.unwrap_or(model.id),
        ),
        _ => "glave".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_clear_yes_says_yes() {
        assert!(yes(Some("y".into())) && yes(Some(" Yes ".into())));
        assert!(!yes(Some(String::new())) && !yes(Some("no".into())) && !yes(None));
    }
}
