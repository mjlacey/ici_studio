use crate::mf4::blocks::common::BlockHeader;
use crate::mf4::blocks::common::BlockParse;
use crate::mf4::error::MdfError;

/// HLBLOCK: History List Block -- a thin wrapper in front of a `##DL` chain,
/// present when the referenced data list's fragments are `##DZ` (zipped)
/// rather than plain `##DT`/`##DV`.
///
/// Resolving past this wrapper to `dl_first` is the entire fix needed in
/// `raw_data_group.rs`'s data-block walk -- see this module's parent
/// (`core::mf4`) doc comment for why upstream mf4-rs doesn't handle it.
/// `zip_type` isn't actually load-bearing for reading: each `##DZ` fragment
/// carries its own zip-type byte, which is what `compressed_data_block`
/// reads.
pub struct HistoryListBlock {
    pub header: BlockHeader,
    pub dl_first: u64,
    pub flags: u16,
    pub zip_type: u8,
}

impl BlockParse<'_> for HistoryListBlock {
    const ID: &'static str = "##HL";

    fn from_bytes(bytes: &[u8]) -> Result<Self, MdfError> {
        let header = Self::parse_header(bytes)?;
        let expected_bytes = 24 + 8 + 2 + 1;
        if bytes.len() < expected_bytes {
            return Err(MdfError::TooShortBuffer {
                actual:   bytes.len(),
                expected: expected_bytes,
                file:     file!(),
                line:     line!(),
            });
        }
        let dl_first = u64::from_le_bytes(bytes[24..32].try_into().unwrap());
        let flags = u16::from_le_bytes(bytes[32..34].try_into().unwrap());
        let zip_type = bytes[34];
        Ok(HistoryListBlock { header, dl_first, flags, zip_type })
    }
}
