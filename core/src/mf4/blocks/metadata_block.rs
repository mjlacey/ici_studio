use crate::mf4::blocks::common::BlockHeader;
use crate::mf4::blocks::common::BlockParse;
use crate::mf4::error::MdfError;

#[derive(Debug)]
pub struct MetadataBlock {
    pub header: BlockHeader,
    pub xml: String,
}

impl BlockParse<'_> for MetadataBlock {
    const ID: &'static str = "##MD";
    fn from_bytes(bytes: &[u8]) -> Result<Self, MdfError> {

        let header = Self::parse_header(bytes)?;

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
        let data = &bytes[24..24 + data_len];

        let xml = String::from_utf8_lossy(data)
            .trim_matches('\0')
            .to_string();

        Ok(Self { header, xml })
    }
}
