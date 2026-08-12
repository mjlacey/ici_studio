use byteorder::{ByteOrder, LittleEndian};

use crate::mf4::error::MdfError;
use crate::mf4::blocks::common::BlockParse;
use crate::mf4::blocks::common::{BlockHeader, DataType};
use crate::mf4::blocks::text_block::TextBlock;
use crate::mf4::blocks::conversion::ConversionBlock;
use crate::mf4::parsing::decoder::DecodedValue;

#[derive(Debug, Clone)]
pub struct ChannelBlock {
    pub header: BlockHeader, // Common header
    // The rest of your fields:
    pub next_ch_addr: u64,     // 8 bytes
    pub component_addr: u64,   // 8 bytes
    pub name_addr: u64,        // 8 bytes – pointer to a TextBlock containing the channel name
    pub source_addr: u64,      // 8 bytes
    pub conversion_addr: u64,  // 8 bytes
    pub data: u64,             // 8 bytes
    pub unit_addr: u64,        // 8 bytes
    pub comment_addr: u64,     // 8 bytes
    pub channel_type: u8,      // 1 byte
    pub sync_type: u8,         // 1 byte
    pub data_type: DataType,   // Data type enum
    pub bit_offset: u8,        // 1 byte
    pub byte_offset: u32,      // 4 bytes
    pub bit_count: u32,        // 4 bytes
    pub flags: u32,            // 4 bytes
    pub pos_invalidation_bit: u32, // 4 bytes
    pub precision: u8,         // 1 byte
    pub reserved1: u8,         // 1 byte
    pub attachment_nr: u16,    // 2 bytes
    pub min_raw_value: f64,    // 8 bytes
    pub max_raw_value: f64,    // 8 bytes
    pub lower_limit: f64,      // 8 bytes
    pub upper_limit: f64,      // 8 bytes
    pub lower_ext_limit: f64,  // 8 bytes
    pub upper_ext_limit: f64,  // 8 bytes

    pub name: Option<String>,
    pub conversion: Option<ConversionBlock>,
}

impl BlockParse<'_> for ChannelBlock {
    const ID: &'static str = "##CN";
    /// Creates a ChannelBlock from a 160-byte slice.
    /// This version does NOT automatically resolve the channel name.
    fn from_bytes(bytes: &[u8]) -> Result<Self, MdfError> {

        let header = Self::parse_header(bytes)?;

        let expected_bytes = 160;
        if bytes.len() < expected_bytes {
            return Err(MdfError::TooShortBuffer {
                actual:   bytes.len(),
                expected: expected_bytes,
                file:     file!(),
                line:     line!(),
            });
        }

        Ok(Self {
            header: header,
            next_ch_addr: LittleEndian::read_u64(&bytes[24..32]),
            component_addr: LittleEndian::read_u64(&bytes[32..40]),
            name_addr: LittleEndian::read_u64(&bytes[40..48]),
            source_addr: LittleEndian::read_u64(&bytes[48..56]),
            conversion_addr: LittleEndian::read_u64(&bytes[56..64]),
            data: LittleEndian::read_u64(&bytes[64..72]),
            unit_addr: LittleEndian::read_u64(&bytes[72..80]),
            comment_addr: LittleEndian::read_u64(&bytes[80..88]),
            channel_type: bytes[88],
            sync_type: bytes[89],
            data_type: DataType::from_u8(bytes[90]),
            bit_offset: bytes[91],
            byte_offset: LittleEndian::read_u32(&bytes[92..96]),
            bit_count: LittleEndian::read_u32(&bytes[96..100]),
            flags: LittleEndian::read_u32(&bytes[100..104]),
            pos_invalidation_bit: LittleEndian::read_u32(&bytes[104..108]),
            precision: bytes[108],
            reserved1: bytes[109],
            attachment_nr: LittleEndian::read_u16(&bytes[110..112]),
            min_raw_value: LittleEndian::read_f64(&bytes[112..120]),
            max_raw_value: LittleEndian::read_f64(&bytes[120..128]),
            lower_limit: LittleEndian::read_f64(&bytes[128..136]),
            upper_limit: LittleEndian::read_f64(&bytes[136..144]),
            lower_ext_limit: LittleEndian::read_f64(&bytes[144..152]),
            upper_ext_limit: LittleEndian::read_f64(&bytes[152..160]),
            name: None,
            conversion: None,
        })
    }
}

impl ChannelBlock {
    
