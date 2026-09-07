//! OS integration. Only Windows is implemented; other targets get inert
//! stubs so the core crate and GUI still compile for CI on Linux.

use std::path::PathBuf;

/// A running process that owns a visible top-level window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowedApp {
    pub pid: u32,
    pub title: String,
    pub exe: PathBuf,
}

#[cfg(windows)]
pub mod windows;
#[cfg(windows)]
pub use self::windows::{process, shell};

#[cfg(not(windows))]
pub mod stub;
#[cfg(not(windows))]
pub use self::stub::{process, shell};

pub mod watcher;
