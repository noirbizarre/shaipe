//! How a logo is put together from visual components. See ADR 029.
//!
//! The analysis reports connected regions one by one, and typography reports
//! the ones that are lettering. Neither says that a ring and the small disc
//! beside it are *one mark*, that a wordmark is centred under it, or that
//! four squares are one row spaced alike. That is what an agent needs to place
//! shapes in an SVG from measurements rather than by eye, and it is what this
//! module reports: components, and the relationships between them.
//!
//! It makes no judgement a pixel cannot back. A component's *role* is its size
//! against the largest one, never what it depicts: Shaipe does not know what a
//! logo means (ADR 004). Geometry and appearance are reported apart, in
//! sibling objects of a component, and every relationship, alignment, spacing
//! and repetition is geometry alone, so recolouring artwork changes the one
//! and leaves the other untouched.
//!
//! The honest default is to report nothing: fewer than two components is not
//! a composition, and more than [`MAX_COMPONENTS`] of them, or one made of more
//! than [`MAX_FRAGMENTS`] pieces, is a pattern or a texture.
//! Everything is integer geometry and sorts with total orders, so the same
//! regions produce the same composition on any machine.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use super::{BoundingBox, Gap, GroupAppearance, Point, Region, Typography, appearance, typography};

/// More regions than this and nothing is considered: a photograph or a
/// texture, not a composition, and it bounds the cost of grouping them.
const MAX_REGIONS: usize = 512;
/// More components than this and nothing is reported. A composition is a
/// handful of parts; dozens of them are a pattern, and listing every pair of
/// them would bury what an agent needs.
const MAX_COMPONENTS: usize = 32;
/// Tolerance for "the same", at least this many pixels: an anti-aliased edge
/// is up to a pixel soft and a bounding box rounds out.
const MIN_TOLERANCE: f64 = 2.0;
/// The tolerance as a share of the longer side of the image, when larger. One
/// tolerance serves for merging fragments, alignment, spacing and size, so
/// the same two pixels are never a gap in one place and nothing in another.
const TOLERANCE_FRACTION: f64 = 0.015;
/// A component of more regions than this is a texture. One shape that an
/// encoder broke into a few pieces is a component; a heap of a dozen specks
/// that happen to lie close is noise, and calling it "the primary artwork"
/// would mislead the agent more than reporting nothing. Lettering is exempt:
/// a line has as many letters as it has.
const MAX_FRAGMENTS: usize = 8;
/// A component at least this share of the largest one's area is primary
/// artwork: the thing the logo is, not something beside it.
const PRIMARY_SHARE: f64 = 0.5;
/// A component under this share of the largest one's area, and enclosing
/// nothing, is decorative: a dot, a spark, a flourish.
const DECORATIVE_SHARE: f64 = 0.05;
/// A run of this many components is a row, a column or a repetition. Two are a
/// pair, and a pair is only ever alike by chance.
const MIN_RUN: usize = 3;

/// How the image's regions are put together.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Composition {
    /// How many components were found. Zero when there are fewer than two,
    /// or too many to be a composition.
    pub component_count: usize,
    /// The components, largest first.
    pub components: Vec<Component>,
    /// Pairs of components and where one is against the other.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub relationships: Vec<Relationship>,
    /// Components that share an edge or a centre line.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub alignments: Vec<AlignedGroup>,
    /// Rows and columns of three or more components, and how far apart they
    /// are set.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub spacings: Vec<Spacing>,
    /// Components of one size, three or more.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub repeats: Vec<Repeat>,
}

impl Composition {
    /// Whether there is anything to report. A single shape, or a single line
    /// of lettering, is not a composition and leaves the field out entirely.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.component_count < 2
    }
}

/// What a component is to the whole, judged by size alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// As large as the largest component, or nearly: the artwork itself.
    Primary,
    /// Smaller than the primary artwork and not a small ornament.
    Secondary,
    /// Very small against the largest component and enclosing nothing: a dot,
    /// a spark, an ornament.
    Decorative,
    /// A line of lettering that typography found. It is described there; here
    /// it is only a part among others.
    Lettering,
}

/// One visual component: regions that belong together.
#[derive(Debug, Clone, Serialize)]
pub struct Component {
    /// Position in `components`: largest area first. The other lists refer to
    /// components by this.
    pub id: usize,
    /// Primary artwork, a secondary part, an ornament or lettering.
    pub role: Role,
    /// The [`super::Region::id`]s that make it up, ascending.
    pub region_ids: Vec<usize>,
    /// Where it is, how big, and how its outline is drawn.
    pub geometry: ComponentGeometry,
    /// How it is filled, apart from where it is.
    pub appearance: GroupAppearance,
}

/// A component's geometry.
#[derive(Debug, Clone, Serialize)]
pub struct ComponentGeometry {
    /// The box around all of its regions.
    pub bounding_box: BoundingBox,
    /// Pixels it covers.
    pub area: u64,
    /// Its share of everything that is not background, 0-1.
    pub area_share: f64,
    /// Where its mass is, weighted by region area.
    pub centroid: Point,
    /// How many regions it has.
    pub region_count: usize,
    /// Closed, open or filled. Absent for lettering, whose letters are
    /// described by typography.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contour: Option<Contour>,
    /// For lettering, the [`super::TextLine::id`] it is.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line_id: Option<usize>,
}

/// Whether a component is an outline that closes, a line that does not, or a
/// solid shape.
#[derive(Debug, Clone, Serialize)]
pub struct Contour {
    /// Which of the three it is.
    pub kind: ContourKind,
    /// How many holes its regions enclose, all of them.
    pub hole_count: usize,
    /// For a component shaped like a stroke, how wide the stroke is.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stroke_width: Option<f64>,
}

/// The kind of contour a component has.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ContourKind {
    /// It encloses at least one hole: a ring, a frame, an outline, a shape
    /// with a cut-out.
    Closed,
    /// It is a stroke that encloses nothing: a bar, an arc, an open outline.
    /// Drawn closed, it would be a different shape.
    Open,
    /// A solid shape, neither.
    Filled,
}

