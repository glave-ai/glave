//! glave's memory allocator where it isn't the system's own. In the static Linux builds,
//! mimalloc, in place of musl's, which is slow at the many small allocations reading a thread
//! makes (#682). On macOS, the system's, counted (`cap`), so a reader of a file a turn made is
//! held to a limit (`reading.rs`), which macOS enforces no other way; Linux holds a reader with
//! `RLIMIT_DATA` instead, so a Linux server pays nothing per allocation. The budgets benchmark
//! uses this module too, so it measures the allocator that ships.

#[cfg(target_env = "musl")]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

/// Every allocation glave makes on macOS, counted against a limit: none until a reader sets one.
#[cfg(target_os = "macos")]
#[global_allocator]
pub(crate) static GLOBAL: cap::Cap<std::alloc::System> =
    cap::Cap::new(std::alloc::System, usize::MAX);
