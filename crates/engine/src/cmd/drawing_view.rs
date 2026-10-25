//! `drawing.view.snapshot` (W07, #5866): one orthographic view of the design, worked out and
//! returned without saving anything. It reads the evaluated model and changes nothing.
//!
//! The view's settings are `doc::drawing_view::ViewSettings`; its basis, scale and map onto
//! paper are `geom::drawing_view`'s. Without a scale it takes the largest ISO 5455 scale at
//! which the view, centred where it goes, lies inside the sheet's drawing border.

use serde::Serialize;
use serde_json::Value;
use solidcraft_doc::drawing_view::ViewDirection;
use solidcraft_doc::{MAX_PLACEMENT_DEPTH, MAX_PLACEMENTS};
use solidcraft_geom::drawing_sheet::{DrawingBorderProfile, Sheet};
use solidcraft_geom::drawing_view::{PRESETS, Scale, ViewFrame};
use solidcraft_geom::paper::MAX_COORDINATE_MM;
use solidcraft_geom::{Aabb2, Vec2};

use super::CommandSpec;
use crate::params::bad;
use crate::schema::drawing_view::{BoxResult, DPI_RANGE, ExtentsResult, ScaleResult, SnapshotParams, SnapshotResult, SourceResult, ViewResult};
use crate::schema::{TypedContract, output_schema_for};
use crate::{Result, Session};

const CMD: &str = "drawing.view.snapshot";
const CONTRACT: TypedContract<SnapshotParams> = TypedContract::new();

pub static COMMANDS: &[CommandSpec] = &[CommandSpec::new(CMD, "Drawing View Snapshot", crate::checked!("drawing.view.snapshot", CONTRACT, snapshot))
    .icon("export")
    .noundo()
    .contract(&CONTRACT)
    .output(output_schema_for::<SnapshotResult>)
    .params("view?: front (default), back, top, bottom, left, right, iso (= iso_front_right), iso_front_left, iso_back_right, iso_back_left, or {direction, up, origin?}; scale?: \"paper:model\" such as \"1:2\" (default: the largest ISO 5455 scale that fits the sheet's drawing area); position_mm?: [x, y] for the view's centre on the sheet (default the sheet's centre); sheet?: {size: {standard: a3} or {custom: {sides_mm: [w, h]}}, orientation?: landscape|portrait, dpi?: 25..2400} (default A3 landscape, 300 dpi); display_style?, reference_style?: visible (default) or visible_hidden (the shaded styles aren't supported yet); reference_hides_model?: false; reference_margin_mm?: 0..100000 model mm on every side; tangent_edges?: hide (default) or show. Every body the design places, whatever the browser shows. Returns {view, scale: {text, factor, fitted}, position_mm, sheet, extents (model mm on the view's plane), boundary_mm (on the sheet), source}. Read-only: saves nothing.")];

fn snapshot(s: &mut Session, p: &Value) -> Result<Value> {
    let request: SnapshotParams = crate::schema::read(CMD, p)?;
    let settings = request.settings();
    let basis = settings.validate().map_err(|e| bad(CMD, e.to_string()))?;
    if settings.display_style.is_shaded() || settings.reference_style.is_shaded() {
        return Err(bad(CMD, "not supported yet: shaded views need the drawing renderer (#5624)"));
    }
    if !(request.sheet.dpi.is_finite() && DPI_RANGE.contains(&request.sheet.dpi)) {
        return Err(bad(CMD, format!("sheet.dpi is a number from {} to {}", DPI_RANGE.start(), DPI_RANGE.end())));
    }
    let sheet = Sheet { size: request.sheet.size, orientation: request.sheet.orientation };
    let paper = sheet.size_mm().map_err(|e| bad(CMD, e.to_string()))?;
    let border = DrawingBorderProfile::iso().layout(&sheet).map_err(|e| bad(CMD, e.to_string()))?.drawing_border;

    if !s.doc.placements_complete() {
        let limits = format!("deeper than {MAX_PLACEMENT_DEPTH} levels or has more than {MAX_PLACEMENTS} placements");
        return Err(bad(CMD, format!("the assembly is {limits}: a view of it would leave parts out")));
    }
    let extents = crate::drawing_view::extents(s, &basis);
    if extents.model.is_empty() {
        return Err(bad(CMD, "the view's source has no bodies"));
    }
    let shown = extents.model.inflated(settings.reference_margin_mm);
    let position = request.position_mm.map_or(paper * 0.5, |p| p.0);
    let (scale, fitted) = match settings.scale {
        Some(scale) => (scale, false),
        None => (fit(shown.size(), position, border)?, true),
    };
    let frame = ViewFrame::new(basis, scale, shown.center(), position).map_err(|e| bad(CMD, e.to_string()))?;
    let boundary = frame.boundary(&shown).map_err(|e| bad(CMD, e.to_string()))?;
    let on_paper = [boundary.min, boundary.max].iter().all(|c| c.is_finite() && c.x.abs() <= MAX_COORDINATE_MM && c.y.abs() <= MAX_COORDINATE_MM);
    if !on_paper {
        return Err(bad(CMD, format!("at scale {scale} the view reaches past ±{MAX_COORDINATE_MM} mm on paper: choose a smaller scale")));
    }

    let named = match settings.view {
        ViewDirection::Named(view) => Some(snake_name(view)),
        ViewDirection::Looking(_) => None,
    };
    let reply = SnapshotResult {
        view: ViewResult {
            named,
            // The line of sight, -(x × y), written so no component reads -0.
            direction: basis.y.cross(basis.x).into(),
            x: basis.x.into(),
            y: basis.y.into(),
            origin: basis.origin.into(),
        },
        scale: ScaleResult { text: scale.to_string(), factor: scale.factor(), fitted },
        position_mm: position.into(),
        sheet: request.sheet,
        display_style: snake_name(settings.display_style),
        reference_style: snake_name(settings.reference_style),
        extents: ExtentsResult {
            min: extents.model.min.into(),
            max: extents.model.max.into(),
            error_mm: extents.error_mm,
            status: "approximate",
            provenance: "display_mesh",
        },
        boundary_mm: BoxResult { min: boundary.min.into(), max: boundary.max.into() },
        source: SourceResult { bodies: extents.bodies, timeline_at_end: s.doc.marker.is_none() },
        param_warnings: Vec::new(),
    };
    serde_json::to_value(reply).map_err(|e| crate::EngineError::Other(e.to_string()))
}

/// The largest preset scale at which a view `size` (model mm), centred at `position`, lies inside
/// the drawing border: the room is twice the distance from the position to the nearer side,
/// each way.
fn fit(size: Vec2, position: Vec2, border: Aabb2) -> Result<Scale> {
    let room =
        Vec2::new(2.0 * (position.x - border.min.x).min(border.max.x - position.x), 2.0 * (position.y - border.min.y).min(border.max.y - position.y));
    Scale::fit(size, room).ok_or_else(|| {
        let name = |s: Option<&Scale>| s.map(|s| s.to_string()).unwrap_or_default();
        let presets = format!("from {} to {}", name(PRESETS.first()), name(PRESETS.last()));
        bad(CMD, format!("the view fits the sheet's drawing area at no preset scale {presets} here: give a scale, a position or a larger sheet"))
    })
}

/// A unit enum's name as it reads and writes (`iso_front_right`, `visible_hidden`).
fn snake_name(value: impl Serialize) -> String {
    serde_json::to_value(value).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default()
}

#[cfg(test)]
#[path = "drawing_view_tests.rs"]
mod tests;
