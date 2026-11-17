//! glave's memory allocator, counted (`cap`), so a reader of a file a turn made can be held to
//! a limit on every computer (`reading.rs`): what is beneath is the system's allocator, or, in
//! the static Linux builds, mimalloc in place of musl's own, which is slow at the many small
//! allocations reading a thread makes (#682). Glave as anything but a reader has no limit. The
//! budgets benchmark uses this module too, so it measures the allocator that ships.

#[cfg(target_env = "musl")]
type Beneath = mimalloc::MiMalloc;
#[cfg(target_env = "musl")]
const BENEATH: Beneath = mimalloc::MiMalloc;
#[cfg(not(target_env = "musl"))]
type Beneath = std::alloc::System;
#[cfg(not(target_env = "musl"))]
const BENEATH: Beneath = std::alloc::System;

/// Every allocation glave makes, counted against a limit: none until one is set.
#[global_allocator]
pub(crate) static GLOBAL: cap::Cap<Beneath> = cap::Cap::new(BENEATH, usize::MAX);
