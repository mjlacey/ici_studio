use crate::mf4::blocks::common::BlockHeader;
use crate::mf4::blocks::common::BlockParse;
use crate::mf4::error::MdfError;

/// A decoded `##DT`/`##DV`/`##DZ` data fragment.
///
/// Always owns its bytes (unlike upstream mf4-rs, which borrows zero-copy
/// from a memory map for the uncompressed case): a `##DZ` fragment's bytes
/// don't exist anywhere in the file, only after inflating, so there's no
/// slice to borrow from for that case regardless. Making every variant
/// uniformly owned avoids threading a lifetime through every caller for the
/// sake of the (here, rare) uncompressed case -- these files are tens of MB
/// at most, so the extra copy is not worth the complexity.
#[derive(Debug)]
pub struct DataBlock {
    pub header: BlockHeader,
    pub data: Vec<u8>,
}

impl<'a> BlockParse<'a> for DataBlock {
    const ID: &'static str = "##DT";
    /// Parse a DTBLOCK or DVBLOCK from the given byte slice.
    ///
    /// Both `##DT` (record data) and `##DV` (sample data of a column-oriented
    /// group) blocks share the same layout: a 24-byte header followed by raw
    /// data. Any other block id is rejected -- `##DZ` (compressed) fragments
    /// go through `compressed_data_block::read_dz_block` instead, since they
    /// need inflating rather than a plain copy.
    fn from_bytes(bytes: &'a [u8]) -> Result<Self, MdfError> {
        let header = BlockHeader::from_bytes(bytes)?;
        if header.id != "##DT" && header.id != "##DV" {
            return Err(MdfError::BlockIDError {
                actual: header.id.clone(),
                expected: "##DT / ##DV".to_string(),
            });
        }

        let data_len = (header.block_len as usize).saturating_sub(24);
        let expected_bytes = 24 + data_len;
        if bytes.len() < expected_bytes {
            return Err(MdfError::TooShortBuffer {
                actual:   bytes.len(),
                expected: expected_bytes,
                file:     file!(),
                line:     line!(),
            });
        }
        let data = bytes[24..24 + data_len].to_vec();
        Ok(Self { header, data })
    }
}
