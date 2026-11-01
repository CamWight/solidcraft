//! A drawing's dimensions (W19b, #4896): each the distance or angle between named parts of its
//! view's model, measured as the view shows it ([`solidcraft_geom::drawing_measure`]). A dimension
//! saves what it measures by name, never a picked point, so when the model changes it follows the
//! model or says what it lost: it never moves to other geometry. Its value and status come from
//! the model each time. The one thing worked out that is saved is its last good value, what it last
//! read, for when it can't be measured any more: nothing reads that as its value.
//!
//! - **Linear**: between two points ([`ModelPoint`]): corners, or the centres of circles and arcs
//!   (a hole's), horizontal, vertical or aligned as the view shows them, or along a direction on
//!   the view.
//! - **Length**: a straight edge's ends ([`ModelEdge`]).
//! - **Angle**: between two straight edges, the smaller angle between their lines, or on its other
//!   side π minus it, whichever way the kernel runs the edges.
//! - **Radial** (W20a): a circle's or an arc's radius, or its diameter, on a view that sees it face
//!   on, read exactly from the body.
//! - **Chain** (W20c): from each of its points to the next along one line, as the view shows them,
//!   the members adding up to the overall from the first point to the last.
//! - **Cylinder** (W20b): a whole cylinder's diameter between its two silhouettes, on a view that
//!   doesn't look along its axis (a face, [`ModelFace`]).
//! - **Silhouettes** (W20b): between a silhouette of one cylinder and a silhouette of another, each
//!   named by its face and by whether it is the one nearer the other cylinder or the one farther,
//!   as the view shows them ([`Silhouette`]). Two silhouettes of one cylinder are its diameter.
//! - **Faces**: between two flat, parallel faces the view sees edge-on, which a fillet that takes
//!   their corners leaves measured.
//!
//! What it measures, how that is named and how it is found again, is [`super::reference`].
//! The dimension line sits `offset_mm` of paper from what it measures, and the text at a place on
//! paper from its view's centre, so it moves with the view and not with the model; or centred on
//! the dimension line.
//!
//! The rules, beside the record's and its references': a dimension is on one of the drawing's views;
//! a chain has 3 to [`MAX_CHAIN_POINTS`] points and shows its members' numbers, never a text instead;
//! a true length is measured aligned; a
//! direction to measure along isn't zero, and is at most [`MAX_REAL`] each way, as every number a
//! drawing saves (a whole number past it reads, then saves as one the file's check refuses); the
//! offset and the text place are on paper ([`is_paper_coordinate`]); at most [`MAX_DECIMALS`]
//! decimals; an override text of 1 to [`MAX_TEXT_BYTES`] bytes; a last good value from 0 to
//! [`MAX_REAL`]; at most [`MAX_DIMENSIONS`] dimensions, listed by id.
//!
//! [`Document::resolve_dimensions`] finds what a dimension measures again, strictly, with the
//! silhouettes it measures on a cylinder's faces. What it can't decide it says ([`Status`]), and it
//! never chooses.

use std::collections::BTreeMap;
use std::f64::consts::PI;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use solidcraft_geom::drawing_measure::{self, AngleFrom, LinearDirection, Measured, Reading, SilhouetteSide};
use solidcraft_geom::drawing_view::{Invalid, is_paper_coordinate};
use solidcraft_geom::paper::MAX_SIDE_MM;
use solidcraft_geom::{Plane, Vec3};

use super::reference::{
    Lookup, ModelEdge, ModelFace, ModelPoint, OnPlane, Status, check_edge, check_face, check_point, edge_candidates, is_false, same_cylinder,
    straight,
};
use super::{Drawing, Units, add_by_id, in_id_order, require};
use crate::display_units::length_number;
use crate::format::MAX_REAL;

use crate::{DocError, Document, Model, Result, ids};

/// What a reading's number is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LabelKind {
    Length,
    Angle,
    Radius,
    Diameter,
}

