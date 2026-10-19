# Invariants

What must stay true of glave whatever changes, each with the test that proves it. The other specs say how each piece works; this one gathers the promises that span them, so a reviewer can check a change against one list.

## Status

Written from the architecture pass of 2026-10-18 (#395). Each invariant says whether it holds today: **holds**, **partly** (with the gap) or **not yet**, and names the issue that closes a gap. A change that touches an invariant says so in its pull request and keeps its test; one that closes a gap moves its entry to **holds** and names the new test.

## Where state lives

Every invariant below follows this rule (decided 2026-10-18).

- **State at rest lives in the store,** the event log on SQLite or Postgres, and nowhere else: no Redis, no second system. A file under `GLAVE_HOME` holds a decision only while it is listed below as a gap.
- **Work in flight runs under a lease recorded in the store:** a row with a holder and an expiry, on the database's clock, the same on SQLite and Postgres.
- **LISTEN/NOTIFY only wakes.** Another process may be woken by it, but correctness never depends on it: the log is the truth.
- **OS file locks only for machine resources,** such as `server.lock` for the socket and port.
- **Files only for what cannot live in the store:** `root.key`, which unlocks it, and this machine's own state, never shared through an organization's database (`computer`, `run/processes/`, `serve.token`, `serve.url`, `open.html`, `worktrees-checked`, the locks), and each worktree's record beside it (`<worktree>.json`), which describes the disk of the computer that holds the worktree: a remote computer cannot reach the store, and who owns a worktree is already in the log (`workspace.bound`). The write-once ones, `root.key` and `computer`, are created whole or not at all (`glave_ports::file`); the rest are written anew at each start or check, so a torn one is rewritten, and a process record that cannot be read is skipped. A worktree record is replaced whole (`glave_ports::file::replace`); a torn one is read as none and written whole at the next open or hold. Proved by `crates/ports/src/file.rs` `a_create_stopped_at_any_step_leaves_no_file_or_a_whole_one`; `crates/keys/src/root.rs` `a_key_file_whose_creation_stopped_at_any_step_opens_cleanly`; `crates/cli/src/identity.rs` `a_computer_id_whose_creation_stopped_at_any_step_is_read_or_made_whole`; `crates/cli/tests/serve.rs` `torn_token_and_address_files_are_written_anew_at_start_and_work`; `crates/cli/src/worktrees/cleanup/tests.rs` `a_torn_check_stamp_is_checked_again_and_written_whole`; `crates/process/src/processes/tests.rs` `an_entry_that_cannot_be_read_is_skipped_and_the_others_are_still_stopped`; `crates/computer/src/worktrees/tests.rs` `a_torn_record_is_read_as_none_and_written_whole_again`.
- **No deltas in the database.** Token-by-token deltas go to watchers live and are never stored; the events they add up to are.

## The invariants

Tests are named by file and function. A conformance suite is a function in `crates/ports/src/conformance.rs` (or `conformance/`) that every adapter's own test runs, so it binds the in-memory store, SQLite and Postgres alike.

1. **The log is the truth, and everything in memory is a cache of it**: views, thread slots, ledgers, memory books, meter streams.
   - *Partly.* Within one process it holds. State with no log behind it remains: idempotency keys and in-process gateway budgets (#410), pending approvals and questions (PR 5 of `one-server.md`, #294), queued messages (#335), and the context window a refusal stated for each model. Worktree records stay on the computer that holds the worktree, as the computer's own state (above). The local person's and their agent's ids are kept in the store, and so are project trust (`crates/cli/tests/postgres.rs` `two_homes_on_one_database_read_one_trust_record`) and browser sessions (`crates/store-postgres/tests/postgres.rs` `a_browser_signed_in_on_one_server_is_signed_in_on_another_on_the_database`) (`crates/store-postgres/tests/postgres.rs` `servers_on_one_database_racing_to_make_the_identity_all_get_the_first`).
   - *Proved by:* `conformance::thread_views` ("a rebuilt view must be stored"); `crates/session/tests/server.rs` `a_fresh_server_continues_a_thread_from_the_log_alone`; `crates/harness/tests/turn.rs` `a_resumed_thread_continues_where_it_left_off`; `crates/memory/tests/loading.rs` `returning_after_changes_gets_a_note_and_resuming_rebuilds_the_same_requests`; `crates/gateway-service/tests/surfaces.rs` `each_caller_has_their_own_budget_and_a_restart_does_not_reset_it`; `crates/store-sqlite/src/tests.rs` `views_rebuilt_from_the_log_equal_the_kept_ones`.
2. **One writer per thread, enforced by the store**: a thread's `seq` is gap-free and unique, checked in the append's transaction.
   - *Holds* at the store. Racing writers are tested on Postgres only (#405 moves the case into the shared suite).
   - *Proved by:* `conformance::event_log` (conflict, gap, all or nothing); `crates/store-postgres/tests/postgres.rs` `writers_racing_for_one_position_leave_exactly_one_winner`.
3. **One harness at a time per thread, under a lease**; another server is refused (`-32011`).
   - *Not yet:* no lease exists, so only the store's check in 2 stands between two servers (#410).
   - *Proved by:* no test yet.
4. **No side effect runs twice without the person**: an interrupted call is closed, never redone, and a retry with the same key never starts a second turn.
   - *Partly.* It holds in one process. Across servers, loading a thread can close a turn another is running, and keys kept in memory let a retry start a second turn (#410).
   - *Proved by:* `crates/harness/tests/turn.rs` `resuming_after_a_crash_closes_what_was_left_open`; `crates/session/tests/server.rs` `a_retry_with_the_same_key_follows_the_running_turn_instead_of_starting_one` (one process).
5. **Reading never writes.**
   - *Not yet:* `thread/get` loads the thread, and loading repairs open turns by appending to them (#410, with PR 5 of `one-server.md`).
   - *Proved by:* no test yet; one needs two servers on one store.
6. **Scope is enforced by the store on every query.** Only `locate` takes a bare thread id.
   - *Holds* for the log, blobs, views and search. Spaces are asked by organization, and membership is checked by the session.
   - *Proved by:* `conformance::event_log` ("another scope must not see this thread"); `conformance::blob_store`; `conformance::thread_views` ("views must not cross scopes"); `conformance::search` ("a search must stay inside its scope"); `crates/session/tests/spaces.rs` `one_person_can_neither_read_nor_answer_anothers_threads`.
7. **In-process and remote are one code path**: the same scenarios on every transport, the same suite on every computer.
   - *Partly.* It holds for the session API on all four transports and for local and remote computers; workers (`WorkerComputers`) do not run the computers suite (#405).
   - *Proved by:* `crates/session/tests/conformance.rs` (in-process, stdio, Unix socket, HTTP and SSE); `conformance::computers`, run by `crates/computer/src/local/tests.rs` `this_machine_passes_the_computers_suite`, `crates/computer/tests/remote.rs` `a_computer_across_a_pipe_passes_the_suite`, `a_computer_over_tcp_passes_the_suite_once_signed_in`, `a_computer_over_tls_passes_the_suite_with_its_certificate_pinned`, and `crates/cli/tests/computer.rs` `the_binary_serves_a_computer_that_passes_the_suite`.
8. **Secrets never reach logs, clients, `Debug`, the model or a command's environment.**
   - *Holds,* as far as tested.
   - *Proved by:* `crates/cli/tests/telemetry.rs` `the_log_has_spans_and_metrics_and_never_content`, `a_collector_gets_spans_and_metrics_and_never_content`; `crates/session/tests/http.rs` `the_signed_in_caller_is_recorded_and_the_token_never_is`; `crates/session/tests/credentials.rs` `a_token_never_shows_in_a_request_s_debug`; `crates/session/tests/providers.rs` `a_key_never_shows_in_a_request_s_debug`; `crates/computer/src/local/confinement_tests.rs` `commands_never_see_secret_variables`; `crates/computer/tests/remote.rs` `a_computer_across_a_pipe_keeps_secret_variables_from_commands`, `a_computer_over_tcp_keeps_secret_variables_from_commands`; `cargo xtask secrets`.
9. **Content at rest is sealed,** bound to its row by associated data, so a sealed payload moved to another row does not open.
   - *Holds* in both stores, tested per store rather than in the ports suite, so a third store is held to nothing (#405).
   - *Proved by:* `crates/store-sqlite/src/tests/sealing.rs` `payloads_are_sealed_in_the_file_and_read_back_as_written`, `a_sealed_payload_moved_to_another_row_thread_or_space_does_not_open`; `crates/store-postgres/tests/sealing.rs` `payloads_are_sealed_and_a_moved_payload_does_not_open`; `glave_keys::conformance::keys`, run by `crates/keys/src/keys/tests.rs` `in_memory_keys_pass_the_conformance_suite` and both stores.
10. **Ledgers are glave's own streams**: never listed or read as threads; any process may append to one, and a conflict retries.
    - *Holds.*
    - *Proved by:* `crates/session/tests/schedules.rs` `the_task_ledger_is_never_listed_or_read_as_a_thread`; `conformance::thread_views` ("a stream without thread.started must have no view").
11. **Records from outside never change the conversation**: only glave's own record kinds are taken in, and a message from outside is a conflict.
    - *Holds.*
    - *Proved by:* `crates/session/tests/outside.rs` `glaves_own_records_are_taken_in_and_the_model_is_sent_the_same`, `a_message_from_outside_is_still_a_conflict`.
12. **Waits are durable**: an approval or a question survives a restart and is answered from the log.
    - *Not yet:* a turn waiting for the person is interrupted at a stop (PR 5 of `one-server.md`, #294).
    - *Proved by:* no test yet; PR 5 adds them to the conformance suite.
13. **One firer per schedule and one recoverer per task.**
    - *Partly:* only per `GLAVE_HOME`, by `server.lock`. Two servers on one store both fire every schedule, and each interrupts the other's running tasks (#410).
    - *Proved by:* `crates/session/tests/tasks.rs` `recovering_tasks_covers_every_space` (one process).
14. **Every spawned task and process has an owner** that awaits or stops it.
    - *Holds* for every `tokio::spawn` read in the pass, and for every process, which starts only through glave-process.
    - *Proved by:* `cargo xtask spawns` for processes; nothing checks tasks.
15. **Everything persisted or sent carries a version.**
    - *Partly:* events, the session protocol, the computer protocol, the process ledger and worktree records do (a worktree record with no version is read as version 1, and one of a newer version is refused). The `computer` file is one id, created once, with no version.
    - *Proved by:* `crates/event/tests/fixtures.rs` `every_known_version_has_a_fixture_that_decodes`, `latest_versions_encode_to_exactly_the_saved_bytes`, `every_fixture_is_a_known_version`; `crates/store-sqlite/src/tests.rs` `a_newer_schema_is_refused`; `crates/computer/src/worktrees/tests.rs` `a_record_of_a_newer_version_is_refused_plainly`.
16. **Time and randomness are injected.**
    - *Partly:* library code in glave-computer and glave-process still reads the real clock (a lint in #366). Randomness that fails is an error, never a weaker id.
    - *Proved by:* nothing enforces injection; tests use `glave_ports::testing::ManualClock`. `crates/harness/tests/turn.rs` `a_random_source_that_fails_makes_no_thread`.
