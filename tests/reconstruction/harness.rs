//! Driving the reconstruction tools the way an agent would, with no agent.
//!
//! [`Session`] is a project in a temporary directory and the real tool
//! [`Registry`], and every call goes through the same `Registry::call` the MCP
//! server and the workspace use. [`refine`] is a stand-in for the model: it
//! reads a comparison and moves the artwork. It is deliberately not clever —
//! it can only use what `compare_reference` *reports*, which is exactly the
//! information the real loop gives a model, so if the numbers were not enough
//! to steer by, this loop would stall and a test would say so.

use serde_json::{Value, json};
use shaipe::Project;
use shaipe::tools::{Registry, ToolOutput};
use shaipe::workflow::Phase;

use crate::corpus::{Fixture, symbol};

/// A project on disk, with a fixture attached, and the record of what was done
/// to it.
pub struct Session {
    /// Held so the directory outlives the project that resolves references
    /// against it.
    _directory: tempfile::TempDir,
    project: Project,
    registry: Registry,
    phases: Vec<Phase>,
    compared: bool,
}

impl Session {
    /// Open a project for `fixture`, drawing `body` in its `icon` variant.
    pub fn open(fixture: &Fixture, body: &str) -> Self {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let path = directory.path().join("project.svg");
        std::fs::write(&path, fixture.project(body)).expect("the project is written");
        std::fs::write(directory.path().join("reference.png"), &fixture.png)
            .expect("the reference is written");

        Self {
            project: Project::open(&path).expect("the generated project is valid"),
            _directory: directory,
            registry: Registry::new(),
            phases: Vec::new(),
            compared: false,
        }
    }

    /// Call a tool, recording which phase of the workflow it belongs to.
    ///
    /// # Panics
    ///
    /// If the tool refuses. Every call this harness makes is one a correct
    /// agent could make, so a refusal is a regression rather than a case.
    pub fn call(&mut self, tool: &str, input: &Value) -> ToolOutput {
        let output = self
            .registry
            .call(tool, &mut self.project, input)
            .unwrap_or_else(|error| panic!("`{tool}` refused {input}: {error:?}"));
        if let Some(phase) = self.phase_of(tool) {
            self.phases.push(phase);
        }
        output
    }

    /// The phase a tool call belongs to, if it belongs to one.
    ///
    /// A write is `Construct` until the first comparison and `Refine` after
    /// it, because the tool is the same and only what came before differs.
    /// `Validate` has no tool: it is the decision that a comparison is good
    /// enough, made by whoever reads it.
    fn phase_of(&mut self, tool: &str) -> Option<Phase> {
        Some(match tool {
            "get_reference_image" => Phase::Inspect,
            "get_reference_analysis" => Phase::Analyse,
            "get_workflow" => Phase::ChooseStrategy,
            "render_svg" => Phase::Render,
            "compare_reference" => {
                self.compared = true;
                Phase::Compare
            }
            "get_reference_trace" | "write_variant" | "write_svg" => {
                if self.compared {
                    Phase::Refine
                } else {
                    Phase::Construct
                }
            }
            _ => return None,
        })
    }

    /// Every phase visited, in order, with repeats.
    pub fn phases(&self) -> &[Phase] {
        &self.phases
    }

    /// Replace the `icon` variant's contents with `body`.
    pub fn draw(&mut self, body: &str) {
        self.call(
            "write_variant",
            &json!({ "variant": "icon", "svg": symbol(body) }),
        );
    }

    /// `compare_reference` on the `icon` variant, as its JSON report.
    pub fn compare(&mut self) -> Value {
        self.call(
            "compare_reference",
            &json!({ "src": "reference.png", "variant": "icon" }),
        )
        .value
    }

    /// The `icon` variant rendered to PNG at `size` pixels square, transparent.
    pub fn render(&mut self, size: u32) -> Vec<u8> {
        let mut output = self.call(
            "render_svg",
            &json!({ "variant": "icon", "width": size, "height": size }),
        );
        output.images.remove(0).bytes
    }

    /// The reference's own analysis.
    pub fn analysis(&mut self) -> Value {
        self.call("get_reference_analysis", &json!({ "src": "reference.png" }))
            .value["analysis"]
            .clone()
    }

    /// `get_workflow` for reference work on this project's reference.
    pub fn workflow(&mut self) -> Value {
        self.call(
            "get_workflow",
            &json!({ "kind": "reference", "src": "reference.png" }),
        )
        .value
    }

    /// `get_reference_trace`, with whichever options a test wants.
    pub fn trace(&mut self, options: &Value) -> Value {
        let mut input = json!({ "src": "reference.png" });
        input
            .as_object_mut()
            .expect("an object")
            .extend(options.as_object().expect("options are an object").clone());
        self.call("get_reference_trace", &input).value["trace"].clone()
    }
}