/// What a reading says: `value`, in model millimetres or radians for an angle, in `units` with
/// `decimals`. The one rule for a dimension's text and a view's measurements.
pub fn reading_label(kind: LabelKind, value: f64, units: &str, decimals: u8) -> String {
    let length = || length_number(value, units, decimals);
    match kind {
        LabelKind::Length => length(),
        LabelKind::Angle => format!("{:.*}°", usize::from(decimals), value.to_degrees()),
        LabelKind::Radius => format!("R{}", length()),
        LabelKind::Diameter => format!("{DIAMETER_SIGN}{}", length()),
    }
}

/// The most dimensions a drawing may have.
pub const MAX_DIMENSIONS: usize = 10_000;
/// The most decimals a dimension shows.
pub const MAX_DECIMALS: u8 = 6;
/// The longest text shown instead of a dimension's number, in bytes.
pub const MAX_TEXT_BYTES: usize = 1024;
/// The most points a chain of dimensions runs through.
pub const MAX_CHAIN_POINTS: usize = 64;
/// What a diameter's number follows: Ø (U+00D8), as tolerance texts write it. The diameter sign
/// U+2300 isn't in the bundled fonts, so it would print as a missing glyph.
const DIAMETER_SIGN: &str = "Ø";

/// One dimension of a drawing.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Dimension {
    #[serde(with = "crate::ids")]
    pub id: u64,
    /// The view it measures in and moves with.
    #[serde(with = "crate::ids")]
    pub view: u64,
    pub kind: DimensionKind,
    /// How far the dimension line is from what it measures, in paper millimetres; the sign says on
    /// which side.
    pub offset_mm: f64,
    pub text: TextPlace,
    /// 0 to [`MAX_DECIMALS`].
    pub decimals: u8,
    /// Shown instead of the number, which is still worked out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub override_text: Option<String>,
    /// What it last read (model millimetres, or radians for an angle), as of the last save.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_good: Option<f64>,
}

/// What a dimension measures.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum DimensionKind {
    /// The distance between two corners.
    Linear { a: ModelPoint, b: ModelPoint, direction: LinearDirection, reading: Reading },
    /// A straight edge's length: the distance between its ends.
    Length { edge: ModelEdge, direction: LinearDirection, reading: Reading },
    /// The angle between two straight edges' lines: the smaller one, or with `other_side` π minus it.
    Angle {
        a: ModelEdge,
        b: ModelEdge,
        reading: Reading,
        #[serde(default, skip_serializing_if = "is_false")]
        other_side: bool,
    },
    /// A circle's or an arc's radius, or with `diameter` its diameter, on a view that sees it face
    /// on.
    Radial {
        edge: ModelEdge,
        #[serde(default, skip_serializing_if = "is_false")]
        diameter: bool,
    },
    /// The distances from each point to the next, in order along one line as the view shows them.
    Chain { points: Vec<ModelPoint>, direction: LinearDirection },
    /// A whole cylinder's diameter between its two silhouettes, on a view that doesn't look along
    /// its axis: the cylinder a face of it lies on.
    Cylinder { face: ModelFace },
    /// The distance between a silhouette of one cylinder and a silhouette of another, as the view
    /// shows them.
    Silhouettes { a: Silhouette, b: Silhouette, direction: LinearDirection },
    /// The distance between two flat, parallel faces, on a view that sees both edge-on.
    Faces { a: ModelFace, b: ModelFace, direction: LinearDirection },
}

/// One of a cylinder's two silhouettes on a view: the cylinder by a face of it, and the side by
/// where the other cylinder of its dimension is.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Silhouette {
    pub face: ModelFace,
    pub side: SilhouetteSide,
}

/// Where a dimension's text sits.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum TextPlace {
    /// At this point of paper, in millimetres from its view's centre: it stays there when the
    /// model changes.
    At([f64; 2]),
    /// On the middle of the dimension line, wherever that is.
    Centred,
}

/// Whether a dimension's record may keep `value` as its last good one.
fn is_last_good(value: f64) -> bool {
    (0.0..=MAX_REAL).contains(&value)
}

