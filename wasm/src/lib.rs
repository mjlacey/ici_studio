//! Thin wasm-bindgen shim over `ici-core`. JSON in, JSON out (playbook §5's
//! "never let NaN/Infinity reach the JS boundary as a bare float" is
//! already handled for free: `serde_json` serialises Rust `NaN`/`Infinity`
//! as JSON `null`, and §7.8 explicitly wants that, not a sentinel). No
//! maths lives here -- see `core` for that; DTOs here only reshape core
//! types into the JSON the UI expects (camelCase, string enums).

use ici_core::decimate::lttb;
use ici_core::deriv::local_poly_derivative;
use ici_core::derive::{derive, AnalysisTable, DeriveConfig};
use ici_core::optimal_window::{self, GridConfig, RestCandidate};
use ici_core::parse::{
    self, Column, DecimalSeparator, Delimiter, Encoding, ParseError, ParsedTable,
};
use ici_core::regress::{fit_rest_window, rest_regression, RegressionConfig, RestFitError};
use ici_core::segment::{
    self, interruption_summary, reanchor_charge, split_current_levels, AnchorPoint,
    IciDetectionConfig, QAnchorConfig, SegmentConfig, SegmentLog, SegmentedData, State,
    SummaryConfig,
};
use ici_core::spline::{smooth_bspline_vec, Direction, DirectionUsed, SplineConfig, SplineDiagnostics};
use ici_core::types::{
    interpolate_endpoint, make_group_id, select_final_window, select_initial_window,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use wasm_bindgen::prelude::*;

// ---------------------------------------------------------------------
// DTOs
// ---------------------------------------------------------------------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ParseReportDto {
    encoding: &'static str,
    delimiter: &'static str,
    decimal_separator: &'static str,
    preamble_lines_skipped: usize,
    header_present: bool,
    header_synthesized: bool,
    n_rows: usize,
    n_columns: usize,
    trailing_column_dropped: bool,
    ragged_rows_dropped: usize,
    ragged_row_line_numbers: Vec<usize>,
    coercion_failures: Vec<CoercionFailureDto>,
    warnings: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CoercionFailureDto {
    column: String,
    count: usize,
    first_rows: Vec<usize>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ColumnInfoDto {
    name: String,
    is_numeric: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ColumnStatsDto {
    is_numeric: bool,
    n_finite: usize,
    min: Option<f64>,
    median: Option<f64>,
    max: Option<f64>,
    distinct_count: Option<usize>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MonotonicityDto {
    is_monotonic: bool,
    first_offending_row: Option<usize>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PageDto {
    column_names: Vec<String>,
    rows: Vec<Vec<serde_json::Value>>,
    total_rows: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ParseErrorDto {
    message: String,
    delimiters_tried: Option<Vec<&'static str>>,
}

/// One rest's plotting-relevant boundaries + sorted step.t values (§10.1's
/// live "points in window" feedback needs every rest's step.t distribution,
/// not just the selected one -- shipped once here rather than re-fetched on
/// every drag frame).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RestBoundaryDto {
    index: u32,
    rest_id: u32,
    group_id: u32,
    t_start: f64,
    t_end: f64,
    rest_step_ts: Vec<f64>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GroupKeyColumnsDto {
    grouping_column_names: Vec<String>,
    /// Keyed by `group_id` as a string (JSON object keys are always
    /// strings) -- values in the same order as `grouping_column_names`.
    values: HashMap<String, Vec<String>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SegmentationSummaryDto {
    total_rests: usize,
    rest_boundaries: Vec<RestBoundaryDto>,
    /// §12.3's run-log wants these two counts by name -- straight from
    /// `SegmentLog`, previously computed by `segment()` and discarded.
    reversal_rows_dropped: usize,
    incomplete_final_rows_dropped: usize,
    /// §7.2: a Rest run at the very start of a group's data, with no
    /// preceding active run, dropped by `segment()` and counted here so the
    /// run log can flag it rather than let it surface as a mysterious
    /// regression failure.
    leading_rest_rows_dropped: usize,
    /// §7.1: when a group ends up with no rests or no active samples (the
    /// current `state_threshold` misclassified the instrument's noise
    /// floor), a suggested threshold from `split_current_levels()` over
    /// that group's raw current -- surfaced as a blocking "use suggested
    /// threshold" banner rather than applied silently.
    threshold_suggestions: Vec<ThresholdSuggestionDto>,
    /// Rows dropped by `IciDetectionConfig` -- outside "the ICI cycle"
    /// itself (a capacity-check cycle, an OCV rest, a DCIR leg, ...).
    non_ici_rows_dropped: usize,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ThresholdSuggestionDto {
    group_id: u32,
    suggested_threshold: f64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RestPointsDto {
    active_step_t: Vec<f64>,
    active_voltage: Vec<f64>,
    active_current: Vec<f64>,
    rest_step_t: Vec<f64>,
    rest_voltage: Vec<f64>,
    rest_current: Vec<f64>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DecimatedSeriesDto {
    t: Vec<f64>,
    e: Vec<f64>,
    i: Vec<f64>,
}

/// §10.2's live single-rest preview fit result. `ok = false` still carries
/// `nPointsInWindow`/`e`/`i`/`i0` -- §10.1 wants the point count visible
/// even when the fit itself can't run ("a window that yields 3 points is a
/// silent disaster otherwise"). No non-physical NaN'ing: this is an
/// interactive diagnostic view, so a negative R/k is shown, not hidden.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RestPreviewDto {
    ok: bool,
    n_points_in_window: usize,
    e: Option<f64>,
    i: Option<f64>,
    i0: Option<f64>,
    e0: Option<f64>,
    e0_err: Option<f64>,
    s: Option<f64>,
    s_err: Option<f64>,
    n_pts: Option<usize>,
    r2: Option<f64>,
    adj_r2: Option<f64>,
    rmse: Option<f64>,
    edge_mae_ratio: Option<f64>,
    edge_max_z: Option<f64>,
    r: Option<f64>,
    r_err: Option<f64>,
    k: Option<f64>,
    k_err: Option<f64>,
    error: Option<String>,
}

// -- Milestone 8: Stage A/B DTOs. --

/// Column-oriented (mirrors `core::derive::AnalysisTable` directly), bounded
/// by rest count (hundreds-low-thousands), not row count -- fine per §2.1.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AnalysisTableDto {
    group_id: Vec<u32>,
    cyc_n: Vec<f64>,
    state: Vec<&'static str>,
    rest: Vec<u32>,
    t: Vec<f64>,
    step_t: Vec<f64>,
    e: Vec<f64>,
    i: Vec<f64>,
    q: Vec<f64>,
    e0: Vec<f64>,
    e0_err: Vec<f64>,
    s: Vec<f64>,
    s_err: Vec<f64>,
    i0: Vec<f64>,
    n_pts: Vec<usize>,
    r2: Vec<f64>,
    adj_r2: Vec<f64>,
    rmse: Vec<f64>,
    edge_mae_ratio: Vec<f64>,
    edge_max_z: Vec<f64>,
    r: Vec<f64>,
    r_err: Vec<f64>,
    k: Vec<f64>,
    k_err: Vec<f64>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RegressionFailureDto {
    group_id: u32,
    rest: u32,
    reason: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct NonphysicalReportDto {
    r_by_state: HashMap<String, usize>,
    k_by_state: HashMap<String, usize>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StageAResultDto {
    segmentation: SegmentationSummaryDto,
    analysis_table: AnalysisTableDto,
    regression_failures: Vec<RegressionFailureDto>,
    nonphysical_report: NonphysicalReportDto,
}

/// Mirrors `core::spline::SplineDiagnostics`; `None` when a smoothing group
/// had fewer than 8 distinct x values (`smooth_bspline_vec`'s own NA path).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SplineDiagnosticsDto {
    direction_used: &'static str,
    k_effective: usize,
    lambda: f64,
    edf: f64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SmoothingGroupDto {
    key: String,
    label: String,
    n: usize,
    e0: Option<SplineDiagnosticsDto>,
    k: Option<SplineDiagnosticsDto>,
    r: Option<SplineDiagnosticsDto>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StageBResultDto {
    e0_smooth: Vec<f64>,
    k_smooth: Vec<f64>,
    r_smooth: Vec<f64>,
    // `rename_all = "camelCase"` would produce "dVDQ"/"dQDV" here (each
    // underscore-separated single letter gets capitalized) -- explicit
    // renames to match the dV/dQ notation the TS side expects.
    #[serde(rename = "dVdQ")]
    d_v_d_q: Vec<f64>,
    #[serde(rename = "dQdV")]
    d_q_d_v: Vec<f64>,
    /// Aligned to the analysis table rows -- which `groups[].key` each row
    /// belongs to. Plots 3/4 (§11.3) need this to draw one smoothing line
    /// per group; the group diagnostics alone don't carry row membership.
    row_group_key: Vec<String>,
    groups: Vec<SmoothingGroupDto>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SmoothingKeyDto {
    use_cyc_n: bool,
    use_state: bool,
    grouping_columns: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SmootherConfigDto {
    monotonic: bool,
    direction: String,
    k: usize,
    m: usize,
}

// -- Milestone 9: "Estimate optimal window" (§10.3) DTOs. --

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SampledRestDto {
    group_id: u32,
    rest_id: u32,
    state: &'static str,
    q: f64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SetupOptimalWindowDto {
    sampled_rests: Vec<SampledRestDto>,
    t_max_observed: f64,
    heterogeneous_lengths: bool,
    total_candidates: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CandidateScoreDto {
    t_min: f64,
    t_max: f64,
    mean_adj_r2: f64,
    median_adj_r2: f64,
    n_valid: usize,
    n_sampled: usize,
    median_n_pts: f64,
    median_edge_max_z: f64,
    rejected: bool,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct ParseOverridesDto {
    encoding: Option<String>,
    delimiter: Option<String>,
    decimal_separator: Option<String>,
    skip_lines: Option<usize>,
    header_present: Option<bool>,
}

fn encoding_str(e: Encoding) -> &'static str {
    match e {
        Encoding::Utf8 => "Utf8",
        Encoding::Cp1252 => "Cp1252",
    }
}

fn delimiter_str(d: Delimiter) -> &'static str {
    match d {
        Delimiter::Tab => "Tab",
        Delimiter::Comma => "Comma",
        Delimiter::Semicolon => "Semicolon",
        Delimiter::Pipe => "Pipe",
    }
}

fn decimal_separator_str(d: DecimalSeparator) -> &'static str {
    match d {
        DecimalSeparator::Dot => "Dot",
        DecimalSeparator::Comma => "Comma",
    }
}

fn parse_encoding(s: &str) -> Result<Encoding, String> {
    match s {
        "Utf8" => Ok(Encoding::Utf8),
        "Cp1252" => Ok(Encoding::Cp1252),
        other => Err(format!("unknown encoding override '{other}'")),
    }
}

fn parse_delimiter(s: &str) -> Result<Delimiter, String> {
    match s {
        "Tab" => Ok(Delimiter::Tab),
        "Comma" => Ok(Delimiter::Comma),
        "Semicolon" => Ok(Delimiter::Semicolon),
        "Pipe" => Ok(Delimiter::Pipe),
        other => Err(format!("unknown delimiter override '{other}'")),
    }
}

fn parse_decimal_separator(s: &str) -> Result<DecimalSeparator, String> {
    match s {
        "Dot" => Ok(DecimalSeparator::Dot),
        "Comma" => Ok(DecimalSeparator::Comma),
        other => Err(format!("unknown decimal separator override '{other}'")),
    }
}

fn overrides_from_json(overrides_json: &str) -> Result<parse::ParseOverrides, JsValue> {
    if overrides_json.trim().is_empty() {
        return Ok(parse::ParseOverrides::default());
    }
    let dto: ParseOverridesDto = serde_json::from_str(overrides_json)
        .map_err(|e| js_error(format!("invalid overrides JSON: {e}")))?;

    let encoding = dto
        .encoding
        .as_deref()
        .map(parse_encoding)
        .transpose()
        .map_err(js_error)?;
    let delimiter = dto
        .delimiter
        .as_deref()
        .map(parse_delimiter)
        .transpose()
        .map_err(js_error)?;
    let decimal_separator = dto
        .decimal_separator
        .as_deref()
        .map(parse_decimal_separator)
        .transpose()
        .map_err(js_error)?;

    Ok(parse::ParseOverrides {
        encoding,
        delimiter,
        decimal_separator,
        skip_lines: dto.skip_lines,
        header_present: dto.header_present,
    })
}

fn js_error(message: impl Into<String>) -> JsValue {
    let dto = ParseErrorDto {
        message: message.into(),
        delimiters_tried: None,
    };
    JsValue::from_str(&serde_json::to_string(&dto).unwrap_or_else(|_| "{}".to_string()))
}

fn parse_error_to_js(err: ParseError) -> JsValue {
    let (message, delimiters_tried) = match &err {
        ParseError::NoDataBlockFound { delimiters_tried } => (
            err.to_string(),
            Some(delimiters_tried.iter().map(|d| delimiter_str(*d)).collect()),
        ),
        _ => (err.to_string(), None),
    };
    let dto = ParseErrorDto {
        message,
        delimiters_tried,
    };
    JsValue::from_str(&serde_json::to_string(&dto).unwrap_or_else(|_| "{}".to_string()))
}

fn report_dto(report: &parse::ParseReport) -> ParseReportDto {
    ParseReportDto {
        encoding: encoding_str(report.encoding),
        delimiter: delimiter_str(report.delimiter),
        decimal_separator: decimal_separator_str(report.decimal_separator),
        preamble_lines_skipped: report.preamble_lines_skipped,
        header_present: report.header_present,
        header_synthesized: report.header_synthesized,
        n_rows: report.n_rows,
        n_columns: report.n_columns,
        trailing_column_dropped: report.trailing_column_dropped,
        ragged_rows_dropped: report.ragged_rows_dropped,
        ragged_row_line_numbers: report.ragged_row_line_numbers.clone(),
        coercion_failures: report
            .coercion_failures
            .iter()
            .map(|c| CoercionFailureDto {
                column: c.column.clone(),
                count: c.count,
                first_rows: c.first_rows.clone(),
            })
            .collect(),
        warnings: report.warnings.clone(),
    }
}

fn column_stats(column: &Column) -> ColumnStatsDto {
    if column.is_numeric {
        let mut finite: Vec<f64> = column
            .values
            .iter()
            .copied()
            .filter(|v| v.is_finite())
            .collect();
        finite.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let n_finite = finite.len();
        let (min, median, max) = if n_finite == 0 {
            (None, None, None)
        } else {
            let median = if n_finite % 2 == 1 {
                finite[n_finite / 2]
            } else {
                (finite[n_finite / 2 - 1] + finite[n_finite / 2]) / 2.0
            };
            (Some(finite[0]), Some(median), Some(finite[n_finite - 1]))
        };
        ColumnStatsDto {
            is_numeric: true,
            n_finite,
            min,
            median,
            max,
            distinct_count: None,
        }
    } else {
        let mut distinct: Vec<&str> = column.strings.iter().map(String::as_str).collect();
        distinct.sort_unstable();
        distinct.dedup();
        ColumnStatsDto {
            is_numeric: false,
            n_finite: column.strings.iter().filter(|s| !s.is_empty()).count(),
            min: None,
            median: None,
            max: None,
            distinct_count: Some(distinct.len()),
        }
    }
}

// ---------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------

/// Parses `bytes` and returns a [`ParsedDataset`] that owns the parsed
/// columns for the lifetime of the caller's WASM instance (the worker, per
/// §2.1 -- the full columnar dataset never needs to leave this instance).
#[wasm_bindgen]
pub fn parse_file(bytes: &[u8], overrides_json: &str) -> Result<ParsedDataset, JsValue> {
    let overrides = overrides_from_json(overrides_json)?;
    let table = parse::parse(bytes, &overrides).map_err(parse_error_to_js)?;
    Ok(ParsedDataset {
        inner: table,
        segmentation: None,
        segmentation_log: None,
        raw_series: None,
        group_key_columns: None,
        grouping_column_names: Vec::new(),
        analysis: None,
        optimal_window_sample: None,
        optimal_window_grid: None,
        threshold_suggestions: Vec::new(),
    })
}

#[wasm_bindgen]
pub struct ParsedDataset {
    inner: ParsedTable,
    /// §7.1-7.5 segmentation over the currently mapped+scaled columns,
    /// cached here after `do_segment` (called by both `runSegmentation` and
    /// `runStageA`) so `restPointsJson`/`decimatedSeriesJson` don't re-run
    /// it per call. `None` until the first successful segmentation.
    segmentation: Option<SegmentedData>,
    /// §12.3's run-log wants the counts of rows `segment()` itself dropped
    /// (unrested reversals, incomplete final step) -- cached alongside
    /// `segmentation` since both come from the same `do_segment` call.
    segmentation_log: Option<SegmentLog>,
    /// The mapped+scaled `(t, E, I)` triple, cached independently of
    /// `segmentation` -- `decimatedSeriesJson` (Plot 1's raw time series)
    /// reads from here rather than `segmentation`'s own arrays specifically
    /// so a non-ICI region (§ `IciDetectionConfig`) that `do_segment` drops
    /// from `segmentation` for Stage A's purposes doesn't also vanish from
    /// the raw view -- the whole point of shading it there instead of
    /// hiding it. Was previously read straight from `segmentation` (a pure
    /// optimisation, since §7.4/7.5's own drops are a tiny fraction of the
    /// file); that assumption no longer holds once non-ICI drops can be a
    /// large fraction, hence the separate cache.
    raw_series: Option<(Vec<f64>, Vec<f64>, Vec<f64>)>,
    /// `group_id -> that group's grouping-column string values`, cached
    /// alongside `segmentation`. Stage B's smoothing key (§8.1) can select
    /// an arbitrary *subset* of the grouping columns, which the combined
    /// numeric `group_id` alone can't reconstruct -- this lookup can.
    group_key_columns: Option<HashMap<u32, Vec<String>>>,
    /// The grouping column *names*, in the same order as each
    /// `group_key_columns` value -- `runStageB` needs this to resolve the
    /// smoothing key's selected column names to positions.
    grouping_column_names: Vec<String>,
    /// Stage A's merged per-interruption `R`/`k` table, cached after
    /// `runStageA` so `runStageB` (smoothing/derivatives) doesn't need
    /// Stage A's inputs re-supplied. `None` until the first successful
    /// `runStageA`.
    analysis: Option<AnalysisTable>,
    /// §10.3's sampled rests + candidate grid, cached by `setupOptimalWindow`
    /// so `scoreOptimalWindowChunk` can be called repeatedly (once per
    /// chunk, from the worker's cancellable loop) without re-sampling.
    optimal_window_sample: Option<Vec<RestCandidate>>,
    optimal_window_grid: Option<Vec<(f64, f64)>>,
    /// §7.1 threshold-suggestion banner data, recomputed by `do_segment`
    /// each run since it depends on the current `state_threshold`.
    threshold_suggestions: Vec<ThresholdSuggestionDto>,
}

#[wasm_bindgen]
impl ParsedDataset {
    #[wasm_bindgen(js_name = reportJson)]
    pub fn report_json(&self) -> String {
        serde_json::to_string(&report_dto(&self.inner.report)).unwrap_or_else(|_| "{}".to_string())
    }

    #[wasm_bindgen(js_name = columnsJson)]
    pub fn columns_json(&self) -> String {
        let cols: Vec<ColumnInfoDto> = self
            .inner
            .columns
            .iter()
            .map(|c| ColumnInfoDto {
                name: c.name.clone(),
                is_numeric: c.is_numeric,
            })
            .collect();
        serde_json::to_string(&cols).unwrap_or_else(|_| "[]".to_string())
    }

    #[wasm_bindgen(js_name = columnStatsJson)]
    pub fn column_stats_json(&self, name: &str) -> String {
        match self.inner.columns.iter().find(|c| c.name == name) {
            Some(col) => {
                serde_json::to_string(&column_stats(col)).unwrap_or_else(|_| "null".to_string())
            }
            None => "null".to_string(),
        }
    }

    #[wasm_bindgen(js_name = nRows)]
    pub fn n_rows(&self) -> usize {
        self.inner.report.n_rows
    }

    /// §5.3: is the mapped time column monotonically non-decreasing? Used
    /// to gate Stage A ("do not proceed until resolved").
    #[wasm_bindgen(js_name = checkTimeMonotonicJson)]
    pub fn check_time_monotonic_json(&self, time_column: &str) -> String {
        let dto = match self
            .inner
            .columns
            .iter()
            .find(|c| c.name == time_column && c.is_numeric)
        {
            Some(col) => {
                let mut first_offending_row: Option<usize> = None;
                for i in 1..col.values.len() {
                    if col.values[i] < col.values[i - 1] {
                        first_offending_row = Some(i);
                        break;
                    }
                }
                MonotonicityDto {
                    is_monotonic: first_offending_row.is_none(),
                    first_offending_row,
                }
            }
            None => MonotonicityDto {
                is_monotonic: true,
                first_offending_row: None,
            },
        };
        serde_json::to_string(&dto).unwrap_or_else(|_| "null".to_string())
    }

    /// A page of `limit` rows starting at `offset`, all columns, for the
    /// raw data inspector (§5.2). Cell values are numbers, strings, or
    /// `null` (NaN / missing).
    #[wasm_bindgen(js_name = pageJson)]
    pub fn page_json(&self, offset: usize, limit: usize) -> String {
        let total_rows = self.inner.report.n_rows;
        let end = (offset + limit).min(total_rows);
        let column_names: Vec<String> = self.inner.columns.iter().map(|c| c.name.clone()).collect();

        let mut rows: Vec<Vec<serde_json::Value>> = Vec::with_capacity(end.saturating_sub(offset));
        for row_idx in offset..end {
            let mut row = Vec::with_capacity(self.inner.columns.len());
            for col in &self.inner.columns {
                let value = if col.is_numeric {
                    let v = col.values.get(row_idx).copied().unwrap_or(f64::NAN);
                    if v.is_finite() {
                        serde_json::json!(v)
                    } else {
                        serde_json::Value::Null
                    }
                } else {
                    match col.strings.get(row_idx) {
                        Some(s) => serde_json::json!(s),
                        None => serde_json::Value::Null,
                    }
                };
                row.push(value);
            }
            rows.push(row);
        }

        let dto = PageDto {
            column_names,
            rows,
            total_rows,
        };
        serde_json::to_string(&dto).unwrap_or_else(|_| "{}".to_string())
    }

    /// §5.1: does `charge_column` already reset near zero at the start of
    /// every `(cyc.n, state)` group? Backs the "anchoring will be a no-op"
    /// info note.
    #[wasm_bindgen(js_name = detectPrenormalizedCharge)]
    pub fn detect_prenormalized_charge(
        &self,
        charge_column: &str,
        cycle_column: &str,
        current_column: &str,
        state_threshold: f64,
    ) -> Result<bool, JsValue> {
        let get_numeric = |name: &str| -> Result<&[f64], JsValue> {
            self.inner
                .columns
                .iter()
                .find(|c| c.name == name && c.is_numeric)
                .map(|c| c.values.as_slice())
                .ok_or_else(|| js_error(format!("column '{name}' not found or not numeric")))
        };
        let charge = get_numeric(charge_column)?;
        let cyc_n = get_numeric(cycle_column)?;
        let current = get_numeric(current_column)?;
        Ok(ici_core::types::looks_prenormalized_charge(
            cyc_n,
            current,
            charge,
            state_threshold,
        ))
    }

    // -- §5.3 validation actions. Each mutates this dataset in place; the
    // caller must re-fetch report/columns/pages afterwards (n_rows changes). --

    /// Row indices where `column` is non-finite (for the "drop these rows" action).
    #[wasm_bindgen(js_name = nonFiniteRowIndices)]
    pub fn non_finite_row_indices(&self, column: &str) -> Vec<usize> {
        match self
            .inner
            .columns
            .iter()
            .find(|c| c.name == column && c.is_numeric)
        {
            Some(col) => col
                .values
                .iter()
                .enumerate()
                .filter(|(_, v)| !v.is_finite())
                .map(|(i, _)| i)
                .collect(),
            None => Vec::new(),
        }
    }

    /// Row indices `i` where `column[i] < column[i-1]` (for "drop the decreasing rows").
    #[wasm_bindgen(js_name = decreasingRowIndices)]
    pub fn decreasing_row_indices(&self, column: &str) -> Vec<usize> {
        match self
            .inner
            .columns
            .iter()
            .find(|c| c.name == column && c.is_numeric)
        {
            Some(col) => (1..col.values.len())
                .filter(|&i| col.values[i] < col.values[i - 1])
                .collect(),
            None => Vec::new(),
        }
    }

    /// Drops the given row indices from every column, in place.
    #[wasm_bindgen(js_name = dropRows)]
    pub fn drop_rows(&mut self, indices: Vec<usize>) {
        let drop_set: std::collections::HashSet<usize> = indices.into_iter().collect();
        if drop_set.is_empty() {
            return;
        }
        for col in &mut self.inner.columns {
            if col.is_numeric {
                col.values = col
                    .values
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| !drop_set.contains(i))
                    .map(|(_, v)| *v)
                    .collect();
            } else {
                col.strings = col
                    .strings
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| !drop_set.contains(i))
                    .map(|(_, v)| v.clone())
                    .collect();
            }
        }
        self.inner.report.n_rows = self.inner.report.n_rows.saturating_sub(drop_set.len());
    }

    /// Stably re-sorts every column by ascending `column`, in place.
    #[wasm_bindgen(js_name = sortByColumn)]
    pub fn sort_by_column(&mut self, column: &str) -> Result<(), JsValue> {
        let values = self
            .inner
            .columns
            .iter()
            .find(|c| c.name == column && c.is_numeric)
            .map(|c| c.values.clone())
            .ok_or_else(|| js_error(format!("column '{column}' not found or not numeric")))?;

        let mut order: Vec<usize> = (0..values.len()).collect();
        order.sort_by(|&a, &b| {
            values[a]
                .partial_cmp(&values[b])
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        for col in &mut self.inner.columns {
            if col.is_numeric {
                col.values = order.iter().map(|&i| col.values[i]).collect();
            } else {
                col.strings = order.iter().map(|&i| col.strings[i].clone()).collect();
            }
        }
        Ok(())
    }

    // -- Milestone 7/8: segmentation (§7.1-7.5), Stage A, Stage B. --

    /// Scales the mapped columns to internal units (§2.2 -- this is where
    /// that conversion actually happens), runs §7.1-7.5 segmentation +
    /// §6 Q anchoring, and caches the result (plus the grouping-column
    /// lookup Stage B needs) for `restPointsJson`/`decimatedSeriesJson`/
    /// `runStageB`. Shared by `runSegmentation` (Plot 1/2's cheap reactive
    /// path) and `runStageA`, so the two can never drift apart.
    #[allow(clippy::too_many_arguments)]
    fn do_segment(
        &mut self,
        time_column: &str,
        time_scale: f64,
        cycle_column: &str,
        current_column: &str,
        current_scale: f64,
        voltage_column: &str,
        voltage_scale: f64,
        charge_column: &str,
        charge_scale: f64,
        grouping_columns_json: &str,
        state_threshold: f64,
        drop_unrested_reversals: bool,
        charge_anchor: &str,
        discharge_anchor: &str,
        ici_detection_enabled: bool,
        non_ici_max_rest_duration_s: f64,
        non_ici_min_repeat_count: usize,
    ) -> Result<(), JsValue> {
        let find_numeric = |name: &str| -> Result<&Column, JsValue> {
            self.inner
                .columns
                .iter()
                .find(|c| c.name == name)
                .ok_or_else(|| js_error(format!("column '{name}' not found")))
        };
        let time_col = find_numeric(time_column)?;
        let cycle_col = find_numeric(cycle_column)?;
        let current_col = find_numeric(current_column)?;
        let voltage_col = find_numeric(voltage_column)?;
        let charge_col = find_numeric(charge_column)?;
        if !time_col.is_numeric
            || !cycle_col.is_numeric
            || !current_col.is_numeric
            || !voltage_col.is_numeric
            || !charge_col.is_numeric
        {
            return Err(js_error("all mapped fields must be numeric columns"));
        }

        let n = self.inner.report.n_rows;
        let t: Vec<f64> = time_col.values.iter().map(|v| v * time_scale).collect();
        let cyc_n: Vec<f64> = cycle_col.values.clone();
        let current: Vec<f64> = current_col
            .values
            .iter()
            .map(|v| v * current_scale)
            .collect();
        let voltage: Vec<f64> = voltage_col
            .values
            .iter()
            .map(|v| v * voltage_scale)
            .collect();
        let charge: Vec<f64> = charge_col.values.iter().map(|v| v * charge_scale).collect();

        // Cached independently of `segmentation` -- see `raw_series`'s own
        // doc comment for why `decimatedSeriesJson` needs this rather than
        // reading `self.segmentation`'s (possibly non-ICI-filtered) arrays.
        self.raw_series = Some((t.clone(), voltage.clone(), current.clone()));

        let grouping_columns: Vec<String> = serde_json::from_str(grouping_columns_json)
            .map_err(|e| js_error(format!("invalid groupingColumns JSON: {e}")))?;
        let grouping_cols: Vec<Vec<String>> = grouping_columns
            .iter()
            .map(|name| find_numeric(name).map(column_as_strings))
            .collect::<Result<_, _>>()?;
        let group_id = if grouping_cols.is_empty() {
            vec![0u32; n]
        } else {
            make_group_id(&grouping_cols, n)
        };

        let mut group_key_columns: HashMap<u32, Vec<String>> = HashMap::new();
        for row in 0..n {
            group_key_columns
                .entry(group_id[row])
                .or_insert_with(|| grouping_cols.iter().map(|c| c[row].clone()).collect());
        }

        let config = SegmentConfig {
            state_threshold,
            drop_unrested_reversals,
            ici_detection: IciDetectionConfig {
                enabled: ici_detection_enabled,
                max_rest_duration_s: non_ici_max_rest_duration_s,
                min_repeat_count: non_ici_min_repeat_count,
            },
        };
        let (mut seg, log) =
            segment::segment(&group_id, &t, &cyc_n, &current, &voltage, &charge, &config);

        self.threshold_suggestions = threshold_suggestions(&group_id, &current, &seg);

        let anchor_config = QAnchorConfig {
            charge: parse_anchor(charge_anchor)?,
            discharge: parse_anchor(discharge_anchor)?,
        };
        seg.charge = reanchor_charge(&seg, &anchor_config);

        self.segmentation = Some(seg);
        self.segmentation_log = Some(log);
        self.group_key_columns = Some(group_key_columns);
        self.grouping_column_names = grouping_columns;
        Ok(())
    }

    /// Thin wrapper around `do_segment` for Plot 1/2's cheap, reactive path
    /// (§10-11) -- doesn't run the regression/derive/smoothing passes.
    /// Returns a small summary (bounded by rest count, not row count --
    /// §2.1).
    #[wasm_bindgen(js_name = runSegmentation)]
    #[allow(clippy::too_many_arguments)]
    pub fn run_segmentation(
        &mut self,
        time_column: &str,
        time_scale: f64,
        cycle_column: &str,
        current_column: &str,
        current_scale: f64,
        voltage_column: &str,
        voltage_scale: f64,
        charge_column: &str,
        charge_scale: f64,
        grouping_columns_json: &str,
        state_threshold: f64,
        drop_unrested_reversals: bool,
        charge_anchor: &str,
        discharge_anchor: &str,
        ici_detection_enabled: bool,
        non_ici_max_rest_duration_s: f64,
        non_ici_min_repeat_count: usize,
    ) -> Result<String, JsValue> {
        self.do_segment(
            time_column,
            time_scale,
            cycle_column,
            current_column,
            current_scale,
            voltage_column,
            voltage_scale,
            charge_column,
            charge_scale,
            grouping_columns_json,
            state_threshold,
            drop_unrested_reversals,
            charge_anchor,
            discharge_anchor,
            ici_detection_enabled,
            non_ici_max_rest_duration_s,
            non_ici_min_repeat_count,
        )?;
        let summary = segmentation_summary_dto(
            self.segmentation.as_ref().unwrap(),
            self.segmentation_log.as_ref().unwrap(),
            self.threshold_suggestions.clone(),
        );
        Ok(serde_json::to_string(&summary).unwrap_or_else(|_| "{}".to_string()))
    }

    /// §12.1's TSV export needs each row's actual grouping-column string
    /// values, not just the synthetic `group_id` int -- `group_key_columns`
    /// already holds exactly that (populated by `do_segment`, otherwise only
    /// used internally by `runStageB`), just never exposed publicly before.
    #[wasm_bindgen(js_name = groupKeyColumnsJson)]
    pub fn group_key_columns_json(&self) -> Result<String, JsValue> {
        let group_key_columns = self
            .group_key_columns
            .as_ref()
            .ok_or_else(|| js_error("segmentation has not run yet"))?;
        let values: HashMap<String, Vec<String>> = group_key_columns
            .iter()
            .map(|(group_id, cols)| (group_id.to_string(), cols.clone()))
            .collect();
        let dto = GroupKeyColumnsDto {
            grouping_column_names: self.grouping_column_names.clone(),
            values,
        };
        Ok(serde_json::to_string(&dto).unwrap_or_else(|_| "{}".to_string()))
    }

    /// §7: Stage A. Runs `do_segment` then the interruption summary, the
    /// per-rest regression, and the `R`/`k` derivation -- exactly
    /// `core/tests/golden.rs`'s sequence. Caches the resulting
    /// `AnalysisTable` for `runStageB`.
    #[wasm_bindgen(js_name = runStageA)]
    #[allow(clippy::too_many_arguments)]
    pub fn run_stage_a(
        &mut self,
        time_column: &str,
        time_scale: f64,
        cycle_column: &str,
        current_column: &str,
        current_scale: f64,
        voltage_column: &str,
        voltage_scale: f64,
        charge_column: &str,
        charge_scale: f64,
        grouping_columns_json: &str,
        state_threshold: f64,
        drop_unrested_reversals: bool,
        charge_anchor: &str,
        discharge_anchor: &str,
        ici_detection_enabled: bool,
        non_ici_max_rest_duration_s: f64,
        non_ici_min_repeat_count: usize,
        t_min: f64,
        t_max: f64,
        voltage_interp_window: Option<f64>,
        current_avg_window: Option<f64>,
        edge_points: usize,
        legacy_compatibility: bool,
    ) -> Result<String, JsValue> {
        self.do_segment(
            time_column,
            time_scale,
            cycle_column,
            current_column,
            current_scale,
            voltage_column,
            voltage_scale,
            charge_column,
            charge_scale,
            grouping_columns_json,
            state_threshold,
            drop_unrested_reversals,
            charge_anchor,
            discharge_anchor,
            ici_detection_enabled,
            non_ici_max_rest_duration_s,
            non_ici_min_repeat_count,
        )?;
        let seg = self.segmentation.as_ref().unwrap();

        let summary_config = SummaryConfig {
            voltage_interpolation_window: voltage_interp_window,
            current_average_window: current_avg_window,
            legacy_compatibility,
        };
        let summary = interruption_summary(seg, &summary_config);

        let regression_config = RegressionConfig {
            regression_window: (t_min, t_max),
            current_average_window: current_avg_window,
            edge_points,
        };
        let (regression, regression_log) = rest_regression(seg, &regression_config);

        let (analysis, nonphysical_report) =
            derive(&summary, &regression, &DeriveConfig::default());

        let dto = StageAResultDto {
            segmentation: segmentation_summary_dto(
                seg,
                self.segmentation_log.as_ref().unwrap(),
                self.threshold_suggestions.clone(),
            ),
            analysis_table: analysis_table_dto(&analysis),
            regression_failures: regression_log
                .failed_rests
                .into_iter()
                .map(|(group_id, rest, reason)| RegressionFailureDto {
                    group_id,
                    rest,
                    reason,
                })
                .collect(),
            nonphysical_report: nonphysical_report_dto(&nonphysical_report),
        };

        self.analysis = Some(analysis);
        Ok(serde_json::to_string(&dto).unwrap_or_else(|_| "{}".to_string()))
    }

    /// §8: Stage B. Requires `runStageA` to have run first (returns an
    /// error otherwise). Groups the cached analysis rows by the smoothing
    /// key (§8.1: an arbitrary subset of grouping columns, plus optional
    /// `cyc.n`/`state`), then for each group runs `smooth_bspline_vec`
    /// (`E0`, `k`, and `R`-or-inherit-`k`) and `local_poly_derivative`
    /// (`dV/dQ`, `dQ/dV`) -- the same sequence `core/tests/golden.rs` uses.
    #[wasm_bindgen(js_name = runStageB)]
    #[allow(clippy::too_many_arguments)]
    pub fn run_stage_b(
        &self,
        smoothing_key_json: &str,
        e0_config_json: &str,
        k_config_json: &str,
        r_inherits_k: bool,
        r_config_json: &str,
        derivative_window: usize,
        derivative_degree: usize,
        excluded_row_indices_json: &str,
    ) -> Result<String, JsValue> {
        let analysis = self
            .analysis
            .as_ref()
            .ok_or_else(|| js_error("runStageA must succeed before runStageB"))?;
        let group_key_columns = self.group_key_columns.as_ref().unwrap();

        // §9: rows QC excludes from smoothing (flag-driven or a manual
        // override) never enter any group's fit input -- they simply keep
        // their initial NaN in the output arrays below.
        let excluded_rows: std::collections::HashSet<usize> = serde_json::from_str(excluded_row_indices_json)
            .map_err(|e| js_error(format!("invalid excludedRowIndices JSON: {e}")))?;

        let smoothing_key: SmoothingKeyDto = serde_json::from_str(smoothing_key_json)
            .map_err(|e| js_error(format!("invalid smoothingKey JSON: {e}")))?;
        let e0_config = spline_config_from_dto(
            &serde_json::from_str::<SmootherConfigDto>(e0_config_json)
                .map_err(|e| js_error(format!("invalid e0 smoother JSON: {e}")))?,
        )?;
        let k_config = spline_config_from_dto(
            &serde_json::from_str::<SmootherConfigDto>(k_config_json)
                .map_err(|e| js_error(format!("invalid k smoother JSON: {e}")))?,
        )?;
        let r_config = if r_inherits_k {
            k_config
        } else {
            spline_config_from_dto(
                &serde_json::from_str::<SmootherConfigDto>(r_config_json)
                    .map_err(|e| js_error(format!("invalid r smoother JSON: {e}")))?,
            )?
        };

        // (column name, its position in `group_key_columns`'s values) for
        // each selected grouping column -- `self.grouping_column_names` is
        // `do_segment`'s original grouping-column list, which may be a
        // *superset* of what the smoothing key selects (§8.1).
        let selected: Vec<(&str, usize)> = smoothing_key
            .grouping_columns
            .iter()
            .filter_map(|name| {
                self.grouping_column_names
                    .iter()
                    .position(|g| g == name)
                    .map(|idx| (name.as_str(), idx))
            })
            .collect();

        let n = analysis.rest.len();
        let mut order: Vec<String> = Vec::new();
        let mut groups: HashMap<String, Vec<usize>> = HashMap::new();
        let mut labels: HashMap<String, String> = HashMap::new();
        for i in 0..n {
            if excluded_rows.contains(&i) {
                continue;
            }
            let group_vals = &group_key_columns[&analysis.group_id[i]];
            let mut parts: Vec<String> = selected
                .iter()
                .map(|&(name, idx)| format!("{name}={}", group_vals[idx]))
                .collect();
            if smoothing_key.use_cyc_n {
                parts.push(format!("cyc.n={}", analysis.cyc_n[i]));
            }
            if smoothing_key.use_state {
                parts.push(analysis.state[i].as_str().to_string());
            }
            let key = parts.join("\u{1f}");
            let label = if parts.is_empty() {
                "(all rows)".to_string()
            } else {
                parts.join(", ")
            };
            groups
                .entry(key.clone())
                .or_insert_with(|| {
                    order.push(key.clone());
                    labels.insert(key.clone(), label);
                    Vec::new()
                })
                .push(i);
        }

        let mut e0_smooth = vec![f64::NAN; n];
        let mut k_smooth = vec![f64::NAN; n];
        let mut r_smooth = vec![f64::NAN; n];
        let mut dvdq = vec![f64::NAN; n];
        let mut dqdv = vec![f64::NAN; n];
        let mut row_group_key = vec![String::new(); n];
        let mut group_dtos: Vec<SmoothingGroupDto> = Vec::with_capacity(order.len());

        for key in &order {
            let idx = &groups[key];
            let q: Vec<f64> = idx.iter().map(|&i| analysis.q[i]).collect();
            let e0: Vec<f64> = idx.iter().map(|&i| analysis.e0[i]).collect();
            let k_vals: Vec<f64> = idx.iter().map(|&i| analysis.k[i]).collect();
            let r_vals: Vec<f64> = idx.iter().map(|&i| analysis.r[i]).collect();

            let (e0_fit, e0_diag) = smooth_bspline_vec(&q, &e0, &e0_config);
            let (k_fit, k_diag) = smooth_bspline_vec(&q, &k_vals, &k_config);
            let (r_fit, r_diag) = smooth_bspline_vec(&q, &r_vals, &r_config);
            let dvdq_fit = local_poly_derivative(&q, &e0_fit, derivative_window, derivative_degree);

            for (local_i, &global_i) in idx.iter().enumerate() {
                e0_smooth[global_i] = e0_fit[local_i];
                k_smooth[global_i] = k_fit[local_i];
                r_smooth[global_i] = r_fit[local_i];
                dvdq[global_i] = dvdq_fit[local_i];
                dqdv[global_i] = 1.0 / dvdq_fit[local_i];
                row_group_key[global_i] = key.clone();
            }

            group_dtos.push(SmoothingGroupDto {
                key: key.clone(),
                label: labels[key].clone(),
                n: idx.len(),
                e0: spline_diagnostics_dto(&e0_diag),
                k: spline_diagnostics_dto(&k_diag),
                r: spline_diagnostics_dto(&r_diag),
            });
        }

        let dto = StageBResultDto {
            e0_smooth,
            k_smooth,
            r_smooth,
            d_v_d_q: dvdq,
            d_q_d_v: dqdv,
            row_group_key,
            groups: group_dtos,
        };
        Ok(serde_json::to_string(&dto).unwrap_or_else(|_| "{}".to_string()))
    }

    /// §10.3 setup: samples rests, builds the candidate `(t_min, t_max)`
    /// grid, and caches both for `scoreOptimalWindowChunk`. Requires
    /// `runSegmentation`/`runStageA` to have cached `self.segmentation`
    /// first.
    #[wasm_bindgen(js_name = setupOptimalWindow)]
    pub fn setup_optimal_window(&mut self, n: usize, l_min: f64, t_min_lower_bound: f64) -> Result<String, JsValue> {
        let seg = self
            .segmentation
            .as_ref()
            .ok_or_else(|| js_error("no segmentation available -- map columns and run segmentation first"))?;

        let candidates = optimal_window::rest_candidates(seg);
        let sample_idx = optimal_window::sample_rests(&candidates, n);
        let sample: Vec<RestCandidate> = sample_idx.iter().map(|&i| candidates[i].clone()).collect();
        let (t_max_observed, heterogeneous_lengths) = optimal_window::observed_t_max(&sample);
        let grid = optimal_window::candidate_grid(
            t_max_observed.floor() as i64,
            &GridConfig {
                t_min_lower_bound,
                l_min,
            },
        );

        let dto = SetupOptimalWindowDto {
            sampled_rests: sample
                .iter()
                .map(|c| SampledRestDto {
                    group_id: c.group_id,
                    rest_id: c.rest_id,
                    state: c.state.as_str(),
                    q: c.q,
                })
                .collect(),
            t_max_observed,
            heterogeneous_lengths,
            total_candidates: grid.len(),
        };

        self.optimal_window_sample = Some(sample);
        self.optimal_window_grid = Some(grid);
        Ok(serde_json::to_string(&dto).unwrap_or_else(|_| "{}".to_string()))
    }

    /// §10.3 scoring, one chunk of the cached candidate grid at a time --
    /// stateless per call (only reads the cache `setupOptimalWindow`
    /// filled in), which is what lets the worker call this repeatedly with
    /// yields between calls for cancellation (§2.1-style worker-owned
    /// computation, chunked here specifically for that purpose).
    #[wasm_bindgen(js_name = scoreOptimalWindowChunk)]
    pub fn score_optimal_window_chunk(&self, start: usize, count: usize, edge_points: usize) -> Result<String, JsValue> {
        let sample = self
            .optimal_window_sample
            .as_ref()
            .ok_or_else(|| js_error("call setupOptimalWindow first"))?;
        let grid = self
            .optimal_window_grid
            .as_ref()
            .ok_or_else(|| js_error("call setupOptimalWindow first"))?;

        let start = start.min(grid.len());
        let end = (start + count).min(grid.len());
        let scores: Vec<CandidateScoreDto> = grid[start..end]
            .iter()
            .map(|&window| {
                let s = optimal_window::score_candidate(sample, window, edge_points);
                CandidateScoreDto {
                    t_min: s.t_min,
                    t_max: s.t_max,
                    mean_adj_r2: s.mean_adj_r2,
                    median_adj_r2: s.median_adj_r2,
                    n_valid: s.n_valid,
                    n_sampled: s.n_sampled,
                    median_n_pts: s.median_n_pts,
                    median_edge_max_z: s.median_edge_max_z,
                    rejected: s.rejected,
                }
            })
            .collect();
        Ok(serde_json::to_string(&scores).unwrap_or_else(|_| "[]".to_string()))
    }

    /// The raw `(step_t, voltage, current)` points for one rest's active
    /// segment and rest segment (§7.2: they share `rest_id`, so one lookup
    /// gets both) -- ~50 points total, exactly what §2.1 says to ship for
    /// the live preview.
    #[wasm_bindgen(js_name = restPointsJson)]
    pub fn rest_points_json(&self, group_id: u32, rest_id: u32) -> String {
        let Some(seg) = &self.segmentation else {
            return "null".to_string();
        };
        let mut dto = RestPointsDto {
            active_step_t: Vec::new(),
            active_voltage: Vec::new(),
            active_current: Vec::new(),
            rest_step_t: Vec::new(),
            rest_voltage: Vec::new(),
            rest_current: Vec::new(),
        };
        for i in 0..seg.state.len() {
            if seg.group_id[i] != group_id || seg.rest[i] != rest_id {
                continue;
            }
            if seg.state[i] == State::Rest {
                dto.rest_step_t.push(seg.step_t[i]);
                dto.rest_voltage.push(seg.voltage[i]);
                dto.rest_current.push(seg.current[i]);
            } else {
                dto.active_step_t.push(seg.step_t[i]);
                dto.active_voltage.push(seg.voltage[i]);
                dto.active_current.push(seg.current[i]);
            }
        }
        serde_json::to_string(&dto).unwrap_or_else(|_| "null".to_string())
    }

    /// LTTB-decimated `(t, E, I)` series within `[x_min, x_max]` for Plot 1
    /// (§11.1), re-called on zoom so detail returns. Reads the independently
    /// -cached, already-scaled `raw_series` (see its own doc comment) --
    /// *not* `segmentation`'s arrays, which can have a large fraction of the
    /// file's rows missing once `IciDetectionConfig` is enabled. Plot 1's
    /// whole point is to still show that excluded data (shaded, via the
    /// rest boundaries it separately gets), not silently drop it.
    #[wasm_bindgen(js_name = decimatedSeriesJson)]
    pub fn decimated_series_json(&self, x_min: f64, x_max: f64, target_points: usize) -> String {
        let Some((raw_t, raw_e, raw_i)) = &self.raw_series else {
            return "null".to_string();
        };
        let idx_in_range: Vec<usize> = (0..raw_t.len())
            .filter(|&i| raw_t[i] >= x_min && raw_t[i] <= x_max)
            .collect();
        let t: Vec<f64> = idx_in_range.iter().map(|&i| raw_t[i]).collect();
        let e: Vec<f64> = idx_in_range.iter().map(|&i| raw_e[i]).collect();
        let i_series: Vec<f64> = idx_in_range.iter().map(|&i| raw_i[i]).collect();

        let selected = lttb(&t, &e, target_points);
        let dto = DecimatedSeriesDto {
            t: selected.iter().map(|&k| t[k]).collect(),
            e: selected.iter().map(|&k| e[k]).collect(),
            i: selected.iter().map(|&k| i_series[k]).collect(),
        };
        serde_json::to_string(&dto).unwrap_or_else(|_| "null".to_string())
    }
}

fn parse_anchor(s: &str) -> Result<AnchorPoint, JsValue> {
    match s {
        "start" => Ok(AnchorPoint::Start),
        "end" => Ok(AnchorPoint::End),
        other => Err(js_error(format!("unknown Q anchor '{other}' (expected 'start' or 'end')"))),
    }
}

fn column_as_strings(col: &Column) -> Vec<String> {
    if col.is_numeric {
        col.values
            .iter()
            .map(|v| {
                if !v.is_finite() {
                    "NaN".to_string()
                } else if v.fract() == 0.0 && v.abs() < 1e15 {
                    format!("{}", *v as i64)
                } else {
                    format!("{v}")
                }
            })
            .collect()
    } else {
        col.strings.clone()
    }
}

/// §7.1: for each raw `group_id`, if the segmented (retained) data for that
/// group has no `Rest` rows or no active (charge/discharge) rows -- i.e.
/// the current `state_threshold` misclassified the group entirely -- run
/// `split_current_levels()` over that group's *raw* (pre-drop) current and
/// suggest its midpoint threshold, but only when the two current levels are
/// clearly separated (otherwise there's nothing useful to suggest).
fn threshold_suggestions(
    raw_group_id: &[u32],
    raw_current: &[f64],
    seg: &SegmentedData,
) -> Vec<ThresholdSuggestionDto> {
    let mut has_rest: HashMap<u32, bool> = HashMap::new();
    let mut has_active: HashMap<u32, bool> = HashMap::new();
    for &g in raw_group_id {
        has_rest.entry(g).or_insert(false);
        has_active.entry(g).or_insert(false);
    }
    for i in 0..seg.state.len() {
        let entry = if seg.state[i] == State::Rest {
            has_rest.get_mut(&seg.group_id[i])
        } else {
            has_active.get_mut(&seg.group_id[i])
        };
        if let Some(flag) = entry {
            *flag = true;
        }
    }

    let mut groups: Vec<u32> = has_rest.keys().copied().collect();
    groups.sort_unstable();

    let mut out = Vec::new();
    for g in groups {
        let needs_suggestion = !has_rest.get(&g).copied().unwrap_or(false)
            || !has_active.get(&g).copied().unwrap_or(false);
        if !needs_suggestion {
            continue;
        }
        let group_current: Vec<f64> = raw_group_id
            .iter()
            .zip(raw_current.iter())
            .filter(|(&gid, _)| gid == g)
            .map(|(_, &c)| c)
            .collect();
        if let Some(levels) = split_current_levels(&group_current) {
            if levels.clearly_separated {
                out.push(ThresholdSuggestionDto {
                    group_id: g,
                    suggested_threshold: levels.suggested_threshold,
                });
            }
        }
    }
    out
}

fn segmentation_summary_dto(
    seg: &SegmentedData,
    log: &SegmentLog,
    threshold_suggestions: Vec<ThresholdSuggestionDto>,
) -> SegmentationSummaryDto {
    let n = seg.state.len();
    let mut order: Vec<(u32, u32)> = Vec::new();
    let mut groups: HashMap<(u32, u32), Vec<usize>> = HashMap::new();
    for i in 0..n {
        if seg.state[i] != State::Rest {
            continue;
        }
        let key = (seg.group_id[i], seg.rest[i]);
        groups
            .entry(key)
            .or_insert_with(|| {
                order.push(key);
                Vec::new()
            })
            .push(i);
    }

    let mut boundaries = Vec::with_capacity(order.len());
    for (index, &key) in order.iter().enumerate() {
        let idx = &groups[&key];
        let mut rest_step_ts: Vec<f64> = idx.iter().map(|&i| seg.step_t[i]).collect();
        rest_step_ts.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let t_start = idx.iter().map(|&i| seg.t[i]).fold(f64::INFINITY, f64::min);
        let t_end = idx
            .iter()
            .map(|&i| seg.t[i])
            .fold(f64::NEG_INFINITY, f64::max);
        boundaries.push(RestBoundaryDto {
            index: index as u32,
            rest_id: key.1,
            group_id: key.0,
            t_start,
            t_end,
            rest_step_ts,
        });
    }

    SegmentationSummaryDto {
        total_rests: boundaries.len(),
        rest_boundaries: boundaries,
        reversal_rows_dropped: log.reversal_rows_dropped,
        incomplete_final_rows_dropped: log.incomplete_final_rows_dropped,
        leading_rest_rows_dropped: log.leading_rest_rows_dropped,
        threshold_suggestions,
        non_ici_rows_dropped: log.non_ici_rows_dropped,
    }
}

fn analysis_table_dto(t: &AnalysisTable) -> AnalysisTableDto {
    AnalysisTableDto {
        group_id: t.group_id.clone(),
        cyc_n: t.cyc_n.clone(),
        state: t.state.iter().map(|s| s.as_str()).collect(),
        rest: t.rest.clone(),
        t: t.t.clone(),
        step_t: t.step_t.clone(),
        e: t.e.clone(),
        i: t.i.clone(),
        q: t.q.clone(),
        e0: t.e0.clone(),
        e0_err: t.e0_err.clone(),
        s: t.s.clone(),
        s_err: t.s_err.clone(),
        i0: t.i0.clone(),
        n_pts: t.n_pts.clone(),
        r2: t.r2.clone(),
        adj_r2: t.adj_r2.clone(),
        rmse: t.rmse.clone(),
        edge_mae_ratio: t.edge_mae_ratio.clone(),
        edge_max_z: t.edge_max_z.clone(),
        r: t.r.clone(),
        r_err: t.r_err.clone(),
        k: t.k.clone(),
        k_err: t.k_err.clone(),
    }
}

fn nonphysical_report_dto(report: &ici_core::derive::NonphysicalReport) -> NonphysicalReportDto {
    NonphysicalReportDto {
        r_by_state: report.r_by_state.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
        k_by_state: report.k_by_state.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
    }
}

fn direction_used_str(d: DirectionUsed) -> &'static str {
    match d {
        DirectionUsed::Increasing => "increasing",
        DirectionUsed::Decreasing => "decreasing",
        DirectionUsed::NotApplicable => "notApplicable",
    }
}

fn spline_diagnostics_dto(diag: &Option<SplineDiagnostics>) -> Option<SplineDiagnosticsDto> {
    diag.as_ref().map(|d| SplineDiagnosticsDto {
        direction_used: direction_used_str(d.direction_used),
        k_effective: d.k_effective,
        lambda: d.lambda,
        edf: d.edf,
    })
}

fn parse_direction(s: &str) -> Result<Direction, JsValue> {
    match s {
        "automatic" => Ok(Direction::Automatic),
        "increasing" => Ok(Direction::Increasing),
        "decreasing" => Ok(Direction::Decreasing),
        other => Err(js_error(format!("unknown smoothing direction '{other}'"))),
    }
}

fn spline_config_from_dto(dto: &SmootherConfigDto) -> Result<SplineConfig, JsValue> {
    Ok(SplineConfig {
        monotonic: dto.monotonic,
        direction: parse_direction(&dto.direction)?,
        k: dto.k,
        m: dto.m,
    })
}

/// The live single-rest preview fit (§10.2), called from the second,
/// always-resident main-thread WASM instance (§2.1) -- NOT the worker.
/// Takes the raw points for one rest's active+rest segments (shipped by the
/// worker via `restPointsJson`) and does its own summary + regression, so
/// the worker never needs to run the full per-rest summary computation
/// eagerly for every rest just to support this.
#[wasm_bindgen(js_name = fitRestPreview)]
#[allow(clippy::too_many_arguments)]
pub fn fit_rest_preview(
    active_step_t: Vec<f64>,
    active_voltage: Vec<f64>,
    active_current: Vec<f64>,
    rest_step_t: Vec<f64>,
    rest_voltage: Vec<f64>,
    rest_current: Vec<f64>,
    t_min: f64,
    t_max: f64,
    edge_points: usize,
    voltage_interp_window: Option<f64>,
    current_avg_window: Option<f64>,
) -> String {
    let e = interpolate_endpoint(&active_step_t, &active_voltage, voltage_interp_window);
    let i = mean_or_nan(&select_final_window(
        &active_step_t,
        &active_current,
        current_avg_window,
    ));
    let i0 = mean_or_nan(&select_initial_window(
        &rest_step_t,
        &rest_current,
        current_avg_window,
    ));

    let n_points_in_window = rest_step_t
        .iter()
        .filter(|&&t| t.is_finite() && t >= t_min && t <= t_max)
        .count();

    let dto = match fit_rest_window(&rest_step_t, &rest_voltage, (t_min, t_max), edge_points) {
        Ok(fit) => {
            let delta_i = i - i0;
            RestPreviewDto {
                ok: true,
                n_points_in_window,
                e: Some(e),
                i: Some(i),
                i0: Some(i0),
                e0: Some(fit.e0),
                e0_err: Some(fit.e0_err),
                s: Some(fit.s),
                s_err: Some(fit.s_err),
                n_pts: Some(fit.n_pts),
                r2: Some(fit.r2),
                adj_r2: Some(fit.adj_r2),
                rmse: Some(fit.rmse),
                edge_mae_ratio: Some(fit.edge_mae_ratio),
                edge_max_z: Some(fit.edge_max_z),
                r: Some((e - fit.e0) / delta_i),
                r_err: Some(fit.e0_err / delta_i.abs()),
                k: Some(-fit.s / delta_i),
                k_err: Some(fit.s_err / delta_i.abs()),
                error: None,
            }
        }
        Err(err) => RestPreviewDto {
            ok: false,
            n_points_in_window,
            e: Some(e),
            i: Some(i),
            i0: Some(i0),
            e0: None,
            e0_err: None,
            s: None,
            s_err: None,
            n_pts: None,
            r2: None,
            adj_r2: None,
            rmse: None,
            edge_mae_ratio: None,
            edge_max_z: None,
            r: None,
            r_err: None,
            k: None,
            k_err: None,
            error: Some(match err {
                RestFitError::TooFewPoints { n } => {
                    format!("only {n} usable point(s) in window (need >= 3)")
                }
                RestFitError::Degenerate => {
                    "degenerate regression (zero variance in sqrt(step.t))".to_string()
                }
            }),
        },
    };
    serde_json::to_string(&dto).unwrap_or_else(|_| "{}".to_string())
}

fn mean_or_nan(values: &[f64]) -> f64 {
    if values.is_empty() {
        f64::NAN
    } else {
        values.iter().sum::<f64>() / values.len() as f64
    }
}

/// Smoke-test export retained from milestone 1's scaffolding check.
#[wasm_bindgen]
pub fn core_version() -> String {
    ici_core::placeholder_version().to_string()
}
