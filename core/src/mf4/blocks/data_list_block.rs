use crate::mf4::blocks::common::BlockHeader;
use crate::mf4::blocks::common::BlockParse;
use crate::mf4::error::MdfError;

/// DLBLOCK: Data List Block (ordered list of data blocks for signal/reduction)
pub struct DataListBlock {
    pub header: BlockHeader,
    pub next: u64,             // link to next DLBLOCK
    pub data_links: Vec<u64>,  // list of offsets to DT/RD/DV/RV/SDBLOCKs
    pub flags: u8,
    pub reserved1: [u8; 3],
    pub data_block_nr: u32,
    pub data_block_len: Option<u64>,
    pub offsets: Option<Vec<u64>>,
}

impl BlockParse<'_> for DataListBlock {
    const ID: &'static str = "##DL";
    /// Parse a DLBLOCK from raw bytes.
    ///
    /// The DLBLOCK contains a list of links to data fragments. This function
    /// validates the minimum size based on the number of links declared in the
    /// header and reads all additional fields.
    fn from_bytes(bytes: &[u8]) -> Result<Self, MdfError> {

        let header = Self::parse_header(bytes)?;

        // A DLBLOCK always carries at least the 'next' link. links_nr == 0
        // would underflow the data-link count below.
        if header.links_nr == 0 {
            return Err(MdfError::BlockSerializationError(
                "DataListBlock must have links_nr >= 1 (the 'next' link)".to_string(),
            ));
        }

        // u64 arithmetic avoids overflow for absurd links_nr values.
        let min_len_u64 = 24u64
            .saturating_add(header.links_nr.saturating_mul(8))
            .saturating_add(1 + 3 + 4);
        if (bytes.len() as u64) < min_len_u64 {
            return Err(MdfError::TooShortBuffer {
                actual: bytes.len(),
                expected: usize::try_from(min_len_u64).unwrap_or(usize::MAX),
                file: file!(), line: line!(),
            });
        }
        // Parse links: first is 'next', then data links
        let mut off = 24;
        let next = u64::from_le_bytes(bytes[off..off+8].try_into().unwrap());
        off += 8;

        // Remaining links all point to data blocks
        let link_count = header.links_nr as usize;
        let mut data_links = Vec::with_capacity(link_count - 1);
        for _ in 1..link_count {
            let l = u64::from_le_bytes(bytes[off..off+8].try_into().unwrap());
            data_links.push(l);
            off += 8;
        }
        let flags = bytes[off];
        off += 1;
        let reserved1 = [bytes[off], bytes[off+1], bytes[off+2]];
        off += 3;
        let data_block_nr = u32::from_le_bytes(bytes[off..off+4].try_into().unwrap());
        off += 4;

        let (data_block_len, offsets) = if flags & 1 != 0 {
            if bytes.len() < off + 8 {
                return Err(MdfError::TooShortBuffer {
                    actual: bytes.len(),
                    expected: off + 8,
                    file: file!(), line: line!(),
                });
            }
            let len = u64::from_le_bytes(bytes[off..off+8].try_into().unwrap());
            (Some(len), None)
        } else {
            // Check the length BEFORE allocating, so a bogus data_block_nr
            // cannot trigger a huge allocation.
            let needed = (off as u64).saturating_add(data_block_nr as u64 * 8);
            if (bytes.len() as u64) < needed {
                return Err(MdfError::TooShortBuffer {
                    actual: bytes.len(),
                    expected: usize::try_from(needed).unwrap_or(usize::MAX),
                    file: file!(), line: line!(),
                });
            }
            let mut offs = Vec::with_capacity(data_block_nr as usize);
            for _ in 0..data_block_nr {
                let o = u64::from_le_bytes(bytes[off..off+8].try_into().unwrap());
                offs.push(o);
                off += 8;
            }
            (None, Some(offs))
        };

        Ok(DataListBlock { header, next, data_links, flags, reserved1, data_block_nr, data_block_len, offsets })
    }
}

