//! A person's terminals: opened in a folder for an owner, typed into, read from an offset and
//! read again, resized as its shell sees, ended by the shell itself, listed and closed, with
//! nothing its shell started outliving it, and bounded in number and in size. A suite of its
//! own, apart from [`super::computers`]: a computer opens terminals only where it can start a
//! shell in a session of its own (glave's own program does it), so only those run it.

use std::time::{Duration, Instant};

use super::helpers::failed;
use super::{Verdict, ensure};
use crate::computer::{
    Bytes, ComputerError, ComputerId, Computers, MAX_OWNED_TERMINALS, MAX_TERMINALS,
    TERMINAL_CHUNK, TerminalId, TerminalSize,
};

/// How long the suite gives a shell to answer what was typed into it.
const SOON: Duration = Duration::from_secs(30);
/// How long each read waits while the suite looks for output.
const POLL: Duration = Duration::from_millis(200);
const SIZE: TerminalSize = TerminalSize { cols: 80, rows: 24 };
/// Whose terminals the suite opens.
const OWNER: &str = "suite/terminals";

/// One terminal the suite drives.
struct At<'a, C> {
    computers: &'a C,
    computer: ComputerId,
    id: TerminalId,
    owner: &'a str,
}

impl<'a, C: Computers> At<'a, C> {
    async fn open(
        computers: &'a C,
        computer: ComputerId,
        folder: &str,
        owner: &'a str,
    ) -> Result<Self, String> {
        let id = computers
            .open_terminal(computer, folder, owner, SIZE)
            .await
            .map_err(failed("open a terminal"))?;
        Ok(Self {
            computers,
            computer,
            id,
            owner,
        })
    }

    /// It, as its owner names it.
    fn at(&self) -> (TerminalId, &'a str) {
        (self.id, self.owner)
    }

    /// Types `line` and a return.
    async fn typed(&self, line: &str) -> Verdict {
        let input = Bytes::new(format!("{line}\r"));
        self.computers
            .write_terminal(self.computer, self.at(), &input)
            .await
            .map_err(|error| format!("type {line:?} into a terminal: {error}"))
    }

    /// What it printed from `from` until it printed `marker`, and the offset after it.
    async fn until(&self, from: u64, marker: &str) -> Result<(String, u64), String> {
        self.until_seen(from, marker, |text| text.contains(marker))
            .await
    }

    /// What it printed from `from` until what it printed is `done`, and the offset after it.
    async fn until_seen(
        &self,
        from: u64,
        marker: &str,
        done: impl Fn(&str) -> bool,
    ) -> Result<(String, u64), String> {
        let (mut seen, mut at) = (Vec::new(), from);
        let began = Instant::now();
        while began.elapsed() < SOON {
            let read = self
                .computers
                .read_terminal(self.computer, self.at(), at, POLL)
                .await
                .map_err(failed("read a terminal"))?;
            ensure(
                read.from == at,
                "a terminal read keeps every byte of a few lines",
            )?;
            seen.extend_from_slice(read.output.as_slice());
            at = read.next;
            let text = String::from_utf8_lossy(&seen).into_owned();
            if done(&text) {
                return Ok((text, at));
            }
            if read.ended {
                return Err(format!(
                    "its shell ended before it printed {marker:?}: {text}"
                ));
            }
        }
        let text = String::from_utf8_lossy(&seen);
        Err(format!("a terminal did not print {marker:?} soon: {text}"))
    }

    /// Where it is now: the offset after all it printed.
    async fn end(&self) -> Result<u64, String> {
        let read = self
            .computers
            .read_terminal(self.computer, self.at(), u64::MAX, Duration::ZERO);
        Ok(read.await.map_err(failed("read where a terminal is"))?.next)
    }

    /// The number its shell prints after `name-` for `line`, typed.
    async fn number(&self, line: &str, name: &str) -> Result<String, String> {
        let at = self.end().await?;
        self.typed(line).await?;
        let marker = format!("{name}-");
        let (text, _) = self
            .until_seen(at, &marker, |text| number_in(text, &marker).is_some())
            .await?;
        number_in(&text, &marker).ok_or_else(|| format!("no number after {marker}: {text}"))
    }

    async fn close(&self) -> Verdict {
        self.computers
            .close_terminal(self.computer, self.at())
            .await
            .map_err(failed("close a terminal"))
    }
}

/// The digits printed after `marker`, once a line's end follows them: what was typed, echoed,
/// has none there.
fn number_in(text: &str, marker: &str) -> Option<String> {
    text.match_indices(marker).find_map(|(at, _)| {
        let after = text.get(at + marker.len()..)?;
        let digits: String = after.chars().take_while(char::is_ascii_digit).collect();
        let ended = after.get(digits.len()..)?.starts_with(['\r', '\n']);
        (!digits.is_empty() && ended).then_some(digits)
    })
}

