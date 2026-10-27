//! Measurement on drawing views (W19a, #4896): what a dimension on a view reads, in model
//! millimetres or radians, from model points and edges and the view's basis. One rule for every
//! dimension: the dimension record (W19b) keeps what to measure, and W09 formats the number.
//!
//! A value is **projected** (as the view shows it) or **true** (as the model has it);
//! `foreshortened` says a measured line isn't parallel to the view's plane, so the two differ. The
//! view's scale and place on the sheet never change a measurement.

use serde::{Deserialize, Serialize};

use crate::drawing_view::{Invalid, PARALLEL, require};
use crate::section::{MAX_NUMBER, validate_plane};
use crate::{EPS, Plane, Vec2, Vec3};

type Result<T> = std::result::Result<T, Invalid>;

/// Which way a linear dimension measures on its view.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum LinearDirection {
    /// Along the view's horizontal (paper x).
    Horizontal,
    /// Along the view's vertical (paper y).
    Vertical,
    /// Along the segment as the view shows it.
    Aligned,
    /// Along a direction on the view, in its plane's coordinates (any length but zero): a rotated
    /// dimension, or one along a picked edge.
    Along(Vec2),
}

/// What a dimension reads: the length or angle as the view shows it, or as the model has it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reading {
    Projected,
    True,
}

/// The line an angle is measured from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AngleFrom {
    /// The view's horizontal, pointing right (paper +x).
    Horizontal,
    /// The view's vertical, pointing up (paper +y).
    Vertical,
    /// A model direction, such as the part's axis, as the view shows it.
    ModelAxis(Vec3),
    /// Another edge, from its first point to its second.
    Edge(Vec3, Vec3),
}

/// What a dimension reads, and whether a measured line is out of the view's plane.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Measured {
    /// Model millimetres, or radians for an angle.
    pub value: f64,
    /// A measured line isn't parallel to the view's plane, so its projected value differs from
    /// the model's.
    pub foreshortened: bool,
}

/// The distance from `a` to `b` (model points) that a linear dimension measuring `direction` on
/// the view through `basis` reads. A true reading is the segment's own length, so it goes with
/// [`LinearDirection::Aligned`] only: a true horizontal has no one meaning in a tilted view.
pub fn linear(basis: &Plane, a: Vec3, b: Vec3, direction: LinearDirection, reading: Reading) -> Result<Measured> {
    let d = segment(basis, a, b)?;
    let shown = Vec2::new(d.dot(basis.x), d.dot(basis.y));
    let value = match (direction, reading) {
        (LinearDirection::Aligned, Reading::True) => d.len(),
        (_, Reading::True) => return Err(Invalid("a true length is along the segment: measure it aligned")),
        (LinearDirection::Horizontal, Reading::Projected) => shown.x.abs(),
        (LinearDirection::Vertical, Reading::Projected) => shown.y.abs(),
        (LinearDirection::Aligned, Reading::Projected) => shown.len(),
        (LinearDirection::Along(along), Reading::Projected) => {
            let along = along.normalized().ok_or(Invalid("the direction to measure along is zero or not finite"))?;
            shown.dot(along).abs()
        }
    };
    require(value > EPS, "the dimension would read 0: the points line up across the direction it measures")?;
    Ok(Measured { value, foreshortened: out_of_plane(basis, d) })
}

/// The angle from the line `from` to `edge` (model points, from its first to its second) on the
/// view through `basis`, from 0 to π: each line goes the way its caller gives it (from the vertex
/// outwards), so the other side of the angle reads π minus it. A true reading is the angle
/// between the model lines, so it isn't given from the view's horizontal or vertical.
pub fn angle(basis: &Plane, edge: (Vec3, Vec3), from: AngleFrom, reading: Reading) -> Result<Measured> {
    let d = segment(basis, edge.0, edge.1)?;
    let (reference, model_line) = match from {
        AngleFrom::Horizontal => (basis.x, false),
        AngleFrom::Vertical => (basis.y, false),
        AngleFrom::ModelAxis(axis) => (axis.normalized().ok_or(Invalid("the model axis is zero or not finite"))?, true),
        AngleFrom::Edge(a, b) => (segment(basis, a, b)?, true),
    };
    let value = match reading {
        Reading::True if model_line => between(d, reference)?,
        Reading::True => return Err(Invalid("a true angle is between two model lines, not from the view's horizontal or vertical")),
        Reading::Projected => between(on_view(basis, d)?, on_view(basis, reference)?)?,
    };
    Ok(Measured { value, foreshortened: out_of_plane(basis, d) || (model_line && out_of_plane(basis, reference)) })
}

/// `b - a`, both finite model points within 1e12 mm of the origin, the segment not of zero length,
/// on a valid basis.
fn segment(basis: &Plane, a: Vec3, b: Vec3) -> Result<Vec3> {
    validate_plane(basis).map_err(|_| Invalid("a view's basis is finite and orthonormal"))?;
    let point = |p: Vec3| p.is_finite() && [p.x, p.y, p.z].iter().all(|v| v.abs() <= MAX_NUMBER);
    require(point(a) && point(b), "a model point is finite and within 1e12 mm of the origin")?;
    let d = b - a;
    require(d.len() > EPS, "a segment or edge has no length")?;
    Ok(d)
}

/// `d` as the view shows it, in its plane's coordinates: refused when the view sees it end-on.
fn on_view(basis: &Plane, d: Vec3) -> Result<Vec3> {
    let shown = Vec3::new(d.dot(basis.x), d.dot(basis.y), 0.0);
    require(shown.len() > PARALLEL * d.len(), "a line is seen end-on in this view: it has no direction there")?;
    Ok(shown)
}

/// The angle between `u` and `v`, from 0 to π: refused when they are parallel either way.
fn between(u: Vec3, v: Vec3) -> Result<f64> {
    let sine = u.cross(v).len();
    require(sine > PARALLEL * u.len() * v.len(), "the lines are parallel: there is no angle between them")?;
    Ok(sine.atan2(u.dot(v)))
}

/// Whether `d` leaves the view's plane: the sine of its angle to the plane is over [`PARALLEL`].
fn out_of_plane(basis: &Plane, d: Vec3) -> bool {
    d.dot(basis.normal()).abs() > PARALLEL * d.len()
}

#[cfg(test)]
#[path = "drawing_measure_tests.rs"]
mod tests;
