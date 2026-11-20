//! The application's pages: Composition and Mixing (user's decision, 19/11) and the MIDI maps (I-21.2), switched by
//! the tabs on top (Cmd/Ctrl+1, +2, +3, Ctrl+Tab), with the transport bar over them.

pub mod composition;
pub mod desk;
pub mod midi_maps;
pub mod mixing;
pub mod transport;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Composition,
    Mixing,
    MidiMaps,
}

impl Page {
    pub fn title(self) -> &'static str {
        match self {
            Page::Composition => "Composition",
            Page::Mixing => "Mixing",
            Page::MidiMaps => "MIDI maps",
        }
    }
}
