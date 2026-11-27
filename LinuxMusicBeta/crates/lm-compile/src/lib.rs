//! LinuxMusic: the compiler from the project tree to the render program (interfaces.md, I-5.4).
//!
//! - [`Compiler`]: on its own thread (`Role::Compile`); `submit` keeps only the newest root (R17), `poll` gives the
//!   results in order, `returned` frees indices and slots (S6), `ready` takes a processor the session built (T6),
//!   `from_table`/`rebuild` start over from the installed table (H14);
//! - [`CompilerCore`]: the same, synchronous (tests, the export);
//! - the translation (`core.rs`): tracks, outputs, the desk with its rack, inserts at their points, sends and
//!   sidechains, solo, PDC per path, monitoring outside the PDC, metronome and preview bus, the parameter slots;
//!   items and curves (`items.rs`) reused while their tree node is the same (V8).
//!
//! The compiler never builds a processor: a new node waits (silent) and a replaced node keeps its old processor
//! until the new one is ready (T7), then goes to a tail node if it cannot adopt (U32). It is the only one that hands
//! out node indices and slots, and never reuses one before the install that freed it has come back.

#![forbid(unsafe_code)]

mod alloc;
mod catalog;
mod core;
mod items;
mod keys;
mod map;
mod rack;
mod read;
pub mod stretch;
mod thread;
mod types;

pub use crate::catalog::{NoResources, PluginCatalog, PluginShape, ResourceState, Resources};
pub use crate::core::{CompilerConfig, CompilerCore, TAIL_NET_SECONDS, Usage};
pub use crate::items::{Made, STREAM_SECONDS, StreamRegistration, WaitingItem};
pub use crate::map::{
    CompiledMap, MapInput, PortAlias, Resolved, Selection, compile_map, find, mapping_id, resolve_node,
};
pub use crate::rack::{InsertPoint, RackParam, Stage, rack_params};
pub use crate::read::id_number;
pub use crate::thread::{CompileActivity, Compiler};
pub use crate::types::{
    Audition, Build, BuildRequest, Compiled, Environment, InstalledProcessor, InstalledTable, InstalledTail, MetronomeWhen,
    NodeInfo, NodeKey, Route, Ticket,
};
