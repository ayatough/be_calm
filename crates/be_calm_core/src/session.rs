//! Focus session state: timer, block log, early-exit challenge.

use std::path::PathBuf;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockEvent {
    /// Time since the session started.
    pub at: Duration,
    pub exe: PathBuf,
}

#[derive(Debug, Clone)]
pub struct Session {
    started: Instant,
    duration: Duration,
    blocks: Vec<BlockEvent>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionSummary {
    pub planned: Duration,
    pub elapsed: Duration,
    pub blocked_count: usize,
    pub ended_early: bool,
}

impl Session {
    pub fn start(now: Instant, duration: Duration) -> Self {
        Self {
            started: now,
            duration,
            blocks: Vec::new(),
        }
    }

    pub fn duration(&self) -> Duration {
        self.duration
    }

    pub fn elapsed(&self, now: Instant) -> Duration {
        now.saturating_duration_since(self.started)
    }

    pub fn remaining(&self, now: Instant) -> Duration {
        self.duration.saturating_sub(self.elapsed(now))
    }

    pub fn is_over(&self, now: Instant) -> bool {
        self.elapsed(now) >= self.duration
    }

    /// Fraction of the session completed, in `0.0..=1.0`.
    pub fn progress(&self, now: Instant) -> f32 {
        if self.duration.is_zero() {
            return 1.0;
        }
        (self.elapsed(now).as_secs_f32() / self.duration.as_secs_f32()).clamp(0.0, 1.0)
    }

    pub fn record_block(&mut self, now: Instant, exe: PathBuf) {
        self.blocks.push(BlockEvent {
            at: self.elapsed(now),
            exe,
        });
    }

    pub fn blocks(&self) -> &[BlockEvent] {
        &self.blocks
    }

    pub fn summary(&self, now: Instant, ended_early: bool) -> SessionSummary {
        SessionSummary {
            planned: self.duration,
            elapsed: self.elapsed(now).min(self.duration),
            blocked_count: self.blocks.len(),
            ended_early,
        }
    }
}

/// The early-exit challenge: the user must type the phrase exactly
/// (surrounding whitespace ignored, Unicode-aware comparison).
pub fn exit_challenge_passed(input: &str, phrase: &str) -> bool {
    let phrase = phrase.trim();
    !phrase.is_empty() && input.trim() == phrase
}

/// Format a duration as `mm:ss` (or `h:mm:ss` beyond an hour).
pub fn format_clock(d: Duration) -> String {
    let total = d.as_secs();
    let (h, m, s) = (total / 3600, (total % 3600) / 60, total % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m:02}:{s:02}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timer_math() {
        let t0 = Instant::now();
        let s = Session::start(t0, Duration::from_secs(600));
        assert_eq!(s.remaining(t0), Duration::from_secs(600));
        let t1 = t0 + Duration::from_secs(150);
        assert_eq!(s.remaining(t1), Duration::from_secs(450));
        assert!((s.progress(t1) - 0.25).abs() < 1e-6);
        assert!(!s.is_over(t1));
        let t2 = t0 + Duration::from_secs(600);
        assert!(s.is_over(t2));
        assert_eq!(s.remaining(t2 + Duration::from_secs(5)), Duration::ZERO);
        assert_eq!(s.progress(t2 + Duration::from_secs(5)), 1.0);
    }

    #[test]
    fn blocks_are_recorded_with_offsets() {
        let t0 = Instant::now();
        let mut s = Session::start(t0, Duration::from_secs(60));
        s.record_block(t0 + Duration::from_secs(3), PathBuf::from(r"C:\x\game.exe"));
        assert_eq!(s.blocks().len(), 1);
        assert_eq!(s.blocks()[0].at, Duration::from_secs(3));
        let sum = s.summary(t0 + Duration::from_secs(10), true);
        assert_eq!(sum.blocked_count, 1);
        assert_eq!(sum.elapsed, Duration::from_secs(10));
        assert!(sum.ended_early);
    }

    #[test]
    fn summary_elapsed_is_capped_at_planned() {
        let t0 = Instant::now();
        let s = Session::start(t0, Duration::from_secs(60));
        let sum = s.summary(t0 + Duration::from_secs(90), false);
        assert_eq!(sum.elapsed, Duration::from_secs(60));
    }

    #[test]
    fn challenge_requires_exact_phrase() {
        assert!(exit_challenge_passed("  本当に抜ける  ", "本当に抜ける"));
        assert!(!exit_challenge_passed("本当に抜けろ", "本当に抜ける"));
        assert!(!exit_challenge_passed("", ""));
        assert!(!exit_challenge_passed("x", "   "));
    }

    #[test]
    fn clock_formatting() {
        assert_eq!(format_clock(Duration::from_secs(0)), "00:00");
        assert_eq!(format_clock(Duration::from_secs(65)), "01:05");
        assert_eq!(format_clock(Duration::from_secs(3661)), "1:01:01");
    }
}