impl Drawing {
    /// Adds `dimensions`, each where its id puts it, as [`Drawing::add_views`] adds views.
    pub fn add_dimensions(&mut self, dimensions: impl IntoIterator<Item = Arc<Dimension>>) {
        add_by_id(&mut self.dimensions, dimensions, |d| d.id);
    }

    /// Dimension `id`, found by halves in the dimensions' id order.
    pub fn dimension(&self, id: u64) -> Option<&Dimension> {
        self.dimension_index(id).and_then(|i| self.dimensions.get(i)).map(|d| &**d)
    }

    /// Where dimension `id` is in [`Drawing::dimensions`].
    pub fn dimension_index(&self, id: u64) -> Option<usize> {
        self.dimensions.binary_search_by_key(&id, |d| d.id).ok()
    }

    /// The dimensions' rules (the module's notes). The views are checked first, so they are in id
    /// order here.
    pub(super) fn check_dimensions(&self) -> Result<()> {
        require(self.dimensions.len() <= MAX_DIMENSIONS, || format!("a drawing has at most {MAX_DIMENSIONS} dimensions"))?;
        require(in_id_order(&self.dimensions, |d| d.id), || "a drawing lists its dimensions by id".into())?;
        for dimension in &self.dimensions {
            self.check_dimension(dimension).map_err(|e| DocError::Invalid(format!("dimension {}: {e}", ids::encode(dimension.id))))?;
        }
        Ok(())
    }

    fn check_dimension(&self, d: &Dimension) -> Result<()> {
        let on = || format!("it is on view {}, which the drawing doesn't have", ids::encode(d.view));
        require(self.view(d.view).is_some(), on)?;
        match &d.kind {
            DimensionKind::Linear { a, b, direction, reading } => {
                check_point(a)?;
                check_point(b)?;
                check_direction(*direction, *reading)?;
            }
            DimensionKind::Length { edge, direction, reading } => {
                check_edge(edge)?;
                check_direction(*direction, *reading)?;
            }
            DimensionKind::Angle { a, b, .. } => {
                check_edge(a)?;
                check_edge(b)?;
            }
            DimensionKind::Radial { edge, .. } => check_edge(edge)?,
            DimensionKind::Chain { points, direction } => {
                check_chain_length(points.len())?;
                points.iter().try_for_each(check_point)?;
                check_direction(*direction, Reading::Projected)?;
                let instead = || "a chain shows its members' numbers, never a text instead".into();
                require(d.override_text.is_none(), instead)?;
            }
            DimensionKind::Cylinder { face } => check_face(face)?,
            DimensionKind::Silhouettes { a, b, direction } => {
                check_face(&a.face)?;
                check_face(&b.face)?;
                check_direction(*direction, Reading::Projected)?;
            }
            DimensionKind::Faces { a, b, direction } => {
                check_face(a)?;
                check_face(b)?;
                check_direction(*direction, Reading::Projected)?;
            }
        }
        let off_paper = || format!("its offset and its text's place are at most {MAX_SIDE_MM} mm on paper");
        let text = match d.text {
            TextPlace::At([x, y]) => is_paper_coordinate(x) && is_paper_coordinate(y),
            TextPlace::Centred => true,
        };
        require(is_paper_coordinate(d.offset_mm) && text, off_paper)?;
        require(d.decimals <= MAX_DECIMALS, || format!("it shows at most {MAX_DECIMALS} decimals"))?;
        require(d.last_good.is_none_or(is_last_good), || format!("its last good value is from 0 to {MAX_REAL}"))?;
        match &d.override_text {
            Some(text) => require(!text.is_empty() && text.len() <= MAX_TEXT_BYTES, || {
                format!("the text shown instead of its number is 1 to {MAX_TEXT_BYTES} bytes")
            }),
            None => Ok(()),
        }
    }
}

