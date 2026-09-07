//! Process lineage: children of an allowed app inherit its permission.
//!
//! Without this, an allowed editor could not spawn its own terminal, language
//! server, or updater, because those often live under different paths. The
//! flip side is that a launcher (e.g. a game store client) that the user
//! explicitly allows can start anything it likes — that is the user's call.
//!
//! Two kinds of trust are tracked:
//! - `Listed`: the tree under an app on the allowlist. These may spawn
//!   children *and* be used (their windows may be in the foreground).
//! - `Grandfathered`: apps that were already running when the session
//!   started. Their children are left alone (browser tabs, sync helpers),
//!   but their windows are not allowed in the foreground.

use crate::policy::Verdict;
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    Listed,
    Grandfathered,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Allow(Verdict),
    /// Allowed only because an ancestor is trusted.
    AllowDescendant(Origin),
    Block,
}

impl Decision {
    pub fn is_blocked(self) -> bool {
        matches!(self, Decision::Block)
    }
}

#[derive(Debug, Default, Clone)]
pub struct Lineage {
    trusted: HashMap<u32, Origin>,
}

impl Lineage {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register an already-running process.
    pub fn trust(&mut self, pid: u32, origin: Origin) {
        self.trusted.insert(pid, origin);
    }

    pub fn is_trusted(&self, pid: u32) -> bool {
        self.trusted.contains_key(&pid)
    }

    /// True if the process descends from (or is) an app on the allowlist.
    pub fn is_listed_tree(&self, pid: u32) -> bool {
        self.trusted.get(&pid) == Some(&Origin::Listed)
    }

    /// Decide about a new process given the policy's verdict on its exe.
    /// Anything spawned by a trusted process becomes trusted itself, so
    /// permission flows down the whole tree (editor -> shell -> compiler).
    pub fn decide(&mut self, pid: u32, parent: u32, verdict: Verdict) -> Decision {
        let parent_origin = self.trusted.get(&parent).copied();
        match verdict {
            Verdict::AllowListed | Verdict::AllowSameDir => {
                self.trusted.insert(pid, Origin::Listed);
                Decision::Allow(verdict)
            }
            Verdict::Block => match parent_origin {
                Some(origin) => {
                    self.trusted.insert(pid, origin);
                    Decision::AllowDescendant(origin)
                }
                None => Decision::Block,
            },
            Verdict::AllowSystem | Verdict::AllowSelf => {
                if let Some(origin) = parent_origin {
                    self.trusted.insert(pid, origin);
                }
                Decision::Allow(verdict)
            }
        }
    }

    /// Drop PIDs that no longer exist so a recycled PID cannot inherit trust.
    pub fn retain_alive(&mut self, alive: &HashSet<u32>) {
        self.trusted.retain(|p, _| alive.contains(p));
    }
}

/// May a window owned by this process be in the foreground?
pub fn foreground_allowed(lineage: &Lineage, pid: u32, verdict: Verdict) -> bool {
    match verdict {
        Verdict::AllowSelf
        | Verdict::AllowSystem
        | Verdict::AllowListed
        | Verdict::AllowSameDir => true,
        Verdict::Block => lineage.is_listed_tree(pid),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn listed_app_and_its_tree_are_allowed() {
        let mut l = Lineage::new();
        assert_eq!(
            l.decide(10, 1, Verdict::AllowListed),
            Decision::Allow(Verdict::AllowListed)
        );
        // editor (10) -> cmd.exe (system, 11) -> cargo.exe (12, not listed)
        assert_eq!(
            l.decide(11, 10, Verdict::AllowSystem),
            Decision::Allow(Verdict::AllowSystem)
        );
        assert_eq!(
            l.decide(12, 11, Verdict::Block),
            Decision::AllowDescendant(Origin::Listed)
        );
        assert!(l.is_trusted(12));
        assert!(l.is_listed_tree(12));
        assert!(foreground_allowed(&l, 12, Verdict::Block));
    }

    #[test]
    fn children_of_untrusted_processes_are_blocked() {
        let mut l = Lineage::new();
        // explorer (system, not trusted) launches a game
        assert_eq!(
            l.decide(5, 1, Verdict::AllowSystem),
            Decision::Allow(Verdict::AllowSystem)
        );
        assert_eq!(l.decide(6, 5, Verdict::Block), Decision::Block);
        assert!(l.decide(6, 5, Verdict::Block).is_blocked());
        assert!(!foreground_allowed(&l, 6, Verdict::Block));
    }

    #[test]
    fn grandfathered_apps_may_spawn_but_not_be_used() {
        let mut l = Lineage::new();
        l.trust(42, Origin::Grandfathered);
        assert_eq!(
            l.decide(43, 42, Verdict::Block),
            Decision::AllowDescendant(Origin::Grandfathered)
        );
        assert!(!l.is_listed_tree(43));
        assert!(!foreground_allowed(&l, 42, Verdict::Block));
        assert!(!foreground_allowed(&l, 43, Verdict::Block));
    }

    #[test]
    fn system_and_listed_windows_are_always_allowed_in_front() {
        let l = Lineage::new();
        assert!(foreground_allowed(&l, 1, Verdict::AllowSystem));
        assert!(foreground_allowed(&l, 1, Verdict::AllowListed));
        assert!(foreground_allowed(&l, 1, Verdict::AllowSameDir));
        assert!(foreground_allowed(&l, 1, Verdict::AllowSelf));
    }

    #[test]
    fn dead_pids_lose_trust() {
        let mut l = Lineage::new();
        l.trust(1, Origin::Listed);
        l.trust(2, Origin::Listed);
        l.retain_alive(&HashSet::from([2]));
        assert!(!l.is_trusted(1));
        assert!(l.is_trusted(2));
        assert_eq!(l.decide(9, 1, Verdict::Block), Decision::Block);
    }
}
