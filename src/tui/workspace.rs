//! Session-only navigation through workflow guidance.
//!
//! A selected phase is where the user is looking, not a claim that work was
//! completed. In particular, an agent tool call must never advance it.

use crate::workflow::{Phase, WorkflowKind};

/// The user's explicit workflow context; never serialized into the SVG.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Workspace {
    kind: Option<WorkflowKind>,
    phase: Option<Phase>,
}

impl Workspace {
    /// An unset kind leaves the choice to the user rather than guessing from
    /// the presence of a reference, which may be inspiration rather than a target.
    #[must_use]
    pub const fn kind(self) -> Option<WorkflowKind> {
        self.kind
    }

    /// The phase currently being inspected, if a kind has been chosen.
    #[must_use]
    pub const fn phase(self) -> Option<Phase> {
        self.phase
    }

    /// Cycle the explicit kind; retain a shared phase when possible.
    pub fn next_kind(&mut self) {
        let kind = match self.kind {
            None | Some(WorkflowKind::Hybrid) => WorkflowKind::FromScratch,
            Some(WorkflowKind::FromScratch) => WorkflowKind::Reference,
            Some(WorkflowKind::Reference) => WorkflowKind::Hybrid,
        };
        self.phase = self
            .phase
            .filter(|phase| kind.phases().contains(phase))
            .or_else(|| kind.phases().first().copied());
        self.kind = Some(kind);
    }

    /// Move through the applicable phases, wrapping so refinement can return
    /// to construction without fabricating a completed-workflow state.
    pub fn step(&mut self, direction: isize) {
        let Some(kind) = self.kind else {
            return;
        };
        let phases = kind.phases();
        let current = self
            .phase
            .and_then(|phase| phases.iter().position(|candidate| *candidate == phase))
            .unwrap_or(0);
        let next = (current as isize + direction).rem_euclid(phases.len() as isize) as usize;
        self.phase = Some(phases[next]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workflow_navigation_starts_unset_until_the_user_chooses_a_kind() {
        let mut workspace = Workspace::default();
        workspace.step(1);
        assert_eq!(workspace.kind(), None);
        assert_eq!(workspace.phase(), None);
    }

    #[test]
    fn from_scratch_skips_reference_comparison_and_navigation_wraps() {
        let mut workspace = Workspace::default();
        workspace.next_kind();
        assert_eq!(workspace.phase(), Some(Phase::Construct));
        for _ in 0..WorkflowKind::FromScratch.phases().len() {
            assert_ne!(workspace.phase(), Some(Phase::Compare));
            workspace.step(1);
        }
        assert_eq!(workspace.phase(), Some(Phase::Construct));
        workspace.step(-1);
        assert_eq!(workspace.phase(), Some(Phase::Validate));
    }

    #[test]
    fn switching_kinds_preserves_shared_phases_and_resets_inapplicable_ones() {
        let mut workspace = Workspace::default();
        workspace.next_kind();
        workspace.step(1);
        assert_eq!(workspace.phase(), Some(Phase::Render));
        workspace.next_kind();
        assert_eq!(workspace.kind(), Some(WorkflowKind::Reference));
        assert_eq!(workspace.phase(), Some(Phase::Render));
        workspace.step(1);
        assert_eq!(workspace.phase(), Some(Phase::Compare));
        workspace.next_kind();
        assert_eq!(workspace.phase(), Some(Phase::Compare));
        workspace.next_kind();
        assert_eq!(workspace.phase(), Some(Phase::Construct));
    }
}