/// A chain's number of points: 3 (2 make a linear dimension) to [`MAX_CHAIN_POINTS`].
pub fn check_chain_length(points: usize) -> Result<()> {
    require((3..=MAX_CHAIN_POINTS).contains(&points), || format!("a chain has 3 to {MAX_CHAIN_POINTS} points"))
}

/// A linear dimension's direction and reading: a true length is the segment's own, so it is
/// measured aligned (as [`solidcraft_geom::drawing_measure::linear`] reads it), and a direction to
/// measure along isn't zero.
fn check_direction(direction: LinearDirection, reading: Reading) -> Result<()> {
    if let LinearDirection::Along(along) = direction {
        require(along.normalized().is_some(), || "the direction to measure along is zero or not finite".into())?;
        require(along.x.abs() <= MAX_REAL && along.y.abs() <= MAX_REAL, || format!("the direction to measure along is at most {MAX_REAL} each way"))?;
    }
    require(reading == Reading::Projected || direction == LinearDirection::Aligned, || "a true length is measured aligned".into())
}

/// A dimension as the model has it now ([`Document::resolve_dimensions`]): never saved.
#[derive(Clone, Debug, PartialEq)]
pub struct Resolved {
    pub status: Status,
    /// What it reads (model millimetres, or radians for an angle), and whether a measured line
    /// leaves the view's plane: whenever what it measures was found and measured, so an
    /// overridden dimension has it too.
    pub measured: Option<Measured>,
    /// What it shows: the text given instead of its number, else the number in the drawing's
    /// units with its decimals (degrees for an angle, after R or Ø for a radius or a diameter).
    /// `None` with neither, and for a chain, which shows its members'.
    pub text: Option<String>,
    /// A chain's members, from its first point to its last, each as it reads and its text: empty
    /// unless measured, and for other kinds. `measured` is then the chain's overall.
    pub members: Vec<Member>,
    /// Every name it measures was found by exact history, each as one edge.
    pub exact: bool,
    /// What it measures, in the design's frame, for its lines: unless measured, none.
    pub geometry: Option<Geometry>,
}

/// What a dimension measures, in the design's frame, each way of measuring with what its lines need.
/// A new way joins here, and whatever draws them can't compile without it.
#[derive(Clone, Debug, PartialEq)]
pub enum Geometry {
    /// Between two points, a linear dimension's or a straight edge's ends.
    Line { a: Vec3, b: Vec3, direction: LinearDirection },
    /// Between two lines, each from its first point to its second.
    Angle { a: [Vec3; 2], b: [Vec3; 2], other_side: bool },
    /// A circle or an arc: its centre and its start, middle and end.
    Arc { centre: Vec3, start: Vec3, middle: Vec3, end: Vec3, diameter: bool },
    /// From each point to the next.
    Chain { points: Vec<Vec3>, direction: LinearDirection },
    /// Between two silhouettes or two faces, each from its one end to its other: a cylinder's two
    /// run along its axis.
    Across { a: [Vec3; 2], b: [Vec3; 2], direction: LinearDirection },
}

impl Resolved {
    /// A dimension with nothing to read it from: why.
    pub fn lost(why: String) -> Resolved {
        Resolved { status: Status::Lost(why), measured: None, text: None, members: Vec::new(), exact: false, geometry: None }
    }
}

/// One of a chain's members as the model has it now.
#[derive(Clone, Debug, PartialEq)]
pub struct Member {
    pub measured: Measured,
    /// Its number in the drawing's units with the chain's decimals.
    pub text: String,
}