    /// Serializes the ChannelBlock to bytes according to MDF 4.1 specification.
    /// 
    /// # Structure (160 bytes total):
    /// - BlockHeader (24 bytes): Standard block header with id="##CN"
    /// - Link section (64 bytes): Eight 8-byte links to other blocks
    ///   * next_ch_addr: Link to next channel block
    ///   * component_addr: Link to component block
    ///   * name_addr: Link to name text block
    ///   * source_addr: Link to source block
    ///   * conversion_addr: Link to conversion block
    ///   * data: Signal data (or link to data block depending on type)
    ///   * unit_addr: Link to unit text block
    ///   * comment_addr: Link to comment text block
    /// - Format section (24 bytes): Information about data format
    ///   * channel_type: Channel type (1 byte)
    ///   * sync_type: Sync type (1 byte)
    ///   * data_type: Data type (1 byte)
    ///   * bit_offset: Bit offset (1 byte)
    ///   * byte_offset: Byte offset (4 bytes)
    ///   * bit_count: Bit count (4 bytes)
    ///   * flags: Flags (4 bytes)
    ///   * pos_invalidation_bit: Invalidation bit position (4 bytes)
    ///   * precision: Precision (1 byte)
    ///   * reserved1: Reserved space (1 byte)
    ///   * attachment_nr: Attachment number (2 bytes)
    /// - Range section (48 bytes): Six 8-byte double values for range information
    ///   * min_raw_value: Minimum raw value (f64)
    ///   * max_raw_value: Maximum raw value (f64)
    ///   * lower_limit: Lower limit (f64)
    ///   * upper_limit: Upper limit (f64)
    ///   * lower_ext_limit: Lower extended limit (f64)
    ///   * upper_ext_limit: Upper extended limit (f64)
    /// 
    /// # Notes
    /// - The `name` and `conversion` fields are not serialized directly
    ///   as they are resolved separately via the corresponding link addresses.
    /// 
    /// # Returns
    /// - `Ok(Vec<u8>)` containing the serialized channel block
    /// - `Err(MdfError)` if serialization fails
    pub fn to_bytes(&self) -> Result<Vec<u8>, MdfError> {
        // Validate the header before serializing
        if self.header.id != "##CN" {
            return Err(MdfError::BlockSerializationError(
                format!("ChannelBlock must have ID '##CN', found '{}'", self.header.id)
            ));
        }
        
        if self.header.block_len != 160 {
            return Err(MdfError::BlockSerializationError(
                format!("ChannelBlock must have block_len=160, found {}", self.header.block_len)
            ));
        }
        
        // Create a buffer with exact capacity for efficiency
        let mut buffer = Vec::with_capacity(160);
        
        // 1. Write the block header (24 bytes)
        buffer.extend_from_slice(&self.header.to_bytes()?);
        
        // 2. Write the link addresses (64 bytes total, 8 links at 8 bytes each)
        buffer.extend_from_slice(&self.next_ch_addr.to_le_bytes());     // Next channel block
        buffer.extend_from_slice(&self.component_addr.to_le_bytes());   // Component block
        buffer.extend_from_slice(&self.name_addr.to_le_bytes());        // Name text block
        buffer.extend_from_slice(&self.source_addr.to_le_bytes());      // Source block
        buffer.extend_from_slice(&self.conversion_addr.to_le_bytes());  // Conversion block
        buffer.extend_from_slice(&self.data.to_le_bytes());            // Signal data
        buffer.extend_from_slice(&self.unit_addr.to_le_bytes());        // Unit text block
        buffer.extend_from_slice(&self.comment_addr.to_le_bytes());     // Comment text block
        
        // 3. Write the format section (24 bytes)
        buffer.push(self.channel_type);                               // Channel type (1 byte)
        buffer.push(self.sync_type);                                  // Sync type (1 byte)
        // Convert DataType enum to u8 using the enum's to_u8() method
        buffer.push(self.data_type.to_u8());                        // Data type (1 byte)
        buffer.push(self.bit_offset);                                // Bit offset (1 byte)
        buffer.extend_from_slice(&self.byte_offset.to_le_bytes());     // Byte offset (4 bytes)
        buffer.extend_from_slice(&self.bit_count.to_le_bytes());       // Bit count (4 bytes)
        buffer.extend_from_slice(&self.flags.to_le_bytes());           // Flags (4 bytes)
        buffer.extend_from_slice(&self.pos_invalidation_bit.to_le_bytes()); // Invalidation bit pos (4 bytes)
        buffer.push(self.precision);                                 // Precision (1 byte)
        buffer.push(self.reserved1);                                 // Reserved (1 byte)
        buffer.extend_from_slice(&self.attachment_nr.to_le_bytes());   // Attachment number (2 bytes)
        
        // 4. Write the range section (48 bytes, 6 doubles at 8 bytes each)
        buffer.extend_from_slice(&self.min_raw_value.to_le_bytes());    // Minimum raw value (f64)
        buffer.extend_from_slice(&self.max_raw_value.to_le_bytes());    // Maximum raw value (f64)
        buffer.extend_from_slice(&self.lower_limit.to_le_bytes());      // Lower limit (f64)
        buffer.extend_from_slice(&self.upper_limit.to_le_bytes());      // Upper limit (f64)
        buffer.extend_from_slice(&self.lower_ext_limit.to_le_bytes());  // Lower extended limit (f64)
        buffer.extend_from_slice(&self.upper_ext_limit.to_le_bytes());  // Upper extended limit (f64)
        
        // Verify the buffer is exactly 160 bytes
        if buffer.len() != 160 {
            return Err(MdfError::BlockSerializationError(
                format!("ChannelBlock must be exactly 160 bytes, got {}", buffer.len())
            ));
        }
        
        // Ensure 8-byte alignment (should always be true since 160 is divisible by 8)
        debug_assert_eq!(buffer.len() % 8, 0, "ChannelBlock size is not 8-byte aligned");
        
        Ok(buffer)
    }
    
