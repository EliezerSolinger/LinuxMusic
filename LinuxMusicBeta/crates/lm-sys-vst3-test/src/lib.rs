//! The VST3 test plugins of `lm_sys_vst3::testing`, as a shared library: `cargo` builds the `cdylib` next to the
//! tests of a crate that depends on this one (the plugin host's), which open it as a `.vst3`.
//!
//! The only unsafe here is the exported entry points (`#[unsafe(no_mangle)]`) the VST3 ABI asks for.

lm_sys_vst3::export_test_plugins!();
