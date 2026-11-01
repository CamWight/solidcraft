//! What a drawing's dimensions, balloons and notes point at on the model, and how it is found again.
//! They save what they name by name, never a picked point ([`ModelEdge`], [`ModelFace`],
//! [`ModelPoint`]), so when the model changes they follow it or say what they lost: they never move
//! to other geometry.
//!
//! A reference names its body, the feature that made the body (so a suppressed feature can be told
//! from a deleted one) and its occurrence path (so a part placed twice has a dimension on each copy).
//! The feature and the names on the body are what it is; the body's own name is a label, so a
//! renamed body keeps its references and no record is rewritten ([`Lookup::body`]).
//!
//! The rules: a reference names a body and its feature, and 1 to [`MAX_REFERENCE_NAME_BYTES`] bytes
//! in each name, without control characters; a corner names 2 to [`MAX_VERTEX_EDGES`] edges, a centre
//! one; an occurrence path is at most [`MAX_PLACEMENT_DEPTH`] ids, none 0.
//!
//! [`Lookup`] finds what a reference names again, strictly: an edge by its name, or the one edge
//! between the faces it names, never the piece nearest an old pick; a corner where all its named
//! edges meet, never the nearer of two; a centre the centre of its one named edge, which must be a
//! circle or an arc; a cylinder the faces its named face is now, all on one cylinder; a flat face the
//! faces its named face is now, all on one oriented plane and not split on purpose. What it can't
//! decide it says ([`Status`]), and it never chooses. [`Document::edge_reference`],
//! [`Document::corner_reference`] and [`Document::face_reference`] turn what a command picks (a
//! point, or an edge's or a face's index) into those names, and refuse a pick that doesn't name
//! one thing.

use std::collections::BTreeMap;
use std::f64::consts::PI;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use solidcraft_geom::drawing_measure::Cylinder;
use solidcraft_geom::{Plane, Seg2, Vec2, Vec3};
use solidcraft_kernel::{FaceSurface, face_surface};

use super::require;
use crate::appearance::face_index_at;
use crate::config::MAX_REFERENCE_NAME_BYTES;
use crate::construct::{MAX_VERTEX_EDGES, as_circle, as_line, point_tolerance, shared_ends};
use crate::name_lookup::{BodyEdges, EdgeMatch, face_has_name, faces_with_stem};
use crate::naming::{face_name_exact, face_names};
use crate::persistent_name::{EdgeName, FaceName, FeatureId};
use crate::{DocError, Document, MAX_PLACEMENT_DEPTH, Mat, Model, ModelBody, ModelState, Result, apply_point, apply_vector, map_plane, mat_inverse};

/// The most candidates an ambiguous dimension lists.
pub const MAX_CANDIDATES: usize = 16;

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

/// A face of a body, by its persistent name, as [`ModelEdge`] names an edge.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelFace {
    pub body: String,
    /// The feature that made the body.
    pub origin_feature: u64,
    /// The face's persistent name.
    pub name: String,
    /// As [`ModelEdge::path`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub path: Vec<u64>,
}

/// A point of a body: a corner, the end its named edges share, or with `centre` the centre of the
/// circle or arc its one named edge is.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelPoint {
    pub body: String,
    /// The feature that made the body.
    pub origin_feature: u64,
    /// The persistent names of the edges that meet there, or of the circle or arc.
    pub edges: Vec<String>,
    /// The centre of the circle or arc `edges` names, not a corner.
    #[serde(default, skip_serializing_if = "is_false")]
    pub centre: bool,
    /// As [`ModelEdge::path`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub path: Vec<u64>,
}

pub(super) fn is_false(v: &bool) -> bool {
    !*v
}

/// An edge reference's rules (a balloon's edge keeps them too).
pub(super) fn check_edge(edge: &ModelEdge) -> Result<()> {
    check_body(&edge.body, edge.origin_feature, &edge.path)?;
    check_reference_name(&edge.name)
}

/// A point reference's rules.
pub(super) fn check_face(face: &ModelFace) -> Result<()> {
    check_body(&face.body, face.origin_feature, &face.path)?;
    check_reference_name(&face.name)
}

pub(super) fn check_point(point: &ModelPoint) -> Result<()> {
    check_body(&point.body, point.origin_feature, &point.path)?;
    if point.centre {
        require(point.edges.len() == 1, || "a centre is of one named circle or arc".into())?;
    } else {
        require((2..=MAX_VERTEX_EDGES).contains(&point.edges.len()), || format!("a corner is where 2 to {MAX_VERTEX_EDGES} named edges meet"))?;
    }
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

/// What a balloon or a note points at as the model has it now: never saved.
#[derive(Clone, Debug, PartialEq)]
pub struct Located {
    /// Always [`Status::Ok`] for what points at nothing, which reads no model.
    pub status: Status,
    /// Where its arrow or leader ends, in the design's frame: when what it points at was found.
    pub point: Option<Vec3>,
    /// What it points at was found by exact history, as one edge.
    pub exact: bool,
}

impl Located {
    /// With nothing to read it from: why.
    pub fn lost(why: String) -> Located {
        Located { status: Status::Lost(why), point: None, exact: false }
    }
}

/// Where a dimension, or a balloon's edge, stands with the model.
#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    /// Measured.
    Ok,
    /// Measured, and showing the text given instead of its number.
    Overridden,
    /// A name finds several things, and none is chosen: at most [`MAX_CANDIDATES`] of them, in
    /// the model's order.
    Ambiguous(Vec<Candidate>),
    /// A feature it depends on is suppressed or rolled back.
    Suppressed,
    /// A feature it depends on failed: why. It comes back when that feature works again.
    Failed(String),
    /// What it measures is gone: why.
    Lost(String),
    /// Found, but the view can't measure it so: why (such as an angle to a line the view sees
    /// end-on).
    Unmeasurable(String),
}

