//! `glave credits`: an organization's credits, where the server keeps them (`specs/gateway.md`,
//! "Credits"), through the session API of the person's server (`reach.rs`): what is left and
//! what was spent this month, the month's limit set, and credits added by whoever runs the
//! server, the same operation a billing service calls.

use std::fmt::Write as _;
use std::process::ExitCode;

use anyhow::{Context as _, bail};
use glave_event::Money;
use glave_ports::{Entropy as _, OrgId, OsEntropy};
use glave_session::protocol::{CreditsInfo, Reply, Request};
use uuid::Uuid;

use crate::reach;
use crate::tasks::run_with;

pub(crate) const USAGE: &str = "\
glave credits: what is left to spend on models, where this glave keeps credits

Usage:
  glave credits                 what is left, and what was spent this month and by whom
  glave credits limit <dollars> set the most it may spend in a month (its owners)
  glave credits add <dollars> [--organization <id>] [--reference <reference>]
                                add credits, for whoever runs this glave, from its computer:
                                to its own organization, or the one --organization names;
                                --reference names the payment, which adds once however often
                                it is sent (a fresh one unless given)";

/// What was asked.
#[derive(Debug, PartialEq, Eq)]
enum Command {
    Show,
    Limit(Money),
    Add {
        amount: Money,
        organization: Option<OrgId>,
        reference: Option<String>,
    },
}

fn parse(words: &[String]) -> anyhow::Result<Command> {
    let words: Vec<&str> = words.iter().map(String::as_str).collect();
    let dollars = |text: &str| Money::parse_dollars(text).map_err(anyhow::Error::new);
    match words.as_slice() {
        [] => Ok(Command::Show),
        ["limit", amount] => Ok(Command::Limit(dollars(amount)?)),
        ["add", amount, rest @ ..] => {
            let (mut organization, mut reference) = (None, None);
            let mut rest = rest.iter();
            while let Some(flag) = rest.next() {
                let value = rest
                    .next()
                    .with_context(|| format!("{flag} needs a value"))?;
                match *flag {
                    "--organization" => {
                        let id = Uuid::try_parse(value)
                            .with_context(|| format!("{value:?} is not an organization's id"))?;
                        organization = Some(OrgId::from_uuid(id));
                    }
                    "--reference" => reference = Some((*value).to_owned()),
                    other => bail!("unexpected {other:?}\n\n{USAGE}"),
                }
            }
            let amount = dollars(amount)?;
            Ok(Command::Add {
                amount,
                organization,
                reference,
            })
        }
        ["limit" | "add"] => bail!("how many dollars?\n\n{USAGE}"),
        [first, ..] => bail!("unexpected {first:?}\n\n{USAGE}"),
    }
}

/// `glave credits ...`.
pub(crate) fn main(words: &[String]) -> ExitCode {
    run_with(words, USAGE, |words| async move {
        let request = match parse(&words)? {
            Command::Show => Request::CreditsGet {},
            Command::Limit(limit) => Request::CreditsLimit { limit },
            Command::Add {
                amount,
                organization,
                reference,
            } => Request::CreditsAdd {
                amount,
                reference: reference.map_or_else(fresh_reference, Ok)?,
                organization,
            },
        };
        let Reply::Credits { credits } =
            Box::pin(reach::with(async |server| server.request(request).await)).await?
        else {
            bail!("glave answered about credits with something else");
        };
        print!("{}", said(&credits));
        Ok(())
    })
}

/// A reference no payment has, for credits added by hand.
fn fresh_reference() -> anyhow::Result<String> {
    let random = OsEntropy.bytes()?;
    Ok(random
        .iter()
        .fold(String::from("by-hand-"), |mut text, byte| {
            let _ = write!(text, "{byte:02x}");
            text
        }))
}

/// `credits` for the terminal: amounts to the cent, and the month's spending by person.
fn said(credits: &CreditsInfo) -> String {
    let mut text = String::new();
    let month = credits.since.timestamp().strftime("%B %Y");
    if let Some(account) = &credits.account {
        let _ = writeln!(text, "Left: {:.2}", account.balance);
        let _ = match account.limit {
            Some(limit) => writeln!(text, "{month}: {:.2} spent of {limit:.2}", account.spent),
            None => writeln!(text, "{month}: {:.2} spent", account.spent),
        };
    } else {
        let _ = writeln!(text, "{month}:");
    }
    for person in &credits.people {
        let who = match (&person.name, person.you) {
            (_, true) => "you".to_owned(),
            (Some(name), false) => name.clone(),
            (None, false) => person.person.to_string(),
        };
        let _ = writeln!(text, "  {:.2}  {who}", person.spent);
    }
    text
}

#[cfg(test)]
mod tests {
    use glave_event::{At, Currency};
    use glave_session::protocol::{CreditsAccount, PersonSpent};

    use super::*;

    fn words(text: &[&str]) -> Vec<String> {
        text.iter().map(|word| (*word).to_owned()).collect()
    }

    fn dollars(micros: i64) -> Money {
        Money {
            micros,
            currency: Currency::USD,
        }
    }

    #[test]
    fn the_words_after_credits_pick_a_command() {
        assert_eq!(parse(&words(&[])).unwrap(), Command::Show);
        assert_eq!(
            parse(&words(&["limit", "25"])).unwrap(),
            Command::Limit(dollars(25_000_000))
        );
        let id = "0192f5c0-11aa-7e21-8c3d-5f9b2e4a6c13";
        let asked = parse(&words(&[
            "add",
            "1.50",
            "--organization",
            id,
            "--reference",
            "o-1",
        ]));
        assert_eq!(
            asked.unwrap(),
            Command::Add {
                amount: dollars(1_500_000),
                organization: Some(OrgId::from_uuid(Uuid::try_parse(id).unwrap())),
                reference: Some("o-1".into()),
            }
        );
        for wrong in [
            &["add"][..],
            &["add", "five"],
            &["add", "1", "--organization"],
            &["off"],
        ] {
            assert!(parse(&words(wrong)).is_err(), "{wrong:?}");
        }
        assert!(fresh_reference().unwrap().starts_with("by-hand-"));
    }

    #[test]
    fn credits_read_to_the_cent_with_each_person_s_spending() {
        let you = glave_event::PrincipalId::new(jiff::Timestamp::UNIX_EPOCH, [1; 10]).unwrap();
        let credits = CreditsInfo {
            account: Some(CreditsAccount {
                balance: dollars(4_200_000),
                limit: Some(dollars(10_000_000)),
                spent: dollars(5_800_000),
            }),
            people: vec![
                PersonSpent {
                    person: you,
                    name: None,
                    spent: dollars(5_000_000),
                    you: true,
                },
                PersonSpent {
                    person: you,
                    name: Some("Bo".into()),
                    spent: dollars(800_000),
                    you: false,
                },
            ],
            since: At::from(jiff::Timestamp::from_second(1_790_812_800).unwrap()),
            can_set_limit: true,
        };
        assert_eq!(
            said(&credits),
            "Left: $4.20\nOctober 2026: $5.80 spent of $10.00\n  $5.00  you\n  $0.80  Bo\n"
        );
    }
}
