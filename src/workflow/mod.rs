//! The reconstruction workflow: from-scratch, reference and hybrid.
//!
//! An agent building or rebuilding artwork is doing one of three different
//! things, and the sequence of useful moves is not the same for any of them:
//!
//! - **`from_scratch`** — there is nothing to reproduce, only a brief. No
//!   reference exists to measure or compare against, so tracing is never
//!   forced and "validate" means checking the render itself.
//! - **`reference`** — a `Source` reference (see
//!   [`crate::project::ReferenceKind`]) exists and the point is to reproduce
//!   it. Measurement ([`crate::analysis`]) and tracing
//!   ([`crate::vectorize`]) are preferred over redrawing what a vision model
//!   merely *describes*, and completion requires an actual visual comparison
//!   ([`crate::compare`]) against that reference — not just a plausible-
//!   looking render.
//! - **`hybrid`** — a reference exists but reproducing it exactly is not the
//!   goal; a deterministic trace and hand-constructed geometry are both
//!   expected to end up in the same artwork.
//!
//! This module is deliberately narrow: it names the kinds, their ordered
//! phases and a construction-strategy recommendation, as plain, serialisable
//! Rust values — no knowledge of `Project`, `Tool`, MCP, ACP or the TUI,
//! mirroring [`crate::analysis`] and [`crate::vectorize`]'s own isolation.
//!
//! # What this is not
//!
//! This is workflow *guidance*, not a replacement for the agent's own
//! reasoning, and not an enforcement mechanism: nothing here tracks whether a
//! phase actually happened, and there is no persisted "workflow state" on the
//! project. The agent still chooses how the SVG gets constructed; this module
//! only names the intended sequence and, for reference/hybrid work, surfaces
//! the measurements a strategy choice should be grounded in. See
//! `src/tools/builtin.rs`'s `GetWorkflow` for the one place this is exposed
//! to an agent, and issue `#7` for the decision this module implements.

use serde::Serialize;

use crate::analysis::Analysis;

pub mod instructions;

/// A region count at or below this, together with [`MEASURABLE_MAX_COLOURS`],
/// is treated as "clean enough to trace" — a judgement call, tuned toward a
/// single-colour mark or simple multi-colour logo rather than a busy or
/// photographic image. Revisit against real references if it proves wrong in
/// either direction.
const MEASURABLE_MAX_REGIONS: usize = 6;

/// See [`MEASURABLE_MAX_REGIONS`]. A busy multi-colour photograph blows past
/// this even when it happens to form few large regions.
const MEASURABLE_MAX_COLOURS: usize = 4;

/// Which kind of reconstruction work this is.
///
/// The agent chooses this; Shaipe never infers it silently — the same
/// restraint [`crate::compare`] and [`crate::vectorize`] take with their own
/// options, and consistent with this module's own "guidance, not a
/// replacement for the agent's reasoning" scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowKind {
    /// No reference exists. Built from a brief alone.
    FromScratch,
    /// A `Source` reference exists and the point is to reproduce it.
    Reference,
    /// A reference exists as a starting point, but the result is expected to
    /// mix a deterministic trace with hand-constructed geometry rather than
    /// reproduce the reference exactly.
    Hybrid,
}

impl std::fmt::Display for WorkflowKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::FromScratch => "from_scratch",
            Self::Reference => "reference",
            Self::Hybrid => "hybrid",
        })
    }
}

/// A step in the reconstruction loop.
///
/// Which phases apply, and in what order, depends on [`WorkflowKind`] — see
/// [`WorkflowKind::phases`]. A single flat enum rather than one per kind: the
/// phases that are shared (`Construct`, `Render`, `Refine`, `Validate`) mean
/// the same thing regardless of kind, and giving `reference`/`hybrid`'s
/// `Compare` a different name for `from_scratch` would invite exactly the
/// kind of drift a shared vocabulary exists to prevent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// Look at the reference itself — the image, or `get_reference_image`.
    /// Only meaningful when a reference exists.
    Inspect,
    /// Measure the reference's pixels into objective facts
    /// (`get_reference_analysis`). Only exists for `reference`/`hybrid`.
    Analyse,
    /// Decide whether the geometry is clean enough to trace deterministically
    /// or better hand-constructed — see [`recommend_strategy`]. Only exists
    /// for `reference`/`hybrid`.
    ChooseStrategy,
    /// Write or edit the SVG (`write_svg`/`write_variant`), tracing
    /// (`get_reference_trace`), hand-authoring, or both.
    Construct,
    /// Render what was constructed (`render_svg`/`render_grid`).
    Render,
    /// Compare the render against the reference (`compare_reference`). Only
    /// exists for `reference`/`hybrid` — there is nothing to compare a
    /// `from_scratch` render against.
    Compare,
    /// Address what inspecting the render (or the comparison) turned up.
    Refine,
    /// Confirm the result is done. For `reference`/`hybrid` this is only
    /// reachable after `Compare` — see [`WorkflowKind::phases`]'s own doc
    /// comment for why that is structural rather than a convention.
    Validate,
}

