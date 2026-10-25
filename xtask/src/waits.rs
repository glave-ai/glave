//! `cargo xtask waits`: every wait has a limit, sized to what it waits for (#710,
//! `specs/invariants.md`, 20). In the non-test sources under `crates/` and `xtask/`, a call
//! that waits for something to end or arrive (a process, a thread, a channel, an async lock, a
//! notification, a watched value, a semaphore's permit) has its limit on the same line
//! (`timeout(`), or is listed in [`BOUNDED`] with what bounds it instead: a limit set around
//! it elsewhere, a stop or a close that always comes, or a person, the limit of a wait for
//! them, who can always cancel it. A new wait fails the check until it has one or the other,
//! and an entry that no longer matches anything fails it too, so the list stays true.
//!
//! Sockets, pipes and calls across the network cannot be told from a line, so their limits
//! are kept by the tests the invariant names.

use crate::sources;

/// Where waits are looked for.
const FOLDERS: &[&str] = &["crates", "xtask"];

/// What waits, as code calls it, and what it waits for.
const WAITS: &[(&str, &str)] = &[
    (".wait()", "a process's or a thread's end"),
    (".wait_with_output()", "a process's end"),
    (".join()", "a thread's end"),
    (".recv()", "a channel's next message"),
    (".lock().await", "an async lock"),
    (".notified()", "a notification"),
    (".changed()", "a watched value's change"),
    (".acquire().await", "a semaphore's permit"),
    (".acquire_owned().await", "a semaphore's permit"),
];

/// What waits in xtask, which may run a program without glave-process: a process's end. Not
/// looked for in `crates/`, where an HTTP response's `status()` shares the name and `cargo
/// xtask spawns` already sends every process through glave-process.
const XTASK_WAITS: &[(&str, &str)] = &[
    (".status()", "a process's end"),
    (".output()", "a process's end"),
];

/// What says a wait's limit is on its line.
const LIMITED: &str = "timeout(";

