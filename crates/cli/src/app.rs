//! `glave app`: glave in the browser, signed in, with nothing to type, as `opencode web` and
//! `jupyter lab` open theirs. It opens the server this home already runs, or starts one here
//! and opens that. `glave` alone does the same when no model is set up yet, since the browser
//! is where a first model is added; with one, `glave` stays the terminal session.
//!
//! The browser is sent to a sign-in link, whose one-time code the server gives glave signed in
//! with its local token, through a page in glave's folder, readable only by you, as Jupyter
//! does, so the code never shows in the list of running programs. The page is opened with the
//! operating system's own opener (`open`, `xdg-open`, `start`): a dependency would add nothing
//! these three lines do not.

use std::io::IsTerminal as _;
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::Path;
use std::process::ExitCode;
use std::time::Duration;

use anyhow::Context as _;
use glave_session::client::Remote;
use glave_session::protocol::{Reply, Request};

use crate::args::{Action, Args};
use crate::home;

/// The port glave in the browser is served on when it is free, so its address stays the same.
const PORT: u16 = 4180;

/// The file a running server's address is kept in, beside its token.
pub const URL_FILE: &str = "serve.url";

/// The page that sends the browser to the sign-in link.
pub const OPEN_FILE: &str = "open.html";

/// How long a server the home names may take to answer.
const KNOCK: Duration = Duration::from_millis(500);

/// What `glave app` does.
#[derive(Debug, PartialEq, Eq)]
pub enum Opening {
    /// A server for this home runs at `url`: open it.
    Running { url: String, token: String },
    /// None does: start one.
    Start,
}

/// What to do, from what the home says runs (`url`, `token`) and whether `answers` reaches it.
pub fn decide(url: Option<&str>, token: Option<&str>, answers: impl Fn(&str) -> bool) -> Opening {
    match (url.map(str::trim), token.map(str::trim)) {
        (Some(url), Some(token)) if !url.is_empty() && !token.is_empty() && answers(url) => {
            Opening::Running {
                url: url.to_owned(),
                token: token.to_owned(),
            }
        }
        _ => Opening::Start,
    }
}

/// Whether `glave` with `args` opens the browser rather than a terminal session: nothing asked
/// of it, a person at the terminal of a computer with a screen (`screen`), and no model set up
/// anywhere.
pub fn first_run(args: &Args, terminal: bool, screen: bool, set_up: bool) -> bool {
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
pub fn has_screen(os: &str, var: impl Fn(&str) -> bool) -> bool {
    if var("SSH_CONNECTION") || var("SSH_TTY") {
        return false;
    }
    matches!(os, "macos" | "windows") || var("DISPLAY") || var("WAYLAND_DISPLAY")
}

/// Whether a browser here would show on the person's screen.
pub fn screen_here() -> bool {
    has_screen(std::env::consts::OS, |name| {
        std::env::var_os(name).is_some()
    })
}

/// The program that opens `target` with the person's browser on `os`, and its arguments.
pub fn opener(os: &str, target: &Path) -> (&'static str, Vec<String>) {
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
pub fn redirect(link: &str) -> String {
    format!(
        "<!doctype html>\n<meta charset=\"utf-8\">\n<meta http-equiv=\"refresh\" \
         content=\"0;url={link}\">\n<title>Opening glave</title>\n<p><a href=\"{link}\">Open \
         glave</a></p>\n"
    )
}

/// `glave app`: opens the server this home runs, or serves here and opens that.
pub async fn main(mut args: Args) -> anyhow::Result<ExitCode> {
    let folder = home::folder()?;
    let url = std::fs::read_to_string(folder.join(URL_FILE)).ok();
    let token = std::fs::read_to_string(folder.join(crate::serving::TOKEN_FILE)).ok();
    match decide(url.as_deref(), token.as_deref(), answers) {
        Opening::Running { url, token } => {
            let code = link(&url, token).await?;
            if let Err(error) = show(&folder, &url, &code).await {
                // Shown only to a person at a terminal: error output often ends up in a log.
                anyhow::ensure!(std::io::stderr().is_terminal(), error);
                eprintln!(
                    "glave: {error:#}; open {url}/#code={} in a browser here (it works once, \
                     for five minutes)",
                    code.expose()
                );
                return Ok(ExitCode::FAILURE);
            }
            eprintln!("glave: opened glave in your browser");
            Ok(ExitCode::SUCCESS)
        }
        Opening::Start => {
            args.action = Action::Serve;
            args.http = Some(address());
            args.open = true;
            crate::session::serve(args).await
        }
    }
}

/// A new one-time code for a sign-in link, from the server at `url`, signed in with its local
/// `token`.
async fn link(url: &str, token: String) -> anyhow::Result<glave_event::Secret> {
    let server = Remote::new(url, token)?;
    match server.request(&Request::SigninLink {}).await? {
        Reply::Link { code } => Ok(code),
        other => anyhow::bail!("asked {url} for a sign-in link, it answered {other:?}"),
    }
}

/// Opens glave at `url` in the person's browser, signed in by the link with `code`: through the
/// page in glave's folder, so the code is never an argument of a program.
pub async fn show(folder: &Path, url: &str, code: &glave_event::Secret) -> anyhow::Result<()> {
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

/// Whether a server answers at `url`, an address on this computer.
fn answers(url: &str) -> bool {
    url.strip_prefix("http://")
        .and_then(|rest| rest.trim_end_matches('/').parse::<SocketAddr>().ok())
        .filter(|address| address.ip().is_loopback())
        .is_some_and(|address| TcpStream::connect_timeout(&address, KNOCK).is_ok())
}

/// Where to serve: the usual port when it is free, else any.
fn address() -> SocketAddr {
    let usual = SocketAddr::from((Ipv4Addr::LOCALHOST, PORT));
    if TcpListener::bind(usual).is_ok() {
        usual
    } else {
        SocketAddr::from((Ipv4Addr::LOCALHOST, 0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_server_the_home_names_is_opened_only_when_it_answers() {
        let up = |_: &str| true;
        let down = |_: &str| false;
        let url = Some("http://127.0.0.1:4180");
        assert_eq!(
            decide(url, Some("glave_local_x\n"), up),
            Opening::Running {
                url: "http://127.0.0.1:4180".into(),
                token: "glave_local_x".into(),
            }
        );
        assert_eq!(decide(url, Some("glave_local_x"), down), Opening::Start);
        assert_eq!(decide(None, Some("glave_local_x"), up), Opening::Start);
        assert_eq!(decide(url, None, up), Opening::Start);
        assert_eq!(decide(url, Some(" "), up), Opening::Start);
    }

    #[test]
    fn only_a_server_on_this_computer_that_listens_answers() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        assert!(answers(&url));
        drop(listener);
        assert!(!answers("http://10.0.0.1:80"), "not on this computer");
        assert!(!answers("https://127.0.0.1:1"));
        assert!(!answers("nonsense"));
    }

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
