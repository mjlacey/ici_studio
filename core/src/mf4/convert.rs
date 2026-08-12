//! Turns a parsed MDF4 file into the exact same [`crate::parse::ParsedTable`]
//! shape the TSV/CSV path produces, so column mapping, segmentation, and
//! everything after it stays unaware of which parser actually ran.
//!
//! A real instrument export commonly puts each logged signal in its own
//! channel group (`(Time, ClimaTemp)`, `(Time, I)`, `(Time, U)`, ...) rather
//! than bundling them into one group with a shared master -- verified
//! against a real "lab-discover" export, not assumed. The strategy here:
//! pick the group with the most samples as the "primary" one (its master
//! becomes the table's `t`-like column), then fold in every other group
//! whose own sample count *and* first/last master timestamp agree with the
//! primary's closely enough to trust row-index alignment. A group that
//! doesn't line up is left out of the table rather than guessed at, with a
//! warning naming it -- the same "don't silently drop or misalign data"
//! stance `core::parse` already takes for ragged rows.

use crate::mf4::api::channel::Channel;
use crate::mf4::api::channel_group::ChannelGroup;
use crate::mf4::api::mdf::MDF;
use crate::mf4::parsing::decoder::DecodedValue;
use crate::parse::{Column, DecimalSeparator, Delimiter, Encoding, ParseReport, ParsedTable};

/// MDF4's own channel-type code for a master (time) channel.
const CHANNEL_TYPE_MASTER: u8 = 2;
/// How much a candidate group's own master can disagree with the primary
/// group's master (at matching sample count) before the group is treated as
/// incompatible rather than row-aligned. Generous on purpose: this is a
/// sanity check against "these aren't actually the same acquisition", not a
/// precision requirement -- MDF4 timestamps are already seconds as f64.
const TIMESTAMP_AGREEMENT_TOLERANCE_S: f64 = 1e-3;

#[derive(Debug)]
pub struct Mf4ConvertError(pub String);

impl std::fmt::Display for Mf4ConvertError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

fn mdf_err(e: crate::mf4::error::MdfError) -> Mf4ConvertError {
    Mf4ConvertError(e.to_string())
}

pub fn convert_to_table(bytes: &[u8]) -> Result<ParsedTable, Mf4ConvertError> {
    let mdf = MDF::from_bytes(bytes.to_vec()).map_err(mdf_err)?;
    let groups = mdf.channel_groups();
    if groups.is_empty() {
        return Err(Mf4ConvertError("MDF4 file has no channel groups".to_string()));
    }

    let mut warnings = Vec::new();

    let primary_index = groups
        .iter()
        .enumerate()
        .max_by_key(|(_, g)| g.raw_channel_group().block.cycles_nr)
        .map(|(i, _)| i)
        .expect("groups is non-empty by construction");
    let primary = &groups[primary_index];
    let primary_cycles = primary.raw_channel_group().block.cycles_nr;

    let (primary_master, primary_master_values) = master_channel_values(primary)?;

    let mut columns: Vec<Column> = Vec::new();
    if let Some(master) = &primary_master {
        let name = channel_display_name(master, "Time")?;
        columns.push(Column {
            name,
            is_numeric: true,
            values: primary_master_values.clone(),
            strings: Vec::new(),
        });
    } else {
        warnings.push(format!(
            "The largest channel group ({} sample(s)) has no master/time channel; \
             the resulting table has no time column until one is mapped manually.",
            primary_cycles
        ));
    }

    for channel in primary.channels() {
        if channel.block().channel_type == CHANNEL_TYPE_MASTER {
            continue; // already emitted above
        }
        columns.push(channel_to_column(&channel)?);
    }

    for (i, group) in groups.iter().enumerate() {
        if i == primary_index {
            continue;
        }
        let cycles = group.raw_channel_group().block.cycles_nr;
        let group_label = group_display_name(group, i);

        if cycles != primary_cycles {
            warnings.push(format!(
                "Skipped channel group {group_label} ({cycles} sample(s), primary group has \
                 {primary_cycles}): sample counts don't match, so its rows can't be \
                 aligned with the rest of the table."
            ));
            continue;
        }

        let (_, master_values) = master_channel_values(group)?;
        if !timestamps_agree(&primary_master_values, &master_values) {
            warnings.push(format!(
                "Skipped channel group {group_label}: its master/time channel doesn't \
                 match the primary group's, so its rows can't be trusted to align \
                 with the rest of the table."
            ));
            continue;
        }

        for channel in group.channels() {
            if channel.block().channel_type == CHANNEL_TYPE_MASTER {
                continue;
            }
            columns.push(channel_to_column(&channel)?);
        }
    }

    dedupe_column_names(&mut columns);

    let n_rows = columns.iter().map(|c| c.values.len().max(c.strings.len())).max().unwrap_or(0);
    let n_columns = columns.len();

    let report = ParseReport {
        // These three describe text-file sniffing decisions that don't
        // apply to a binary MDF4 source; kept at harmless defaults rather
        // than adding MDF4-specific enum variants purely for display --
        // the Parse Card hides them for an MDF4-sourced dataset instead
        // (see ParseReport::is_mdf4 below).
        encoding: Encoding::Utf8,
        delimiter: Delimiter::Comma,
        decimal_separator: DecimalSeparator::Dot,
        preamble_lines_skipped: 0,
        header_present: true,
        header_synthesized: false,
        n_rows,
        n_columns,
        trailing_column_dropped: false,
        ragged_rows_dropped: 0,
        ragged_row_line_numbers: Vec::new(),
        coercion_failures: Vec::new(),
        warnings,
        source: crate::parse::SourceFormat::Mdf4,
    };

    Ok(ParsedTable { columns, report })
}

