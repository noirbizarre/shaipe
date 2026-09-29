//! The reference-reconstruction loop, proven without a model.
//!
//! reference -> analyse / trace -> construct -> render -> compare.
//!
//! Every step here is deterministic and local, so every step can be tested
//! for the one thing a vision model cannot promise: that the same input gives
//! the same answer. What a model *adds* — deciding what to draw — is replaced
//! by the fixture's known-good construction and a small policy that steers by
//! the comparison report alone (`harness::refine`). Nothing in this file needs
//! a network, a credential, or a font that was not shipped with the project.
//!
//! The one thing not tested here is a real model looking at a real reference;
//! that costs money and is `#[ignore]`d in `tests/acp_opencode.rs`.
//!
//! Goldens are `insta` snapshots in `tests/snapshots/`. They record *measured
//! facts* — counts, boxes, colours, rounded ratios — rather than PNG or SVG
//! bytes, because a fact survives a change of image encoder and a byte does
//! not. Review them with `mise run snapshots`; never edit one by hand.

#[path = "reconstruction/corpus.rs"]
mod corpus;
#[path = "reconstruction/harness.rs"]
mod harness;

use std::path::Path;

use harness::{Placement, Session, number, overlap, run_loop, trace_body};
use serde_json::{Value, json};
use shaipe::analysis;
use shaipe::compare;
use shaipe::workflow::{Phase, Workflow, WorkflowKind};

/// Round every float in a report, so a golden records a measurement rather
/// than the last digit of one platform's arithmetic.
fn rounded(mut value: Value, places: i32) -> Value {
    fn walk(value: &mut Value, scale: f64) {
        match value {
            Value::Number(number) if number.is_f64() => {
                let rounded = (number.as_f64().expect("a float") * scale).round() / scale;
                // `-0.0` and `0.0` are the same measurement and different text.
                let rounded = if rounded == 0.0 { 0.0 } else { rounded };
                *value = json!(rounded);
            }
            Value::Array(items) => items.iter_mut().for_each(|item| walk(item, scale)),
            Value::Object(fields) => fields.values_mut().for_each(|item| walk(item, scale)),
            _ => {}
        }
    }
    walk(&mut value, 10_f64.powi(places));
    value
}

/// A report as the text a snapshot stores.
fn snapshot_text(value: Value, places: i32) -> String {
    serde_json::to_string_pretty(&rounded(value, places)).expect("a report serialises")
}

#[test]
fn the_corpus_is_deterministic_and_small_enough_to_redistribute() {
    let first = corpus::all();
    let second = corpus::all();

    let names: Vec<_> = first.iter().map(|fixture| fixture.name).collect();
    assert_eq!(
        names,
        [
            "silhouette",
            "light_on_dark",
            "disconnected",
            "holes",
            "multicolour",
            "symmetric",
            "noisy"
        ],
        "one fixture per kind of reference the reconstruction epic names",
    );

    for (a, b) in first.iter().zip(&second) {
        assert_eq!(a.png, b.png, "`{}` differs between two generations", a.name);
        // A generous ceiling that a 64-pixel flat-colour PNG is nowhere near:
        // it exists so that a fixture which quietly became a photograph fails.
        assert!(
            a.png.len() < 8 * 1024,
            "`{}` is {} bytes; a fixture is a few kilobytes",
            a.name,
            a.png.len()
        );
    }

    let total: usize = first.iter().map(|fixture| fixture.png.len()).sum();
    assert!(total < 32 * 1024, "the corpus is {total} bytes");
}

#[test]
fn every_fixture_analyses_to_the_facts_it_was_drawn_with() {
    for fixture in corpus::all() {
        let mut session = Session::open(&fixture, &fixture.construction);
        let analysis = session.analysis();
        let name = fixture.name;
        let expected = &fixture.expected;

        let regions = analysis["region_count"].as_u64().expect("a count") as usize;
        let holes = analysis["hole_count"].as_u64().expect("a count") as usize;
        // Colours that cover at least 2% of the pixels. The analysis also
        // lists the intermediate colours of an anti-aliased edge, none of them
        // more than about 1.4% here; the strategy ignores them, so the
        // expectation is stated in the same terms.
        let colours = analysis["dominant_colours"]
            .as_array()
            .expect("a list")
            .iter()
            .filter(|colour| number(colour, "fraction") >= 0.02)
            .count();

        assert!(
            expected.regions.contains(&regions),
            "`{name}`: {regions} regions, expected {:?}",
            expected.regions
        );
        assert!(
            expected.holes.contains(&holes),
            "`{name}`: {holes} holes, expected {:?}",
            expected.holes
        );
        assert!(
            expected.dominant_colours.contains(&colours),
            "`{name}`: {colours} dominant colours, expected {:?}",
            expected.dominant_colours
        );

        insta::assert_snapshot!(format!("analysis_{name}"), snapshot_text(analysis, 4));
    }
}