impl Document {
    /// What a dimension between silhouettes `a` and `b` is on `model` (this design's): when both
    /// faces lie on one cylinder of one body placed once, that cylinder's diameter, which takes no
    /// direction; else the distance between them along `direction`, aligned if none.
    pub fn silhouettes_kind(&self, model: &Model, a: Silhouette, b: Silhouette, direction: Option<LinearDirection>) -> Result<DimensionKind> {
        let mut lookup = Lookup::new(self, model);
        let one = match (lookup.on_cylinder(&a.face), lookup.on_cylinder(&b.face)) {
            (Ok(x), Ok(y)) => x.body == y.body && a.face.path == b.face.path && same_cylinder(&x.cylinder, &y.cylinder, x.tol),
            _ => false,
        };
        match (one, direction) {
            (true, Some(_)) => Err(DocError::Invalid("both silhouettes are on one cylinder: its diameter takes no direction".into())),
            (true, None) => Ok(DimensionKind::Cylinder { face: a.face }),
            (false, direction) => Ok(DimensionKind::Silhouettes { a, b, direction: direction.unwrap_or(LinearDirection::Aligned) }),
        }
    }

    /// Keeps what each dimension last read, by drawing and dimension id, in its record, as a
    /// design's file does. A dimension no longer there is passed over, and so is a value its record
    /// would refuse (a model can read past what a drawing saves), so the file still opens. A drawing
    /// with no new value stays shared.
    pub fn keep_last_good(&mut self, values: &BTreeMap<(u64, u64), f64>) {
        for drawing in &mut self.drawings {
            let id = drawing.id;
            for (&(_, dimension), &value) in values.range((id, 0)..=(id, u64::MAX)) {
                let Some(at) = drawing.dimension_index(dimension) else { continue };
                let new = is_last_good(value) && drawing.dimensions.get(at).is_some_and(|d| d.last_good != Some(value));
                if new && let Some(record) = Arc::make_mut(drawing).dimensions.get_mut(at) {
                    Arc::make_mut(record).last_good = Some(value);
                }
            }
        }
    }

    /// The radius of the circle or arc `edge` names, on `model` (this design's), as the view through
    /// `basis` sees it face on, and whether it was found exactly.
    pub fn radius_on_view(&self, model: &Model, edge: &ModelEdge, basis: &Plane) -> std::result::Result<(f64, bool), Status> {
        let (radius, _, exact) = Lookup::new(self, model).whole_arc(edge, basis)?;
        Ok((radius, exact))
    }

    /// The units `drawing` (one of this design's drawings) shows its numbers in.
    pub fn drawing_units<'a>(&'a self, drawing: &'a Drawing) -> &'a str {
        match &drawing.units {
            Units::Fixed { unit } => unit.as_str(),
            Units::FollowModel => self.units.as_str(),
        }
    }

    /// The dimensions `wanted` of `drawing` (one of this design's drawings) as `model` (this
    /// design's) has them now: what each measures found again by name, measured on its view, and
    /// the text it shows. A body is looked up and its edge names worked out once for the call,
    /// however many dimensions name it; a body none names isn't touched. `Err` only for an id the
    /// drawing doesn't have, or a damaged record.
    pub fn resolve_dimensions(&self, model: &Model, drawing: &Drawing, wanted: &[u64]) -> Result<Vec<Resolved>> {
        let units = self.drawing_units(drawing);
        let mut lookup = Lookup::new(self, model);
        wanted
            .iter()
            .map(|id| {
                let d = drawing.dimension(*id).ok_or_else(|| DocError::Unknown(format!("dimension {}", ids::encode(*id))))?;
                let basis = drawing.resolve_view(d.view)?.basis;
                Ok(lookup.resolve(d, &basis, units))
            })
            .collect()
    }
}

/// What a dimension measures, found and measured: a chain's overall, and its members.
struct Found {
    measured: Measured,
    members: Vec<Measured>,
    geometry: Geometry,
    exact: bool,
}

