//! Which regions are probably lettering, and how the lettering is set out.
//! See ADR 028.
//!
//! This module reads no text and names no font: Shaipe has no OCR and never
//! will (ADR 004). It measures what geometry can honestly say about a row of
//! regions: that they share a baseline, are about as tall as each other,
//! and are spaced like letters and words. That is *evidence* that an agent
//! looking at the image is reading text, together with the numbers it needs to
//! set the text it reads: where the baseline is, how tall the letters are, how
//! far apart. What the letters say, and which font they are set in, stays the
//! agent's to read and to be unsure about.
//!
//! The honest default is to report nothing. A line is only reported when
//! several independent signals agree, and a row of shapes all of one size
//! (dots, bars, a pattern) is deliberately not one: it is repetition, not
//! lettering. Everything is integer geometry and sorts with total
//! orders, so the same regions produce the same lines on any machine.
//!
//! Text is found along one axis at a time. Everything is computed in a frame
//! where `a` runs along the line and `c` across it, which is `x` and `y` for
//! horizontal text and the other way round for vertical text, so there is one
//! detector, not two.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use super::{BoundingBox, Region, appearance};

/// More regions than this and nothing is considered. A page of noise, a
/// photograph or a texture has hundreds of tiny regions, and none of it is
/// lettering a logo needs set; it also bounds the cost of chaining them.
const MAX_CANDIDATES: usize = 512;
/// A region must be at least this many pixels across the line to be a letter
/// rather than a mark (a dot, a comma, a speck).
const MIN_GLYPH_EXTENT: i64 = 4;
/// A line needs at least this many letters. Two shapes side by side are a
/// pair, not a word.
const MIN_GLYPHS: usize = 3;
/// How much taller or shorter than the line's median a letter may be. Capitals
/// against lowercase, ascenders against x-height and descenders all fit.
const HEIGHT_RATIO: f64 = 2.5;
/// Share of a letter's extent across the line that must lie in the line's
/// band for it to belong there.
const MIN_BAND_OVERLAP: f64 = 0.5;
/// The widest gap between letters, in multiples of the line's median extent
/// across it. Wide enough for a word space, narrow enough not to join two
/// unrelated marks.
const MAX_GAP_EXTENTS: f64 = 1.5;
/// How much two neighbours may overlap along the line, as a share of the
/// narrower. Kerning and italics overlap a little; a mark sitting over a
/// letter overlaps almost entirely, and is not a letter.
const MAX_ALONG_OVERLAP: f64 = 0.5;
/// A letter's extent along the line over its extent across it, as a median
/// over the line. Lettering is neither slivers nor slabs; bars of a chart and
/// stripes of a flag are.
const MIN_ASPECT: f64 = 0.25;
/// See [`MIN_ASPECT`].
const MAX_ASPECT: f64 = 1.6;
/// Tolerance for "on the baseline", at least this many pixels: an
/// anti-aliased edge is up to a pixel soft and a bounding box rounds out.
const MIN_TOLERANCE: f64 = 1.5;
/// The tolerance as a share of the line's median extent across it, when that
/// is larger than [`MIN_TOLERANCE`].
const TOLERANCE_FRACTION: f64 = 0.12;
/// The share of letters that must sit on the baseline. The rest are
/// descenders, punctuation and ornaments, which legitimately do not.
const MIN_BASELINE_SHARE: f64 = 0.6;
/// A mark is at most this share of the line's letter height across and along.
const MAX_MARK_FRACTION: f64 = 0.6;
/// How far from a letter a mark may sit and still belong to the line, in
/// multiples of the line's letter height.
const MARK_REACH: f64 = 0.6;
/// How many letters the baseline slope is estimated from, evenly strided. The
/// estimate is quadratic in this.
const SLOPE_SAMPLES: usize = 64;
/// Letters at least this share of the tallest are the tall group.
const TALL_GROUP: f64 = 0.85;
/// Letters under this share of the tall height are the short group.
const SHORT_GROUP: f64 = 0.8;
/// The short group is only reported when it is at least this share of the
/// letters on the baseline: two lowercase letters in a capitalised word are
/// not evidence of an x-height.
const MIN_SHORT_SHARE: f64 = 0.25;
/// A gap is a word space when it exceeds this many median letter gaps...
const WORD_GAP_RATIO: f64 = 2.5;
/// ...and this share of the letter height, so a line set with touching
/// letters (a median gap of zero) still finds its spaces.
const WORD_GAP_HEIGHT: f64 = 0.4;
/// Letters at which the count stops adding confidence.
const COUNT_SATURATION: f64 = 6.0;
/// Spread of letter widths, as a share of the median letter height, at which
/// variety stops adding confidence. Real lettering has narrow and wide letters;
/// a pattern does not.
const VARIETY_SATURATION: f64 = 0.5;
/// Below this confidence a candidate is not reported at all.
const MIN_CONFIDENCE: f64 = 0.6;
/// How many lines are reported. `line_count` keeps the true total.
const MAX_LINES: usize = 16;
/// How far apart two lines of a block may be, in multiples of the taller
/// line's letter height.
const BLOCK_REACH: f64 = 2.5;
/// Share of the narrower line's width that two lines must share along the
/// line to be one block.
const BLOCK_MIN_OVERLAP: f64 = 0.3;
/// Alignment tolerance, at least this many pixels...
const MIN_ALIGNMENT_TOLERANCE: f64 = 2.0;
/// ...or this share of the block's width, when larger.
const ALIGNMENT_TOLERANCE_FRACTION: f64 = 0.03;
/// Mean colour distance (RGB, 0-441) under which letters are one colour.
const GROUP_COLOUR_TOLERANCE: f64 = 30.0;
/// How many distinct colours a mixed group lists.
const MAX_GROUP_COLOURS: usize = 6;

