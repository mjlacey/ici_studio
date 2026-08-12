//! Verifies `core::mf4::convert_to_table` against a real, unmodified MDF4
//! export -- the compressed (`##HL`/`##DZ`) storage variant that the
//! vendored+patched reader in `core/src/mf4/` exists to handle, not just the
//! synthetic fixtures in `compressed_data_block.rs`'s unit tests.
//!
//! `data/` is gitignored (proprietary experimental data), so this test skips
//! with a clear message when the reference file isn't present, mirroring
//! `parse_real_file.rs`'s pattern for the `.mpt` reference file.

use std::fs;
use std::path::Path;

const REFERENCE_FILE: &str = "lab-discover_RPT_2026-06-23_08.23.32_1.mf4";

#[test]
fn converts_the_real_compressed_mf4_reference_file_end_to_end() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../data")
        .join(REFERENCE_FILE);

    let Ok(bytes) = fs::read(&path) else {
        eprintln!(
            "SKIP: {} not present locally (data/ is gitignored proprietary data). \
             Populate it to exercise this test.",
            path.display()
        );
        return;
    };

    let table = ici_core::mf4::convert_to_table(&bytes)
        .unwrap_or_else(|e| panic!("failed to convert {}: {e}", path.display()));

    assert_eq!(table.report.source, ici_core::parse::SourceFormat::Mdf4);
    assert!(
        table.report.warnings.is_empty(),
        "expected every channel group to merge cleanly, got warnings: {:?}",
        table.report.warnings
    );
    assert_eq!(table.report.n_rows, 340_271);
    assert_eq!(table.report.n_columns, 17);
    assert_eq!(table.report.n_rows, table.columns[0].values.len());

    let names: Vec<&str> = table.columns.iter().map(|c| c.name.as_str()).collect();
    for expected in [
        "Time",
        "U",
        "ClimaTemp",
        "cycle_number",
        "E",
        "Eneg",
        "Epos",
        "I",
        "P",
        "Q",
        "Qneg",
        "Qpos",
        "step_number",
        "Temp[1]",
        "Temp[2]",
        "Temp[3]",
        "Timer1",
    ] {
        assert!(names.contains(&expected), "expected column '{expected}' in {names:?}");
    }

    for column in &table.columns {
        assert!(column.is_numeric, "column '{}' should be numeric", column.name);
        assert!(
            column.values.iter().all(|v| !v.is_nan()),
            "column '{}' has a NaN value",
            column.name
        );
    }

    let time = &table.columns.iter().find(|c| c.name == "Time").unwrap().values;
    assert!(
        time.windows(2).all(|w| w[1] >= w[0]),
        "time must be monotonically non-decreasing"
    );

    let voltage = &table.columns.iter().find(|c| c.name == "U").unwrap().values;
    assert!(
        voltage.iter().all(|&v| (2.0..=4.5).contains(&v)),
        "voltage should be in a plausible Li-ion cell range"
    );

    let current = &table.columns.iter().find(|c| c.name == "I").unwrap().values;
    assert!(
        current.iter().all(|&v| v.abs() <= 200.0),
        "current should be within a plausible range for this test"
    );
}
