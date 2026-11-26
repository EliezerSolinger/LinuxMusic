//! What a device list offers the user (U17), the same for every backend

/// One device the user can choose (U17)
#[derive(Clone, Debug, PartialEq)]
pub struct DeviceChoice {
    /// What to put in [`DeviceConfig`]: `hw:CARD=<id>,DEV=<n>`
    pub name: String,
    pub card: String,
    pub description: String,
    pub playback: Option<ChoiceCaps>,
    pub capture: Option<ChoiceCaps>,
    /// Another program has it (PipeWire, PulseAudio): opening asks it with ReserveDevice1 (U8)
    pub busy: bool,
    /// No permission (U8): the `audio` group
    pub denied: bool,
}

/// What one direction of a device can do
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChoiceCaps {
    pub channels_min: u32,
    pub channels_max: u32,
    pub rates: Vec<u32>,
}