/// The waits whose limit is not on their line: the file, what its line says, and what bounds
/// the wait. One entry may stand for several lines of its file that say the same.
const BOUNDED: &[(&str, &str, &str)] = &[
    // A person is the limit: glave waits for them, and they can always stop it.
    (
        "crates/cli/src/connections.rs",
        "_ = interrupts.recv() => bail!(\"stopped\"),",
        "the person typing a git host's token, who stops it with Ctrl-C: this arm",
    ),
    (
        "crates/cli/src/input.rs",
        "self.lines.lock().await.recv().await",
        "the person's next line at the terminal; Ctrl-C stops the wait",
    ),
    (
        "crates/cli/src/input.rs",
        "interrupts.recv().await",
        "Ctrl-C itself, from the person at the terminal",
    ),
    (
        "crates/session/src/turn.rs",
        "if done.changed().await.is_err() {",
        "a turn stopped at an approval or a question goes on once the person answers; \
         cancelling the turn ends it",
    ),
    (
        "crates/session/src/held.rs",
        "if done.changed().await.is_err() {",
        "work on a thread ending: a turn, whose model call has its idle limit, each tool its \
         own and each wait the person; cancelling it ends it",
    ),
    // A life that ends when it is stopped or closed: a server's, a connection's, a watch's.
    (
        "crates/cli/src/server/serve.rs",
        "_ = terminate.recv() => break None,",
        "the person's server serves until told to stop: SIGTERM, which ends it",
    ),
    (
        "crates/cli/src/server/serve.rs",
        "_ = interrupt.recv() => break None,",
        "the person's server serves until told to stop: Ctrl-C, which ends it",
    ),
    (
        "crates/computer/src/remote.rs",
        "line = queued.recv() => line,",
        "the connection's writer, until it closes or stops (the arm beside it); each call on it \
         waits within its own deadline",
    ),
    (
        "crates/computer/src/remote.rs",
        "() = shared.stop.notified() => None,",
        "the stop of the connection's writer, which runs for as long as the connection",
    ),
    (
        "crates/computer/src/serve.rs",
        "while let Some(mut line) = queued.recv().await {",
        "the connection's writer, which ends once its requests have and the queue closes",
    ),
    (
        "crates/session/src/client/lines.rs",
        "while let Some(mut line) = queued.recv().await {",
        "the connection's writer, which ends when the connection drops",
    ),
    (
        "crates/session/src/rpc.rs",
        "while let Some(line) = queued.recv().await {",
        "the connection's writer, which ends when the connection does",
    ),
    (
        "crates/session/src/client/lines.rs",
        "Some(item) = self.items.recv() => item,",
        "a turn followed on this computer's socket: a turn waiting for the person sends \
         nothing, and a server that goes closes the socket, which ends it",
    ),
    (
        "crates/session/src/rpc/watches.rs",
        "self.end.notified().await;",
        "a watch's life: it ends when its client unwatches or its connection closes",
    ),
    (
        "crates/session/src/lists.rs",
        "self.items.recv().await",
        "a watch of a list, followed until its client drops it",
    ),
    (
        "crates/session/src/lists.rs",
        "let behind = match changes.recv().await {",
        "a list's changes, heard until its watch is dropped or the server stops",
    ),
    (
        "crates/session/src/watch.rs",
        "self.notes.recv().await",
        "a thread's notes, heard until it is unwatched or its connection closes",
    ),
    (
        "crates/memory/src/client.rs",
        "while let Some(ledger) = asked.recv().await {",
        "the tidy-up queue, until memory is dropped; each tidy-up is a model call within the \
         provider's limits",
    ),
    // A limit around it, set elsewhere.
    (
        "crates/computer/src/local/background.rs",
        "status = child.wait() => break status.map_or(Ending::Signalled, exited),",
        "a command in the background, until a stop, its time limit or its idle limit, the arms \
         beside it",
    ),
    (
        "crates/computer/src/local/background.rs",
        "() = shared.stop.notified() => {",
        "the stop of a command in the background, whose time and idle limits are the arms \
         beside it",
    ),
    (
        "crates/computer/src/remote.rs",
        "Some(text) = chunks.recv() => output.write(&text).await,",
        "a running command's output, beside the timer of its call's deadline",
    ),
    (
        "crates/computer/src/remote.rs",
        "Some((id, ask)) = asks.recv() => self.give(id, &ask, &hosts, credentials).await,",
        "a running command's credential requests, beside the timer of its call's deadline",
    ),
    (
        "crates/process/src/child.rs",
        "let status = self.inner.wait().await?;",
        "Child::wait, which every caller bounds: a command's time limit, a stop or a cancel, \
         each checked here where it waits",
    ),
    (
        "crates/process/src/child.rs",
        "if self.inner.wait().await.is_ok() {",
        "Child::gone, only ever awaited within GRACE in Child::stop",
    ),
    (
        "crates/process/src/program.rs",
        "child.wait_with_output().await",
        "inside Program::output_with's run, which has the caller's limit",
    ),
    (
        "crates/process/src/within.rs",
        "let _ = child.wait();",
        "reaps a program just killed, which has ended",
    ),
    (
        "crates/process/src/within.rs",
        "reading.join()",
        "the readers of a program's output, whose pipes close once it has ended or been \
         killed at its limit",
    ),
    (
        "crates/computer/src/worker/kubernetes.rs",
        "let _ = process.join().await;",
        "the end of the copy into a worker, within COPIED_WITHIN",
    ),
    (
        "crates/computer/src/worker.rs",
        "let mut done = worker.set_up.lock().await;",
        "held while the worker's setup runs, each command within SETUP_TIMEOUT_SECS",
    ),
    (
        "crates/computer/src/worker/reconnect.rs",
        "let mut failed = self.failed.lock().await;",
        "held while the connection to a worker is opened again, within the worker's START_WAIT \
         and the remote computer's deadlines",
    ),
    (
        "crates/cli/src/reread.rs",
        "let mut seen = self.seen.lock().await;",
        "held while the person's settings are read again from this computer's disk",
    ),
    (
        "crates/session/src/providers.rs",
        "let _changing = self.inner.changing_models.lock().await;",
        "held while a provider is connected, refreshed or removed, its models listed within \
         the check's limit",
    ),
    (
        "crates/session/src/reread.rs",
        "let _changing = self.inner.changing_models.lock().await;",
        "held while the models offered are set again, from settings already read",
    ),
    (
        "crates/cli/src/connected.rs",
        "let _asker = askers.acquire().await.ok()?;",
        "a permit held only for the ask beside it, within ASKING",
    ),
    (
        "crates/eval/src/judging/model.rs",
        "let _permit = gate.acquire().await;",
        "a permit held for one judge call, within the provider's limits",
    ),
    (
        "crates/eval/src/suite.rs",
        "let _permit = permits.acquire_owned().await;",
        "a permit held for one attempt, within its task's time limit",
    ),
    (
        "xtask/src/cmd.rs",
        "Ok::<_, std::io::Error>((child.wait().await?, output))",
        "a query within QUERY_WITHIN; a step (a build, the tests, an eval) is the work asked \
         for, stopped by Ctrl-C at the terminal and by the job's time limit in CI",
    ),
    // Test doubles compiled outside tests, behind a feature tests turn on.
    (
        "crates/provider-openai/src/fake.rs",
        "let _ = thread.join();",
        "the fake server's thread, which its drop has just told to stop",
    ),
    (
        "crates/provider-anthropic/src/fake.rs",
        "let _ = thread.join();",
        "the fake server's thread, which its drop has just told to stop",
    ),
];