fn master_channel_values<'a>(group: &ChannelGroup<'a>) -> Result<(Option<Channel<'a>>, Vec<f64>), Mf4ConvertError> {
    for channel in group.channels() {
        if channel.block().channel_type == CHANNEL_TYPE_MASTER {
            let values = channel.values_as_f64().map_err(mdf_err)?;
            return Ok((Some(channel), values));
        }
    }
    Ok((None, Vec::new()))
}

fn timestamps_agree(a: &[f64], b: &[f64]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let close = |x: f64, y: f64| (x - y).abs() <= TIMESTAMP_AGREEMENT_TOLERANCE_S;
    match (a.first(), b.first(), a.last(), b.last()) {
        (Some(&fa), Some(&fb), Some(&la), Some(&lb)) => close(fa, fb) && close(la, lb),
        // Both empty counts as "agreeing" (nothing to disagree about);
        // a.len() == b.len() was already checked above.
        (None, None, None, None) => true,
        _ => false,
    }
}

fn channel_to_column(channel: &Channel) -> Result<Column, Mf4ConvertError> {
    let name = channel_display_name(channel, "channel")?;
    let decoded = channel.values().map_err(mdf_err)?;

    // Majority-numeric classification, mirroring core::parse's own
    // is_numeric rule so a text/enum channel becomes a grouping-only
    // column instead of silently NaN-filling every row.
    let (numeric_count, total) = decoded.iter().flatten().fold((0usize, 0usize), |(num, tot), v| {
        let is_num = matches!(v, DecodedValue::Float(_) | DecodedValue::UnsignedInteger(_) | DecodedValue::SignedInteger(_));
        (num + is_num as usize, tot + 1)
    });
    let is_numeric = total == 0 || numeric_count * 2 >= total;

    if is_numeric {
        let values = decoded
            .iter()
            .map(|v| match v {
                Some(DecodedValue::Float(f)) => *f,
                Some(DecodedValue::UnsignedInteger(u)) => *u as f64,
                Some(DecodedValue::SignedInteger(i)) => *i as f64,
                _ => f64::NAN,
            })
            .collect();
        Ok(Column { name, is_numeric: true, values, strings: Vec::new() })
    } else {
        let strings = decoded
            .iter()
            .map(|v| match v {
                Some(DecodedValue::String(s)) => s.clone(),
                Some(other) => format!("{other:?}"),
                None => String::new(),
            })
            .collect();
        Ok(Column { name, is_numeric: false, values: Vec::new(), strings })
    }
}

fn channel_display_name(channel: &Channel, fallback: &str) -> Result<String, Mf4ConvertError> {
    Ok(channel.name().map_err(mdf_err)?.filter(|s| !s.is_empty()).unwrap_or_else(|| fallback.to_string()))
}

fn group_display_name(group: &ChannelGroup, index: usize) -> String {
    match group.name().ok().flatten() {
        Some(name) if !name.is_empty() => format!("'{name}' (#{index})"),
        _ => format!("#{index}"),
    }
}

/// Suffixes a repeated column name as `"name (2)"`, `"name (3)"`, ... --
/// same convention `core::parse::dedupe_header` uses for a text file's
/// duplicate header cells, kept consistent since both feed the same
/// downstream column-mapping UI.
fn dedupe_column_names(columns: &mut [Column]) {
    let mut seen: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for column in columns.iter_mut() {
        let count = seen.entry(column.name.clone()).or_insert(0);
        *count += 1;
        if *count > 1 {
            column.name = format!("{} ({})", column.name, count);
        }
    }
}
