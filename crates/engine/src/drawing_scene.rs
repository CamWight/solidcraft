//! The sheet scene (#4890 W10, #3890 W15): everything drawn on one sheet of a drawing as one paper
//! scene ([`Scene`]), for the sheet page and every export, so the screen and the files draw the
//! same thing. In paint order: the sheet's drawing border ([`SheetLayout::paper`]), then each view
//! on the sheet by id, its visible lines as [`paper_group`] makes them, then the dimensions of those
//! views, by view and id ([`crate::drawing_dimension_paper`]). Text, the title block, tables, hidden
//! lines and shaded views each join as one more kind of group.
//!
//! [`build`] draws a sheet at once, each view the one way: its lines worked out centred on the view,
//! then moved to its centre, so the same lines on the same centre are the same bytes however they
//! got there. It reads the model as it is: no model is evaluated.
//!
//! The sheet page draws through the provider instead, which keeps each view's lines between frames:
//! [`extract`] returns the last scene at once and says which views are out of date, and [`step`]
//! brings one of them up to date, a short piece of work for the UI's tick: a view's projection with
//! its dimensions in the same swap, so new lines never show under old dimensions, or, once the views
//! are, the dimensions of one view that changed. A view moved across its sheet is moved, never
//! projected again, and an edited dimension is read again alone. The provider
//! evaluates no model either. Its cache is the
//! session's view state, beside the drawing: never saved, never an undo step, and dropped when its
//! drawing closes. What a save keeps of it is apart, in [`KeptLines`].
//!
//! The first [`extract`] after a drawing opens takes the lines its file keeps
//! ([`solidcraft_io::read_sheet_lines`]), so opening a design with many drawings costs nothing extra.
//! Written by this build for this very model and as a view would draw now, a view's lines are fresh
//! and nothing is drawn; otherwise they show, listed in `stale`, until they are drawn again.
//!
//! [`SheetLayout::paper`]: solidcraft_geom::drawing_sheet::SheetLayout::paper

use std::borrow::Borrow;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use solidcraft_doc::drawing::dimension::{self, Dimension};
use solidcraft_doc::drawing::view::{ResolvedView, View};
use solidcraft_doc::drawing::{Drawing, Sheet};
use solidcraft_doc::drawing_view::TangentEdges;
use solidcraft_doc::{DocError, Document, ModelState, ids};
use solidcraft_geom::drawing_view::ViewFrame;
use solidcraft_geom::paper::{Group, Item, MAX_RECORDS, ObjectKind, Scene};
use solidcraft_geom::{Aabb2, Loop2, Region2, Seg2, Vec2};

use crate::drawing_view::{CHORD_MM, MIN_MODEL_CHORD_MM, ShownSet, Stats, Unmeasured, paper_group};
use crate::params::bad;
use crate::{EngineError, Result, Session};

/// The command the sheet scene answers for, in its errors.
pub const CMD: &str = "drawing.scene";

/// Numbers every scene the provider assembles, in every session, so a page never sees one number
/// for two scenes, not even across closing and opening its drawing.
static GENERATION: AtomicU64 = AtomicU64::new(0);

/// What became of one view of a sheet scene.
#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    /// Its visible lines are in the scene.
    Drawn,
    /// It has nothing to measure, so it draws nothing: the sheet page shows its placeholder.
    Placeholder(Unmeasured),
    /// It can't be drawn yet, or its lines are more than a paper scene holds: why. The sheet's
    /// other views are drawn all the same.
    Failed(String),
}

impl Status {
    /// Its name in replies.
    pub fn name(&self) -> &'static str {
        match self {
            Status::Drawn => "drawn",
            Status::Placeholder(_) => "placeholder",
            Status::Failed(_) => "failed",
        }
    }
}

/// One view of a sheet scene.
#[derive(Clone, Debug)]
pub struct ViewState {
    pub view: u64,
    pub status: Status,
    /// What working out its lines cost, when they were.
    pub stats: Option<Stats>,
}

/// One dimension of a sheet scene.
#[derive(Clone, Debug, PartialEq)]
pub struct DimensionState {
    pub dimension: u64,
    pub view: u64,
    /// Where it stands with the model.
    pub status: dimension::Status,
    /// Whether its lines are in the scene. A dimension that isn't found again has none, and neither
    /// has one whose view isn't drawn.
    pub drawn: bool,
    /// Why not, when its status doesn't say.
    pub why: Option<String>,
}

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

/// One sheet drawn at once ([`build`]).
#[derive(Clone, Debug)]
pub struct SheetScene {
    pub scene: Scene,
    /// Each view on the sheet, in id order.
    pub views: Vec<ViewState>,
    /// The sheet's border, then each view on it, then each dimension drawn, in paint order.
    pub objects: Vec<SheetObject>,
    /// Each dimension of a view on the sheet, by view and id.
    pub dimensions: Vec<DimensionState>,
}