/// The lettering found in an image.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Typography {
    /// How many text lines were found, before `lines` is capped.
    pub line_count: usize,
    /// Candidate lines of lettering, top to bottom then left to right,
    /// capped. Each is a *candidate*: geometry that behaves like text. Whether
    /// it is text, and what it says, is for the agent to read from the image.
    pub lines: Vec<TextLine>,
    /// Consecutive horizontal lines that are set as one block, with the
    /// alignment and line pitch they share.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub blocks: Vec<TextBlock>,
}

impl Typography {
    /// Whether anything was found. An analysis without lettering leaves the
    /// field out entirely, so a reference without text reads as it always did.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.line_count == 0
    }
}

/// Which way a line of lettering runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Orientation {
    /// Left to right: letters side by side, sharing a baseline below them.
    Horizontal,
    /// Top to bottom: letters stacked, upright or turned a quarter.
    Vertical,
}

/// The edge of the letters that `baseline` is measured on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BaselineEdge {
    /// The bottom of the letters: the baseline of horizontal text.
    Bottom,
    /// The left edge of vertical text.
    Left,
    /// The right edge of vertical text.
    Right,
    /// The middle of vertical text, as with upright letters stacked and
    /// centred.
    Centre,
}

/// The mean and spread of the gaps between letters, in pixels.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct Gap {
    /// Mean gap between neighbouring letters of one word.
    pub mean: f64,
    /// Standard deviation of those gaps: small for even tracking.
    pub deviation: f64,
}

/// One candidate line of lettering.
#[derive(Debug, Clone, Serialize)]
pub struct TextLine {
    /// Numbered top to bottom, then left to right.
    pub id: usize,
    /// Every region in the line, letters and marks, ascending. They are ids
    /// in the region map, so one may be beyond the reported `regions`, and
    /// `get_reference_trace` names the same ids.
    pub region_ids: Vec<usize>,
    /// The line's bounding box, marks included.
    pub bounding_box: BoundingBox,
    /// Which way the line runs.
    pub orientation: Orientation,
    /// Where the letters sit across the line, in image pixels (`y` for
    /// horizontal text), measured at the line's start. For horizontal text this
    /// is the baseline; descenders hang below it.
    pub baseline: f64,
    /// Which edge of the letters `baseline` is measured on.
    pub baseline_edge: BaselineEdge,
    /// How far the baseline is turned from the image axis, in degrees:
    /// positive runs clockwise (downwards, for horizontal text). Zero for
    /// text that is level to within what the pixels can tell.
    pub slope_degrees: f64,
    /// The height of the tallest letters on the baseline: capitals and
    /// ascenders. Across the line, so width for vertical text. Font size is
    /// about this over the typeface's cap height, usually near 0.7.
    pub tall_height: f64,
    /// The height of the shorter letters on the baseline, the x-height of
    /// mixed case. `None` for text with only one height, as in capitals.
    pub short_height: Option<f64>,
    /// How many letter-sized regions the line has.
    pub glyph_count: usize,
    /// How many small regions belong to it: dots of i and j, accents,
    /// punctuation.
    pub mark_count: usize,
    /// Letters that are not on the baseline: descenders, punctuation, and
    /// anything raised or lowered.
    pub off_baseline_count: usize,
    /// Spacing of letters inside words.
    pub letter_gap: Gap,
    /// Mean gap of the spaces between words, when the line has more than one.
    pub word_gap: Option<f64>,
    /// The words, in reading order.
    pub words: Vec<TextWord>,
    /// 0-1. How strongly the geometry behaves like lettering: letters on a
    /// shared baseline, enough of them, of varied width. Not a claim that this
    /// is text.
    pub confidence: f64,
    /// How the line is filled, apart from the words in it.
    pub appearance: GroupAppearance,
}

/// One word of a [`TextLine`]: letters between two word spaces.
#[derive(Debug, Clone, Serialize)]
pub struct TextWord {
    /// Its regions, ascending.
    pub region_ids: Vec<usize>,
    /// The word's bounding box.
    pub bounding_box: BoundingBox,
    /// How many letter-sized regions it has.
    pub glyph_count: usize,
    /// How the word is filled, so a word and its neighbour (or a symbol) can
    /// carry different appearances.
    pub appearance: GroupAppearance,
}

/// How a group of regions is filled: one colour, or several.
#[derive(Debug, Clone, Serialize)]
pub struct GroupAppearance {
    /// Whether every member is one flat colour, within the noise.
    pub uniform: bool,
    /// The distinct fill kinds among the members: `flat`, `linear_gradient`,
    /// `radial_gradient` and `varied`.
    pub fill_kinds: Vec<&'static str>,
    /// The shared colour as `#rrggbb`, when `uniform`.
    pub colour: Option<String>,
    /// When not uniform, the distinct member colours in reading order, up to
    /// a handful. A gradient across the word shows here as a run of colours.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub colours: Vec<String>,
}

/// Consecutive lines set as one block.
#[derive(Debug, Clone, Serialize)]
pub struct TextBlock {
    /// The [`TextLine::id`]s, top to bottom.
    pub line_ids: Vec<usize>,
    /// What the lines share along their length.
    pub alignment: Alignment,
    /// Mean distance between consecutive baselines.
    pub line_pitch: f64,
    /// Standard deviation of those distances: zero for even leading.
    pub pitch_deviation: f64,
}

/// How a block's lines line up along their length.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Alignment {
    /// Left edges agree.
    Left,
    /// Centres agree.
    Centre,
    /// Right edges agree.
    Right,
    /// Both edges agree: lines of the same width.
    Justified,
    /// None of them agrees to within the tolerance.
    None,
}

/// Which edge of an item is compared across a line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Edge {
    Near,
    Far,
    Centre,
}

