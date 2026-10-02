//! The reference corpus: seven small marks, drawn by integer arithmetic.
//!
//! Nothing here is a file. Each reference is *computed*, so the pixels a test
//! depends on are readable next to the test, there is no binary to review or
//! to drift between platforms, and "redistributable" is true by construction:
//! there is no third-party artwork to have a licence.
//!
//! Every shape is tested against integer sub-pixel samples, never against a
//! float, so the same code produces the same bytes on Linux, macOS and
//! Windows. That matters because a golden is only worth committing if every
//! CI runner reproduces it.

use std::ops::RangeInclusive;

use image::{ImageFormat, Rgba, RgbaImage};

/// Every fixture is this many pixels square.
///
/// Small enough that a whole corpus is a few kilobytes and a test runs in
/// milliseconds; large enough that a 3-pixel speck and a 6-pixel-wide hole
/// are still distinct features rather than rounding.
pub const SIZE: u32 = 64;

/// Sub-samples per pixel edge, per axis. `4` gives 16 samples and so a
/// coverage in `0..=16`, which is the same anti-aliasing granularity a
/// rasteriser's edge has — a reference with hard 1-bit edges would make every
/// fitted render look wrong for a reason that has nothing to do with the
/// geometry.
const SAMPLES: i64 = 4;

/// Sub-sample positions are expressed in `1/UNIT` of a pixel. Twice `SAMPLES`
/// so a sample sits at the centre of its sub-cell (`2 * i + 1`) with no
/// fraction to round.
const UNIT: i64 = 2 * SAMPLES;

/// A filled shape, in whole-pixel coordinates.
#[derive(Debug, Clone, Copy)]
enum Shape {
    Disc {
        cx: i64,
        cy: i64,
        r: i64,
    },
    Rect {
        x0: i64,
        y0: i64,
        x1: i64,
        y1: i64,
    },
    RoundedRect {
        x0: i64,
        y0: i64,
        x1: i64,
        y1: i64,
        r: i64,
    },
}

impl Shape {
    /// Whether a point, given in `1/UNIT` pixel units, lies inside.
    fn contains(self, x: i64, y: i64) -> bool {
        match self {
            // Squared distances on both sides: no square root, so no float.
            Self::Disc { cx, cy, r } => {
                let (dx, dy) = (x - cx * UNIT, y - cy * UNIT);
                dx * dx + dy * dy <= (r * UNIT) * (r * UNIT)
            }
            Self::Rect { x0, y0, x1, y1 } => {
                (x0 * UNIT..x1 * UNIT).contains(&x) && (y0 * UNIT..y1 * UNIT).contains(&y)
            }
            Self::RoundedRect { x0, y0, x1, y1, r } => {
                if !(x0 * UNIT..x1 * UNIT).contains(&x) || !(y0 * UNIT..y1 * UNIT).contains(&y) {
                    return false;
                }
                // The nearest point of the rectangle shrunk by the radius:
                // inside the shrunk rectangle the distance is zero, and in a
                // corner it is the distance to the corner's circle centre.
                let nearest_x = x.clamp((x0 + r) * UNIT, (x1 - r) * UNIT);
                let nearest_y = y.clamp((y0 + r) * UNIT, (y1 - r) * UNIT);
                let (dx, dy) = (x - nearest_x, y - nearest_y);
                dx * dx + dy * dy <= (r * UNIT) * (r * UNIT)
            }
        }
    }

    /// How many of a pixel's `SAMPLES * SAMPLES` sub-samples are inside.
    fn coverage(self, pixel_x: i64, pixel_y: i64) -> u32 {
        let mut inside = 0;
        for sy in 0..SAMPLES {
            for sx in 0..SAMPLES {
                // `2 * s + 1`: the centre of sub-cell `s`, in `1/UNIT` units.
                let x = pixel_x * UNIT + 2 * sx + 1;
                let y = pixel_y * UNIT + 2 * sy + 1;
                if self.contains(x, y) {
                    inside += 1;
                }
            }
        }
        inside
    }
}

/// A canvas the shapes are painted onto, in order.
struct Canvas {
    image: RgbaImage,
}

impl Canvas {
    fn opaque(background: [u8; 3]) -> Self {
        let [r, g, b] = background;
        Self {
            image: RgbaImage::from_pixel(SIZE, SIZE, Rgba([r, g, b, 255])),
        }
    }

    fn transparent() -> Self {
        Self {
            image: RgbaImage::from_pixel(SIZE, SIZE, Rgba([0, 0, 0, 0])),
        }
    }

    fn paint(&mut self, shape: Shape, colour: [u8; 3]) -> &mut Self {
        let full = (SAMPLES * SAMPLES) as u32;
        for y in 0..SIZE {
            for x in 0..SIZE {
                let coverage = shape.coverage(i64::from(x), i64::from(y));
                if coverage == 0 {
                    continue;
                }
                let pixel = self.image.get_pixel_mut(x, y);
                let [r, g, b] = colour;
                *pixel = match pixel.0[3] {
                    // Straight (not premultiplied) alpha, which is how a PNG
                    // stores it and what the analyser reads back.
                    0 => Rgba([r, g, b, (coverage * 255 / full) as u8]),
                    255 => {
                        let mix = |below: u8, above: u8| {
                            ((u32::from(below) * (full - coverage)
                                + u32::from(above) * coverage
                                + full / 2)
                                / full) as u8
                        };
                        Rgba([
                            mix(pixel.0[0], r),
                            mix(pixel.0[1], g),
                            mix(pixel.0[2], b),
                            255,
                        ])
                    }
                    // Blending over a half-transparent edge needs a real
                    // compositing model. No fixture paints there, and
                    // guessing would hide a mistake in one that started to.
                    alpha => panic!("a shape was painted over an edge pixel with alpha {alpha}"),
                };
            }
        }
        self
    }

