//! Decides whether a newly started process may live.
//!
//! Rules, in order:
//! 1. be_calm itself is always allowed.
//! 2. Anything under a system root (e.g. `C:\Windows`) is allowed — the OS
//!    constantly spawns helpers there and killing them breaks the desktop.
//! 3. An explicitly allowed app matches by full path (case-insensitive).
//!    For Microsoft Store apps (installed under `WindowsApps`) the versioned
//!    directory changes on every update, so those match by file name only.
//! 4. Anything else inside an allowed app's directory tree is allowed too
//!    (`AllowSameDir`). Apps ship helpers next to themselves (browser
//!    renderers, terminals, updaters) and the parent/child chain is not
//!    always observable, so this is what keeps allowed apps working.
//! 5. Everything else is blocked.

use crate::config::{normalize_path, AllowedApp};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// The process is be_calm itself.
    AllowSelf,
    /// The process lives under a system root.
    AllowSystem,
    /// The process is in the user's allowlist.
    AllowListed,
    /// The process lives inside an allowed app's directory tree.
    AllowSameDir,
    /// Kill it.
    Block,
}

impl Verdict {
    pub fn is_blocked(self) -> bool {
        matches!(self, Verdict::Block)
    }
}

#[derive(Debug, Clone)]
struct Rule {
    /// Normalized full path.
    full: String,
    /// Normalized file name, used when `by_name` is set.
    file_name: String,
    by_name: bool,
    /// Normalized parent directory with trailing backslash.
    dir: String,
}

#[derive(Debug, Clone)]
pub struct Policy {
    rules: Vec<Rule>,
    system_roots: Vec<String>,
    self_exe: Option<String>,
}

fn file_name_of(p: &Path) -> String {
    crate::config::file_name_of(p).to_lowercase()
}

fn is_store_app(normalized: &str) -> bool {
    normalized.contains(r"\windowsapps\")
}

impl Policy {
    /// `system_roots` are directories whose contents are always allowed
    /// (typically `%SystemRoot%`). `self_exe` is be_calm's own executable.
    pub fn new(allowed: &[AllowedApp], system_roots: &[PathBuf], self_exe: Option<&Path>) -> Self {
        let rules = allowed
            .iter()
            .map(|a| {
                let full = normalize_path(&a.path);
                let dir = match full.rfind('\\') {
                    Some(i) => full[..=i].to_string(),
                    None => String::new(),
                };
                Rule {
                    by_name: is_store_app(&full),
                    file_name: file_name_of(&a.path),
                    dir,
                    full,
                }
            })
            .collect();
        let system_roots = system_roots
            .iter()
            .map(|r| {
                let mut s = normalize_path(r);
                if !s.ends_with('\\') {
                    s.push('\\');
                }
                s
            })
            .collect();
        Self {
            rules,
            system_roots,
            self_exe: self_exe.map(normalize_path),
        }
    }

    pub fn verdict(&self, exe: &Path) -> Verdict {
        let n = normalize_path(exe);
        if self.self_exe.as_deref() == Some(n.as_str()) {
            return Verdict::AllowSelf;
        }
        if self.system_roots.iter().any(|r| n.starts_with(r.as_str())) {
            return Verdict::AllowSystem;
        }
        let name = file_name_of(exe);
        for rule in &self.rules {
            if rule.full == n || (rule.by_name && rule.file_name == name) {
                return Verdict::AllowListed;
            }
        }
        for rule in &self.rules {
            if !rule.dir.is_empty() && n.starts_with(&rule.dir) {
                return Verdict::AllowSameDir;
            }
        }
        Verdict::Block
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy() -> Policy {
        Policy::new(
            &[
                AllowedApp::from_path(r"C:\Program Files\Editor\editor.exe"),
                AllowedApp::from_path(
                    r"C:\Program Files\WindowsApps\Microsoft.WindowsNotepad_11.0_x64__8wekyb3d8bbwe\Notepad\Notepad.exe",
                ),
            ],
            &[PathBuf::from(r"C:\Windows")],
            Some(Path::new(r"C:\Users\me\be_calm.exe")),
        )
    }

    #[test]
    fn self_is_allowed() {
        assert_eq!(
            policy().verdict(Path::new(r"c:\users\ME\BE_CALM.EXE")),
            Verdict::AllowSelf
        );
    }

    #[test]
    fn system_root_is_allowed_but_not_lookalikes() {
        let p = policy();
        assert_eq!(
            p.verdict(Path::new(r"C:\Windows\System32\svchost.exe")),
            Verdict::AllowSystem
        );
        assert_eq!(
            p.verdict(Path::new(r"C:\WINDOWS\explorer.exe")),
            Verdict::AllowSystem
        );
        // "C:\Windows2\..." must not match "C:\Windows\"
        assert_eq!(
            p.verdict(Path::new(r"C:\Windows2\game.exe")),
            Verdict::Block
        );
    }

    #[test]
    fn listed_app_matches_by_full_path_case_insensitively() {
        let p = policy();
        assert_eq!(
            p.verdict(Path::new(r"c:\program files\editor\EDITOR.exe")),
            Verdict::AllowListed
        );
        // same file name elsewhere is not enough for a regular app
        assert_eq!(p.verdict(Path::new(r"D:\evil\editor.exe")), Verdict::Block);
    }

    #[test]
    fn helpers_inside_an_allowed_apps_directory_are_allowed() {
        let p = policy();
        assert_eq!(
            p.verdict(Path::new(r"C:\Program Files\Editor\helper.exe")),
            Verdict::AllowSameDir
        );
        assert_eq!(
            p.verdict(Path::new(r"C:\Program Files\Editor\bin\lsp.exe")),
            Verdict::AllowSameDir
        );
        // sibling directory with a shared prefix must not match
        assert_eq!(
            p.verdict(Path::new(r"C:\Program Files\EditorGame\game.exe")),
            Verdict::Block
        );
        // a store app's siblings are covered by its (versioned) directory too
        let helper = r"C:\Program Files\WindowsApps\Microsoft.WindowsNotepad_11.0_x64__8wekyb3d8bbwe\Notepad\helper.exe";
        assert_eq!(p.verdict(Path::new(helper)), Verdict::AllowSameDir);
    }

    #[test]
    fn store_apps_match_by_name_across_versions() {
        let p = policy();
        let updated = r"C:\Program Files\WindowsApps\Microsoft.WindowsNotepad_12.3_x64__8wekyb3d8bbwe\Notepad\Notepad.exe";
        assert_eq!(p.verdict(Path::new(updated)), Verdict::AllowListed);
    }

    #[test]
    fn everything_else_is_blocked() {
        let p = policy();
        assert!(p
            .verdict(Path::new(r"C:\Program Files\Steam\steam.exe"))
            .is_blocked());
        assert!(p
            .verdict(Path::new(r"C:\Users\me\AppData\Local\Discord\Discord.exe"))
            .is_blocked());
    }

    #[test]
    fn verbatim_prefix_paths_are_handled() {
        let p = policy();
        assert_eq!(
            p.verdict(Path::new(r"\\?\C:\Windows\notepad.exe")),
            Verdict::AllowSystem
        );
    }
}