/// A region's box in the line frame: `a` along the line, `c` across it.
/// Ends are exclusive.
#[derive(Debug, Clone, Copy)]
struct Item {
    id: usize,
    a0: i64,
    a1: i64,
    c0: i64,
    c1: i64,
}

impl Item {
    fn of(region: &Region, orientation: Orientation) -> Self {
        let b = region.bounding_box;
        let (x, y, w, h) = (
            i64::from(b.x),
            i64::from(b.y),
            i64::from(b.width),
            i64::from(b.height),
        );
        match orientation {
            Orientation::Horizontal => Self {
                id: region.id,
                a0: x,
                a1: x + w,
                c0: y,
                c1: y + h,
            },
            Orientation::Vertical => Self {
                id: region.id,
                a0: y,
                a1: y + h,
                c0: x,
                c1: x + w,
            },
        }
    }

    fn a_ext(&self) -> f64 {
        (self.a1 - self.a0) as f64
    }

    fn c_ext(&self) -> f64 {
        (self.c1 - self.c0) as f64
    }

    fn a_mid(&self) -> f64 {
        (self.a0 + self.a1) as f64 / 2.0
    }

    fn c_mid(&self) -> f64 {
        (self.c0 + self.c1) as f64 / 2.0
    }

    fn edge(&self, edge: Edge) -> f64 {
        match edge {
            Edge::Near => self.c0 as f64,
            Edge::Far => self.c1 as f64,
            Edge::Centre => self.c_mid(),
        }
    }
}

/// A line that passed every test, before it is numbered and filled in.
struct Found {
    orientation: Orientation,
    edge: Edge,
    baseline: f64,
    slope: f64,
    tall: f64,
    short: Option<f64>,
    members: Vec<Item>,
    marks: Vec<Item>,
    off_baseline: usize,
    /// Indices into `members` at which a new word begins.
    word_starts: Vec<usize>,
    letter_gap: Gap,
    word_gap: Option<f64>,
    confidence: f64,
}

/// Find and describe the lettering among `regions`: every region of the
/// image, not only the reported ones, because a wordmark has more letters
/// than [`super::Analysis::regions`] holds.
pub(super) fn measure(
    pixels: &[u8],
    width: usize,
    region_map: &[u32],
    regions: &[Region],
) -> Typography {
    let found = find_lines(regions);
    if found.is_empty() {
        return Typography::default();
    }

    // Numbered by where they are, never by the order the detector met them.
    let mut lines: Vec<(BoundingBox, Found)> = found
        .into_iter()
        .map(|line| (line_box(&line), line))
        .collect();
    lines.sort_by(|a, b| {
        a.0.y
            .cmp(&b.0.y)
            .then(a.0.x.cmp(&b.0.x))
            .then(a.1.members[0].id.cmp(&b.1.members[0].id))
    });
    let line_count = lines.len();
    lines.truncate(MAX_LINES);

    // Each member is measured once, however many groups it belongs to.
    let by_id: BTreeMap<usize, &Region> = regions.iter().map(|r| (r.id, r)).collect();
    let mut looks: BTreeMap<usize, appearance::MemberLook> = BTreeMap::new();
    for (_, line) in &lines {
        for item in line.members.iter().chain(&line.marks) {
            looks.entry(item.id).or_insert_with(|| {
                appearance::member_look(pixels, width, region_map, by_id[&item.id])
            });
        }
    }

    let described: Vec<TextLine> = lines
        .into_iter()
        .enumerate()
        .map(|(id, (bounding_box, line))| describe(id, bounding_box, &line, &looks))
        .collect();
    let blocks = blocks(&described);

    Typography {
        line_count,
        lines: described,
        blocks,
    }
}

/// The geometry of lettering, with no pixels involved: horizontal text first,
/// then vertical text among what horizontal text left over.
fn find_lines(regions: &[Region]) -> Vec<Found> {
    if regions.len() < MIN_GLYPHS || regions.len() > MAX_CANDIDATES {
        return Vec::new();
    }

    let mut taken: BTreeSet<usize> = BTreeSet::new();
    let mut found = Vec::new();
    for orientation in [Orientation::Horizontal, Orientation::Vertical] {
        let items: Vec<Item> = regions
            .iter()
            .filter(|region| !taken.contains(&region.id))
            .map(|region| Item::of(region, orientation))
            .collect();

        let mut lines: Vec<Found> = chain(&items)
            .into_iter()
            .filter_map(|members| evaluate(members, orientation))
            .collect();

        let used: BTreeSet<usize> = lines
            .iter()
            .flat_map(|line| line.members.iter().map(|item| item.id))
            .collect();
        let spare: Vec<Item> = items
            .iter()
            .copied()
            .filter(|item| !used.contains(&item.id))
            .collect();
        attach_marks(&mut lines, &spare);

        for line in &lines {
            taken.extend(line.members.iter().chain(&line.marks).map(|item| item.id));
        }
        found.extend(lines);
    }
    found
}

/// Median of a slice, `0.0` when empty. Sorted by total order, so the same
/// values give the same answer however they arrived.
fn median(values: impl IntoIterator<Item = f64>) -> f64 {
    let mut values: Vec<f64> = values.into_iter().collect();
    values.sort_by(f64::total_cmp);
    match values.len() {
        0 => 0.0,
        n if n % 2 == 1 => values[n / 2],
        n => f64::midpoint(values[n / 2 - 1], values[n / 2]),
    }
}

/// Rounded to a tenth: the pixels cannot tell more, and it keeps a golden
/// stable across floating-point implementations.
fn tenth(value: f64) -> f64 {
    let rounded = (value * 10.0).round() / 10.0;
    // `-0.0` would print as a different golden to `0.0`.
    if rounded == 0.0 { 0.0 } else { rounded }
}