/// One sheet as the provider last drew it ([`extract`]).
#[derive(Clone, Debug)]
pub struct SheetView {
    pub scene: Arc<Scene>,
    /// Goes up each time `scene` changes, so the page redraws only then.
    pub generation: u64,
    /// Each view on the sheet drawn so far, in id order, as it was last drawn.
    pub views: Vec<ViewState>,
    /// The views out of date, in id order: not drawn yet, or drawn from another model or with other
    /// settings than they have now. Their last lines stay in the scene until [`step`] draws them.
    pub stale: Vec<u64>,
    /// The sheet's border, then each view and dimension drawn so far, where it was last drawn.
    pub objects: Arc<[SheetObject]>,
    /// Each dimension of a view on the sheet read so far, by view and id, as it last was.
    pub dimensions: Vec<DimensionState>,
    /// The dimensions out of date whose views aren't: [`step`] reads them once the views are done.
    pub stale_dimensions: Vec<u64>,
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

/// The id of the sheet `named` names in `drawing` (its id or its one name), refused as `cmd`: the
/// sheet a command asks to draw.
pub fn sheet_id(drawing: &Drawing, named: &str, cmd: &str) -> Result<u64> {
    let at = drawing.sheet_index(named).map_err(|e| bad(cmd, e.to_string()))?;
    drawing.sheets.get(at).map(|sheet| sheet.id).ok_or_else(|| bad(cmd, format!("no sheet `{named}`")))
}

/// Sheet `sheet` of `drawing`, one of `s`'s design's drawings, drawn at once: its border, then each
/// of its views by id. A view with nothing to measure, or one that can't be drawn, is left out and
/// says why; the sheet is refused only when it can't be drawn at all.
pub fn build(s: &Session, drawing: &Drawing, sheet: u64) -> Result<SheetScene> {
    let record = sheet_of(drawing, sheet)?;
    let on = views_on(drawing, sheet)?;
    // Worked out once for the sheet: every view draws from the same one. Each view's lines are
    // placed as they are drawn, so at most one view's are held beside the scene.
    let shown = crate::drawing_view::shown(s);
    let drawn = on.iter().map(|(id, view, centre)| (*id, *centre, draw(s, &shown, *id, view)));
    let mut assembly = assemble(record, drawn)?;
    // After the views, as they need the views' frames: one read for the sheet's dimensions.
    let frames: BTreeMap<u64, ViewFrame> = assembly.placed.iter().map(|(view, (frame, _))| (*view, *frame)).collect();
    let wanted = dimensions_on(drawing, on.iter().map(|(id, ..)| *id));
    let dimensions = draw_dimensions(s, drawing, &wanted, |view| frames.get(&view).copied());
    assembly.add_dimensions(dimensions.iter());
    let (scene, views, objects, dimensions) = assembly.finish()?;
    Ok(SheetScene { scene, views, objects, dimensions })
}

/// A view's lines for a sheet scene, centred on the view, or why it has none.
#[derive(Clone, Debug)]
struct Drawn {
    group: Option<Arc<Group>>,
    status: Status,
    stats: Option<Stats>,
    /// Its boundary, centred on the view as its lines are.
    boundary: Option<Aabb2>,
    /// The frame its lines were drawn through, for its dimensions.
    frame: Option<ViewFrame>,
}

/// A dimension's lines for a sheet scene, centred on its view, and how it read.
#[derive(Clone, Debug)]
struct DrawnDimension {
    dimension: u64,
    view: u64,
    status: dimension::Status,
    group: Option<Group>,
    why: Option<String>,
}

/// View `id`, drawing as `view` does, of the bodies `shown` holds: its visible lines, sampled at
/// [`CHORD_MM`] on paper, or coarser past 100:1, where that would ask the mesh for chords finer than
/// [`MIN_MODEL_CHORD_MM`]; centred on the view ([`assemble`] moves them to its centre). Lines past
/// the paper scene's [`MAX_RECORDS`] are refused here, so no view holds more than a scene takes.
fn draw(s: &Session, shown: &std::result::Result<ShownSet, Unmeasured>, id: u64, view: &ResolvedView) -> Drawn {
    let measured = shown.as_ref().map_err(|why| *why).and_then(|set| crate::drawing_view::measure(set, &view.basis).map(|e| (set, e)));
    let extents = measured.as_ref().map(|(_, extents)| *extents).map_err(|why| *why);
    let boundary = crate::drawing_view::placed(view, extents, Vec2::ZERO, CMD).ok().map(|placed| placed.boundary);
    let none = |status| Drawn { group: None, status, stats: None, boundary, frame: None };
    // A style that draws hidden lines is refused here too, so the lines below are visible ones.
    if let Some(why) = crate::drawing_view::not_drawable(view.display_style.value, view.reference_style.value, true) {
        return none(Status::Failed(why.into()));
    }
    // Every body the design places, until saved views have `source` and `hide` (V4) to keep a part.
    let (set, extents) = match measured {
        Ok(measured) => measured,
        Err(why) => return none(Status::Placeholder(why)),
    };
    let around = extents.model.inflated(view.reference_margin_mm);
    let tangent_edges = view.tangent_edges.value == TangentEdges::Show;
    // The margin keeps the mesh's chord at or above its floor once divided back by the factor.
    let chord_mm = CHORD_MM.max(MIN_MODEL_CHORD_MM * view.scale.value.factor() * (1.0 + 1e-9));
    let frame = match crate::drawing_view::frame(view.basis, view.scale.value, &around, Vec2::ZERO, CMD) {
        Ok(frame) => frame,
        Err(e) => return none(Status::Failed(reason(e))),
    };
    match crate::drawing_view::lines(s, set, &frame, false, tangent_edges, chord_mm, CMD) {
        Ok(lines) => {
            let group = paper_group(&lines, id);
            if group.records() > MAX_RECORDS {
                return none(Status::Failed(format!("its lines are more than a paper scene's {MAX_RECORDS} records: choose a smaller scale")));
            }
            Drawn { group: Some(Arc::new(group)), status: Status::Drawn, stats: Some(lines.stats), boundary, frame: Some(frame) }
        }
        Err(e) => none(Status::Failed(reason(e))),
    }
}

/// The records of `wanted` read once, each drawn through its view's frame (`None`: its view isn't
/// drawn), in `wanted`'s order. A record that can't be read says so instead.
fn draw_dimensions(s: &Session, d: &Drawing, wanted: &[Arc<Dimension>], frame: impl Fn(u64) -> Option<ViewFrame>) -> Vec<DrawnDimension> {
    let ids: Vec<u64> = wanted.iter().map(|record| record.id).collect();
    let read = s.doc.resolve_dimensions(&s.model, d, &ids);
    let one = |at: usize, record: &Arc<Dimension>| match read.as_ref().map(|all| all.get(at)) {
        Ok(Some(resolved)) => {
            let group = frame(record.view).and_then(|frame| crate::drawing_dimension_paper::paper_group(record, resolved, &frame));
            let nothing = group.is_none() && !resolved.points.is_empty() && frame(record.view).is_some();
            let why = nothing.then(|| "what it measures has no place to draw on its view".to_string());
            DrawnDimension { dimension: record.id, view: record.view, status: resolved.status.clone(), group, why }
        }
        Ok(None) | Err(_) => {
            let status = dimension::Status::Unmeasurable(format!("dimension {} can't be read", ids::encode(record.id)));
            DrawnDimension { dimension: record.id, view: record.view, status, group: None, why: None }
        }
    };
    wanted.iter().enumerate().map(|(at, record)| one(at, record)).collect()
}

/// The dimensions of `views` of `d`, by view and id.
fn dimensions_on(d: &Drawing, views: impl Iterator<Item = u64>) -> Vec<Arc<Dimension>> {
    let views: BTreeSet<u64> = views.collect();
    let mut on: Vec<Arc<Dimension>> = d.dimensions.iter().filter(|record| views.contains(&record.view)).cloned().collect();
    on.sort_by_key(|record| (record.view, record.id));
    on
}

/// A sheet's scene being put together: what [`assemble`] gives, for its dimensions to join.
struct Assembly {
    scene: Scene,
    /// The groups in the scene, as a paper scene counts them.
    records: usize,
    /// One group checked alone on the sheet before it joins, so one past the limits leaves out only
    /// itself, and checking costs one pass over each group.
    alone: Scene,
    states: Vec<ViewState>,
    objects: Vec<SheetObject>,
    /// The frame and centre of each view whose lines are in the scene.
    placed: BTreeMap<u64, (ViewFrame, Vec2)>,
    dimensions: Vec<DimensionState>,
}

/// Why a group stayed out of a scene.
enum LeftOut {
    TooManyRecords,
    Invalid(String),
}

impl Assembly {
    /// Puts `group` in the scene moved to `centre`, and gives the box round it there.
    fn add(&mut self, group: &Group, centre: Vec2) -> std::result::Result<Aabb2, LeftOut> {
        let more = group.records();
        if self.records.saturating_add(more) > MAX_RECORDS {
            return Err(LeftOut::TooManyRecords);
        }
        self.alone.groups.push(moved(group, centre));
        if let Err(e) = self.alone.validate() {
            self.alone.groups.clear();
            return Err(LeftOut::Invalid(e.to_string()));
        }
        self.records += more;
        let bounds = self.alone.groups.first().map_or(Aabb2::EMPTY, group_bounds);
        self.scene.groups.append(&mut self.alone.groups);
        Ok(bounds)
    }

