//! Generic, instrument-agnostic delimited-text parsing.
//!
//! Implements ICI_WEB_SPEC.md §4.2: an optional preamble of arbitrary length,
//! followed by an optional single header line, followed by numeric rows
//! delimited by tab/comma/semicolon/pipe. There are no vendor-specific code
//! paths here and no per-format branches — the same algorithm must reach the
//! right answer on a bare two-column CSV and on a 112-line EC-Lab preamble.

use std::fmt;

/// Reject files above this size outright (§4.4).
pub const MAX_FILE_BYTES: usize = 250 * 1024 * 1024;
/// Warn (but proceed) above this size (§4.4).
pub const WARN_FILE_BYTES: usize = 100 * 1024 * 1024;

const SNIFF_LINE_CAP: usize = 5_000;
const MIN_RUN_LENGTH: usize = 5;
const MIN_FIELD_COUNT: usize = 2;
/// How many anomalous trailing lines (e.g. one truncated final export line)
/// the terminal-run search tolerates while still locating the data block.
const TAIL_LOOKBACK: usize = 3;
const MISSING_TOKENS: &[&str] = &["na", "nan", "inf", "-inf", "null", "-"];
const MAX_REPORTED_ROWS: usize = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Encoding {
    Utf8,
    Cp1252,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Delimiter {
    Tab,
    Comma,
    Semicolon,
    Pipe,
}

impl Delimiter {
    pub fn as_char(self) -> char {
        match self {
            Delimiter::Tab => '\t',
            Delimiter::Comma => ',',
            Delimiter::Semicolon => ';',
            Delimiter::Pipe => '|',
        }
    }

    fn candidates() -> [Delimiter; 4] {
        [
            Delimiter::Tab,
            Delimiter::Comma,
            Delimiter::Semicolon,
            Delimiter::Pipe,
        ]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DecimalSeparator {
    Dot,
    Comma,
}

impl DecimalSeparator {
    fn candidates() -> [DecimalSeparator; 2] {
        [DecimalSeparator::Dot, DecimalSeparator::Comma]
    }
}

/// One parsed column. Exactly one of `values`/`strings` is populated,
/// selected by `is_numeric`: numeric columns carry `values` (NaN for
/// missing/unparseable cells) and leave `strings` empty; predominantly
/// non-numeric columns carry the trimmed raw cell text in `strings` (usable
/// only as a grouping column downstream) and leave `values` empty.
#[derive(Debug, Clone)]
pub struct Column {
    pub name: String,
    pub is_numeric: bool,
    pub values: Vec<f64>,
    pub strings: Vec<String>,
}

/// Per-column count of cells that were neither a parseable number nor a
/// recognised missing token (empty/NA/NaN/Inf/-Inf/null/-).
#[derive(Debug, Clone)]
pub struct CoercionFailure {
    pub column: String,
    pub count: usize,
    /// 0-based indices into the final row set, first `MAX_REPORTED_ROWS` only.
    pub first_rows: Vec<usize>,
}

/// Which parser produced a [`ParsedTable`] -- `Delimiter`/`DecimalSeparator`/
/// `Encoding` above describe text-file sniffing decisions that simply don't
/// apply to a binary MDF4 source (`core::mf4`); callers use this to decide
/// whether to display those fields at all rather than show meaningless
/// defaults for an `Mdf4`-sourced report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SourceFormat {
    Text,
    Mdf4,
}

#[derive(Debug, Clone)]
pub struct ParseReport {
    pub encoding: Encoding,
    pub delimiter: Delimiter,
    pub decimal_separator: DecimalSeparator,
    pub preamble_lines_skipped: usize,
    pub header_present: bool,
    pub header_synthesized: bool,
    pub n_rows: usize,
    pub n_columns: usize,
    pub trailing_column_dropped: bool,
    pub ragged_rows_dropped: usize,
    /// 1-based file line numbers, first `MAX_REPORTED_ROWS` only.
    pub ragged_row_line_numbers: Vec<usize>,
    pub coercion_failures: Vec<CoercionFailure>,
    pub warnings: Vec<String>,
    pub source: SourceFormat,
}

#[derive(Debug, Clone)]
pub struct ParsedTable {
    pub columns: Vec<Column>,
    pub report: ParseReport,
}

/// Manual overrides for every sniffer decision (§4.2 "Manual override").
/// Each `Some` value replaces the corresponding auto-detected decision;
/// leaving a field `None` keeps the sniffer's own choice for that dimension.
#[derive(Debug, Clone, Default)]
pub struct ParseOverrides {
    pub encoding: Option<Encoding>,
    pub delimiter: Option<Delimiter>,
    pub decimal_separator: Option<DecimalSeparator>,
    pub skip_lines: Option<usize>,
    pub header_present: Option<bool>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ParseError {
    EmptyFile,
    FileTooLarge { bytes: usize, limit: usize },
    NoDataBlockFound { delimiters_tried: Vec<Delimiter> },
    NoDataRows,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseError::EmptyFile => write!(f, "File is empty."),
            ParseError::FileTooLarge { bytes, limit } => write!(
                f,
                "File is {bytes} bytes, exceeding the {limit} byte limit."
            ),
            ParseError::NoDataBlockFound { delimiters_tried } => write!(
                f,
                "Could not locate a delimited data block. Tried delimiters: {delimiters_tried:?}."
            ),
            ParseError::NoDataRows => write!(
                f,
                "No data rows remain after the header/preamble boundary currently in effect."
            ),
        }
    }
}

impl std::error::Error for ParseError {}

/// Parse `bytes` with all sniffer decisions automatic.
pub fn parse_default(bytes: &[u8]) -> Result<ParsedTable, ParseError> {
    parse(bytes, &ParseOverrides::default())
}

/// Parse `bytes`, honouring any manual overrides. See `ParseOverrides`.
pub fn parse(bytes: &[u8], overrides: &ParseOverrides) -> Result<ParsedTable, ParseError> {
    if bytes.is_empty() {
        return Err(ParseError::EmptyFile);
    }
    if bytes.len() > MAX_FILE_BYTES {
        return Err(ParseError::FileTooLarge {
            bytes: bytes.len(),
            limit: MAX_FILE_BYTES,
        });
    }

    let mut warnings = Vec::new();
    if bytes.len() > WARN_FILE_BYTES {
        warnings.push(format!(
            "File is {:.1} MB; parsing may take a moment.",
            bytes.len() as f64 / (1024.0 * 1024.0)
        ));
    }

    let (decoded, encoding) = decode(bytes, overrides.encoding);
    let normalized = normalize_line_endings(&decoded);
    let mut lines: Vec<&str> = normalized.split('\n').collect();
    while lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }
    if lines.is_empty() {
        return Err(ParseError::EmptyFile);
    }

    let delim_candidates: Vec<Delimiter> = match overrides.delimiter {
        Some(d) => vec![d],
        None => Delimiter::candidates().to_vec(),
    };
    let decimal_candidates: Vec<DecimalSeparator> = match overrides.decimal_separator {
        Some(d) => vec![d],
        None => DecimalSeparator::candidates().to_vec(),
    };

    // When the user has told us exactly where the data starts, there is
    // nothing left to *locate* -- the strict terminal-run search (built to
    // find an unknown boundary) is the wrong tool and would wrongly demand
    // every field in the known data region be numeric (defeating e.g. a
    // legitimate text grouping column). Use a lenient delimiter/decimal
    // scorer over the known region instead.
    let (delimiter, decimal_separator, preamble_lines, header_present) = if let Some(skip) =
        overrides.skip_lines
    {
        let (delimiter, decimal_separator) =
            match (overrides.delimiter, overrides.decimal_separator) {
                (Some(d), Some(dec)) => (d, dec),
                _ => pick_delimiter_lenient(&lines, skip, &delim_candidates, &decimal_candidates)?,
            };
        let header_present = overrides
            .header_present
            .unwrap_or_else(|| header_present_at(&lines, skip, delimiter, decimal_separator));
        (delimiter, decimal_separator, skip, header_present)
    } else {
        let sniff = sniff_constrained(&lines, &delim_candidates, &decimal_candidates)?;
        let delimiter = overrides.delimiter.unwrap_or(sniff.delimiter);
        let decimal_separator = overrides
            .decimal_separator
            .unwrap_or(sniff.decimal_separator);
        let header_present = overrides.header_present.unwrap_or(sniff.header_present);
        (
            delimiter,
            decimal_separator,
            sniff.preamble_lines,
            header_present,
        )
    };
    let data_start_line = preamble_lines + usize::from(header_present);

    if data_start_line >= lines.len() {
        return Err(ParseError::NoDataRows);
    }

    let mut header_synthesized = false;
    let mut header_fields: Vec<String> = if header_present {
        lines[data_start_line - 1]
            .split(delimiter.as_char())
            .map(|s| s.trim().to_string())
            .collect()
    } else {
        header_synthesized = true;
        let f = lines[data_start_line].split(delimiter.as_char()).count();
        (1..=f).map(|i| format!("col{i}")).collect()
    };
    if header_synthesized {
        warnings.push(
            "No header row detected; columns were named col1, col2, ... by position.".to_string(),
        );
    }

    let body_line_indices: Vec<usize> = (data_start_line..lines.len()).collect();
    let mut body_fields: Vec<Vec<&str>> = body_line_indices
        .iter()
        .map(|&i| lines[i].split(delimiter.as_char()).collect())
        .collect();

    // Step 4: trailing-empty-column fix. In practice the trailing delimiter
    // can show up on the header line, on every data row, or both -- e.g. the
    // committed reference file has it only on the header line, not on any
    // data row -- so each side is checked and fixed independently.
    let mut trailing_column_dropped = false;
    let body_len = body_fields.first().map_or(0, |r| r.len());

    let header_has_trailing_empty =
        !header_synthesized && header_fields.last().is_some_and(|s| s.is_empty());
    if header_has_trailing_empty && header_fields.len() == body_len + 1 {
        header_fields.pop();
        trailing_column_dropped = true;
        warnings
            .push("Dropped a trailing empty column present on the header row only.".to_string());
    }

    let all_body_have_trailing_empty = !body_fields.is_empty()
        && body_fields
            .iter()
            .all(|row| row.last().is_some_and(|s| s.is_empty()));
    if all_body_have_trailing_empty {
        let header_ok = header_synthesized
            || header_fields.len() == body_len
            || header_fields.len() + 1 == body_len;
        if header_ok {
            for row in &mut body_fields {
                row.pop();
            }
            if header_synthesized || header_fields.len() == body_len {
                header_fields.pop();
            }
            trailing_column_dropped = true;
            warnings.push(
                "Dropped a trailing empty column produced by a trailing delimiter.".to_string(),
            );
        }
    }

    // Step 6: header cleanup (trim done above; empty-cell + de-dup here).
    for (i, name) in header_fields.iter_mut().enumerate() {
        if name.trim().is_empty() {
            *name = format!("col{}", i + 1);
        }
    }
    dedupe_header(&mut header_fields);

    let expected_f = header_fields.len();

    // Step 5: ragged rows.
    let mut ragged_row_line_numbers = Vec::new();
    let mut kept_body: Vec<Vec<&str>> = Vec::with_capacity(body_fields.len());
    for (k, row) in body_fields.into_iter().enumerate() {
        if row.len() == expected_f {
            kept_body.push(row);
        } else {
            let line_number = body_line_indices[k] + 1; // 1-based
            if ragged_row_line_numbers.len() < MAX_REPORTED_ROWS {
                ragged_row_line_numbers.push(line_number);
            }
        }
    }
    let ragged_rows_dropped = body_line_indices.len() - kept_body.len();
    if ragged_rows_dropped > 0 {
        warnings.push(format!(
            "Dropped {ragged_rows_dropped} row(s) with an unexpected field count (first line(s): {ragged_row_line_numbers:?})."
        ));
    }

    // Step 7: parse body into a columnar store.
    let n_rows = kept_body.len();
    let mut number_counts = vec![0usize; expected_f];
    let mut invalid_counts = vec![0usize; expected_f];
    let mut coercion_rows: Vec<Vec<usize>> = vec![Vec::new(); expected_f];
    let mut cells_by_column: Vec<Vec<Cell>> = (0..expected_f)
        .map(|_| Vec::with_capacity(n_rows))
        .collect();

    for (row_idx, row) in kept_body.iter().enumerate() {
        for (col_idx, field) in row.iter().enumerate() {
            let cell = parse_cell(field, decimal_separator);
            match cell {
                Cell::Number(_) => number_counts[col_idx] += 1,
                Cell::Invalid => {
                    invalid_counts[col_idx] += 1;
                    if coercion_rows[col_idx].len() < MAX_REPORTED_ROWS {
                        coercion_rows[col_idx].push(row_idx);
                    }
                }
                Cell::Missing => {}
            }
            cells_by_column[col_idx].push(cell);
        }
    }

    let mut columns = Vec::with_capacity(expected_f);
    let mut coercion_failures = Vec::new();
    for col_idx in 0..expected_f {
        let denom = number_counts[col_idx] + invalid_counts[col_idx];
        let is_numeric = denom == 0 || number_counts[col_idx] * 2 >= denom;

        let (values, strings) = if is_numeric {
            let values = cells_by_column[col_idx]
                .iter()
                .map(|c| match c {
                    Cell::Number(v) => *v,
                    _ => f64::NAN,
                })
                .collect();
            (values, Vec::new())
        } else {
            let strings = kept_body
                .iter()
                .map(|row| row[col_idx].trim().to_string())
                .collect();
            (Vec::new(), strings)
        };

        columns.push(Column {
            name: header_fields[col_idx].clone(),
            is_numeric,
            values,
            strings,
        });

        if is_numeric && invalid_counts[col_idx] > 0 {
            coercion_failures.push(CoercionFailure {
                column: header_fields[col_idx].clone(),
                count: invalid_counts[col_idx],
                first_rows: coercion_rows[col_idx].clone(),
            });
        }
    }
    for failure in &coercion_failures {
        warnings.push(format!(
            "Column '{}': {} value(s) could not be parsed as numbers (first row(s): {:?}).",
            failure.column, failure.count, failure.first_rows
        ));
    }

    let report = ParseReport {
        encoding,
        delimiter,
        decimal_separator,
        preamble_lines_skipped: preamble_lines,
        header_present,
        header_synthesized,
        n_rows,
        n_columns: expected_f,
        trailing_column_dropped,
        ragged_rows_dropped,
        ragged_row_line_numbers,
        coercion_failures,
        warnings,
        source: SourceFormat::Text,
    };

    Ok(ParsedTable { columns, report })
}

