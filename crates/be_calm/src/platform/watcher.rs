//! Background thread that enforces the session:
//!
//! - polls the process table and kills newcomers the policy rejects (only
//!   processes that appear *after* the watcher starts; what the user already
//!   had open is left running), and
//! - polls the foreground window and minimizes it if it belongs to an app
//!   that is not allowed (this is what makes Alt+Tab / Win key useless for
//!   getting at an already-open distraction).
//!
//! Children of allowed apps inherit permission (see `be_calm_core::lineage`).

use super::process::{self, ProcInfo};
use be_calm_core::{foreground_allowed, Lineage, Origin, Policy, Verdict};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

pub const POLL_INTERVAL: Duration = Duration::from_millis(100);
/// Minimum gap between two foreground notices for the same window, so a
/// window fighting to come back does not flood the UI.
const FOREGROUND_NOTICE_GAP: Duration = Duration::from_secs(3);
/// Minimum gap between two Ctrl+W presses (a tab needs a moment to close).
const TITLE_CLOSE_GAP: Duration = Duration::from_millis(700);
/// After this many Ctrl+W presses without effect, minimize the window instead.
const TITLE_CLOSE_MAX_TRIES: u32 = 3;

#[derive(Debug, Clone)]
pub enum WatchEvent {
    Blocked {
        pid: u32,
        exe: PathBuf,
    },
    /// We decided to block but the kill failed (already gone, or access denied).
    KillFailed {
        pid: u32,
        exe: PathBuf,
        reason: String,
    },
    /// A disallowed window reached the foreground and was minimized.
    Foreground {
        pid: u32,
        exe: PathBuf,
    },
    /// An allowed window showed a blocked keyword in its title; Ctrl+W was sent.
    TitleBlocked {
        keyword: String,
        title: String,
    },
}

#[derive(Debug, Clone, Copy)]
pub struct WatchOptions {
    /// Kill rejected processes immediately instead of waiting for a window.
    pub block_windowless: bool,
    /// Minimize disallowed foreground windows.
    pub guard_foreground: bool,
}

/// Non-`Copy` part of the options.
#[derive(Debug, Clone, Default)]
pub struct TitleRules {
    pub keywords: Vec<String>,
}

pub struct Watcher {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
    rx: Receiver<WatchEvent>,
}

impl Watcher {
    pub fn start(policy: Policy, opts: WatchOptions, titles: TitleRules) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel();
        let stop2 = stop.clone();
        let handle = std::thread::Builder::new()
            .name("be_calm-watcher".into())
            .spawn(move || run(policy, opts, titles, stop2, tx))
            .expect("spawn watcher thread");
        Self {
            stop,
            handle: Some(handle),
            rx,
        }
    }

    pub fn try_recv(&self) -> Option<WatchEvent> {
        self.rx.try_recv().ok()
    }

    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        self.stop();
    }
}

struct Judge {
    policy: Policy,
    lineage: Lineage,
    opts: WatchOptions,
    titles: TitleRules,
    /// Last time Ctrl+W was sent, so a stubborn page is not spammed.
    last_close_tab: Option<Instant>,
    /// (hwnd, consecutive Ctrl+W attempts) for the current blocked title.
    title_attempts: Option<(isize, u32)>,
    /// Rejected processes waiting to show a window before we kill them.
    pending: HashMap<u32, PathBuf>,
    /// Last time we reported a given window being pushed back.
    last_notice: HashMap<isize, Instant>,
    /// Last foreground window seen (for change logging only).
    last_fg: isize,
    tx: Sender<WatchEvent>,
}

impl Judge {
    fn judge(&mut self, p: &ProcInfo) {
        let Some(exe) = p.exe.as_ref() else {
            // Could not read the image path (protected/system process). Leave it.
            log::debug!("pid {} has unreadable path; skipping", p.pid);
            return;
        };
        let decision = self
            .lineage
            .decide(p.pid, p.parent, self.policy.verdict(exe));
        log::debug!(
            "new pid {} (parent {}) {} -> {:?}",
            p.pid,
            p.parent,
            exe.display(),
            decision
        );
        if !decision.is_blocked() {
            return;
        }
        if self.opts.block_windowless {
            self.kill(p.pid, exe);
        } else {
            self.pending.insert(p.pid, exe.clone());
        }
    }

    /// Kill pending processes that have shown a window; forget dead ones.
    fn sweep_pending(&mut self, alive: &HashSet<u32>) {
        self.pending.retain(|pid, _| alive.contains(pid));
        if self.pending.is_empty() {
            return;
        }
        let windowed = process::pids_with_visible_windows();
        let ready: Vec<(u32, PathBuf)> = self
            .pending
            .iter()
            .filter(|(pid, _)| windowed.contains(pid))
            .map(|(pid, exe)| (*pid, exe.clone()))
            .collect();
        for (pid, exe) in ready {
            self.pending.remove(&pid);
            self.kill(pid, &exe);
        }
    }

    fn kill(&self, pid: u32, exe: &Path) {
        match process::terminate(pid) {
            Ok(()) => {
                log::info!("blocked pid {pid} {}", exe.display());
                let _ = self.tx.send(WatchEvent::Blocked {
                    pid,
                    exe: exe.to_path_buf(),
                });
            }
            Err(reason) => {
                log::warn!("failed to kill pid {pid} {}: {reason}", exe.display());
                let _ = self.tx.send(WatchEvent::KillFailed {
                    pid,
                    exe: exe.to_path_buf(),
                    reason,
                });
            }
        }
    }

