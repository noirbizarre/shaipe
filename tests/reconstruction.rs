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
fn the_appearance_controls_are_deterministic_and_small_enough_to_redistribute() {
    let controls = || {
        vec![
            corpus::linear_gradient(),
            corpus::radial_gradient(),
            corpus::translucent_fill(),
            corpus::hard_step(),
            corpus::speckled_fill(),
            corpus::antialiased_gradient(),
            corpus::stroked_ring(),
            corpus::stroked_bar(),
            corpus::translucent_overlay(),
            corpus::near_flat_ramp(),
            corpus::blocky_artefacts(),
        ]
    };
    for (a, b) in controls().iter().zip(&controls()) {
        assert_eq!(a.png, b.png, "`{}` differs between two generations", a.name);
        assert!(
            a.png.len() < 8 * 1024,
            "`{}` is {} bytes",
            a.name,
            a.png.len()
        );
    }
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

/// The fill of the first region, as the report says it.
fn first_fill(analysis: &Value) -> &Value {
    &analysis["appearance"]["regions"][0]["fill"]
}

#[test]
fn a_linear_gradient_is_reported_with_its_axis_and_end_colours() {
    let fixture = corpus::linear_gradient();
    let analysis = Session::open(&fixture, &fixture.construction).analysis();
    let fill = first_fill(&analysis);

    assert_eq!(fill["kind"], "linear_gradient", "{fill}");
    assert!(number(fill, "angle_degrees").abs() <= 2.0, "{fill}");
    let stops = fill["stops"].as_array().expect("stops");
    assert_eq!(stops.first().unwrap()["colour"], "#c81e1e", "{fill}");
    assert_eq!(stops.last().unwrap()["colour"], "#1e28d2", "{fill}");
    // The axis runs across the square, left to right, along its middle row.
    assert!(
        number(fill, "start.x") < 16.0 && number(fill, "end.x") > 48.0,
        "{fill}"
    );
    insta::assert_snapshot!("analysis_linear_gradient", snapshot_text(analysis, 1));
}

#[test]
fn a_radial_gradient_is_reported_with_its_centre_and_extent() {
    let fixture = corpus::radial_gradient();
    let analysis = Session::open(&fixture, &fixture.construction).analysis();
    let fill = first_fill(&analysis);

    assert_eq!(fill["kind"], "radial_gradient", "{fill}");
    // Drawn about (32, 32) with radius 22.
    assert!((number(fill, "centre.x") - 32.0).abs() <= 2.0, "{fill}");
    assert!((number(fill, "centre.y") - 32.0).abs() <= 2.0, "{fill}");
    assert!((16.0..=24.0).contains(&number(fill, "radius")), "{fill}");
    insta::assert_snapshot!("analysis_radial_gradient", snapshot_text(analysis, 1));
}

#[test]
fn transparency_is_measured_apart_from_colour() {
    let fixture = corpus::translucent_fill();
    let analysis = Session::open(&fixture, &fixture.construction).analysis();
    let region = &analysis["appearance"]["regions"][0];

    // One flat colour, at about half strength: alpha is not folded into RGB.
    assert_eq!(region["fill"]["kind"], "flat", "{region}");
    assert_eq!(region["fill"]["colour"], "#c81e1e", "{region}");
    assert!(
        (number(region, "opacity.mean") - 128.0 / 255.0).abs() < 1e-6,
        "{region}"
    );
    assert!(number(&analysis, "appearance.alpha.interior_translucent_fraction") > 0.3);
    insta::assert_snapshot!("analysis_translucent_fill", snapshot_text(analysis, 1));
}

#[test]
fn colour_variation_that_is_not_a_gradient_is_not_reported_as_one() {
    // The controls that make the tests above able to fail: a detector that
    // says "gradient" for any colour variation passes all of them.
    let mut cases = vec![
        corpus::hard_step(),
        corpus::speckled_fill(),
        corpus::near_flat_ramp(),
        corpus::blocky_artefacts(),
    ];
    // Every corpus fixture has anti-aliased edges and `noisy` has speckle.
    cases.extend(corpus::all());
    for fixture in cases {
        let analysis = Session::open(&fixture, &fixture.construction).analysis();
        for region in analysis["appearance"]["regions"]
            .as_array()
            .expect("regions")
        {
            let kind = region["fill"]["kind"].as_str().expect("a kind");
            assert!(
                matches!(kind, "flat" | "varied"),
                "`{}`: region {} reported as {kind}: {}",
                fixture.name,
                region["region_id"],
                region["fill"]
            );
        }
    }
}

#[test]
fn a_step_between_two_colours_is_varied_with_a_high_linear_fit() {
    let fixture = corpus::hard_step();
    let analysis = Session::open(&fixture, &fixture.construction).analysis();
    let fill = first_fill(&analysis);

    assert_eq!(fill["kind"], "varied", "{fill}");
    assert!(number(fill, "linear_fit") > 0.9, "{fill}");
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

/// A colour trace with appearance awareness on (the default) or off.
fn colour_trace(session: &mut Session, appearance: bool) -> Value {
    session.trace(&json!({ "mode": "colour", "appearance": appearance }))
}

/// A trace with the markup removed and its paths cut short, as a golden.
fn trace_golden(mut trace: Value) -> String {
    let object = trace.as_object_mut().expect("an object");
    object.remove("svg");
    if let Some(Value::Array(paths)) = object.get_mut("paths") {
        paths.truncate(4);
    }
    snapshot_text(trace, 1)
}

#[test]
fn a_gradient_is_traced_as_one_painted_path_rather_than_a_stack_of_layers() {
    for fixture in [corpus::linear_gradient(), corpus::radial_gradient()] {
        let mut session = Session::open(&fixture, &fixture.construction);

        let plain = colour_trace(&mut session, false);
        let aware = colour_trace(&mut session, true);

        // Without it, one fade is a pile of unrelated colours; the pile is the
        // reason this exists, so the test fails if the pile stops being one.
        assert!(
            plain["path_count"].as_u64().expect("a count") > 5,
            "`{}`: {plain}",
            fixture.name
        );
        assert_eq!(aware["path_count"], 1, "`{}`: {aware}", fixture.name);
        let kind = &aware["paths"][0]["appearance"]["fill"]["kind"];
        assert_eq!(
            kind.as_str().map(|kind| kind.trim_end_matches("_gradient")),
            fixture.name.strip_suffix("_gradient"),
            "`{}`: {aware}",
            fixture.name
        );
        insta::assert_snapshot!(format!("trace_{}", fixture.name), trace_golden(aware));
    }
}

#[test]
fn a_translucent_fill_is_traced_with_the_opacity_a_trace_would_drop() {
    let fixture = corpus::translucent_fill();
    let mut session = Session::open(&fixture, &fixture.construction);

    let plain = colour_trace(&mut session, false);
    let aware = colour_trace(&mut session, true);

    assert!(
        !plain["svg"]
            .as_str()
            .expect("markup")
            .contains("fill-opacity"),
        "{plain}"
    );
    let svg = aware["svg"].as_str().expect("markup");
    assert!(svg.contains(r#"fill-opacity="0.5""#), "{svg}");
    assert_eq!(aware["paths"][0]["fill_colour"], "#c81e1e");
    insta::assert_snapshot!("trace_translucent_fill", trace_golden(aware));
}

#[test]
fn a_hard_step_is_reported_as_a_fallback_and_traced_as_it_always_was() {
    // The counterpart that lets the tests above fail: a trace that lifted
    // anything varied would pass them, and would invent a gradient here.
    let fixture = corpus::hard_step();
    let mut session = Session::open(&fixture, &fixture.construction);

    let plain = colour_trace(&mut session, false);
    let aware = colour_trace(&mut session, true);

    assert_eq!(aware["svg"], plain["svg"]);
    assert_eq!(aware["fallbacks"][0]["region_id"], 0, "{aware}");
    assert!(plain.get("fallbacks").is_none(), "{plain}");
}

#[test]
fn a_traced_gradient_can_be_read_as_outline_and_as_fill_separately() {
    let fixture = corpus::gradient_badge();
    let mut session = Session::open(&fixture, &fixture.construction);
    let analysis = session.analysis();
    let aware = colour_trace(&mut session, true);

    let lifted: Vec<&Value> = aware["paths"]
        .as_array()
        .expect("a list")
        .iter()
        .filter(|path| path.get("region_id").is_some())
        .collect();
    assert_eq!(lifted.len(), 1, "only the panel is a gradient: {aware}");
    let path = lifted[0];

    // The outline joins the analysis by `region_id`: the geometry the trace
    // measured lands on the region the analysis measured.
    let region = analysis["regions"]
        .as_array()
        .expect("regions")
        .iter()
        .find(|region| region["id"] == path["region_id"])
        .expect("the region the path came from");
    for (traced, measured) in [
        ("bounding_box.x", "bounding_box.x"),
        ("bounding_box.y", "bounding_box.y"),
        ("bounding_box.width", "bounding_box.width"),
        ("bounding_box.height", "bounding_box.height"),
    ] {
        assert!(
            (number(path, traced) - number(region, measured)).abs() <= 1.5,
            "{traced}: {path} against {region}"
        );
    }

    // The fill is the analysis's own, unchanged, so what an agent reads from
    // either tool is one fact.
    assert_eq!(
        path["appearance"]["fill"],
        analysis["appearance"]["regions"]
            .as_array()
            .expect("regions")
            .iter()
            .find(|entry| entry["region_id"] == path["region_id"])
            .expect("the region's appearance")["fill"]
    );
    insta::assert_snapshot!("trace_gradient_badge", trace_golden(aware));
}

#[test]
fn keeping_a_gradient_as_a_gradient_reconstructs_it_more_faithfully_than_its_layers() {
    let fixture = corpus::gradient_badge();
    let mut session = Session::open(&fixture, &fixture.construction);

    let aware = colour_trace(&mut session, true);
    let plain = colour_trace(&mut session, false);

    session.draw(trace_body(plain["svg"].as_str().expect("markup")));
    let plain_error = number(&session.compare(), "pixel_error.mean_absolute_error");
    session.draw(trace_body(aware["svg"].as_str().expect("markup")));
    let comparison = session.compare();
    let aware_error = number(&comparison, "pixel_error.mean_absolute_error");

    // The hybrid is both smaller and closer: two paths, not dozens, and the
    // ramp is drawn as a ramp instead of as its nearest few colours.
    assert!(aware["path_count"].as_u64() < plain["path_count"].as_u64());
    assert!(
        aware_error < plain_error,
        "aware {aware_error} against plain {plain_error}"
    );
    assert!(overlap(&comparison) > 0.9, "{comparison:#}");

    // The window in the panel is still a window: the lifted outline kept its
    // hole, and the gradient is painted around it and not across it.
    let render = image::load_from_memory(&session.render(64))
        .expect("a PNG")
        .to_rgba8();
    assert_eq!(render.get_pixel(32, 24)[3], 0, "the window is filled in");
    assert_eq!(render.get_pixel(16, 24)[3], 255, "the panel is missing");
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

fn findings(comparison: &serde_json::Value) -> Vec<String> {
    comparison["appearance"]["findings"]
        .as_array()
        .expect("appearance.findings is a list")
        .iter()
        .map(|f| f.as_str().expect("a sentence").to_owned())
        .collect()
}

#[test]
fn an_exact_gradient_construction_has_no_appearance_findings() {
    let fixture = corpus::linear_gradient();
    let comparison = Session::open(&fixture, &fixture.construction).compare();

    assert!(findings(&comparison).is_empty(), "{comparison:#}");
}

#[test]
fn a_flat_colour_in_place_of_a_gradient_is_the_right_shape_with_the_wrong_fill() {
    let fixture = corpus::linear_gradient();
    let flat = r##"<rect x="12" y="12" width="40" height="40" fill="#800080"/>"##;
    let comparison = Session::open(&fixture, flat).compare();

    // The silhouette is right, which is why appearance is reported apart.
    assert!(overlap(&comparison) > 0.95, "{comparison:#}");
    let found = findings(&comparison);
    assert!(
        found
            .iter()
            .any(|f| f.contains("linear_gradient") && f.contains("linearGradient")),
        "{found:?}"
    );
}

#[test]
fn a_reversed_gradient_is_reported_as_a_colour_error_on_the_same_shape() {
    let fixture = corpus::linear_gradient();
    let reversed = fixture.construction.replace(
        r#"x1="12" y1="0" x2="52" y2="0""#,
        r#"x1="52" y1="0" x2="12" y2="0""#,
    );
    let comparison = Session::open(&fixture, &reversed).compare();

    assert!(overlap(&comparison) > 0.95, "{comparison:#}");
    assert!(
        findings(&comparison)
            .iter()
            .any(|f| f.contains("colours differ")),
        "{comparison:#}"
    );
}

#[test]
fn an_opaque_render_of_a_translucent_reference_reports_the_opacity() {
    let fixture = corpus::translucent_fill();
    let matching = Session::open(&fixture, &fixture.construction).compare();
    let opaque = fixture.construction.replace(" fill-opacity=\"0.5\"", "");
    let comparison = Session::open(&fixture, &opaque).compare();

    assert!(findings(&matching).is_empty(), "{matching:#}");
    assert!(
        number(&comparison, "appearance.alpha.interior_translucent_delta") < -0.05,
        "{comparison:#}"
    );
    assert!(!findings(&comparison).is_empty(), "{comparison:#}");
}

#[test]
fn a_gradient_with_antialiased_edges_is_still_a_gradient() {
    let fixture = corpus::antialiased_gradient();
    let analysis = Session::open(&fixture, &fixture.construction).analysis();
    let fill = first_fill(&analysis);

    assert_eq!(fill["kind"], "radial_gradient", "{fill}");
    assert!((number(fill, "centre.x") - 32.0).abs() <= 2.0, "{fill}");
    insta::assert_snapshot!("analysis_antialiased_gradient", snapshot_text(analysis, 1));
}

#[test]
fn a_stroked_ring_is_a_closed_stroke_and_a_bar_an_open_one() {
    let ring = corpus::stroked_ring();
    let analysis = Session::open(&ring, &ring.construction).analysis();
    let stroke = &analysis["appearance"]["regions"][0]["stroke"];
    assert!((number(stroke, "width") - 4.0).abs() <= 1.5, "{stroke}");
    assert_eq!(stroke["closed"], true, "{stroke}");

    let bar = corpus::stroked_bar();
    let analysis = Session::open(&bar, &bar.construction).analysis();
    let stroke = &analysis["appearance"]["regions"][0]["stroke"];
    assert!((number(stroke, "width") - 3.0).abs() <= 1.0, "{stroke}");
    assert_eq!(stroke["closed"], false, "{stroke}");
}

#[test]
fn a_translucent_overlay_keeps_its_alpha_apart_from_its_colour() {
    let fixture = corpus::translucent_overlay();
    let analysis = Session::open(&fixture, &fixture.construction).analysis();

    assert!(number(&analysis, "appearance.alpha.interior_translucent_fraction") > 0.1);
    insta::assert_snapshot!("analysis_translucent_overlay", snapshot_text(analysis, 1));
}

#[test]
fn an_exact_overlay_has_no_findings_and_a_dropped_opacity_has_some() {
    let fixture = corpus::translucent_overlay();
    let matching = Session::open(&fixture, &fixture.construction).compare();
    let opaque = fixture.construction.replace(" fill-opacity=\"0.5\"", "");
    let comparison = Session::open(&fixture, &opaque).compare();

    assert!(findings(&matching).is_empty(), "{matching:#}");
    assert!(!findings(&comparison).is_empty(), "{comparison:#}");
}

#[test]
fn a_stroke_drawn_too_thick_is_not_an_exact_match() {
    let fixture = corpus::stroked_ring();
    let matching = Session::open(&fixture, &fixture.construction).compare();
    let thick = fixture
        .construction
        .replace("stroke-width=\"4\"", "stroke-width=\"9\"");
    let comparison = Session::open(&fixture, &thick).compare();

    assert!(overlap(&matching) > overlap(&comparison), "{comparison:#}");
}

// --- typography (ADR 028) ---------------------------------------------------

fn text_fixtures() -> Vec<corpus::Fixture> {
    vec![
        corpus::text_line(),
        corpus::coloured_words(),
        corpus::centred_block(),
        corpus::stacked_letters(),
    ]
}

/// Everything that is a reference but not lettering: the corpus, the
/// appearance controls and the typography controls.
fn fixtures_without_lettering() -> Vec<corpus::Fixture> {
    let mut fixtures = corpus::all();
    fixtures.extend([
        corpus::linear_gradient(),
        corpus::radial_gradient(),
        corpus::translucent_fill(),
        corpus::hard_step(),
        corpus::speckled_fill(),
        corpus::antialiased_gradient(),
        corpus::stroked_ring(),
        corpus::stroked_bar(),
        corpus::translucent_overlay(),
        corpus::near_flat_ramp(),
        corpus::blocky_artefacts(),
        corpus::scattered_shapes(),
        corpus::row_of_dots(),
        corpus::single_large_letter(),
    ]);
    fixtures
}

fn typography_of(fixture: &corpus::Fixture) -> Value {
    Session::open(fixture, &fixture.construction).analysis()["typography"].clone()
}

#[test]
fn the_text_fixtures_are_deterministic_and_small_enough_to_redistribute() {
    for (a, b) in text_fixtures().iter().zip(&text_fixtures()) {
        assert_eq!(a.png, b.png, "`{}` differs between two generations", a.name);
        assert!(
            a.png.len() < 8 * 1024,
            "`{}` is {} bytes",
            a.name,
            a.png.len()
        );
    }
}

#[test]
fn a_line_of_mixed_case_text_is_found_with_its_baseline_heights_and_marks() {
    let fixture = corpus::text_line();
    let typography = typography_of(&fixture);
    let line = &typography["lines"][0];

    assert_eq!(typography["line_count"], 1, "{typography:#}");
    assert_eq!(line["orientation"], "horizontal");
    assert_eq!(line["baseline"], 36.0, "drawn standing on y=36");
    assert_eq!(line["baseline_edge"], "bottom");
    assert_eq!(line["tall_height"], 12.0, "the capital");
    assert_eq!(line["short_height"], 8.0, "the short letters");
    assert_eq!(line["glyph_count"], 6, "the dot is not a letter");
    assert_eq!(line["mark_count"], 1, "the dot over the stem");
    assert_eq!(line["off_baseline_count"], 1, "the descender");
    assert_eq!(line["words"].as_array().expect("words").len(), 1);
    insta::assert_snapshot!("typography_text_line", snapshot_text(typography, 1));
}

#[test]
fn two_words_are_split_and_each_keeps_its_own_appearance() {
    let fixture = corpus::coloured_words();
    let typography = typography_of(&fixture);
    let line = &typography["lines"][0];
    let words = line["words"].as_array().expect("words");

    assert_eq!(words.len(), 2, "{typography:#}");
    assert_eq!(words[0]["appearance"]["uniform"], true);
    assert_eq!(words[0]["appearance"]["colour"], "#18181b");
    assert_eq!(words[1]["appearance"]["colour"], "#c81e1e");
    assert_eq!(
        line["appearance"]["uniform"], false,
        "the line as a whole is two colours: {line:#}"
    );
    insta::assert_snapshot!("typography_coloured_words", snapshot_text(typography, 1));
}

#[test]
fn two_centred_lines_are_one_block_with_their_alignment_and_pitch() {
    let fixture = corpus::centred_block();
    let typography = typography_of(&fixture);
    let block = &typography["blocks"][0];

    assert_eq!(typography["line_count"], 2, "{typography:#}");
    assert_eq!(block["alignment"], "centre", "{block}");
    // Baselines at y=26 and y=44.
    assert_eq!(block["line_pitch"], 18.0, "{block}");
    insta::assert_snapshot!("typography_centred_block", snapshot_text(typography, 1));
}

#[test]
fn stacked_letters_are_vertical_lettering() {
    let fixture = corpus::stacked_letters();
    let typography = typography_of(&fixture);
    let line = &typography["lines"][0];

    assert_eq!(line["orientation"], "vertical", "{typography:#}");
    assert_eq!(line["baseline_edge"], "centre");
    assert_eq!(line["baseline"], 32.0, "centred on x=32");
    insta::assert_snapshot!("typography_stacked_letters", snapshot_text(typography, 1));
}

#[test]
fn shapes_that_are_not_lettering_are_not_reported_as_lettering() {
    for fixture in fixtures_without_lettering() {
        let analysis = Session::open(&fixture, &fixture.construction).analysis();
        assert!(
            analysis.get("typography").is_none(),
            "`{}` was reported as lettering: {:#}",
            fixture.name,
            analysis["typography"]
        );
    }
}

#[test]
fn typography_is_measured_identically_every_time() {
    for fixture in text_fixtures() {
        assert_eq!(
            typography_of(&fixture),
            typography_of(&fixture),
            "`{}`",
            fixture.name
        );
    }
}

fn baseline_offset(comparison: &Value) -> f64 {
    comparison["typography"]["lines"][0]["baseline_offset"]
        .as_f64()
        .unwrap_or_else(|| panic!("no baseline offset in {comparison:#}"))
}

fn typography_findings(comparison: &Value) -> Vec<String> {
    comparison["typography"]["findings"]
        .as_array()
        .map(|list| {
            list.iter()
                .map(|f| f.as_str().expect("a sentence").to_owned())
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn an_exact_reconstruction_of_lettering_has_no_typography_findings() {
    for fixture in text_fixtures() {
        let comparison = Session::open(&fixture, &fixture.construction).compare();

        assert!(
            typography_findings(&comparison).is_empty(),
            "`{}`: {comparison:#}",
            fixture.name
        );
        assert!(
            comparison["typography"]["reference_line_count"].as_u64() > Some(0),
            "`{}` should be compared as lettering",
            fixture.name
        );
    }
}

#[test]
fn lettering_set_too_low_says_how_far_and_which_way_to_move_it() {
    let fixture = corpus::text_line();
    let shifted = format!(
        r#"<g transform="translate(0 4)">{}</g>"#,
        fixture.construction
    );
    let comparison = Session::open(&fixture, &shifted).compare();

    assert_eq!(baseline_offset(&comparison), 4.0, "{comparison:#}");
    assert!(
        typography_findings(&comparison)
            .iter()
            .any(|f| f.contains("up") && f.contains("4.0px")),
        "{comparison:#}"
    );
}

#[test]
fn lettering_set_too_small_says_how_much_to_scale_the_font() {
    let fixture = corpus::text_line();
    // Half the size, standing on the same baseline.
    let small = format!(
        r#"<g transform="translate(0 18) scale(1 0.5)">{}</g>"#,
        fixture.construction
    );
    let comparison = Session::open(&fixture, &small).compare();

    assert!(
        typography_findings(&comparison)
            .iter()
            .any(|f| f.contains("shorter") && f.contains("font-size")),
        "{comparison:#}"
    );
}

#[test]
fn lettering_that_is_missing_is_reported_where_the_reference_has_it() {
    let fixture = corpus::text_line();
    let comparison = Session::open(&fixture, "").compare();

    assert!(
        typography_findings(&comparison)
            .iter()
            .any(|f| f.contains("render sets none")),
        "{comparison:#}"
    );
    // Without a `<text>`, the comparison says so as well.
    assert!(
        comparison["declared_text"]["findings"]
            .to_string()
            .contains("no `<text>`"),
        "{comparison:#}"
    );
}

#[test]
fn two_words_set_in_one_colour_are_not_the_two_colours_the_reference_has() {
    let fixture = corpus::coloured_words();
    let one_colour = fixture.construction.replace("#c81e1e", "#18181b");
    let comparison = Session::open(&fixture, &one_colour).compare();

    assert!(
        typography_findings(&comparison)
            .iter()
            .any(|f| f.contains("one colour")),
        "{comparison:#}"
    );
}

#[test]
fn a_baseline_found_by_comparison_alone_is_enough_to_fix_it() {
    // The model-free version of what an agent does: read the offset the
    // comparison reports, move the text by it, and compare again.
    let fixture = corpus::text_line();
    let wrap = |dy: f64| {
        format!(
            r#"<g transform="translate(0 {dy})">{}</g>"#,
            fixture.construction
        )
    };

    let mut session = Session::open(&fixture, &wrap(5.0));
    let before = session.compare();
    let offset = baseline_offset(&before);
    assert_eq!(offset, 5.0);

    session.draw(&wrap(5.0 - offset));
    let after = session.compare();

    assert!(overlap(&after) > overlap(&before), "{after:#}");
    assert!(typography_findings(&after).is_empty(), "{after:#}");
    assert_eq!(baseline_offset(&after), 0.0);
}

#[test]
fn a_text_line_can_be_looked_at_enlarged_by_its_own_bounding_box() {
    let fixture = corpus::text_line();
    let mut session = Session::open(&fixture, &fixture.construction);
    let line = session.analysis()["typography"]["lines"][0].clone();
    let b = &line["bounding_box"];

    let output = session.call(
        "get_reference_image",
        &json!({
            "src": "reference.png",
            "x": b["x"], "y": b["y"], "width": b["width"], "height": b["height"],
            "scale": 4,
        }),
    );

    let (width, height) = (b["width"].as_u64().unwrap(), b["height"].as_u64().unwrap());
    assert_eq!(output.value["crop"]["output"]["width"], width * 4);
    assert_eq!(output.value["crop"]["output"]["height"], height * 4);
}

#[test]
fn text_a_variant_declares_is_reported_with_the_confidence_it_was_given() {
    let fixture = corpus::text_line();
    let body = format!(
        r#"{}<text x="6" y="36" font-size="16" font-family="sans-serif" data-shaipe-confidence="low" data-shaipe-note="the fourth letter may be an l">Acme</text>"#,
        fixture.construction
    );
    let comparison = Session::open(&fixture, &body).compare();
    let declared = &comparison["declared_text"];

    assert_eq!(declared["elements"][0]["content"], "Acme");
    assert!(
        declared["findings"]
            .to_string()
            .contains("low-confidence text"),
        "{declared:#}"
    );
}
