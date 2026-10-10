# Storage

How glave keeps what lasts: the event log, blobs, memory and attachments, sealed and scoped. Builds on "Storage" and "Retention" in `events.md`, which this document details.

## Status

Draft, specified ahead of the code (issue #22 and the storage ports): today `key_id` is always empty and payloads are stored as written. Everything below is a contract for the first sealed release.

## Scope

- **Every table and every key carries the organization and the space.** Every storage call takes a scope, and no call can cross scopes. This is already true of the ports.
- **A thread's space never changes.** Promoting a one-off chat into a project starts a new thread in the project's space that continues it (`thread.started` with `fork`). The original stays where it was. Nothing is moved or re-sealed.
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
- Erasure is recorded as an event in the thread it affects, and envelopes and content-free events may outlive it, as `events.md` says. A legal hold pauses all of it.

## On a person's computer

- The whole database file is encrypted too, with its key in the OS keychain, so even the readable envelope members are protected at rest. The method (SQLCipher or page-level encryption) is chosen against the binary-size budget when it is built.
- Headless Linux without a keychain falls back to systemd credentials or the TPM, then to a key file readable only by the person, with a warning.