fn dedupe_header(names: &mut [String]) {
    let mut seen: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for name in names.iter_mut() {
        let count = seen.entry(name.clone()).or_insert(0);
        *count += 1;
        if *count > 1 {
            *name = format!("{name} ({count})");
        }
    }
}

// ---------------------------------------------------------------------
// Decoding
// ---------------------------------------------------------------------

fn decode(bytes: &[u8], override_encoding: Option<Encoding>) -> (String, Encoding) {
    let bytes = strip_bom(bytes);
    match override_encoding {
        Some(Encoding::Utf8) => (String::from_utf8_lossy(bytes).into_owned(), Encoding::Utf8),
        Some(Encoding::Cp1252) => (decode_cp1252(bytes), Encoding::Cp1252),
        None => match std::str::from_utf8(bytes) {
            Ok(s) => (s.to_string(), Encoding::Utf8),
            Err(_) => (decode_cp1252(bytes), Encoding::Cp1252),
        },
    }
}

fn strip_bom(bytes: &[u8]) -> &[u8] {
    if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        rest
    } else {
        bytes
    }
}

fn decode_cp1252(bytes: &[u8]) -> String {
    let (text, _, _) = encoding_rs::WINDOWS_1252.decode(bytes);
    text.into_owned()
}

fn normalize_line_endings(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\r' {
            if chars.peek() == Some(&'\n') {
                chars.next();
            }
            out.push('\n');
        } else {
            out.push(c);
        }
    }
    out
}