    /// Adds `dimensions` after the views, each moved to its view's centre. One whose view isn't in
    /// the scene or that would take it past a paper scene's limits is left out and says why.
    fn add_dimensions<'a>(&mut self, dimensions: impl Iterator<Item = &'a DrawnDimension>) {
        for one in dimensions {
            let mut why = one.why.clone();
            let mut drawn = false;
            match (self.placed.get(&one.view).map(|(_, centre)| *centre), &one.group) {
                (None, _) => why = Some("its view isn't drawn".into()),
                (Some(centre), Some(group)) => match self.add(group, centre) {
                    Ok(bounds_mm) => {
                        drawn = true;
                        self.objects.push(SheetObject { object: one.dimension, kind: ObjectKind::Dimension, bounds_mm, boundary_mm: None });
                    }
                    Err(LeftOut::TooManyRecords) => {
                        why = Some(format!("with its lines the sheet is more than a paper scene's {MAX_RECORDS} records"))
                    }
                    Err(LeftOut::Invalid(e)) => why = Some(e),
                },
                (Some(_), None) => {}
            }
            self.dimensions.push(DimensionState { dimension: one.dimension, view: one.view, status: one.status.clone(), drawn, why });
        }
    }

    fn finish(self) -> Result<(Scene, Vec<ViewState>, Vec<SheetObject>, Vec<DimensionState>)> {
        self.scene.validate().map_err(|e| bad(CMD, e.to_string()))?;
        Ok((self.scene, self.states, self.objects, self.dimensions))
    }
}