    /// Load the channel name from the file using the stored `name_addr`.
    ///
    /// # Arguments
    /// * `file_data` - Memory mapped bytes of the entire MDF file.
    ///
    /// # Returns
    /// `Ok(())` on success or an [`MdfError`] if the referenced block is
    /// incomplete.
    pub fn resolve_name(&mut self, file_data: &[u8]) -> Result<(), MdfError> {
        if self.name.is_none() && self.name_addr != 0 {
            let offset = self.name_addr as usize;
            // Check that the offset is within bounds; adjust the minimum length if needed
            if offset + 16 <= file_data.len() {
                let text_block = TextBlock::from_bytes(&file_data[offset..])?;
                self.name = Some(text_block.text);
            }
        }
        Ok(())
    }

    /// Resolve and store the conversion block pointed to by `conversion_addr`.
    ///
    /// # Arguments
    /// * `bytes` - Memory mapped MDF file bytes.
    ///
    /// # Returns
    /// `Ok(())` on success or an [`MdfError`] if the conversion block cannot be
    /// read or parsed.
    pub fn resolve_conversion(&mut self, bytes: &[u8]) -> Result<(), MdfError> {
        if self.conversion.is_none() && self.conversion_addr != 0 {
            let offset = self.conversion_addr as usize;
            
            let expected_bytes = offset + 24;
            if bytes.len() < expected_bytes {
                return Err(MdfError::TooShortBuffer {
                    actual:   bytes.len(),
                    expected: expected_bytes,
                    file:     file!(),
                    line:     line!(),
                });
            }

            let mut conv_block = ConversionBlock::from_bytes(&bytes[offset..])?;

            let _ = conv_block.resolve_formula(bytes);
            self.conversion = Some(conv_block);
            

        }
        Ok(())
    }

    /// Apply the stored conversion to a decoded value.
    ///
    /// If no conversion block is attached the input value is returned
    /// unchanged.
    ///
    /// # Arguments
    /// * `raw` - The raw decoded value as returned by [`decode_channel_value`].
    /// * `file_data` - Memory mapped MDF data used to resolve formulas.
    ///
    /// # Returns
    /// The converted value or the original value on failure.
    pub fn apply_conversion_value(
        &self,
        raw: DecodedValue,
        file_data: &[u8],
    ) -> Result<DecodedValue, MdfError> {
        let decoded = if let Some(conv) = &self.conversion {
            conv.apply_decoded(raw, file_data)?
        } else {
            raw
        };
        Ok(decoded)
    }
}

impl Default for ChannelBlock {
    fn default() -> Self {
        let header = BlockHeader {
            id: String::from("##CN"),
            reserved0: 0,
            block_len: 160,
            links_nr: 8,
        };

        ChannelBlock {
            header,
            next_ch_addr: 0,
            component_addr: 0,
            name_addr: 0,
            source_addr: 0,
            conversion_addr: 0,
            data: 0,
            unit_addr: 0,
            comment_addr: 0,
            channel_type: 0,
            sync_type: 0,
            data_type: DataType::UnsignedIntegerLE,
            bit_offset: 0,
            byte_offset: 0,
            bit_count: 0,
            flags: 0,
            pos_invalidation_bit: 0,
            precision: 0,
            reserved1: 0,
            attachment_nr: 0,
            min_raw_value: 0.0,
            max_raw_value: 0.0,
            lower_limit: 0.0,
            upper_limit: 0.0,
            lower_ext_limit: 0.0,
            upper_ext_limit: 0.0,
            name: None,
            conversion: None,
        }
    }
}