// ---------------------------------------------------------------------
// Cell parsing
// ---------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
enum Cell {
    Number(f64),
    Missing,
    Invalid,
}

fn parse_cell(field: &str, decimal: DecimalSeparator) -> Cell {
    let t = field.trim();
    if t.is_empty() {
        return Cell::Missing;
    }
    let lower = t.to_ascii_lowercase();
    if MISSING_TOKENS.contains(&lower.as_str()) {
        return Cell::Missing;
    }
    let normalized: std::borrow::Cow<str> = match decimal {
        DecimalSeparator::Dot => std::borrow::Cow::Borrowed(t),
        DecimalSeparator::Comma => std::borrow::Cow::Owned(t.replacen(',', ".", 1)),
    };
    if let Ok(v) = normalized.parse::<f64>() {
        if v.is_finite() {
            return Cell::Number(v);
        }
    }
    match parse_numeric_with_unit_suffix(&normalized) {
        Some(v) => Cell::Number(v),
        None => Cell::Invalid,
    }
}

/// Some instrument exports stringify an otherwise-numeric column with a
/// trailing unit label baked into every cell -- e.g. a direct .mf4->.csv
/// conversion producing a time column of "0 sec", "1 sec", ... instead of
/// a bare number (observed directly in a real export; R would read the
/// same column in as character for the same reason). Not vendor-specific:
/// this is a fallback tried only after a plain numeric parse has already
/// failed, so it can't reclassify a column that already parses cleanly.
///
/// Only strips a *short trailing run of unit-like characters* (ASCII
/// letters, or one of a handful of common unit symbols) -- so a genuine
/// text column (e.g. "charge"/"rest"/"discharge", or a cell ID like
/// "N23BM3") is untouched: those have no leading digit for this to anchor
/// on, or don't end in a short alphabetic run at all.
fn parse_numeric_with_unit_suffix(t: &str) -> Option<f64> {
    const MAX_SUFFIX_LEN: usize = 8;
    let trimmed = t.trim_end();
    let chars: Vec<char> = trimmed.chars().collect();
    let n = chars.len();
    let mut split = n;
    while split > 0 {
        let c = chars[split - 1];
        if c.is_ascii_alphabetic() || matches!(c, '%' | '°' | 'µ' | 'Ω') {
            split -= 1;
        } else {
            break;
        }
    }
    // No suffix at all, or the entire cell is the "suffix" (no numeric
    // prefix to anchor on -- e.g. plain text like "charge").
    if split == n || split == 0 || n - split > MAX_SUFFIX_LEN {
        return None;
    }
    let numeric_part: String = chars[..split].iter().collect();
    match numeric_part.trim_end().parse::<f64>() {
        Ok(v) if v.is_finite() => Some(v),
        _ => None,
    }
}