    /// Push back a foreground window that belongs to a disallowed app (if
    /// enabled), or close a blocked-title tab inside an allowed one.
    fn guard_foreground(&mut self, now: Instant) {
        let Some(fg) = process::foreground() else {
            return;
        };
        if fg.pid == std::process::id() {
            return;
        }
        let Some(exe) = fg.exe.as_ref() else { return };
        let verdict = self.policy.verdict(exe);
        let allowed = foreground_allowed(&self.lineage, fg.pid, verdict);
        if fg.hwnd != self.last_fg {
            self.last_fg = fg.hwnd;
            log::debug!(
                "foreground hwnd {:#x} pid {} {} -> {:?} allowed={allowed}",
                fg.hwnd,
                fg.pid,
                exe.display(),
                verdict
            );
        }
        if allowed {
            self.guard_title(&fg, now);
            return;
        }
        if !self.opts.guard_foreground {
            return;
        }
        process::minimize(fg.hwnd);
        let notify = self
            .last_notice
            .get(&fg.hwnd)
            .is_none_or(|t| now.duration_since(*t) >= FOREGROUND_NOTICE_GAP);
        if notify {
            self.last_notice.insert(fg.hwnd, now);
            log::info!("pushed back foreground pid {} {}", fg.pid, exe.display());
            let _ = self.tx.send(WatchEvent::Foreground {
                pid: fg.pid,
                exe: exe.clone(),
            });
        }
    }
}

impl Judge {
    /// Close the current tab of an allowed window whose title is blocked.
    /// If Ctrl+W does not make the title go away after a few tries (the app
    /// is not a browser), fall back to minimizing the window.
    fn guard_title(&mut self, fg: &super::Foreground, now: Instant) {
        let Some(keyword) = be_calm_core::titles::blocked_keyword(&fg.title, &self.titles.keywords)
        else {
            self.title_attempts = None;
            return;
        };
        if self
            .last_close_tab
            .is_some_and(|t| now.duration_since(t) < TITLE_CLOSE_GAP)
        {
            return;
        }
        self.last_close_tab = Some(now);
        let attempts = match self.title_attempts {
            Some((hwnd, n)) if hwnd == fg.hwnd => n + 1,
            _ => 1,
        };
        self.title_attempts = Some((fg.hwnd, attempts));
        if attempts > TITLE_CLOSE_MAX_TRIES {
            log::info!("blocked title {:?} did not close; minimizing", fg.title);
            process::minimize(fg.hwnd);
            self.title_attempts = None;
        } else {
            log::info!(
                "blocked title {:?} (keyword {keyword}); sending Ctrl+W",
                fg.title
            );
            process::send_close_tab();
        }
        if attempts == 1 {
            let _ = self.tx.send(WatchEvent::TitleBlocked {
                keyword: keyword.to_string(),
                title: fg.title.clone(),
            });
        }
    }
}

fn run(
    policy: Policy,
    opts: WatchOptions,
    titles: TitleRules,
    stop: Arc<AtomicBool>,
    tx: Sender<WatchEvent>,
) {
    // Baseline: everything alive right now is grandfathered in, and every
    // already-running *user* app is trusted so the helpers it keeps spawning
    // (browser tabs, language servers, ...) are left alone — we only want to
    // stop apps the user tries to open from now on. System processes
    // (explorer, shells) are deliberately not trusted: they are how new apps
    // get launched. Already-running allowed apps are `Listed` so their
    // windows may be used; everything else is `Grandfathered` (may spawn,
    // may not be in front).
    let mut lineage = Lineage::new();
    let baseline = process::list();
    for p in &baseline {
        if let Some(exe) = &p.exe {
            match policy.verdict(exe) {
                Verdict::AllowListed | Verdict::AllowSameDir => {
                    lineage.trust(p.pid, Origin::Listed)
                }
                Verdict::Block => lineage.trust(p.pid, Origin::Grandfathered),
                Verdict::AllowSystem | Verdict::AllowSelf => {}
            }
        }
    }
    let mut known: HashSet<u32> = baseline.iter().map(|p| p.pid).collect();
    log::info!(
        "watcher started; {} existing processes ignored; {opts:?}",
        known.len()
    );
    let mut judge = Judge {
        policy,
        lineage,
        opts,
        titles,
        last_close_tab: None,
        title_attempts: None,
        pending: HashMap::new(),
        last_notice: HashMap::new(),
        last_fg: 0,
        tx,
    };

    while !stop.load(Ordering::SeqCst) {
        std::thread::sleep(POLL_INTERVAL);
        let now = Instant::now();
        let current = process::list();
        let alive: HashSet<u32> = current.iter().map(|p| p.pid).collect();
        judge.lineage.retain_alive(&alive);
        judge.sweep_pending(&alive);

        // A parent and its child can show up in the same snapshot, in any
        // order. Judge processes whose parent is already settled first, and
        // loop until nothing changes, so trust flows down before we decide.
        let mut fresh: Vec<&ProcInfo> =
            current.iter().filter(|p| !known.contains(&p.pid)).collect();
        while !fresh.is_empty() {
            let fresh_pids: HashSet<u32> = fresh.iter().map(|p| p.pid).collect();
            let (deferred, ready): (Vec<&ProcInfo>, Vec<&ProcInfo>) = fresh.iter().partition(|p| {
                fresh_pids.contains(&p.parent) && !judge.lineage.is_trusted(p.parent)
            });
            if ready.is_empty() {
                // Only cycles/orphans left; judge them as they are.
                for p in deferred {
                    judge.judge(p);
                }
                break;
            }
            for p in ready {
                judge.judge(p);
            }
            fresh = deferred;
        }
        known = alive;

        judge.guard_foreground(now);
    }
    log::info!("watcher stopped");
}
