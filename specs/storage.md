# Storage

How glave keeps what lasts: the event log, blobs, memory and attachments, sealed and scoped. Builds on "Storage" and "Retention" in `events.md`, which this document details.

## Status

Partly built (issue #22). **Built:** the key hierarchy, sealing and erasure in `glave-keys`; both stores (SQLite and Postgres) seal every new event payload and blob and open them on read, and rows written before sealing (NULL `key_id`) still read as written; blobs are stored under their keyed name with the threads that use them; the person's root in the OS keychain, or a key file (0600) when there is no keychain or `GLAVE_HOME` moves glave's folder. **Archive and delete:** `thread.archived`, `thread.unarchived` and `thread.deleted` events (`events.md`) decide whether a thread is shown. Per-thread erasure is not possible with today's keys (data keys are per space and month), so delete is archive plus a permanent marker: the thread is hidden everywhere, its content stays sealed in the log, and `thread/list` with `show: deleted` lists the threads waiting for erasure. **Spaces as records** (issue #64): both stores keep each space's record, its members and the folder keys that lead to it (schema 3 in each), pass the spaces conformance suite, and find a thread's scope by its id alone (`EventLog::locate`, over an index on `events.thread`). Upgrading a local database turns each folder's space into a project record named after the folder, owned by no one until the local person opens glave (`Store::claim_spaces`), and each stand-in personal space (`glave:personal/<person>`) into that person's personal space, its memories kept. **The threads view** (`events.md`, "Storage"): a `threads` table in both stores (schema 4 in each), content-free: a thread's title is read from the sealed event that gives it, so erasing the content erases the title. **Which store:** the local SQLite file in glave's folder by default; `[store] url` in the person's or server's `config.toml`, or `GLAVE_STORE_URL`, names a Postgres database instead, for the CLI and `glave serve` alike (never a project's file). A database on another computer is reached over TLS (rustls, the operating system's verifier) unless its URL says `sslmode=disable`; the organization's id is kept in the database (`glave_settings`) the first time. **Projects' folders** (#294): a project's folder in its space ledger (`space.folder.set`), one per project; names unique per owner in both stores and in memory, kept by the write itself (SQLite holds the database for the write, Postgres an advisory lock per owner), so names two projects already shared stay as they are until one is renamed. **Not yet:** `glave gateway serve` on Postgres, promotion, recording erasure as an event, legal holds, KMS roots, encrypting the whole local file, and data keys per thread.

Settled while building, within this contract:

- An event is sealed under the data key of the month of its `at`, in UTC; a blob under the month it is written in.
- A space key has two halves: one wraps data keys and is replaced on every rotation; the other names blobs and is carried forward, so blob names stay stable until the space is erased.
- The data store keeps one wrapped key per organization and one record of wrapped keys per space, replaced whole by compare-and-swap on a revision, so a rotation is a single atomic write. A destroyed key's entry stays, emptied, so numbers are never reused.
- A blob's associated data is a `glave/blob/v1` tag, organization, space, stored name and `key_id`, length-prefixed like an event's.
- Unwrapped keys are cached per process. Another process (a second server on the same Postgres) keeps a destroyed key until it leaves its cache.
- **Per-item keys for accounts:** each `provider.connected` and `credential.connected` (`events.md`, "The accounts ledger") is sealed under the account's own data key, `v1/<space>/provider/<account>/<n>` (named for model accounts, which had it first), in the person's personal space. Removing or replacing the account destroys that key and rotates the space key at once, so neither the store nor a backup can open the key or token again; such a payload reads as `{}`, as a forgotten memory's does.
- **Per-item keys, first for memory** (`memory.md`): `memory.saved` and `memory.edited` are sealed under the memory's own data key, `v1/<space>/memory/<memory>/<n>`, kept in the space's record beside the month keys. Forgetting destroys it alone, at once, without a rotation; the space key is rotated afterwards (`rotate_space_key`, which rotates only when a key was destroyed under the live space key), batching the forgets since the last rotation. An event sealed under a destroyed memory key reads with `data` `{}`; one under a destroyed month key still fails the read.

## Spaces

A space is a record, not a folder: an id (UUID v7), a kind, a name, an owner, members, and when it was made and last changed.

| Kind | Is | Owner |
| --- | --- | --- |
| `project` | the work of a team, a person or an agent, with its folder or none | whoever made it; later a team or an agent |
| `personal` | a person's own space, named Chats: their one-off chats and their personal memory. Each person has exactly one in each organization, made the first time it is needed | that person |

- **Who works in a space:** its owner and its members (principals, in order). Who may do what is decided from that on every request (`session.md`, "Spaces and who may do what"). Only the owner renames it.
- **A project's name is unique among its owner's projects,** ignoring case and the spaces around it, and the data store keeps it so in the same transaction as the write: a new project whose name is taken gets the next free one (`web`, `web 2`, `web 3`), and renaming one to a taken name is refused. Projects of different owners may share a name.
- **A folder leads to a space; it is not one.** The data store keeps folder keys: the folder's canonical path (`path:<path>`), and for a git repository its `origin` remote as `.git/config` names it (`remote:<url>`), as `trust.toml` recognises a project. A folder is looked up by path first, then by remote, so a moved or re-cloned project, or a second checkout of it, finds its space and keeps its history and `--continue`. A folder no key leads to gets a new project space, owned by the person who opened it, named after the folder. Every key of the folder then leads to that space, unless it already led elsewhere.
- **A project is its identity; its folder is an attachment.** The project (its name, its instructions, its threads) stays what it is wherever its files are. Its folder records two things apart: where the files come from (a repository, by the address it is cloned from, or a folder of the person's with no repository behind it) and where they are checked out (a computer, by the id it keeps across runs, and the path there). So a project can be checked out again from its source alone, on another computer or after one is reset. The record is `space.folder.set` in the space's ledger (`events.md`), sealed like any event; each folder has an id of its own and its latest record holds, so a second folder later is one more id. A project has one folder for now. A folder that moved on its computer is given its new place (`session.md`), which then leads to the project too; a removed project keeps its record, its ledger saying it was removed (`space.removed`), until a folder is set for it again.
- **A project's folder leads back to it:** making a project from a folder keeps that folder's keys (its path and its repository's `origin`) as glave run in it does, so either way finds the same project, and glave run in a folder makes that folder its project's checkout.
- **Where new projects' folders go:** a repository cloned, or a new empty project, is made in `~/glave/<name>`, named after the project (`My App` is `~/glave/My-App`, then `My-App-2` when that is taken, never a folder that was there). It is visible because the files are the person's work: they find and open it as they would any project of theirs. It is outside glave's folder (`~/.glave`) on purpose: wiping or moving glave's data never takes work with it, and the sandbox hides glave's folder from commands (`sandbox.md`), while a worktree's commands must read the repository it came from. A glave whose folder `GLAVE_HOME` moves keeps its projects beside it, in `<GLAVE_HOME>-projects`, so separate homes (tests, a second profile) never touch the person's.
- **Every thread has a space from birth**, and its `for` is the person it works for (`events.md`, "Who acted"). A thread in a project with a folder works there; any other (a one-off chat, or a project with no folder) gets a scratch folder (`computers.md`).

## Scope

- **Every table and every key carries the organization and the space.** Every storage call takes a scope, and no call can cross scopes. This is already true of the ports.
- **A thread's space never changes.** Nothing stored is ever moved or rewritten in place.
- **Promoting a one-off chat into a project copies, then archives.** The chat's history is written as a new thread in the project's space, sealed under the project's keys, by the same path that writes any new thread. Its first event records where it came from. The original in the personal space is then archived as "moved to the project" and deleted (its keys destroyed) 30 days later, so a mistaken promotion can still be undone. Archived originals never appear in search, thread lists or memory; only the project's copy does.
- **One-off chats live in each person's personal space** (Chats, as clients call it), made the first time they need it. Every thread has a space from birth.
- **A thread id is resolved to its scope by one small index:** `(thread) → (organization, space)`, kept by the data store and written when a thread starts. It is the only lookup that takes a bare thread id. It answers only "which scope", and every read that follows is scoped as usual and checked against the caller.
- **Exports carry their scope** in a header line, and an import names the scope it goes into. An import never takes the scope from the file.

## Keys

| Level | Holds | Kept |
| --- | --- | --- |
| Root | wraps organization keys | the OS keychain for a person; a KMS (AWS KMS, Vault transit, GCP KMS, Azure Key Vault) for an organization |
| Organization key | wraps space keys | wrapped by the root, in the data store |
| Space key | wraps data keys; names blobs | wrapped by the organization key, versioned |
| Data key | seals content | one per space per month, wrapped by the space key |

- The data store holds only wrapped keys. The root is called only to unwrap an organization key; unwrapped keys are cached in memory, with a size cap.
- **`key_id`** names the data key that sealed a payload: `v1/<space>/<YYYY-MM>/<n>`. Data keys per thread can come later, under the same field, without re-sealing anything.

## Sealed format

- **Algorithm:** XChaCha20-Poly1305. Its 24-byte random nonces make nonce reuse a non-issue at any volume.
- **Stored:** `key_id`, plus the nonce followed by the ciphertext and tag, in place of the payload bytes.
- **Associated data:** the envelope version, organization, space, thread, `seq`, `type`, `v` and `key_id`, in that order, length-prefixed. That binds a sealed payload to its row, so it cannot be swapped between rows, threads or scopes.
- **What stays readable:** the envelope members and `key_id`. Never content: `data` is always sealed.
- A record of the algorithm is part of `key_id`'s version prefix, so a future algorithm is a new prefix, not a migration.

## Blobs

- Blobs (large outputs, attachments, configurations) are sealed under the data key of the space and month they are written in.
- **On the wire and in events** a blob is still named by its SHA-256, as `events.md` defines.
- **In storage** its object name is HMAC-SHA-256 of that hash under the space key. Someone with access to the bucket cannot check whether a known file is stored.
- **Each blob's referencing threads are recorded,** so erasing a thread can remove the blobs only it used. Blobs are shared only within a space, never across spaces.

## Search index

`recall` (`memory.md`) searches past conversations with the store's own full-text engine: SQLite FTS5 (the `event_search` table), Postgres full-text search (`event_search`, a `tsvector` with a GIN index).

- **What is indexed:** the text of each `message.added` and each successful `tool.call.completed`, as normalised words (`glave_ports::search`: lowercase letters and digits, a plural's `s` dropped, common words left out, at most 2,048 distinct words an event). The index row is written in the same transaction as its event. Nothing else is indexed, memories included: `recall` matches those in the memory view, so forgetting one leaves nothing behind in the index.
- **No word in the clear.** Each word is stored as HMAC-SHA-256 under the space key's naming half (the half that names blobs), labelled `glave/search/v1`, its first 16 bytes in hex. The same words match until the space is erased; another space's never do; once the space key is destroyed, nothing in the index can be matched again.
- **Results are positions,** `(thread, seq)`, best match first (BM25 in SQLite, `ts_rank` in Postgres), then newest first. The event itself is read and opened as any other read; one whose key was destroyed no longer opens and is skipped.
- **Before schema 5** (SQLite and Postgres) nothing was indexed: conversations from before the index are not found.

## Erasure

- **A space** is erased by destroying its space key. Everything sealed under it, in the store and in every backup, becomes unreadable at once.
- **A month, or later a thread,** is erased by destroying its data key and then rotating the space key: the surviving data keys are re-wrapped under a new space-key version, and the old version is destroyed. Backups that still hold the old wrapped data key can no longer unwrap it. Without the rotation, destroying a data key alone does not reach backups.
- **Deleting a thread** records `thread.deleted` and waits for erasure of its content. Until data keys exist per thread, that erasure is a month's key plus a rotation, or a space's, which loses more than the one thread, so it is not done automatically.
- Erasure is recorded as an event in the thread it affects, and envelopes and content-free events may outlive it, as `events.md` says. A legal hold pauses all of it.

## On a person's computer

- The whole database file is encrypted too, with its key in the OS keychain, so even the readable envelope members are protected at rest. The method (SQLCipher or page-level encryption) is chosen against the binary-size budget when it is built.
- Headless Linux without a keychain falls back to systemd credentials or the TPM, then to a key file readable only by the person, with a warning.