/// One of the things an ambiguous name finds.
#[derive(Clone, Debug, PartialEq)]
pub enum Candidate {
    /// An edge of the body, by its index (as a `{body, index}` reference names it).
    Edge { body: String, index: usize },
    /// A point where the named edges meet, in the design's frame.
    Corner(Vec3),
    /// A face of the body, by its index (as a `{body, index}` face pick names it).
    Face { body: String, index: usize },
}

/// How a command names an edge (or a face) to measure, before it is saved by name
/// ([`Document::edge_reference`], [`Document::face_reference`]).
#[derive(Clone, Debug, PartialEq)]
pub enum EdgePick {
    /// A point on it, in the design's frame.
    Point(Vec3),
    /// Its index among its body's edges (or faces) as the model lists them: a [`Candidate::Edge`]'s
    /// (or a [`Candidate::Face`]'s).
    Index { body: String, index: usize },
}

/// How a command names a point to measure ([`Document::corner_reference`]).
#[derive(Clone, Debug, PartialEq)]
pub enum CornerPick {
    /// The corner, in the design's frame.
    Point(Vec3),
    /// Edges that meet there.
    Edges(Vec<EdgePick>),
    /// The centre of a circle or an arc: the edge.
    Centre(EdgePick),
}

impl Document {
    /// The edge `pick` names on `model` (this design's), on a body placed through the occurrences
    /// `path`, as a dimension saves it: its body, the feature that made the body, and its
    /// persistent name. A point must be on exactly one edge.
    pub fn edge_reference(&self, model: &Model, pick: &EdgePick, path: &[u64]) -> Result<ModelEdge> {
        let mut picker = Picker::new(self, model, path);
        let (body, edge) = picker.edge(pick)?;
        let (body, origin_feature, mut names) = picker.names(body, &[edge])?;
        Ok(ModelEdge { body, origin_feature, name: names.remove(0), path: path.to_vec() })
    }

    /// The face `pick` names, by its persistent name, as [`Document::edge_reference`] names an edge:
    /// a point must be on one face of one body (to the body's pick tolerance).
    pub fn face_reference(&self, model: &Model, pick: &EdgePick, path: &[u64]) -> Result<ModelFace> {
        let mut picker = Picker::new(self, model, path);
        let (body, face) = picker.face(pick)?;
        let b = picker.state.bodies.get(body).ok_or_else(|| DocError::Invalid("the body picked no longer exists".into()))?;
        let name = face_names(b).get(face).filter(|n| !n.is_empty()).cloned();
        let name = name.ok_or_else(|| DocError::Invalid(format!("face {face} of `{}` has no name", b.name)))?;
        Ok(ModelFace { body: b.name.clone(), origin_feature: b.feature, name, path: path.to_vec() })
    }

    /// The point `pick` names, as [`Document::edge_reference`] names an edge: a corner by the edges
    /// that meet there, a centre by its circle or arc. A point must be the end of 2 to
    /// [`MAX_VERTEX_EDGES`] edges of one body, edges given must meet at one point, and a centre's
    /// edge must be a circle or an arc.
    pub fn corner_reference(&self, model: &Model, pick: &CornerPick, path: &[u64]) -> Result<ModelPoint> {
        let mut picker = Picker::new(self, model, path);
        let (body, edges) = match pick {
            CornerPick::Point(p) => picker.corner_at(*p)?,
            CornerPick::Edges(picks) => {
                // One past the most a corner names, so the record's rule refuses the rest.
                let picked = picks.iter().take(MAX_VERTEX_EDGES + 1).map(|e| picker.edge(e)).collect::<Result<Vec<_>>>()?;
                let body = picked.first().map_or(0, |(b, _)| *b);
                require(picked.iter().all(|(b, _)| *b == body), || "a corner's edges are on one body".into())?;
                let edges: Vec<usize> = picked.iter().map(|(_, e)| *e).collect();
                picker.meeting(body, &edges)?;
                (body, edges)
            }
            CornerPick::Centre(pick) => {
                let (body, edge) = picker.edge(pick)?;
                picker.round(body, edge)?;
                (body, vec![edge])
            }
        };
        let centre = matches!(pick, CornerPick::Centre(_));
        let (body, origin_feature, edges) = picker.names(body, &edges)?;
        Ok(ModelPoint { body, origin_feature, edges, centre, path: path.to_vec() })
    }
}

/// One call's lookups: the model's bodies, and the edge names of each body named, worked out once.
/// Balloons find their edges through it too ([`super::balloon`]).
pub(super) struct Lookup<'a> {
    doc: &'a Document,
    model: &'a Model,
    state: Arc<ModelState>,
    /// Each named body's edge names, by its index in `state`.
    named: BTreeMap<usize, BodyEdges>,
    /// Each body a cylinder dimension names: the cylinder each face lies on (its axis a unit
    /// vector, in the body's own frame), by face.
    cylinders: BTreeMap<usize, Vec<Option<Cylinder>>>,
}

