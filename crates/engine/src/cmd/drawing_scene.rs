//! `drawing.scene` (#4890 W10): one sheet of the open drawing as one paper scene, drawn at once
//! ([`crate::drawing_scene::build`]): its drawing border, then each view on it by id. The sheet
//! page draws the same scene through the provider, and the drawing export (A7) writes it.
//!
//! `drawing.scene.hit` (#4816 E7b) says what is under a point of a sheet, by the one rule the sheet
//! page's hover uses ([`crate::drawing_scene::hit`]).
//!
//! `drawing.scene.line` (#4816 E7c) says which line of a view is under a point, by the one rule the
//! sheet page's press uses ([`crate::drawing_scene::line_at`]).
//!
//! `drawing.status` (#4894 WE2) says where the drawing open stands: each view as the provider last
//! drew it, and each dimension as the model has it now, in one read. `drawing.update` (#4892) lets a
//! deferred drawing draw from the model as it is now.

use serde_json::Value;
use solidcraft_doc::ids;
use solidcraft_geom::paper::ObjectKind;

use super::CommandSpec;
use crate::drawing_scene::{AnnotationState, CMD};
use crate::schema::drawing_scene::{
    DimensionStatusResult, DrawingStatusParams, DrawingStatusResult, DrawingUpdateParams, DrawingUpdateResult, HitResult, LineHitResult,
    SheetHitParams, SheetHitResult, SheetLineParams, SheetLineResult, SheetPaperScene, SheetSceneBalloon, SheetSceneDimension, SheetSceneParams,
    SheetSceneResult, ViewStatusResult,
};
use crate::schema::{TypedContract, output_schema_for};
use crate::{Result, Session};

const CONTRACT: TypedContract<SheetSceneParams> = TypedContract::new();
const HIT: &str = "drawing.scene.hit";
const HIT_CONTRACT: TypedContract<SheetHitParams> = TypedContract::new();
const LINE: &str = "drawing.scene.line";
const LINE_CONTRACT: TypedContract<SheetLineParams> = TypedContract::new();
const STATUS: &str = "drawing.status";
const STATUS_CONTRACT: TypedContract<DrawingStatusParams> = TypedContract::new();
const UPDATE: &str = "drawing.update";
const UPDATE_CONTRACT: TypedContract<DrawingUpdateParams> = TypedContract::new();

