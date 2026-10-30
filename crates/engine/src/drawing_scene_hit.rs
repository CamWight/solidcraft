//! What is under a point of a sheet: the objects of a sheet's scene as boxes on paper, and a view's
//! strokes. Plain data in, an answer out, for each hover and each press.

use solidcraft_geom::paper::{Item, ObjectKind};
use solidcraft_geom::{Aabb2, Vec2};

/// Where one object of a sheet's scene sits on paper, for picking and marking it. Beside the scene,
/// not in it, so no writer sees it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SheetObject {
    pub object: u64,
    pub kind: ObjectKind,
    /// The box round what it draws, along its lines' paths; with no lines, its boundary.
    pub bounds_mm: Aabb2,
    /// A view's boundary ([`crate::drawing_view::on_sheet`]), so a view that can't be drawn can
    /// still be picked.
    pub boundary_mm: Option<Aabb2>,
}

/// What is under a point of a sheet ([`hit`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hit {
    pub object: u64,
    pub kind: ObjectKind,
    /// The point is within the band of the box's edge: where a view is grabbed rather than clicked
    /// through.
    pub on_edge: bool,
}

/// The object of `objects`, in paint order, under `point_mm` on paper: the one whose box holds the
/// point within `band_mm`, the smallest of them, then the topmost. A view's box is its boundary, so
/// one that can't be drawn is still there; a dimension's is round its lines. The sheet's border is
/// never hit. A point or a band that isn't a finite number (or a band below 0) hits nothing. One pass
/// over the objects, for each hover.
pub fn hit(objects: &[SheetObject], point_mm: Vec2, band_mm: f64) -> Option<Hit> {
    if !point_mm.is_finite() || !band_mm.is_finite() || band_mm < 0.0 {
        return None;
    }
    let mut found: Option<(f64, Hit)> = None;
    for object in objects.iter().filter(|o| o.kind != ObjectKind::DrawingBorder) {
        let b = object.boundary_mm.unwrap_or(object.bounds_mm);
        if !b.inflated(band_mm).contains(point_mm) {
            continue;
        }
        let area = (b.max.x - b.min.x) * (b.max.y - b.min.y);
        if found.as_ref().is_none_or(|(smallest, _)| area <= *smallest) {
            let to_edge = [point_mm.x - b.min.x, b.max.x - point_mm.x, point_mm.y - b.min.y, b.max.y - point_mm.y];
            let on_edge = to_edge.iter().any(|gap| gap.abs() <= band_mm);
            found = Some((area, Hit { object: object.object, kind: object.kind, on_edge }));
        }
    }
    found.map(|(_, hit)| hit)
}

/// The stroke of `items` nearest `point`, within `band_mm`, and how far it is: the one painted last
/// when two are as near. Only strokes whose box reaches the point are measured.
pub(crate) fn nearest_stroke(items: &[Item], point: Vec2, band_mm: f64) -> Option<(usize, f64)> {
    let mut found: Option<(usize, f64)> = None;
    for (at, item) in items.iter().enumerate() {
        let Item::Stroke { path, .. } = item else { continue };
        for seg in path.iter().filter(|seg| seg.bounds().inflated(band_mm).contains(point)) {
            let distance = seg.point_at(seg.closest_param(point)).dist(point);
            if distance <= band_mm && found.is_none_or(|(_, nearest)| distance <= nearest) {
                found = Some((at, distance));
            }
        }
    }
    found
}