/// Fails on a wait with no limit on its line that [`BOUNDED`] does not list, and on an entry
/// of it that matches nothing.
pub(crate) fn check() -> Result<(), String> {
    let read = sources::read(FOLDERS)?;
    let mut used = vec![false; BOUNDED.len()];
    let mut problems = Vec::new();
    let mut waits = 0_usize;
    for (path, text) in &read {
        for (n, what) in unbounded(path, text) {
            waits += 1;
            match listed(path, &what.line) {
                Some(entry) => {
                    if let Some(used) = used.get_mut(entry) {
                        *used = true;
                    }
                }
                None => problems.push(format!(
                    "{path}:{n}: waits for {} with no limit: `{}`",
                    what.waits_for, what.line
                )),
            }
        }
    }
    for ((path, line, _), used) in BOUNDED.iter().zip(&used) {
        if !*used {
            problems.push(format!(
                "{path}: no wait says `{line}` any more; remove its entry from xtask/src/waits.rs"
            ));
        }
    }
    if problems.is_empty() {
        println!(
            "xtask: every wait in {} sources has a limit ({waits} bounded elsewhere, each said why)",
            read.len()
        );
        Ok(())
    } else {
        Err(format!(
            "{} wait(s) with no limit: give each one, sized to what it waits for, on its line \
             (tokio::time::timeout), or list it in xtask/src/waits.rs with what bounds it \
             (specs/invariants.md, 20):\n  {}",
            problems.len(),
            problems.join("\n  ")
        ))
    }
}

/// A wait found on one line.
struct Found {
    waits_for: &'static str,
    line: String,
}

/// Each wait in `text`, a source at `path`, with no limit on its line, by line number. What
/// a string says is not a call.
fn unbounded(path: &str, text: &str) -> Vec<(usize, Found)> {
    let xtask = path.starts_with("xtask/");
    sources::code(text)
        .into_iter()
        .filter(|(_, code)| !code.contains(LIMITED))
        .filter_map(|(n, code)| {
            let called = unquoted(code);
            let waits_for = WAITS
                .iter()
                .chain(XTASK_WAITS.iter().filter(|_| xtask))
                .find(|(call, _)| called.contains(call))
                .map(|(_, waits_for)| *waits_for)?;
            Some((
                n,
                Found {
                    waits_for,
                    line: code.trim_end().to_owned(),
                },
            ))
        })
        .collect()
}

/// `code` with what its string literals say left out, their quotes kept.
fn unquoted(code: &str) -> String {
    let mut kept = String::with_capacity(code.len());
    let (mut quoted, mut escaped) = (false, false);
    for c in code.chars() {
        match (quoted, escaped, c) {
            (true, false, '\\') => escaped = true,
            (true, true, _) => escaped = false,
            (_, _, '"') => {
                quoted = !quoted;
                kept.push(c);
            }
            (false, _, c) => kept.push(c),
            (true, false, _) => {}
        }
    }
    kept
}

/// The entry of [`BOUNDED`] that lists `line` of `path`, if one does.
fn listed(path: &str, line: &str) -> Option<usize> {
    BOUNDED
        .iter()
        .position(|(file, said, _)| *file == path && line.contains(said))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wait_with_no_limit_on_its_line_is_found() {
        let text = "async fn go() {\n    let status = child.wait().await?;\n    \
            let held = lock.lock().await;\n    let _ = rx.recv().await;\n}\n";
        let found: Vec<usize> = unbounded("crates/a/src/b.rs", text)
            .into_iter()
            .map(|(n, _)| n)
            .collect();
        assert_eq!(found, [2, 3, 4]);
    }

    #[test]
    fn a_limit_on_the_line_strings_comments_and_tests_pass() {
        let text = "let _ = tokio::time::timeout(LIMIT, child.wait()).await;\n\
            // child.wait() is not called here\nlet said = \"rx.recv() \\\" child.wait()\";\n\
            #[cfg(test)]\nmod tests {\n    fn t() { child.wait(); }\n}\n";
        assert!(unbounded("crates/a/src/b.rs", text).is_empty());
        assert_eq!(unquoted(r#"a("b.wait()", c.recv())"#), r#"a("", c.recv())"#);
        assert!(unbounded("crates/a/src/b.rs", "let x = words.join(\" \");\n").is_empty());
    }

    #[test]
    fn a_process_waited_for_in_xtask_is_found_and_an_http_status_elsewhere_is_not() {
        let text = "let ran = Command::new(\"ls\").status();\nlet code = response.status();\n";
        assert_eq!(unbounded("xtask/src/a.rs", text).len(), 2);
        assert!(unbounded("crates/a/src/b.rs", text).is_empty());
    }

    #[test]
    fn every_entry_names_a_source_and_says_why() {
        for (path, line, why) in BOUNDED {
            assert!(sources::non_test(path), "{path} is not a source the check reads");
            assert!(!line.trim().is_empty(), "{path}: an entry with no line");
            assert!(why.len() > 20, "{path}: `{line}` needs its reason");
        }
    }
}
