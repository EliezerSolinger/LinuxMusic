//! The install message and what comes back (interfaces.md, I-5.2, I-5.3, S5, S6, D2, T9, U32)

use std::fmt;

use crate::command::CommandReceiver;
use crate::ids::{NodeIndex, ProducerIndex, ProgramGen};
use crate::midimap::MidiMap;
use crate::processor::{DisplayWriter, HostOutbox, Processor};
use crate::program::Program;
use crate::rng::Rng;

/// A processor with what lives beside it in its table slot: its random generator (seeded by the compiler from the
/// project seed and the node's id, C13), its outbox to the host and its display writer. Made on the main thread
pub struct NodeCore {
    pub(crate) processor: Box<dyn Processor>,
    pub(crate) rng: Rng,
    pub(crate) outbox: HostOutbox,
    pub(crate) display: Option<DisplayWriter>,
}

impl NodeCore {
    pub fn new(processor: Box<dyn Processor>, seed: u64, display: Option<DisplayWriter>) -> Box<Self> {
        Box::new(Self { processor, rng: Rng::new(seed), outbox: HostOutbox::default(), display })
    }

    pub fn processor_mut(&mut self) -> &mut dyn Processor {
        self.processor.as_mut()
    }

    pub fn into_processor(self: Box<Self>) -> Box<dyn Processor> {
        self.processor
    }
}

impl fmt::Debug for NodeCore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NodeCore").field("display", &self.display.is_some()).finish()
    }
}

/// A processor that left the table, on its way to the main thread
#[derive(Debug)]
pub struct Retired {
    pub node: NodeIndex,
    pub core: Box<NodeCore>,
}

/// H9: the tempo map changed while playing; `tick` is the musical position the compiler saw (informative: the
/// renderer keeps the musical position of the playhead it actually has, with `lm_time::relocate`)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Relocate {
    pub tick: i64,
}

#[derive(Debug)]
pub(crate) struct Replacement {
    pub(crate) node: NodeIndex,
    pub(crate) core: Box<NodeCore>,
    pub(crate) tail: Option<NodeIndex>,
}

/// Everything the audio thread put back, in space reserved on the main thread (D2)
#[derive(Debug, Default)]
pub struct Returned {
    /// The generation of the install that brought this back
    pub generation: ProgramGen,
    /// The program that was replaced
    pub program: Option<Box<Program>>,
    /// Removed and replaced processors (and, if an install wrongly added onto an occupied slot, the occupant)
    pub processors: Vec<Retired>,
    /// Command rings taken out (and a ring replaced at an occupied producer index)
    pub receivers: Vec<CommandReceiver>,
    /// The indices this install removed: free again for the compiler from now on (S6)
    pub freed: Vec<NodeIndex>,
    /// Parts of the install that named something outside the limits and were skipped (a compiler bug)
    pub skipped: u32,
    /// The MIDI map that was replaced (I-21.2)
    pub map: Option<Box<MidiMap>>,
}

/// A difference to the installed state (S5): the renderer applies every install, in order, at the start of a
/// block: (1) moves the `add`s into the table; (2) swaps the `replace`s with `adopt`; (3) takes out the `remove`s;
/// (4) installs the new command rings and removes the old; (5) swaps the program (and relocates, H9); (6) puts what
/// left into the reserved [`Returned`] and sends the message back
#[derive(Debug)]
pub struct Install {
    pub generation: ProgramGen,
    pub(crate) program: Option<Box<Program>>,
    pub(crate) add: Vec<(NodeIndex, Box<NodeCore>)>,
    pub(crate) replace: Vec<Replacement>,
    pub(crate) remove: Vec<NodeIndex>,
    pub(crate) producers: Vec<CommandReceiver>,
    pub(crate) remove_producers: Vec<ProducerIndex>,
    pub(crate) relocate: Option<Relocate>,
    pub(crate) map: Option<Box<MidiMap>>,
    pub(crate) returned: Returned,
}

impl Install {
    /// An install of `program` (its generation is the install's)
    pub fn new(program: Box<Program>) -> Self {
        Self {
            generation: program.generation(),
            program: Some(program),
            add: Vec::new(),
            replace: Vec::new(),
            remove: Vec::new(),
            producers: Vec::new(),
            remove_producers: Vec::new(),
            relocate: None,
            map: None,
            returned: Returned::default(),
        }
    }

    /// An install with only a new MIDI map (following the selection, a bank change, a mapping edited: I-21.2).
    /// `generation` is the generation of the last program sent (the map refers to its slots)
    pub fn map_only(generation: ProgramGen, map: Box<MidiMap>) -> Self {
        Self {
            generation,
            program: None,
            add: Vec::new(),
            replace: Vec::new(),
            remove: Vec::new(),
            producers: Vec::new(),
            remove_producers: Vec::new(),
            relocate: None,
            map: Some(map),
            returned: Returned::default(),
        }
    }

    /// An install with only rings to add or remove (a MIDI input that appeared or went away, I-7.1): no program, no
    /// map. `generation` is the generation of the last program sent
    pub fn producers_only(generation: ProgramGen) -> Self {
        Self {
            generation,
            program: None,
            add: Vec::new(),
            replace: Vec::new(),
            remove: Vec::new(),
            producers: Vec::new(),
            remove_producers: Vec::new(),
            relocate: None,
            map: None,
            returned: Returned::default(),
        }
    }

    /// A new MIDI map with this install (the old one comes back in [`Returned::map`])
    pub fn set_map(&mut self, map: Box<MidiMap>) -> &mut Self {
        self.map = Some(map);
        self
    }

    /// Whether this install carries a program (a map-only install does not)
    pub fn has_program(&self) -> bool {
        self.program.is_some()
    }

    /// A new processor at a free index
    pub fn add(&mut self, node: NodeIndex, core: Box<NodeCore>) -> &mut Self {
        self.add.push((node, core));
        self
    }

    /// A new processor for an existing node (R18). It adopts the old one's state if it can; if not
    /// ([`crate::Adopted::No`]) and `tail` names an index the program has as a tail node, the old processor moves
    /// there with all its notes released and keeps sounding up to 2 s (U32); otherwise it is returned
    pub fn replace(&mut self, node: NodeIndex, core: Box<NodeCore>, tail: Option<NodeIndex>) -> &mut Self {
        self.replace.push(Replacement { node, core, tail });
        self
    }

    /// Takes a node's processor out (it comes back in [`Returned::processors`]; the index is free after that)
    pub fn remove(&mut self, node: NodeIndex) -> &mut Self {
        self.remove.push(node);
        self
    }

    /// A new command ring (a MIDI input that appeared)
    pub fn add_producer(&mut self, receiver: CommandReceiver) -> &mut Self {
        self.producers.push(receiver);
        self
    }

    /// Takes a command ring out (its receiver comes back)
    pub fn remove_producer(&mut self, producer: ProducerIndex) -> &mut Self {
        self.remove_producers.push(producer);
        self
    }

    /// The tempo map changed while playing (H9)
    pub fn relocate(&mut self, relocate: Relocate) -> &mut Self {
        self.relocate = Some(relocate);
        self
    }

    /// Reserves room for everything the audio thread may put back (called by `EngineControl::install`)
    pub(crate) fn reserve_returns(&mut self) {
        let r = &mut self.returned;
        r.generation = self.generation;
        r.processors.reserve_exact(self.add.len() + 2 * self.replace.len() + self.remove.len());
        r.receivers.reserve_exact(self.producers.len() + self.remove_producers.len());
        r.freed.reserve_exact(self.remove.len());
    }
}
