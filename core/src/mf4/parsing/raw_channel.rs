use crate::mf4::blocks::channel_block::ChannelBlock;
use crate::mf4::blocks::compressed_data_block::read_dz_block;
use crate::mf4::blocks::data_list_block::DataListBlock;
use crate::mf4::blocks::history_list_block::HistoryListBlock;
use crate::mf4::blocks::signal_data_block::SignalDataBlock;
use crate::mf4::blocks::common::BlockParse;
use crate::mf4::parsing::raw_channel_group::RawChannelGroup;
use crate::mf4::parsing::raw_data_group::RawDataGroup;
use crate::mf4::error::MdfError;

/// A channel with lazy access to its raw record bytes (fixed-length or VLSD).
#[derive(Debug)]
pub struct RawChannel {
    pub block:  ChannelBlock,
}

/// Resolves a channel's own VLSD data pointer (`##HL`/`##DL`/`##SD`/`##DZ`)
/// into a single, continuous byte buffer.
///
/// Fragments in a `##DL` chain are just a storage-level chunking of one
/// logical byte stream -- nothing requires a fragment boundary to land on an
/// entry (`[u32 len][value]`) boundary, and a real-world writer has been seen
/// to split on a fixed chunk size (4 MiB) with no regard for where entries
/// fall. Concatenating every fragment up front, rather than parsing entries
/// out of each fragment's buffer independently, means an entry that straddles
/// two fragments is simply bytes in the middle of one buffer like any other.
fn collect_vlsd_bytes(bytes: &[u8], start_addr: u64) -> Result<Vec<u8>, MdfError> {
    let mut out = Vec::new();
    let mut next_addr = start_addr;
    let mut visited: std::collections::HashSet<u64> = std::collections::HashSet::new();

    while next_addr != 0 {
        // Cycle detection: a chain that revisits an address would otherwise
        // loop forever.
        if !visited.insert(next_addr) {
            return Err(MdfError::BlockLinkError(format!(
                "cycle detected in VLSD data chain at address {:#x}",
                next_addr
            )));
        }
        let off = next_addr as usize;
        let id = bytes.get(off..off.saturating_add(4)).ok_or(MdfError::TooShortBuffer {
            actual:   bytes.len(),
            expected: off.saturating_add(4),
            file:     file!(),
            line:     line!(),
        })?;
        match id {
            b"##HL" => {
                // History-list wrapper (present when the ##DL chain it wraps
                // has ##DZ-compressed fragments): resolve straight through
                // to the ##DL it points at and keep walking from there --
                // same as raw_data_group.rs's own ##HL handling for a
                // channel group's data.
                let hl = HistoryListBlock::from_bytes(&bytes[off..])?;
                next_addr = hl.dl_first;
            }
            b"##DL" => {
                let dl = DataListBlock::from_bytes(&bytes[off..])?;
                for &frag_addr in &dl.data_links {
                    if frag_addr == 0 {
                        continue; // null link
                    }
                    let frag_off = frag_addr as usize;
                    let frag_id = bytes.get(frag_off..frag_off.saturating_add(4)).ok_or(MdfError::TooShortBuffer {
                        actual:   bytes.len(),
                        expected: frag_off.saturating_add(4),
                        file:     file!(),
                        line:     line!(),
                    })?;
                    match frag_id {
                        b"##DZ" => out.extend_from_slice(&read_dz_block(&bytes[frag_off..])?.data),
                        _ => out.extend_from_slice(SignalDataBlock::from_bytes(&bytes[frag_off..])?.data),
                    }
                }
                next_addr = dl.next;
            }
            b"##SD" => {
                out.extend_from_slice(SignalDataBlock::from_bytes(&bytes[off..])?.data);
                next_addr = 0;
            }
            b"##DZ" => {
                out.extend_from_slice(&read_dz_block(&bytes[off..])?.data);
                next_addr = 0;
            }
            other => {
                return Err(MdfError::BlockIDError {
                    actual:   String::from_utf8_lossy(other).into(),
                    expected: "##HL / ##DL / ##SD / ##DZ".to_string(),
                });
            }
        }
    }
    Ok(out)
}