/// The ways one component can sit against another.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Relation {
    /// `to` lies inside the bounding box of `from`.
    Contains,
    /// The boxes cross, and neither holds the other.
    Overlaps,
    /// `to` is the nearest component on the right of `from`, with some height
    /// in common.
    RightOf,
    /// `to` is the nearest component below `from`, with some width in common.
    Below,
}

/// How one component is placed against another.
#[derive(Debug, Clone, Serialize)]
pub struct Relationship {
    /// The [`Component::id`] the relation is seen from.
    pub from: usize,
    /// The [`Component::id`] it describes.
    pub to: usize,
    /// How `to` sits against `from`.
    pub relation: Relation,
    /// Pixels of empty space between the two boxes. Zero when they touch,
    /// cross or nest.
    pub gap: u32,
    /// Centre of `to` minus centre of `from`, in pixels.
    pub centre_offset: Point,
    /// `to`'s area over `from`'s.
    pub size_ratio: f64,
}

/// A line that more than one component's edge or centre lies on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    /// A horizontal line, shared `y` positions, or a run laid out left to
    /// right.
    Horizontal,
    /// A vertical line, shared `x` positions, or a run laid out top to
    /// bottom.
    Vertical,
}

/// Which part of a component is compared across a line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AlignedEdge {
    /// The left edge of a vertical line, the top of a horizontal one.
    Start,
    /// The centre.
    Centre,
    /// The right edge of a vertical line, the bottom of a horizontal one.
    End,
}

/// Components that agree on one line, to within the tolerance.
#[derive(Debug, Clone, Serialize)]
pub struct AlignedGroup {
    /// The shared line: `vertical` when their `x` agree, `horizontal` when
    /// their `y` do.
    pub line: Direction,
    /// Which part of each lies on it: `start` (left or top), `centre` or `end`
    /// (right or bottom).
    pub edge: AlignedEdge,
    /// Where the line is, as the mean of the members, in pixels.
    pub position: f64,
    /// The [`Component::id`]s, ascending.
    pub component_ids: Vec<usize>,
}

/// A row or column of three or more components, and how far apart they are.
#[derive(Debug, Clone, Serialize)]
pub struct Spacing {
    /// Which way the run is laid out.
    pub along: Direction,
    /// The [`Component::id`]s, in order along the run.
    pub component_ids: Vec<usize>,
    /// The empty space between neighbours: its mean and deviation, in pixels.
    pub gap: Gap,
    /// Whether the gaps agree to within the tolerance, so one value spaces the
    /// whole run.
    pub even: bool,
}

/// Components of one size.
#[derive(Debug, Clone, Serialize)]
pub struct Repeat {
    /// The [`Component::id`]s, ascending.
    pub component_ids: Vec<usize>,
    /// Their width, in pixels: the mean.
    pub width: f64,
    /// Their height, in pixels: the mean.
    pub height: f64,
}

/// What [`measure`] reads. Gathered into one value because the region map,
/// the regions and the hole counts all describe the same image.
pub(super) struct Input<'a> {
    /// RGBA8 pixels, row-major.
    pub pixels: &'a [u8],
    /// Image width.
    pub width: usize,
    /// Image height.
    pub height: usize,
    /// The id of the region at each pixel, or [`super::NO_REGION`].
    pub region_map: &'a [u32],
    /// Every region, not the reported ones: a component can be made of more
    /// regions than [`super::Analysis::regions`] holds.
    pub regions: &'a [Region],
    /// How many holes each region encloses, indexed by region id.
    pub hole_counts: &'a [u32],
}

/// An axis-aligned box in integers, its far edges exclusive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Rect {
    x0: i64,
    y0: i64,
    x1: i64,
    y1: i64,
}

impl Rect {
    fn of(b: &BoundingBox) -> Self {
        Self {
            x0: i64::from(b.x),
            y0: i64::from(b.y),
            x1: i64::from(b.x) + i64::from(b.width),
            y1: i64::from(b.y) + i64::from(b.height),
        }
    }

    fn union(self, other: Self) -> Self {
        Self {
            x0: self.x0.min(other.x0),
            y0: self.y0.min(other.y0),
            x1: self.x1.max(other.x1),
            y1: self.y1.max(other.y1),
        }
    }

    fn width(self) -> i64 {
        self.x1 - self.x0
    }

    fn height(self) -> i64 {
        self.y1 - self.y0
    }

    fn to_box(self) -> BoundingBox {
        // Never negative: a union of boxes only grows.
        BoundingBox {
            x: self.x0 as u32,
            y: self.y0 as u32,
            width: self.width() as u32,
            height: self.height() as u32,
        }
    }

    /// Whether the two share any area. Boxes that only touch do not.
    fn overlaps(self, other: Self) -> bool {
        self.x0 < other.x1 && other.x0 < self.x1 && self.y0 < other.y1 && other.y0 < self.y1
    }

    fn contains(self, other: Self) -> bool {
        self.x0 <= other.x0 && self.y0 <= other.y0 && self.x1 >= other.x1 && self.y1 >= other.y1
    }

    /// Empty space between the two along `x`, zero when they overlap or touch.
    fn gap_x(self, other: Self) -> i64 {
        0.max(other.x0 - self.x1).max(self.x0 - other.x1)
    }

    fn gap_y(self, other: Self) -> i64 {
        0.max(other.y0 - self.y1).max(self.y0 - other.y1)
    }

    fn centre(self) -> (f64, f64) {
        (
            (self.x0 + self.x1) as f64 / 2.0,
            (self.y0 + self.y1) as f64 / 2.0,
        )
    }