/// The cylinder a face lies on as the model has it now ([`Lookup::on_cylinder`]).
pub(super) struct OnCylinder {
    /// In the design's frame, its axis a unit vector.
    pub(super) cylinder: Cylinder,
    /// The body's index, and the corners of its faces' triangles on the cylinder, in the design's
    /// frame.
    pub(super) body: usize,
    corners: Vec<[Vec3; 3]>,
    /// The kernel's tolerance for faces on one cylinder of the body.
    pub(super) tol: f64,
    pub(super) exact: bool,
}

impl OnCylinder {
    /// Whether the silhouette `way` from the axis (a unit vector square to it) lies on the faces:
    /// a triangle of them lies across it.
    pub(super) fn covers(&self, way: Vec3) -> bool {
        let Cylinder { origin, axis, radius } = self.cylinder;
        let round = |p: Vec3| (p - origin).dot(axis.cross(way)).atan2((p - origin).dot(way));
        self.corners.iter().any(|t| straddles(t.map(round), self.tol / radius.max(self.tol)))
    }

    /// That silhouette's ends, from the faces' one end along the axis to the other.
    pub(super) fn ends(&self, way: Vec3) -> [Vec3; 2] {
        let Cylinder { origin, axis, radius } = self.cylinder;
        let along = |p: &Vec3| (*p - origin).dot(axis);
        let (low, high) = self.corners.iter().flatten().fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), p| (lo.min(along(p)), hi.max(along(p))));
        [origin + axis * low + way * radius, origin + axis * high + way * radius]
    }
}

/// A flat face as the model has it now ([`Lookup::on_plane`]), in the design's frame: a point on it,
/// its normal, and the corners of its triangles.
pub(super) struct OnPlane {
    pub(super) origin: Vec3,
    pub(super) normal: Vec3,
    corners: Vec<[Vec3; 3]>,
    pub(super) exact: bool,
}

impl OnPlane {
    /// The face's two ends as the view through `basis` sees it, edge-on: a line on paper.
    pub(super) fn ends(&self, basis: &Plane) -> Option<[Vec3; 2]> {
        let across = basis.normal().cross(self.normal);
        let order = |p: &&Vec3, q: &&Vec3| p.dot(across).total_cmp(&q.dot(across));
        let points = || self.corners.iter().flatten();
        Some([*points().min_by(order)?, *points().max_by(order)?])
    }
}

/// The edges a name finds on a body: their indices and polylines in the design's frame, whether
/// they are the one edge it names, whether exact history found it, and the body's index and the
/// frame it is placed in.
pub(super) struct Edges {
    pub(super) pieces: Vec<(usize, Vec<Vec3>)>,
    pub(super) whole: bool,
    pub(super) exact: bool,
    body: usize,
    /// The body's name now, which may not be the one saved.
    body_name: String,
    frame: Mat,
}

/// What a reference names on its body, to tell a renamed body by ([`Lookup::body`]).
#[derive(Clone, Copy)]
enum OnBody<'n> {
    Edges(&'n [String]),
    Face(&'n str),
}

impl OnBody<'_> {
    /// The features that made what it names.
    fn makers(self) -> Vec<FeatureId> {
        match self {
            OnBody::Edges(names) => names.iter().flat_map(|name| edge_makers(name)).collect(),
            OnBody::Face(name) => face_maker(name).into_iter().collect(),
        }
    }
}

/// The features that made the faces an edge's name says it separates.
fn edge_makers(name: &str) -> Vec<FeatureId> {
    EdgeName::parse(name).map_or_else(|_| Vec::new(), |edge| edge.faces.iter().filter_map(FaceName::feature).collect())
}

/// The feature that made the face a name says: none for a name without history.
fn face_maker(name: &str) -> Option<FeatureId> {
    FaceName::parse(name).ok()?.feature()
}

/// How a reference stands when what it names isn't found, by the features that made it: suppressed
/// or rolled back, failed, or else lost, and why.
fn gone(model: &Model, makers: &[FeatureId], lost: String) -> Status {
    let results = || model.results.iter().filter(|r| makers.iter().any(|m| m.0 == r.id));
    if results().any(|r| r.skipped) {
        return Status::Suppressed;
    }
    results().find_map(|r| r.error.clone()).map_or(Status::Lost(lost), Status::Failed)
}

