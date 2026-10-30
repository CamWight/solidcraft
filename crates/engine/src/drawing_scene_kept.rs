//! What a save keeps of a drawing's sheet scene: each view's last lines, apart from the provider's
//! cache, which goes when its drawing closes.

use std::collections::BTreeMap;
use std::sync::Arc;

use solidcraft_doc::drawing::Drawing;
use solidcraft_doc::drawing::view::ResolvedView;
use solidcraft_doc::{Document, ModelState};
use solidcraft_geom::drawing_view::ViewFrame;
use solidcraft_geom::paper::Group;
use solidcraft_geom::{Aabb2, Vec2};

use crate::Session;

/// Each drawing's last lines, by drawing and view: what a save can keep. A closed drawing's stay here
/// too, as plain data: a weak reference to the world state they came from says whether they are
/// current without holding it.
#[derive(Clone, Default)]
pub struct KeptLines(BTreeMap<u64, BTreeMap<u64, KeptView>>);

impl KeptLines {
    /// Keeps `lines` as drawing `drawing`'s view `view`'s.
    pub(crate) fn keep(&mut self, drawing: u64, view: u64, lines: KeptView) {
        self.0.entry(drawing).or_default().insert(view, lines);
    }
}

#[derive(Clone)]
pub(crate) struct KeptView {
    key: String,
    state: std::sync::Weak<ModelState>,
    group: Arc<Group>,
    boundary: Aabb2,
    origin_mm: Vec2,
}

impl KeptView {
    /// A view's lines as a save keeps them.
    pub(crate) fn new(view: &ResolvedView, state: &Arc<ModelState>, group: Arc<Group>, boundary: Aabb2, frame: &ViewFrame) -> KeptView {
        let origin_mm = frame.paper_map().apply(Vec2::ZERO);
        KeptView { key: lines_key(view), state: Arc::downgrade(state), group, boundary, origin_mm }
    }
}

/// What a view's lines were drawn from, as a save writes it: compared only within one build, whose
/// lines these are, so its text can be anything that tells two views apart.
pub(crate) fn lines_key(view: &ResolvedView) -> String {
    format!("{view:?}")
}

/// What a save of `design` keeps of its drawings' lines: only each view's own, drawn from the world
/// state there is now and as it would draw now, and it never draws one (`saved_geometry`'s rule). A
/// drawing with none keeps what its file has.
pub(crate) fn saved_lines(s: &Session, design: &Document) -> solidcraft_io::SheetCaches {
    let keep_from = s.file.as_ref().map(|(path, _)| path.clone());
    let world = s.cached_world_state().filter(|_| s.active_sketch.is_none() && s.pending_moves.is_empty());
    let Some(world) = world else { return solidcraft_io::SheetCaches { lines: Vec::new(), keep_from } };
    let current = |d: &Drawing, id: u64, kept: &KeptView| {
        let same = kept.state.upgrade().is_some_and(|state| Arc::ptr_eq(&state, &world));
        (same && d.resolve_view(id).is_ok_and(|view| lines_key(&view) == kept.key)).then(|| solidcraft_io::SavedView {
            view: id,
            key: kept.key.clone(),
            group: kept.group.clone(),
            boundary: kept.boundary,
            origin_mm: kept.origin_mm,
        })
    };
    let lines = design
        .drawings
        .iter()
        .filter_map(|d| {
            let kept = s.kept_lines.0.get(&d.id)?;
            let views: Vec<_> = d.views.iter().filter_map(|v| current(d, v.id, kept.get(&v.id)?)).collect();
            (!views.is_empty()).then_some(solidcraft_io::SheetLines { drawing: d.id, views })
        })
        .collect();
    solidcraft_io::SheetCaches { lines, keep_from }
}
