//! `glave signup link`: a sign-up link whoever runs the server makes from its computer to hand
//! to one person (`specs/model.md`, "Identity"), through the session API of the person's
//! server (`reach.rs`): opened, it makes their account and their own personal space, though
//! the server takes no sign-ups. Printed this once.

use std::process::ExitCode;

use anyhow::bail;
use glave_event::{At, Money, Secret};
use glave_session::protocol::{Reply, Request};

use crate::reach;
use crate::tasks::run_with;

pub(crate) const USAGE: &str = "\
glave signup: let someone sign up for their own personal space, while this glave takes no
sign-ups

Usage:
  glave signup link [--credits <dollars>]
                                a link to give one person, from this glave's computer: good
                                once and for 7 days, it makes their account and their own
                                personal space; where this glave keeps credits, --credits is
                                the balance the space starts with";

/// The starting balance the words after `signup` ask for, if any, once they ask for a link.
fn parse(words: &[String]) -> anyhow::Result<Option<Money>> {
    let words: Vec<&str> = words.iter().map(String::as_str).collect();
    match words.as_slice() {
        ["link"] => Ok(None),
        ["link", "--credits", dollars] => Ok(Some(Money::parse_dollars(dollars)?)),
        ["link", "--credits"] => bail!("how many dollars?\n\n{USAGE}"),
        [] => bail!("{USAGE}"),
        ["link", first, ..] | [first, ..] => bail!("unexpected {first:?}\n\n{USAGE}"),
    }
}

/// `glave signup ...`.
pub(crate) fn main(words: &[String]) -> ExitCode {
    run_with(words, USAGE, |words| async move {
        let credits = parse(&words)?;
        let request = Request::SignupLink { credits };
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
    fn the_words_after_signup_ask_for_a_link_with_its_starting_balance() {
        let words = |text: &[&str]| {
            text.iter()
                .map(|word| (*word).to_owned())
                .collect::<Vec<_>>()
        };
        assert_eq!(parse(&words(&["link"])).unwrap(), None);
        let five = Money::parse_dollars("5").unwrap();
        assert_eq!(
            parse(&words(&["link", "--credits", "5"])).unwrap(),
            Some(five)
        );
        for wrong in [
            &[][..],
            &["link", "--credits"],
            &["link", "--credits", "five"],
            &["link", "--other", "5"],
            &["off"],
        ] {
            assert!(parse(&words(wrong)).is_err(), "{wrong:?}");
        }
    }

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