impl<'a> Lookup<'a> {
    pub(super) fn new(doc: &'a Document, model: &'a Model) -> Lookup<'a> {
        Lookup { doc, model, state: model.state(), named: BTreeMap::new(), cylinders: BTreeMap::new() }
    }

    /// The body a reference names, as the model has it now: its index, and the frame its geometry
    /// is in through the reference's occurrence path. Among the bodies its feature made: the one
    /// with its name, or, when none has it, the one that has what it names, so a renamed body keeps
    /// its references; two that do are ambiguous, and it never chooses.
    fn body(&mut self, body: &str, feature: u64, path: &[u64], on: OnBody) -> std::result::Result<(usize, Mat), Status> {
        let state = Arc::clone(&self.state);
        let made: Vec<usize> =
            (0..state.bodies.len()).filter(|i| state.bodies.get(*i).is_some_and(|b| b.feature == feature && !b.body.is_mesh())).collect();
        let named = made.iter().copied().find(|i| state.bodies.get(*i).is_some_and(|b| b.name == body));
        let i = match named {
            Some(i) => i,
            None => self.renamed(body, feature, &made, on)?,
        };
        let b = state.bodies.get(i).ok_or_else(|| Status::Lost("its body no longer exists".into()))?;
        let frame = self.doc.body_frame_at(b, path).map_err(|_| Status::Lost("the occurrence its body is placed through no longer exists".into()))?;
        Ok((i, frame))
    }

    /// The one of the bodies `made` by `feature`, none named `body`, that has what `on` names.
    fn renamed(&mut self, body: &str, feature: u64, made: &[usize], on: OnBody) -> std::result::Result<usize, Status> {
        let holding: Vec<(usize, Candidate)> = made.iter().filter_map(|i| self.holds(*i, on).map(|found| (*i, found))).collect();
        match holding.as_slice() {
            [(i, _)] => Ok(*i),
            [] if !made.is_empty() => Err(gone(self.model, &on.makers(), format!("what it names on body `{body}` no longer exists"))),
            [] if self.state.bodies.iter().any(|b| b.name == body) => {
                Err(gone(self.model, &[FeatureId(feature)], format!("body `{body}` is made by another feature now")))
            }
            [] => Err(gone(self.model, &[FeatureId(feature)], format!("body `{body}` no longer exists"))),
            several => Err(Status::Ambiguous(several.iter().take(MAX_CANDIDATES).map(|(_, found)| found.clone()).collect())),
        }
    }

    /// What `on` names on body `i`, as a candidate, if it has it all.
    fn holds(&mut self, i: usize, on: OnBody) -> Option<Candidate> {
        let state = Arc::clone(&self.state);
        let b = state.bodies.get(i)?;
        match on {
            OnBody::Edges(names) => {
                let edges = self.names(i).ok()?;
                let found: Vec<Option<EdgeMatch>> = names.iter().map(|name| edges.matching(name)).collect();
                let index = match found.first()?.as_ref()? {
                    EdgeMatch::Edge(e) => *e,
                    EdgeMatch::Pieces(pieces) => *pieces.first()?,
                };
                found.iter().all(Option::is_some).then(|| Candidate::Edge { body: b.name.clone(), index })
            }
            OnBody::Face(name) => {
                let named = (0..b.body.face_count()).find(|f| face_has_name(b, *f, name));
                let index = named.or_else(|| faces_with_stem(b, name).first().copied())?;
                Some(Candidate::Face { body: b.name.clone(), index })
            }
        }
    }

    /// Body `i`'s edge names, worked out on first use.
    fn names(&mut self, i: usize) -> std::result::Result<&BodyEdges, Status> {
        let b = self.state.bodies.get(i).ok_or_else(|| Status::Lost("its body no longer exists".into()))?;
        Ok(self.named.entry(i).or_insert_with(|| BodyEdges::new(b)))
    }

    /// The middle of the edge `edge` names, in the design's frame, where a balloon's arrow or a note's
    /// leader ends: how it stands, the point, and whether it was found by exact history. A split edge
    /// has no one middle: the pieces are listed, and none is chosen.
    pub(super) fn middle(&mut self, edge: &ModelEdge) -> Located {
        let (status, point, exact) = match self.edges(edge) {
            Ok(found) if !found.whole => (Status::Ambiguous(edge_candidates(&found)), None, false),
            Ok(found) => {
                let point = found.pieces.first().and_then(|(_, line)| crate::joints::edge::middle(line)).map(|(p, _)| p);
                let status = if point.is_some() { Status::Ok } else { Status::Lost(format!("edge `{}` has no length", edge.name)) };
                (status, point, found.exact)
            }
            Err(status) => (status, None, false),
        };
        Located { status, point, exact }
    }

    /// The edges `edge` names: the one edge, or the pieces of it. One piece is that edge, though not
    /// found exactly: a name saved from a split face finds it on a body with the one face.
    pub(super) fn edges(&mut self, edge: &ModelEdge) -> std::result::Result<Edges, Status> {
        let (i, frame) = self.body(&edge.body, edge.origin_feature, &edge.path, OnBody::Edges(std::slice::from_ref(&edge.name)))?;
        let body_name = self.state.bodies.get(i).map_or_else(String::new, |b| b.name.clone());
        let names = self.names(i)?;
        let (indices, named) = match names.matching(&edge.name) {
            Some(EdgeMatch::Edge(e)) => (vec![e], true),
            Some(EdgeMatch::Pieces(pieces)) => (pieces, false),
            None => return Err(gone(self.model, &edge_makers(&edge.name), format!("edge `{}` no longer exists", edge.name))),
        };
        let whole = named || indices.len() == 1;
        let exact = named && indices.iter().all(|e| names.verified(*e));
        let placed = |e: &usize| names.mesh.edges.get(*e).map(|line| (*e, line.iter().map(|p| apply_point(&frame, *p)).collect()));
        Ok(Edges { pieces: indices.iter().filter_map(placed).collect(), whole, exact, body: i, body_name, frame })
    }

    /// The circle or arc `edge` names, as [`Lookup::arc`] reads it, and whether it was found exactly.
    /// The pieces of a split edge on one circle are still that circle, as [`Lookup::line`] keeps a
    /// line: read from the longest, so its points lie on what is left of it. Pieces on several
    /// circles are listed, and none is chosen.
    pub(super) fn whole_arc(&mut self, edge: &ModelEdge, basis: &Plane) -> std::result::Result<(f64, [Vec3; 4], bool), Status> {
        let found = self.edges(edge)?;
        if !found.whole && one_circle(&found.pieces).is_none() {
            return Err(Status::Ambiguous(edge_candidates(&found)));
        }
        let length = |line: &[Vec3]| line.windows(2).map(|w| w.first().zip(w.last()).map_or(0.0, |(a, b)| a.dist(*b))).sum::<f64>();
        let longest = found.pieces.iter().max_by(|a, b| length(&a.1).total_cmp(&length(&b.1)));
        let not_found = || Status::Lost(format!("edge `{}` no longer exists", edge.name));
        let (radius, points) = self.arc(edge, &found, longest.ok_or_else(not_found)?, basis)?;
        Ok((radius, points, found.exact))
    }

    /// The circle or arc that `piece` of `found` is, as the view through `basis` sees it face on:
    /// its radius, and its centre and its start, middle and end in the design's frame. Read exactly
    /// from the body, in the view's plane through the edge, so the points are the edge's.
    fn arc(&self, edge: &ModelEdge, found: &Edges, piece: &(usize, Vec<Vec3>), basis: &Plane) -> std::result::Result<(f64, [Vec3; 4]), Status> {
        let face_on = || Status::Unmeasurable(format!("edge `{}` isn't a circle in this view (it isn't parallel to it)", edge.name));
        let (index, line) = piece;
        let through = Plane { origin: *line.first().ok_or_else(face_on)?, ..*basis };
        let local = mat_inverse(&found.frame).and_then(|back| map_plane(&back, &through)).ok_or_else(face_on)?;
        let body = self.state.bodies.get(found.body).ok_or_else(|| Status::Lost("its body no longer exists".into()))?;
        let segs = body.body.planar_edges(&local, &[*index]).pop().flatten().ok_or_else(face_on)?;
        let not_round = || Status::Unmeasurable(format!("edge `{}` isn't a circle or an arc", edge.name));
        let (radius, points) = one_arc(&segs).ok_or_else(not_round)?;
        Ok((radius, points.map(|p| apply_point(&found.frame, local.to_world(p)))))
    }

    /// The straight line `edge` names, from end to end, in the design's frame. The pieces of a
    /// split edge that lie on one line are still that line.
    pub(super) fn line(&mut self, edge: &ModelEdge) -> std::result::Result<([Vec3; 2], bool), Status> {
        let found = self.edges(edge)?;
        let not_straight = || Status::Unmeasurable(format!("edge `{}` isn't straight", edge.name));
        let (_, along) = found.pieces.first().and_then(|(_, line)| as_line(line)).ok_or_else(not_straight)?;
        let mut points: Vec<Vec3> = found.pieces.iter().flat_map(|(_, line)| line.iter().copied()).collect();
        points.sort_by(|p, q| p.dot(along).total_cmp(&q.dot(along)));
        match straight(&points) {
            Some(ends) => Ok((ends, found.exact)),
            None if found.whole => Err(not_straight()),
            None => Err(Status::Ambiguous(edge_candidates(&found))),
        }
    }

    /// The point `point` names, in the design's frame: a corner, the one end all its named edges
    /// (or their pieces) share, or a centre.
    pub(super) fn corner(&mut self, point: &ModelPoint) -> std::result::Result<(Vec3, bool), Status> {
        if point.centre {
            return self.centre(point);
        }
        let (i, frame) = self.body(&point.body, point.origin_feature, &point.path, OnBody::Edges(&point.edges))?;
        let names = self.names(i)?;
        let mut exact = true;
        let mut ends: Vec<Vec<Vec3>> = Vec::with_capacity(point.edges.len());
        for name in &point.edges {
            let (indices, whole) = match names.matching(name) {
                Some(EdgeMatch::Edge(e)) => (vec![e], true),
                Some(EdgeMatch::Pieces(pieces)) => (pieces, false),
                None => return Err(gone(self.model, &edge_makers(name), format!("edge `{name}` no longer exists"))),
            };
            exact &= whole && indices.iter().all(|e| names.verified(*e));
            let lines = indices.iter().filter_map(|e| names.mesh.edges.get(*e));
            ends.push(lines.flat_map(|line| [line.first(), line.last()]).flatten().copied().collect());
        }
        match shared_ends(&ends).as_slice() {
            [] => Err(Status::Lost(format!("edges `{}` no longer meet", point.edges.join("`, `")))),
            [p] => Ok((apply_point(&frame, *p), exact)),
            several => Err(Status::Ambiguous(several.iter().take(MAX_CANDIDATES).map(|p| Candidate::Corner(apply_point(&frame, *p))).collect())),
        }
    }

    /// The centre `point` names: of the circle or arc its one named edge is, or the one circle its
    /// pieces lie on, as construction geometry finds a round edge's ([`as_circle`]).
    fn centre(&mut self, point: &ModelPoint) -> std::result::Result<(Vec3, bool), Status> {
        let name = point.edges.first().ok_or_else(|| Status::Lost("its centre names no edge".into()))?;
        let edge = ModelEdge { body: point.body.clone(), origin_feature: point.origin_feature, name: name.clone(), path: point.path.clone() };
        let found = self.edges(&edge)?;
        match one_circle(&found.pieces) {
            Some(centre) => Ok((centre, found.exact)),
            None if found.whole => Err(Status::Unmeasurable(format!("edge `{name}` isn't a circle or an arc"))),
            None => Err(Status::Ambiguous(edge_candidates(&found))),
        }
    }

    /// The cylinder `face` lies on (or the pieces it was cut into, all on one cylinder), with every
    /// face of the body on that cylinder.
    pub(super) fn on_cylinder(&mut self, face: &ModelFace) -> std::result::Result<OnCylinder, Status> {
        let (i, frame) = self.body(&face.body, face.origin_feature, &face.path, OnBody::Face(&face.name))?;
        let state = Arc::clone(&self.state);
        let b = state.bodies.get(i).ok_or_else(|| Status::Lost("its body no longer exists".into()))?;
        let (found, exact) = faces_named(b, &face.name);
        let candidates = || face_candidates(b, &found);
        let cylinders = self.cylinders.entry(i).or_insert_with(|| cylinders_of(b));
        let on = |f: &usize| cylinders.get(*f).copied().flatten();
        let first = match found.first() {
            None => {
                return Err(gone(
                    self.model,
                    &face_maker(&face.name).into_iter().collect::<Vec<_>>(),
                    format!("face `{}` no longer exists", face.name),
                ));
            }
            Some(f) => on(f).ok_or_else(|| Status::Unmeasurable(format!("face `{}` isn't a cylinder", face.name)))?,
        };
        let tol = cylinder_tolerance(b);
        if !found.iter().all(|f| on(f).is_some_and(|c| same_cylinder(&first, &c, tol))) {
            return Err(Status::Ambiguous(candidates()));
        }
        let faces: Vec<usize> = (0..cylinders.len()).filter(|f| on(f).is_some_and(|c| same_cylinder(&first, &c, tol))).collect();
        let axis = apply_vector(&frame, first.axis).normalized().ok_or_else(|| Status::Lost("its occurrence flattens it".into()))?;
        let cylinder = Cylinder { origin: apply_point(&frame, first.origin), axis, radius: first.radius };
        Ok(OnCylinder { cylinder, body: i, corners: face_corners(b, &frame, &faces), tol, exact })
    }

    /// The flat face `face` names, or the pieces it was cut into, all on one oriented plane. A face
    /// split on purpose (Split Face) is separate faces, so its pieces are listed and none is chosen.
    pub(super) fn on_plane(&mut self, face: &ModelFace) -> std::result::Result<OnPlane, Status> {
        let (i, frame) = self.body(&face.body, face.origin_feature, &face.path, OnBody::Face(&face.name))?;
        let state = Arc::clone(&self.state);
        let b = state.bodies.get(i).ok_or_else(|| Status::Lost("its body no longer exists".into()))?;
        let (found, exact) = faces_named(b, &face.name);
        let makers: Vec<FeatureId> = face_maker(&face.name).into_iter().collect();
        let first = found.first().ok_or_else(|| gone(self.model, &makers, format!("face `{}` no longer exists", face.name)))?;
        let plane = b.body.planar_face(*first).ok_or_else(|| Status::Unmeasurable(format!("face `{}` isn't flat", face.name)))?;
        if self.doc.coplanar_pieces(b, &found, |_| true).is_err() {
            return Err(Status::Ambiguous(face_candidates(b, &found)));
        }
        let normal = apply_vector(&frame, plane.normal()).normalized().ok_or_else(|| Status::Lost("its occurrence flattens it".into()))?;
        Ok(OnPlane { origin: apply_point(&frame, plane.origin), normal, corners: face_corners(b, &frame, &found), exact })
    }
}