    /// Where `edge` of this box lies on a line of the given direction.
    fn position(self, line: Direction, edge: AlignedEdge) -> f64 {
        let (start, end) = match line {
            Direction::Vertical => (self.x0, self.x1),
            Direction::Horizontal => (self.y0, self.y1),
        };
        match edge {
            AlignedEdge::Start => start as f64,
            AlignedEdge::Centre => (start + end) as f64 / 2.0,
            AlignedEdge::End => end as f64,
        }
    }
}

/// Where a box starts or ends along a run.
type Extent = fn(Rect) -> i64;

/// Regions that belong together, before they are described.
#[derive(Debug, Clone)]
struct Group {
    /// Ascending.
    region_ids: Vec<usize>,
    rect: Rect,
    area: u64,
    /// Area-weighted mean of the regions' centroids.
    centroid: (f64, f64),
    /// Set for a line of lettering.
    line_id: Option<usize>,
}

/// What the relationships are computed from: a component's box, size and kind.
#[derive(Debug, Clone, Copy)]
struct Part {
    rect: Rect,
    area: u64,
    lettering: bool,
}

/// Everything about how the parts sit, as geometry alone.
#[derive(Debug, Default)]
struct Layout {
    relationships: Vec<Relationship>,
    alignments: Vec<AlignedGroup>,
    spacings: Vec<Spacing>,
    repeats: Vec<Repeat>,
}

/// Group the regions of an image into components and describe how they sit.
pub(super) fn measure(input: &Input, typography: &Typography) -> Composition {
    let regions = input.regions;
    if regions.len() < 2 || regions.len() > MAX_REGIONS {
        return Composition::default();
    }
    let tolerance = tolerance(input.width, input.height);

    let lines: Vec<(usize, Vec<usize>)> = typography
        .lines
        .iter()
        .map(|line| (line.id, line.region_ids.clone()))
        .collect();
    let groups = group_regions(regions, &lines, tolerance);
    if groups.len() < 2
        || groups.len() > MAX_COMPONENTS
        || groups
            .iter()
            .any(|g| g.line_id.is_none() && g.region_ids.len() > MAX_FRAGMENTS)
    {
        return Composition::default();
    }

    let parts: Vec<Part> = groups
        .iter()
        .map(|group| Part {
            rect: group.rect,
            area: group.area,
            lettering: group.line_id.is_some(),
        })
        .collect();
    let layout = layout(&parts, tolerance);

    // The largest component that is not lettering is what the others are
    // measured against. Lettering is never primary: a wordmark larger than the
    // symbol beside it does not make the symbol decorative.
    let largest = parts
        .iter()
        .filter(|p| !p.lettering)
        .map(|p| p.area)
        .max()
        .unwrap_or(0);
    let foreground: u64 = regions.iter().map(|r| r.area).sum();
    let by_id: BTreeMap<usize, &Region> = regions.iter().map(|r| (r.id, r)).collect();

    let components: Vec<Component> = groups
        .iter()
        .enumerate()
        .map(|(id, group)| {
            let encloses = parts
                .iter()
                .enumerate()
                .any(|(other, p)| other != id && group.rect.contains(p.rect));
            describe(input, &by_id, id, group, largest, foreground, encloses)
        })
        .collect();

    Composition {
        component_count: components.len(),
        components,
        relationships: layout.relationships,
        alignments: layout.alignments,
        spacings: layout.spacings,
        repeats: layout.repeats,
    }
}

/// One tolerance for every "the same" in the module. See
/// [`TOLERANCE_FRACTION`].
fn tolerance(width: usize, height: usize) -> f64 {
    MIN_TOLERANCE.max(TOLERANCE_FRACTION * width.max(height) as f64)
}

/// Gather regions into groups: each line of lettering is one, and the other
/// regions merge when their boxes are disjoint but closer than `tolerance`, which
/// is what the pieces of one anti-aliased shape look like. Boxes that overlap
/// are never merged: a ring and the disc in its hole are two components, one
/// containing the other, and that is a relationship worth reporting, not
/// hiding. Ordered largest area first, then by position, then by lowest region
/// id, so the numbering depends on the artwork and not on the order regions
/// arrived in.
fn group_regions(regions: &[Region], lines: &[(usize, Vec<usize>)], tolerance: f64) -> Vec<Group> {
    let by_id: BTreeMap<usize, &Region> = regions.iter().map(|r| (r.id, r)).collect();
    let lettering: BTreeSet<usize> = lines
        .iter()
        .flat_map(|(_, ids)| ids.iter().copied())
        .collect();

    let mut groups: Vec<Group> = Vec::new();
    for (line_id, ids) in lines {
        let members: Vec<&Region> = ids.iter().filter_map(|id| by_id.get(id).copied()).collect();
        if !members.is_empty() {
            groups.push(group_of(&members, Some(*line_id)));
        }
    }

    // Everything else, merged by proximity. Sorted by id first so the result
    // cannot depend on the order the caller listed the regions in.
    let mut loose: Vec<&Region> = regions
        .iter()
        .filter(|r| !lettering.contains(&r.id))
        .collect();
    loose.sort_by_key(|r| r.id);
    let rects: Vec<Rect> = loose.iter().map(|r| Rect::of(&r.bounding_box)).collect();
    let mut parent: Vec<usize> = (0..loose.len()).collect();
    for i in 0..loose.len() {
        for j in i + 1..loose.len() {
            let (a, b) = (rects[i], rects[j]);
            if !a.overlaps(b) && a.gap_x(b).max(a.gap_y(b)) as f64 <= tolerance {
                let (ri, rj) = (root(&mut parent, i), root(&mut parent, j));
                // The lower index is always the root, so the result does not
                // depend on which pair was met first.
                parent[ri.max(rj)] = ri.min(rj);
            }
        }
    }
    let mut merged: BTreeMap<usize, Vec<&Region>> = BTreeMap::new();
    for (index, region) in loose.iter().enumerate() {
        let top = root(&mut parent, index);
        merged.entry(top).or_default().push(region);
    }
    for members in merged.values() {
        groups.push(group_of(members, None));
    }

    groups.sort_by(|a, b| {
        b.area
            .cmp(&a.area)
            .then(a.rect.y0.cmp(&b.rect.y0))
            .then(a.rect.x0.cmp(&b.rect.x0))
            .then(a.region_ids[0].cmp(&b.region_ids[0]))
    });
    groups
}