pub static COMMANDS: &[CommandSpec] = &[CommandSpec::new(STATUS, "Drawing Status", crate::checked!("drawing.status", status))
    .icon("inspect")
    .in_drawing()
    .noundo()
    .doc_only()
    .contract(&STATUS_CONTRACT)
    .output(output_schema_for::<DrawingStatusResult>)
    .params("→ {views: [{view, sheet, status: current|out_of_date|placeholder|failed, reason?}], dimensions: [{dimension, status: ok|overridden|ambiguous|suppressed|lost|unmeasurable, why?}], waiting: how many views are out of date, held?: true when a deferred drawing's wait for drawing.update}. Read-only: nothing is drawn, and each body a dimension names is read once for the call."),
    CommandSpec::new(UPDATE, "Update Drawing", crate::checked!("drawing.update", update))
        .at("DRAWING REVIEW", "CHANGES")
        .icon("update")
        .in_drawing()
        .noundo()
        .doc_only()
        .contract(&UPDATE_CONTRACT)
        .output(output_schema_for::<DrawingUpdateResult>)
        .params("→ {waiting}: a deferred drawing's views may draw from the model as it is now, until that moves on; waiting: how many are out of date until they are drawn. Nothing is drawn here: the sheet page draws them, as drawing.scene or an export of their sheet does"),
    CommandSpec::new(HIT, "What Is Under a Point", crate::checked!("drawing.scene.hit", hit))
        .icon("inspect")
        .in_drawing()
        .noundo()
        .doc_only()
        .contract(&HIT_CONTRACT)
        .output(output_schema_for::<SheetHitResult>)
        .params("sheet: id or name; point_mm: [x, y] on paper; band_mm?: how near a box's edge counts as on it (default 0) → {hit: {object, kind: view|dimension, on_edge} or null}: the smallest box round a view or a dimension that holds the point within the band, then the topmost. Read-only; no model is evaluated."),
    CommandSpec::new(LINE, "Which Line Is Under a Point", crate::checked!("drawing.scene.line", line))
        .icon("inspect")
        .in_drawing()
        .noundo()
        .doc_only()
        .contract(&LINE_CONTRACT)
        .output(output_schema_for::<SheetLineResult>)
        .params("sheet: id or name; point_mm: [x, y] on paper; band_mm?: how near a line counts as under the point (default 1) → {line: {view, occurrence: [the occurrences that place its body], body, reference, source: {kind: edge, edge, tangent} or {kind: silhouette, face}, distance_mm} or null}: in the view under the point, smallest boundary first, the nearest of its drawn lines within the band, the one painted last when two are as near. Draws the sheet's views first if they aren't; evaluates nothing."),
    CommandSpec::new(CMD, "Draw Sheet", crate::checked!("drawing.scene", scene))
    .icon("export")
    .in_drawing()
    .noundo()
    .doc_only()
    .contract(&CONTRACT)
    .output(output_schema_for::<SheetSceneResult>)
    .params("sheet: id or name → {sheet, scene: {size_mm, groups: [{object, kind: drawing_border|view|dimension|balloon, items: [{kind: stroke, path, closed, width_mm}, {kind: fill, region} or {kind: text, font, scale, origin, angle, slant, glyphs, text, clusters, rtl}]}], fonts?: [the fonts its text names, by id, with their glyphs and never their bytes]} in paint order: the sheet's drawing border (object: the sheet), then each view on it by id (object: the view; its visible lines only for now), then the dimensions of those views by view and id (object: the dimension; lines, arrowheads and text, in paper millimetres at any scale), and balloons (object: the balloon; a circle, a leader and its number), views: [{view, status: drawn|placeholder|failed, reason?, stats?}], objects: [{object, kind, bounds_mm: {min, max}, boundary_mm?}], where the border, each view and each dimension sit on paper, dimensions: [{dimension, view, status: ok|overridden|ambiguous|suppressed|lost|unmeasurable, drawn, why?}], balloons: [{balloon, view, status, drawn, why?}]}. A view with nothing to measure, or one that can't be drawn yet, is left out and says why, and so is a dimension that isn't found again or whose view isn't drawn; one whose text the drawing font can't set keeps its lines and says why. Read-only; no model is evaluated. A deferred drawing draws only what drawing.update allows; out_of_date lists the views and annotations it shows as they last were.")];

fn status(s: &mut Session, p: &Value) -> Result<Value> {
    let _: DrawingStatusParams = crate::schema::read(STATUS, p)?;
    let d = s.drawing(STATUS)?.clone();
    let views: Vec<ViewStatusResult> = crate::drawing_scene::standing(s, &d)
        .into_iter()
        .map(|v| ViewStatusResult { view: ids::encode(v.view), sheet: ids::encode(v.sheet), status: v.status, reason: v.reason })
        .collect();
    let wanted: Vec<u64> = d.dimensions.iter().map(|dimension| dimension.id).collect();
    let read = crate::drawing_sources::resolve_dimensions(s, &d, &wanted).map_err(|e| crate::params::bad(STATUS, e.to_string()))?;
    let dimensions = wanted
        .iter()
        .zip(read)
        .map(|(id, r)| {
            let (status, why, _) = super::drawing_dimension::status_words(r.status);
            DimensionStatusResult { dimension: ids::encode(*id), status, why }
        })
        .collect();
    let waiting = views.iter().filter(|v| v.status == "out_of_date").count();
    let held = waiting > 0 && crate::drawing_scene::held(s, &d);
    let reply = DrawingStatusResult { views, dimensions, waiting, held, param_warnings: Vec::new() };
    crate::schema::reply(reply)
}

fn update(s: &mut Session, p: &Value) -> Result<Value> {
    let _: DrawingUpdateParams = crate::schema::read(UPDATE, p)?;
    let d = s.drawing(UPDATE)?.clone();
    crate::drawing_scene::update(s, &d);
    let waiting = crate::drawing_scene::standing(s, &d).iter().filter(|v| v.status == "out_of_date").count();
    crate::schema::reply(DrawingUpdateResult { waiting, param_warnings: Vec::new() })
}