impl<'a> Lookup<'a> {
    fn resolve(&mut self, d: &Dimension, basis: &Plane, units: &str) -> Resolved {
        let (status, found) = match self.measure(d, basis) {
            Ok(found) => (if d.override_text.is_some() { Status::Overridden } else { Status::Ok }, Some(found)),
            Err(status) => (status, None),
        };
        let measured = found.as_ref().map(|f| f.measured);
        let length = |m: Measured| length_number(m.value, units, d.decimals);
        let number = |m: Measured| {
            let kind = match d.kind {
                DimensionKind::Angle { .. } => LabelKind::Angle,
                DimensionKind::Radial { diameter: false, .. } => LabelKind::Radius,
                DimensionKind::Radial { diameter: true, .. } | DimensionKind::Cylinder { .. } => LabelKind::Diameter,
                DimensionKind::Chain { .. } => return None,
                _ => LabelKind::Length,
            };
            Some(reading_label(kind, m.value, units, d.decimals))
        };
        let (members, geometry, exact) = found.map_or_else(Default::default, |f| (f.members, Some(f.geometry), f.exact));
        Resolved {
            status,
            text: d.override_text.clone().or_else(|| measured.and_then(number)),
            members: members.into_iter().map(|m| Member { measured: m, text: length(m) }).collect(),
            exact,
            geometry,
            measured,
        }
    }

    /// `d` found and measured on the view through `basis`, else the status that says why not.
    fn measure(&mut self, d: &Dimension, basis: &Plane) -> std::result::Result<Found, Status> {
        let unmeasurable = |e: Invalid| Status::Unmeasurable(e.0.to_string());
        match &d.kind {
            DimensionKind::Linear { a, b, direction, reading } => {
                let (a, exact_a) = self.corner(a)?;
                let (b, exact_b) = self.corner(b)?;
                let measured = drawing_measure::linear(basis, a, b, *direction, *reading).map_err(unmeasurable)?;
                Ok(Found { measured, members: Vec::new(), geometry: Geometry::Line { a, b, direction: *direction }, exact: exact_a && exact_b })
            }
            DimensionKind::Length { edge, direction, reading } => {
                let found = self.edges(edge)?;
                // A split edge's length is no longer one number.
                if !found.whole {
                    return Err(Status::Ambiguous(edge_candidates(&found)));
                }
                let ends = found.pieces.first().and_then(|(_, line)| straight(line));
                let [a, b] = ends.ok_or_else(|| Status::Unmeasurable(format!("edge `{}` isn't straight", edge.name)))?;
                let measured = drawing_measure::linear(basis, a, b, *direction, *reading).map_err(unmeasurable)?;
                Ok(Found { measured, members: Vec::new(), geometry: Geometry::Line { a, b, direction: *direction }, exact: found.exact })
            }
            DimensionKind::Angle { a, b, reading, other_side } => {
                let (a, exact_a) = self.line(a)?;
                let (b, exact_b) = self.line(b)?;
                let between = drawing_measure::angle(basis, (a[0], a[1]), AngleFrom::Edge(b[0], b[1]), *reading).map_err(unmeasurable)?;
                let value = between_lines(between.value, *other_side);
                let geometry = Geometry::Angle { a, b, other_side: *other_side };
                Ok(Found { measured: Measured { value, ..between }, members: Vec::new(), geometry, exact: exact_a && exact_b })
            }
            DimensionKind::Radial { edge, diameter } => {
                let (radius, [centre, start, middle, end], exact) = self.whole_arc(edge, basis)?;
                let value = if *diameter { 2.0 * radius } else { radius };
                let measured = Measured { value, foreshortened: false };
                let geometry = Geometry::Arc { centre, start, middle, end, diameter: *diameter };
                Ok(Found { measured, members: Vec::new(), geometry, exact })
            }
            DimensionKind::Chain { points, direction } => {
                let found = points.iter().map(|p| self.corner(p)).collect::<std::result::Result<Vec<_>, _>>()?;
                let exact = found.iter().all(|(_, exact)| *exact);
                let points: Vec<Vec3> = found.into_iter().map(|(p, _)| p).collect();
                let chain = drawing_measure::chain(basis, &points, *direction).map_err(unmeasurable)?;
                Ok(Found { measured: chain.overall, members: chain.members, geometry: Geometry::Chain { points, direction: *direction }, exact })
            }
            DimensionKind::Cylinder { face } => self.cylinder(face, basis),
            DimensionKind::Silhouettes { a, b, direction } => self.silhouettes(a, b, *direction, basis),
            DimensionKind::Faces { a, b, direction } => self.faces(a, b, *direction, basis),
        }
    }

