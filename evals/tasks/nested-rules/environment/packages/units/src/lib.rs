//! Small numeric helpers.

/// `value` clamped to `low..=high`.
pub fn clamp(value: i64, low: i64, high: i64) -> i64 {
    value.max(low).min(high)
}
