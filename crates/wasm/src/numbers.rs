//! Conversion of JS numbers that the shared argument contract has already validated.
//!
//! Non-number JS values are mapped to NaN before reaching the contract, so the count helper
//! here cannot report a value above the safe-integer range.

const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

#[allow(clippy::cast_precision_loss)]
pub(crate) fn js_count(value: u64) -> f64 {
    value.min(MAX_SAFE_INTEGER) as f64
}