    /// Fill the pixel rectangle `x0..x1` by `y0..y1` with whatever `colour`
    /// says for each pixel, hard-edged and unblended.
    ///
    /// For a fill that varies from pixel to pixel, which `paint` cannot
    /// express. `colour` is given whole-pixel coordinates and must use
    /// integers only, for the same reason the shapes do.
    fn fill_with(
        &mut self,
        (x0, y0, x1, y1): (i64, i64, i64, i64),
        mut colour: impl FnMut(i64, i64) -> [u8; 4],
    ) -> &mut Self {
        for y in y0..y1 {
            for x in x0..x1 {
                self.image.put_pixel(x as u32, y as u32, Rgba(colour(x, y)));
            }
        }
        self
    }

    fn png(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        self.image
            .write_to(&mut std::io::Cursor::new(&mut bytes), ImageFormat::Png)
            .expect("an in-memory PNG encodes");
        bytes
    }
}

/// What a fixture is *known* to contain. These are the facts the analysis and
/// strategy tests hold the tools to; they come from how the fixture is drawn,
/// not from running the tools and copying what they said.
#[derive(Debug, Clone)]
pub struct Expected {
    /// Foreground regions.
    pub regions: RangeInclusive<usize>,
    /// Enclosed background regions.
    pub holes: RangeInclusive<usize>,
    /// Colours covering at least 2% of the pixels, background included.
    pub dominant_colours: RangeInclusive<usize>,
    /// What `get_workflow` should recommend.
    pub strategy: &'static str,
}

/// One reference, and everything needed to reconstruct it.
#[derive(Debug, Clone)]
pub struct Fixture {
    pub name: &'static str,
    /// The reference image, as PNG bytes.
    pub png: Vec<u8>,
    /// The children of the `icon` symbol that draw this mark, in the
    /// reference's own pixel coordinates — what a good reconstruction looks
    /// like.
    pub construction: String,
    pub expected: Expected,
}

impl Fixture {
    /// A complete project with this fixture attached as its `source`
    /// reference, and `icon` drawing `body`.
    ///
    /// The reference is recorded in the document itself rather than pushed
    /// into the parsed metadata: `write_variant` re-parses the source, and a
    /// reference that only ever existed in memory would vanish on the first
    /// write.
    pub fn project(&self, body: &str) -> String {
        format!(
            r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:shaipe="https://shaipe.dev/ns/2026" viewBox="0 0 {SIZE} {SIZE}" width="{SIZE}" height="{SIZE}">
  <metadata>
    <shaipe:project version="1" primary="icon">
      <shaipe:prompt>Reproduce the {name} reference.</shaipe:prompt>
      <shaipe:variants>
        <shaipe:variant name="icon"/>
      </shaipe:variants>
      <shaipe:references>
        <shaipe:reference src="reference.png" kind="source"/>
      </shaipe:references>
    </shaipe:project>
  </metadata>
  {icon}
  <use href="#icon" width="{SIZE}" height="{SIZE}"/>
</svg>
"##,
            name = self.name,
            icon = symbol(body),
        )
    }
}