impl DataListBlock {
    /// Create a new `DataListBlock` for equal-length data blocks.
    ///
    /// # Arguments
    /// * `data_links` - Addresses of all data blocks referenced by this list.
    /// * `data_block_len` - Length in bytes of every referenced data block.
    ///
    /// # Returns
    /// A [`DataListBlock`] ready for serialization.
    pub fn new_equal(data_links: Vec<u64>, data_block_len: u64) -> Self {
        let links_nr = data_links.len() as u64 + 1; // +1 for 'next'
        let block_len = 24 + links_nr * 8 + 1 + 3 + 4 + 8;
        let header = BlockHeader {
            id: "##DL".to_string(),
            reserved0: 0,
            block_len,
            links_nr,
        };
        Self {
            header,
            next: 0,
            data_links,
            flags: 1,
            reserved1: [0; 3],
            data_block_nr: links_nr as u32 - 1,
            data_block_len: Some(data_block_len),
            offsets: None,
        }
    }

    /// Create a new `DataListBlock` for variable-length data blocks (flags=0).
    ///
    /// The `offsets` vector must provide, for each entry in `data_links`, the
    /// virtual byte position of that fragment's data section in the concatenated
    /// logical data section. By convention `offsets[0] == 0` and
    /// `offsets[i] == offsets[i-1] + (data_section_size_of_fragment_{i-1})`.
    ///
    /// Use this form when fragment sizes differ — for example, ##SD chains
    /// where entries are variable-length and fragment splits land at entry
    /// boundaries. Lying to a spec-conformant reader with the equal-length
    /// form (`flags=1`) when fragment sizes actually differ causes random-
    /// access lookups (as performed by VLSD inline-offset readers) to land in
    /// the wrong fragment or out of bounds.
    ///
    /// # Panics
    /// Panics in debug builds if `data_links.len() != offsets.len()`.
    pub fn new_variable(data_links: Vec<u64>, offsets: Vec<u64>) -> Self {
        debug_assert_eq!(data_links.len(), offsets.len());
        let links_nr = data_links.len() as u64 + 1; // +1 for 'next'
        let block_len = 24 + links_nr * 8 + 1 + 3 + 4 + offsets.len() as u64 * 8;
        let header = BlockHeader {
            id: "##DL".to_string(),
            reserved0: 0,
            block_len,
            links_nr,
        };
        Self {
            header,
            next: 0,
            data_links,
            flags: 0,
            reserved1: [0; 3],
            data_block_nr: links_nr as u32 - 1,
            data_block_len: None,
            offsets: Some(offsets),
        }
    }

    /// Serialize this DLBLOCK to bytes.
    ///
    /// # Returns
    /// The binary representation of the block or an [`MdfError`] on failure.
    pub fn to_bytes(&self) -> Result<Vec<u8>, MdfError> {
        if self.header.id != "##DL" {
            return Err(MdfError::BlockSerializationError(
                format!("DataListBlock must have ID '##DL', found '{}'", self.header.id)
            ));
        }

        let links_nr = self.data_links.len() as u64 + 1;
        let extra = if self.flags & 1 != 0 {
            1 + 3 + 4 + 8
        } else {
            1 + 3 + 4 + (self.data_block_nr as usize * 8)
        };
        let block_len = 24 + links_nr * 8 + extra as u64;

        if self.header.links_nr != links_nr {
            return Err(MdfError::BlockSerializationError(
                format!("DataListBlock links_nr mismatch: header {} vs actual {}", self.header.links_nr, links_nr)
            ));
        }
        if self.header.block_len != block_len {
            return Err(MdfError::BlockSerializationError(
                format!("DataListBlock block_len mismatch: header {} vs actual {}", self.header.block_len, block_len)
            ));
        }

        let mut buf = Vec::with_capacity(block_len as usize);
        buf.extend_from_slice(&self.header.to_bytes()?);
        buf.extend_from_slice(&self.next.to_le_bytes());
        for link in &self.data_links {
            buf.extend_from_slice(&link.to_le_bytes());
        }
        buf.push(self.flags);
        buf.extend_from_slice(&self.reserved1);
        buf.extend_from_slice(&self.data_block_nr.to_le_bytes());
        if self.flags & 1 != 0 {
            buf.extend_from_slice(&self.data_block_len.unwrap_or(0).to_le_bytes());
        } else if let Some(offsets) = &self.offsets {
            for o in offsets {
                buf.extend_from_slice(&o.to_le_bytes());
            }
        }
        Ok(buf)
    }
}
