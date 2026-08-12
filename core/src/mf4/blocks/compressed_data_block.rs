use crate::mf4::blocks::common::BlockHeader;
use crate::mf4::blocks::data_block::DataBlock;
use crate::mf4::error::MdfError;

/// DZBLOCK: a zipped data fragment. Layout (little-endian, after the
/// 24-byte common header):
///
/// | field               | bytes | meaning                                    |
/// |---------------------|-------|--------------------------------------------|
/// | dz_org_block_type   | 2     | ASCII id of the block this stands in for ("DT"/"DV"/"SD"/"RD") |
/// | dz_zip_type         | 1     | 0 = deflate, 1 = transpose + deflate        |
/// | dz_reserved         | 1     |                                              |
/// | dz_zip_parameter    | 4     | transpose row length in bytes (zip_type 1 only) |
/// | dz_org_data_length  | 8     | uncompressed length                         |
/// | dz_data_length      | 8     | compressed length (bytes that follow)       |
///
/// Not part of the original mf4-rs crate -- see `core::mf4`'s module doc
/// comment for why this exists as a local addition rather than upstream.
fn read_dz_header(bytes: &[u8]) -> Result<(u8, u32, u64, u64), MdfError> {
    let header = BlockHeader::from_bytes(bytes)?;
    if header.id != "##DZ" {
        return Err(MdfError::BlockIDError {
            actual: header.id.clone(),
            expected: "##DZ".to_string(),
        });
    }
    let expected_bytes = 24 + 2 + 1 + 1 + 4 + 8 + 8;
    if bytes.len() < expected_bytes {
        return Err(MdfError::TooShortBuffer {
            actual:   bytes.len(),
            expected: expected_bytes,
            file:     file!(),
            line:     line!(),
        });
    }
    let zip_type = bytes[26];
    let zip_parameter = u32::from_le_bytes(bytes[28..32].try_into().unwrap());
    let org_data_length = u64::from_le_bytes(bytes[32..40].try_into().unwrap());
    let data_length = u64::from_le_bytes(bytes[40..48].try_into().unwrap());
    Ok((zip_type, zip_parameter, org_data_length, data_length))
}

/// Parse and fully decode a `##DZ` fragment into a plain [`DataBlock`] (as if
/// it had been an uncompressed `##DT`/`##DV`/`##SD` fragment all along) --
/// the caller doesn't need to know or care that this one happened to be
/// zipped.
pub fn read_dz_block(bytes: &[u8]) -> Result<DataBlock, MdfError> {
    let header = BlockHeader::from_bytes(bytes)?;
    let (zip_type, zip_parameter, org_data_length, data_length) = read_dz_header(bytes)?;

    let payload_start = 48usize;
    let payload_end = payload_start.saturating_add(data_length as usize);
    let compressed = bytes.get(payload_start..payload_end).ok_or(MdfError::TooShortBuffer {
        actual:   bytes.len(),
        expected: payload_end,
        file:     file!(),
        line:     line!(),
    })?;

    let inflated = miniz_oxide::inflate::decompress_to_vec_zlib(compressed).map_err(|e| {
        MdfError::BlockSerializationError(format!("##DZ zlib inflate failed: {e:?}"))
    })?;
    if inflated.len() as u64 != org_data_length {
        return Err(MdfError::BlockSerializationError(format!(
            "##DZ inflated to {} byte(s), expected dz_org_data_length={}",
            inflated.len(),
            org_data_length
        )));
    }

    let data = match zip_type {
        0 => inflated,
        1 => untranspose(&inflated, zip_parameter as usize),
        other => {
            return Err(MdfError::BlockSerializationError(format!(
                "##DZ has unsupported dz_zip_type {other} (expected 0=deflate or 1=transpose+deflate)"
            )))
        }
    };

    Ok(DataBlock { header, data })
}

