//! The waits whose limit is not on their line, and what bounds each.

/// Each wait whose limit is not on its line: the file, what its line says, how many lines of
/// that file say it, and what bounds the wait.
pub(super) const BOUNDED: &[(&str, &str, usize, &str)] = &[
    // A person is the limit: glave waits for them, and they can always stop it.
    (
        "crates/cli/src/connections.rs",
        "_ = interrupts.recv() => bail!(\"stopped\"),",
        1,
        "the person typing a git host's token, who stops it with Ctrl-C: this arm",
    ),
    (
        "crates/cli/src/input.rs",
        "self.lines.lock().await.recv().await",
        1,
        "the person's next line at the terminal; Ctrl-C stops the wait",
    ),
    (
        "crates/cli/src/input.rs",
        "interrupts.recv().await",
        1,
        "Ctrl-C itself, from the person at the terminal",
    ),
    (
        "crates/session/src/turn.rs",
        "if done.changed().await.is_err() {",
        1,
        "a turn stopped at an approval or a question goes on once the person answers; \
         cancelling the turn ends it",
    ),
    (
        "crates/session/src/background.rs",
        "let _ = working.wait_for(|done| *done).await;",
        1,
        "work on a thread ending, bounded as held.rs says of its own wait for it; a watcher \
         waits for it to record an end it noted, and is dropped when the thread is let go",
    ),
    (
        "crates/harness/src/background.rs",
        "_ = kept.changed() => break,",
        1,
        "a thread's next command in the background, or its commands ending in the other arm; \
         the session drops the watcher when it lets the thread go or stops",
    ),
    (
        "crates/session/src/held.rs",
        "if done.changed().await.is_err() {",
        1,
        "work on a thread ending: a turn, whose model call has its idle limit, each tool its \
         own and each wait the person; cancelling it ends it",
    ),
    (
        "crates/session/src/answers.rs",
        "let _ = running.wait_for(|done| *done).await;",
        1,
        "the turn already running on the thread, whose model, tools and waits for the person \
         bound it; cancelling it ends it",
    ),
    // A turn just cancelled, which stops at once: its commands within their grace.
    (
        "crates/session/src/goal.rs",
        "let _ = running.wait_for(|done| *done).await;",
        1,
        "the goal's turn, cancelled the line before, which stops at once",
    ),
    (
        "crates/session/src/server.rs",
        "let _ = finished.wait_for(|done| *done).await;",
        1,
        "the turn cancelled just before, which stops at once",
    ),
    (
        "crates/session/src/tasks.rs",
        "let _ = finished.wait_for(|done| *done).await;",
        1,
        "the task's turn, cancelled just before, which stops at once",
    ),
    (
        "crates/session/src/stopping.rs",
        "let _ = finished.wait_for(|done| *done).await;",
        1,
        "the turns a stopping server stops: given its grace, then interrupted, when they \
         stop at once",
    ),
    (
        "crates/session/src/turn.rs",
        "match ending.wait_for(|ended| *ended).await {",
        1,
        "a turn already ending, whose last events are being written",
    ),
    // A life that ends when it is stopped or closed: a server's, a connection's, a watch's.
    (
        "crates/cli/src/server/serve.rs",
        "_ = terminate.recv() => break None,",
        1,
        "the person's server serves until told to stop: SIGTERM, which ends it",
    ),
    (
        "crates/cli/src/server/serve.rs",
        "_ = interrupt.recv() => break None,",
        1,
        "the person's server serves until told to stop: Ctrl-C, which ends it",
    ),
    (
        "crates/cli/src/server/serve.rs",
        "while clients.join_next().await.is_some() {}",
        1,
        "the socket's clients, aborted the line before",
    ),
    (
        "crates/session/src/http.rs",
        "while connections.join_next().await.is_some() {}",
        1,
        "the connections, aborted the line before",
    ),
    (
        "crates/gateway-service/src/http.rs",
        "while connections.join_next().await.is_some() {}",
        1,
        "the connections, aborted the line before",
    ),
    (
        "crates/computer/src/serve.rs",
        "connections.join_next().await;",
        2,
        "room for one more connection at the cap: one that signs in lasts while its harness \
         is connected, and one that does not is let go at its handshake's limit",
    ),
    (
        "crates/computer/src/waking.rs",
        ".real.lock().await",
        2,
        "a workspace opened on a person's computer, behind the one call opening it: its wake, \
         within WAKE_WAIT, and its open, within its own deadline",
    ),
    (
        "crates/computer/src/serve/socket.rs",
        "connections.join_next().await;",
        1,
        "room for one more connection at the cap: one that signs in lasts while its bridge \
         is open, and one that does not is let go at its handshake's limit",
    ),
    (
        "crates/computer/src/serve.rs",
        "running.join_next().await;",
        1,
        "room for one more request on a connection, each within its own limit (a command's \
         timeout, a read's wait)",
    ),
    (
        "crates/computer/src/serve.rs",
        "while running.join_next().await.is_some() {}",
        1,
        "a closed connection's requests, its commands cancelled the lines before",
    ),
    (
        "crates/session/src/rpc.rs",
        "while requests.join_next().await.is_some() {}",
        1,
        "a closed connection's requests, its watches ended and its turns stopped first",
    ),
    (
        "crates/computer/src/remote.rs",
        "line = queued.recv() => line,",
        1,
        "the connection's writer, until it closes or stops (the arm beside it); each call on it \
         waits within its own deadline",
    ),
    (
        "crates/computer/src/remote.rs",
        "() = shared.stop.notified() => None,",
        1,
        "the stop of the connection's writer, which runs for as long as the connection",
    ),
    (
        "crates/computer/src/serve.rs",
        "while let Some(mut line) = queued.recv().await {",
        1,
        "the connection's writer, which ends once its requests have and the queue closes",
    ),
    (
        "crates/session/src/client/lines.rs",
        "while let Some(mut line) = queued.recv().await {",
        1,
        "the connection's writer, which ends when the connection drops",
    ),
    (
        "crates/session/src/rpc.rs",
        "while let Some(line) = queued.recv().await {",
        1,
        "the connection's writer, which ends when the connection does",
    ),
    (
        "crates/session/src/client/lines.rs",
        "Some(item) = self.items.recv() => Came::Item(item),",
        1,
        "a turn followed on this computer's socket: a turn waiting for the person sends \
         nothing, and a server that goes closes the socket, which ends it",
    ),
    (
        "crates/session/src/rpc/watches.rs",
        "self.end.notified().await;",
        1,
        "a watch's life: it ends when its client unwatches or its connection closes",
    ),
    (
        "crates/session/src/rpc/watches.rs",
        "seen.wait_for(|progress| progress.following).await",
        1,
        "the connection's watch of the thread starting to follow it, or ending",
    ),
    (
        "crates/session/src/rpc/watches.rs",
        ".wait_for(|progress| progress.sent.is_some_and(|sent| sent >= seq))",
        1,
        "the connection's watch writing an event already recorded, or ending",
    ),
    (
        "crates/session/src/lists.rs",
        "self.items.recv().await",
        1,
        "a watch of a list, followed until its client drops it",
    ),
    (
        "crates/session/src/lists.rs",
        "received = changes.recv() => received,",
        1,
        "a list's changes, heard until its watch is dropped or the server stops",
    ),
    (
        "crates/session/src/watch.rs",
        "received = self.notes.recv() => received,",
        1,
        "a thread's notes, heard until it is unwatched or its connection closes",
    ),
    (
        "crates/session/src/access.rs",
        "access.heard = access.sign_ins.changed().await.is_ok();",
        1,
        "a watch's caller's sign-in, heard beside what it follows until it is dropped",
    ),
    (
        "crates/session/src/awake.rs",
        "state.wait_for(",
        2,
        "keeping the computer awake while work runs, for as long as the server does",
    ),
    (
        "crates/memory/src/client.rs",
        "while let Some(ledger) = asked.recv().await {",
        1,
        "the tidy-up queue, until memory is dropped; each tidy-up is a model call within the \
         provider's limits",
    ),
    (
        "crates/computer/src/cancel.rs",
        "let _ = receiver.wait_for(|cancelled| *cancelled).await;",
        1,
        "a cancellation, which every caller races against the work it would cancel",
    ),
    (
        "crates/harness/src/cancel.rs",
        "let _ = receiver.wait_for(Option::is_some).await;",
        1,
        "a cancellation, which every caller races against the work it would cancel",
    ),
    // A limit around it, set elsewhere.
    (
        "crates/computer/src/local/background.rs",
        "status = child.wait() => break status.map_or(Ending::Signalled, exited),",
        1,
        "a command in the background, until a stop, its time limit or its idle limit, the arms \
         beside it",
    ),
    (
        "crates/computer/src/local/background.rs",
        "() = shared.stop.notified() => {",
        1,
        "the stop of a command in the background, whose time and idle limits are the arms \
         beside it",
    ),
    (
        "crates/computer/src/local/background.rs",
        "let _ = ended.wait_for(Option::is_some).await;",
        1,
        "a command in the background ending, which its callers wait for within a read's wait \
         or STOPPING",
    ),
    (
        "crates/computer/src/remote.rs",
        "Some(text) = chunks.recv() => output.write(&text).await,",
        1,
        "a running command's output, beside the timer of its call's deadline",
    ),
    (
        "crates/computer/src/remote.rs",
        "Some((id, ask)) = asks.recv() => self.give(id, &ask, &hosts, credentials).await,",
        1,
        "a running command's credential requests, beside the timer of its call's deadline",
    ),
    (
        "crates/process/src/child.rs",
        "let status = self.inner.wait().await?;",
        1,
        "Child::wait, which every caller bounds: a command's time limit, a stop or a cancel, \
         each checked here where it waits",
    ),
    (
        "crates/process/src/child.rs",
        "if self.inner.wait().await.is_ok() {",
        1,
        "Child::gone, only ever awaited within GRACE in Child::stop",
    ),
    (
        "crates/process/src/waited.rs",
        "tokio::try_join!(child.wait(), written, drained(stdout), drained(stderr))?;",
        1,
        "inside Processes::waited, under the caller's limit",
    ),
    (
        "crates/process/src/within.rs",
        "let _ = child.wait();",
        1,
        "reaps a program just killed, which has ended",
    ),
    (
        "crates/process/src/within.rs",
        "reading.join()",
        2,
        "the readers of a program's output, whose pipes close once its group has ended or been \
         killed at its limit",
    ),
    (
        "crates/process/src/lifeline.rs",
        "match pending.wait_for(Option::is_some).await.as_deref() {",
        1,
        "another start of the helper, which says it is ready or is refused within READY_WITHIN",
    ),
    (
        "crates/mcp/src/server.rs",
        ".wait_for(|state| !matches!(state, State::Starting))",
        1,
        "the server's start, which is ready or fails within its startup limit",
    ),
    (
        "crates/computer/src/worker/kubernetes.rs",
        "let _ = process.join().await;",
        1,
        "the end of the copy into a worker, within COPIED_WITHIN",
    ),
    (
        "crates/cli/src/mcp.rs",
        "while closing.join_next().await.is_some() {}",
        1,
        "the servers closing, each within its exit's limit and then stopped",
    ),
    (
        "crates/cli/src/connected.rs",
        "while let Some(asked) = asking.join_next().await {",
        1,
        "the asks whether each server offers a sign-in, each within ASKING",
    ),
    (
        "crates/eval/src/judging/model.rs",
        "while let Some(joined) = tasks.join_next().await {",
        1,
        "the judge calls, each within the provider's limits",
    ),
    // The work it guards, which has limits of its own.
    (
        "crates/computer/src/worker.rs",
        "let mut done = worker.set_up.lock().await;",
        1,
        "held while the worker's setup runs, each command within SETUP_TIMEOUT_SECS",
    ),
    (
        "crates/computer/src/worker/reconnect.rs",
        "let mut failed = self.failed.lock().await;",
        1,
        "held while the connection to a worker is opened again, within the worker's START_WAIT \
         and the remote computer's deadlines",
    ),
    (
        "crates/cli/src/reread.rs",
        "let mut seen = self.seen.lock().await;",
        1,
        "held while the person's settings are read again from this computer's disk",
    ),
    (
        "crates/cli/src/project_folders.rs",
        "let _keeping = self.keeping.lock().await;",
        1,
        "held while a project's trust is read and recorded, from this computer's disk and the \
         store",
    ),
    (
        "crates/session/src/providers.rs",
        "let _changing = self.inner.changing_models.lock().await;",
        3,
        "held while a provider is connected, refreshed or removed, its models listed within \
         the check's limit",
    ),
    (
        "crates/session/src/reread.rs",
        "let _changing = self.inner.changing_models.lock().await;",
        1,
        "held while the models offered are set again, from settings already read",
    ),
    (
        "crates/cli/src/connected.rs",
        "let _asker = askers.acquire().await.ok()?;",
        1,
        "a permit held only for the ask beside it, within ASKING",
    ),
    (
        "crates/eval/src/judging/model.rs",
        "let _permit = gate.acquire().await;",
        1,
        "a permit held for one judge call, within the provider's limits",
    ),
    (
        "crates/harness/src/files.rs",
        "let _permit = self.fitting.acquire().await;",
        1,
        "a permit held while one image is decoded and scaled, work its size bounds \
         (MAX_SIDE, 256 MiB decoded)",
    ),
    (
        "crates/eval/src/suite.rs",
        "let _permit = permits.acquire_owned().await;",
        1,
        "a permit held for one attempt, within its task's time limit",
    ),
    (
        "crates/eval/src/suite.rs",
        "let _turn = turn.acquire_owned().await;",
        1,
        "a task's turn, held for one attempt of it, within its task's time limit",
    ),
    (
        "xtask/src/cmd.rs",
        "Ok::<_, std::io::Error>((child.wait().await?, output))",
        1,
        "a query within QUERY_WITHIN; a step (a build, the tests, an eval) is the work asked \
         for, stopped by Ctrl-C at the terminal and by the job's time limit in CI",
    ),
];
