//! Drawing views of the session's model (W07, #5866): what a view shows and how far it reaches.
//! The snapshot command uses it now; saved views (W14) and their pictures (W17b) will too.
//!
//! A view shows every body the design places, where its occurrences place it. The browser's eye
//! counts for nothing: hiding a part in the model never changes a drawing.

use solidcraft_geom::{Aabb2, Plane};

use crate::Session;

/// How far a view reaches on its own plane, in model millimetres.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Extents {
    /// The bodies' box in the plane's own coordinates ([`Plane::to_local`]); empty with no bodies.
    pub model: Aabb2,
    /// How far a curved face may lie outside its display mesh: the largest display tolerance.
    pub error_mm: f64,
    pub bodies: usize,
}

/// The extents of every body the design places, seen on `basis`.
pub fn extents(s: &Session, basis: &Plane) -> Extents {
    let st = s.world_state();
    st.bodies.iter().fold(Extents { model: Aabb2::EMPTY, error_mm: 0.0, bodies: 0 }, |e, body| Extents {
        model: e.model.union(&body.mesh_bounds_in(basis)),
        error_mm: e.error_mm.max(body.display_tol()),
        bodies: e.bodies + 1,
    })
}
