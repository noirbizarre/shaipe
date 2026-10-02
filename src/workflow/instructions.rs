//! What Shaipe tells an agent about reconstruction, before it is asked to do any.
//!
//! This is the third of three different things an agent reads, and they must
//! not be confused: the project's *prompt* is the user's design brief and lives
//! in the document's metadata; a *turn* is whatever the user sends this time;
//! and these are Shaipe's own instructions, which belong to neither. They are
//! plain text so that any transport can carry them — the MCP server sends them
//! as its `instructions`, which is the one channel every ACP agent shares —
//! and they name no agent product, because the workflow is the same whichever
//! model is following it.
//!
//! Like the rest of this module they are guidance, not enforcement (see
//! [`super`]): an agent that decides to skip a step still can.

use super::WorkflowKind;

/// The reconstruction section of what an agent is told.
///
/// Built rather than a `const` for one reason: the phase sequences are spliced
/// in from [`WorkflowKind::phases`], so this text can never disagree with what
/// `get_workflow` returns about the order. Everything else is fixed, so the
/// result is identical on every call and reviewable as one piece of prose.
///
/// Tool names are in backticks because a test checks that each one exists; a
/// tool renamed without this text being updated fails there rather than
/// sending an agent to call something that is not there.
#[must_use]
pub fn reconstruction() -> String {
    let from_scratch = sequence(WorkflowKind::FromScratch);
    let reference = sequence(WorkflowKind::Reference);
    format!(
        "\
# Reconstruction workflow

Decide first which kind of work this is. `get_references` lists what is attached to the \
project and what each file is for.
- `from_scratch`: nothing is attached to reproduce. You are working from the brief alone.
- `reference`: a `source` reference is attached and the point is to reproduce it.
- `hybrid`: a reference is attached, but the result should mix a trace of it with geometry \
you construct yourself.
The choice is yours, and nothing checks it. `get_workflow` with that kind returns the \
ordered phases; give it `src` for a reference or hybrid and it also recommends whether to \
trace or construct, from measurements of that image. Its phases are:
- `from_scratch`: {from_scratch}. Here `inspect` means looking at your own render.
- `reference` and `hybrid`: {reference}. Here `inspect` means looking at the reference.

Inspect the reference before you edit anything: call `get_reference_image` and look at it. \
`get_references` only says a file exists, not what it shows.

Measure; do not estimate. You cannot read exact positions, sizes, colours or curves off an \
image by looking at it. Call `get_reference_analysis` for regions, bounding boxes, \
colours and symmetry, and `get_reference_trace` for outlines. Treat what they return as \
fact, and treat your own reading of pixel positions as a guess. What a region is, whether \
a letter, an animal or a shield, remains your judgement: measurement cannot name it.

A region is not always one colour. The `appearance` in that analysis says per region whether \
its fill is `flat`, a `linear_gradient` or `radial_gradient` with an axis or centre and stops, \
or `varied`. A `colour` trace does not fragment a gradient into flat layers: it returns the \
region as one path with its `region_id` and the same measured `appearance`, and paints it \
with that fill, so take the path's outline and the fill as two separate pieces of evidence \
and check the result with `compare_reference`. `varied` means it was not vouched for as a \
gradient, so do not assume one; the trace lists it under `fallbacks` and leaves it as flat \
layers. A `silhouette` trace is geometry only. A `stroke` \
is a candidate from the region's geometry alone: a ring may equally be a filled shape. \
Opacity is reported apart from colour, so a translucent fill keeps its colour and an \
`opacity` to apply.

Lettering is typography, not shape. When `get_reference_analysis` reports `typography`, each \
line is a candidate: regions that share a baseline and are spaced like letters, with the \
baseline, letter height, spacing, words and the colour of each word. It cannot say what the \
letters say or which font sets them; that is your reading. Enlarge a line with \
`get_reference_image` and its `bounding_box` before you read it, and never invent text: \
when a letter or the font is doubtful, write your best reading and say so with \
`data-shaipe-confidence` (`high`, `medium` or `low`) and a `data-shaipe-note` naming what is \
doubtful on the `<text>`, and tell the user. Prefer `<text>` in a font the project declares \
(`get_project` lists them) to drawing the letters as paths, so the words stay editable; draw \
them as paths only when no declared font is close, and say that. Size the text from the \
measured letter height, not by eye (font size is about the tall height over the typeface's \
cap height, near 0.7). Keep each word's and symbol's fill as its own assignment; do not give \
a wordmark and the mark beside it one colour because they sit together. `compare_reference` \
reports `typography` per line and `declared_text` for what you set.

A logo is put together from parts. When `get_reference_analysis` reports `composition`, its \
`components` group regions that belong together, each with a `role` (`primary`, `secondary`, \
`decorative` or `lettering`, judged by size alone) and a `contour`. Build one element or \
group per component and keep a `decorative` one separate from the primary artwork. A \
contour that is `open` is a stroke that does not close: draw it as an unclosed path with \
`fill=\"none\"` and a `stroke`, never as a closed shape; `closed` encloses a hole, so keep \
the hole. Place parts from the measured `relationships`, `alignments` and `spacings`, not \
by eye: components on one line share it, and a run whose spacing is `even` takes one gap. \
`repeats` are parts of one size, so draw one and reuse it. Geometry and `appearance` are \
separate evidence, so a part's colour is not a reason to move it. A `composition` is \
absent when the artwork is one shape or too fragmented to group, and you then work from \
`regions` alone.

Trace when the reference is a clean, flat-colour mark with few regions and few colours. \
Construct it yourself, with real shapes, when it is a photograph, busy or textured, or when \
you need clean primitives such as circles, rectangles and text. Mix the two when part of it \
is measurable and part is not. The recommendation from `get_workflow` is a starting point \
and you may overrule it, but say why.

Work in a loop, not in one pass. Write with `write_variant` or `write_svg`, look with \
`render_svg`, and for a `reference` or `hybrid` compare with `compare_reference`, which \
renders your variant at the reference's own size and reports what differs and by how much \
as numbers and as an overlay and a difference image. Fix what the comparison shows, \
largest offset first, and go round again. When the shape already matches, read \
`appearance.findings` in the comparison: it says where a fill, gradient or transparency \
differs. `compare_reference` and `render_svg` measure and \
show; they never say you are finished.

A valid SVG is not a finished one. `write_svg` succeeding means the document parses, not \
that it looks right. For a `reference` or `hybrid`, do not stop before you have run \
`compare_reference` on the artwork as it now stands and looked at the result. For \
`from_scratch`, do not stop before you have looked at `render_svg` of the final artwork. \
If something still differs when you stop, say what.

Keep the result editable. A trace is raw material, not the deliverable: simplify it, drop \
specks, snap it to the palette, and keep the variant's element id so the project still \
renders it. Never embed the reference as an `<image>` and never paste raster data into \
the SVG. Do not chase pixel noise, since matching the reference closely enough and staying \
clean to edit matters more than matching every pixel."
    )
}

/// One kind's phases as the text an agent reads: names in order, joined by
/// commas. The same rendering the tests look for in `get_workflow`'s
/// description, so the two are compared on one definition of "the same order".
#[must_use]
pub fn sequence(kind: WorkflowKind) -> String {
    kind.phases()
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_instructions_name_every_workflow_kind() {
        let text = reconstruction();
        for kind in [
            WorkflowKind::FromScratch,
            WorkflowKind::Reference,
            WorkflowKind::Hybrid,
        ] {
            assert!(
                text.contains(&format!("`{kind}`")),
                "the instructions never name `{kind}`"
            );
        }
    }

    #[test]
    fn the_instructions_carry_each_kinds_phases_in_order() {
        let text = reconstruction();
        // Verbatim, so a phase reordered in the model changes the text with it
        // rather than leaving the two to disagree.
        assert!(text.contains(&sequence(WorkflowKind::FromScratch)));
        assert!(text.contains(&sequence(WorkflowKind::Reference)));
        assert_eq!(
            sequence(WorkflowKind::Reference),
            sequence(WorkflowKind::Hybrid),
            "hybrid shares reference's phases, and the text says so"
        );
    }

    #[test]
    fn the_instructions_say_a_valid_svg_is_not_a_finished_one() {
        let text = reconstruction();
        assert!(text.contains("A valid SVG is not a finished one"));
        assert!(text.contains("do not stop before you have run `compare_reference`"));
    }

    #[test]
    fn the_instructions_say_to_inspect_before_editing_and_to_measure() {
        let text = reconstruction();
        assert!(text.contains("Inspect the reference before you edit anything"));
        assert!(text.contains("Measure; do not estimate"));
    }

    #[test]
    fn the_instructions_say_to_read_lettering_and_admit_doubt_rather_than_invent_it() {
        let text = reconstruction();
        assert!(text.contains("never invent text"));
        assert!(text.contains("`data-shaipe-confidence`"));
        assert!(text.contains("Prefer `<text>` in a font the project declares"));
    }

    #[test]
    fn the_instructions_say_to_compose_from_components_and_keep_open_contours_open() {
        let text = reconstruction();
        assert!(text.contains("keep a `decorative` one separate from the primary artwork"));
        assert!(text.contains("draw it as an unclosed path"));
        assert!(text.contains("Place parts from the measured `relationships`"));
    }

    #[test]
    fn the_instructions_forbid_embedding_the_reference_raster() {
        assert!(reconstruction().contains("Never embed the reference as an `<image>`"));
    }

    #[test]
    fn the_instructions_mention_no_agent_product() {
        let text = reconstruction().to_lowercase();
        for product in ["opencode", "claude", "gpt", "gemini", "cursor", "copilot"] {
            assert!(
                !text.contains(product),
                "the workflow is generic, but the instructions name {product}"
            );
        }
    }

    #[test]
    fn the_instructions_are_identical_on_every_call() {
        assert_eq!(reconstruction(), reconstruction());
    }
}