/// The scene of sheet `record`: its border, then each of `views` (its id, centre and lines, in id
/// order) moved to its centre. A view that would take the scene past a paper scene's limits is left
/// out and says why, and the views after it are still drawn when they fit.
fn assemble<D: Borrow<Drawn>>(record: &Sheet, views: impl Iterator<Item = (u64, Vec2, D)>) -> Result<Assembly> {
    let size_mm = record.sheet.size_mm().map_err(|e| bad(CMD, e.to_string()))?;
    let border = match &record.border {
        Some(border) => border.layout(&record.sheet).map_err(|e| bad(CMD, e.to_string()))?.paper(record.id),
        None => Vec::new(),
    };
    let records = border.iter().map(Group::records).fold(0usize, usize::saturating_add);
    let mut objects = Vec::new();
    if !border.is_empty() {
        // The border's groups are one object.
        let bounds_mm = border.iter().map(group_bounds).fold(Aabb2::EMPTY, |all, one| all.union(&one));
        objects.push(SheetObject { object: record.id, kind: ObjectKind::DrawingBorder, bounds_mm, boundary_mm: None });
    }
    let scene = Scene { size_mm, groups: border };
    let alone = Scene { size_mm, groups: Vec::with_capacity(1) };
    let mut assembly = Assembly { scene, records, alone, states: Vec::new(), objects, placed: BTreeMap::new(), dimensions: Vec::new() };
    for (view, centre, drawn) in views {
        let drawn: &Drawn = drawn.borrow();
        let mut status = drawn.status.clone();
        let mut lines = None;
        if let Some(group) = &drawn.group {
            match assembly.add(group, centre) {
                Ok(bounds) => {
                    lines = Some(bounds);
                    if let Some(frame) = drawn.frame {
                        assembly.placed.insert(view, (frame, centre));
                    }
                }
                Err(LeftOut::TooManyRecords) => {
                    status = Status::Failed(format!(
                        "with its lines the sheet is more than a paper scene's {MAX_RECORDS} records: put it on another sheet, or choose a smaller scale"
                    ));
                }
                Err(LeftOut::Invalid(e)) => status = Status::Failed(format!("{e}: choose a smaller scale")),
            }
        }
        let boundary_mm = drawn.boundary.map(|b| Aabb2 { min: b.min + centre, max: b.max + centre });
        if let Some(bounds_mm) = lines.filter(|b| !b.is_empty()).or(boundary_mm) {
            assembly.objects.push(SheetObject { object: view, kind: ObjectKind::View, bounds_mm, boundary_mm });
        }
        assembly.states.push(ViewState { view, status, stats: drawn.stats });
    }
    Ok(assembly)
}

