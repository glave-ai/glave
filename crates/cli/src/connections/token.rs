//! Reading a token the person pastes: from standard input, never from an argument, and on a
//! terminal with its echo off, so it shows neither in the shell's history and the list of
//! processes nor on the screen.

use std::io::IsTerminal as _;

use anyhow::bail;
use glave_event::Secret;
use rustix::termios::{LocalModes, OptionalActions, Termios, tcgetattr, tcsetattr};

use crate::input::Input;

/// Said when a token was given as an argument, without repeating it.
pub const NOT_AN_ARGUMENT: &str = "that looks like a token. glave never takes one as an \
     argument, since it would stay in your shell's history and show in the list of processes: \
     revoke it on its host, then run glave connect <address> and paste a new one when asked";

/// How the tokens GitHub and GitLab make begin.
const PREFIXES: [&str; 7] = [
    "ghp_",
    "gho_",
    "ghu_",
    "ghs_",
    "ghr_",
    "github_pat_",
    "glpat-",
];

/// Whether `word` looks like a token rather than an address.
pub fn looks_like(word: &str) -> bool {
    PREFIXES.iter().any(|prefix| word.starts_with(prefix))
}

/// The token for `url` from standard input: on a terminal, asked for with its echo off; else
/// the first line piped in.
pub async fn read(url: &str) -> anyhow::Result<Secret> {
    let terminal = std::io::stdin().is_terminal();
    // Echo off before the question, and anything typed before it thrown away.
    let hidden = terminal.then(Hidden::on).flatten();
    if terminal {
        let shows = if hidden.is_some() {
            "it does not show"
        } else {
            "this terminal will show it"
        };
        eprint!("Paste the token for {url} ({shows}), then press Enter: ");
    }
    let line = Input::start().line().await;
    if hidden.is_some() {
        eprintln!();
    }
    drop(hidden);
    let line = line.unwrap_or_default();
    let token = line.trim();
    if token.is_empty() {
        bail!("no token given: paste it when asked, or pipe it in: ... | glave connect {url}");
    }
    Ok(Secret::new(token))
}

/// The terminal's echo, off while this lives; its settings as they were after.
struct Hidden(Termios);

impl Hidden {
    /// Echo off on standard input's terminal, what was typed before thrown away; none when the
    /// terminal cannot say or change its settings.
    fn on() -> Option<Self> {
        let stdin = std::io::stdin();
        let saved = tcgetattr(&stdin).ok()?;
        let mut quiet = saved.clone();
        quiet.local_modes.remove(LocalModes::ECHO);
        tcsetattr(&stdin, OptionalActions::Flush, &quiet).ok()?;
        Some(Self(saved))
    }
}

impl Drop for Hidden {
    fn drop(&mut self) {
        let _ = tcsetattr(std::io::stdin(), OptionalActions::Now, &self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_token_is_told_from_an_address() {
        for token in ["ghp_x", "github_pat_x", "glpat-x", "ghu_x"] {
            assert!(looks_like(token), "{token}");
        }
        for address in ["https://gitlab.com", "github", "git.example.com"] {
            assert!(!looks_like(address), "{address}");
        }
    }
}
