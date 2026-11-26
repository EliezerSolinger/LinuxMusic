//! The project document (interfaces.md, I-3; arquitetura-nova.md, pieces 2 and 2a).
//!
//! - [`Document`]: the current root (an immutable [`lm_state::Value`]) with its two persistent indices (id →
//!   location, and who points to each id), the history of the states that existed (each step with its diff and
//!   inverse, U18), `meta` (the global id counter, also for note numbers, and the uuid) that undo never restores;
//! - [`Edit`]: the changes inside one all-or-nothing transaction: `set` at any depth, `insert`, `remove` (with the
//!   `OnRemove` rules), `move_to` (final index), notes by number, paste with new ids (H13); then the domain
//!   [`Rule`]s (the instrument's ports and the track's outputs, routing cycles, folders, the engine's limits...);
//! - [`Diff`]: what changed, merged by the final state; [`Feed`]: the pull model of change notices (I-3.6);
//! - gestures (I-3.7: no step while open, G1–G3), the journal ([`JournalPatch`], I-4) and heavy edits ([`Patch`],
//!   H7);
//! - [`schemas`]: the project's node types, and the readers ([`TrackRef`], [`ChannelRef`]...).
//!
//! Errors are [`lm_base::Error`] values with keys under `doc.` (`doc.routing.cycle`, `doc.remove.refused`,
//! `doc.gesture.target_gone`, `doc.limits.nodes`...).

mod diff;
mod document;
mod edit;
mod index;
mod migrate;
mod ops;
mod refs;
mod rules;
pub mod schemas;
pub mod tuning;

pub use diff::{Changed, Diff, NoteChanges, Origin};
pub use document::{Delivered, Document, Feed, GestureId, Opened, Options, Topic, new_uuid};
pub use edit::{Clip, Edit, param_key, param_path};
pub use index::{Index, Loc, RefFrom, RelPath};
pub use ops::{JournalOp, JournalPatch, Patch, PatchOp, replay};
pub use refs::{AutomationRef, ChannelRef, ItemRef, LaneRef, MasterRef, OutputRef, PluginRef, SendRef, TrackRef};
pub use rules::{
    Limits, Port, Ports, PortsFn, Rule, RuleFn, RuleKind, Touched, Usage, UsageFn, default_usage, project_rules,
    valid_bpm,
};
pub use schemas::{MASTER, PROJECT, project_registry, register_project_schemas};