/// The box round what `group` draws, along its lines' paths.
fn group_bounds(group: &Group) -> Aabb2 {
    let mut bounds = Aabb2::EMPTY;
    let mut add = |path: &[Seg2]| path.iter().for_each(|seg| bounds = bounds.union(&seg.bounds()));
    for item in &group.items {
        match item {
            Item::Stroke { path, .. } => add(path),
            Item::Fill { region } => region.loops().for_each(|ring| add(&ring.segs)),
        }
    }
    bounds
}

/// `group` moved `by` on paper: each point plus `by`, exactly, so the same lines moved to the same
/// centre are the same bytes however they got there.
fn moved(group: &Group, by: Vec2) -> Group {
    let segs = |path: &[Seg2]| -> Vec<Seg2> {
        let shift = |seg: &Seg2| match *seg {
            // An arc's centre moves; its radius and its angles stay.
            Seg2::Arc { center, radius, start, sweep } => Seg2::Arc { center: center + by, radius, start, sweep },
            _ => seg.map_controls(|p| p + by),
        };
        path.iter().map(shift).collect()
    };
    let ring = |l: &Loop2| Loop2 { segs: segs(&l.segs) };
    let items = group.items.iter().map(|item| match item {
        Item::Stroke { path, closed, width_mm } => Item::Stroke { path: segs(path), closed: *closed, width_mm: *width_mm },
        Item::Fill { region } => Item::Fill { region: Region2 { outer: ring(&region.outer), holes: region.holes.iter().map(ring).collect() } },
    });
    Group { object: group.object, kind: group.kind, items: items.collect() }
}

/// Sheet `sheet` of `d`.
fn sheet_of(d: &Drawing, sheet: u64) -> Result<&Sheet> {
    d.sheets.iter().map(|s| &**s).find(|s| s.id == sheet).ok_or_else(|| bad(CMD, format!("the drawing has no sheet {}", ids::encode(sheet))))
}

/// The views on sheet `sheet` of `d`, in id order, each as it draws and with its centre.
fn views_on(d: &Drawing, sheet: u64) -> Result<Vec<(u64, ResolvedView, Vec2)>> {
    let fail = |e: DocError| bad(CMD, e.to_string());
    let mut on: Vec<u64> = d.views.iter().filter(|v| v.sheet == sheet).map(|v| v.id).collect();
    on.sort_unstable();
    on.into_iter()
        .map(|id| -> Result<(u64, ResolvedView, Vec2)> { Ok((id, d.resolve_view(id).map_err(fail)?, d.view_centre(id).map_err(fail)?)) })
        .collect()
}

/// `e`'s message without the command's name: a view's reason in a sheet scene.
fn reason(e: EngineError) -> String {
    match e {
        EngineError::BadParams { msg, .. } => msg,
        e => e.to_string(),
    }
}

/// The provider's cache: per view, the lines it was last drawn with. The session holds one, for
/// the drawing open.
#[derive(Default)]
pub struct Cache {
    /// The drawing it is for.
    drawing: Option<u64>,
    /// The sheet last extracted: the one [`step`] brings up to date.
    sheet: Option<u64>,
    /// Each view's last drawing, by id.
    views: BTreeMap<u64, Entry>,
    /// Each dimension's last drawing, by id.
    dimensions: BTreeMap<u64, DimensionEntry>,
    /// The shown set of the world state views were last drawn from, worked out once for it.
    shown: Option<(Arc<ModelState>, std::result::Result<ShownSet, Unmeasured>)>,
    last: Option<Assembled>,
    /// Numbers each view drawn and each read of dimensions, so a scene is assembled again only when
    /// one of its views or dimensions was.
    serial: u64,
    /// Views drawn, for tests: a move draws none.
    #[cfg(test)]
    projections: usize,
}

/// A view's last drawing, and what it was drawn from: the world state (held, so its address can't
/// be reused while it is compared) and the view as it drew.
struct Entry {
    state: Arc<ModelState>,
    view: ResolvedView,
    drawn: Drawn,
    serial: u64,
}

/// A dimension's last drawing, and what it was drawn from: its record, the world state, its view's
/// drawing (by serial, which holds its frame) and the drawing's units.
struct DimensionEntry {
    state: Arc<ModelState>,
    record: Arc<Dimension>,
    view_serial: u64,
    units: String,
    drawn: DrawnDimension,
    serial: u64,
}

/// What a scene is assembled from: the sheet's record, each view on it with its entry's serial
/// (0: not drawn yet) and its centre, and each dimension read so far with its entry's serial.
#[derive(PartialEq)]
struct Inputs {
    sheet: Sheet,
    views: Vec<(u64, u64, Vec2)>,
    dimensions: Vec<(u64, u64)>,
}

