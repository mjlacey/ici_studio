//! Verifies the parser against a real, unmodified instrument export file --
//! spec build order §15 item 2: "the 112-line-preamble case as the hardest
//! instance rather than the design target", after the synthetic hazard
//! fixtures in `core/src/parse.rs` already pass.
//!
//! `data/` is gitignored (proprietary experimental data, §13.5), so this
//! test skips with a clear message when the reference file isn't present --
//! the same pattern the spec asks for around the (larger, untrimmed)
//! golden-file fixtures.

use std::fs;
use std::path::Path;

const REFERENCE_FILE: &str = "Gen 1 C2 BOL Cell 423 N23BM3_03_MB_C13.mpt";

#[test]
fn parses_the_real_mpt_reference_file_end_to_end() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../data")
        .join(REFERENCE_FILE);

    let Ok(bytes) = fs::read(&path) else {
        eprintln!(
            "SKIP: {} not present locally (data/ is gitignored proprietary data; \
             see ICI_WEB_SPEC.md §13.5). Populate it to exercise this test.",
            path.display()
        );
        return;
    };

    let table = ici_core::parse::parse_default(&bytes)
        .unwrap_or_else(|e| panic!("failed to parse {}: {e}", path.display()));

    // Verified properties from ICI_WEB_SPEC.md §3.
    assert_eq!(table.report.encoding, ici_core::parse::Encoding::Cp1252);
    assert_eq!(table.report.delimiter, ici_core::parse::Delimiter::Tab);
    assert_eq!(
        table.report.decimal_separator,
        ici_core::parse::DecimalSeparator::Dot
    );
    assert_eq!(table.report.preamble_lines_skipped, 112);
    assert!(table.report.header_present);
    assert!(!table.report.header_synthesized);
    assert!(
        table.report.trailing_column_dropped,
        "the spurious 39th column from the trailing tab should be dropped"
    );
    assert_eq!(table.report.n_columns, 38);
    assert_eq!(table.report.ragged_rows_dropped, 0);

    let names: Vec<&str> = table.columns.iter().map(|c| c.name.as_str()).collect();
    for expected in [
        "time/s",
        "Ewe/V",
        "I/mA",
        "(Q-Qo)/mA.h",
        "Q charge/discharge/mA.h",
        "cycle number",
        "half cycle",
        "Ns",
        "step time/s",
        "mode",
        "ox/red",
        "x",
        "Capacity/mA.h",
    ] {
        assert!(
            names.contains(&expected),
            "expected column '{expected}' in {names:?}"
        );
    }

    let current_col = table.columns.iter().find(|c| c.name == "I/mA").unwrap();
    assert!(current_col.is_numeric);
    let n_rest = current_col.values.iter().filter(|&&v| v == 0.0).count();
    assert_eq!(
        n_rest, 25_602,
        "current is exactly 0 during rest for 25,602 of the samples"
    );

    let time_col = table.columns.iter().find(|c| c.name == "time/s").unwrap();
    assert!(time_col.is_numeric);
    assert!(
        time_col.values.windows(2).all(|w| w[1] >= w[0]),
        "time must be monotonically non-decreasing"
    );

    assert_eq!(table.report.n_rows, current_col.values.len());
}