/// The markup inside a traced document's `<svg>` element, ready to be grafted
/// into a symbol.
///
/// The trace is a standalone document; a variant is one element of a project
/// document. Grafting is the step the tool descriptions tell an agent to do,
/// so the test does it the way an agent would: keep the paths, drop the shell.
pub fn trace_body(svg: &str) -> &str {
    let start = svg.find("<svg").expect("a traced document has a root");
    let open_end = start + svg[start..].find('>').expect("the root tag closes") + 1;
    let close = svg.rfind("</svg>").expect("the root is closed");
    &svg[open_end..close]
}

/// A number from a JSON report, by path.
///
/// # Panics
///
/// If the path is missing or not a number: a report that stopped carrying a
/// field is exactly the regression this harness exists to catch, so it must
/// not read as `0.0`.
pub fn number(report: &Value, path: &str) -> f64 {
    path.split('.')
        .try_fold(report, |value, key| value.get(key))
        .and_then(Value::as_f64)
        .unwrap_or_else(|| panic!("`{path}` is not a number in {report:#}"))
}

/// Foreground intersection-over-union from a comparison.
pub fn overlap(comparison: &Value) -> f64 {
    number(comparison, "foreground.intersection_over_union")
}

/// Where the artwork sits: a translation and a uniform scale, applied to a
/// group around the true geometry.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    pub x: f64,
    pub y: f64,
    pub scale: f64,
}

impl Placement {
    /// The markup for `body` under this placement.
    pub fn markup(self, body: &str) -> String {
        format!(
            r#"<g transform="translate({:.3} {:.3}) scale({:.4})">{body}</g>"#,
            self.x, self.y, self.scale
        )
    }
}

/// Centre of a bounding box in a comparison, as `(x, y)`.
fn centre(comparison: &Value, which: &str) -> (f64, f64) {
    let at = |field: &str| number(comparison, &format!("bounding_box.{which}.{field}"));
    (at("x") + at("width") / 2.0, at("y") + at("height") / 2.0)
}

/// The next placement, chosen from a comparison alone. `None` when nothing it
/// reports is worth acting on.
///
/// Position first, then size: a mark that is the wrong size *and* in the wrong
/// place has a bounding-box ratio that is still true but an offset that moves
/// when the size is fixed, so the offset is settled first. Correcting size
/// moves the centre, so it is followed in the same step by re-centring, which
/// the two boxes make possible without knowing the geometry: the render's
/// centre is `placement + scale * (the body's own centre)`, and both other
/// terms are known.
pub fn refine(placement: Placement, comparison: &Value) -> Option<Placement> {
    /// Below one pixel a bounding box is measuring anti-aliasing, not layout.
    const POSITION_TOLERANCE: f64 = 1.0;
    /// Within this of `1.0` a size ratio is a pixel of edge, not a scale error.
    const SIZE_TOLERANCE: f64 = 0.03;

    let dx = number(comparison, "bounding_box.offset.dx");
    let dy = number(comparison, "bounding_box.offset.dy");
    let ratio = number(comparison, "bounding_box.width_ratio");

    if dx.hypot(dy) > POSITION_TOLERANCE {
        return Some(Placement {
            x: placement.x - dx,
            y: placement.y - dy,
            ..placement
        });
    }

    if (ratio - 1.0).abs() > SIZE_TOLERANCE {
        let scale = placement.scale / ratio;
        let (render_x, render_y) = centre(comparison, "render");
        let (reference_x, reference_y) = centre(comparison, "reference");
        // The body's own centre, recovered from where the render put it.
        let (body_x, body_y) = (
            (render_x - placement.x) / placement.scale,
            (render_y - placement.y) / placement.scale,
        );
        return Some(Placement {
            x: reference_x - scale * body_x,
            y: reference_y - scale * body_y,
            scale,
        });
    }

    None
}

/// Run the whole reference loop on a session, and return the overlap after
/// each comparison.
///
/// inspect -> analyse -> choose a strategy -> construct -> render -> compare,
/// then refine and compare again until [`refine`] has nothing left to do or
/// `limit` comparisons have been made. `body` is the geometry, `start` where
/// it is first placed — deliberately wrong, so there is something to improve.
pub fn run_loop(session: &mut Session, body: &str, start: Placement, limit: usize) -> Vec<f64> {
    session.call("get_reference_image", &json!({ "src": "reference.png" }));
    session.analysis();
    session.workflow();

    let mut placement = start;
    session.draw(&placement.markup(body));
    session.render(64);

    let mut history = Vec::new();
    while history.len() < limit {
        let comparison = session.compare();
        history.push(overlap(&comparison));
        match refine(placement, &comparison) {
            Some(next) => {
                placement = next;
                session.draw(&placement.markup(body));
            }
            None => break,
        }
    }
    history
}