fn hit(s: &mut Session, p: &Value) -> Result<Value> {
    let request: SheetHitParams = crate::schema::read(HIT, p)?;
    if !request.band_mm.is_finite() || request.band_mm < 0.0 {
        return Err(crate::params::bad(HIT, "band_mm is a finite number, 0 or more"));
    }
    let d = s.drawing(HIT)?.clone();
    let sheet = crate::drawing_scene::sheet_id(&d, &request.sheet, HIT)?;
    let built = crate::drawing_scene::as_shown(s, &d, sheet)?;
    let found = crate::drawing_scene::hit(&built.objects, request.point_mm.0, request.band_mm);
    let hit = found.map(|h| HitResult { object: ids::encode(h.object), kind: h.kind, on_edge: h.on_edge });
    crate::schema::reply(SheetHitResult { hit, param_warnings: Vec::new() })
}

fn line(s: &mut Session, p: &Value) -> Result<Value> {
    let request: SheetLineParams = crate::schema::read(LINE, p)?;
    if !request.band_mm.is_finite() || request.band_mm < 0.0 {
        return Err(crate::params::bad(LINE, "band_mm is a finite number, 0 or more"));
    }
    let d = s.drawing(LINE)?.clone();
    let sheet = crate::drawing_scene::sheet_id(&d, &request.sheet, LINE)?;
    crate::drawing_scene::extract(s, sheet)?;
    crate::drawing_scene::settle(s, &d);
    let found = crate::drawing_scene::line_at(s, sheet, request.point_mm.0, request.band_mm)?;
    let line = found.map(|l| LineHitResult {
        view: ids::encode(l.view),
        occurrence: l.occurrence,
        body: l.body,
        reference: l.reference,
        source: l.source.into(),
        distance_mm: l.distance_mm,
    });
    crate::schema::reply(SheetLineResult { line, param_warnings: Vec::new() })
}

/// Each dimension of a scene in the words of a reply.
pub(super) fn dimension_results(states: &[AnnotationState]) -> Vec<SheetSceneDimension> {
    let of_kind = states.iter().filter(|state| state.kind == ObjectKind::Dimension);
    of_kind.filter_map(words).map(|(dimension, view, status, drawn, why)| SheetSceneDimension { dimension, view, status, drawn, why }).collect()
}

/// Each balloon of a scene in the words of a reply.
pub(super) fn balloon_results(states: &[AnnotationState]) -> Vec<SheetSceneBalloon> {
    let of_kind = states.iter().filter(|state| state.kind == ObjectKind::Balloon);
    of_kind.filter_map(words).map(|(balloon, view, status, drawn, why)| SheetSceneBalloon { balloon, view, status, drawn, why }).collect()
}

/// What a reply says of an annotation on a view: its id, its view, where it stands, whether it is
/// drawn, and why not all of it is.
fn words(state: &AnnotationState) -> Option<(String, String, &'static str, bool, Option<String>)> {
    let (status, why, _) = super::drawing_dimension::status_words(state.status.clone());
    Some((ids::encode(state.object), ids::encode(state.view?), status, state.drawn, why.or_else(|| state.why.clone())))
}

fn scene(s: &mut Session, p: &Value) -> Result<Value> {
    let request: SheetSceneParams = crate::schema::read(CMD, p)?;
    let d = s.drawing(CMD)?.clone();
    let sheet = crate::drawing_scene::sheet_id(&d, &request.sheet, CMD)?;
    let built = crate::drawing_scene::as_shown(s, &d, sheet)?;
    let out_of_date = built.out_of_date.iter().map(|id| ids::encode(*id)).collect();
    let views = built.views.into_iter().map(Into::into).collect();
    let objects = built.objects.into_iter().map(Into::into).collect();
    let (dimensions, balloons) = (dimension_results(&built.annotations), balloon_results(&built.annotations));
    let reply = SheetSceneResult {
        sheet: ids::encode(sheet),
        scene: SheetPaperScene(built.scene),
        views,
        objects,
        dimensions,
        balloons,
        out_of_date,
        param_warnings: Vec::new(),
    };
    crate::schema::reply(reply)
}