impl std::fmt::Display for Phase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Inspect => "inspect",
            Self::Analyse => "analyse",
            Self::ChooseStrategy => "choose_strategy",
            Self::Construct => "construct",
            Self::Render => "render",
            Self::Compare => "compare",
            Self::Refine => "refine",
            Self::Validate => "validate",
        })
    }
}

impl WorkflowKind {
    /// The ordered phases this kind of work moves through.
    ///
    /// `FromScratch` has no reference to measure or compare against, so it
    /// skips `Analyse`, `ChooseStrategy` and `Compare` entirely: tracing is
    /// never forced on it, and its own `Validate` means checking the render
    /// itself rather than a visual diff against nothing.
    ///
    /// `Reference` and `Hybrid` share the same eight-phase sequence,
    /// including `Compare` immediately before `Refine`/`Validate` —
    /// structurally, a reference/hybrid workflow's phase list cannot reach
    /// `Validate` without `Compare` already having been named as a
    /// predecessor. That is what makes "visual validation before completion"
    /// part of the model itself rather than a convention documented
    /// somewhere else.
    #[must_use]
    pub fn phases(self) -> &'static [Phase] {
        use Phase::{
            Analyse, ChooseStrategy, Compare, Construct, Inspect, Refine, Render, Validate,
        };
        match self {
            WorkflowKind::FromScratch => &[Construct, Render, Inspect, Refine, Validate],
            WorkflowKind::Reference | WorkflowKind::Hybrid => &[
                Inspect,
                Analyse,
                ChooseStrategy,
                Construct,
                Render,
                Compare,
                Refine,
                Validate,
            ],
        }
    }
}

/// A kind of work and the phases it moves through — what `get_workflow`
/// reports for a bare `{"kind": ...}` call.
#[derive(Debug, Clone, Serialize)]
pub struct Workflow {
    /// Which kind of reconstruction work this is.
    pub kind: WorkflowKind,
    /// The ordered phases for `kind` — see [`WorkflowKind::phases`].
    pub phases: Vec<Phase>,
}

impl Workflow {
    /// The workflow for `kind`, with its phases resolved.
    #[must_use]
    pub fn for_kind(kind: WorkflowKind) -> Self {
        Self {
            kind,
            phases: kind.phases().to_vec(),
        }
    }
}

/// What `choose_strategy` recommends for a `reference`/`hybrid` workflow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConstructionStrategy {
    /// Trace the reference deterministically (`get_reference_trace`) rather
    /// than hand-describing its geometry.
    Trace,
    /// Hand-construct the geometry: the reference is too complex, or too
    /// photographic, for a deterministic trace to produce a clean result.
    Construct,
    /// Mix both: trace what is measurable, hand-construct the rest. Always
    /// recommended for a [`WorkflowKind::Hybrid`] workflow, regardless of
    /// measurability — see [`recommend_strategy`].
    Hybrid,
}

impl std::fmt::Display for ConstructionStrategy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Trace => "trace",
            Self::Construct => "construct",
            Self::Hybrid => "hybrid",
        })
    }
}

/// A construction-strategy recommendation, grounded in a reference's own
/// measurements rather than asserted on faith — the same restraint
/// [`crate::compare::Comparison`] takes in reporting numbers instead of a
/// single opaque verdict.
#[derive(Debug, Clone, Serialize)]
pub struct StrategyRecommendation {
    /// What `choose_strategy` recommends.
    pub recommended: ConstructionStrategy,
    /// [`Analysis::region_count`] the recommendation was computed from.
    pub region_count: usize,
    /// [`Analysis::dominant_colours`]'s length the recommendation was
    /// computed from.
    pub dominant_colour_count: usize,
    /// [`Analysis::hole_count`] the recommendation was computed from.
    pub hole_count: usize,
}

