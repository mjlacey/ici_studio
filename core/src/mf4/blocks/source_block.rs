use byteorder::{ByteOrder, LittleEndian};
use crate::mf4::blocks::common::BlockHeader;
use crate::mf4::error::MdfError;
use crate::mf4::blocks::common::BlockParse;

/// Represents an SIBLOCK (“##SI”) from the MDF4 file.
///
/// - Links:
///   • si_tx_name    LINK → TXBLOCK (source name)  
///   • si_tx_path    LINK → TXBLOCK (tool-specific path)  
///   • si_md_comment LINK → TXBLOCK/MDBLOCK (additional XML/text)
/// - Data:
///   • si_type      UINT8 (0=OTHER,1=ECU,2=BUS,3=I/O,4=TOOL,5=USER)  
///   • si_bus_type  UINT8 (0=NONE,1=OTHER,2=CAN,3=LIN,…,8=USB)  
///   • si_flags     UINT8 (bit 0 = simulated)  
///   • si_reserved  BYTE[5] (padding)
#[derive(Debug)]
pub struct SourceBlock {
    pub header:        BlockHeader,
    /// Link to a TXBLOCK containing the human-readable source name
    pub name_addr:     u64,
    /// Link to a TXBLOCK containing a tool-specific path/namespace
    pub path_addr:     u64,
    /// Link to TXBLOCK or MDBLOCK with extended comment/XML
    pub comment_addr:  u64,

    pub source_type:   u8,
    pub bus_type:      u8,
    pub flags:         u8,
    // 5 bytes reserved for 8-byte alignment
}

impl BlockParse<'_> for SourceBlock {
    const ID: &'static str = "##SI";
    /// Parse an SIBLOCK from its raw bytes (starting at the “##SI…” header).
    fn from_bytes(bytes: &[u8]) -> Result<Self, MdfError> {

        let header = Self::parse_header(bytes)?;

        // Validate the buffer is long enough for the link section AND the
        // three data bytes that follow it, before reading anything.
        // (u64 arithmetic avoids overflow for absurd links_nr values.)
        let required = 24u64
            .saturating_add(header.links_nr.saturating_mul(8))
            .saturating_add(3);
        if (bytes.len() as u64) < required {
            return Err(MdfError::TooShortBuffer {
                actual:   bytes.len(),
                expected: usize::try_from(required).unwrap_or(usize::MAX),
                file:     file!(),
                line:     line!(),
            });
        }
        let link_count = header.links_nr as usize;

        // Link section: one LINK (u64 LE) per link_count (max 3 meaningful)
        let mut name_addr    = 0;
        let mut path_addr    = 0;
        let mut comment_addr = 0;
        for i in 0..link_count.min(3) {
            let off = 24 + i * 8;
            let link = LittleEndian::read_u64(&bytes[off..off + 8]);
            match i {
                0 => name_addr    = link,
                1 => path_addr    = link,
                2 => comment_addr = link,
                _ => {}
            }
        }

        // Data section immediately after all links:
        let data_start = 24 + link_count * 8;
        let source_type = bytes[data_start];
        let bus_type    = bytes[data_start + 1];
        let flags       = bytes[data_start + 2];
        // bytes [data_start+3 .. data_start+8] are reserved/padding

        Ok(SourceBlock {
            header,
            name_addr,
            path_addr,
            comment_addr,
            source_type,
            bus_type,
            flags,
        })
    }
}

/// Read an [`SIBLOCK`](SourceBlock) from the memory mapped file.
///
/// # Arguments
/// * `mmap` - The entire MDF file mapped into memory.
/// * `address` - File offset of the `##SI` block.
///
/// # Returns
/// The parsed [`SourceBlock`] or an [`MdfError`] if decoding fails.
pub fn read_source_block(mmap: &[u8], address: u64) -> Result<SourceBlock, MdfError> {

    let start = address as usize;
    if start.checked_add(24).map_or(true, |end| end > mmap.len()) {
        return Err(MdfError::TooShortBuffer {
            actual:   mmap.len(),
            expected: start.saturating_add(24),
            file:     file!(),
            line:     line!(),
        });
    }
    let header = BlockHeader::from_bytes(&mmap[start..start + 24])?;
    // We know the total length from the header:
    let total_len = header.block_len as usize;
    let end = start.checked_add(total_len).ok_or(MdfError::TooShortBuffer {
        actual:   mmap.len(),
        expected: usize::MAX,
        file:     file!(),
        line:     line!(),
    })?;
    if end > mmap.len() {
        return Err(MdfError::TooShortBuffer {
            actual:   mmap.len(),
            expected: end,
            file:     file!(),
            line:     line!(),
        });
    }
    let slice = &mmap[start..end];
    Ok(SourceBlock::from_bytes(slice)?)
}
