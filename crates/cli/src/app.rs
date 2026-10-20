//! `glave app`: glave in the browser, signed in, with nothing to type, as `opencode web` and
//! `jupyter lab` open theirs. It opens the person's server, started in the background first
//! when it is not running and someone is at the terminal. `glave` alone does the same when no
//! model is set up yet, since the browser is where a first model is added; with one, `glave`
//! stays the terminal session.
//!
//! The browser is sent to a sign-in link, whose one-time code the server gives glave over its
//! socket, through a page in glave's folder, readable only by you, as Jupyter does, so the
//! code never shows in the list of running programs. The page is opened with the operating
//! system's own opener (`open`, `xdg-open`, `start`): a dependency would add nothing these
//! three lines do not.

use std::io::IsTerminal as _;
use std::path::Path;
use std::process::ExitCode;

use anyhow::Context as _;
use glave_session::protocol::{Reply, Request};

use crate::args::{Action, Args};
use crate::home;

/// The file a running server's address is kept in, beside its token.
pub(crate) const URL_FILE: &str = "serve.url";

/// The page that sends the browser to the sign-in link.
pub(crate) const OPEN_FILE: &str = "open.html";

/// Whether `glave` with `args` opens the browser rather than a terminal session: nothing asked
/// of it, a person at the terminal of a computer with a screen (`screen`), and no model set up
/// anywhere.
pub(crate) fn first_run(args: &Args, terminal: bool, screen: bool, set_up: bool) -> bool {
    let bare = args.action == Action::Run
        && args.prompt.is_empty()
        && args.goal.is_none()
        && !args.background
        && !args.resume
        && args.events.is_none();
    bare && terminal && screen && !set_up
}

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

/// `glave app`: opens the person's server in the browser, starting it first when it is not
/// running and someone is at the terminal.
pub(crate) async fn main() -> anyhow::Result<ExitCode> {
    let Some(server) = crate::server::connect_or_start().await? else {
        anyhow::bail!(
            "glave is not running, and only glave app run in a terminal starts it: run it in \
             one, or run glave serve"
        );
    };
    let Reply::Link { code } = server.request(Request::SigninLink {}).await? else {
        anyhow::bail!("glave answered signin/link with something else");
    };
    let folder = home::folder()?;
    let url = std::fs::read_to_string(folder.join(URL_FILE))
        .context("glave runs, but did not say where a browser reaches it")?;
    let url = url.trim();
    let shown = if screen_here() {
        show(&folder, url, &code).await
    } else {
        Err(anyhow::anyhow!("no screen here to open a browser on"))
    };
    match shown {
        Ok(()) => eprintln!("glave: opened glave in your browser"),
        Err(error) => {
            // Shown only to a person at a terminal: error output often ends up in a log.
            anyhow::ensure!(std::io::stderr().is_terminal(), error);
            eprintln!(
                "glave: {error:#}; open {url}/#code={} in a browser on this computer (it works \
                 once, for five minutes)",
                code.expose()
            );
        }
    }
    Ok(ExitCode::SUCCESS)
}

/// Opens glave at `url` in the person's browser, signed in by the link with `code`: through the
/// page in glave's folder, so the code is never an argument of a program.
pub(crate) async fn show(
    folder: &Path,
    url: &str,
    code: &glave_event::Secret,
) -> anyhow::Result<()> {
    let page = folder.join(OPEN_FILE);
    let link = format!("{}/#code={}", url.trim_end_matches('/'), code.expose());
    crate::serving::keep(&page, &redirect(&link))?;
    let (program, arguments) = opener(std::env::consts::OS, &page);
    let opened = glave_process::Program::new(program)
        .args(&arguments)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .await
        .with_context(|| format!("could not run {program} to open the browser"))?;
    anyhow::ensure!(
        opened.status.success(),
        "{program} could not open the browser"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_glave_opens_the_browser_only_on_a_first_run_at_a_terminal() {
        let bare = crate::args::parse(Vec::<String>::new()).unwrap();
        assert!(first_run(&bare, true, true, false));
        assert!(!first_run(&bare, true, true, true), "a model is set up");
        assert!(
            !first_run(&bare, false, true, false),
            "no one at the terminal"
        );
        assert!(
            !first_run(&bare, true, false, false),
            "no screen to show a browser on"
        );
        for words in [&["fix", "it"][..], &["--continue"], &["--goal", "pass"]] {
            let args = crate::args::parse(words.iter().map(|w| (*w).to_owned())).unwrap();
            assert!(!first_run(&args, true, true, false), "{words:?}");
        }
    }

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
    fn the_page_sends_the_browser_to_the_link() {
        let page = redirect("http://127.0.0.1:4180/#code=abc");
        assert!(page.contains("content=\"0;url=http://127.0.0.1:4180/#code=abc\""));
        assert!(page.contains("<a href=\"http://127.0.0.1:4180/#code=abc\">"));
    }
}