/// The `icon` variant's element.
pub fn symbol(body: &str) -> String {
    format!(r#"<symbol id="icon" viewBox="0 0 {SIZE} {SIZE}">{body}</symbol>"#)
}

fn hex([r, g, b]: [u8; 3]) -> String {
    format!("#{r:02x}{g:02x}{b:02x}")
}

const INK: [u8; 3] = [24, 24, 27];
const WHITE: [u8; 3] = [255, 255, 255];
const NAVY: [u8; 3] = [16, 24, 64];

/// A full-canvas background as SVG.
fn background(colour: [u8; 3]) -> String {
    format!(
        r#"<rect width="{SIZE}" height="{SIZE}" fill="{}"/>"#,
        hex(colour)
    )
}

/// The three discs of [`multicolour`]: centre, radius and true colour.
///
/// Four pixels apart, so no anti-aliased edge of one is painted over by
/// another.
const DISCS: [(i64, i64, i64, [u8; 3]); 3] = [
    (14, 32, 7, [220, 30, 30]),
    (32, 32, 7, [30, 160, 60]),
    (50, 32, 7, [30, 90, 220]),
];

/// The three discs as SVG, filled with `fills` rather than their true colours
/// — how a test builds "right shape, wrong colour" without a second fixture.
pub fn discs(fills: [&str; 3]) -> String {
    DISCS
        .iter()
        .zip(fills)
        .map(|((cx, cy, r, _), fill)| {
            format!(r#"<circle cx="{cx}" cy="{cy}" r="{r}" fill="{fill}"/>"#)
        })
        .collect()
}

/// The true colours of [`discs`].
pub fn true_fills() -> [String; 3] {
    DISCS.map(|(_, _, _, colour)| hex(colour))
}

/// One dark rounded rectangle on white: the simplest mark there is.
fn silhouette() -> Fixture {
    let mut canvas = Canvas::opaque(WHITE);
    canvas.paint(
        Shape::RoundedRect {
            x0: 12,
            y0: 14,
            x1: 52,
            y1: 50,
            r: 8,
        },
        INK,
    );
    Fixture {
        name: "silhouette",
        png: canvas.png(),
        construction: format!(
            r#"{}<rect x="12" y="14" width="40" height="36" rx="8" fill="{}"/>"#,
            background(WHITE),
            hex(INK)
        ),
        expected: Expected {
            regions: 1..=1,
            holes: 0..=0,
            dominant_colours: 2..=2,
            strategy: "trace",
        },
    }
}

/// A white disc on navy: the polarity a default trace gets backwards.
fn light_on_dark() -> Fixture {
    let mut canvas = Canvas::opaque(NAVY);
    canvas.paint(
        Shape::Disc {
            cx: 28,
            cy: 34,
            r: 18,
        },
        WHITE,
    );
    Fixture {
        name: "light_on_dark",
        png: canvas.png(),
        construction: format!(
            r##"{}<circle cx="28" cy="34" r="18" fill="#ffffff"/>"##,
            background(NAVY)
        ),
        expected: Expected {
            regions: 1..=1,
            holes: 0..=0,
            dominant_colours: 2..=2,
            strategy: "trace",
        },
    }
}

/// Three squares that never touch, of three different sizes so their ids —
/// assigned by area — are unambiguous.
fn disconnected() -> Fixture {
    let squares = [(6, 6, 14), (34, 10, 10), (44, 44, 6)];
    let mut canvas = Canvas::opaque(WHITE);
    let mut body = background(WHITE);
    for (x, y, side) in squares {
        canvas.paint(
            Shape::Rect {
                x0: x,
                y0: y,
                x1: x + side,
                y1: y + side,
            },
            INK,
        );
        body.push_str(&format!(
            r#"<rect x="{x}" y="{y}" width="{side}" height="{side}" fill="{}"/>"#,
            hex(INK)
        ));
    }
    Fixture {
        name: "disconnected",
        png: canvas.png(),
        construction: body,
        expected: Expected {
            regions: 3..=3,
            holes: 0..=0,
            dominant_colours: 2..=2,
            strategy: "trace",
        },
    }
}

/// A ring with a disc floating in its hole: one enclosed background region,
/// and a region nested inside another region's bounding box.
fn holes() -> Fixture {
    let mut canvas = Canvas::opaque(WHITE);
    canvas
        .paint(
            Shape::Disc {
                cx: 32,
                cy: 32,
                r: 24,
            },
            INK,
        )
        // Painted in the background colour: the canvas is opaque, so a hole is
        // a disc of background drawn over the ring.
        .paint(
            Shape::Disc {
                cx: 32,
                cy: 32,
                r: 14,
            },
            WHITE,
        )
        .paint(
            Shape::Disc {
                cx: 32,
                cy: 32,
                r: 6,
            },
            INK,
        );
    Fixture {
        name: "holes",
        png: canvas.png(),
        construction: format!(
            r#"{}<circle cx="32" cy="32" r="24" fill="{ink}"/><circle cx="32" cy="32" r="14" fill="{white}"/><circle cx="32" cy="32" r="6" fill="{ink}"/>"#,
            background(WHITE),
            ink = hex(INK),
            white = hex(WHITE),
        ),
        expected: Expected {
            regions: 2..=2,
            holes: 1..=1,
            dominant_colours: 2..=2,
            strategy: "trace",
        },
    }
}

/// Three flat-colour discs on a transparent canvas.
///
/// Transparent, so that a comparison against a render (which is always
/// transparent) reaches a near-perfect score on *every* metric rather than
/// only on the foreground overlap.
fn multicolour() -> Fixture {
    let mut canvas = Canvas::transparent();
    for (cx, cy, r, colour) in DISCS {
        canvas.paint(Shape::Disc { cx, cy, r }, colour);
    }
    Fixture {
        name: "multicolour",
        png: canvas.png(),
        construction: discs(true_fills().each_ref().map(String::as_str)),
        expected: Expected {
            regions: 3..=3,
            holes: 0..=0,
            dominant_colours: 3..=3,
            strategy: "trace",
        },
    }
}

/// A plus sign, mirrored exactly about both axes.
///
/// Every bar spans `[a, 63 - a]` in pixel indices, so the mirror of the mask
/// is the mask, and the score is exactly `1.0` rather than close to it.
fn symmetric() -> Fixture {
    let mut canvas = Canvas::opaque(WHITE);
    let bars = [(8, 26, 56, 38), (26, 8, 38, 56)];
    let mut body = background(WHITE);
    for (x0, y0, x1, y1) in bars {
        canvas.paint(Shape::Rect { x0, y0, x1, y1 }, INK);
        body.push_str(&format!(
            r#"<rect x="{x0}" y="{y0}" width="{}" height="{}" fill="{}"/>"#,
            x1 - x0,
            y1 - y0,
            hex(INK)
        ));
    }
    Fixture {
        name: "symmetric",
        png: canvas.png(),
        construction: body,
        expected: Expected {
            // The two bars overlap, so they are one region.
            regions: 1..=1,
            holes: 0..=0,
            dominant_colours: 2..=2,
            strategy: "trace",
        },
    }
}

/// An "L": the control that keeps the symmetric fixture's `1.0` honest.
///
/// A test that only ever sees a perfect score cannot tell a symmetry
/// measurement from a constant.
pub fn asymmetric_control() -> Vec<u8> {
    let mut canvas = Canvas::opaque(WHITE);
    canvas
        .paint(
            Shape::Rect {
                x0: 10,
                y0: 10,
                x1: 22,
                y1: 54,
            },
            INK,
        )
        .paint(
            Shape::Rect {
                x0: 10,
                y0: 42,
                x1: 54,
                y1: 54,
            },
            INK,
        );
    canvas.png()
}

/// The palette the speckle is drawn from. All dark enough to fall on the
/// foreground side of a default binary trace — noise a trace *keeps* is the
/// noise worth demonstrating.
const SPECKLE: [[u8; 3]; 6] = [
    [24, 24, 27],
    [180, 30, 30],
    [30, 110, 40],
    [30, 60, 190],
    [110, 30, 140],
    [150, 90, 20],
];

/// The clean disc that [`noisy`] is a corrupted copy of.
const NOISY_DISC: Shape = Shape::Disc {
    cx: 32,
    cy: 32,
    r: 12,
};

/// What the artist *meant* by [`noisy`]: the disc, and nothing else.
pub fn noisy_truth() -> Vec<u8> {
    let mut canvas = Canvas::opaque(WHITE);
    canvas.paint(NOISY_DISC, INK);
    canvas.png()
}

/// A disc surrounded by coloured specks, the way a re-compressed or scanned
/// logo arrives.
///
/// The specks are 5x5 blocks on a 6-pixel grid, so none touch and none are
/// small enough for the tracer to discard: `vtracer` drops any patch under
/// `filter_speckle * filter_speckle` pixels, and its default of 4 removes
/// anything below 16. A first version of this fixture used 3x3 blocks and was
/// cleaned by the trace itself — which is a fact about the tracer worth
/// knowing, and the reason these are the size they are. Positions come from a
/// fixed linear congruential generator, not a random source: the same corpus
/// on every machine.
fn noisy() -> Fixture {
    let mut canvas = Canvas::opaque(WHITE);
    canvas.paint(NOISY_DISC, INK);

    let mut state: u32 = 0x5eed_1e55;
    // The high byte only: the low bits of an LCG have short periods.
    let mut next = || {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        state >> 24
    };
    // Cells 1..=9 put every block between 6 and 59, clear of the border.
    for cell_y in 1..10_i64 {
        for cell_x in 1..10_i64 {
            // Both draws happen for every cell, kept or not, so a change to
            // the exclusion zone below does not reshuffle every later speck.
            let keep = next() % 10 < 8;
            let colour = SPECKLE[next() as usize % SPECKLE.len()];
            let (x, y) = (cell_x * 6, cell_y * 6);
            // Specks that would fuse with the disc are dropped: fused, they
            // would change the disc's own outline rather than add noise.
            let (dx, dy) = (x + 2 - 32, y + 2 - 32);
            if !keep || dx * dx + dy * dy < 19 * 19 {
                continue;
            }
            canvas.paint(
                Shape::Rect {
                    x0: x,
                    y0: y,
                    x1: x + 5,
                    y1: y + 5,
                },
                colour,
            );
        }
    }
    Fixture {
        name: "noisy",
        png: canvas.png(),
        // The clean mark, as a person would draw it — not the noise.
        construction: format!(
            r#"{}<circle cx="32" cy="32" r="12" fill="{}"/>"#,
            background(WHITE),
            hex(INK)
        ),
        expected: Expected {
            regions: 7..=200,
            holes: 0..=0,
            dominant_colours: 5..=8,
            strategy: "construct",
        },
    }
}

/// Nine separate squares in two colours: many regions, few colours.
///
/// A control rather than a member of the corpus. Every corpus fixture that has
/// many regions also has many colours, so without this the region threshold in
/// the strategy recommendation is never the deciding factor and nothing would
/// notice it being loosened.
pub fn many_parts() -> Fixture {
    let mut canvas = Canvas::opaque(WHITE);
    let mut body = background(WHITE);
    for row in 0..3 {
        for column in 0..3 {
            let (x, y) = (8 + column * 16, 8 + row * 16);
            canvas.paint(
                Shape::Rect {
                    x0: x,
                    y0: y,
                    x1: x + 8,
                    y1: y + 8,
                },
                INK,
            );
            body.push_str(&format!(
                r#"<rect x="{x}" y="{y}" width="8" height="8" fill="{}"/>"#,
                hex(INK)
            ));
        }
    }
    Fixture {
        name: "many_parts",
        png: canvas.png(),
        construction: body,
        expected: Expected {
            regions: 9..=9,
            holes: 0..=0,
            dominant_colours: 2..=2,
            strategy: "construct",
        },
    }
}

/// Integer interpolation of two colours: `num / den` of the way from `from`
/// to `to`, per channel, rounded to nearest.
fn mix(from: [u8; 4], to: [u8; 4], num: i64, den: i64) -> [u8; 4] {
    let channel = |i: usize| {
        let (a, b) = (i64::from(from[i]), i64::from(to[i]));
        ((a * (den - num) + b * num + den / 2) / den) as u8
    };
    [channel(0), channel(1), channel(2), channel(3)]
}

const RED: [u8; 4] = [200, 30, 30, 255];
const BLUE: [u8; 4] = [30, 40, 210, 255];

/// A fixture that exists to show what the appearance analysis reports for one
/// region, on a transparent canvas.
///
/// Controls beside the corpus, not members of it: `all()` is pinned by name
/// and by a size budget, and these say nothing about tracing.
fn appearance_fixture(name: &'static str, canvas: &Canvas, construction: String) -> Fixture {
    Fixture {
        name,
        png: canvas.png(),
        construction,
        expected: Expected {
            regions: 1..=1,
            holes: 0..=0,
            dominant_colours: 1..=8,
            strategy: "construct",
        },
    }
}

/// A square that ramps from red on the left to blue on the right.
pub fn linear_gradient() -> Fixture {
    let mut canvas = Canvas::transparent();
    canvas.fill_with((12, 12, 52, 52), |x, _| mix(RED, BLUE, x - 12, 39));
    appearance_fixture(
        "linear_gradient",
        &canvas,
        format!(
            r#"<defs><linearGradient id="ramp" gradientUnits="userSpaceOnUse" x1="12" y1="0" x2="52" y2="0"><stop offset="0" stop-color="{}"/><stop offset="1" stop-color="{}"/></linearGradient></defs><rect x="12" y="12" width="40" height="40" fill="url(#ramp)"/>"#,
            hex([RED[0], RED[1], RED[2]]),
            hex([BLUE[0], BLUE[1], BLUE[2]]),
        ),
    )
}

/// A disc centred on (32, 32), red in the middle and blue at its edge.
pub fn radial_gradient() -> Fixture {
    let mut canvas = Canvas::transparent();
    let radius = 22;
    canvas.fill_with((10, 10, 54, 54), |x, y| {
        let squared = (x - 32) * (x - 32) + (y - 32) * (y - 32);
        if squared > radius * radius {
            return [0, 0, 0, 0];
        }
        mix(RED, BLUE, squared.isqrt() * 4, radius * 4)
    });
    appearance_fixture(
        "radial_gradient",
        &canvas,
        format!(
            r#"<defs><radialGradient id="glow" gradientUnits="userSpaceOnUse" cx="32" cy="32" r="22"><stop offset="0" stop-color="{}"/><stop offset="1" stop-color="{}"/></radialGradient></defs><circle cx="32" cy="32" r="22" fill="url(#glow)"/>"#,
            hex([RED[0], RED[1], RED[2]]),
            hex([BLUE[0], BLUE[1], BLUE[2]]),
        ),
    )
}

/// A red square at half opacity: one colour, and an alpha that is not 255.
pub fn translucent_fill() -> Fixture {
    let mut canvas = Canvas::transparent();
    canvas.fill_with((12, 12, 52, 52), |_, _| [RED[0], RED[1], RED[2], 128]);
    appearance_fixture(
        "translucent_fill",
        &canvas,
        format!(
            r#"<rect x="12" y="12" width="40" height="40" fill="{}" fill-opacity="0.5"/>"#,
            hex([RED[0], RED[1], RED[2]])
        ),
    )
}

/// Two flat colours side by side: a fit as good as a gradient's, and not one.
pub fn hard_step() -> Fixture {
    let mut canvas = Canvas::transparent();
    canvas.fill_with((12, 12, 52, 52), |x, _| if x < 32 { RED } else { BLUE });
    appearance_fixture("hard_step", &canvas, String::new())
}

/// Random-looking variation over a square: varied colour with no direction to
/// it. From the same LCG as `noisy`, at a different seed.
pub fn speckled_fill() -> Fixture {
    let mut canvas = Canvas::transparent();
    let mut state: u32 = 0x0bad_cafe;
    canvas.fill_with((12, 12, 52, 52), |_, _| {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let (a, b) = (((state >> 24) % 96) as u8, ((state >> 16) % 96) as u8);
        [80 + a, 60 + b, 120 + a / 2, 255]
    });
    appearance_fixture("speckled_fill", &canvas, String::new())
}

/// A gradient panel with a transparent window cut through it, and a flat bar
/// beneath: the smallest reference where a trace has both to keep apart.
///
/// The panel is one region whose outline has a hole and whose fill is a ramp;
/// the bar is a second region of one flat colour. A control beside the corpus,
/// like the other appearance fixtures. Its `strategy` is not asserted, because
/// the recommendation reads no appearance (ADR 026).
pub fn gradient_badge() -> Fixture {
    const BAR: [u8; 4] = [20, 20, 40, 255];
    let mut canvas = Canvas::transparent();
    canvas.fill_with((8, 8, 56, 40), |x, y| {
        if (26..38).contains(&x) && (18..30).contains(&y) {
            [0, 0, 0, 0]
        } else {
            mix(RED, BLUE, x - 8, 47)
        }
    });
    canvas.fill_with((8, 46, 56, 56), |_, _| BAR);
    Fixture {
        name: "gradient_badge",
        png: canvas.png(),
        construction: format!(
            r#"<defs><linearGradient id="ramp" gradientUnits="userSpaceOnUse" x1="8" y1="0" x2="56" y2="0"><stop offset="0" stop-color="{}"/><stop offset="1" stop-color="{}"/></linearGradient></defs><path fill-rule="evenodd" d="M8 8H56V40H8Z M26 18H38V30H26Z" fill="url(#ramp)"/><rect x="8" y="46" width="48" height="10" fill="{}"/>"#,
            hex([RED[0], RED[1], RED[2]]),
            hex([BLUE[0], BLUE[1], BLUE[2]]),
            hex([BAR[0], BAR[1], BAR[2]]),
        ),
        expected: Expected {
            regions: 2..=2,
            holes: 1..=1,
            dominant_colours: 1..=8,
            strategy: "hybrid",
        },
    }
}

/// A radial gradient whose rim fades out over two pixels instead of ending
/// hard: the edge has partial alpha that is not the interior's translucency.
pub fn antialiased_gradient() -> Fixture {
    let mut canvas = Canvas::transparent();
    let radius = 22;
    canvas.fill_with((8, 8, 56, 56), |x, y| {
        let squared = (x - 32) * (x - 32) + (y - 32) * (y - 32);
        let d = squared.isqrt();
        let [r, g, b, _] = mix(RED, BLUE, d.min(radius) * 4, radius * 4);
        match d {
            0..=20 => [r, g, b, 255],
            21 => [r, g, b, 170],
            22 => [r, g, b, 85],
            _ => [0, 0, 0, 0],
        }
    });
    appearance_fixture(
        "antialiased_gradient",
        &canvas,
        format!(
            r#"<defs><radialGradient id="glow" gradientUnits="userSpaceOnUse" cx="32" cy="32" r="22"><stop offset="0" stop-color="{}"/><stop offset="1" stop-color="{}"/></radialGradient></defs><circle cx="32" cy="32" r="22" fill="url(#glow)"/>"#,
            hex([RED[0], RED[1], RED[2]]),
            hex([BLUE[0], BLUE[1], BLUE[2]]),
        ),
    )
}

/// A four-pixel ring, as a stroked circle draws it.
pub fn stroked_ring() -> Fixture {
    const INK: [u8; 4] = [20, 20, 20, 255];
    let mut canvas = Canvas::transparent();
    canvas.fill_with((8, 8, 56, 56), |x, y| {
        let squared = (x - 32) * (x - 32) + (y - 32) * (y - 32);
        if (18 * 18..22 * 22).contains(&squared) {
            INK
        } else {
            [0, 0, 0, 0]
        }
    });
    appearance_fixture(
        "stroked_ring",
        &canvas,
        r##"<circle cx="32" cy="32" r="20" fill="none" stroke="#141414" stroke-width="4"/>"##
            .to_owned(),
    )
}

/// A three-pixel horizontal bar, as a stroked line draws it.
pub fn stroked_bar() -> Fixture {
    const INK: [u8; 4] = [20, 20, 20, 255];
    let mut canvas = Canvas::transparent();
    canvas.fill_with((12, 31, 52, 34), |_, _| INK);
    appearance_fixture(
        "stroked_bar",
        &canvas,
        r##"<line x1="12" y1="32.5" x2="52" y2="32.5" stroke="#141414" stroke-width="3"/>"##
            .to_owned(),
    )
}

/// An opaque blue square with a half-transparent red square across its corner.
pub fn translucent_overlay() -> Fixture {
    let mut canvas = Canvas::transparent();
    canvas.fill_with((10, 10, 40, 40), |_, _| BLUE);
    canvas.fill_with((28, 28, 54, 54), |x, y| {
        if x < 40 && y < 40 {
            mix(BLUE, RED, 128, 255)
        } else {
            [RED[0], RED[1], RED[2], 128]
        }
    });
    Fixture {
        name: "translucent_overlay",
        png: canvas.png(),
        construction: format!(
            r#"<rect x="10" y="10" width="30" height="30" fill="{}"/><rect x="28" y="28" width="26" height="26" fill="{}" fill-opacity="0.5"/>"#,
            hex([BLUE[0], BLUE[1], BLUE[2]]),
            hex([RED[0], RED[1], RED[2]]),
        ),
        expected: Expected {
            regions: 1..=4,
            holes: 0..=0,
            dominant_colours: 1..=8,
            strategy: "construct",
        },
    }
}

/// A ramp of three colour levels over forty pixels: a gradient to the eye's
/// arithmetic and not to anyone's.
pub fn near_flat_ramp() -> Fixture {
    let mut canvas = Canvas::transparent();
    canvas.fill_with((12, 12, 52, 52), |x, _| {
        let level = ((x - 12) * 3 / 39) as u8;
        [100 + level, 100 + level, 100 + level, 255]
    });
    appearance_fixture("near_flat_ramp", &canvas, String::new())
}

/// Eight-pixel blocks, each a few levels off its neighbours: what a lossy
/// codec leaves on a flat fill.
pub fn blocky_artefacts() -> Fixture {
    let mut canvas = Canvas::transparent();
    canvas.fill_with((12, 12, 52, 52), |x, y| {
        let offset = (((x - 12) / 8 * 7 + (y - 12) / 8 * 13) % 5) as u8;
        [120 + offset * 2, 90 + offset, 150 - offset * 2, 255]
    });
    appearance_fixture("blocky_artefacts", &canvas, String::new())
}

/// One letter-shaped block: left edge, right edge, top and bottom, in pixels.
type Block = (i64, i64, i64, i64);

const ACCENT: [u8; 3] = [200, 30, 30];

/// Lettering, as geometry: blocks of varied widths and heights standing on a
/// shared baseline, in the colours given.
///
/// There is no font here. A glyph rasteriser would make the corpus depend on
/// anti-aliasing a platform chose, and what the typography analysis measures is
/// layout — where letters stand, how tall they are, how far apart — which
/// blocks carry exactly. `blocks` pairs each block with its colour.
fn lettering(name: &'static str, blocks: &[(Block, [u8; 3])]) -> Fixture {
    let mut canvas = Canvas::opaque(WHITE);
    let mut construction = String::new();
    for &((x0, x1, y0, y1), colour) in blocks {
        canvas.paint(Shape::Rect { x0, y0, x1, y1 }, colour);
        construction.push_str(&format!(
            r#"<rect x="{x0}" y="{y0}" width="{}" height="{}" fill="{}"/>"#,
            x1 - x0,
            y1 - y0,
            hex(colour),
        ));
    }
    Fixture {
        name,
        png: canvas.png(),
        construction,
        expected: Expected {
            regions: 1..=16,
            holes: 0..=0,
            dominant_colours: 1..=3,
            strategy: "construct",
        },
    }
}

/// `blocks` all in one colour.
fn in_ink(blocks: &[Block]) -> Vec<(Block, [u8; 3])> {
    blocks.iter().map(|&block| (block, INK)).collect()
}

/// One line of lettering with everything a line of mixed-case text has: a
/// capital, short letters, a narrow stem with a dot over it, and a descender.
///
/// The baseline is y=36. The capital is 12 tall and the short letters 8; the
/// fifth letter hangs to y=41 as a g does; the dot is a 2x2 block over the
/// stem.
pub fn text_line() -> Fixture {
    lettering(
        "text_line",
        &in_ink(&[
            (6, 13, 24, 36),
            (16, 22, 28, 36),
            (25, 33, 28, 36),
            (36, 39, 28, 36),
            (36, 38, 24, 26),
            (42, 48, 28, 41),
            (51, 58, 28, 36),
        ]),
    )
}

/// Two words, the first in ink and the second in red: a wide gap between
/// them, and one appearance for each.
pub fn coloured_words() -> Fixture {
    let first = [(4, 10, 24, 36), (13, 18, 24, 36), (21, 28, 24, 36)];
    let second = [(38, 44, 24, 36), (47, 54, 24, 36), (57, 62, 24, 36)];
    let blocks: Vec<_> = first
        .iter()
        .map(|&b| (b, INK))
        .chain(second.iter().map(|&b| (b, ACCENT)))
        .collect();
    lettering("coloured_words", &blocks)
}

/// Two lines set as one block, centred on x=32, the second narrower and
/// shorter than the first.
pub fn centred_block() -> Fixture {
    lettering(
        "centred_block",
        &in_ink(&[
            (10, 16, 14, 26),
            (19, 27, 14, 26),
            (30, 35, 14, 26),
            (38, 45, 14, 26),
            (48, 54, 14, 26),
            (16, 21, 34, 44),
            (24, 30, 34, 44),
            (33, 40, 34, 44),
            (43, 48, 34, 44),
        ]),
    )
}

/// Four letters one above another, centred on x=32: vertical lettering.
pub fn stacked_letters() -> Fixture {
    lettering(
        "stacked_letters",
        &in_ink(&[
            (26, 38, 4, 14),
            (28, 36, 18, 28),
            (25, 39, 32, 42),
            (27, 37, 46, 56),
        ]),
    )
}

/// Control: shapes of varied size scattered with no common baseline. Nothing
/// here is lettering, and the analysis must not say it is.
pub fn scattered_shapes() -> Fixture {
    lettering(
        "scattered_shapes",
        &in_ink(&[
            (4, 12, 6, 18),
            (30, 38, 3, 9),
            (50, 58, 14, 30),
            (10, 20, 30, 36),
            (36, 44, 40, 58),
            (52, 60, 44, 52),
        ]),
    )
}

/// Control: seven identical squares evenly spaced, one missing — a dotted
/// pattern, which has every property of a row of letters except that the
/// letters differ.
pub fn row_of_dots() -> Fixture {
    lettering(
        "row_of_dots",
        &in_ink(&[
            (4, 9, 30, 35),
            (13, 18, 30, 35),
            (22, 27, 30, 35),
            (40, 45, 30, 35),
            (49, 54, 30, 35),
            (58, 63, 30, 35),
        ]),
    )
}

/// Control: one large block. A single shape is not a line.
pub fn single_large_letter() -> Fixture {
    lettering("single_large_letter", &in_ink(&[(14, 50, 10, 54)]))
}

/// What one part of a composition is drawn as: its shape and colour on the
/// canvas, and the SVG that draws the same thing.
struct Part {
    shape: Shape,
    colour: [u8; 3],
    svg: String,
}

fn disc(cx: i64, cy: i64, r: i64, colour: [u8; 3]) -> Part {
    Part {
        shape: Shape::Disc { cx, cy, r },
        colour,
        svg: format!(
            r#"<circle cx="{cx}" cy="{cy}" r="{r}" fill="{}"/>"#,
            hex(colour)
        ),
    }
}

fn square(x0: i64, y0: i64, x1: i64, y1: i64, colour: [u8; 3]) -> Part {
    Part {
        shape: Shape::Rect { x0, y0, x1, y1 },
        colour,
        svg: format!(
            r#"<rect x="{x0}" y="{y0}" width="{}" height="{}" fill="{}"/>"#,
            x1 - x0,
            y1 - y0,
            hex(colour)
        ),
    }
}

/// A ring: a disc with a disc of the background cut out of it. Drawn in SVG as
/// the stroked circle it is.
fn ring(cx: i64, cy: i64, outer: i64, inner: i64, colour: [u8; 3]) -> Part {
    Part {
        shape: Shape::Disc { cx, cy, r: outer },
        colour,
        svg: format!(
            r#"<circle cx="{cx}" cy="{cy}" r="{}" fill="none" stroke="{}" stroke-width="{}"/>"#,
            (outer + inner) / 2,
            hex(colour),
            outer - inner
        ),
    }
}

/// A frame: a square with a square of the background cut out of it.
fn frame(x0: i64, y0: i64, x1: i64, y1: i64, thickness: i64, colour: [u8; 3]) -> Part {
    Part {
        shape: Shape::Rect { x0, y0, x1, y1 },
        colour,
        svg: format!(
            r#"<rect x="{}" y="{}" width="{}" height="{}" fill="none" stroke="{}" stroke-width="{thickness}"/>"#,
            x0 + thickness / 2,
            y0 + thickness / 2,
            x1 - x0 - thickness,
            y1 - y0 - thickness,
            hex(colour)
        ),
    }
}

/// A composition of parts on a white canvas, drawn in order. A part drawn with
/// [`ring`] or [`frame`] has its hole cut by painting the background over it,
/// so `holes` names the hollow parts as `(index, inner shape)`.
fn composition(
    name: &'static str,
    parts: &[Part],
    holes: &[(usize, Shape)],
    lettering: &[Block],
) -> Fixture {
    let mut canvas = Canvas::opaque(WHITE);
    let mut construction = String::new();
    for (index, part) in parts.iter().enumerate() {
        canvas.paint(part.shape, part.colour);
        // A hole is cut straight after its part, before the next part is
        // painted, so a part inside it is not wiped out.
        for (_, hole) in holes.iter().filter(|(owner, _)| *owner == index) {
            canvas.paint(*hole, WHITE);
        }
        construction.push_str(&part.svg);
    }
    for &(x0, x1, y0, y1) in lettering {
        canvas.paint(Shape::Rect { x0, y0, x1, y1 }, INK);
        construction.push_str(&format!(
            r#"<rect x="{x0}" y="{y0}" width="{}" height="{}" fill="{}"/>"#,
            x1 - x0,
            y1 - y0,
            hex(INK),
        ));
    }
    Fixture {
        name,
        png: canvas.png(),
        construction,
        expected: Expected {
            regions: 2..=16,
            holes: 0..=2,
            dominant_colours: 1..=4,
            strategy: "construct",
        },
    }
}

/// A ringed mark with a dot in its hole, over a line of lettering centred
/// beneath it: a symbol and a wordmark on one vertical centre line.
///
/// The ring is centred on x=32, and the lettering spans 10..54, so both sit on
/// x=32.
pub fn icon_with_wordmark() -> Fixture {
    composition(
        "icon_with_wordmark",
        &[ring(32, 20, 14, 8, INK), disc(32, 20, 4, ACCENT)],
        &[(
            0,
            Shape::Disc {
                cx: 32,
                cy: 20,
                r: 8,
            },
        )],
        &[
            (10, 16, 44, 56),
            (19, 27, 44, 56),
            (30, 35, 44, 56),
            (38, 45, 44, 56),
            (48, 54, 44, 56),
        ],
    )
}

/// A disc with four equal small dots round it, one on each side: the dots are
/// ornament, and repeat.
pub fn satellite_dots() -> Fixture {
    composition(
        "satellite_dots",
        &[
            disc(32, 32, 12, INK),
            disc(32, 6, 2, ACCENT),
            disc(58, 32, 2, ACCENT),
            disc(32, 58, 2, ACCENT),
            disc(6, 32, 2, ACCENT),
        ],
        &[],
        &[],
    )
}

/// Four equal squares at an even pitch: a row with one gap, shared top and
/// bottom edges, and one size.
pub fn even_row() -> Fixture {
    row("even_row", &[4, 20, 36, 52])
}

/// Control: the same four squares, spaced unevenly. Same size, same row, and a
/// gap that is not one value.
pub fn uneven_row() -> Fixture {
    row("uneven_row", &[4, 16, 36, 52])
}

fn row(name: &'static str, lefts: &[i64]) -> Fixture {
    let parts: Vec<Part> = lefts
        .iter()
        .map(|&x| square(x, 28, x + 8, 36, INK))
        .collect();
    composition(name, &parts, &[], &[])
}

/// A ring, a long bar and a solid disc, far apart: a closed contour, an open
/// one and a filled one.
pub fn open_and_closed() -> Fixture {
    composition(
        "open_and_closed",
        &[
            ring(18, 18, 10, 6, INK),
            square(8, 44, 56, 48, INK),
            disc(46, 18, 10, INK),
        ],
        &[(
            0,
            Shape::Disc {
                cx: 18,
                cy: 18,
                r: 6,
            },
        )],
        &[],
    )
}

/// A square frame with a disc in the middle of it: the frame contains the mark.
pub fn framed_mark() -> Fixture {
    composition(
        "framed_mark",
        &[frame(8, 8, 56, 56, 4, INK), disc(32, 32, 8, ACCENT)],
        &[(
            0,
            Shape::Rect {
                x0: 12,
                y0: 12,
                x1: 52,
                y1: 52,
            },
        )],
        &[],
    )
}

/// The whole corpus, in a fixed order.
pub fn all() -> Vec<Fixture> {
    vec![
        silhouette(),
        light_on_dark(),
        disconnected(),
        holes(),
        multicolour(),
        symmetric(),
        noisy(),
    ]
}

/// One fixture by name.
pub fn named(name: &str) -> Fixture {
    all()
        .into_iter()
        .find(|fixture| fixture.name == name)
        .unwrap_or_else(|| panic!("there is no `{name}` fixture"))
}