/// The faces of `b` that `name` finds: the ones with that name, else the pieces of it; and whether
/// it found one face by exact history.
fn faces_named(b: &ModelBody, name: &str) -> (Vec<usize>, bool) {
    let named: Vec<usize> = (0..b.body.face_count()).filter(|f| face_has_name(b, *f, name)).collect();
    match named.as_slice() {
        [f] => {
            let exact = face_name_exact(b, *f);
            (named, exact)
        }
        [] => (faces_with_stem(b, name), false),
        _ => (named, false),
    }
}

/// `faces` of `b`, as candidates.
fn face_candidates(b: &ModelBody, faces: &[usize]) -> Vec<Candidate> {
    faces.iter().take(MAX_CANDIDATES).map(|f| Candidate::Face { body: b.name.clone(), index: *f }).collect()
}

/// The cylinder each face of `b` lies on, by face; `None` for a face of another surface.
fn cylinders_of(b: &ModelBody) -> Vec<Option<Cylinder>> {
    (0..b.body.face_count())
        .map(|f| match face_surface(&b.body, f) {
            Some(FaceSurface::Cylinder { origin, axis, radius }) => Some(Cylinder { origin, axis: Vec3::from(axis), radius }),
            _ => None,
        })
        .collect()
}

/// How near two cylinders of `b` must be to be one: the kernel's own tolerance for two faces on a
/// common cylinder.
fn cylinder_tolerance(b: &ModelBody) -> f64 {
    (b.body.size() * 1e-7).max(1e-9) * 10.0
}