/// The last answer of [`extract`], for the sheet, drawing, world state, units and serial it was for:
/// the same five again return it as it is.
struct Assembled {
    sheet: u64,
    drawing: Arc<Drawing>,
    state: Arc<ModelState>,
    units: String,
    serial: u64,
    inputs: Inputs,
    answer: SheetView,
}

impl Cache {
    /// Whether view `id`, drawing as `view` now, was last drawn so, from `state`.
    fn fresh(&self, id: u64, state: &Arc<ModelState>, view: &ResolvedView) -> bool {
        self.views.get(&id).is_some_and(|e| Arc::ptr_eq(&e.state, state) && e.view == *view)
    }

    /// Keeps `records` as `lines` drew them, numbered with the cache's serial.
    fn keep_dimensions(&mut self, records: Vec<Arc<Dimension>>, lines: Vec<DrawnDimension>, state: &Arc<ModelState>, view_serial: u64, units: &str) {
        let serial = self.serial;
        for (record, drawn) in records.into_iter().zip(lines) {
            let entry = DimensionEntry { state: state.clone(), record, view_serial, units: units.to_string(), drawn, serial };
            self.dimensions.insert(entry.record.id, entry);
        }
    }

    /// The dimensions of the views `on`, by view and id, to read again: each whose view is up to
    /// date, last drawn from another record, model, drawing of its view, or units.
    fn stale_dimensions(&self, d: &Drawing, on: &[(u64, ResolvedView, Vec2)], state: &Arc<ModelState>, units: &str) -> Vec<Arc<Dimension>> {
        let fresh = on.iter().filter(|(id, view, _)| self.fresh(*id, state, view));
        let serials: BTreeMap<u64, u64> = fresh.filter_map(|(id, ..)| self.views.get(id).map(|e| (*id, e.serial))).collect();
        let current = |record: &Arc<Dimension>, serial: u64| {
            self.dimensions
                .get(&record.id)
                .is_some_and(|e| Arc::ptr_eq(&e.record, record) && Arc::ptr_eq(&e.state, state) && e.view_serial == serial && e.units == units)
        };
        let mut stale = dimensions_on(d, serials.keys().copied());
        stale.retain(|record| serials.get(&record.view).is_some_and(|serial| !current(record, *serial)));
        stale
    }
}

/// Where one view of a drawing stands with the provider ([`standing`]).
#[derive(Clone, Debug, PartialEq)]
pub struct ViewStanding {
    pub view: u64,
    pub sheet: u64,
    /// `current`, `out_of_date` (not drawn yet, or drawn from another model or with other settings
    /// than it has now), `placeholder` or `failed`.
    pub status: &'static str,
    /// Why, for a placeholder or a failed view.
    pub reason: Option<String>,
}

/// Each view of `d`, the drawing open, in id order, as the provider last drew it, whichever sheet it
/// is on. It reads the cache: nothing is drawn. A view that drew but didn't fit its sheet's scene is
/// as the last assembly left it, as long as nothing has been drawn since.
pub fn standing(s: &Session, d: &Drawing) -> Vec<ViewStanding> {
    let state = s.world_state();
    let cache = &s.drawing_scenes;
    let assembled = cache.last.as_ref().filter(|last| last.serial == cache.serial).map_or(&[][..], |last| last.answer.views.as_slice());
    let in_assembly = |id: u64| assembled.binary_search_by_key(&id, |v| v.view).ok().and_then(|at| assembled.get(at));
    let word = |status: &Status| match status {
        Status::Drawn => ("current", None),
        Status::Placeholder(why) => ("placeholder", Some(why.to_string())),
        Status::Failed(why) => ("failed", Some(why.clone())),
    };
    let one = |view: &Arc<View>| {
        let fresh = d.resolve_view(view.id).is_ok_and(|resolved| cache.fresh(view.id, &state, &resolved));
        let (status, reason) = match cache.views.get(&view.id).filter(|_| fresh) {
            Some(entry) => word(in_assembly(view.id).map_or(&entry.drawn.status, |v| &v.status)),
            None => ("out_of_date", None),
        };
        ViewStanding { view: view.id, sheet: view.sheet, status, reason }
    };
    d.views.iter().map(one).collect()
}

