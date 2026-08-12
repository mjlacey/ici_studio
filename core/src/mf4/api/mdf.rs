use crate::mf4::error::MdfError;
use crate::mf4::parsing::mdf_file::MdfFile;
use crate::mf4::api::channel_group::ChannelGroup;
use crate::mf4::api::channel::Channel;

#[derive(Debug)]
/// High level representation of an MDF file.
///
/// The struct stores the file's bytes internally and lazily exposes
/// [`ChannelGroup`] wrappers for easy inspection.
pub struct MDF {
    raw: MdfFile,
}

impl MDF {
    /// Parse an MDF4 file from an owned byte buffer -- the only entry point
    /// this app needs, since it never has a filesystem path to read: bytes
    /// arrive as a browser `ArrayBuffer` (via wasm) or `std::fs::read` (in a
    /// native test).
    pub fn from_bytes(data: Vec<u8>) -> Result<Self, MdfError> {
        let raw = MdfFile::parse_from_bytes(data)?;
        Ok(MDF { raw })
    }

    /// Retrieve channel groups contained in the file.
    ///
    /// Each [`ChannelGroup`] is created lazily and does not decode any samples.
    pub fn channel_groups(&self) -> Vec<ChannelGroup<'_>> {
        let mut groups = Vec::new();

        for raw_data_group in &self.raw.data_groups {
            for raw_channel_group in &raw_data_group.channel_groups {
                groups.push(ChannelGroup::new(
                    raw_data_group,
                    raw_channel_group,
                    &self.raw.mmap,
                ));
            }
        }

        groups
    }

    /// Find a channel group by name (first match).
    ///
    /// Convenience over [`MDF::channel_groups`] for the common case of
    /// addressing a group by its acquisition name.
    pub fn group(&self, name: &str) -> Option<ChannelGroup<'_>> {
        self.channel_groups()
            .into_iter()
            .find(|g| g.name().ok().flatten().as_deref() == Some(name))
    }

    /// Find a channel by name across all groups (first match).
    pub fn channel(&self, name: &str) -> Option<Channel<'_>> {
        for group in self.channel_groups() {
            for channel in group.channels() {
                if channel.name().ok().flatten().as_deref() == Some(name) {
                    return Some(channel);
                }
            }
        }
        None
    }

    /// Read a channel by name as a [`Signal`] (values paired with the master
    /// time axis of the channel's group). First match across all groups.
    ///
    /// Returns `Ok(None)` if no channel with that name exists.
    pub fn signal(&self, name: &str) -> Result<Option<crate::mf4::signal::Signal>, MdfError> {
        for group in self.channel_groups() {
            if let Some(sig) = group.signal(name)? {
                return Ok(Some(sig));
            }
        }
        Ok(None)
    }

    /// Get the start time of the measurement in nanoseconds since epoch.
    ///
    /// This is the absolute timestamp stored in the MDF file header.
    /// Returns None if the start time is 0 (not set).
    pub fn start_time_ns(&self) -> Option<u64> {
        let time = self.raw.header.abs_time;
        if time == 0 {
            None
        } else {
            Some(time)
        }
    }
}