/// Whether two cylinders are one: parallel axes, on one line, and one radius, to `tol` (the
/// kernel's own condition for faces on a common cylinder).
pub(super) fn same_cylinder(a: &Cylinder, b: &Cylinder, tol: f64) -> bool {
    let off = b.origin - a.origin;
    a.axis.dot(b.axis).abs() > 1.0 - 1e-9 && (a.radius - b.radius).abs() < tol && (off - a.axis * off.dot(a.axis)).len() < tol
}

/// The corners of the triangles of `faces` of `b`'s mesh, in the design's frame through `frame`.
fn face_corners(b: &ModelBody, frame: &Mat, faces: &[usize]) -> Vec<[Vec3; 3]> {
    let mesh = b.mesh();
    let on = mesh.triangles.iter().zip(&mesh.tri_face).filter(|(_, f)| faces.contains(&(**f as usize)));
    on.filter_map(|(t, _)| mesh.tri(t)).map(|corners| corners.map(|p| apply_point(frame, p))).collect()
}

/// Whether a triangle whose corners lie at `angles` round a cylinder's axis (from the way a
/// silhouette is, each from −π to π) lies across the silhouette: its corners round it, to `slack`,
/// and less than half a turn apart (so not round the far side).
fn straddles(angles: [f64; 3], slack: f64) -> bool {
    let (low, high) = angles.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), a| (lo.min(*a), hi.max(*a)));
    low <= slack && high >= -slack && high - low < PI
}

