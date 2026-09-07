//! Session history: one JSON line per finished session.

use crate::session::SessionSummary;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    /// Local wall-clock start, ISO 8601 with offset (e.g. `2026-09-07T22:30:00+09:00`).
    pub started: String,
    pub planned_secs: u64,
    pub elapsed_secs: u64,
    pub blocked: u64,
    pub ended_early: bool,
    pub apps: Vec<String>,
}

impl Record {
    pub fn from_summary(started: String, s: &SessionSummary, apps: Vec<String>) -> Self {
        Self {
            started,
            planned_secs: s.planned.as_secs(),
            elapsed_secs: s.elapsed.as_secs(),
            blocked: s.blocked_count as u64,
            ended_early: s.ended_early,
            apps,
        }
    }

    pub fn elapsed(&self) -> Duration {
        Duration::from_secs(self.elapsed_secs)
    }

    /// `YYYY-MM-DD` part of `started`.
    pub fn day(&self) -> &str {
        self.started.get(..10).unwrap_or(&self.started)
    }
}

pub fn append(path: &Path, r: &Record) -> std::io::Result<()> {
    use std::io::Write;
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    let line = serde_json::to_string(r).map_err(std::io::Error::other)?;
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    writeln!(f, "{line}")
}

/// Load all records, oldest first. Malformed lines are skipped.
pub fn load(path: &Path) -> Vec<Record> {
    match std::fs::read_to_string(path) {
        Ok(s) => s
            .lines()
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect(),
        Err(_) => Vec::new(),
    }
}

/// Total focused time on a given `YYYY-MM-DD`.
pub fn total_for_day(records: &[Record], day: &str) -> Duration {
    records
        .iter()
        .filter(|r| r.day() == day)
        .map(Record::elapsed)
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(started: &str, secs: u64) -> Record {
        Record {
            started: started.into(),
            planned_secs: 3000,
            elapsed_secs: secs,
            blocked: 1,
            ended_early: false,
            apps: vec!["a".into()],
        }
    }

    #[test]
    fn roundtrip_and_day_totals() {
        let dir = std::env::temp_dir().join(format!("be_calm_hist_{}", std::process::id()));
        let p = dir.join("history.jsonl");
        append(&p, &rec("2026-09-07T10:00:00+09:00", 600)).unwrap();
        append(&p, &rec("2026-09-07T13:00:00+09:00", 900)).unwrap();
        append(&p, &rec("2026-09-06T13:00:00+09:00", 100)).unwrap();
        std::fs::OpenOptions::new()
            .append(true)
            .open(&p)
            .and_then(|mut f| {
                use std::io::Write;
                writeln!(f, "not json")
            })
            .unwrap();
        let all = load(&p);
        assert_eq!(all.len(), 3);
        assert_eq!(total_for_day(&all, "2026-09-07"), Duration::from_secs(1500));
        assert_eq!(total_for_day(&all, "2026-09-06"), Duration::from_secs(100));
        assert_eq!(total_for_day(&all, "2026-09-05"), Duration::ZERO);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn missing_file_is_empty() {
        assert!(load(Path::new("/definitely/not/here.jsonl")).is_empty());
    }
}
