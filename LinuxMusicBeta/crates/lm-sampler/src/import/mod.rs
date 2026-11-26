//! The importers: each reads a format into an [`crate::Instrument`], away from the audio thread. None keeps the file
//! open: what the instrument needs is copied into its zones' samples.

pub mod audio;
pub mod avl;
pub mod drums;
pub mod gig;
pub mod pat;
pub mod sf2;
pub mod sfz;
