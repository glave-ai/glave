//! Squares by lookup.

mod table;

pub use table::SQUARES;

/// `n * n`, for `n` below 5000.
pub fn square(n: usize) -> Option<u32> {
    SQUARES.get(n).copied()
}
