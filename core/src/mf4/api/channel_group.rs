use crate::mf4::blocks::common::read_string_block;
use crate::mf4::parsing::raw_data_group::RawDataGroup;
use crate::mf4::parsing::raw_channel_group::RawChannelGroup;
use crate::mf4::parsing::source_info::SourceInfo;
use crate::mf4::api::channel::Channel;
use crate::mf4::error::MdfError;
use crate::mf4::signal::Signal;

/// High level wrapper for a channel group.
///
/// The struct references raw channel group data and provides ergonomic access
/// to its metadata and channels without decoding any actual samples.
pub struct ChannelGroup<'a> {
    raw_data_group:    &'a RawDataGroup,
    raw_channel_group: &'a RawChannelGroup,
    mmap:              &'a [u8],
}

impl<'a> ChannelGroup<'a> {
    /// Create a new [`ChannelGroup`] referencing the underlying raw blocks.
    ///
    /// # Arguments
    /// * `raw_data_group` - Parent data group containing this channel group
    /// * `raw_channel_group` - The raw channel group block
    /// * `mmap` - Memory mapped file backing all data
    ///
    /// # Returns
    /// A [`ChannelGroup`] handle with no decoded data.
    pub fn new(
        raw_data_group: &'a RawDataGroup,
        raw_channel_group: &'a RawChannelGroup,
        mmap: &'a [u8],
    ) -> Self {
        ChannelGroup { raw_data_group, raw_channel_group, mmap }
    }

    /// Retrieve the human readable group name.
    pub fn name(&self) -> Result<Option<String>, MdfError> {
        read_string_block(self.mmap, self.raw_channel_group.block.acq_name_addr)
    }

    /// Retrieve the group comment if present.
    pub fn comment(&self) -> Result<Option<String>, MdfError> {
        read_string_block(self.mmap, self.raw_channel_group.block.comment_addr)
    }

    /// Get the acquisition source information if available.
    pub fn source(&self) -> Result<Option<SourceInfo>, MdfError> {
        let addr = self.raw_channel_group.block.acq_source_addr;
        SourceInfo::from_mmap(self.mmap, addr)
    }

    /// Build all [`Channel`] objects for this group.
    ///
    /// No channel data is decoded; the returned channels simply reference the
    /// raw blocks.
    pub fn channels(&self) -> Vec<Channel<'a>> {

        let mut channels = Vec::new();
        for raw_channel in &self.raw_channel_group.raw_channels {
            let channel = Channel::new(
                &raw_channel.block,
                self.raw_data_group,
                self.raw_channel_group,
                raw_channel,
                self.mmap,
            );
            channels.push(channel);
        }

        channels
    }

    /// Find a channel in this group by name (first match).
    pub fn channel(&self, name: &str) -> Option<Channel<'a>> {
        self.channels()
            .into_iter()
            .find(|c| c.name().ok().flatten().as_deref() == Some(name))
    }

    /// Read a channel by name as a [`Signal`] (values paired with the group's
    /// master/time axis).
    ///
    /// Returns `Ok(None)` if no channel with that name exists in this group.
    /// `timestamps` is empty when the group has no master channel or when the
    /// requested channel *is* the master.
    pub fn signal(&self, name: &str) -> Result<Option<Signal>, MdfError> {
        let channels = self.channels();
        let mut target: Option<usize> = None;
        let mut master: Option<usize> = None;
        for (i, ch) in channels.iter().enumerate() {
            // First master wins (matches MdfIndex's master selection)
            if master.is_none() && ch.block().channel_type == 2 {
                master = Some(i);
            }
            if target.is_none() && ch.name()?.as_deref() == Some(name) {
                target = Some(i);
            }
        }
        let Some(ci) = target else { return Ok(None) };

        let values = channels[ci].values()?;
        let timestamps = match master {
            Some(mi) if mi != ci => channels[mi].values_as_f64()?,
            _ => Vec::new(),
        };
        Ok(Some(Signal {
            name: name.to_string(),
            unit: channels[ci].unit()?,
            timestamps,
            values,
        }))
    }

    /// Get the raw data group (for internal use)
    pub fn raw_data_group(&self) -> &RawDataGroup {
        self.raw_data_group
    }

    /// Get the raw channel group (for internal use) 
    pub fn raw_channel_group(&self) -> &RawChannelGroup {
        self.raw_channel_group
    }

    /// Get the memory mapped data (for internal use)
    pub fn mmap(&self) -> &[u8] {
        self.mmap
    }
}