/// The representative of `index`'s set, flattening the path as it goes.
fn root(parent: &mut [usize], mut index: usize) -> usize {
    while parent[index] != index {
        parent[index] = parent[parent[index]];
        index = parent[index];
    }
    index
}

/// One group from its members. Never called with none.
fn group_of(members: &[&Region], line_id: Option<usize>) -> Group {
    let mut region_ids: Vec<usize> = members.iter().map(|r| r.id).collect();
    region_ids.sort_unstable();
    let rect = members
        .iter()
        .map(|r| Rect::of(&r.bounding_box))
        .reduce(Rect::union)
        .expect("a group has at least one region");
    let area: u64 = members.iter().map(|r| r.area).sum();
    let weight = area.max(1) as f64;
    let (mut x, mut y) = (0.0, 0.0);
    for region in members {
        x += region.centroid.x * region.area as f64 / weight;
        y += region.centroid.y * region.area as f64 / weight;
    }
    Group {
        region_ids,
        rect,
        area,
        centroid: (x, y),
        line_id,
    }
}

/// A component's role, from its size against the largest non-lettering one.
fn role(group: &Group, largest: u64, encloses: bool) -> Role {
    if group.line_id.is_some() {
        Role::Lettering
    } else if group.area as f64 >= PRIMARY_SHARE * largest as f64 {
        Role::Primary
    } else if (group.area as f64) < DECORATIVE_SHARE * largest as f64 && !encloses {
        Role::Decorative
    } else {
        Role::Secondary
    }
}

/// Everything reported about one group, the pixels read once.
fn describe(
    input: &Input,
    by_id: &BTreeMap<usize, &Region>,
    id: usize,
    group: &Group,
    largest: u64,
    foreground: u64,
    encloses: bool,
) -> Component {
    let members: Vec<&Region> = group.region_ids.iter().map(|rid| by_id[rid]).collect();

    // Letters are described by typography; an outline's closedness says
    // nothing about a word.
    let contour = group.line_id.is_none().then(|| {
        let hole_count: usize = group
            .region_ids
            .iter()
            .map(|&rid| input.hole_counts.get(rid).copied().unwrap_or(0) as usize)
            .sum();
        // The largest region decides whether it is a stroke: a fragment beside
        // it is a bit of the same shape, not a line.
        let main = members
            .iter()
            .max_by(|a, b| a.area.cmp(&b.area).then(b.id.cmp(&a.id)))
            .expect("a group has at least one region");
        let stroke = appearance::stroke_of(main, input.region_map, input.width);
        let kind = if hole_count > 0 {
            ContourKind::Closed
        } else if stroke.is_some() {
            ContourKind::Open
        } else {
            ContourKind::Filled
        };
        Contour {
            kind,
            hole_count,
            stroke_width: stroke.map(|s| tenth(s.width)),
        }
    });

    let looks: Vec<appearance::MemberLook> = members
        .iter()
        .map(|region| appearance::member_look(input.pixels, input.width, input.region_map, region))
        .collect();
    let look_refs: Vec<&appearance::MemberLook> = looks.iter().collect();

    Component {
        id,
        role: role(group, largest, encloses),
        region_ids: group.region_ids.clone(),
        geometry: ComponentGeometry {
            bounding_box: group.rect.to_box(),
            area: group.area,
            area_share: hundredth(group.area as f64 / foreground.max(1) as f64),
            centroid: Point {
                x: tenth(group.centroid.0),
                y: tenth(group.centroid.1),
            },
            region_count: group.region_ids.len(),
            contour,
            line_id: group.line_id,
        },
        appearance: typography::summarise(&look_refs),
    }
}

/// How the parts sit against each other, as numbers about boxes alone.
fn layout(parts: &[Part], tolerance: f64) -> Layout {
    Layout {
        relationships: relationships(parts),
        alignments: alignments(parts, tolerance),
        spacings: spacings(parts, tolerance),
        repeats: repeats(parts, tolerance),
    }
}

/// Containment, crossing, and each part's nearest neighbour on its right and
/// below it. Neighbours only, so the list grows with the parts and not with the
/// pairs of them.
fn relationships(parts: &[Part]) -> Vec<Relationship> {
    let mut found = Vec::new();
    let mut push = |from: usize, to: usize, relation: Relation, gap: i64| {
        let (a, b) = (&parts[from], &parts[to]);
        let (ax, ay) = a.rect.centre();
        let (bx, by) = b.rect.centre();
        found.push(Relationship {
            from,
            to,
            relation,
            gap: gap.max(0) as u32,
            centre_offset: Point {
                x: tenth(bx - ax),
                y: tenth(by - ay),
            },
            size_ratio: hundredth(b.area as f64 / a.area.max(1) as f64),
        });
    };

    for i in 0..parts.len() {
        for j in i + 1..parts.len() {
            let (a, b) = (parts[i].rect, parts[j].rect);
            // Largest first, so when the boxes are equal the earlier part
            // holds the later one.
            if a.contains(b) {
                push(i, j, Relation::Contains, 0);
            } else if b.contains(a) {
                push(j, i, Relation::Contains, 0);
            } else if a.overlaps(b) {
                push(i, j, Relation::Overlaps, 0);
            }
        }
    }

    for (i, part) in parts.iter().enumerate() {
        // Nearest part wholly to the right that shares some height, then the
        // same below it. Ties go to the lower id.
        let right = parts
            .iter()
            .enumerate()
            .filter(|(j, p)| {
                *j != i
                    && p.rect.x0 >= part.rect.x1
                    && part.rect.y0 < p.rect.y1
                    && p.rect.y0 < part.rect.y1
            })
            .min_by_key(|(j, p)| (p.rect.x0 - part.rect.x1, *j));
        if let Some((j, p)) = right {
            push(i, j, Relation::RightOf, p.rect.x0 - part.rect.x1);
        }
        let below = parts
            .iter()
            .enumerate()
            .filter(|(j, p)| {
                *j != i
                    && p.rect.y0 >= part.rect.y1
                    && part.rect.x0 < p.rect.x1
                    && p.rect.x0 < part.rect.x1
            })
            .min_by_key(|(j, p)| (p.rect.y0 - part.rect.y1, *j));
        if let Some((j, p)) = below {
            push(i, j, Relation::Below, p.rect.y0 - part.rect.y1);
        }
    }

    found.sort_by_key(|r| (r.from, r.to, r.relation));
    found
}

