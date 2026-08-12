use crate::mf4::blocks::conversion::base::ConversionBlock;
use crate::mf4::blocks::common::{BlockHeader, read_string_block, BlockParse};
use crate::mf4::error::MdfError;
use crate::mf4::parsing::decoder::DecodedValue;

/// Safely read a name text block referenced by `addr` from `file_data`.
///
/// Returns `Ok(None)` (instead of panicking or erroring) when the address is
/// missing, NIL, or outside the bounds of `file_data` — e.g. during index
/// reads where `file_data` is empty.
fn read_name_checked(addr: Option<u64>, file_data: &[u8]) -> Result<Option<String>, MdfError> {
    if let Some(a) = addr {
        let off = a as usize;
        if a != 0 && off.saturating_add(24) <= file_data.len() {
            return read_string_block(file_data, a);
        }
    }
    Ok(None)
}

pub fn apply_bitfield_text(block: &ConversionBlock, value: DecodedValue, file_data: &[u8]) -> Result<DecodedValue, MdfError> {
    let raw = match value {
        DecodedValue::UnsignedInteger(u) => u as u64,
        DecodedValue::SignedInteger(i) => i as u64,
        _ => return Ok(value),
    };

    let mut parts = Vec::new();
    let masks = &block.cc_val;
    let links = &block.cc_ref;

    for (i, &link_addr) in links.iter().enumerate() {
        if i >= masks.len() { break; }
        let mask = masks[i].to_bits();
        let masked = raw & mask;
        if link_addr == 0 { continue; }

        // First try to use resolved conversions if available
        if let Some(resolved_conversion) = block.get_resolved_conversion(i) {
            let decoded_masked = resolved_conversion.apply_decoded(DecodedValue::UnsignedInteger(masked), &[])?;
            if let DecodedValue::String(s) = decoded_masked {
                // The bit group's name is stored in resolved_texts at the same
                // index during dependency resolution; fall back to reading it
                // from file_data only when the address is within bounds (never
                // panic on index reads with empty file_data).
                let name = if let Some(resolved_name) = block.get_resolved_text(i) {
                    Some(resolved_name.clone())
                } else {
                    read_name_checked(resolved_conversion.cc_tx_name, file_data)?
                };
                let part = match name {
                    Some(name_text) => format!("{} = {}", name_text, s),
                    None => s,
                };
                parts.push(part);
            }
            continue;
        }

        // A bit group whose cc_ref points directly at a ##TX contributes its
        // resolved text (asammdf appends the TX bytes).
        if let Some(text) = block.get_resolved_text(i) {
            parts.push(text.clone());
            continue;
        }

        // Fallback to legacy behavior if no resolved data (for backward compatibility)
        // Note: This should rarely be used now that we have deep resolution
        let off = link_addr as usize;
        if off + 24 > file_data.len() {
            // If we can't access the data, try default conversion as last resort
            if let Some(default_conversion) = block.get_default_conversion() {
                let decoded_masked = default_conversion.apply_decoded(DecodedValue::UnsignedInteger(masked), &[])?;
                if let DecodedValue::String(s) = decoded_masked {
                    parts.push(s);
                }
            }
            continue;
        }

        let hdr = BlockHeader::from_bytes(&file_data[off..off+24])?;
        if hdr.id == "##TX" {
            // Direct ##TX reference: append the text bytes (asammdf behavior).
            if let Some(text) = read_string_block(file_data, link_addr)? {
                parts.push(text);
            }
            continue;
        }
        if &hdr.id != "##CC" { continue; }

        // Create nested conversion but don't do deep resolution to avoid double work
        // since this is fallback code that should rarely execute
        let mut nested = ConversionBlock::from_bytes(&file_data[off..])?;
        let _ = nested.resolve_formula(file_data);
        let decoded_masked = nested.apply_decoded(DecodedValue::UnsignedInteger(masked), file_data)?;
        if let DecodedValue::String(s) = decoded_masked {
            let part = match read_name_checked(nested.cc_tx_name, file_data)? {
                Some(name) => format!("{} = {}", name, s),
                None => s,
            };
            parts.push(part);
        }
    }

    let out = parts.join("|");
    Ok(DecodedValue::String(out))
}
