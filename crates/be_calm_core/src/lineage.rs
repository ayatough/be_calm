//! Process lineage: children of an allowed app inherit its permission.
//!
//! Without this, an allowed editor could not spawn its own terminal, language
//! server, or updater, because those often live under different paths. The
//! flip side is that a launcher (e.g. a game store client) that the user
//! explicitly allows can start anything it likes — that is the user's call.

use crate::policy::Verdict;
use std::collections::HashSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Allow(Verdict),
    /// Allowed only because an ancestor is an allowed app.
    AllowDescendant,
    Block,
}

impl Decision {
    pub fn is_blocked(self) -> bool {
        matches!(self, Decision::Block)
    }
}

#[derive(Debug, Default, Clone)]
pub struct Lineage {
    /// PIDs whose descendants are allowed.
    trusted: HashSet<u32>,
}

impl Lineage {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register an already-running process that is on the allowlist.
    pub fn trust(&mut self, pid: u32) {
        self.trusted.insert(pid);
    }

    pub fn is_trusted(&self, pid: u32) -> bool {
        self.trusted.contains(&pid)
    }

    /// Decide about a new process given the policy's verdict on its exe.
    /// Anything spawned by a trusted process becomes trusted itself, so
    /// permission flows down the whole tree (editor -> shell -> compiler).
    pub fn decide(&mut self, pid: u32, parent: u32, verdict: Verdict) -> Decision {
        let parent_trusted = self.trusted.contains(&parent);
        match verdict {
            Verdict::AllowListed | Verdict::AllowSameDir => {
                self.trusted.insert(pid);
                Decision::Allow(verdict)
            }
            Verdict::Block if parent_trusted => {
                self.trusted.insert(pid);
                Decision::AllowDescendant
            }
            Verdict::Block => Decision::Block,
            Verdict::AllowSystem | Verdict::AllowSelf => {
                if parent_trusted {
                    self.trusted.insert(pid);
                }
                Decision::Allow(verdict)
            }
        }
    }

    /// Drop PIDs that no longer exist so a recycled PID cannot inherit trust.
    pub fn retain_alive(&mut self, alive: &HashSet<u32>) {
        self.trusted.retain(|p| alive.contains(p));
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
        assert_eq!(l.decide(12, 11, Verdict::Block), Decision::AllowDescendant);
        assert!(l.is_trusted(12));
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
    }

    #[test]
    fn pre_existing_listed_apps_can_be_trusted() {
        let mut l = Lineage::new();
        l.trust(42);
        assert_eq!(l.decide(43, 42, Verdict::Block), Decision::AllowDescendant);
    }

    #[test]
    fn dead_pids_lose_trust() {
        let mut l = Lineage::new();
        l.trust(1);
        l.trust(2);
        l.retain_alive(&HashSet::from([2]));
        assert!(!l.is_trusted(1));
        assert!(l.is_trusted(2));
        assert_eq!(l.decide(9, 1, Verdict::Block), Decision::Block);
    }
}