#[test]
fn regions_are_numbered_by_area_so_their_ids_are_stable() {
    let fixture = corpus::named("disconnected");
    let analysis = Session::open(&fixture, &fixture.construction).analysis();

    // 14x14, 10x10, 6x6: the ids must follow the areas, not the scan order,
    // which would put the top-left square first and the bottom-right last for
    // a different reason.
    let sides: Vec<_> = analysis["regions"]
        .as_array()
        .expect("a list")
        .iter()
        .map(|region| region["bounding_box"]["width"].as_u64().expect("a width"))
        .collect();
    assert_eq!(sides, [14, 10, 6]);
}

#[test]
fn a_hole_is_attributed_to_the_ring_around_it_and_not_to_the_disc_inside() {
    let fixture = corpus::named("holes");
    let analysis = Session::open(&fixture, &fixture.construction).analysis();

    let hole = &analysis["holes"][0];
    // Region 0 is the ring (the larger area). The floating disc's own box is
    // too small to contain the hole's, so it must not be named.
    assert_eq!(hole["enclosed_by"], 0);
    assert_eq!(analysis["regions"][0]["bounding_box"]["width"], 48);
}

#[test]
fn symmetry_is_exactly_one_for_a_mirrored_mark_and_low_for_an_asymmetric_one() {
    let fixture = corpus::named("symmetric");
    let analysis = Session::open(&fixture, &fixture.construction).analysis();
    assert!((number(&analysis, "symmetry.horizontal") - 1.0).abs() < 1e-9);
    assert!((number(&analysis, "symmetry.vertical") - 1.0).abs() < 1e-9);

    // The control: without it, a measurement that always says `1.0` passes.
    let control = analysis::analyze(Path::new("control.png"), &corpus::asymmetric_control())
        .expect("the control decodes");
    assert!(control.symmetry.horizontal < 0.6, "{control:?}");
    assert!(control.symmetry.vertical < 0.6, "{control:?}");
}

#[test]
fn the_strategy_recommended_for_each_fixture_is_the_one_it_was_drawn_to_need() {
    // ADR 021 called `MEASURABLE_MAX_REGIONS = 6` and `MEASURABLE_MAX_COLOURS
    // = 4` "a starting judgement call, not a tuned result". This is where they
    // are held to real references: a clean mark should be traced, and a mark
    // speckled with noise should be constructed.
    for fixture in corpus::all() {
        let mut session = Session::open(&fixture, &fixture.construction);
        let workflow = session.workflow();
        assert_eq!(
            workflow["strategy"]["recommended"], fixture.expected.strategy,
            "`{}`: {}",
            fixture.name, workflow["strategy"]
        );
    }
}

#[test]
fn many_separate_parts_are_constructed_however_few_colours_they_have() {
    // The corpus's other "construct" fixture is also multi-coloured, so it
    // would pass on colour count alone. Nine dark squares on white are two
    // colours and nine regions: only the region threshold can send this one to
    // `construct`.
    let fixture = corpus::many_parts();
    let workflow = Session::open(&fixture, &fixture.construction).workflow();

    assert_eq!(workflow["strategy"]["dominant_colour_count"], 2);
    assert_eq!(workflow["strategy"]["region_count"], 9);
    assert_eq!(workflow["strategy"]["recommended"], "construct");
}

#[test]
fn a_hybrid_workflow_recommends_hybrid_whatever_the_reference_measures() {
    let fixture = corpus::named("noisy");
    let mut session = Session::open(&fixture, &fixture.construction);
    let workflow = session
        .call(
            "get_workflow",
            &json!({ "kind": "hybrid", "src": "reference.png" }),
        )
        .value;
    assert_eq!(workflow["strategy"]["recommended"], "hybrid");
}