/// Reverses MDF4's DZBLOCK "transpose" pre-compression step.
///
/// The writer rearranges `row_len`-byte rows from row-major
/// (`row0[0], row0[1], ..., row0[row_len-1], row1[0], ...`) into column-major
/// (`row0[0], row1[0], row2[0], ..., row0[1], row1[1], ...`) before deflating
/// -- neighbouring bytes at the same offset within a record tend to be far
/// more similar across records than neighbouring bytes *within* one record
/// (e.g. every record's IEEE-754 high byte for a slowly-changing channel),
/// which compresses better. This undoes exactly that rearrangement. Any
/// trailing bytes that don't fill a complete `row_len`-byte row are left
/// untransposed at the end, per spec.
fn untranspose(data: &[u8], row_len: usize) -> Vec<u8> {
    if row_len == 0 {
        return data.to_vec();
    }
    let num_rows = data.len() / row_len;
    let transposed_len = num_rows * row_len;
    let mut out = vec![0u8; data.len()];
    for col in 0..row_len {
        for row in 0..num_rows {
            out[row * row_len + col] = data[col * num_rows + row];
        }
    }
    if transposed_len < data.len() {
        out[transposed_len..].copy_from_slice(&data[transposed_len..]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn untranspose_reverses_a_hand_built_transpose() {
        // Two 3-byte rows: [1,2,3] and [4,5,6]. Transposed (column-major):
        // col0 = [1,4], col1 = [2,5], col2 = [3,6] -> [1,4,2,5,3,6].
        let transposed = [1u8, 4, 2, 5, 3, 6];
        let restored = untranspose(&transposed, 3);
        assert_eq!(restored, vec![1, 2, 3, 4, 5, 6]);
    }

    #[test]
    fn untranspose_leaves_a_trailing_partial_row_untouched() {
        // Two full 2-byte rows [1,2],[3,4] transposed to [1,3,2,4], plus one
        // trailing byte (5) that doesn't fill a row -- stored as-is per spec.
        let transposed = [1u8, 3, 2, 4, 5];
        let restored = untranspose(&transposed, 2);
        assert_eq!(restored, vec![1, 2, 3, 4, 5]);
    }

    #[test]
    fn untranspose_row_len_zero_is_identity() {
        let data = [9u8, 8, 7];
        assert_eq!(untranspose(&data, 0), vec![9, 8, 7]);
    }

    #[test]
    fn read_dz_block_roundtrips_plain_deflate() {
        let payload = b"hello ici world, this is a test payload for deflate";
        let compressed = miniz_oxide::deflate::compress_to_vec_zlib(payload, 6);

        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"##DZ");
        bytes.extend_from_slice(&[0u8; 4]); // reserved0
        let block_len = (48 + compressed.len()) as u64;
        bytes.extend_from_slice(&block_len.to_le_bytes());
        bytes.extend_from_slice(&1u64.to_le_bytes()); // links_nr
        bytes.extend_from_slice(b"DT"); // dz_org_block_type
        bytes.push(0); // dz_zip_type = plain deflate
        bytes.push(0); // dz_reserved
        bytes.extend_from_slice(&0u32.to_le_bytes()); // dz_zip_parameter (unused for type 0)
        bytes.extend_from_slice(&(payload.len() as u64).to_le_bytes()); // dz_org_data_length
        bytes.extend_from_slice(&(compressed.len() as u64).to_le_bytes()); // dz_data_length
        bytes.extend_from_slice(&compressed);

        let block = read_dz_block(&bytes).unwrap();
        assert_eq!(block.data, payload);
    }

    #[test]
    fn read_dz_block_roundtrips_transpose_deflate() {
        // 4 records of 3 bytes each, row-major.
        let row_len = 3usize;
        let rows: [[u8; 3]; 4] = [[1, 2, 3], [4, 5, 6], [7, 8, 9], [10, 11, 12]];
        let original: Vec<u8> = rows.iter().flatten().copied().collect();

        // Build the transposed (column-major) form the writer would have deflated.
        let num_rows = original.len() / row_len;
        let mut transposed = vec![0u8; original.len()];
        for col in 0..row_len {
            for row in 0..num_rows {
                transposed[col * num_rows + row] = original[row * row_len + col];
            }
        }
        let compressed = miniz_oxide::deflate::compress_to_vec_zlib(&transposed, 6);

        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"##DZ");
        bytes.extend_from_slice(&[0u8; 4]);
        let block_len = (48 + compressed.len()) as u64;
        bytes.extend_from_slice(&block_len.to_le_bytes());
        bytes.extend_from_slice(&1u64.to_le_bytes());
        bytes.extend_from_slice(b"DT");
        bytes.push(1); // dz_zip_type = transpose + deflate
        bytes.push(0);
        bytes.extend_from_slice(&(row_len as u32).to_le_bytes());
        bytes.extend_from_slice(&(transposed.len() as u64).to_le_bytes());
        bytes.extend_from_slice(&(compressed.len() as u64).to_le_bytes());
        bytes.extend_from_slice(&compressed);

        let block = read_dz_block(&bytes).unwrap();
        assert_eq!(block.data, original);
    }
}
