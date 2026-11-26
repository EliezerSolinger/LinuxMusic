//! The numbers that name things across the two halves of the engine (interfaces.md, C13). The audio side never sees
//! the document's ids (rule 6): the compiler maps each `NodeId` to a [`NodeIndex`] and each parameter to a
//! [`ParamSlot`], and every message about a node carries the [`ProgramGen`] its table belongs to (S1, S9).

/// The generation of a program: incremented by the compiler for every [`crate::Install`] it produces; starts at 1 and
/// never goes back. Generation 0 is the empty program `engine()` starts with
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProgramGen(pub u64);

/// A slot in the renderer's node table (`< Limits::max_nodes`), with the slot's generation beside it.
///
/// The compiler is the only one that hands indices out (S6) and bumps `generation` every time it reuses a slot, so a
/// message about an old occupant of the slot can never be mistaken for the new one
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeIndex {
    pub index: u32,
    pub generation: u32,
}

impl NodeIndex {
    pub const fn new(index: u32, generation: u32) -> Self {
        Self { index, generation }
    }

    pub(crate) fn slot(self) -> usize {
        self.index as usize
    }
}

/// A parameter slot (`< Limits::max_params`), handed out by the compiler (I-6.1)
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ParamSlot(pub u32);

/// A MIDI input port, as the program's port table knows it (I-7.1)
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PortIndex(pub u16);

/// The first port index of the interface's own live notes (I-15 `note_on`): the port of track number `n` (its place in
/// the tree's track list) is `GUI_PORT_BASE + n`. Device ports are numbered from 0 by the session and never reach this
pub const GUI_PORT_BASE: u16 = 0x8000;

/// The live-note port of the track at `ordinal` in the tree's track list
pub fn gui_port(ordinal: usize) -> PortIndex {
    PortIndex(GUI_PORT_BASE.saturating_add(u16::try_from(ordinal).unwrap_or(u16::MAX >> 1).min(0x7FFF)))
}

impl PortIndex {
    /// Whether this is an interface live-note port (no mapping, no recording: those are for devices)
    pub fn is_gui(self) -> bool {
        self.0 >= GUI_PORT_BASE
    }
}

/// One of the fixed command producers (`< Limits::max_command_producers`); 0 is the main thread's (I-7.1)
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProducerIndex(pub u16);

/// A preview, numbered by the session (C13)
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PreviewId(pub u64);

/// A note as a processor sees it: the renderer numbers every note-on, and the matching note-off carries the same
/// number (U25), so a processor never has to match keys
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NoteId(pub u32);