#[test]
fn every_fixtures_trace_matches_its_golden() {
    // (fixture, trace options). The options are the ones the tool's own
    // description tells an agent to choose from the analysis: colour mode for
    // several colours, `invert` for light on dark.
    let cases = [
        ("silhouette", json!({})),
        ("light_on_dark", json!({ "invert": true })),
        ("disconnected", json!({})),
        ("holes", json!({})),
        ("multicolour", json!({ "mode": "colour" })),
        ("symmetric", json!({})),
        ("noisy", json!({})),
    ];

    for (name, options) in cases {
        let fixture = corpus::named(name);
        let mut session = Session::open(&fixture, &fixture.construction);
        let mut trace = session.trace(&options);

        if name == "silhouette" {
            // The document's text is deliberately not a golden. `vtracer`'s
            // curve fitting is floating point, and macOS fitted one segment of
            // this outline differently from Linux (`c-.01-.57-.01-.57` against
            // `c-.01-1.13-.02-2.27`): the same shape, different bytes, with
            // the version already pinned. What is stable is the structure, and
            // the measured geometry snapshotted below.
            let svg = trace["svg"].as_str().expect("markup");
            assert_eq!(svg.matches("<path").count(), 1, "{svg}");
            assert!(svg.contains(r##"fill="#000000""##), "{svg}");
            assert!(
                !svg.contains("viewBox"),
                "a trace carries no viewBox: {svg}"
            );
        }

        // The markup is large and already covered above; the paths past the
        // first few are the same shape of fact. A golden that is a wall of
        // numbers is one nobody reviews.
        let object = trace.as_object_mut().expect("an object");
        object.remove("svg");
        if let Some(Value::Array(paths)) = object.get_mut("paths") {
            paths.truncate(4);
        }
        insta::assert_snapshot!(format!("trace_{name}"), snapshot_text(trace, 1));
    }
}

#[test]
fn tracing_light_artwork_without_inverting_traces_the_background_instead() {
    let fixture = corpus::named("light_on_dark");
    let mut session = Session::open(&fixture, &fixture.construction);

    let wrong = session.trace(&json!({}));
    let right = session.trace(&json!({ "invert": true }));

    // The disc is 36 pixels across on a 64-pixel canvas. Traced the wrong way
    // round, the shape found is the navy that surrounds it.
    assert!(number(&wrong, "bounding_box.width") > 60.0, "{wrong}");
    assert!(number(&right, "bounding_box.width") < 40.0, "{right}");
}

#[test]
fn colour_tracing_finds_each_disc_as_its_own_path() {
    let fixture = corpus::named("multicolour");
    let mut session = Session::open(&fixture, &fixture.construction);
    let trace = session.trace(&json!({ "mode": "colour" }));

    assert!(
        trace["path_count"].as_u64().expect("a count") >= 3,
        "{trace}"
    );
    let fills: std::collections::BTreeSet<_> = trace["paths"]
        .as_array()
        .expect("a list")
        .iter()
        .filter_map(|path| path["fill_colour"].as_str())
        .collect();
    assert!(fills.len() >= 3, "{fills:?}");
}

#[test]
fn an_exact_construction_scores_as_a_match_on_every_metric() {
    let fixture = corpus::named("multicolour");
    let comparison = Session::open(&fixture, &fixture.construction).compare();

    // Anti-aliasing means never *exactly* 1.0, which is why these are
    // thresholds with the reason they hold, not equalities.
    assert!(overlap(&comparison) > 0.95, "{comparison:#}");
    assert!(number(&comparison, "edges.intersection_over_union") > 0.5);
    assert!(number(&comparison, "pixel_error.mean_absolute_error") < 0.02);
    assert!(number(&comparison, "perceptual_similarity.score") > 0.9);
    assert!(number(&comparison, "centroid.offset.dx").abs() < 0.5);
    assert!(number(&comparison, "centroid.offset.dy").abs() < 0.5);
}

#[test]
fn a_translated_construction_reports_the_offset_it_was_moved_by() {
    let fixture = corpus::named("multicolour");
    // Offsets are render minus reference, so moving right and down is
    // positive. (3, 5) keeps every disc on the canvas: an edge would clip the
    // render and move the centroid for a reason that is not the translation.
    let body = format!(
        r#"<g transform="translate(3 5)">{}</g>"#,
        fixture.construction
    );
    let comparison = Session::open(&fixture, &body).compare();

    assert!((number(&comparison, "centroid.offset.dx") - 3.0).abs() < 0.5);
    assert!((number(&comparison, "centroid.offset.dy") - 5.0).abs() < 0.5);
    assert!((number(&comparison, "bounding_box.offset.dx") - 3.0).abs() < 1.0);
    assert!((number(&comparison, "bounding_box.offset.dy") - 5.0).abs() < 1.0);
    assert!(
        overlap(&comparison) < 0.9,
        "a moved mark still overlaps perfectly"
    );
}

#[test]
fn a_half_size_construction_reports_half_the_width() {
    let fixture = corpus::named("multicolour");
    // Scaled about the canvas centre, so the offset stays near zero and it is
    // only the size that differs.
    let body = format!(
        r#"<g transform="translate(16 16) scale(0.5)">{}</g>"#,
        fixture.construction
    );
    let comparison = Session::open(&fixture, &body).compare();

    assert!((number(&comparison, "bounding_box.width_ratio") - 0.5).abs() < 0.05);
    assert!((number(&comparison, "bounding_box.height_ratio") - 0.5).abs() < 0.1);
    assert!(number(&comparison, "bounding_box.offset.dx").abs() < 1.0);
    assert!(overlap(&comparison) < 0.5);
}

#[test]
fn a_construction_with_nothing_drawn_overlaps_nothing() {
    let fixture = corpus::named("multicolour");
    let comparison = Session::open(&fixture, "").compare();

    assert_eq!(overlap(&comparison), 0.0);
    // No render foreground means no box to measure an offset from, which the
    // report says as `null` rather than as an offset of zero.
    assert!(
        comparison["bounding_box"]["offset"].is_null(),
        "{comparison:#}"
    );
}

#[test]
fn the_right_shape_in_the_wrong_colour_is_a_colour_error_and_not_a_shape_error() {
    let fixture = corpus::named("multicolour");
    let exact = Session::open(&fixture, &fixture.construction).compare();
    let grey = Session::open(&fixture, &corpus::discs(["#808080"; 3])).compare();

    // The foreground masks do not care what colour a pixel is...
    assert!(overlap(&grey) > 0.95, "{grey:#}");
    // ...which is exactly why the report carries per-channel error as a
    // separate field, and never folds the two into one score.
    let exact_error = number(&exact, "pixel_error.mean_absolute_error");
    let grey_error = number(&grey, "pixel_error.mean_absolute_error");
    assert!(grey_error > 0.02, "{grey_error}");
    assert!(
        grey_error > 3.0 * exact_error,
        "{grey_error} vs {exact_error}"
    );
}

#[test]
fn reconstruction_improves_with_each_iteration_until_it_converges() {
    let fixture = corpus::named("multicolour");
    let mut session = Session::open(&fixture, &fixture.construction);

    // Small, and off to one side: a placement that is wrong in both position
    // and size, so there is something to improve in two separate steps.
    let start = Placement {
        x: 12.0,
        y: -9.0,
        scale: 0.5,
    };
    let history = run_loop(&mut session, &fixture.construction, start, 8);

    // A loop that started right, or never changed anything, proves nothing.
    assert!(
        history[0] < 0.5,
        "the starting point was already good: {history:?}"
    );
    assert!(history.len() >= 3, "it converged in one step: {history:?}");
    assert!(history.len() < 8, "it never converged: {history:?}");

    for pair in history.windows(2) {
        assert!(pair[1] > pair[0], "an iteration made it worse: {history:?}");
    }
    let last = *history.last().expect("at least one comparison");
    assert!(last > 0.9, "it converged to a poor match: {history:?}");

    insta::assert_snapshot!("improvement_history", snapshot_text(json!(history), 2));
}

#[test]
fn the_refinement_policy_does_nothing_when_the_comparison_shows_nothing_to_fix() {
    // If this were not true the loop above would run to its limit and the
    // "converges" assertion would be about the limit, not the policy.
    let fixture = corpus::named("multicolour");
    let comparison = Session::open(&fixture, &fixture.construction).compare();
    let placement = Placement {
        x: 0.0,
        y: 0.0,
        scale: 1.0,
    };
    assert_eq!(harness::refine(placement, &comparison), None);
}

#[test]
fn a_hand_built_mark_beats_a_trace_at_matching_what_was_meant_and_loses_at_matching_the_pixels() {
    let fixture = corpus::named("noisy");
    let mut session = Session::open(&fixture, &fixture.construction);

    let trace = session.trace(&json!({}));
    let path_count = trace["path_count"].as_u64().expect("a count");
    // The noise survives the trace: dozens of paths for one disc, and more
    // than the tool will list.
    assert!(path_count > 32, "{path_count} paths");
    assert_eq!(trace["paths"].as_array().expect("a list").len(), 32);

    let truth = corpus::noisy_truth();
    let against_truth = |render: &[u8]| {
        compare::compare(
            Path::new("truth.png"),
            &truth,
            Path::new("render.png"),
            render,
        )
        .expect("the render and the truth are the same size")
        .0
        .foreground
        .intersection_over_union
    };

    session.draw(trace_body(trace["svg"].as_str().expect("markup")));
    let trace_render = session.render(64);
    let trace_vs_reference = overlap(&session.compare());

    session.draw(&fixture.construction);
    let built_render = session.render(64);
    let built_vs_reference = overlap(&session.compare());

    // Against what the artist meant (the clean disc), the hand-built mark wins
    // by a wide margin, because the trace reproduced every speck...
    let trace_vs_truth = against_truth(&trace_render);
    let built_vs_truth = against_truth(&built_render);
    assert!(built_vs_truth > 0.9, "{built_vs_truth}");
    assert!(trace_vs_truth < 0.8, "{trace_vs_truth}");

    // ...and against the reference itself, the order reverses. That is the
    // limit of the comparison, recorded rather than hidden: `compare_reference`
    // measures fidelity to pixels, so an agent chasing its overlap alone would
    // learn to copy the noise. Choosing the strategy is what prevents that.
    assert!(
        trace_vs_reference > built_vs_reference,
        "trace {trace_vs_reference} vs built {built_vs_reference}"
    );
}

#[test]
fn the_loop_visits_the_phases_in_the_order_the_workflow_names_them() {
    let fixture = corpus::named("multicolour");
    let mut session = Session::open(&fixture, &fixture.construction);
    let start = Placement {
        x: 12.0,
        y: -9.0,
        scale: 0.5,
    };
    run_loop(&mut session, &fixture.construction, start, 8);

    // Which phase first appears where. Repeats are the point of a loop, so
    // only the order of first visits is compared.
    let mut first_visits = Vec::new();
    for phase in session.phases() {
        if !first_visits.contains(phase) {
            first_visits.push(*phase);
        }
    }

    let expected = Workflow::for_kind(WorkflowKind::Reference).phases;
    // `Validate` is the decision that a comparison is good enough. It has no
    // tool to call, so it is the assertions about the final overlap in the
    // test above, not a step this record can contain.
    assert_eq!(expected.last(), Some(&Phase::Validate));
    assert_eq!(first_visits, expected[..expected.len() - 1]);

    // And it is a loop: refining sends it back through render and compare.
    let refines = session
        .phases()
        .iter()
        .filter(|phase| **phase == Phase::Refine)
        .count();
    let compares = session
        .phases()
        .iter()
        .filter(|phase| **phase == Phase::Compare)
        .count();
    assert!(refines >= 2 && compares > refines, "{:?}", session.phases());
}

#[test]
fn rendering_a_reconstruction_twice_produces_identical_bytes() {
    // Invariant 1 of AGENTS.md, applied to the loop rather than to a project
    // on its own: two sessions, built independently from the same fixture and
    // drawn the same way, render the same PNG.
    let fixture = corpus::named("holes");
    let render = || {
        let mut session = Session::open(&fixture, "");
        session.draw(&fixture.construction);
        session.render(128)
    };
    assert_eq!(render(), render());
}

#[test]
fn comparing_a_reconstruction_twice_produces_identical_reports() {
    let fixture = corpus::named("holes");
    let report = || {
        let mut session = Session::open(&fixture, &fixture.construction);
        session.compare()
    };
    assert_eq!(report(), report());
}
