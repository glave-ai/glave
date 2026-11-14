//! The mail server a throwaway glave that takes sign-ups sends through (`--signup`): a
//! stand-in on this computer, at an address the system gives it, that takes every message as
//! SMTP does and shows its links at the terminal, so the person opens a sign-up's, an invite's or
//! a way back in from there, with no mailbox, account or network. glave speaks to a mail server
//! on this computer without TLS, as nothing crosses a network.

use std::time::Duration;

use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;

/// How long glave may take to say its next line: it sends a message in one go.
const LINE_WITHIN: Duration = Duration::from_secs(30);

/// The most lines of a message it keeps, reading the rest to its end: glave's are a few lines.
const MOST_LINES: usize = 1_000;

/// The stand-in, taking messages until it is dropped.
pub(crate) struct Mailbox {
    /// What takes them, stopped as it drops.
    _taking: JoinSet<()>,
}

fn failed(err: &std::io::Error) -> String {
    format!("could not start the stand-in mail server: {err}")
}

/// Where the stand-in will take messages, at 127.0.0.1, before any runtime starts, so glave's
/// settings can name its port: the listener, and the port.
pub(crate) fn bind() -> Result<(std::net::TcpListener, u16), String> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").map_err(|err| failed(&err))?;
    let port = listener.local_addr().map_err(|err| failed(&err))?.port();
    listener.set_nonblocking(true).map_err(|err| failed(&err))?;
    Ok((listener, port))
}

/// Starts the stand-in taking messages at `listener`, on the runtime this is called on.
pub(crate) fn start(listener: std::net::TcpListener) -> Result<Mailbox, String> {
    let listener = TcpListener::from_std(listener).map_err(|err| failed(&err))?;
    let mut taking = JoinSet::new();
    taking.spawn(take(listener));
    Ok(Mailbox { _taking: taking })
}

/// Takes each connection to `listener`, one at a time, as glave sends one message at a time.
async fn take(listener: TcpListener) {
    while let Ok((stream, _)) = listener.accept().await {
        if let Err(err) = converse(stream).await {
            eprintln!("xtask: the stand-in mail server dropped a message: {err}");
        }
    }
}

/// Answers one SMTP conversation on `stream`, showing what each message it takes links to.
async fn converse(stream: TcpStream) -> Result<(), String> {
    let (reading, mut writing) = stream.into_split();
    let mut lines = BufReader::new(reading).lines();
    let mut say = async |line: &str| {
        writing
            .write_all(format!("{line}\r\n").as_bytes())
            .await
            .map_err(|err| format!("could not answer: {err}"))
    };
    say("220 try-org").await?;
    let mut to = Vec::new();
    loop {
        let Some(line) = next(&mut lines).await? else {
            return Ok(());
        };
        let verb = line.get(..4).unwrap_or(&line).to_ascii_uppercase();
        match verb.as_str() {
            "EHLO" | "HELO" => say("250 try-org").await?,
            "RCPT" => {
                to.push(address(&line));
                say("250 OK").await?;
            }
            "DATA" => {
                say("354 go on").await?;
                let mut text = Vec::new();
                // Read to the end of the message however long, so what follows is a command.
                while let Some(line) = next(&mut lines).await? {
                    if line == "." {
                        break;
                    }
                    if text.len() >= MOST_LINES {
                        continue;
                    }
                    let unstuffed = line.strip_prefix('.').filter(|_| line.starts_with(".."));
                    text.push(unstuffed.unwrap_or(&line).to_owned());
                }
                show(&to, &text);
                to.clear();
                say("250 OK").await?;
            }
            "QUIT" => return say("221 bye").await,
            _ => say("250 OK").await?,
        }
    }
}

/// The next line glave sends, none once it hangs up.
async fn next(
    lines: &mut tokio::io::Lines<BufReader<tokio::net::tcp::OwnedReadHalf>>,
) -> Result<Option<String>, String> {
    tokio::time::timeout(LINE_WITHIN, lines.next_line())
        .await
        .map_err(|_| {
            format!(
                "glave said nothing for {}",
                glave_ports::spoken(LINE_WITHIN)
            )
        })?
        .map_err(|err| format!("could not read what glave sent: {err}"))
}

/// The address a `RCPT TO:<…>` line names.
fn address(line: &str) -> String {
    line.split_once('<')
        .and_then(|(_, rest)| rest.split_once('>'))
        .map_or_else(|| line.to_owned(), |(address, _)| address.to_owned())
}

/// Shows at the terminal whom a message went to and each link in its `text`, its soft line
/// breaks (quoted-printable's `=` at a line's end) joined again.
fn show(to: &[String], text: &[String]) {
    let mut joined = String::new();
    for line in text {
        if let Some(soft) = line.strip_suffix('=') {
            joined.push_str(soft);
        } else {
            joined.push_str(line);
            joined.push('\n');
        }
    }
    let joined = joined.replace("=3D", "=");
    let links: Vec<&str> = joined
        .split_whitespace()
        .filter(|word| word.starts_with("http://") || word.starts_with("https://"))
        .collect();
    eprintln!(
        "xtask: glave emailed {}: open {} in a private window",
        to.join(", "),
        links.join(" or ")
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_recipient_is_read_from_between_its_brackets() {
        assert_eq!(address("RCPT TO:<ada@example.com>"), "ada@example.com");
    }
}
