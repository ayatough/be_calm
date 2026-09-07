//! User configuration: allowed apps, session length, distraction tweaks.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Hard cap on the number of allowed applications per session.
pub const MAX_ALLOWED_APPS: usize = 3;

/// Default phrase the user must type to end a session early.
pub const DEFAULT_EXIT_PHRASE: &str = "今は集中する時間なのに、本当に抜ける必要がある";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AllowedApp {
    /// Display name (usually the exe file stem or window title).
    pub name: String,
    /// Full path to the executable.
    pub path: PathBuf,
}

impl AllowedApp {
    pub fn from_path(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let name = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string_lossy().into_owned());
        Self { name, path }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub allowed_apps: Vec<AllowedApp>,
    pub session_minutes: u32,
    pub hide_taskbar: bool,
    pub hide_desktop_icons: bool,
    /// Kill disallowed processes even if they never show a window. Off by
    /// default so background helpers (cloud sync, updaters) are left alone.
    pub block_windowless: bool,
    pub exit_phrase: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            allowed_apps: Vec::new(),
            session_minutes: 50,
            hide_taskbar: true,
            hide_desktop_icons: true,
            block_windowless: false,
            exit_phrase: DEFAULT_EXIT_PHRASE.to_string(),
        }
    }
}

#[derive(Debug)]
pub enum ConfigError {
    Io(std::io::Error),
    Parse(toml::de::Error),
    Serialize(toml::ser::Error),
    TooManyApps,
    Duplicate,
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::Io(e) => write!(f, "I/O error: {e}"),
            ConfigError::Parse(e) => write!(f, "config parse error: {e}"),
            ConfigError::Serialize(e) => write!(f, "config serialize error: {e}"),
            ConfigError::TooManyApps => write!(f, "at most {MAX_ALLOWED_APPS} apps can be allowed"),
            ConfigError::Duplicate => write!(f, "that app is already in the list"),
        }
    }
}

impl std::error::Error for ConfigError {}

impl Config {
    /// Add an app, enforcing the cap and rejecting duplicates (case-insensitive path match).
    pub fn add_app(&mut self, app: AllowedApp) -> Result<(), ConfigError> {
        if self.allowed_apps.len() >= MAX_ALLOWED_APPS {
            return Err(ConfigError::TooManyApps);
        }
        let key = normalize_path(&app.path);
        if self
            .allowed_apps
            .iter()
            .any(|a| normalize_path(&a.path) == key)
        {
            return Err(ConfigError::Duplicate);
        }
        self.allowed_apps.push(app);
        Ok(())
    }

    pub fn remove_app(&mut self, index: usize) {
        if index < self.allowed_apps.len() {
            self.allowed_apps.remove(index);
        }
    }

    pub fn can_start(&self) -> bool {
        !self.allowed_apps.is_empty() && self.session_minutes > 0
    }

    pub fn to_toml(&self) -> Result<String, ConfigError> {
        toml::to_string_pretty(self).map_err(ConfigError::Serialize)
    }

    pub fn from_toml(s: &str) -> Result<Self, ConfigError> {
        let mut cfg: Config = toml::from_str(s).map_err(ConfigError::Parse)?;
        cfg.allowed_apps.truncate(MAX_ALLOWED_APPS);
        if cfg.exit_phrase.trim().is_empty() {
            cfg.exit_phrase = DEFAULT_EXIT_PHRASE.to_string();
        }
        Ok(cfg)
    }

    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        match std::fs::read_to_string(path) {
            Ok(s) => Self::from_toml(&s),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(ConfigError::Io(e)),
        }
    }

    pub fn save(&self, path: &Path) -> Result<(), ConfigError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(ConfigError::Io)?;
        }
        std::fs::write(path, self.to_toml()?).map_err(ConfigError::Io)
    }
}

/// Directory where be_calm keeps its config, session lock and log.
pub fn data_dir() -> PathBuf {
    directories::ProjectDirs::from("", "", "be_calm")
        .map(|d| d.config_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."))
}

pub fn config_path() -> PathBuf {
    data_dir().join("config.toml")
}

/// Canonical, case-insensitive, slash-normalized form of a Windows path
/// (also strips the `\\?\` verbatim prefix). Used for equality checks only.
pub fn normalize_path(p: &Path) -> String {
    let s = p.to_string_lossy();
    let s = s.strip_prefix(r"\\?\").unwrap_or(&s);
    s.replace('/', "\\").to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_app_enforces_cap_and_duplicates() {
        let mut c = Config::default();
        c.add_app(AllowedApp::from_path(r"C:\a\one.exe")).unwrap();
        assert!(matches!(
            c.add_app(AllowedApp::from_path(r"c:/A/ONE.EXE")),
            Err(ConfigError::Duplicate)
        ));
        c.add_app(AllowedApp::from_path(r"C:\a\two.exe")).unwrap();
        c.add_app(AllowedApp::from_path(r"C:\a\three.exe")).unwrap();
        assert!(matches!(
            c.add_app(AllowedApp::from_path(r"C:\a\four.exe")),
            Err(ConfigError::TooManyApps)
        ));
        assert_eq!(c.allowed_apps.len(), 3);
    }

    #[test]
    fn from_path_uses_file_stem_as_name() {
        let a = AllowedApp::from_path(r"C:\Program Files\Foo\Foo Editor.exe");
        assert_eq!(a.name, "Foo Editor");
    }

    #[test]
    fn toml_roundtrip() {
        let mut c = Config::default();
        c.add_app(AllowedApp::from_path(r"C:\a\one.exe")).unwrap();
        c.session_minutes = 25;
        let s = c.to_toml().unwrap();
        let back = Config::from_toml(&s).unwrap();
        assert_eq!(c, back);
    }

    #[test]
    fn missing_fields_fall_back_to_defaults() {
        let c = Config::from_toml("session_minutes = 10\n").unwrap();
        assert_eq!(c.session_minutes, 10);
        assert!(c.hide_taskbar);
        assert_eq!(c.exit_phrase, DEFAULT_EXIT_PHRASE);
        assert!(!c.can_start());
    }

    #[test]
    fn oversized_list_in_file_is_truncated() {
        let s = r#"
[[allowed_apps]]
name = "a"
path = "C:\\a.exe"
[[allowed_apps]]
name = "b"
path = "C:\\b.exe"
[[allowed_apps]]
name = "c"
path = "C:\\c.exe"
[[allowed_apps]]
name = "d"
path = "C:\\d.exe"
"#;
        let c = Config::from_toml(s).unwrap();
        assert_eq!(c.allowed_apps.len(), MAX_ALLOWED_APPS);
    }

    #[test]
    fn load_missing_file_gives_default() {
        let dir = std::env::temp_dir().join(format!("be_calm_test_{}", std::process::id()));
        let p = dir.join("nope").join("config.toml");
        assert_eq!(Config::load(&p).unwrap(), Config::default());
        let c = Config {
            session_minutes: 7,
            ..Config::default()
        };
        c.save(&p).unwrap();
        assert_eq!(Config::load(&p).unwrap().session_minutes, 7);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn normalize_strips_verbatim_prefix() {
        assert_eq!(
            normalize_path(Path::new(r"\\?\C:\Foo/Bar.EXE")),
            r"c:\foo\bar.exe"
        );
    }
}