/// Recommend a construction strategy from a reference's own measurements.
///
/// [`WorkflowKind::Hybrid`] always recommends [`ConstructionStrategy::Hybrid`]:
/// the point of choosing that workflow kind is permission to mix a
/// deterministic trace with hand-constructed geometry, independent of how
/// measurable the reference turns out to be — this function still reports
/// the measurements alongside, since they remain useful context even when
/// they do not change the recommendation.
///
/// [`WorkflowKind::Reference`] recommends [`ConstructionStrategy::Trace`]
/// when the geometry looks clean enough to trace well — few regions, few
/// dominant colours, few holes, the same facts `get_reference_analysis`
/// already exposes — and [`ConstructionStrategy::Construct`] otherwise, e.g.
/// a photograph or a busy multi-region image where tracing would reproduce
/// noise rather than a clean mark.
///
/// # Panics
///
/// Never meaningfully called with [`WorkflowKind::FromScratch`], which has no
/// `choose_strategy` phase — see [`WorkflowKind::phases`]. Does not panic;
/// simply recommends as `Reference` would, since there is no reference-free
/// notion of "strategy" to fall back to instead. Callers should not reach
/// this for `from_scratch` work in the first place.
#[must_use]
pub fn recommend_strategy(kind: WorkflowKind, analysis: &Analysis) -> StrategyRecommendation {
    let region_count = analysis.region_count;
    let dominant_colour_count = analysis.dominant_colours.len();
    let hole_count = analysis.hole_count;

    let measurable =
        region_count <= MEASURABLE_MAX_REGIONS && dominant_colour_count <= MEASURABLE_MAX_COLOURS;

    let recommended = match kind {
        WorkflowKind::Hybrid => ConstructionStrategy::Hybrid,
        WorkflowKind::Reference | WorkflowKind::FromScratch if measurable => {
            ConstructionStrategy::Trace
        }
        WorkflowKind::Reference | WorkflowKind::FromScratch => ConstructionStrategy::Construct,
    };

    StrategyRecommendation {
        recommended,
        region_count,
        dominant_colour_count,
        hole_count,
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;
    use crate::analysis::{Background, Dimensions, Foreground, Symmetry};

    /// A minimal, otherwise-empty [`Analysis`] with the three fields
    /// [`recommend_strategy`] actually reads set to the given values —
    /// everything else is measurement this function does not consult.
    fn analysis_with(region_count: usize, colour_count: usize, hole_count: usize) -> Analysis {
        Analysis {
            dimensions: Dimensions {
                width: 64,
                height: 64,
                aspect_ratio: 1.0,
            },
            background: Background {
                transparent_fraction: 0.0,
                border_colour: None,
                border_uniformity: 1.0,
            },
            dominant_colours: (0..colour_count)
                .map(|index| crate::analysis::DominantColour {
                    colour: format!("#{index:06x}"),
                    fraction: 1.0 / colour_count.max(1) as f64,
                })
                .collect(),
            foreground: Foreground {
                pixel_fraction: 0.5,
                bounding_box: None,
            },
            region_count,
            regions: Vec::new(),
            hole_count,
            holes: Vec::new(),
            symmetry: Symmetry {
                horizontal: 1.0,
                vertical: 1.0,
            },
        }
    }

    #[test]
    fn from_scratch_has_no_analyse_choose_strategy_or_compare_phase() {
        let phases = WorkflowKind::FromScratch.phases();
        assert!(!phases.contains(&Phase::Analyse));
        assert!(!phases.contains(&Phase::ChooseStrategy));
        assert!(!phases.contains(&Phase::Compare));
        assert_eq!(
            phases,
            &[
                Phase::Construct,
                Phase::Render,
                Phase::Inspect,
                Phase::Refine,
                Phase::Validate
            ]
        );
    }

    #[test]
    fn reference_and_hybrid_share_the_same_phase_sequence() {
        assert_eq!(
            WorkflowKind::Reference.phases(),
            WorkflowKind::Hybrid.phases()
        );
    }

    #[test]
    fn a_reference_workflow_cannot_reach_validate_without_compare_first() {
        // Structural, not enforced at runtime: `Compare` names a position
        // earlier in the slice than `Validate`, for both kinds that have it.
        let phases = WorkflowKind::Reference.phases();
        let compare = phases.iter().position(|phase| *phase == Phase::Compare);
        let validate = phases.iter().position(|phase| *phase == Phase::Validate);
        assert!(compare.unwrap() < validate.unwrap());
    }

    #[test]
    fn a_measurable_reference_is_recommended_tracing() {
        let analysis = analysis_with(1, 1, 0);
        let recommendation = recommend_strategy(WorkflowKind::Reference, &analysis);
        assert_eq!(recommendation.recommended, ConstructionStrategy::Trace);
        assert_eq!(recommendation.region_count, 1);
        assert_eq!(recommendation.dominant_colour_count, 1);
    }

    #[test]
    fn a_complex_reference_is_recommended_manual_construction() {
        let analysis = analysis_with(30, 12, 4);
        let recommendation = recommend_strategy(WorkflowKind::Reference, &analysis);
        assert_eq!(recommendation.recommended, ConstructionStrategy::Construct);
    }

    #[test]
    fn hybrid_always_recommends_mixing_regardless_of_measurements() {
        let clean = analysis_with(1, 1, 0);
        let busy = analysis_with(30, 12, 4);
        assert_eq!(
            recommend_strategy(WorkflowKind::Hybrid, &clean).recommended,
            ConstructionStrategy::Hybrid
        );
        assert_eq!(
            recommend_strategy(WorkflowKind::Hybrid, &busy).recommended,
            ConstructionStrategy::Hybrid
        );
    }

    #[test]
    fn workflow_kind_displays_as_the_tools_input_schema_names_it() {
        assert_eq!(WorkflowKind::FromScratch.to_string(), "from_scratch");
        assert_eq!(WorkflowKind::Reference.to_string(), "reference");
        assert_eq!(WorkflowKind::Hybrid.to_string(), "hybrid");
    }
}