fn is_number_or_missing(field: &str, decimal: DecimalSeparator) -> bool {
    !matches!(parse_cell(field, decimal), Cell::Invalid)
}

// ---------------------------------------------------------------------
// Joint delimiter / decimal-separator / data-block search (§4.2 step 2)
// plus header-row detection (§4.2 step 3).
// ---------------------------------------------------------------------

struct Sniff {
    delimiter: Delimiter,
    decimal_separator: DecimalSeparator,
    preamble_lines: usize,
    header_present: bool,
}

fn sniff_constrained(
    lines: &[&str],
    delim_candidates: &[Delimiter],
    decimal_candidates: &[DecimalSeparator],
) -> Result<Sniff, ParseError> {
    let sample_len = lines.len().min(SNIFF_LINE_CAP);
    let sample = &lines[..sample_len];

    let mut best: Option<(usize, Delimiter, DecimalSeparator, usize, usize)> = None;
    // (score, delimiter, decimal_separator, data_start_line, field_count)

    // A single-candidate list means the caller (or a user override) has
    // pinned this dimension. The F>=2 floor exists to stop *automatic*
    // detection from trivially "winning" with a delimiter that doesn't
    // really appear in the text; that protection doesn't apply once the
    // user has explicitly forced the delimiter.
    let min_field_count = if delim_candidates.len() == 1 {
        1
    } else {
        MIN_FIELD_COUNT
    };

    for &delim in delim_candidates {
        if sample_len == 0 {
            continue;
        }
        let split_lines: Vec<Vec<&str>> = sample
            .iter()
            .map(|l| l.split(delim.as_char()).collect())
            .collect();

        for &decimal in decimal_candidates {
            if delim == Delimiter::Comma && decimal == DecimalSeparator::Comma {
                continue;
            }
            let last_idx = sample_len - 1;
            let is_numeric_row = |idx: usize, f: usize| {
                split_lines[idx].len() == f
                    && split_lines[idx]
                        .iter()
                        .all(|field| is_number_or_missing(field, decimal))
            };

            // F is normally the last line's field count. But the motivating
            // case for ragged-row handling (§4.2 step 5) is a truncated
            // final line from an interrupted export -- exactly a line whose
            // field count *won't* match. Tolerate up to TAIL_LOOKBACK
            // anomalous trailing lines by also trying the field counts of
            // those lines as candidate F values, each anchored at the last
            // line (within the lookback window) that actually matches it.
            let lookback = TAIL_LOOKBACK.min(sample_len);
            let mut f_candidates: Vec<usize> = Vec::new();
            for k in 0..lookback {
                let f = split_lines[last_idx - k].len();
                if f >= min_field_count && !f_candidates.contains(&f) {
                    f_candidates.push(f);
                }
            }

            for f in f_candidates {
                let mut end_idx = None;
                for k in 0..lookback {
                    let idx = last_idx - k;
                    if is_numeric_row(idx, f) {
                        end_idx = Some(idx);
                        break;
                    }
                }
                let Some(end_idx) = end_idx else { continue };

                let mut run_length = 1usize;
                let mut i = end_idx;
                while i > 0 && is_numeric_row(i - 1, f) {
                    run_length += 1;
                    i -= 1;
                }
                if run_length < MIN_RUN_LENGTH {
                    continue;
                }
                let score = run_length * f;
                let data_start_line = end_idx + 1 - run_length;
                let is_better = match &best {
                    None => true,
                    Some((best_score, ..)) => score > *best_score,
                };
                if is_better {
                    best = Some((score, delim, decimal, data_start_line, f));
                }
            }
        }
    }

    let (_, delimiter, decimal_separator, data_start_line, field_count) =
        best.ok_or_else(|| ParseError::NoDataBlockFound {
            delimiters_tried: delim_candidates.to_vec(),
        })?;

    let (header_present, preamble_lines) = detect_header(
        lines,
        data_start_line,
        delimiter,
        decimal_separator,
        field_count,
    );

    Ok(Sniff {
        delimiter,
        decimal_separator,
        preamble_lines,
        header_present,
    })
}