/// One command's picks on the bodies placed through one occurrence path. Each body's edges are
/// placed in the design's frame once, and only the body picked has its edge names worked out.
struct Picker<'a> {
    doc: &'a Document,
    state: Arc<ModelState>,
    path: &'a [u64],
    /// Each body's edges in the design's frame, by its index in `state`; `None` for a mesh body or
    /// one the path doesn't place.
    placed: BTreeMap<usize, Option<Vec<Vec<Vec3>>>>,
}

impl<'a> Picker<'a> {
    fn new(doc: &'a Document, model: &Model, path: &'a [u64]) -> Picker<'a> {
        Picker { doc, state: model.state(), path, placed: BTreeMap::new() }
    }

    /// Body `i`'s edges in the design's frame, worked out on first use.
    fn placed(&mut self, i: usize) -> Option<&Vec<Vec<Vec3>>> {
        if !self.placed.contains_key(&i) {
            let b = self.state.bodies.get(i).filter(|b| !b.body.is_mesh());
            let frame = b.and_then(|b| self.doc.body_frame_at(b, self.path).ok());
            let lines =
                b.zip(frame).map(|(b, frame)| b.mesh().edges.iter().map(|line| line.iter().map(|p| apply_point(&frame, *p)).collect()).collect());
            self.placed.insert(i, lines);
        }
        self.placed.get(&i).and_then(Option::as_ref)
    }

    /// The edge `pick` names: its body's index and its own.
    fn edge(&mut self, pick: &EdgePick) -> Result<(usize, usize)> {
        let on: Vec<(usize, usize)> = match pick {
            EdgePick::Index { body, index } => {
                let b = self.named(body, "edge")?;
                let count = self.placed(b).map_or(0, Vec::len);
                require(*index < count, || format!("body `{body}` has no edge {index}"))?;
                vec![(b, *index)]
            }
            EdgePick::Point(p) => {
                // A point on a curve lies off its chords by up to what the body was meshed to, so
                // that is looked for only when no edge passes through the point itself.
                let (mut on, mut curved) = (Vec::new(), Vec::new());
                for i in 0..self.state.bodies.len() {
                    let meshed_to = self.state.bodies.get(i).map_or(0.0, |b| b.display_tol());
                    let Some(lines) = self.placed(i) else { continue };
                    for (e, line) in lines.iter().enumerate() {
                        let off = line.windows(2).fold(f64::INFINITY, |off, w| if let [a, b] = w { off.min(p.dist_to_segment(*a, *b)) } else { off });
                        if off <= point_tolerance(*p) {
                            on.push((i, e));
                        } else if off <= meshed_to {
                            curved.push((i, e));
                        }
                    }
                }
                if on.is_empty() { curved } else { on }
            }
        };
        match on.as_slice() {
            [one] => Ok(*one),
            [] => Err(DocError::Invalid(format!("no edge at {}", at(pick)))),
            several => {
                Err(DocError::Invalid(format!("{} is on {} edges: give a point inside one edge, or its body and index", at(pick), several.len())))
            }
        }
    }

    /// The one body called `body` that the path places here, for a pick of its `what` (edge or
    /// face) by index.
    fn named(&mut self, body: &str, what: &str) -> Result<usize> {
        let named: Vec<usize> = (0..self.state.bodies.len()).filter(|i| self.state.bodies.get(*i).is_some_and(|b| b.name == *body)).collect();
        let placed: Vec<usize> = named.into_iter().filter(|i| self.placed(*i).is_some()).collect();
        match placed.as_slice() {
            [b] => Ok(*b),
            [] => Err(DocError::Invalid(format!("no body `{body}` with edges is placed here"))),
            _ => Err(DocError::Invalid(format!("more than one body is called `{body}`: give a point on the {what}"))),
        }
    }

    /// The face `pick` names: its body's index and its own. A point picks the face it lies on, to
    /// its body's pick tolerance ([`face_index_at`]), on whichever placed body has one there.
    fn face(&mut self, pick: &EdgePick) -> Result<(usize, usize)> {
        let on: Vec<(usize, usize)> = match pick {
            EdgePick::Index { body, index } => {
                let b = self.named(body, "face")?;
                let count = self.state.bodies.get(b).map_or(0, |b| b.body.face_count());
                require(*index < count, || format!("body `{body}` has no face {index}"))?;
                vec![(b, *index)]
            }
            EdgePick::Point(p) => {
                let mut on = Vec::new();
                for (i, b) in self.state.bodies.iter().enumerate().filter(|(_, b)| !b.body.is_mesh()) {
                    let back = self.doc.body_frame_at(b, self.path).ok().and_then(|frame| mat_inverse(&frame));
                    on.extend(back.and_then(|back| face_index_at(b, apply_point(&back, *p))).map(|f| (i, f)));
                }
                on
            }
        };
        match on.as_slice() {
            [one] => Ok(*one),
            [] => Err(DocError::Invalid(format!("no face at {}", at(pick)))),
            several => Err(DocError::Invalid(format!("{} is on {} faces: give its body and index", at(pick), several.len()))),
        }
    }

    /// The corner at `p`: the body whose edges end there, and those edges.
    fn corner_at(&mut self, p: Vec3) -> Result<(usize, Vec<usize>)> {
        let mut found: Vec<(usize, Vec<usize>)> = Vec::new();
        for i in 0..self.state.bodies.len() {
            let Some(lines) = self.placed(i) else { continue };
            let ends_here = |line: &Vec<Vec3>| [line.first(), line.last()].into_iter().flatten().any(|q| q.dist(p) <= point_tolerance(p));
            let edges: Vec<usize> = lines.iter().enumerate().filter(|(_, line)| ends_here(line)).map(|(e, _)| e).collect();
            if !edges.is_empty() {
                found.push((i, edges));
            }
        }
        let corner = at(&EdgePick::Point(p));
        match found.pop() {
            Some(_) if !found.is_empty() => Err(DocError::Invalid(format!("{corner} is a corner of more than one body: give its edges"))),
            Some((body, edges)) if (2..=MAX_VERTEX_EDGES).contains(&edges.len()) => Ok((body, edges)),
            Some((_, edges)) => {
                Err(DocError::Invalid(format!("{corner} is the end of {} edges: a corner is where 2 to {MAX_VERTEX_EDGES} edges meet", edges.len())))
            }
            None => Err(DocError::Invalid(format!("no corner at {corner}"))),
        }
    }

    /// Body `body`'s edge `edge` is a circle or an arc.
    fn round(&mut self, body: usize, edge: usize) -> Result<()> {
        let round = self.placed(body).and_then(|lines| lines.get(edge)).is_some_and(|line| as_circle(line).is_some());
        require(round, || format!("edge {edge} isn't a circle or an arc: a centre is a circle's or an arc's"))
    }

    /// Body `body`'s `edges` meet at one point.
    fn meeting(&mut self, body: usize, edges: &[usize]) -> Result<()> {
        let lines = self.placed(body).map(Vec::as_slice).unwrap_or_default();
        let ends: Vec<Vec<Vec3>> =
            edges.iter().filter_map(|e| lines.get(*e)).map(|line| [line.first(), line.last()].into_iter().flatten().copied().collect()).collect();
        match shared_ends(&ends).len() {
            1 => Ok(()),
            0 => Err(DocError::Invalid("the corner's edges don't meet".into())),
            n => Err(DocError::Invalid(format!("the corner's edges meet at {n} points: give the corner as a point"))),
        }
    }

    /// Body `body`'s `edges` as a reference names them: the body's name, the feature that made
    /// it, and each edge's persistent name.
    fn names(&self, body: usize, edges: &[usize]) -> Result<(String, u64, Vec<String>)> {
        let b = self.state.bodies.get(body).ok_or_else(|| DocError::Invalid("the body picked no longer exists".into()))?;
        let named = BodyEdges::new(b);
        let names = edges.iter().map(|e| named.name(*e).map(str::to_string).ok_or_else(|| DocError::Invalid(format!("edge {e} has no name"))));
        Ok((b.name.clone(), b.feature, names.collect::<Result<_>>()?))
    }
}

