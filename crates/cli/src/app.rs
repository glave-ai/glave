//! `glave` alone: glave in the browser, signed in, with nothing to type, as `opencode web` and
//! `jupyter lab` open theirs (`specs/one-server.md`, "The CLI as a client"). It opens the
//! person's server, started in the background first when it is not running and someone is at
//! the terminal, at this folder's project (asking once when the folder is new), at the
//! person's Chats in the home folder, when they would not make the folder a project or no one
//! is at the terminal to ask, or with `--continue` at the newest thread there. With no model
//! set up yet, the app adds one first.
//!
//! The browser is sent to a sign-in link, whose one-time code the server gives glave over its
//! socket, through a page in glave's folder, readable only by you, as Jupyter does, so the
//! code never shows in the list of running programs. The page is opened with the operating
//! system's own opener (`open`, `xdg-open`, `start`): a dependency would add nothing these
//! three lines do not. A second link, with a code of its own, is printed for any other browser
//! on this computer; each works once, for five minutes, so neither is a key left behind.

use std::io::IsTerminal as _;
use std::path::Path;
use std::process::ExitCode;

use anyhow::{Context as _, bail};
use glave_ports::ProjectId;
use glave_session::protocol::{Reply, Request};

use crate::args::Args;
use crate::client::Session;
use crate::home;
use crate::input::Input;
use crate::turns::Here;

/// The file a running server's address is kept in, beside its token.
pub(crate) const URL_FILE: &str = "serve.url";

/// The page that sends the browser to the sign-in link.
pub(crate) const OPEN_FILE: &str = "open.html";

/// How long the system's opener has to hand the page to the browser, which takes it at once;
/// past it the person is given the link to open.
const OPENS_WITHIN: std::time::Duration = std::time::Duration::from_secs(10);

/// Whether a browser here would show on the person's screen, on `os` with the variables `var`
/// gives: not over SSH, and on Linux and the like only with a display.
pub(crate) fn has_screen(os: &str, var: impl Fn(&str) -> bool) -> bool {
    if var("SSH_CONNECTION") || var("SSH_TTY") {
        return false;
    }
    matches!(os, "macos" | "windows") || var("DISPLAY") || var("WAYLAND_DISPLAY")
}

/// Whether a browser here would show on the person's screen.
pub(crate) fn screen_here() -> bool {
    has_screen(std::env::consts::OS, |name| {
        std::env::var_os(name).is_some()
    })
}

/// The program that opens `target` with the person's browser on `os`, and its arguments.
pub(crate) fn opener(os: &str, target: &Path) -> (&'static str, Vec<String>) {
    let target = target.display().to_string();
    match os {
        "macos" => ("open", vec![target]),
        // The shell's own handler, as `start` uses, without `cmd` reading the path.
        "windows" => (
            "rundll32",
            vec!["url.dll,FileProtocolHandler".into(), target],
        ),
        _ => ("xdg-open", vec![target]),
    }
}

/// The page that sends a browser to `link` at once, with the link itself for a browser that
/// does not follow it.
pub(crate) fn redirect(link: &str) -> String {
    format!(
        "<!doctype html>\n<meta charset=\"utf-8\">\n<meta http-equiv=\"refresh\" \
         content=\"0;url={link}\">\n<title>Opening glave</title>\n<p><a href=\"{link}\">Open \
         glave</a></p>\n"
    )
}

/// `glave` alone, or `glave -c` alone: opens the person's server in the browser at this
/// folder's project (the person's Chats when they would not make it one), or the newest
/// thread there, starting the server first when it is not running and someone is at the
/// terminal.
pub(crate) async fn open(args: &Args) -> anyhow::Result<ExitCode> {
    let Some(server) = crate::server::connect_or_start().await? else {
        bail!("glave opens in your browser when you run it from a terminal: run glave in one");
    };
    let session = Session::Elsewhere(server.remote());
    let folder = std::env::current_dir()?;
    let input = Input::start();
    let terminal = crate::server::at_terminal();
    let project = match crate::turns::here(session, &folder, terminal, &input).await? {
        Here::Project(project) => Some(project),
        Here::Chats => None,
        // The person wanted glave open, if not this folder as a project, or was not asked.
        Here::NoProject => {
            eprintln!("glave: opening your Chats; this folder stays as it is");
            None
        }
    };
    let page = page(session, project, args.resume).await?;
    let code = link(&server).await?;
    let home = home::folder()?;
    let url = std::fs::read_to_string(home.join(URL_FILE))
        .context("glave runs, but did not say where a browser reaches it")?;
    let url = format!("{}{page}", url.trim().trim_end_matches('/'));
    let shown = if screen_here() {
        show(&home, &url, &code).await
    } else {
        Err(anyhow::anyhow!("no screen here to open a browser on"))
    };
    // Links are shown only to a person at a terminal: error output often ends up in a log.
    let to_person = std::io::stderr().is_terminal();
    match shown {
        Ok(()) => {
            eprintln!("glave: opened glave in your browser");
            // The browser is open: a second link that cannot be had is no failure.
            if to_person && let Ok(other) = link(&server).await {
                eprintln!("{}", another_browser(&url, &other));
            }
        }
        Err(error) => {
            anyhow::ensure!(to_person, error);
            eprintln!(
                "glave: {error:#}; open {url}#code={} in a browser on this computer (it works \
                 once, for five minutes)",
                code.expose()
            );
        }
    }
    Ok(ExitCode::SUCCESS)
}