    /// The diameter of the whole cylinder `face` lies on, between its two silhouettes on the view
    /// through `basis`: both must lie on the body's faces on that cylinder, so a fillet's quarter
    /// round is refused. Its points are the silhouettes' ends.
    fn cylinder(&mut self, face: &ModelFace, basis: &Plane) -> std::result::Result<Found, Status> {
        let on = self.on_cylinder(face)?;
        let side = drawing_measure::silhouette_side(basis, on.cylinder.axis).map_err(|e| Status::Unmeasurable(e.0.to_string()))?;
        if !(on.covers(side) && on.covers(-side)) {
            return Err(Status::Unmeasurable(format!(
                "face `{}` isn't a whole cylinder in this view: one of its silhouettes isn't on it (give an arc's radius)",
                face.name
            )));
        }
        let measured = Measured { value: 2.0 * on.cylinder.radius, foreshortened: false };
        let geometry = Geometry::Across { a: on.ends(side), b: on.ends(-side), direction: LinearDirection::Aligned };
        Ok(Found { measured, members: Vec::new(), geometry, exact: on.exact })
    }

    /// The distance between silhouettes `a` and `b` on the view through `basis`, each on its
    /// cylinder's faces. Its points are the two silhouettes' ends, `a`'s first.
    fn silhouettes(&mut self, a: &Silhouette, b: &Silhouette, direction: LinearDirection, basis: &Plane) -> std::result::Result<Found, Status> {
        let (on_a, on_b) = (self.on_cylinder(&a.face)?, self.on_cylinder(&b.face)?);
        let between = drawing_measure::between_silhouettes(basis, &on_a.cylinder, a.side, &on_b.cylinder, b.side, direction);
        let (measured, [way_a, way_b]) = between.map_err(|e| Status::Unmeasurable(e.0.to_string()))?;
        for (on, way, s) in [(&on_a, way_a, a), (&on_b, way_b, b)] {
            if !on.covers(way) {
                let side = if s.side == SilhouetteSide::Near { "near" } else { "far" };
                return Err(Status::Unmeasurable(format!("face `{}`'s {side} silhouette in this view isn't on it", s.face.name)));
            }
        }
        let geometry = Geometry::Across { a: on_a.ends(way_a), b: on_b.ends(way_b), direction };
        Ok(Found { measured, members: Vec::new(), geometry, exact: on_a.exact && on_b.exact })
    }

    /// The distance between flat faces `a` and `b` on the view through `basis`. Its points are each
    /// face's ends on the view, `a`'s first.
    fn faces(&mut self, a: &ModelFace, b: &ModelFace, direction: LinearDirection, basis: &Plane) -> std::result::Result<Found, Status> {
        let (on_a, on_b) = (self.on_plane(a)?, self.on_plane(b)?);
        let between = drawing_measure::between_faces(basis, (on_a.origin, on_a.normal), (on_b.origin, on_b.normal), direction);
        let measured = between.map_err(|e| Status::Unmeasurable(e.0.to_string()))?;
        let ends = |on: &OnPlane, face: &ModelFace| on.ends(basis).ok_or_else(|| Status::Unmeasurable(format!("face `{}` has no mesh", face.name)));
        let geometry = Geometry::Across { a: ends(&on_a, a)?, b: ends(&on_b, b)?, direction };
        Ok(Found { measured, members: Vec::new(), geometry, exact: on_a.exact && on_b.exact })
    }
}

/// The angle between two lines, from `between` (from one's direction to the other's, 0 to π): the
/// smaller one, whichever way the kernel runs each edge, or on its `other_side` π minus it.
fn between_lines(between: f64, other_side: bool) -> f64 {
    let smaller = between.min(PI - between);
    if other_side { PI - smaller } else { smaller }
}

#[cfg(test)]
#[path = "dimension_tests.rs"]
mod tests;