/// Runs the suite on `computer`, in `folder` there: an empty folder, as the computer names it.
pub async fn terminals<C: Computers>(computers: &C, computer: ComputerId, folder: &str) -> Verdict {
    let terminal = At::open(computers, computer, folder, OWNER).await?;
    let name = folder.rsplit('/').next().unwrap_or(folder);
    terminal
        .typed("printf 'in-%s-%s\\n' \"${PWD##*/}\" $((6*7))")
        .await?;
    let (_, at) = terminal.until(0, &format!("in-{name}-42")).await?;
    read_again(&terminal, at).await?;
    resized(&terminal).await?;
    let listed = computers
        .terminals(computer, OWNER)
        .await
        .map_err(failed("list terminals"))?;
    ensure(
        listed
            .iter()
            .any(|each| each.id == terminal.id && !each.ended),
        "a terminal open for its owner is listed for it",
    )?;
    let others = computers
        .terminals(computer, "suite/nobody")
        .await
        .map_err(failed("list terminals"))?;
    ensure(others.is_empty(), "no other owner's terminal is listed")?;
    not_another_owner_s(&terminal).await?;
    ended_by_its_shell(&terminal).await?;
    terminal.close().await?;
    ensure(
        matches!(
            computers
                .read_terminal(computer, terminal.at(), 0, Duration::ZERO)
                .await,
            Err(ComputerError::NotFound(_))
        ),
        "a closed terminal is not found",
    )?;
    nothing_outlives_it(computers, computer, folder).await?;
    bounded(computers, computer, folder).await
}

/// A read from where the terminal began shows it all again; one at its end, nothing.
async fn read_again<C: Computers>(terminal: &At<'_, C>, at: u64) -> Verdict {
    let (computers, computer, id) = (terminal.computers, terminal.computer, terminal.at());
    let again = computers
        .read_terminal(computer, id, 0, Duration::ZERO)
        .await
        .map_err(failed("read a terminal again from its start"))?;
    ensure(
        again.from == 0 && again.next >= at && again.output.as_slice().len() <= TERMINAL_CHUNK,
        "a terminal read from its start again answers from its start",
    )?;
    ensure(
        String::from_utf8_lossy(again.output.as_slice()).contains("-42"),
        "a terminal read again shows what it printed",
    )
}

/// Another owner reaches no terminal by its id: not to read, type into, resize or close it.
async fn not_another_owner_s<C: Computers>(terminal: &At<'_, C>) -> Verdict {
    let (computers, computer) = (terminal.computers, terminal.computer);
    let theirs = (terminal.id, "suite/nobody");
    let read = computers.read_terminal(computer, theirs, 0, Duration::ZERO);
    let exit = Bytes::new("exit\r");
    let typed = computers.write_terminal(computer, theirs, &exit);
    let resized = computers.resize_terminal(computer, theirs, SIZE);
    let (read, typed, resized) = (read.await.err(), typed.await.err(), resized.await.err());
    let closed = computers.close_terminal(computer, theirs).await.err();
    let reached =
        |error: &Option<ComputerError>| !matches!(error, Some(ComputerError::NotFound(_)));
    ensure(
        !reached(&read) && !reached(&typed) && !reached(&resized) && !reached(&closed),
        "another owner's call on a terminal is not found",
    )?;
    let listed = computers.terminals(computer, terminal.owner).await;
    ensure(
        listed
            .map_err(failed("list terminals"))?
            .iter()
            .any(|each| each.id == terminal.id && !each.ended),
        "a terminal another owner tried to close stays open",
    )
}

/// A terminal given another size tells its shell, which sees it.
async fn resized<C: Computers>(terminal: &At<'_, C>) -> Verdict {
    let (computers, computer, id) = (terminal.computers, terminal.computer, terminal.at());
    let at = terminal.end().await?;
    let size = TerminalSize {
        cols: 101,
        rows: 31,
    };
    computers
        .resize_terminal(computer, id, size)
        .await
        .map_err(failed("resize a terminal"))?;
    terminal.typed("stty size").await?;
    terminal.until(at, "31 101").await?;
    ensure(
        computers
            .resize_terminal(computer, id, TerminalSize { cols: 0, rows: 31 })
            .await
            .is_err(),
        "a terminal of no columns is refused",
    )
}

/// A shell that exits ends its terminal, which is read to its end, kept until closed and
/// listed as ended.
async fn ended_by_its_shell<C: Computers>(terminal: &At<'_, C>) -> Verdict {
    terminal.typed("exit").await?;
    let began = Instant::now();
    let mut at = 0;
    while began.elapsed() < SOON {
        let read = terminal
            .computers
            .read_terminal(terminal.computer, terminal.at(), at, POLL)
            .await
            .map_err(failed("read a terminal whose shell exits"))?;
        at = read.next;
        if read.ended {
            let listed = terminal.computers.terminals(terminal.computer, OWNER).await;
            let listed = listed.map_err(failed("list terminals"))?;
            return ensure(
                listed
                    .iter()
                    .any(|each| each.id == terminal.id && each.ended),
                "a terminal whose shell ended is listed as ended until it is closed",
            );
        }
    }
    Err("a terminal whose shell exited did not say it ended".to_owned())
}

