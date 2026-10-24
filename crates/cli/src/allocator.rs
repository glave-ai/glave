//! The memory allocator of glave's static Linux builds: mimalloc, in place of musl's own,
//! which is slow at the many small allocations reading a thread makes (#682). Elsewhere glave
//! uses the system's allocator. The budgets benchmark uses this module too, so it measures
//! the allocator that ships.

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;
