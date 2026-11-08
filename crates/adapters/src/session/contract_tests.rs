//! Drift guards against the browser copies of the shared limits. CI runs the root workspace, so
//! these tests fail when either side changes alone.

use image_to_binary_core::{
    MAX_CELL_SIZE, MAX_CONTRAST, MAX_FONT_SIZE, MAX_RAMP_LENGTH, MAX_THRESHOLD, MIN_CELL_SIZE,
    MIN_CONTRAST, MIN_FONT_SIZE, MIN_THRESHOLD,
};
use image_to_binary_processing::RenderStyle;

use super::job::OutputKind;

use super::args::{
    output_kind, parse_ramp, preview_settings_styled, StyleArgs, TextArg, BACKGROUND_MESSAGE,
    COLOR_MODE_MESSAGE, CONTRAST_INTEGER_MESSAGE, CONTRAST_RANGE_MESSAGE, DEFAULT_RAMP,
    FOREGROUND_MESSAGE, INVERT_BOOLEAN_MESSAGE, OUTPUT_KIND_MESSAGE, RAMP_EMPTY_MESSAGE,
    RAMP_NOT_PRINTABLE_MESSAGE, RAMP_TOO_LONG_MESSAGE,
};
use super::limits::{DEFAULT_PIXEL_BUDGET, MAX_ENCODED_BYTES, MAX_SOURCE_PIXELS};

const PROTOCOL_TS: &str = include_str!("../../../../frontend/src/processing/protocol.ts");
const BOUNDS_TS: &str = include_str!("../../../../frontend/src/processing/bounds.ts");
const WORKER_HOST_TS: &str = include_str!("../../../../frontend/src/processing/workerHost.ts");
const RAMP_VECTORS: &str = include_str!("ramp_vectors.tsv");
const STYLE_VECTORS: &str = include_str!("style_vectors.tsv");

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

/// Evaluates `export const NAME = <integer>` in the bounds file, allowing a leading minus sign
/// for the signed contrast bounds. Missing or non-integer declarations panic.
fn ts_integer(source: &str, name: &str) -> i64 {
    let prefix = format!("export const {name} = ");
    let literal = source
        .lines()
        .map(str::trim)
        .find_map(|line| line.strip_prefix(prefix.as_str()))
        .unwrap_or_else(|| panic!("{name} is not declared in the browser bounds"));
    literal
        .replace('_', "")
        .parse::<i64>()
        .unwrap_or_else(|_| panic!("{name} is not a plain integer literal"))
}

/// Every numeric bound the controls read. Each frontend value must equal the Rust production
/// constant, so a change to a bound in `core` fails here until the browser follows.
#[test]
fn the_browser_bounds_match_the_core_constants() {
    let pins: [(&str, i64); 9] = [
        ("MIN_CELL_SIZE", i64::from(MIN_CELL_SIZE)),
        ("MAX_CELL_SIZE", i64::from(MAX_CELL_SIZE)),
        ("MIN_FONT_SIZE", i64::from(MIN_FONT_SIZE)),
        ("MAX_FONT_SIZE", i64::from(MAX_FONT_SIZE)),
        ("MIN_THRESHOLD", i64::from(MIN_THRESHOLD)),
        ("MAX_THRESHOLD", i64::from(MAX_THRESHOLD)),
        ("MIN_CONTRAST", i64::from(MIN_CONTRAST)),
        ("MAX_CONTRAST", i64::from(MAX_CONTRAST)),
        ("MAX_RAMP_LENGTH", i64::try_from(MAX_RAMP_LENGTH).unwrap()),
    ];
    for (name, expected) in pins {
        assert_eq!(ts_integer(BOUNDS_TS, name), expected, "{name}");
    }
}

fn hex(channel: u8) -> String {
    format!("{channel:02x}")
}

/// The browser defaults must reproduce the native default style, so the default output is the
/// same in every shell.
#[test]
fn the_browser_default_colours_match_the_native_default_style() {
    let style = RenderStyle::default();
    let as_hex = |colour: image_to_binary_processing::Rgb| {
        format!(
            "#{}{}{}",
            hex(colour.red),
            hex(colour.green),
            hex(colour.blue)
        )
    };
    assert_eq!(
        ts_single_quoted(PROTOCOL_TS, "DEFAULT_FOREGROUND"),
        as_hex(style.foreground)
    );
    assert_eq!(
        ts_single_quoted(PROTOCOL_TS, "DEFAULT_BACKGROUND"),
        as_hex(style.background)
    );
    assert_eq!(style.contrast, 0);
    assert!(!style.invert);
}

/// Returns the right-hand side of the one `export const NAME = ...` declaration in `source`.
/// A name declared zero or several times panics, so a pin cannot match a comment or a copy.
fn ts_declaration<'a>(source: &'a str, name: &str) -> &'a str {
    let prefix = format!("export const {name} = ");
    let mut found = source
        .lines()
        .map(str::trim)
        .filter_map(|line| line.strip_prefix(prefix.as_str()));
    let value = found
        .next()
        .unwrap_or_else(|| panic!("{name} is not declared in the browser source"));
    assert!(found.next().is_none(), "{name} is declared more than once");
    value
}