/// Nothing its shell started outlives a terminal: not a program running in front, not a job
/// in the background, whether the terminal is closed or its shell exits first.
async fn nothing_outlives_it<C: Computers>(
    computers: &C,
    computer: ComputerId,
    folder: &str,
) -> Verdict {
    let closed = At::open(computers, computer, folder, OWNER).await?;
    let job = closed.number("sleep 600 & echo bg-$!", "bg").await?;
    let deaf = closed
        .number("nohup sleep 603 >/dev/null 2>&1 & echo nohup-$!", "nohup")
        .await?;
    let ignoring = closed
        .number(
            "(trap '' HUP; exec sleep 604) & echo ignoring-$!",
            "ignoring",
        )
        .await?;
    let front = closed
        .number("sh -c 'echo fg-$$; exec sleep 601'", "fg")
        .await?;
    closed.close().await?;
    let exited = At::open(computers, computer, folder, OWNER).await?;
    let left = exited
        .number("sleep 602 & echo left-$! ; exit", "left")
        .await;
    let left = left?;
    let probe = At::open(computers, computer, folder, OWNER).await?;
    for (pid, what) in [
        (&job, "a job in the background"),
        (&deaf, "a job under nohup"),
        (&ignoring, "a job that ignores hang-ups"),
        (&front, "a program in front"),
    ] {
        gone(&probe, pid, what).await?;
    }
    exited.close().await?;
    gone(&probe, &left, "a job its shell left as it exited").await?;
    probe.close().await
}

/// Fails unless process `pid` is gone within moments, as `probe`'s shell sees.
async fn gone<C: Computers>(probe: &At<'_, C>, pid: &str, what: &str) -> Verdict {
    let at = probe.end().await?;
    probe
        .typed(&format!(
            "for i in $(seq 100); do kill -0 {pid} 2>/dev/null || break; sleep 0.1; done; \
             kill -0 {pid} 2>/dev/null && echo still-$((1+{pid})) || echo gone-$((1+{pid}))"
        ))
        .await?;
    let one_more: u64 = pid.parse().map_err(|_| format!("{pid} is no process id"))?;
    let (text, _) = probe.until(at, &format!("-{}", one_more + 1)).await?;
    ensure(
        text.contains(&format!("gone-{}", one_more + 1)),
        &format!("{what} outlived its terminal: {text}"),
    )
}

/// A computer keeps at most [`MAX_TERMINALS`] open and [`MAX_OWNED_TERMINALS`] for one owner,
/// refuses one more, and refuses a terminal of no rows or in no folder; closing one it doesn't
/// have is not found.
async fn bounded<C: Computers>(computers: &C, computer: ComputerId, folder: &str) -> Verdict {
    let mut open = Vec::with_capacity(MAX_TERMINALS);
    for index in 0..MAX_TERMINALS {
        let owner = if index < MAX_OWNED_TERMINALS {
            OWNER
        } else {
            "suite/another"
        };
        let opened = computers.open_terminal(computer, folder, owner, SIZE).await;
        let opened = opened.map_err(failed("open as many terminals as a computer keeps"))?;
        open.push((opened, owner));
    }
    let one_more = computers
        .open_terminal(computer, folder, "suite/third", SIZE)
        .await;
    for terminal in &open {
        computers
            .close_terminal(computer, *terminal)
            .await
            .map_err(failed("close a terminal"))?;
    }
    ensure(
        matches!(one_more, Err(ComputerError::Refused(_))),
        "a computer refuses a terminal past the most it keeps",
    )?;
    let mut owned = Vec::with_capacity(MAX_OWNED_TERMINALS);
    for _ in 0..MAX_OWNED_TERMINALS {
        let opened = computers.open_terminal(computer, folder, OWNER, SIZE).await;
        owned.push(opened.map_err(failed("open as many terminals as one owner keeps"))?);
    }
    let one_more = computers.open_terminal(computer, folder, OWNER, SIZE).await;
    for terminal in &owned {
        computers
            .close_terminal(computer, (*terminal, OWNER))
            .await
            .map_err(failed("close a terminal"))?;
    }
    ensure(
        matches!(one_more, Err(ComputerError::Refused(_))),
        "a computer refuses one owner a terminal past the most one keeps",
    )?;
    let flat = TerminalSize { cols: 80, rows: 0 };
    ensure(
        matches!(
            computers.open_terminal(computer, folder, OWNER, flat).await,
            Err(ComputerError::Refused(_))
        ),
        "a terminal of no rows is refused",
    )?;
    let nowhere = format!("{folder}/not-there");
    ensure(
        computers
            .open_terminal(computer, &nowhere, OWNER, SIZE)
            .await
            .is_err(),
        "a terminal in a folder that isn't there is refused",
    )?;
    let stray = TerminalId::from_uuid(uuid::Uuid::nil());
    ensure(
        matches!(
            computers.close_terminal(computer, (stray, OWNER)).await,
            Err(ComputerError::NotFound(_))
        ),
        "closing a terminal the computer doesn't have is not found",
    )
}
