//! Background thread that polls the process table and kills newcomers the
//! policy rejects. Only processes that appear *after* the watcher starts are
//! considered, so whatever the user already had open is left alone.
//! Children of allowed apps inherit permission (see `be_calm_core::lineage`).

use super::process::{self, ProcInfo};
use be_calm_core::{Lineage, Policy, Verdict};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

pub const POLL_INTERVAL: Duration = Duration::from_millis(250);

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
}

pub struct Watcher {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
    rx: Receiver<WatchEvent>,
}

impl Watcher {
    /// `block_windowless`: kill rejected processes immediately. Otherwise a
    /// rejected process is only killed once it shows a visible window, so
    /// background helpers (sync clients, updaters) are left alone.
    pub fn start(policy: Policy, block_windowless: bool) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel();
        let stop2 = stop.clone();
        let handle = std::thread::Builder::new()
            .name("be_calm-watcher".into())
            .spawn(move || run(policy, block_windowless, stop2, tx))
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
    block_windowless: bool,
    /// Rejected processes waiting to show a window before we kill them.
    pending: HashMap<u32, PathBuf>,
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
        if self.block_windowless {
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
}

fn run(policy: Policy, block_windowless: bool, stop: Arc<AtomicBool>, tx: Sender<WatchEvent>) {
    // Baseline: everything alive right now is grandfathered in, and every
    // already-running *user* app is trusted so the helpers it keeps spawning
    // (browser tabs, language servers, ...) are left alone — we only want to
    // stop apps the user tries to open from now on. System processes
    // (explorer, shells) are deliberately not trusted: they are how new apps
    // get launched.
    let mut lineage = Lineage::new();
    let baseline = process::list();
    for p in &baseline {
        if let Some(exe) = &p.exe {
            match policy.verdict(exe) {
                Verdict::AllowListed | Verdict::AllowSameDir | Verdict::Block => {
                    lineage.trust(p.pid)
                }
                Verdict::AllowSystem | Verdict::AllowSelf => {}
            }
        }
    }
    let mut known: HashSet<u32> = baseline.iter().map(|p| p.pid).collect();
    log::info!(
        "watcher started; {} existing processes ignored",
        known.len()
    );
    let mut judge = Judge {
        policy,
        lineage,
        block_windowless,
        pending: HashMap::new(),
        tx,
    };

    while !stop.load(Ordering::SeqCst) {
        std::thread::sleep(POLL_INTERVAL);
        let current = process::list();
        let now: HashSet<u32> = current.iter().map(|p| p.pid).collect();
        judge.lineage.retain_alive(&now);
        judge.sweep_pending(&now);

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
        known = now;
    }
    log::info!("watcher stopped");
}