impl<'a> RawChannel {

    /// Return an iterator over raw record bytes for this channel.
    ///
    /// The iterator yields a `Result` for each record and transparently handles
    /// both fixed-size and VLSD storage schemes.
    ///
    /// # Arguments
    /// * `data_group` - Parent data group owning the records
    /// * `channel_group` - Channel group this channel belongs to
    /// * `mmap` - Memory mapped MDF data
    ///
    /// # Returns
    /// An iterator over byte slices containing each raw record, or an
    /// [`MdfError`] if the underlying blocks could not be parsed.
    pub fn records(
        &self,
        data_group: &'a RawDataGroup,
        channel_group: &'a RawChannelGroup,
        mmap: &'a [u8],
    ) -> Result<Box<dyn Iterator<Item = Result<Vec<u8>, MdfError>> + 'a>, MdfError> {
        // 1) VLSD path: channel has its own data pointer => SD/DL chain
        if self.block.channel_type == 1 && self.block.data != 0 {
            // Resolved eagerly into one continuous buffer -- see
            // `collect_vlsd_bytes`'s doc comment for why entries can't be
            // parsed fragment-by-fragment.
            let all_bytes = collect_vlsd_bytes(mmap, self.block.data)?;
            let mut pos = 0usize;

            let vlsd_iter = std::iter::from_fn(move || -> Option<Result<Vec<u8>, MdfError>> {
                if pos + 4 > all_bytes.len() {
                    return None;
                }
                let len = u32::from_le_bytes(all_bytes[pos..pos+4].try_into().unwrap()) as usize;
                let start = pos + 4;
                let end = start + len;
                if end > all_bytes.len() {
                    return Some(Err(MdfError::TooShortBuffer {
                        actual:   all_bytes.len(),
                        expected: end,
                        file:     file!(),
                        line:     line!(),
                    }));
                }
                let value = all_bytes[start..end].to_vec();
                pos = end;
                Some(Ok(value))
            });

            return Ok(Box::new(vlsd_iter));
        }

        // Compute the size of each record:
        // Record structure: record_id + data_bytes + invalidation_bytes
        let record_id_len       = data_group.block.record_id_len as usize;
        let sample_byte_len     = channel_group.block.samples_byte_nr as usize;
        let invalidation_bytes  = channel_group.block.invalidation_bytes_nr as usize;
        let record_size         = record_id_len + sample_byte_len + invalidation_bytes;

        // A record size of zero (malformed channel group) would make the
        // chunking below divide by zero / panic — there are no records.
        if record_size == 0 {
            return Ok(Box::new(std::iter::empty()));
        }

        // Gather all DataBlock fragments (DT, DV or DZ):
        let blocks = data_group.data_blocks(mmap)?;

        // Build a single iterator that:
        //  - goes block by block
        //  - trims any partial record at the end of each block
        //  - yields one owned Vec<u8> of length `record_size` per record
        //
        // Eagerly collected per block (rather than a lazy `chunks_exact`)
        // because `data_block.data` is now always owned (§DataBlock's own
        // doc comment -- a ##DZ fragment's bytes don't exist anywhere to
        // borrow from), so a lazily-borrowing iterator can't outlive this
        // closure the way it could when `data` was a zero-copy `&'a [u8]`.
        let iter = blocks.into_iter().flat_map(move |data_block| {
            let raw = data_block.data;
            let valid_len = (raw.len() / record_size) * record_size;
            raw[..valid_len]
                .chunks_exact(record_size)
                .map(|s| s.to_vec())
                .collect::<Vec<Vec<u8>>>()
                .into_iter()
                .map(Ok)
                // If you wanted to handle an unexpected remainder, you could check raw.len() % record_size != 0 here.
        });

        Ok(Box::new(iter))
    }
}

