//! Platform-independent core of be_calm.
//!
//! Everything in this crate is testable on any OS. The Windows-specific
//! process watching and shell tweaks live in the `be_calm` binary crate.

pub mod config;
pub mod lineage;
pub mod policy;
pub mod session;

pub use config::{AllowedApp, Config, MAX_ALLOWED_APPS};
pub use lineage::{foreground_allowed, Decision, Lineage, Origin};
pub use policy::{Policy, Verdict};
pub use session::{BlockEvent, Session, SessionSummary};