/// Sheet `sheet` of the drawing open, as the provider last drew it: at once, as nothing is drawn
/// here. The first call for a sheet gives its border, with every view out of date.
pub fn extract(s: &mut Session, sheet: u64) -> Result<SheetView> {
    forget_closed(s);
    let d = s.drawing(CMD)?.clone();
    let state = s.world_state();
    let units = s.doc.drawing_units(&d).to_string();
    if s.drawing_scenes.drawing != Some(d.id) {
        take_last_lines(s, &d, &state);
    }
    let cache = &mut s.drawing_scenes;
    if let Some(last) = &cache.last
        && last.sheet == sheet
        && last.serial == cache.serial
        && last.units == units
        && Arc::ptr_eq(&last.drawing, &d)
        && Arc::ptr_eq(&last.state, &state)
    {
        return Ok(last.answer.clone());
    }
    let record = sheet_of(&d, sheet)?;
    let on = views_on(&d, sheet)?;
    cache.drawing = Some(d.id);
    cache.sheet = Some(sheet);
    let stale = on.iter().filter(|(id, view, _)| !cache.fresh(*id, &state, view)).map(|(id, ..)| *id).collect();
    let stale_dimensions = cache.stale_dimensions(&d, &on, &state, &units).iter().map(|record| record.id).collect();
    let serial = |id: &u64| cache.views.get(id).map_or(0, |e| e.serial);
    let read = dimensions_on(&d, on.iter().map(|(id, ..)| *id));
    let read: Vec<u64> = read.iter().map(|record| record.id).filter(|id| cache.dimensions.contains_key(id)).collect();
    let inputs = Inputs {
        sheet: record.clone(),
        views: on.iter().map(|(id, _, centre)| (*id, serial(id), *centre)).collect(),
        dimensions: read.iter().map(|id| (*id, cache.dimensions.get(id).map_or(0, |e| e.serial))).collect(),
    };
    let (scene, views, objects, dimensions, generation) = match cache.last.take() {
        Some(last) if last.inputs == inputs => {
            (last.answer.scene, last.answer.views, last.answer.objects, last.answer.dimensions, last.answer.generation)
        }
        _ => {
            // Views and dimensions no longer in the drawing are forgotten: each holds a world state.
            let kept: BTreeSet<u64> = d.views.iter().map(|v| v.id).collect();
            cache.views.retain(|id, _| kept.contains(id));
            cache.dimensions.retain(|id, _| d.dimension(*id).is_some());
            let drawn = on.iter().filter_map(|(id, _, centre)| cache.views.get(id).map(|e| (*id, *centre, &e.drawn)));
            let mut assembly = assemble(record, drawn)?;
            assembly.add_dimensions(read.iter().filter_map(|id| cache.dimensions.get(id).map(|e| &e.drawn)));
            let (scene, views, objects, dimensions) = assembly.finish()?;
            (Arc::new(scene), views, objects.into(), dimensions, GENERATION.fetch_add(1, Ordering::Relaxed).wrapping_add(1))
        }
    };
    let answer = SheetView { scene, generation, views, stale, objects, dimensions, stale_dimensions };
    cache.last = Some(Assembled { sheet, drawing: d, state, units, serial: cache.serial, inputs, answer: answer.clone() });
    Ok(answer)
}

/// Draws the first view out of date on the sheet last extracted or, with none, reads the dimensions
/// out of date of one view, and says whether more are waiting: a short piece of work for the UI's
/// tick, one view's projection or one read of a view's dimensions.
pub fn step(s: &mut Session) -> bool {
    forget_closed(s);
    let (Ok(d), Some(sheet)) = (s.drawing(CMD).cloned(), s.drawing_scenes.sheet) else { return false };
    let Ok(on) = views_on(&d, sheet) else { return false };
    let state = s.world_state();
    let units = s.doc.drawing_units(&d).to_string();
    let waiting: Vec<(u64, ResolvedView)> =
        on.iter().filter(|(id, view, _)| !s.drawing_scenes.fresh(*id, &state, view)).map(|(id, view, _)| (*id, *view)).collect();
    let Some(&(id, view)) = waiting.first() else { return step_dimensions(s, &d, &on, &state, units) };
    let shown = match s.drawing_scenes.shown.take() {
        Some((was, shown)) if Arc::ptr_eq(&was, &state) => shown,
        _ => crate::drawing_view::shown(s),
    };
    let drawn = draw(s, &shown, id, &view);
    let frame = drawn.frame;
    let these = dimensions_on(&d, std::iter::once(id));
    let lines = draw_dimensions(s, &d, &these, |_| frame);
    let cache = &mut s.drawing_scenes;
    cache.shown = Some((state.clone(), shown));
    cache.serial += 1;
    let serial = cache.serial;
    let lines_kept = KeptView::of(&view, &state, &drawn);
    cache.views.insert(id, Entry { state: state.clone(), view, drawn, serial });
    cache.keep_dimensions(these, lines, &state, serial, &units);
    #[cfg(test)]
    {
        cache.projections += 1;
    }
    let more = waiting.len() > 1 || !cache.stale_dimensions(&d, &on, &state, &units).is_empty();
    if let Some(kept) = lines_kept {
        Arc::make_mut(&mut s.kept_lines).0.entry(d.id).or_default().insert(id, kept);
    }
    more
}

