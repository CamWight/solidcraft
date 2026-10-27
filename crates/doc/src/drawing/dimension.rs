//! A drawing's dimensions (W19b, #4896): each the distance or angle between named parts of its
//! view's model, measured as the view shows it ([`solidcraft_geom::drawing_measure`]). A dimension
//! saves what it measures by name, never a picked point, so when the model changes it follows the
//! model or says what it lost: it never moves to other geometry. Nothing worked out is saved: its
//! value, its status and its last good value come from the model each time.
//!
//! - **Linear**: between two corners ([`ModelPoint`]), horizontal, vertical or aligned as the view
//!   shows them, or along a direction on the view.
//! - **Length**: a straight edge's ends ([`ModelEdge`]).
//! - **Angle**: between two straight edges, the smaller angle between their lines, or on its other
//!   side π minus it, whichever way the kernel runs the edges.
//!
//! A reference names its body, the feature that made the body (so a suppressed feature can be told
//! from a deleted one) and its occurrence path (so a part placed twice has a dimension on each copy).
//! The dimension line sits `offset_mm` of paper from what it measures, and the text at a place on
//! paper from its view's centre, so it moves with the view and not with the model; or centred on
//! the dimension line.
//!
//! The rules, beside the record's: a dimension is on one of the drawing's views; a reference names
//! a body and its feature, and 1 to [`MAX_REFERENCE_NAME_BYTES`] bytes in each name, without control
//! characters; a corner names 2 to [`MAX_VERTEX_EDGES`] edges; an occurrence path is at most
//! [`MAX_PLACEMENT_DEPTH`] ids, none 0; a true length is measured aligned; a direction to measure
//! along isn't zero, and is at most [`MAX_REAL`] each way, as every number a drawing saves (a whole
//! number past it reads, then saves as one the file's check refuses); the offset and the text place
//! are on paper ([`is_paper_coordinate`]); at most [`MAX_DECIMALS`] decimals; an override text of 1
//! to [`MAX_TEXT_BYTES`] bytes; at most [`MAX_DIMENSIONS`] dimensions, listed by id.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use solidcraft_geom::drawing_measure::{LinearDirection, Reading};
use solidcraft_geom::drawing_view::is_paper_coordinate;
use solidcraft_geom::paper::MAX_SIDE_MM;

use super::{Drawing, add_by_id, in_id_order, require};
use crate::config::MAX_REFERENCE_NAME_BYTES;
use crate::construct::MAX_VERTEX_EDGES;
use crate::format::MAX_REAL;
use crate::{DocError, MAX_PLACEMENT_DEPTH, Result, ids};

/// The most dimensions a drawing may have.
pub const MAX_DIMENSIONS: usize = 10_000;
/// The most decimals a dimension shows.
pub const MAX_DECIMALS: u8 = 6;
/// The longest text shown instead of a dimension's number, in bytes.
pub const MAX_TEXT_BYTES: usize = 1024;

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
}

/// An edge of a body, by its persistent name.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelEdge {
    pub body: String,
    /// The feature that made the body.
    pub origin_feature: u64,
    pub name: String,
    /// The occurrences the body is placed through, from the top; empty for the design's own body.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub path: Vec<u64>,
}

/// A corner of a body: the end its named edges share.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelPoint {
    pub body: String,
    /// The feature that made the body.
    pub origin_feature: u64,
    /// The persistent names of the edges that meet there.
    pub edges: Vec<String>,
    /// As [`ModelEdge::path`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub path: Vec<u64>,
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

fn is_false(v: &bool) -> bool {
    !*v
}

impl Drawing {
    /// Adds `dimensions`, each where its id puts it, as [`Drawing::add_views`] adds views.
    pub fn add_dimensions(&mut self, dimensions: impl IntoIterator<Item = Arc<Dimension>>) {
        add_by_id(&mut self.dimensions, dimensions, |d| d.id);
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
        require(self.views.binary_search_by_key(&d.view, |v| v.id).is_ok(), on)?;
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
        }
        let off_paper = || format!("its offset and its text's place are at most {MAX_SIDE_MM} mm on paper");
        let text = match d.text {
            TextPlace::At([x, y]) => is_paper_coordinate(x) && is_paper_coordinate(y),
            TextPlace::Centred => true,
        };
        require(is_paper_coordinate(d.offset_mm) && text, off_paper)?;
        require(d.decimals <= MAX_DECIMALS, || format!("it shows at most {MAX_DECIMALS} decimals"))?;
        match &d.override_text {
            Some(text) => require(!text.is_empty() && text.len() <= MAX_TEXT_BYTES, || {
                format!("the text shown instead of its number is 1 to {MAX_TEXT_BYTES} bytes")
            }),
            None => Ok(()),
        }
    }
}

/// An edge reference's rules.
fn check_edge(edge: &ModelEdge) -> Result<()> {
    check_body(&edge.body, edge.origin_feature, &edge.path)?;
    check_reference_name(&edge.name)
}

/// A corner reference's rules.
fn check_point(point: &ModelPoint) -> Result<()> {
    check_body(&point.body, point.origin_feature, &point.path)?;
    require((2..=MAX_VERTEX_EDGES).contains(&point.edges.len()), || format!("a corner is where 2 to {MAX_VERTEX_EDGES} named edges meet"))?;
    point.edges.iter().try_for_each(|name| check_reference_name(name))
}

/// The body a reference is on: its name, the feature that made it, and its occurrence path.
fn check_body(body: &str, origin_feature: u64, path: &[u64]) -> Result<()> {
    check_reference_name(body)?;
    require(origin_feature != 0, || "a reference names the feature that made its body".into())?;
    require(path.len() <= MAX_PLACEMENT_DEPTH && !path.contains(&0), || {
        format!("an occurrence path is at most {MAX_PLACEMENT_DEPTH} occurrences, none of them 0")
    })
}

/// A body's or an edge's name in a reference.
fn check_reference_name(name: &str) -> Result<()> {
    require(!name.is_empty() && name.len() <= MAX_REFERENCE_NAME_BYTES && !name.chars().any(char::is_control), || {
        format!("a name in a reference is 1 to {MAX_REFERENCE_NAME_BYTES} bytes, without control characters")
    })
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

#[cfg(test)]
#[path = "dimension_tests.rs"]
mod tests;
