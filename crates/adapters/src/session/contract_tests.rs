//! Drift guards against the browser copies of the shared limits. CI runs the root workspace, so
//! these tests fail when either side changes alone.

use image_to_binary_core::MAX_RAMP_LENGTH;

use super::args::{
    parse_ramp, DEFAULT_RAMP, RAMP_EMPTY_MESSAGE, RAMP_NOT_PRINTABLE_MESSAGE, RAMP_TOO_LONG_MESSAGE,
};
use super::limits::{DEFAULT_PIXEL_BUDGET, MAX_ENCODED_BYTES, MAX_SOURCE_PIXELS};

const PROTOCOL_TS: &str = include_str!("../../../../frontend/src/processing/protocol.ts");
const WORKER_HOST_TS: &str = include_str!("../../../../frontend/src/processing/workerHost.ts");
const RAMP_VECTORS: &str = include_str!("ramp_vectors.tsv");

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

#[test]
fn the_browser_ramp_limit_matches_the_shared_limit() {
    assert_eq!(
        ts_constant(PROTOCOL_TS, "MAX_RAMP_LENGTH"),
        u64::try_from(MAX_RAMP_LENGTH).unwrap()
    );
}

#[test]
fn the_browser_default_ramp_matches_the_shared_default() {
    let line = PROTOCOL_TS
        .lines()
        .map(str::trim)
        .find_map(|line| line.strip_prefix("export const DEFAULT_RAMP = "))
        .unwrap_or_else(|| panic!("DEFAULT_RAMP is not declared in the browser source"));
    assert_eq!(line, format!("'{DEFAULT_RAMP}'"));
}

/// The ramp failure messages must read the same in the browser and the native shell. The native
/// side is the production constant, so this pin fails when either side changes alone.
#[test]
fn the_ramp_failure_messages_match_in_both_shells() {
    assert!(PROTOCOL_TS.contains(&format!("'{RAMP_EMPTY_MESSAGE}'")));
    assert!(PROTOCOL_TS.contains(&format!("'{RAMP_NOT_PRINTABLE_MESSAGE}'")));
    assert!(PROTOCOL_TS.contains("`ramp must be at most ${MAX_RAMP_LENGTH} characters`"));
    assert_eq!(
        RAMP_TOO_LONG_MESSAGE,
        format!("ramp must be at most {MAX_RAMP_LENGTH} characters")
    );
}

/// One row of the vector file: a ramp as the shells receive it and the expected outcome, either
/// `ok` or the exact failure text.
struct RampVector {
    name: String,
    expected: String,
    ramp: String,
}

/// Decodes the `\uXXXX` escapes the TSV uses; each escape is one UTF-16 code unit.
fn decode_escapes(escaped: &str) -> String {
    let mut units: Vec<u16> = Vec::new();
    let mut rest = escaped;
    while let Some(index) = rest.find("\\u") {
        units.extend(rest[..index].encode_utf16());
        let hex = &rest[index + 2..index + 6];
        units.push(u16::from_str_radix(hex, 16).expect("four hex digits"));
        rest = &rest[index + 6..];
    }
    units.extend(rest.encode_utf16());
    String::from_utf16(&units).expect("well-formed UTF-16")
}

fn ramp_vectors() -> Vec<RampVector> {
    RAMP_VECTORS
        .lines()
        .skip(1)
        .map(|line| {
            let mut columns = line.splitn(3, '\t');
            RampVector {
                name: columns.next().unwrap_or_default().to_owned(),
                expected: columns.next().unwrap_or_default().to_owned(),
                ramp: decode_escapes(columns.next().unwrap_or_default()),
            }
        })
        .collect()
}

/// The shared ramp vectors. The browser reads the same file, so both sides must agree on every row.
#[test]
fn the_native_ramp_validation_matches_the_shared_vectors() {
    let vectors = ramp_vectors();
    assert_eq!(vectors.len(), 14);
    for vector in vectors {
        let outcome = match parse_ramp(&vector.ramp) {
            Ok(_) => "ok".to_owned(),
            Err(error) => error.message(),
        };
        assert_eq!(outcome, vector.expected, "{}", vector.name);
    }
}