fn detect_header(
    lines: &[&str],
    data_start_line: usize,
    delimiter: Delimiter,
    decimal: DecimalSeparator,
    field_count: usize,
) -> (bool, usize) {
    if data_start_line == 0 {
        return (false, 0);
    }
    let candidate_idx = data_start_line - 1;
    let candidate_fields: Vec<&str> = lines[candidate_idx].split(delimiter.as_char()).collect();
    let all_numeric = candidate_fields
        .iter()
        .all(|f| is_number_or_missing(f, decimal));
    if field_count_matches_allowing_trailing_empty(&candidate_fields, field_count) && !all_numeric {
        (true, candidate_idx)
    } else {
        (false, data_start_line)
    }
}

/// True if `fields` has exactly `target` fields, or has `target + 1` fields
/// whose last one is empty (a header-only trailing delimiter, which the
/// later trailing-empty-column fix strips).
fn field_count_matches_allowing_trailing_empty(fields: &[&str], target: usize) -> bool {
    fields.len() == target
        || (fields.len() == target + 1 && fields.last().is_some_and(|s| s.is_empty()))
}

const LENIENT_SAMPLE_ROWS: usize = 200;

/// Best-effort delimiter/decimal-separator pick used when the caller has
/// already pinned `skip_lines` (so there is nothing left to *locate*, and
/// requiring every field to be numeric -- as the strict terminal-run search
/// does -- would wrongly reject a legitimate text grouping column). Scores
/// each combination by field-count consistency and numeric-field fraction
/// over a sample of the known data region, rather than requiring perfection.
fn pick_delimiter_lenient(
    lines: &[&str],
    skip: usize,
    delim_candidates: &[Delimiter],
    decimal_candidates: &[DecimalSeparator],
) -> Result<(Delimiter, DecimalSeparator), ParseError> {
    // Skip the line at `skip` itself: it may or may not be a header, and
    // including it would bias scoring against delimiters that correctly
    // split a text header into multiple non-numeric fields.
    let scan_start = (skip + 1).min(lines.len());
    let mut sample: &[&str] = if scan_start < lines.len() {
        &lines[scan_start..]
    } else {
        &lines[skip.min(lines.len())..]
    };
    if sample.len() > LENIENT_SAMPLE_ROWS {
        sample = &sample[..LENIENT_SAMPLE_ROWS];
    }

    let mut best: Option<(f64, Delimiter, DecimalSeparator)> = None;
    for &delim in delim_candidates {
        let split: Vec<Vec<&str>> = sample
            .iter()
            .map(|l| l.split(delim.as_char()).collect())
            .collect();
        let f = split.first().map(|r| r.len()).unwrap_or(0);
        if f < 1 {
            continue;
        }
        for &decimal in decimal_candidates {
            if delim == Delimiter::Comma && decimal == DecimalSeparator::Comma {
                continue;
            }
            let mut numeric_ok = 0usize;
            let mut total = 0usize;
            for row in &split {
                if row.len() != f {
                    continue;
                }
                for field in row {
                    total += 1;
                    if is_number_or_missing(field, decimal) {
                        numeric_ok += 1;
                    }
                }
            }
            if total == 0 {
                continue;
            }
            let score = (numeric_ok as f64 / total as f64) * (f as f64);
            let is_better = match &best {
                None => true,
                Some((best_score, ..)) => score > *best_score,
            };
            if is_better {
                best = Some((score, delim, decimal));
            }
        }
    }

    best.map(|(_, d, dec)| (d, dec))
        .ok_or_else(|| ParseError::NoDataBlockFound {
            delimiters_tried: delim_candidates.to_vec(),
        })
}

