//! `glave signup link`: a sign-up link whoever runs the server makes from its computer to hand
//! to one person (`specs/model.md`, "Identity"), through the session API of the person's
//! server (`reach.rs`): opened, it makes their account and their own personal space, though
//! the server takes no sign-ups. Printed this once.

use std::process::ExitCode;

use anyhow::bail;
use glave_event::{At, Secret};
use glave_session::protocol::{Reply, Request};

use crate::reach;
use crate::tasks::run_with;

pub(crate) const USAGE: &str = "\
glave signup: let someone sign up for their own personal space, while this glave takes no
sign-ups

Usage:
  glave signup link             a link to give one person, from this glave's computer: good
                                once and for 7 days, it makes their account and
                                their own personal space";

/// `glave signup ...`.
pub(crate) fn main(words: &[String]) -> ExitCode {
    run_with(words, USAGE, |words| async move {
        let words: Vec<&str> = words.iter().map(String::as_str).collect();
        match words.as_slice() {
            ["link"] => {}
            [] => bail!("{USAGE}"),
            [first, ..] => bail!("unexpected {first:?}\n\n{USAGE}"),
        }
        let request = Request::SignupLink {};
        let Reply::SignupLinked { link, expires } =
            Box::pin(reach::with(async |server| server.request(request).await)).await?
        else {
            bail!("glave answered about a sign-up link with something else");
        };
        print!("{}", said(&link, expires));
        Ok(())
    })
}

/// The link for the terminal, with when it stops working, in UTC.
fn said(link: &Secret, expires: At) -> String {
    let until = expires.timestamp().strftime("%-d %B %Y, %H:%M UTC");
    format!(
        "A sign-up link, good once until {until}:\n\n{}\n\n\
         Give it to one person: whoever opens it gives their name and makes a passkey, and \
         with it an account and their own personal space.\n",
        link.expose()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_link_reads_with_until_when() {
        let expires = At::from(jiff::Timestamp::from_second(1_795_000_000).unwrap());
        let link = Secret::new("https://glave.example.com/#signup=abc");
        let said = said(&link, expires);
        assert!(
            said.starts_with(
                "A sign-up link, good once until 18 November 2026, 11:06 UTC:\n\n\
                 https://glave.example.com/#signup=abc\n\n"
            ),
            "{said}"
        );
    }
}
