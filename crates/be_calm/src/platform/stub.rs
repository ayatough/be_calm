//! Non-Windows stubs so the binary compiles (and the UI can be poked at) elsewhere.

use super::{Foreground, WindowedApp};
use std::path::{Path, PathBuf};

pub mod process {
    use super::*;

    #[derive(Debug, Clone)]
    pub struct ProcInfo {
        pub pid: u32,
        pub parent: u32,
        pub exe: Option<PathBuf>,
    }

    pub fn list() -> Vec<ProcInfo> {
        Vec::new()
    }
    pub fn terminate(_pid: u32) -> Result<(), String> {
        Ok(())
    }
    pub fn windowed_apps() -> Vec<WindowedApp> {
        Vec::new()
    }
    pub fn pids_with_visible_windows() -> std::collections::HashSet<u32> {
        Default::default()
    }
    pub fn foreground() -> Option<Foreground> {
        None
    }
    pub fn minimize(_hwnd: isize) {}
    pub fn launch(path: &Path) -> Result<(), String> {
        std::process::Command::new(path)
            .spawn()
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
    pub fn system_roots() -> Vec<PathBuf> {
        vec![PathBuf::from("/usr"), PathBuf::from("/bin")]
    }
}

pub mod shell {
    pub fn hide_taskbar() {}
    pub fn hide_desktop_icons() {}
    pub fn restore_all() {}
    pub fn write_recovery_marker(_taskbar_state: Option<u32>) {}
    pub fn clear_recovery_marker() {}
    pub fn recovery_marker_exists() -> bool {
        false
    }
}