/// Header-presence check used alongside `pick_delimiter_lenient`: since
/// there is no terminal run to derive a reference field count from, use the
/// next line's field count as the reference instead.
fn header_present_at(
    lines: &[&str],
    skip: usize,
    delimiter: Delimiter,
    decimal: DecimalSeparator,
) -> bool {
    if skip >= lines.len() {
        return false;
    }
    let candidate: Vec<&str> = lines[skip].split(delimiter.as_char()).collect();
    let reference_f = lines
        .get(skip + 1)
        .map(|l| l.split(delimiter.as_char()).count())
        .unwrap_or(candidate.len());
    let all_numeric = candidate.iter().all(|f| is_number_or_missing(f, decimal));
    field_count_matches_allowing_trailing_empty(&candidate, reference_f) && !all_numeric
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_csv_with_header() {
        let text = "a,b,c\n1,2,3\n4,5,6\n7,8,9\n10,11,12\n13,14,15\n";
        let table = parse_default(text.as_bytes()).unwrap();
        assert_eq!(table.report.delimiter, Delimiter::Comma);
        assert_eq!(table.report.decimal_separator, DecimalSeparator::Dot);
        assert!(table.report.header_present);
        assert!(!table.report.header_synthesized);
        assert_eq!(table.report.n_rows, 5);
        assert_eq!(table.columns[0].name, "a");
        assert_eq!(table.columns[0].values, vec![1.0, 4.0, 7.0, 10.0, 13.0]);
    }

    #[test]
    fn headerless_tsv_synthesizes_columns() {
        let text = "1\t2\n3\t4\n5\t6\n7\t8\n9\t10\n";
        let table = parse_default(text.as_bytes()).unwrap();
        assert!(!table.report.header_present);
        assert!(table.report.header_synthesized);
        assert_eq!(table.columns[0].name, "col1");
        assert_eq!(table.columns[1].name, "col2");
        assert_eq!(table.report.preamble_lines_skipped, 0);
    }

    #[test]
    fn two_line_preamble() {
        let text =
            "Instrument export\nOperator: someone\ntime\tvalue\n0\t1\n1\t2\n2\t3\n3\t4\n4\t5\n";
        let table = parse_default(text.as_bytes()).unwrap();
        assert_eq!(table.report.preamble_lines_skipped, 2);
        assert!(table.report.header_present);
        assert_eq!(table.columns[0].name, "time");
        assert_eq!(table.columns[1].name, "value");
    }

    #[test]
    fn decimal_comma_is_semicolon_delimited() {
        let text = "x;y\n0,0;1,5\n1,0;2,5\n2,0;3,5\n3,0;4,5\n4,0;5,5\n";
        let table = parse_default(text.as_bytes()).unwrap();
        assert_eq!(table.report.delimiter, Delimiter::Semicolon);
        assert_eq!(table.report.decimal_separator, DecimalSeparator::Comma);
        assert_eq!(table.columns[0].values, vec![0.0, 1.0, 2.0, 3.0, 4.0]);
        assert_eq!(table.columns[1].values, vec![1.5, 2.5, 3.5, 4.5, 5.5]);
    }

    #[test]
    fn cp1252_bytes_decode_without_mojibake() {
        // "cm\xB2" in cp1252 is "cm²".
        let mut bytes = b"area/cm\xB2\tvalue\n".to_vec();
        bytes.extend_from_slice(b"1.0\t2\n2.0\t4\n3.0\t6\n4.0\t8\n5.0\t10\n");
        assert!(
            std::str::from_utf8(&bytes).is_err(),
            "fixture must be invalid UTF-8"
        );
        let table = parse_default(&bytes).unwrap();
        assert_eq!(table.report.encoding, Encoding::Cp1252);
        assert_eq!(table.columns[0].name, "area/cm\u{b2}");
    }

    #[test]
    fn utf8_bom_is_stripped() {
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice(b"a,b\n1,2\n3,4\n5,6\n7,8\n9,10\n");
        let table = parse_default(&bytes).unwrap();
        assert_eq!(table.report.encoding, Encoding::Utf8);
        assert_eq!(table.columns[0].name, "a");
    }

    #[test]
    fn ragged_final_row_is_dropped_with_warning() {
        let text = "a,b\n1,2\n3,4\n5,6\n7,8\n9,10\n11\n";
        let table = parse_default(text.as_bytes()).unwrap();
        assert_eq!(table.report.n_rows, 5);
        assert_eq!(table.report.ragged_rows_dropped, 1);
        assert_eq!(table.report.ragged_row_line_numbers, vec![7]);
    }

    #[test]
    fn trailing_delimiter_produces_phantom_column_that_gets_dropped() {
        let text = "a\tb\t\n1\t2\t\n3\t4\t\n5\t6\t\n7\t8\t\n9\t10\t\n";
        let table = parse_default(text.as_bytes()).unwrap();
        assert!(table.report.trailing_column_dropped);
        assert_eq!(table.report.n_columns, 2);
        assert_eq!(table.columns.len(), 2);
        assert_eq!(table.columns[1].name, "b");
    }

    #[test]
    fn trailing_delimiter_header_one_fewer_field() {
        // Header has no trailing tab; data rows do.
        let text = "a\tb\n1\t2\t\n3\t4\t\n5\t6\t\n7\t8\t\n9\t10\t\n";
        let table = parse_default(text.as_bytes()).unwrap();
        assert!(table.report.trailing_column_dropped);
        assert_eq!(table.report.n_columns, 2);
    }

    #[test]
    fn crlf_line_endings_are_normalized() {
        let text = "a,b\r\n1,2\r\n3,4\r\n5,6\r\n7,8\r\n9,10\r\n";
        let table = parse_default(text.as_bytes()).unwrap();
        assert_eq!(table.report.n_rows, 5);
        assert_eq!(table.columns[0].values, vec![1.0, 3.0, 5.0, 7.0, 9.0]);
    }

    #[test]
    fn lf_only_line_endings_work() {
        let text = "a,b\n1,2\n3,4\n5,6\n7,8\n9,10\n";
        let table = parse_default(text.as_bytes()).unwrap();
        assert_eq!(table.report.n_rows, 5);
    }

    #[test]
    fn preamble_with_mismatched_tab_table_defeats_naive_sniffer() {
        // The preamble contains its own tab-delimited table with a DIFFERENT
        // field count (3) from the real data block (2) -- the exact hazard
        // that defeats a "first line with N tabs wins" heuristic.
        let text = "\
Export settings\n\
mode\trange\tstep\n\
A\t1\t2\n\
B\t3\t4\n\
\n\
time/s\tEwe/V\n\
0.0\t1.0\n\
0.2\t1.1\n\
0.4\t1.2\n\
0.6\t1.3\n\
0.8\t1.4\n";
        let table = parse_default(text.as_bytes()).unwrap();
        assert_eq!(table.report.n_columns, 2);
        assert_eq!(table.columns[0].name, "time/s");
        assert_eq!(table.columns[1].name, "Ewe/V");
        assert_eq!(table.report.preamble_lines_skipped, 5);
        assert_eq!(table.report.n_rows, 5);
    }

    #[test]
    fn scientific_notation_parses() {
        let text = "a,b\n2.901364069601998E+005,1\n1.0E-003,2\n3.0,3\n4.0,4\n5.0,5\n";
        let table = parse_default(text.as_bytes()).unwrap();
        assert!((table.columns[0].values[0] - 2.901364069601998E+005).abs() < 1e-6);
        assert!((table.columns[0].values[1] - 1.0E-003).abs() < 1e-12);
    }

    #[test]
    fn numeric_column_with_trailing_unit_suffix_parses_as_numeric() {
        // Observed directly in a real instrument's direct .mf4->.csv export:
        // a time column stringified as "0 sec", "1 sec", ... instead of a
        // bare number.
        let text = "t,x\n0 sec,1\n1 sec,2\n2 sec,3\n3 sec,4\n4 sec,5\n";
        let table = parse_default(text.as_bytes()).unwrap();
        assert!(table.columns[0].is_numeric);
        assert_eq!(table.columns[0].values, vec![0.0, 1.0, 2.0, 3.0, 4.0]);
        assert!(table.report.coercion_failures.is_empty());
    }

    #[test]
    fn unit_suffix_parsing_composes_with_comma_decimal_separator() {
        let text = "t;x\n0,5 sec;1\n1,25 sec;2\n2,5 sec;3\n3,0 sec;4\n4,75 sec;5\n";
        let table = parse_default(text.as_bytes()).unwrap();
        assert!(table.columns[0].is_numeric);
        assert_eq!(table.columns[0].values, vec![0.5, 1.25, 2.5, 3.0, 4.75]);
    }

    #[test]
    fn text_column_without_a_leading_number_is_not_coerced() {
        // "charge"/"rest"/"discharge" have no leading digit at all -- the
        // unit-suffix fallback must not turn a genuine text/grouping column
        // numeric.
        let text = "t,state\n0,charge\n1,charge\n2,rest\n3,rest\n4,discharge\n";
        let overrides = ParseOverrides {
            skip_lines: Some(0),
            ..Default::default()
        };
        let table = parse(text.as_bytes(), &overrides).unwrap();
        assert!(!table.columns[1].is_numeric);
        assert_eq!(
            table.columns[1].strings,
            vec!["charge", "charge", "rest", "rest", "discharge"]
        );
    }

    #[test]
    fn missing_tokens_become_nan_without_coercion_failure() {
        let text = "a,b\n1,NA\n2,NaN\n3,Inf\n4,-Inf\n5,null\n6,-\n7,\n";
        let table = parse_default(text.as_bytes()).unwrap();
        assert!(table.columns[1].is_numeric);
        assert!(table.columns[1].values.iter().all(|v| v.is_nan()));
        assert!(table.report.coercion_failures.is_empty());
    }

    #[test]
    fn non_numeric_column_is_retained_as_text_for_grouping() {
        // A literal text column embedded in the data block defeats the
        // strict full-auto terminal-run search by design (every field in a
        // row must be number-or-missing for that row to count -- see
        // §4.2 step 2), so this is exactly the case the manual "skip N
        // lines" override exists for (§4.2's stated escape hatch).
        let text = "t,state\n0,charge\n1,charge\n2,rest\n3,rest\n4,discharge\n";
        let overrides = ParseOverrides {
            skip_lines: Some(0),
            ..Default::default()
        };
        let table = parse(text.as_bytes(), &overrides).unwrap();
        assert_eq!(table.report.delimiter, Delimiter::Comma);
        assert!(table.report.header_present);
        assert!(!table.columns[1].is_numeric);
        assert_eq!(
            table.columns[1].strings,
            vec!["charge", "charge", "rest", "rest", "discharge"]
        );
        assert!(table.columns[1].values.is_empty());
    }

    #[test]
    fn duplicate_header_names_are_deduplicated() {
        let text = "x,x,x\n1,2,3\n4,5,6\n7,8,9\n10,11,12\n13,14,15\n";
        let table = parse_default(text.as_bytes()).unwrap();
        assert_eq!(table.columns[0].name, "x");
        assert_eq!(table.columns[1].name, "x (2)");
        assert_eq!(table.columns[2].name, "x (3)");
    }

    #[test]
    fn empty_header_cell_is_named_by_position() {
        let text = "a,,c\n1,2,3\n4,5,6\n7,8,9\n10,11,12\n13,14,15\n";
        let table = parse_default(text.as_bytes()).unwrap();
        assert_eq!(table.columns[1].name, "col2");
    }

    #[test]
    fn override_delimiter_forces_reparse() {
        // Would auto-detect as comma-delimited; force semicolon and expect
        // a single (wrong-looking but deliberate) column.
        let text = "a,b\n1,2\n3,4\n5,6\n7,8\n9,10\n";
        let overrides = ParseOverrides {
            delimiter: Some(Delimiter::Semicolon),
            ..Default::default()
        };
        let table = parse(text.as_bytes(), &overrides).unwrap();
        assert_eq!(table.report.delimiter, Delimiter::Semicolon);
        assert_eq!(table.report.n_columns, 1);
    }

    #[test]
    fn override_skip_lines_and_header_present() {
        let text = "junk1\njunk2\njunk3\na,b\n1,2\n3,4\n5,6\n7,8\n9,10\n";
        let overrides = ParseOverrides {
            skip_lines: Some(3),
            header_present: Some(true),
            ..Default::default()
        };
        let table = parse(text.as_bytes(), &overrides).unwrap();
        assert_eq!(table.report.preamble_lines_skipped, 3);
        assert_eq!(table.columns[0].name, "a");
        assert_eq!(table.report.n_rows, 5);
    }

    #[test]
    fn override_header_present_false_treats_header_row_as_data() {
        let text = "a,b\n1,2\n3,4\n5,6\n7,8\n9,10\n";
        let overrides = ParseOverrides {
            header_present: Some(false),
            skip_lines: Some(0),
            ..Default::default()
        };
        let table = parse(text.as_bytes(), &overrides).unwrap();
        assert!(!table.report.header_present);
        assert_eq!(table.columns[0].name, "col1");
        // "a" is non-numeric, so it becomes a coercion failure in what is
        // now the first data row rather than a header.
        assert_eq!(table.report.n_rows, 6);
    }

    #[test]
    fn no_data_block_found_names_delimiters_tried() {
        let text = "just some free text\nwith no structure at all\n";
        let err = parse_default(text.as_bytes()).unwrap_err();
        match err {
            ParseError::NoDataBlockFound { delimiters_tried } => {
                assert_eq!(delimiters_tried.len(), 4);
            }
            other => panic!("expected NoDataBlockFound, got {other:?}"),
        }
    }

    #[test]
    fn empty_file_is_rejected() {
        let err = parse_default(b"").unwrap_err();
        assert_eq!(err, ParseError::EmptyFile);
    }

    #[test]
    fn file_too_large_is_rejected() {
        let err = parse_default_size_only(MAX_FILE_BYTES + 1);
        assert!(matches!(err, ParseError::FileTooLarge { .. }));
    }

    fn parse_default_size_only(size: usize) -> ParseError {
        let bytes = vec![b'a'; size];
        parse_default(&bytes).unwrap_err()
    }
}