/// A new one-time code for a sign-in link, from `server`.
async fn link(server: &crate::socket::Connection) -> anyhow::Result<glave_event::Secret> {
    let Reply::Link { code } = server.request(Request::SigninLink {}).await? else {
        bail!("glave answered signin/link with something else");
    };
    Ok(code)
}

/// What says how to sign in another browser on this computer, at `url`, with `code`.
fn another_browser(url: &str, code: &glave_event::Secret) -> String {
    format!(
        "glave: to sign in another browser on this computer, open {url}#code={} there (it works \
         once, for five minutes)",
        code.expose()
    )
}

/// The app's page for `project` (the person's Chats when none): with `resume`, its newest
/// thread, when it has one; else the project's page, or Chats' new-thread page.
async fn page(
    session: Session<'_>,
    project: Option<ProjectId>,
    resume: bool,
) -> anyhow::Result<String> {
    if resume {
        let listed = Request::ThreadList {
            project,
            limit: Some(1),
            show: None,
            cursor: None,
        };
        if let Reply::Threads { threads, .. } = session.request(listed).await?
            && let Some(newest) = threads.first()
        {
            return Ok(format!("/t/{}", newest.id));
        }
    }
    Ok(project.map_or_else(|| "/".to_owned(), |project| format!("/p/{project}")))
}

/// Opens `link`, glave signed in by the code it carries, in the person's browser: through the
/// page in glave's folder, so the code is never an argument of a program. The opener is
/// recorded in glave's folder while it runs, which takes moments; this glave has no lifeline
/// of its own to tie it to.
pub(crate) async fn show(
    folder: &Path,
    url: &str,
    code: &glave_event::Secret,
) -> anyhow::Result<()> {
    let page = folder.join(OPEN_FILE);
    let link = format!("{url}#code={}", code.expose());
    crate::serving::keep(&page, &redirect(&link))?;
    let (program, arguments) = opener(std::env::consts::OS, &page);
    let mut run = glave_process::Program::new(program);
    run.args(&arguments)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    let opened = glave_process::Processes::recorded(folder.join("run"))
        .status(&mut run, OPENS_WITHIN)
        .await
        .with_context(|| format!("could not run {program} to open the browser"))?;
    anyhow::ensure!(opened.success(), "{program} could not open the browser");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_browser_shows_on_a_screen_here_not_over_ssh() {
        let set = |names: &'static [&'static str]| move |name: &str| names.contains(&name);
        assert!(has_screen("macos", set(&[])));
        assert!(has_screen("windows", set(&[])));
        assert!(has_screen("linux", set(&["WAYLAND_DISPLAY"])));
        assert!(!has_screen("linux", set(&[])), "no display");
        assert!(!has_screen("macos", set(&["SSH_CONNECTION"])), "over SSH");
        assert!(!has_screen("linux", set(&["DISPLAY", "SSH_TTY"])));
    }

    #[test]
    fn each_system_opens_the_page_with_its_own_opener() {
        let page = Path::new("/home/ada/.glave").join(OPEN_FILE);
        let opened = |os| {
            let (program, arguments) = opener(os, &page);
            format!("{program} {}", arguments.join("|"))
        };
        assert_eq!(opened("macos"), "open /home/ada/.glave/open.html");
        assert_eq!(opened("linux"), "xdg-open /home/ada/.glave/open.html");
        assert_eq!(
            opened("windows"),
            "rundll32 url.dll,FileProtocolHandler|/home/ada/.glave/open.html"
        );
    }

    #[test]
    fn another_browser_gets_a_link_of_its_own() {
        let code = glave_ports::testing::any_secret();
        let said = another_browser(
            "http://127.0.0.1:4180/p/x",
            &glave_event::Secret::new(code.clone()),
        );
        assert_eq!(
            said,
            format!(
                "glave: to sign in another browser on this computer, open \
                 http://127.0.0.1:4180/p/x#code={code} there (it works once, for five minutes)"
            )
        );
    }

    #[test]
    fn the_page_sends_the_browser_to_the_link() {
        let page = redirect("http://127.0.0.1:4180/p/x#code=abc");
        assert!(page.contains("content=\"0;url=http://127.0.0.1:4180/p/x#code=abc\""));
        assert!(page.contains("<a href=\"http://127.0.0.1:4180/p/x#code=abc\">"));
    }
}