/// Reads the dimensions out of date of the first view that has any, through its frame, and says
/// whether another view's are waiting.
fn step_dimensions(s: &mut Session, d: &Drawing, on: &[(u64, ResolvedView, Vec2)], state: &Arc<ModelState>, units: String) -> bool {
    let stale = s.drawing_scenes.stale_dimensions(d, on, state, &units);
    let Some(view) = stale.first().map(|record| record.view) else { return false };
    let (these, others): (Vec<_>, Vec<_>) = stale.into_iter().partition(|record| record.view == view);
    let entry = s.drawing_scenes.views.get(&view);
    let (frame, view_serial) = (entry.and_then(|e| e.drawn.frame), entry.map_or(0, |e| e.serial));
    let lines = draw_dimensions(s, d, &these, |_| frame);
    let cache = &mut s.drawing_scenes;
    cache.serial += 1;
    cache.keep_dimensions(these, lines, state, view_serial, &units);
    !others.is_empty()
}

/// Fills the provider of the drawing `d`, just opened, with the lines its file keeps. Any failure is
/// a miss: each view is then drawn as usual.
fn take_last_lines(s: &mut Session, d: &Drawing, state: &Arc<ModelState>) {
    let Some((path, digest)) = &s.file else { return };
    let Some(read) = solidcraft_io::read_sheet_lines(path, d.id, digest) else { return };
    // They were written for the file's model: an edit since, with the drawing closed, is another.
    let model_same = Arc::ptr_eq(&s.doc, &s.saved) || *s.doc == *s.saved || s.doc.differs_only_in_drawings(&s.saved);
    let current = read.current && model_same;
    let out_of_date: Arc<ModelState> = Arc::default();
    for saved in read.views {
        let Ok(view) = d.resolve_view(saved.view) else { continue };
        let factor = view.scale.value.factor();
        let centre = Vec2 { x: -saved.origin_mm.x / factor, y: -saved.origin_mm.y / factor };
        let Ok(frame) = ViewFrame::new(view.basis, view.scale.value, centre, Vec2::ZERO) else { continue };
        let fresh = current && saved.key == lines_key(&view);
        let cache = &mut s.drawing_scenes;
        cache.serial += 1;
        let drawn = Drawn { group: Some(saved.group), status: Status::Drawn, stats: None, boundary: Some(saved.boundary), frame: Some(frame) };
        let lines_kept = fresh.then(|| KeptView::of(&view, state, &drawn)).flatten();
        let serial = cache.serial;
        cache.views.insert(saved.view, Entry { state: if fresh { state.clone() } else { out_of_date.clone() }, view, drawn, serial });
        if let Some(kept) = lines_kept {
            Arc::make_mut(&mut s.kept_lines).0.entry(d.id).or_default().insert(saved.view, kept);
        }
    }
}

/// Each drawing's last lines, by drawing and view: what a save can keep. A closed drawing's stay here
/// too, as plain data: a weak reference to the world state they came from says whether they are
/// current without holding it.
#[derive(Clone, Default)]
pub struct KeptLines(BTreeMap<u64, BTreeMap<u64, KeptView>>);

#[derive(Clone)]
struct KeptView {
    key: String,
    state: std::sync::Weak<ModelState>,
    group: Arc<Group>,
    boundary: Aabb2,
    origin_mm: Vec2,
}

impl KeptView {
    /// A view's lines as a save keeps them, when it drew any.
    fn of(view: &ResolvedView, state: &Arc<ModelState>, drawn: &Drawn) -> Option<KeptView> {
        let (group, boundary, frame) = (drawn.group.as_ref()?, drawn.boundary?, drawn.frame?);
        let origin_mm = frame.paper_map().apply(Vec2::ZERO);
        Some(KeptView { key: lines_key(view), state: Arc::downgrade(state), group: group.clone(), boundary, origin_mm })
    }
}

/// What a view's lines were drawn from, as a save writes it: compared only within one build, whose
/// lines these are, so its text can be anything that tells two views apart.
fn lines_key(view: &ResolvedView) -> String {
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

/// Drops the cache once its drawing isn't the one open: closing a drawing (or opening another) ends
/// its scenes, and the world states they hold. [`Session::execute`] calls it after every command.
pub(crate) fn forget_closed(s: &mut Session) {
    if s.drawing_scenes.drawing.is_some_and(|d| s.open_drawing != Some(d)) {
        s.drawing_scenes = Cache::default();
    }
}

#[cfg(test)]
#[path = "drawing_scene_tests.rs"]
pub(crate) mod tests;
