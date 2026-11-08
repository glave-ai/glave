//! A person's terminals: opened in a folder, typed into, read from an offset and read again,
//! resized as its shell sees, ended by the shell itself, closed, and bounded in number and in
//! size. A suite of its own, apart from [`super::computers`]: a computer opens terminals only
//! where it can start a shell in a session of its own (glave's own program does it), so only
//! the computers that do run it.

use std::time::{Duration, Instant};

use super::helpers::failed;
use super::{Verdict, ensure};
use crate::computer::{
    Bytes, ComputerError, ComputerId, Computers, MAX_TERMINALS, TERMINAL_CHUNK, TerminalId,
    TerminalSize,
};

/// How long the suite gives a shell to answer what was typed into it.
const SOON: Duration = Duration::from_secs(30);
/// How long each read waits while the suite looks for output.
const POLL: Duration = Duration::from_millis(200);
const SIZE: TerminalSize = TerminalSize { cols: 80, rows: 24 };

/// Runs the suite on `computer`, in `folder` there: an empty folder, as the computer names it.
pub async fn terminals<C: Computers>(computers: &C, computer: ComputerId, folder: &str) -> Verdict {
    let terminal = computers
        .open_terminal(computer, folder, SIZE)
        .await
        .map_err(failed("open a terminal"))?;
    let at = typed_and_read(computers, computer, terminal, folder).await?;
    read_again(computers, computer, terminal, at).await?;
    resized(computers, computer, terminal).await?;
    ended_by_its_shell(computers, computer, terminal).await?;
    computers
        .close_terminal(computer, terminal)
        .await
        .map_err(failed("close a terminal whose shell ended"))?;
    ensure(
        matches!(
            computers
                .read_terminal(computer, terminal, 0, Duration::ZERO)
                .await,
            Err(ComputerError::NotFound(_))
        ),
        "a closed terminal is not found",
    )?;
    bounded(computers, computer, folder).await
}

/// Types `line` and a return into `terminal`.
async fn typed<C: Computers>(
    computers: &C,
    computer: ComputerId,
    terminal: TerminalId,
    line: &str,
) -> Verdict {
    let input = Bytes::new(format!("{line}\r"));
    computers
        .write_terminal(computer, terminal, &input)
        .await
        .map_err(|error| format!("type {line:?} into a terminal: {error}"))
}

/// What `terminal` printed from `from` until it printed `marker`, and the offset after it.
async fn until_printed<C: Computers>(
    computers: &C,
    computer: ComputerId,
    terminal: TerminalId,
    from: u64,
    marker: &str,
) -> Result<(String, u64), String> {
    let (mut seen, mut at) = (Vec::new(), from);
    let began = Instant::now();
    while began.elapsed() < SOON {
        let read = computers
            .read_terminal(computer, terminal, at, POLL)
            .await
            .map_err(failed("read a terminal"))?;
        ensure(
            read.from == at,
            "a terminal read keeps every byte of a few lines",
        )?;
        seen.extend_from_slice(read.output.as_slice());
        at = read.next;
        let text = String::from_utf8_lossy(&seen).into_owned();
        if text.contains(marker) {
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

/// A line typed runs in the shell, at the folder: its answer, never in what was typed, is read.
async fn typed_and_read<C: Computers>(
    computers: &C,
    computer: ComputerId,
    terminal: TerminalId,
    folder: &str,
) -> Result<u64, String> {
    let name = folder.rsplit('/').next().unwrap_or(folder);
    typed(
        computers,
        computer,
        terminal,
        "printf 'in-%s-%s\\n' \"${PWD##*/}\" $((6*7))",
    )
    .await?;
    let (_, at) = until_printed(computers, computer, terminal, 0, &format!("in-{name}-42")).await?;
    Ok(at)
}

/// A read from where the terminal began shows it all again; one at its end, nothing.
async fn read_again<C: Computers>(
    computers: &C,
    computer: ComputerId,
    terminal: TerminalId,
    at: u64,
) -> Verdict {
    let again = computers
        .read_terminal(computer, terminal, 0, Duration::ZERO)
        .await
        .map_err(failed("read a terminal again from its start"))?;
    ensure(
        again.from == 0 && again.next >= at && again.output.as_slice().len() <= TERMINAL_CHUNK,
        "a terminal read from its start again answers from its start",
    )?;
    ensure(
        String::from_utf8_lossy(again.output.as_slice()).contains("-42"),
        "a terminal read again shows what it printed",
    )?;
    let now = computers
        .read_terminal(computer, terminal, again.next.max(at), Duration::ZERO)
        .await
        .map_err(failed("read a terminal at its end"))?;
    ensure(
        now.output.as_slice().is_empty() || now.from >= at,
        "a terminal read at its end answers only what came since",
    )
}

/// A terminal given another size tells its shell, which sees it.
async fn resized<C: Computers>(
    computers: &C,
    computer: ComputerId,
    terminal: TerminalId,
) -> Verdict {
    let at = computers
        .read_terminal(computer, terminal, u64::MAX, Duration::ZERO)
        .await
        .map_err(failed("read where a terminal is"))?
        .next;
    let size = TerminalSize {
        cols: 101,
        rows: 31,
    };
    computers
        .resize_terminal(computer, terminal, size)
        .await
        .map_err(failed("resize a terminal"))?;
    typed(computers, computer, terminal, "stty size").await?;
    until_printed(computers, computer, terminal, at, "31 101").await?;
    ensure(
        computers
            .resize_terminal(computer, terminal, TerminalSize { cols: 0, rows: 31 })
            .await
            .is_err(),
        "a terminal of no columns is refused",
    )
}

/// A shell that exits ends its terminal, which is read to its end and kept until closed.
async fn ended_by_its_shell<C: Computers>(
    computers: &C,
    computer: ComputerId,
    terminal: TerminalId,
) -> Verdict {
    typed(computers, computer, terminal, "exit").await?;
    let began = Instant::now();
    let mut at = 0;
    while began.elapsed() < SOON {
        let read = computers
            .read_terminal(computer, terminal, at, POLL)
            .await
            .map_err(failed("read a terminal whose shell exits"))?;
        at = read.next;
        if read.ended {
            return Ok(());
        }
    }
    Err("a terminal whose shell exited did not say it ended".to_owned())
}

/// A computer keeps at most [`MAX_TERMINALS`] open, refuses one more, and refuses a terminal of
/// no rows or in no folder; closing one it doesn't have is not found.
async fn bounded<C: Computers>(computers: &C, computer: ComputerId, folder: &str) -> Verdict {
    let mut open = Vec::with_capacity(MAX_TERMINALS);
    for _ in 0..MAX_TERMINALS {
        let opened = computers.open_terminal(computer, folder, SIZE).await;
        open.push(opened.map_err(failed("open as many terminals as a computer keeps"))?);
    }
    let one_more = computers.open_terminal(computer, folder, SIZE).await;
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
    let flat = TerminalSize { cols: 80, rows: 0 };
    ensure(
        matches!(
            computers.open_terminal(computer, folder, flat).await,
            Err(ComputerError::Refused(_))
        ),
        "a terminal of no rows is refused",
    )?;
    let nowhere = format!("{folder}/not-there");
    ensure(
        computers
            .open_terminal(computer, &nowhere, SIZE)
            .await
            .is_err(),
        "a terminal in a folder that isn't there is refused",
    )?;
    let stray = TerminalId::from_uuid(uuid::Uuid::nil());
    ensure(
        matches!(
            computers.close_terminal(computer, stray).await,
            Err(ComputerError::NotFound(_))
        ),
        "closing a terminal the computer doesn't have is not found",
    )
}
