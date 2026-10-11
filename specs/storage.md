# Storage

How glave keeps what lasts: the event log, blobs, memory and attachments, sealed and scoped. Builds on "Storage" and "Retention" in `events.md`, which this document details.

## Status

Partly built (issue #22). **Built:** the key hierarchy, sealing and erasure in `glave-keys`; both stores (SQLite and Postgres) seal every new event payload and blob and open them on read, and rows written before sealing (NULL `key_id`) still read as written; blobs are stored under their keyed name with the threads that use them; the person's root in the OS keychain, or a key file (0600) when there is no keychain or `GLAVE_HOME` moves glave's folder. **Archive and delete:** `thread.archived`, `thread.unarchived` and `thread.deleted` events (`events.md`) decide whether a thread is shown. Per-thread erasure is not possible with today's keys (data keys are per space and month), so delete is archive plus a permanent marker: the thread is hidden everywhere, its content stays sealed in the log, and `thread/list` with `show: deleted` lists the threads waiting for erasure. **Not yet:** the scope index, promotion, recording erasure as an event, legal holds, KMS roots, encrypting the whole local file, and data keys per thread.

Settled while building, within this contract:

- An event is sealed under the data key of the month of its `at`, in UTC; a blob under the month it is written in.
- A space key has two halves: one wraps data keys and is replaced on every rotation; the other names blobs and is carried forward, so blob names stay stable until the space is erased.
- The data store keeps one wrapped key per organization and one record of wrapped keys per space, replaced whole by compare-and-swap on a revision, so a rotation is a single atomic write. A destroyed key's entry stays, emptied, so numbers are never reused.
- A blob's associated data is a `glave/blob/v1` tag, organization, space, stored name and `key_id`, length-prefixed like an event's.
- Unwrapped keys are cached per process. Another process (a second server on the same Postgres) keeps a destroyed key until it leaves its cache.
- **Per-item keys, first for memory** (`memory.md`): `memory.saved` and `memory.edited` are sealed under the memory's own data key, `v1/<space>/memory/<memory>/<n>`, kept in the space's record beside the month keys. Forgetting destroys it alone, at once, without a rotation; the space key is rotated afterwards (`rotate_space_key`, which rotates only when a key was destroyed under the live space key), batching the forgets since the last rotation. An event sealed under a destroyed memory key reads with `data` `{}`; one under a destroyed month key still fails the read.

## Scope

- **Every table and every key carries the organization and the space.** Every storage call takes a scope, and no call can cross scopes. This is already true of the ports.
- **A thread's space never changes.** Nothing stored is ever moved or rewritten in place.
- **Promoting a one-off chat into a project copies, then archives.** The chat's history is written as a new thread in the project's space, sealed under the project's keys, by the same path that writes any new thread. Its first event records where it came from. The original in the Inbox is then archived as "moved to the project" and deleted (its keys destroyed) 30 days later, so a mistaken promotion can still be undone. Archived originals never appear in search, thread lists or memory; only the project's copy does.
- **One-off chats live in each person's Inbox space,** created with their personal organization. Every thread has a space from birth.
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

## Erasure

- **A space** is erased by destroying its space key. Everything sealed under it, in the store and in every backup, becomes unreadable at once.
- **A month, or later a thread,** is erased by destroying its data key and then rotating the space key: the surviving data keys are re-wrapped under a new space-key version, and the old version is destroyed. Backups that still hold the old wrapped data key can no longer unwrap it. Without the rotation, destroying a data key alone does not reach backups.
- **Deleting a thread** records `thread.deleted` and waits for erasure of its content. Until data keys exist per thread, that erasure is a month's key plus a rotation, or a space's, which loses more than the one thread, so it is not done automatically.
- Erasure is recorded as an event in the thread it affects, and envelopes and content-free events may outlive it, as `events.md` says. A legal hold pauses all of it.

## On a person's computer

- The whole database file is encrypted too, with its key in the OS keychain, so even the readable envelope members are protected at rest. The method (SQLCipher or page-level encryption) is chosen against the binary-size budget when it is built.
- Headless Linux without a keychain falls back to systemd credentials or the TPM, then to a key file readable only by the person, with a warning.