/// Runs of `(id, value)` in which no value is further than `tolerance` from the
/// run's first. Sorted by value then id first, so which run a value falls in
/// never depends on the order they were listed in.
fn cluster(mut values: Vec<(usize, f64)>, tolerance: f64) -> Vec<Vec<(usize, f64)>> {
    values.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)));
    let mut runs: Vec<Vec<(usize, f64)>> = Vec::new();
    for value in values {
        match runs.last_mut() {
            Some(run) if value.1 - run[0].1 <= tolerance => run.push(value),
            _ => runs.push(vec![value]),
        }
    }
    runs
}

fn mean(values: &[f64]) -> f64 {
    values.iter().sum::<f64>() / values.len().max(1) as f64
}

/// Parts whose left, centre or right edge (or top, centre or bottom) lie on one
/// line.
fn alignments(parts: &[Part], tolerance: f64) -> Vec<AlignedGroup> {
    let mut found = Vec::new();
    for line in [Direction::Vertical, Direction::Horizontal] {
        for edge in [AlignedEdge::Start, AlignedEdge::Centre, AlignedEdge::End] {
            let values = parts
                .iter()
                .enumerate()
                .map(|(id, p)| (id, p.rect.position(line, edge)))
                .collect();
            for run in cluster(values, tolerance) {
                if run.len() < 2 {
                    continue;
                }
                let positions: Vec<f64> = run.iter().map(|(_, v)| *v).collect();
                let mut component_ids: Vec<usize> = run.iter().map(|(id, _)| *id).collect();
                component_ids.sort_unstable();
                found.push(AlignedGroup {
                    line,
                    edge,
                    position: tenth(mean(&positions)),
                    component_ids,
                });
            }
        }
    }
    found
}

/// Rows and columns of three or more parts whose centres line up across the
/// run, and the gaps between neighbours.
fn spacings(parts: &[Part], tolerance: f64) -> Vec<Spacing> {
    let mut found = Vec::new();
    for along in [Direction::Horizontal, Direction::Vertical] {
        // Which parts share a row (or a column) is decided by the centre on the
        // line the run lies along: a row's parts share a horizontal centre
        // line, so `along` is also the line compared.
        let values = parts
            .iter()
            .enumerate()
            .map(|(id, p)| (id, p.rect.position(along, AlignedEdge::Centre)))
            .collect();
        for row in cluster(values, tolerance) {
            let mut members: Vec<usize> = row.iter().map(|(id, _)| *id).collect();
            let (start, end): (Extent, Extent) = match along {
                Direction::Horizontal => (|r| r.x0, |r| r.x1),
                Direction::Vertical => (|r| r.y0, |r| r.y1),
            };
            members.sort_by_key(|&id| (start(parts[id].rect), id));

            // A run is broken where neighbours overlap along the run: there is
            // no gap to speak of between them.
            let mut runs: Vec<Vec<usize>> = Vec::new();
            for id in members {
                match runs.last_mut() {
                    Some(run)
                        if start(parts[id].rect)
                            >= end(parts[*run.last().expect("non-empty")].rect) =>
                    {
                        run.push(id);
                    }
                    _ => runs.push(vec![id]),
                }
            }
            for run in runs.into_iter().filter(|r| r.len() >= MIN_RUN) {
                let gaps: Vec<f64> = run
                    .windows(2)
                    .map(|pair| (start(parts[pair[1]].rect) - end(parts[pair[0]].rect)) as f64)
                    .collect();
                let average = mean(&gaps);
                let deviation = (gaps.iter().map(|g| (g - average).powi(2)).sum::<f64>()
                    / gaps.len() as f64)
                    .sqrt();
                found.push(Spacing {
                    along,
                    component_ids: run,
                    gap: Gap {
                        mean: tenth(average),
                        deviation: tenth(deviation),
                    },
                    even: deviation <= tolerance,
                });
            }
        }
    }
    found
}