/// A pick, for a message.
fn at(pick: &EdgePick) -> String {
    match pick {
        EdgePick::Point(p) => format!("[{}, {}, {}]", p.x, p.y, p.z),
        EdgePick::Index { body, index } => format!("edge {index} of `{body}`"),
    }
}

/// The pieces a name found, as candidates.
pub(super) fn edge_candidates(found: &Edges) -> Vec<Candidate> {
    found.pieces.iter().take(MAX_CANDIDATES).map(|(index, _)| Candidate::Edge { body: found.body_name.clone(), index: *index }).collect()
}

/// One edge's arcs as one arc: its radius, and its centre, start, middle and end. The kernel gives a
/// whole circle as two halves. `None` unless every piece is an arc of one circle.
fn one_arc(segs: &[Seg2]) -> Option<(f64, [Vec2; 4])> {
    let &Seg2::Arc { center, radius, start, .. } = segs.first()? else { return None };
    let mut sweep = 0.0;
    for seg in segs {
        let &Seg2::Arc { center: c, radius: r, sweep: s, .. } = seg else { return None };
        if c.dist(center) > radius * 1e-9 || (r - radius).abs() > radius * 1e-9 {
            return None;
        }
        sweep += s;
    }
    let arc = Seg2::Arc { center, radius, start, sweep };
    Some((radius, [center, arc.start(), arc.mid(), arc.end()]))
}

/// The centre of the one circle every one of `pieces` lies on: `None` if one isn't a circle or an arc
/// ([`as_circle`]), or they aren't on one circle.
pub(super) fn one_circle(pieces: &[(usize, Vec<Vec3>)]) -> Option<Vec3> {
    let mut circles = pieces.iter().map(|(_, line)| as_circle(line));
    let (centre, normal, radius) = circles.next()??;
    let tol = radius * 1e-6;
    let same = |(c, n, r): (Vec3, Vec3, f64)| c.dist(centre) <= tol && (r - radius).abs() <= tol && n.dot(normal).abs() > 1.0 - 1e-9;
    circles.all(|c| c.is_some_and(same)).then_some(centre)
}

/// A straight polyline's ends.
pub(super) fn straight(line: &[Vec3]) -> Option<[Vec3; 2]> {
    as_line(line)?;
    Some([*line.first()?, *line.last()?])
}
