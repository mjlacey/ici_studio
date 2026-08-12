use crate::mf4::error::MdfError;
use crate::mf4::parsing::raw_channel_group::RawChannelGroup;
use crate::mf4::blocks::{
    compressed_data_block::read_dz_block,
    data_block::DataBlock,
    data_group_block::DataGroupBlock,
    data_list_block::DataListBlock,
    history_list_block::HistoryListBlock,
    common::BlockParse,
};

/// Read the 4-byte block id at `offset` without committing to any one
/// block's full layout yet -- `##HL`/`##DL` entries need to be told apart
/// from `##DT`/`##DV`/`##DZ` fragments before picking how to parse them.
fn peek_block_id(mmap: &[u8], offset: usize) -> Result<&str, MdfError> {
    let id_bytes = mmap.get(offset..offset.saturating_add(4)).ok_or(MdfError::TooShortBuffer {
        actual:   mmap.len(),
        expected: offset.saturating_add(4),
        file:     file!(),
        line:     line!(),
    })?;
    std::str::from_utf8(id_bytes).map_err(|_| MdfError::BlockIDError {
        actual: format!("{id_bytes:?}"),
        expected: "a 4-byte ASCII block id".to_string(),
    })
}

#[derive(Debug)]
pub struct RawDataGroup {
    pub block: DataGroupBlock,
    pub channel_groups: Vec<RawChannelGroup>,
}
impl RawDataGroup {

    /// Collect all data blocks referenced by this data group.
    ///
    /// The returned vector contains the decoded `DT`/`DV` fragments in the
    /// order they appear on disk, transparently following `DL` list chains
    /// and resolving `HL` (history list / zipped) wrappers -- a `DZ`
    /// fragment inside either is inflated (and, if `dz_zip_type` says so,
    /// un-transposed) here too, so callers never see the difference between
    /// a compressed and an uncompressed file.
    ///
    /// # Arguments
    /// * `mmap` - The MDF file's bytes
    ///
    /// # Returns
    /// A vector of [`DataBlock`] objects or an [`MdfError`] if parsing fails.
    pub fn data_blocks(&self, mmap: &[u8]) -> Result<Vec<DataBlock>, MdfError> {
        // Unsorted data groups interleave records of several channel groups
        // (each prefixed by a record id). Framing them as fixed-size records
        // of a single group silently mis-decodes the data, so refuse loudly.
        // Metadata access (names, channels) is unaffected — only record/data
        // access goes through here.
        if self.block.record_id_len > 0 && self.channel_groups.len() > 1 {
            return Err(MdfError::BlockSerializationError(
                "unsorted data groups (multiple channel groups per data group) are not supported"
                    .to_string(),
            ));
        }

        let mut collected_blocks = Vec::new();

        // Start at the group’s primary data pointer
        let mut current_block_address = self.block.data_block_addr;
        let mut visited = std::collections::HashSet::new();
        while current_block_address != 0 {
            if !visited.insert(current_block_address) {
                return Err(MdfError::BlockLinkError(format!(
                    "cycle detected in data block chain at address {:#x}",
                    current_block_address
                )));
            }
            let byte_offset = current_block_address as usize;
            let id = peek_block_id(mmap, byte_offset)?;

            match id {
                "##DT" | "##DV" => {
                    // Single contiguous, uncompressed DataBlock
                    let data_block = DataBlock::from_bytes(&mmap[byte_offset..])?;
                    collected_blocks.push(data_block);
                    // No list to follow, we’re done
                    current_block_address = 0;
                }
                "##DZ" => {
                    // Single contiguous, compressed DataBlock (no ##DL wrapper).
                    let data_block = read_dz_block(&mmap[byte_offset..])?;
                    collected_blocks.push(data_block);
                    current_block_address = 0;
                }
                "##HL" => {
                    // History-list wrapper: resolve straight through to the
                    // ##DL chain it wraps and keep walking from there.
                    let hl = HistoryListBlock::from_bytes(&mmap[byte_offset..])?;
                    current_block_address = hl.dl_first;
                }
                "##DL" => {
                    // Fragmented list of data blocks
                    let data_list_block = DataListBlock::from_bytes(&mmap[byte_offset..])?;

                    // Parse each fragment in this list -- plain (##DT/##DV)
                    // or zipped (##DZ), decided per-fragment since a ##DL
                    // reached directly (no ##HL) is always uncompressed but
                    // the reverse isn't guaranteed by anything read so far.
                    for &fragment_address in &data_list_block.data_links {
                        if fragment_address == 0 {
                            continue; // null link
                        }
                        let fragment_offset = fragment_address as usize;
                        let fragment_id = peek_block_id(mmap, fragment_offset)?;
                        let fragment_block = match fragment_id {
                            "##DZ" => read_dz_block(&mmap[fragment_offset..])?,
                            _ => DataBlock::from_bytes(&mmap[fragment_offset..])?,
                        };
                        collected_blocks.push(fragment_block);
                    }

                    // Move to the next DLBLOCK in the chain (0 = end)
                    current_block_address = data_list_block.next;
                }

                unexpected_id => {
                    return Err(MdfError::BlockIDError {
                        actual: unexpected_id.to_string(),
                        expected: "##DT / ##DV / ##DZ / ##DL / ##HL".to_string(),
                    });
                }
            }
        }

        Ok(collected_blocks)
    }
}