//! `glave search`: whether the person's agent may search the web, from the terminal, as the
//! switch in Settings has it, through the session API of the person's server (`reach.rs`,
//! `specs/session.md`, "Web search").

use std::process::ExitCode;

use anyhow::bail;
use glave_session::protocol::{Reply, Request, SearchInfo};

use crate::reach::{self, Reached};
use crate::tasks::run_with;

pub(crate) const USAGE: &str = "\
glave search: whether glave may search the web for you

Usage:
  glave search          say whether web search is on, who searches and how often it did this
                        month
  glave search on       turn it on: your threads may search the web from their next message
  glave search off      turn it off";

/// `glave search ...`.
pub(crate) fn main(words: &[String]) -> ExitCode {
    run_with(words, USAGE, |words| async move {
        let words: Vec<&str> = words.iter().map(String::as_str).collect();
        let request = match words.as_slice() {
            [] => Request::SearchGet { usage: true },
            ["on"] => Request::SearchSet { on: true },
            ["off"] => Request::SearchSet { on: false },
            _ => bail!(
                "glave search takes on, off or nothing; to ask glave something that starts \
                 with \"search\", put it after --, as in: glave -- search the docs for retries\
                 \n\n{USAGE}"
            ),
        };
        reach::with(async |server| said(server, request).await).await
    })
}

/// Asks `server` with `request`, and says what web search is then.
async fn said(server: &mut Reached, request: Request) -> anyhow::Result<()> {
    let Reply::Search { search } = server.request(request).await? else {
        bail!("glave answered about web search with something else");
    };
    match search {
        SearchInfo::Never { reason } => {
            println!("Web search isn't yours to choose here: {reason}.");
        }
        SearchInfo::Chosen { on, detail, usage } => {
            let state = if on { "on" } else { "off" };
            println!("Web search is {state}. {detail}");
            if let Some(usage) = usage {
                println!("  {}", usage.sentence);
            }
        }
    }
    Ok(())
}