/// Three or more parts of one width and one height. Lettering is left out: its
/// letters are alike by being letters, and typography says so.
fn repeats(parts: &[Part], tolerance: f64) -> Vec<Repeat> {
    let widths = parts
        .iter()
        .enumerate()
        .filter(|(_, p)| !p.lettering)
        .map(|(id, p)| (id, p.rect.width() as f64))
        .collect();
    let mut found = Vec::new();
    for same_width in cluster(widths, tolerance) {
        let heights = same_width
            .iter()
            .map(|(id, _)| (*id, parts[*id].rect.height() as f64))
            .collect();
        for same_size in cluster(heights, tolerance) {
            if same_size.len() < MIN_RUN {
                continue;
            }
            let mut component_ids: Vec<usize> = same_size.iter().map(|(id, _)| *id).collect();
            component_ids.sort_unstable();
            let width: Vec<f64> = component_ids
                .iter()
                .map(|&id| parts[id].rect.width() as f64)
                .collect();
            found.push(Repeat {
                width: tenth(mean(&width)),
                height: tenth(mean(&same_size.iter().map(|(_, h)| *h).collect::<Vec<_>>())),
                component_ids,
            });
        }
    }
    found.sort_by(|a, b| a.component_ids.cmp(&b.component_ids));
    found
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

#[cfg(test)]
mod tests {
    use std::path::Path;

    use image::{Rgba, RgbaImage};

    use super::*;
    use crate::analysis::{Centroid, analyze};

    /// A region of a given box, filled solid: all the grouping reads.
    fn region(id: usize, x: u32, y: u32, w: u32, h: u32) -> Region {
        Region {
            id,
            bounding_box: BoundingBox {
                x,
                y,
                width: w,
                height: h,
            },
            area: u64::from(w) * u64::from(h),
            centroid: Centroid {
                x: f64::from(x) + f64::from(w) / 2.0,
                y: f64::from(y) + f64::from(h) / 2.0,
            },
        }
    }

    fn part(x: u32, y: u32, w: u32, h: u32) -> Part {
        Part {
            rect: Rect::of(&region(0, x, y, w, h).bounding_box),
            area: u64::from(w) * u64::from(h),
            lettering: false,
        }
    }

    fn groups(regions: &[Region]) -> Vec<Group> {
        group_regions(regions, &[], MIN_TOLERANCE)
    }

    #[test]
    fn fragments_closer_than_the_tolerance_are_one_component() {
        let found = groups(&[region(0, 10, 10, 20, 20), region(1, 31, 10, 20, 20)]);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].region_ids, vec![0, 1]);
    }

    #[test]
    fn shapes_further_apart_than_the_tolerance_are_two_components() {
        // A gap of three pixels, one past a tolerance of two.
        let found = groups(&[region(0, 10, 10, 20, 20), region(1, 33, 10, 20, 20)]);
        assert_eq!(found.len(), 2);
    }

    #[test]
    fn overlapping_boxes_are_never_merged() {
        let found = groups(&[region(0, 10, 10, 30, 30), region(1, 20, 20, 30, 30)]);
        assert_eq!(found.len(), 2);
    }

    #[test]
    fn a_disc_in_a_rings_hole_is_a_component_the_ring_contains() {
        let ring = region(0, 10, 10, 40, 40);
        let disc = region(1, 22, 22, 16, 16);
        let found = groups(&[ring, disc]);
        assert_eq!(found.len(), 2);
        let parts: Vec<Part> = found
            .iter()
            .map(|g| Part {
                rect: g.rect,
                area: g.area,
                lettering: false,
            })
            .collect();
        let relations = relationships(&parts);
        assert_eq!(relations.len(), 1);
        assert_eq!(relations[0].relation, Relation::Contains);
        assert_eq!((relations[0].from, relations[0].to), (0, 1));
        assert_eq!(relations[0].gap, 0);
        assert_eq!(relations[0].size_ratio, 0.16);
    }

    #[test]
    fn a_line_of_lettering_is_one_component_and_its_letters_are_not_merged_again() {
        let regions = vec![
            region(0, 10, 10, 6, 10),
            region(1, 18, 10, 6, 10),
            region(2, 26, 10, 6, 10),
            region(3, 10, 40, 20, 20),
        ];
        let found = group_regions(&regions, &[(0, vec![0, 1, 2])], MIN_TOLERANCE);
        assert_eq!(found.len(), 2);
        let lettering: Vec<&Group> = found.iter().filter(|g| g.line_id.is_some()).collect();
        assert_eq!(lettering.len(), 1);
        assert_eq!(lettering[0].region_ids, vec![0, 1, 2]);
        assert_eq!(
            lettering[0].rect,
            Rect {
                x0: 10,
                y0: 10,
                x1: 32,
                y1: 20
            }
        );
    }

    #[test]
    fn components_are_numbered_by_size_and_not_by_input_order() {
        let regions = vec![
            region(0, 10, 10, 5, 5),
            region(1, 30, 10, 30, 30),
            region(2, 70, 10, 10, 10),
        ];
        let forward = groups(&regions);
        let mut reversed_input = regions.clone();
        reversed_input.reverse();
        let backward = groups(&reversed_input);
        let ids = |g: &[Group]| g.iter().map(|g| g.region_ids.clone()).collect::<Vec<_>>();
        assert_eq!(ids(&forward), vec![vec![1], vec![2], vec![0]]);
        assert_eq!(ids(&forward), ids(&backward));
    }

    #[test]
    fn a_component_near_the_largest_in_size_is_primary_and_a_dot_beside_it_is_decorative() {
        let big = Group {
            region_ids: vec![0],
            rect: Rect {
                x0: 0,
                y0: 0,
                x1: 10,
                y1: 10,
            },
            area: 1000,
            centroid: (0.0, 0.0),
            line_id: None,
        };
        let with_area = |area: u64| Group {
            area,
            ..big.clone()
        };
        assert_eq!(role(&big, 1000, false), Role::Primary);
        assert_eq!(role(&with_area(500), 1000, false), Role::Primary);
        assert_eq!(role(&with_area(499), 1000, false), Role::Secondary);
        assert_eq!(role(&with_area(50), 1000, false), Role::Secondary);
        assert_eq!(role(&with_area(49), 1000, false), Role::Decorative);
    }

    #[test]
    fn a_small_component_that_holds_another_is_not_decorative() {
        let small = Group {
            region_ids: vec![0],
            rect: Rect {
                x0: 0,
                y0: 0,
                x1: 4,
                y1: 4,
            },
            area: 10,
            centroid: (0.0, 0.0),
            line_id: None,
        };
        assert_eq!(role(&small, 1000, false), Role::Decorative);
        assert_eq!(role(&small, 1000, true), Role::Secondary);
    }

    #[test]
    fn lettering_is_always_lettering_however_large() {
        let line = Group {
            region_ids: vec![0],
            rect: Rect {
                x0: 0,
                y0: 0,
                x1: 10,
                y1: 10,
            },
            area: 5000,
            centroid: (0.0, 0.0),
            line_id: Some(0),
        };
        assert_eq!(role(&line, 1000, false), Role::Lettering);
    }

    #[test]
    fn overlapping_boxes_relate_as_overlaps_and_not_as_contains() {
        let relations = relationships(&[part(10, 10, 30, 30), part(30, 30, 30, 30)]);
        assert_eq!(relations.len(), 1);
        assert_eq!(relations[0].relation, Relation::Overlaps);
    }

    #[test]
    fn a_part_is_related_to_its_nearest_neighbour_on_the_right_and_not_to_one_behind_it() {
        // Three in a row: the first is related to the second, not the third.
        let relations = relationships(&[
            part(10, 10, 10, 10),
            part(30, 10, 10, 10),
            part(60, 10, 10, 10),
        ]);
        let right: Vec<(usize, usize, u32)> = relations
            .iter()
            .filter(|r| r.relation == Relation::RightOf)
            .map(|r| (r.from, r.to, r.gap))
            .collect();
        assert_eq!(right, vec![(0, 1, 10), (1, 2, 20)]);
    }

    #[test]
    fn a_part_diagonal_to_another_shares_no_neighbour_relation() {
        let relations = relationships(&[part(10, 10, 10, 10), part(40, 40, 10, 10)]);
        assert!(relations.is_empty());
    }

    #[test]
    fn a_part_below_another_reports_the_gap_and_the_offset_across() {
        let relations = relationships(&[part(10, 10, 20, 10), part(14, 30, 20, 10)]);
        assert_eq!(relations.len(), 1);
        let below = &relations[0];
        assert_eq!(below.relation, Relation::Below);
        assert_eq!(below.gap, 10);
        assert_eq!((below.centre_offset.x, below.centre_offset.y), (4.0, 20.0));
    }

    #[test]
    fn parts_sharing_a_centre_line_are_an_aligned_group() {
        // A symbol and a wordmark centred under it: widths differ, centres agree.
        let found = alignments(&[part(20, 10, 40, 40), part(10, 70, 60, 10)], 2.0);
        let centred: Vec<_> = found
            .iter()
            .filter(|g| g.line == Direction::Vertical && g.edge == AlignedEdge::Centre)
            .collect();
        assert_eq!(centred.len(), 1);
        assert_eq!(centred[0].component_ids, vec![0, 1]);
        assert_eq!(centred[0].position, 40.0);
    }

    #[test]
    fn a_centre_just_past_the_tolerance_is_not_aligned() {
        let near = alignments(&[part(10, 10, 20, 10), part(12, 30, 20, 10)], 2.0);
        assert!(
            near.iter()
                .any(|g| g.edge == AlignedEdge::Centre && g.line == Direction::Vertical)
        );
        let far = alignments(&[part(10, 10, 20, 10), part(13, 30, 20, 10)], 2.0);
        assert!(
            !far.iter()
                .any(|g| g.edge == AlignedEdge::Centre && g.line == Direction::Vertical)
        );
    }

    #[test]
    fn four_evenly_spaced_squares_are_an_even_row() {
        let squares: Vec<Part> = (0..4).map(|i| part(10 + i * 20, 20, 10, 10)).collect();
        let found = spacings(&squares, 2.0);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].along, Direction::Horizontal);
        assert_eq!(found[0].component_ids, vec![0, 1, 2, 3]);
        assert_eq!(found[0].gap.mean, 10.0);
        assert!(found[0].even);
    }

    #[test]
    fn unevenly_spaced_squares_are_a_row_that_is_not_even() {
        let xs = [10, 30, 60, 80];
        let squares: Vec<Part> = xs.iter().map(|&x| part(x, 20, 10, 10)).collect();
        let found = spacings(&squares, 2.0);
        assert_eq!(found.len(), 1);
        assert!(!found[0].even);
    }

    #[test]
    fn two_parts_are_a_pair_and_not_a_row() {
        assert!(spacings(&[part(10, 20, 10, 10), part(40, 20, 10, 10)], 2.0).is_empty());
    }

    #[test]
    fn a_column_is_found_the_same_way_as_a_row() {
        let squares: Vec<Part> = (0..3).map(|i| part(20, 10 + i * 20, 10, 10)).collect();
        let found = spacings(&squares, 2.0);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].along, Direction::Vertical);
    }

    #[test]
    fn parts_that_overlap_along_a_run_are_not_a_spaced_run() {
        let found = spacings(
            &[
                part(10, 20, 20, 10),
                part(20, 20, 20, 10),
                part(50, 20, 20, 10),
            ],
            2.0,
        );
        assert!(found.is_empty());
    }

    #[test]
    fn three_parts_of_one_size_are_a_repeat_and_two_are_not() {
        let three = repeats(
            &[
                part(10, 10, 10, 10),
                part(40, 30, 10, 10),
                part(70, 50, 11, 9),
            ],
            2.0,
        );
        assert_eq!(three.len(), 1);
        assert_eq!(three[0].component_ids, vec![0, 1, 2]);
        let two = repeats(&[part(10, 10, 10, 10), part(40, 30, 10, 10)], 2.0);
        assert!(two.is_empty());
    }

    #[test]
    fn parts_of_different_sizes_are_not_a_repeat() {
        let found = repeats(
            &[
                part(10, 10, 10, 10),
                part(40, 30, 20, 20),
                part(70, 50, 30, 30),
            ],
            2.0,
        );
        assert!(found.is_empty());
    }

    #[test]
    fn lettering_is_not_a_repeat() {
        let mut parts = vec![
            part(10, 10, 10, 10),
            part(40, 30, 10, 10),
            part(70, 50, 10, 10),
        ];
        parts[2].lettering = true;
        assert!(repeats(&parts, 2.0).is_empty());
    }

    #[test]
    fn a_composition_of_one_component_is_empty() {
        let composition = Composition {
            component_count: 1,
            ..Composition::default()
        };
        assert!(composition.is_empty());
        assert!(Composition::default().is_empty());
    }

    // --- end to end, over real pixels ---

    const SIZE: u32 = 96;

    fn encode(img: &RgbaImage) -> Vec<u8> {
        let mut bytes = Vec::new();
        img.write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )
        .expect("encoding a freshly-built test image never fails");
        bytes
    }

    fn fill(img: &mut RgbaImage, x0: u32, y0: u32, x1: u32, y1: u32, colour: [u8; 4]) {
        for y in y0..y1 {
            for x in x0..x1 {
                img.put_pixel(x, y, Rgba(colour));
            }
        }
    }

    fn analyse(img: &RgbaImage) -> crate::analysis::Analysis {
        analyze(Path::new("test.png"), &encode(img)).unwrap()
    }

    const INK: [u8; 4] = [20, 20, 20, 255];

    #[test]
    fn a_ring_a_bar_and_a_block_are_closed_open_and_filled() {
        let mut img = RgbaImage::new(SIZE, SIZE);
        // A square frame, a long thin bar and a solid block, far apart.
        fill(&mut img, 8, 8, 38, 38, INK);
        fill(&mut img, 14, 14, 32, 32, [0, 0, 0, 0]);
        fill(&mut img, 8, 60, 88, 64, INK);
        fill(&mut img, 60, 10, 88, 38, INK);

        let analysis = analyse(&img);
        let kinds: BTreeMap<(u32, u32), ContourKind> = analysis
            .composition
            .components
            .iter()
            .map(|c| {
                let b = c.geometry.bounding_box;
                ((b.x, b.y), c.geometry.contour.as_ref().unwrap().kind)
            })
            .collect();
        assert_eq!(kinds[&(8, 8)], ContourKind::Closed);
        assert_eq!(kinds[&(8, 60)], ContourKind::Open);
        assert_eq!(kinds[&(60, 10)], ContourKind::Filled);
    }

    #[test]
    fn the_holes_of_a_component_are_counted_exactly() {
        let mut img = RgbaImage::new(SIZE, SIZE);
        // One block with two cut-outs, and a second block to make a composition.
        fill(&mut img, 8, 8, 60, 40, INK);
        fill(&mut img, 14, 14, 24, 34, [0, 0, 0, 0]);
        fill(&mut img, 34, 14, 44, 34, [0, 0, 0, 0]);
        fill(&mut img, 70, 60, 88, 88, INK);

        let analysis = analyse(&img);
        let block = &analysis.composition.components[0];
        assert_eq!(block.geometry.contour.as_ref().unwrap().hole_count, 2);
    }

    #[test]
    fn a_single_shape_has_no_composition() {
        let mut img = RgbaImage::new(SIZE, SIZE);
        fill(&mut img, 20, 20, 70, 70, INK);
        let analysis = analyse(&img);
        assert!(analysis.composition.is_empty());
        let json = serde_json::to_value(&analysis).unwrap();
        assert!(json.get("composition").is_none());
    }

    #[test]
    fn recolouring_changes_the_appearance_of_a_composition_and_not_its_geometry() {
        let draw = |a: [u8; 4], b: [u8; 4]| {
            let mut img = RgbaImage::new(SIZE, SIZE);
            fill(&mut img, 10, 10, 40, 40, a);
            fill(&mut img, 56, 20, 86, 50, b);
            fill(&mut img, 10, 60, 30, 70, b);
            serde_json::to_value(analyse(&img).composition).unwrap()
        };
        let one = draw(INK, [200, 30, 30, 255]);
        let two = draw([30, 40, 210, 255], [20, 160, 60, 255]);

        assert_ne!(
            one["components"][0]["appearance"],
            two["components"][0]["appearance"]
        );
        for key in ["relationships", "alignments", "spacings", "repeats"] {
            assert_eq!(one[key], two[key], "{key} depends on colour");
        }
        let geometry = |v: &serde_json::Value| {
            v["components"]
                .as_array()
                .unwrap()
                .iter()
                .map(|c| c["geometry"].clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(geometry(&one), geometry(&two));
    }

    #[test]
    fn analysing_the_same_composition_twice_gives_identical_output() {
        let mut img = RgbaImage::new(SIZE, SIZE);
        fill(&mut img, 10, 10, 40, 40, INK);
        fill(&mut img, 56, 10, 86, 40, INK);
        fill(&mut img, 10, 60, 40, 90, INK);
        let first = serde_json::to_value(analyse(&img).composition).unwrap();
        let second = serde_json::to_value(analyse(&img).composition).unwrap();
        assert_eq!(first, second);
        assert!(first["component_count"].as_u64().unwrap() >= 3);
    }

    #[test]
    fn a_heap_of_specks_lying_close_is_a_texture_and_not_a_component() {
        // Specks a pixel apart merge into one lump, beside a block. Nine
        // pieces is one past the limit, so nothing is reported...
        let draw = |specks: u32| {
            let mut img = RgbaImage::new(SIZE, SIZE);
            fill(&mut img, 10, 50, 40, 85, INK);
            for i in 0..specks {
                let (x, y) = (50 + (i % 3) * 5, 10 + (i / 3) * 5);
                fill(&mut img, x, y, x + 4, y + 4, INK);
            }
            analyse(&img)
        };
        assert!(draw(9).composition.is_empty());
        // ...and with eight, the control either side of the limit, it is a
        // component of eight pieces like any other.
        let eight = draw(8);
        assert_eq!(eight.composition.component_count, 2);
        assert!(
            eight
                .composition
                .components
                .iter()
                .any(|c| c.geometry.region_count == 8)
        );
    }

    #[test]
    fn more_regions_than_a_composition_can_hold_report_nothing() {
        // A grid of 7x7 small squares: 49 components, a texture.
        let mut img = RgbaImage::new(SIZE, SIZE);
        for gy in 0..7 {
            for gx in 0..7 {
                let (x, y) = (6 + gx * 12, 6 + gy * 12);
                fill(&mut img, x, y, x + 6, y + 6, INK);
            }
        }
        assert!(analyse(&img).composition.is_empty());
    }
}
