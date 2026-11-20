//! LinuxMusic: third-party plugins out of the core's process (interfaces.md, I-9.6; arquitetura-nova.md, items 7
//! and 13; decisoes-autonomas.md P-01 to P-05). The program's own plugins run in the process, in the render path
//! (lowest latency); only third-party code is isolated, in `lm-plugin-host` helper processes.
//!
//! This crate is the core's side, without any plugin format library:
//! - [`layout`]: the block shared with a helper (audio, events, transport, the plugin's answers) and its handshake;
//! - [`proxy::Proxy`]: the `Processor` the renderer runs, with a deadline per block (silence and a notice when it is
//!   missed; a hung plugin is killed); no allocation, no lock;
//! - [`hosted`]: starting a helper, its control channel (state, window), and the supervisor;
//! - [`protocol`]: the control channel's messages (versioned);
//! - [`mod@external`]: a scanned plugin as an external plugin of the registry (its id, schema, ports and build);
//! - [`mod@scan`]: the scan, each plugin probed in a helper of its own with a deadline, the ones that crash or hang it
//!   listed as blocked (H15).

#![forbid(unsafe_code)]

pub mod external;
pub mod hosted;
pub mod layout;
pub mod protocol;
pub mod proxy;
pub mod scan;

pub use external::{OnStart, external, plugin_id};
pub use hosted::{HostConfig, Hosted, LoadRequest, Ready, Saved, start};
pub use proxy::{Deadline, HANG_BLOCKS, Proxy, ProxyState};
pub use scan::{Blocked, ScanReport, Scanned, ScannedParam, scan, scanned_from};
