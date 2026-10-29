# Agents working on app

- Every public function has a `///` doc comment and `#[must_use]`.
- No `unwrap()`, `expect()` or `panic!` in `src/`.
- Tests live in `tests/`, one file per function, named after it (`tests/left.rs` tests `left`).
- Every new public function gets a line under `## Unreleased` in `CHANGELOG.md`:
  ``- Added `name`.``