/// Returns the text of a single-quoted declaration, without its quotes.
fn ts_single_quoted<'a>(source: &'a str, name: &str) -> &'a str {
    ts_declaration(source, name)
        .strip_prefix('\'')
        .and_then(|rest| rest.strip_suffix('\''))
        .unwrap_or_else(|| panic!("{name} must be a single-quoted string"))
}

/// Every message the browser and the native shell both show must be the same text, pinned by
/// the exact declaration in the browser source.
#[test]
fn the_style_failure_messages_match_in_both_shells() {
    assert_eq!(
        ts_single_quoted(PROTOCOL_TS, "FOREGROUND_MESSAGE"),
        FOREGROUND_MESSAGE
    );
    assert_eq!(
        ts_single_quoted(PROTOCOL_TS, "BACKGROUND_MESSAGE"),
        BACKGROUND_MESSAGE
    );
    assert_eq!(
        ts_single_quoted(PROTOCOL_TS, "COLOR_MODE_MESSAGE"),
        COLOR_MODE_MESSAGE
    );
    assert_eq!(
        ts_single_quoted(PROTOCOL_TS, "CONTRAST_INTEGER_MESSAGE"),
        CONTRAST_INTEGER_MESSAGE
    );
    assert_eq!(
        ts_single_quoted(PROTOCOL_TS, "INVERT_BOOLEAN_MESSAGE"),
        INVERT_BOOLEAN_MESSAGE
    );
    assert_eq!(
        ts_declaration(PROTOCOL_TS, "CONTRAST_RANGE_MESSAGE"),
        "`contrast must be between ${MIN_CONTRAST} and ${MAX_CONTRAST}`"
    );
    assert_eq!(
        CONTRAST_RANGE_MESSAGE,
        format!("contrast must be between {MIN_CONTRAST} and {MAX_CONTRAST}")
    );
}

/// One row per colour or contrast value, read by both suites, so each rule has one table.
#[test]
fn the_native_style_validation_matches_the_shared_vectors() {
    let mut rows = 0;
    for line in STYLE_VECTORS
        .lines()
        .skip(1)
        .filter(|line| !line.is_empty())
    {
        let mut columns = line.splitn(4, '\t');
        let kind = columns.next().unwrap_or_default();
        let name = columns.next().unwrap_or_default();
        let expected = columns.next().unwrap_or_default();
        let value = columns.next().unwrap_or_default();
        let style = match kind {
            "foreground" => StyleArgs {
                foreground: TextArg::Text(value),
                ..StyleArgs::default()
            },
            "background" => StyleArgs {
                background: TextArg::Text(value),
                ..StyleArgs::default()
            },
            "contrast" => StyleArgs {
                contrast: Some(value.parse::<f64>().unwrap_or(f64::NAN)),
                ..StyleArgs::default()
            },
            other => panic!("unknown vector kind {other}"),
        };
        let outcome = match preview_settings_styled(
            5.0,
            10.0,
            128.0,
            Some("classic"),
            TextArg::Absent,
            TextArg::Absent,
            style,
        ) {
            Ok(_) => "ok".to_owned(),
            Err(error) => error.message(),
        };
        assert_eq!(outcome, expected, "{kind} {name}");
        rows += 1;
    }
    assert_eq!(rows, 35);
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
    assert_eq!(
        ts_single_quoted(PROTOCOL_TS, "RAMP_EMPTY_MESSAGE"),
        RAMP_EMPTY_MESSAGE
    );
    assert_eq!(
        ts_single_quoted(PROTOCOL_TS, "RAMP_NOT_PRINTABLE_MESSAGE"),
        RAMP_NOT_PRINTABLE_MESSAGE
    );
    assert_eq!(
        ts_declaration(PROTOCOL_TS, "RAMP_TOO_LONG_MESSAGE"),
        "`ramp must be at most ${MAX_RAMP_LENGTH} characters`"
    );
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

/// The output-kind message and names are pinned by exact browser declarations, and the
/// native parser accepts exactly the names the browser lists.
#[test]
fn the_output_kind_message_and_names_match_the_browser() {
    assert_eq!(
        ts_single_quoted(PROTOCOL_TS, "OUTPUT_KIND_MESSAGE"),
        OUTPUT_KIND_MESSAGE
    );
    assert!(PROTOCOL_TS.lines().map(str::trim).any(|line| line
        == "export const OUTPUT_KIND_NAMES: readonly OutputKindName[] = ['png', 'text', 'gif']"));
    assert_eq!(output_kind(TextArg::Text("png")).unwrap(), OutputKind::Png);
    assert_eq!(
        output_kind(TextArg::Text("text")).unwrap(),
        OutputKind::Text
    );
    assert_eq!(output_kind(TextArg::Text("gif")).unwrap(), OutputKind::Gif);
    assert_eq!(
        output_kind(TextArg::Text("svg")).unwrap_err().message(),
        OUTPUT_KIND_MESSAGE
    );
}
