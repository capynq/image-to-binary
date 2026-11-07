//! Drift guards against the browser copies of the shared limits. CI runs the root workspace, so
//! these tests fail when either side changes alone.

use super::limits::{DEFAULT_PIXEL_BUDGET, MAX_ENCODED_BYTES, MAX_SOURCE_PIXELS};

const PROTOCOL_TS: &str = include_str!("../../../../frontend/src/processing/protocol.ts");
const WORKER_HOST_TS: &str = include_str!("../../../../frontend/src/processing/workerHost.ts");

/// Evaluates the right-hand side of `export const NAME = …` or `const NAME = …` in TypeScript
/// source. Only integer literals with `_` separators and `*` products are accepted.
fn ts_constant(source: &str, name: &str) -> u64 {
    let prefixes = [
        format!("export const {name} = "),
        format!("const {name} = "),
    ];
    let line = source
        .lines()
        .map(str::trim)
        .find_map(|line| {
            prefixes
                .iter()
                .find_map(|prefix| line.strip_prefix(prefix.as_str()))
        })
        .unwrap_or_else(|| panic!("{name} is not declared in the browser source"));
    line.split('*')
        .map(|factor| {
            factor
                .trim()
                .replace('_', "")
                .parse::<u64>()
                .unwrap_or_else(|_| panic!("{name} is not a plain integer expression"))
        })
        .product()
}

#[test]
fn the_browser_source_pixel_ceiling_matches_the_shared_limit() {
    assert_eq!(
        ts_constant(PROTOCOL_TS, "MAX_SOURCE_PIXELS"),
        MAX_SOURCE_PIXELS
    );
}

#[test]
fn the_browser_encoded_ceiling_matches_the_shared_limit() {
    assert_eq!(
        ts_constant(PROTOCOL_TS, "MAX_ENCODED_BYTES"),
        MAX_ENCODED_BYTES
    );
}

#[test]
fn the_browser_pixel_budget_matches_the_shared_budget() {
    assert_eq!(
        ts_constant(WORKER_HOST_TS, "DEFAULT_PIXEL_BUDGET"),
        DEFAULT_PIXEL_BUDGET
    );
}

#[test]
fn the_drift_guard_parses_the_declared_forms() {
    assert_eq!(
        ts_constant("export const A = 256 * 1024 * 1024", "A"),
        256 * 1024 * 1024
    );
    assert_eq!(ts_constant("  const B = 131_072", "B"), 131_072);
}