/// Rounded to a hundredth.
fn hundredth(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

/// Group letter-sized items into rows, greedily along the line.
///
/// Items are taken left to right and each joins the open row it fits best, or
/// starts a row. That lets several lines of one block be found in a single
/// sweep, because a letter of the second line fits the second row and not the
/// first.
fn chain(items: &[Item]) -> Vec<Vec<Item>> {
    let mut letters: Vec<Item> = items
        .iter()
        .copied()
        .filter(|item| item.c1 - item.c0 >= MIN_GLYPH_EXTENT)
        .collect();
    letters.sort_by(|p, q| {
        p.a_mid()
            .total_cmp(&q.a_mid())
            .then(p.c0.cmp(&q.c0))
            .then(p.id.cmp(&q.id))
    });

    let mut rows: Vec<Vec<Item>> = Vec::new();
    for item in letters {
        let mut best: Option<(usize, f64)> = None;
        for (index, row) in rows.iter().enumerate() {
            if let Some(score) = fit(row, &item)
                && best.is_none_or(|(_, top)| score > top)
            {
                best = Some((index, score));
            }
        }
        match best {
            Some((index, _)) => rows[index].push(item),
            None => rows.push(vec![item]),
        }
    }
    rows
}

/// How well `item` continues `row`, or `None` when it does not.
fn fit(row: &[Item], item: &Item) -> Option<f64> {
    let extent = median(row.iter().map(Item::c_ext));
    let ratio = item.c_ext() / extent;
    if !(1.0 / HEIGHT_RATIO..=HEIGHT_RATIO).contains(&ratio) {
        return None;
    }

    let top = median(row.iter().map(|i| i.c0 as f64));
    let bottom = median(row.iter().map(|i| i.c1 as f64));
    let overlap = ((item.c1 as f64).min(bottom) - (item.c0 as f64).max(top)).max(0.0);
    let share = overlap / item.c_ext().min(bottom - top).max(1.0);
    if share < MIN_BAND_OVERLAP {
        return None;
    }

    let last = row.iter().max_by_key(|i| i.a1)?;
    let gap = (item.a0 - last.a1) as f64;
    // A neighbour that sits mostly over the last letter is a mark on it or a
    // piece of it, not the next letter.
    if -gap > MAX_ALONG_OVERLAP * item.a_ext().min(last.a_ext()) {
        return None;
    }
    if gap > MAX_GAP_EXTENTS * extent {
        return None;
    }
    Some(share)
}

/// Turn a row of letters into a described line, or reject it.
fn evaluate(members: Vec<Item>, orientation: Orientation) -> Option<Found> {
    let count = members.len();
    if count < MIN_GLYPHS {
        return None;
    }

    let extent = median(members.iter().map(Item::c_ext));
    let aspect = median(members.iter().map(|i| i.a_ext() / i.c_ext()));
    if !(MIN_ASPECT..=MAX_ASPECT).contains(&aspect) {
        return None;
    }

    // Letters differ in width and height, a pattern does not.
    let widths: Vec<f64> = members.iter().map(Item::a_ext).collect();
    let heights: Vec<f64> = members.iter().map(Item::c_ext).collect();
    let spread = |values: &[f64]| {
        values.iter().copied().fold(f64::MIN, f64::max)
            - values.iter().copied().fold(f64::MAX, f64::min)
    };

    let gaps = gaps_of(&members);
    // Shapes of one size are repetition however they are spaced: a missing
    // one leaves a wider gap, not a word space. The `noisy` fixture's specks
    // are exactly this, five-pixel squares on a grid with some left out.
    if spread(&widths) <= 1.0 && spread(&heights) <= 1.0 {
        return None;
    }

    // Which edge the letters agree on. Horizontal text has a baseline, at the
    // bottom; vertical text may be aligned on either side or on its middle.
    let edges: &[Edge] = match orientation {
        Orientation::Horizontal => &[Edge::Far],
        Orientation::Vertical => &[Edge::Centre, Edge::Near, Edge::Far],
    };
    let origin = members.iter().map(|i| i.a0).min()? as f64;
    let tolerance = MIN_TOLERANCE.max(TOLERANCE_FRACTION * extent);

    let mut best: Option<(Edge, f64, f64, f64)> = None;
    for &edge in edges {
        let slope = slope_of(&members, edge);
        let residuals: Vec<f64> = members
            .iter()
            .map(|i| i.edge(edge) - slope * (i.a_mid() - origin))
            .collect();
        let baseline = median(residuals.iter().copied());
        let share = residuals
            .iter()
            .filter(|r| (**r - baseline).abs() <= tolerance)
            .count() as f64
            / count as f64;
        if best.is_none_or(|(_, _, _, top)| share > top) {
            best = Some((edge, baseline, slope, share));
        }
    }
    let (edge, baseline, slope, share) = best?;
    if share < MIN_BASELINE_SHARE {
        return None;
    }

    // Heights are taken from the letters on the baseline only, so a descender
    // does not make its letter look tall.
    let on: Vec<f64> = members
        .iter()
        .filter(|i| {
            let residual = i.edge(edge) - slope * (i.a_mid() - origin);
            (residual - baseline).abs() <= tolerance
        })
        .map(Item::c_ext)
        .collect();
    let top = on.iter().copied().fold(0.0, f64::max);
    let tall = median(on.iter().copied().filter(|h| *h >= TALL_GROUP * top));
    let shorter: Vec<f64> = on
        .iter()
        .copied()
        .filter(|h| *h < SHORT_GROUP * tall)
        .collect();
    let short = (shorter.len() as f64 >= MIN_SHORT_SHARE * on.len() as f64 && shorter.len() >= 2)
        .then(|| median(shorter));

    // Words are split where a gap is much larger than the usual letter gap.
    let usual = median(gaps.iter().copied());
    let threshold = (WORD_GAP_RATIO * usual).max(WORD_GAP_HEIGHT * tall);
    let mut word_starts = vec![0];
    let mut letter_gaps = Vec::new();
    let mut word_gaps = Vec::new();
    for (index, gap) in gaps.iter().enumerate() {
        if *gap > threshold {
            word_starts.push(index + 1);
            word_gaps.push(*gap);
        } else {
            letter_gaps.push(*gap);
        }
    }
    let mean = |values: &[f64]| values.iter().sum::<f64>() / values.len().max(1) as f64;
    let letter_mean = mean(&letter_gaps);
    let deviation = (letter_gaps
        .iter()
        .map(|g| (g - letter_mean).powi(2))
        .sum::<f64>()
        / letter_gaps.len().max(1) as f64)
        .sqrt();

    let count_score = ((count as f64 - 1.0) / (COUNT_SATURATION - 1.0)).clamp(0.0, 1.0);
    // Either extent may be the one that varies: the stacked, upright letters of
    // vertical text are all as tall as each other and differ in width.
    let variety = (spread(&widths).max(spread(&heights)) / (VARIETY_SATURATION * extent.max(1.0)))
        .clamp(0.0, 1.0);
    let confidence = 0.4 * share + 0.3 * count_score + 0.3 * variety;
    if confidence < MIN_CONFIDENCE {
        return None;
    }

    Some(Found {
        orientation,
        edge,
        baseline,
        slope,
        tall,
        short,
        off_baseline: count - on.len(),
        members,
        marks: Vec::new(),
        word_starts,
        letter_gap: Gap {
            mean: letter_mean,
            deviation,
        },
        word_gap: (!word_gaps.is_empty()).then(|| mean(&word_gaps)),
        confidence,
    })
}

/// Gaps between each letter and the furthest extent of those before it, never
/// negative: italics and kerning overlap, which is a gap of zero.
fn gaps_of(members: &[Item]) -> Vec<f64> {
    let mut reach = i64::MIN;
    let mut gaps = Vec::new();
    for (index, item) in members.iter().enumerate() {
        if index > 0 {
            gaps.push((item.a0 - reach).max(0) as f64);
        }
        reach = reach.max(item.a1);
    }
    gaps
}

/// The slope of the chosen edge along the line: the median of the slopes
/// between pairs of letters (Theil-Sen), so a descender or a raised letter
/// moves it far less than a least-squares fit would.
fn slope_of(members: &[Item], edge: Edge) -> f64 {
    let step = members.len().div_ceil(SLOPE_SAMPLES).max(1);
    let sample: Vec<&Item> = members.iter().step_by(step).collect();
    let mut slopes = Vec::new();
    for (index, p) in sample.iter().enumerate() {
        for q in &sample[index + 1..] {
            let run = q.a_mid() - p.a_mid();
            if run >= 1.0 {
                slopes.push((q.edge(edge) - p.edge(edge)) / run);
            }
        }
    }
    median(slopes)
}

/// Give every spare small region to the line it belongs to: a dot above a
/// stem, an accent, a comma at the end.
fn attach_marks(lines: &mut [Found], spare: &[Item]) {
    for item in spare {
        let mut best: Option<(usize, f64)> = None;
        for (index, line) in lines.iter().enumerate() {
            if let Some(distance) = mark_distance(line, item)
                && best.is_none_or(|(_, top)| distance < top)
            {
                best = Some((index, distance));
            }
        }
        if let Some((index, _)) = best {
            lines[index].marks.push(*item);
        }
    }
}

/// How far `item` is from `line`, or `None` when it is not a mark of it.
fn mark_distance(line: &Found, item: &Item) -> Option<f64> {
    let limit = MAX_MARK_FRACTION * line.tall;
    if item.a_ext() > limit.max(1.0) || item.c_ext() > limit.max(1.0) {
        return None;
    }
    let reach = MARK_REACH * line.tall;
    let band_start = line.members.iter().map(|i| i.c0).min()? as f64 - reach;
    let band_end = line.members.iter().map(|i| i.c1).max()? as f64 + reach;

    let mut best: Option<f64> = None;
    for member in &line.members {
        let along = (item.a0 - member.a1).max(member.a0 - item.a1).max(0) as f64;
        let across = (item.c0 - member.c1).max(member.c0 - item.c1).max(0) as f64;
        // Over or under a letter, like the dot of an i; or in the line, like
        // a full stop after the last one.
        let attached = (along == 0.0 && across <= reach)
            || (along <= reach && (band_start..=band_end).contains(&item.c_mid()));
        if attached {
            let distance = along + across;
            if best.is_none_or(|top| distance < top) {
                best = Some(distance);
            }
        }
    }
    best
}

/// The bounding box of a line's regions, in image coordinates.
fn line_box(line: &Found) -> BoundingBox {
    let items = || line.members.iter().chain(&line.marks);
    let a0 = items().map(|i| i.a0).min().unwrap_or(0);
    let a1 = items().map(|i| i.a1).max().unwrap_or(0);
    let c0 = items().map(|i| i.c0).min().unwrap_or(0);
    let c1 = items().map(|i| i.c1).max().unwrap_or(0);
    to_box(line.orientation, a0, a1, c0, c1)
}

/// A box in the line frame, back in image coordinates.
fn to_box(orientation: Orientation, a0: i64, a1: i64, c0: i64, c1: i64) -> BoundingBox {
    let (x0, x1, y0, y1) = match orientation {
        Orientation::Horizontal => (a0, a1, c0, c1),
        Orientation::Vertical => (c0, c1, a0, a1),
    };
    BoundingBox {
        x: x0 as u32,
        y: y0 as u32,
        width: (x1 - x0) as u32,
        height: (y1 - y0) as u32,
    }
}

/// Fill in a found line: words, appearance and rounded numbers.
fn describe(
    id: usize,
    bounding_box: BoundingBox,
    line: &Found,
    looks: &BTreeMap<usize, appearance::MemberLook>,
) -> TextLine {
    // Words own the marks nearest to them along the line.
    let mut words: Vec<(Vec<Item>, Vec<Item>)> = line
        .word_starts
        .iter()
        .enumerate()
        .map(|(index, &start)| {
            let end = line
                .word_starts
                .get(index + 1)
                .copied()
                .unwrap_or(line.members.len());
            (line.members[start..end].to_vec(), Vec::new())
        })
        .collect();
    for mark in &line.marks {
        let nearest = words
            .iter()
            .enumerate()
            .min_by(|(_, p), (_, q)| {
                let distance = |word: &(Vec<Item>, Vec<Item>)| {
                    word.0
                        .iter()
                        .map(|m| (mark.a0 - m.a1).max(m.a0 - mark.a1).max(0))
                        .min()
                        .unwrap_or(i64::MAX)
                };
                distance(p).cmp(&distance(q))
            })
            .map(|(index, _)| index);
        if let Some(index) = nearest {
            words[index].1.push(*mark);
        }
    }

    let words: Vec<TextWord> = words
        .into_iter()
        .map(|(letters, marks)| {
            let items: Vec<Item> = letters.iter().chain(&marks).copied().collect();
            let a0 = items.iter().map(|i| i.a0).min().unwrap_or(0);
            let a1 = items.iter().map(|i| i.a1).max().unwrap_or(0);
            let c0 = items.iter().map(|i| i.c0).min().unwrap_or(0);
            let c1 = items.iter().map(|i| i.c1).max().unwrap_or(0);
            TextWord {
                region_ids: sorted_ids(&items),
                bounding_box: to_box(line.orientation, a0, a1, c0, c1),
                glyph_count: letters.len(),
                appearance: group_appearance(&items, looks),
            }
        })
        .collect();

    let all: Vec<Item> = line.members.iter().chain(&line.marks).copied().collect();
    let edge = match (line.orientation, line.edge) {
        (Orientation::Horizontal, _) => BaselineEdge::Bottom,
        (Orientation::Vertical, Edge::Near) => BaselineEdge::Left,
        (Orientation::Vertical, Edge::Far) => BaselineEdge::Right,
        (Orientation::Vertical, Edge::Centre) => BaselineEdge::Centre,
    };

    TextLine {
        id,
        region_ids: sorted_ids(&all),
        bounding_box,
        orientation: line.orientation,
        baseline: tenth(line.baseline),
        baseline_edge: edge,
        slope_degrees: tenth(line.slope.atan().to_degrees()),
        tall_height: tenth(line.tall),
        short_height: line.short.map(tenth),
        glyph_count: line.members.len(),
        mark_count: line.marks.len(),
        off_baseline_count: line.off_baseline,
        letter_gap: Gap {
            mean: tenth(line.letter_gap.mean),
            deviation: tenth(line.letter_gap.deviation),
        },
        word_gap: line.word_gap.map(tenth),
        words,
        confidence: hundredth(line.confidence),
        appearance: group_appearance(&all, looks),
    }
}

fn sorted_ids(items: &[Item]) -> Vec<usize> {
    let mut ids: Vec<usize> = items.iter().map(|i| i.id).collect();
    ids.sort_unstable();
    ids
}

/// Summarise how a group of regions is filled.
fn group_appearance(
    items: &[Item],
    looks: &BTreeMap<usize, appearance::MemberLook>,
) -> GroupAppearance {
    // Reading order, so `colours` runs along the line.
    let mut ordered: Vec<&Item> = items.iter().collect();
    ordered.sort_by(|p, q| p.a_mid().total_cmp(&q.a_mid()).then(p.id.cmp(&q.id)));
    let members: Vec<&appearance::MemberLook> =
        ordered.iter().filter_map(|i| looks.get(&i.id)).collect();

    let mut kinds: Vec<&'static str> = members.iter().map(|m| m.kind).collect();
    kinds.sort_unstable();
    kinds.dedup();

    let count = members.len().max(1) as f64;
    let mut mean = [0.0; 3];
    for member in &members {
        for (total, channel) in mean.iter_mut().zip(member.colour) {
            *total += channel / count;
        }
    }
    let distance = |a: [f64; 3], b: [f64; 3]| {
        a.iter()
            .zip(b)
            .map(|(p, q)| (p - q).powi(2))
            .sum::<f64>()
            .sqrt()
    };
    let hex = |c: [f64; 3]| {
        let channel = |v: f64| v.round().clamp(0.0, 255.0) as u8;
        super::hex([channel(c[0]), channel(c[1]), channel(c[2])])
    };

    let gradient = kinds
        .iter()
        .any(|k| matches!(*k, "linear_gradient" | "radial_gradient"));
    let uniform = !members.is_empty()
        && !gradient
        && members
            .iter()
            .all(|m| distance(m.colour, mean) <= GROUP_COLOUR_TOLERANCE);

    let mut colours: Vec<[f64; 3]> = Vec::new();
    if !uniform {
        for member in &members {
            if colours.len() < MAX_GROUP_COLOURS
                && colours
                    .iter()
                    .all(|seen| distance(*seen, member.colour) > GROUP_COLOUR_TOLERANCE)
            {
                colours.push(member.colour);
            }
        }
    }

    GroupAppearance {
        uniform,
        fill_kinds: kinds,
        colour: uniform.then(|| hex(mean)),
        colours: colours.into_iter().map(hex).collect(),
    }
}

/// Group consecutive horizontal lines that are set together.
fn blocks(lines: &[TextLine]) -> Vec<TextBlock> {
    // Top to bottom; ids already run that way, but this does not rely on it.
    let mut horizontal: Vec<&TextLine> = lines
        .iter()
        .filter(|l| l.orientation == Orientation::Horizontal)
        .collect();
    horizontal.sort_by_key(|l| (l.bounding_box.y, l.bounding_box.x, l.id));

    let mut groups: Vec<Vec<&TextLine>> = Vec::new();
    for line in horizontal {
        let joins = groups.last().and_then(|g| g.last()).is_some_and(|prev| {
            let (p, q) = (prev.bounding_box, line.bounding_box);
            let overlap = (i64::from(p.x + p.width).min(i64::from(q.x + q.width))
                - i64::from(p.x).max(i64::from(q.x)))
            .max(0) as f64;
            let narrower = f64::from(p.width.min(q.width)).max(1.0);
            let gap = i64::from(q.y) - i64::from(p.y + p.height);
            overlap / narrower >= BLOCK_MIN_OVERLAP
                && (gap as f64) <= BLOCK_REACH * prev.tall_height.max(line.tall_height)
        });
        match groups.last_mut() {
            Some(group) if joins => group.push(line),
            _ => groups.push(vec![line]),
        }
    }

    groups
        .into_iter()
        .filter(|group| group.len() >= 2)
        .map(|group| {
            let lefts: Vec<f64> = group.iter().map(|l| f64::from(l.bounding_box.x)).collect();
            let rights: Vec<f64> = group
                .iter()
                .map(|l| f64::from(l.bounding_box.x + l.bounding_box.width))
                .collect();
            let centres: Vec<f64> = lefts
                .iter()
                .zip(&rights)
                .map(|(l, r)| f64::midpoint(*l, *r))
                .collect();
            let spread = |values: &[f64]| {
                values.iter().copied().fold(f64::MIN, f64::max)
                    - values.iter().copied().fold(f64::MAX, f64::min)
            };
            let width = spread(&[
                lefts.iter().copied().fold(f64::MAX, f64::min),
                rights.iter().copied().fold(f64::MIN, f64::max),
            ]);
            let tolerance = MIN_ALIGNMENT_TOLERANCE.max(ALIGNMENT_TOLERANCE_FRACTION * width);
            let (left, right, centre) = (
                spread(&lefts) <= tolerance,
                spread(&rights) <= tolerance,
                spread(&centres) <= tolerance,
            );
            let alignment = match (left, right, centre) {
                (true, true, _) => Alignment::Justified,
                (true, false, _) => Alignment::Left,
                (false, true, _) => Alignment::Right,
                (false, false, true) => Alignment::Centre,
                (false, false, false) => Alignment::None,
            };

            let pitches: Vec<f64> = group
                .windows(2)
                .map(|pair| pair[1].baseline - pair[0].baseline)
                .collect();
            let pitch = pitches.iter().sum::<f64>() / pitches.len() as f64;
            let deviation = (pitches.iter().map(|p| (p - pitch).powi(2)).sum::<f64>()
                / pitches.len() as f64)
                .sqrt();

            TextBlock {
                line_ids: group.iter().map(|l| l.id).collect(),
                alignment,
                line_pitch: tenth(pitch),
                pitch_deviation: tenth(deviation),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::Centroid;

    /// A region with a box and nothing else: all the detector reads.
    fn region(id: usize, x: u32, y: u32, width: u32, height: u32) -> Region {
        Region {
            id,
            bounding_box: BoundingBox {
                x,
                y,
                width,
                height,
            },
            area: u64::from(width) * u64::from(height),
            centroid: Centroid {
                x: f64::from(x) + f64::from(width) / 2.0,
                y: f64::from(y) + f64::from(height) / 2.0,
            },
        }
    }

    /// Letters of varied width, one height, sitting on `bottom`, from `x`.
    fn word(first_id: usize, x: u32, bottom: u32, widths: &[u32], height: u32) -> Vec<Region> {
        let mut at = x;
        widths
            .iter()
            .enumerate()
            .map(|(index, &width)| {
                let r = region(first_id + index, at, bottom - height, width, height);
                at += width + 2;
                r
            })
            .collect()
    }

    #[test]
    fn a_row_of_varied_letters_on_a_baseline_is_a_line() {
        let regions = word(0, 4, 30, &[5, 8, 6, 9, 4], 10);
        let lines = find_lines(&regions);

        assert_eq!(lines.len(), 1, "one row of letters is one line");
        assert_eq!(lines[0].members.len(), 5);
        assert_eq!(lines[0].baseline, 30.0, "the bottom edge is the baseline");
        assert_eq!(lines[0].tall, 10.0);
    }

    #[test]
    fn a_row_of_shapes_of_one_size_is_a_pattern_not_lettering() {
        let regions = word(0, 4, 30, &[6, 6, 6, 6, 6, 6], 10);
        assert!(
            find_lines(&regions).is_empty(),
            "repetition is not lettering"
        );

        // Nor is it once a few are missing and the gaps are uneven.
        let gappy = vec![
            region(0, 4, 20, 6, 10),
            region(1, 14, 20, 6, 10),
            region(2, 34, 20, 6, 10),
            region(3, 44, 20, 6, 10),
            region(4, 64, 20, 6, 10),
        ];
        assert!(find_lines(&gappy).is_empty(), "uneven gaps are not letters");
    }

    #[test]
    fn two_letters_are_a_pair_and_not_a_line() {
        let regions = word(0, 4, 30, &[5, 8], 10);

        assert!(find_lines(&regions).is_empty());
    }

    #[test]
    fn shapes_that_share_no_baseline_are_not_a_line() {
        // Same size and in a row from left to right, but each at its own
        // height: a staircase, not a line of text.
        let regions = vec![
            region(0, 4, 4, 6, 10),
            region(1, 14, 12, 9, 10),
            region(2, 26, 22, 5, 10),
            region(3, 34, 32, 8, 10),
        ];

        assert!(find_lines(&regions).is_empty());
    }

    #[test]
    fn a_descender_does_not_take_a_letter_off_the_baseline_of_the_others() {
        let mut regions = word(0, 4, 30, &[5, 8, 6, 9, 4], 10);
        // One letter hangs below the others, as a g or a p does.
        regions[2] = region(2, regions[2].bounding_box.x, 20, 6, 14);
        let lines = find_lines(&regions);

        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].off_baseline, 1, "the descender is counted");
        assert_eq!(lines[0].baseline, 30.0);
        assert_eq!(
            lines[0].tall, 10.0,
            "its height is not the line's letter height"
        );
    }

    #[test]
    fn a_dot_above_a_stem_belongs_to_the_line_and_is_not_a_letter() {
        let mut regions = word(0, 4, 30, &[5, 8, 6, 9, 4], 10);
        let stem = regions[2].bounding_box;
        // The dot of an i, centred over the third letter, three pixels above.
        regions.push(region(5, stem.x + 2, stem.y - 5, 2, 2));
        let lines = find_lines(&regions);

        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].marks.len(), 1);
        assert_eq!(lines[0].members.len(), 5, "the dot is not counted a letter");
    }

    #[test]
    fn a_speck_far_from_any_line_is_left_alone() {
        let mut regions = word(0, 4, 30, &[5, 8, 6, 9, 4], 10);
        regions.push(region(5, 60, 2, 2, 2));
        let lines = find_lines(&regions);

        assert_eq!(lines.len(), 1);
        assert!(lines[0].marks.is_empty());
    }

    #[test]
    fn a_wide_gap_splits_a_line_into_words() {
        let mut regions = word(0, 4, 30, &[5, 8, 6], 10);
        regions.extend(word(3, 40, 30, &[9, 4, 7], 10));
        let lines = find_lines(&regions);

        assert_eq!(lines.len(), 1, "a word space does not end the line");
        assert_eq!(lines[0].word_starts, vec![0, 3]);
        assert!(lines[0].word_gap.is_some());
    }

    #[test]
    fn mixed_case_reports_a_short_height_and_capitals_do_not() {
        // A capital then lowercase letters, all on one baseline.
        let mut regions = word(0, 4, 30, &[7, 5, 6, 5, 6], 10);
        for region in &mut regions[1..] {
            region.bounding_box.y += 3;
            region.bounding_box.height = 7;
        }
        let mixed = find_lines(&regions);
        assert_eq!(mixed[0].short, Some(7.0));
        assert_eq!(mixed[0].tall, 10.0);

        let capitals = find_lines(&word(0, 4, 30, &[7, 5, 6, 5, 6], 10));
        assert_eq!(capitals[0].short, None);
    }

    #[test]
    fn bars_of_a_chart_are_not_lettering() {
        // Left-aligned slabs stacked down the image: very wide for their
        // height, whichever way the line is read.
        let regions = vec![
            region(0, 4, 4, 40, 4),
            region(1, 4, 12, 28, 4),
            region(2, 4, 20, 52, 4),
            region(3, 4, 28, 20, 4),
        ];

        assert!(find_lines(&regions).is_empty());
    }

    #[test]
    fn upright_letters_stacked_and_centred_are_a_vertical_line() {
        // Four letters one above the other, centred on one axis.
        let regions = vec![
            region(0, 20, 4, 12, 12),
            region(1, 22, 20, 8, 12),
            region(2, 19, 36, 14, 12),
            region(3, 21, 52, 10, 12),
        ];
        let lines = find_lines(&regions);

        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].orientation, Orientation::Vertical);
        assert_eq!(lines[0].edge, Edge::Centre);
        assert_eq!(lines[0].baseline, 26.0, "the shared axis, in x");
    }

    #[test]
    fn a_slightly_turned_baseline_is_followed_and_its_slope_reported() {
        // Bottoms climb one pixel every fourth letter-width: about 2 degrees.
        let regions: Vec<Region> = (0..8)
            .map(|i| {
                let widths = [5, 8, 6, 9, 4, 7, 5, 8];
                let x = 4 + i * 14;
                let bottom = 40 - (i * 14) / 28;
                region(i as usize, x, bottom - 10, widths[i as usize], 10)
            })
            .collect();
        let lines = find_lines(&regions);

        assert_eq!(lines.len(), 1, "a gentle slope is still one line");
        assert!(lines[0].slope < 0.0, "rising to the right is negative");
    }

    #[test]
    fn two_lines_of_a_block_are_found_in_one_pass_and_aligned() {
        let mut regions = word(0, 4, 20, &[5, 8, 6, 9], 10);
        regions.extend(word(4, 4, 40, &[8, 6, 5, 9], 10));
        let found = find_lines(&regions);
        assert_eq!(found.len(), 2);

        let described: Vec<TextLine> = found
            .iter()
            .enumerate()
            .map(|(id, line)| describe(id, line_box(line), line, &BTreeMap::new()))
            .collect();
        let blocks = blocks(&described);

        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].line_ids, vec![0, 1]);
        assert_eq!(blocks[0].alignment, Alignment::Justified);
        assert_eq!(blocks[0].line_pitch, 20.0);
    }

    #[test]
    fn too_many_regions_are_never_considered() {
        let regions: Vec<Region> = (0..=MAX_CANDIDATES)
            .map(|i| region(i, 0, 0, 5, 8))
            .collect();

        assert!(find_lines(&regions).is_empty());
    }

    #[test]
    fn detection_is_independent_of_the_order_regions_arrive_in() {
        let regions = word(0, 4, 30, &[5, 8, 6, 9, 4], 10);
        let mut reversed = regions.clone();
        reversed.reverse();

        let a = find_lines(&regions);
        let b = find_lines(&reversed);

        assert_eq!(a.len(), b.len());
        assert_eq!(a[0].baseline, b[0].baseline);
        assert_eq!(a[0].confidence, b[0].confidence);
    }
}
