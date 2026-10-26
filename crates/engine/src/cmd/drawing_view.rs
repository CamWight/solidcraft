//! `drawing.view.snapshot` (W07, #5866): one orthographic view of the design, worked out and
//! returned without saving anything. It reads the evaluated model and changes nothing.
//!
//! `drawing.view.inspect` and `drawing.view.edit` (W17c, #4816) read and change a saved view of a
//! drawing: each setting with where its value comes from ([`solidcraft_doc::drawing::Drawing::resolve_view`])
//! and where the view sits on its sheet ([`crate::drawing_view::on_sheet`]), and a change as one
//! undo step that evaluates no model.
//!
//! The view's settings are `doc::drawing_view::ViewSettings`; its basis, scale and map onto
//! paper are `geom::drawing_view`'s. Without a scale it takes the largest ISO 5455 scale at
//! which the view, centred where it goes, lies inside the sheet's drawing border.

use serde::Serialize;
use serde_json::Value;
use solidcraft_doc::drawing::Drawing;
use solidcraft_doc::drawing::view::{ResolvedView, SetBy, Setting, View, ViewKind, ViewPlace};
use solidcraft_doc::drawing_view::{ViewDirection, ViewOverrides};
use solidcraft_doc::ids;
use solidcraft_geom::Plane;
use solidcraft_geom::drawing_sheet::{DrawingBorderProfile, Sheet};
use solidcraft_geom::drawing_view::{NamedView, PRESETS, Scale};
use solidcraft_geom::paper::MAX_COORDINATE_MM;
use solidcraft_geom::{Aabb2, Vec2};
use solidcraft_render::VisualStyle;

use super::CommandSpec;
use crate::drawing_view::{Extents, Unmeasured};
use crate::params::bad;
use crate::schema::drawing_view::{
    BoxResult, ChoicesResult, DPI_RANGE, EditParams, EditResult, ExtentsResult, FromResult, InspectParams, InspectResult, PlaceParam, ScaleResult,
    SettingResult, SettingsResult, SnapshotParams, SnapshotResult, SourceResult, ViewResult, ViewSet,
};
use crate::schema::{TypedContract, output_schema_for};
use crate::{Result, Session};

const CMD: &str = "drawing.view.snapshot";
const CONTRACT: TypedContract<SnapshotParams> = TypedContract::new();
const INSPECT: &str = "drawing.view.inspect";
const INSPECT_CONTRACT: TypedContract<InspectParams> = TypedContract::new();
const EDIT: &str = "drawing.view.edit";
const EDIT_CONTRACT: TypedContract<EditParams> = TypedContract::new();

pub static COMMANDS: &[CommandSpec] = &[CommandSpec::new(CMD, "Drawing View Snapshot", crate::checked!("drawing.view.snapshot", CONTRACT, snapshot))
    .icon("export")
    .noundo()
    .contract(&CONTRACT)
    .output(output_schema_for::<SnapshotResult>)
    .params("view?: front (default), back, top, bottom, left, right, iso (= iso_front_right), iso_front_left, iso_back_right, iso_back_left, or {direction, up, origin?}; scale?: \"paper:model\" such as \"1:2\" (default: the largest ISO 5455 scale that fits the sheet's drawing area); position_mm?: [x, y] for the view's centre on the sheet (default the sheet's centre); sheet?: {size: {standard: a3} or {custom: {sides_mm: [w, h]}}, orientation?: landscape|portrait, dpi?: 25..2400} (default A3 landscape, 300 dpi); display_style?, reference_style?: visible (default) or visible_hidden (the shaded styles aren't supported yet); reference_hides_model?: false; reference_margin_mm?: 0..100000 model mm on every side; tangent_edges?: hide (default) or show. Every body the design places, whatever the browser shows. Returns {view, scale: {text, factor, fitted}, position_mm, sheet, extents (model mm on the view's plane), boundary_mm (on the sheet), source}. Read-only: saves nothing."),
    CommandSpec::new(INSPECT, "Inspect Drawing View", crate::checked!("drawing.view.inspect", INSPECT_CONTRACT, inspect))
        .icon("inspect")
        .in_drawing()
        .noundo()
        .doc_only()
        .contract(&INSPECT_CONTRACT)
        .output(output_schema_for::<InspectResult>)
        .params("view: a saved view's id → {view, sheet, kind: base|projected, source, parent?, side?, direction, place: {at: [x, y]} or {aligned: n}, settings: {scale, display_style, reference_style, reference_hides_model, tangent_edges: {value, from: this|parent|default, parent?, set}}, reference_margin_mm, boundary_mm: {min, max} on the sheet (a 20 x 20 mm placeholder round the view's centre when there's nothing to measure), extents: {min, max, error_mm, status: approximate|placeholder, provenance} in model mm on the view's plane, linked? (a projected view's display settings: true, false or \"partly\"), choices: {display_style, scale}}. Read-only; no model is evaluated."),
    CommandSpec::new(EDIT, "Edit Drawing View", crate::checked!("drawing.view.edit", EDIT_CONTRACT, edit))
        .icon("edit")
        .in_drawing()
        .doc_only()
        .contract(&EDIT_CONTRACT)
        .output(output_schema_for::<EditResult>)
        .params("view: a saved view's id; set: {scale?, display_style?, reference_style?, reference_hides_model?, tangent_edges?: each a value, or null to follow the parent (a base view keeps a scale); reference_margin_mm?; view?: a base view's direction; place?: {at: [x, y]} or {aligned: n}; linked?: a projected view's display settings all following its parent (true) or all its own, as they are now (false)} → {view, changed: [fields], followers: [the views projected from it whose settings changed with it]}. One undo step; no model is evaluated."),
];

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

    let extents = crate::drawing_view::measure(s, &basis).map_err(|why| bad(CMD, why.to_string()))?;
    let shown = extents.model.inflated(settings.reference_margin_mm);
    let position = request.position_mm.map_or(paper * 0.5, |p| p.0);
    let (scale, fitted) = match settings.scale {
        Some(scale) => (scale, false),
        None => (fit(shown.size(), position, border)?, true),
    };
    let boundary = crate::drawing_view::boundary(basis, scale, &shown, position, CMD)?;
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
        extents: extents_result(&extents, None),
        boundary_mm: BoxResult { min: boundary.min.into(), max: boundary.max.into() },
        source: SourceResult { bodies: extents.bodies, timeline_at_end: s.doc.marker.is_none() },
        param_warnings: Vec::new(),
    };
    serde_json::to_value(reply).map_err(|e| crate::EngineError::Other(e.to_string()))
}

fn inspect(s: &mut Session, p: &Value) -> Result<Value> {
    let request: InspectParams = crate::schema::read(INSPECT, p)?;
    let d = s.drawing(INSPECT)?;
    let view = find(d, request.view.0, INSPECT)?;
    let resolved = d.resolve_view(view.id).map_err(|e| bad(INSPECT, e.to_string()))?;
    let placed = crate::drawing_view::on_sheet(s, d, view.id, INSPECT)?;
    let (parent, side) = match view.kind {
        ViewKind::Base { .. } => (None, None),
        ViewKind::Projected { parent, side } => (Some(ids::encode(parent)), Some(snake_name(side))),
    };
    let own = view.settings;
    let reply = InspectResult {
        view: ids::encode(view.id),
        sheet: ids::encode(view.sheet),
        kind: if parent.is_some() { "projected" } else { "base" },
        source: ids::encode(resolved.source),
        parent,
        side,
        direction: direction(&resolved.basis),
        place: match view.place {
            ViewPlace::At(at) => PlaceParam::At(at),
            ViewPlace::Aligned(along) => PlaceParam::Aligned(along),
        },
        settings: SettingsResult {
            scale: setting(resolved.scale, own.scale, |s| s.to_string()),
            display_style: setting(resolved.display_style, own.display_style, snake_name),
            reference_style: setting(resolved.reference_style, own.reference_style, snake_name),
            reference_hides_model: setting(resolved.reference_hides_model, own.reference_hides_model, |b| b),
            tangent_edges: setting(resolved.tangent_edges, own.tangent_edges, snake_name),
        },
        reference_margin_mm: view.reference_margin_mm,
        boundary_mm: BoxResult { min: placed.boundary.min.into(), max: placed.boundary.max.into() },
        extents: extents_result(&placed.extents, placed.unmeasured),
        linked: matches!(view.kind, ViewKind::Projected { .. }).then(|| match display_set(&own) {
            0 => Value::Bool(true),
            DISPLAY_GROUP => Value::Bool(false),
            _ => Value::String("partly".into()),
        }),
        choices: ChoicesResult {
            display_style: VisualStyle::ALL.iter().map(|s| snake_name(*s)).collect(),
            scale: PRESETS.iter().map(ToString::to_string).collect(),
        },
        param_warnings: Vec::new(),
    };
    serde_json::to_value(reply).map_err(|e| crate::EngineError::Other(e.to_string()))
}

fn edit(s: &mut Session, p: &Value) -> Result<Value> {
    let request: EditParams = crate::schema::read(EDIT, p)?;
    let id = request.view.0;
    let descendants = descendants(s.drawing(EDIT)?, id);
    let resolve = |d: &Drawing, ids: &[u64]| ids.iter().map(|id| d.resolve_view(*id).ok()).collect::<Vec<Option<ResolvedView>>>();
    let before = resolve(s.drawing(EDIT)?, &descendants);
    let changed = s.edit_drawing(EDIT, |d| change(d, id, &request.set))?;
    let after = resolve(s.drawing(EDIT)?, &descendants);
    let followers = descendants.iter().zip(before.iter().zip(&after)).filter(|(_, (b, a))| b != a).map(|(id, _)| ids::encode(*id)).collect();
    let reply = EditResult { view: ids::encode(id), changed, followers, param_warnings: Vec::new() };
    serde_json::to_value(reply).map_err(|e| crate::EngineError::Other(e.to_string()))
}

/// The display settings a linked view takes from its parent: every inheritable one but the scale,
/// which keeps its own switch.
const DISPLAY_GROUP: usize = 4;

/// How many of the display settings `own` sets itself.
fn display_set(own: &ViewOverrides) -> usize {
    [own.display_style.is_some(), own.reference_style.is_some(), own.reference_hides_model.is_some(), own.tangent_edges.is_some()]
        .into_iter()
        .filter(|set| *set)
        .count()
}

/// View `id` of `d`, for `cmd`.
fn find<'a>(d: &'a Drawing, id: u64, cmd: &str) -> Result<&'a View> {
    d.views.iter().find(|v| v.id == id).map(|v| &**v).ok_or_else(|| bad(cmd, format!("the drawing has no view {}", ids::encode(id))))
}

/// The views projected from view `id`, directly or through others, in the drawing's order.
fn descendants(d: &Drawing, id: u64) -> Vec<u64> {
    let mut found = vec![id];
    // Each pass adds the views whose parent is found; a chain is at most MAX_VIEW_DEPTH views.
    for _ in 0..solidcraft_doc::drawing::view::MAX_VIEW_DEPTH {
        let more: Vec<u64> = d
            .views
            .iter()
            .filter(|v| !found.contains(&v.id) && matches!(v.kind, ViewKind::Projected { parent, .. } if found.contains(&parent)))
            .map(|v| v.id)
            .collect();
        if more.is_empty() {
            break;
        }
        found.extend(more);
    }
    found.retain(|v| *v != id);
    d.views.iter().map(|v| v.id).filter(|v| found.contains(v)).collect()
}

/// `set` made on view `id` of `d`: the fields it changes, by name.
fn change(d: &mut Drawing, id: u64, set: &ViewSet) -> Result<Vec<String>> {
    let resolved = d.resolve_view(id).map_err(|e| bad(EDIT, e.to_string()))?;
    let at = d.views.iter().position(|v| v.id == id).ok_or_else(|| bad(EDIT, format!("the drawing has no view {}", ids::encode(id))))?;
    let Some(view) = d.views.get_mut(at).map(std::sync::Arc::make_mut) else { return Ok(Vec::new()) };
    let before = view.clone();
    let base = matches!(view.kind, ViewKind::Base { .. });
    let own = &mut view.settings;
    if let Some(linked) = set.linked {
        if base {
            return Err(bad(EDIT, "a base view has no parent to follow: `linked` is a projected view's"));
        }
        // Following: the display settings cleared. Its own: each at the value it has now.
        own.display_style = (!linked).then_some(resolved.display_style.value);
        own.reference_style = (!linked).then_some(resolved.reference_style.value);
        own.reference_hides_model = (!linked).then_some(resolved.reference_hides_model.value);
        own.tangent_edges = (!linked).then_some(resolved.tangent_edges.value);
    }
    if let Some(scale) = set.scale {
        if base && scale.is_none() {
            return Err(bad(EDIT, "a base view has no parent to follow: it keeps a scale"));
        }
        own.scale = scale.map(|s| s.0);
    }
    if let Some(style) = set.display_style {
        own.display_style = style.map(|s| s.0);
    }
    if let Some(style) = set.reference_style {
        own.reference_style = style.map(|s| s.0);
    }
    if let Some(hides) = set.reference_hides_model {
        own.reference_hides_model = hides;
    }
    if let Some(edges) = set.tangent_edges {
        own.tangent_edges = edges.map(|e| e.0);
    }
    if let Some(margin) = set.reference_margin_mm {
        view.reference_margin_mm = margin;
    }
    if let Some(direction) = set.view {
        let ViewKind::Base { view: looking, .. } = &mut view.kind else {
            return Err(bad(EDIT, "a projected view looks from its side of its parent: only a base view's direction changes"));
        };
        *looking = direction.0;
    }
    if let Some(place) = set.place {
        view.place = match place {
            PlaceParam::At(at) => ViewPlace::At(at),
            PlaceParam::Aligned(along) => ViewPlace::Aligned(along),
        };
    }
    Ok(changed_fields(&before, view))
}

/// The names of the fields that differ between `a` and `b`, two states of one view.
fn changed_fields(a: &View, b: &View) -> Vec<String> {
    let (x, y) = (&a.settings, &b.settings);
    let fields = [
        ("scale", x.scale != y.scale),
        ("display_style", x.display_style != y.display_style),
        ("reference_style", x.reference_style != y.reference_style),
        ("reference_hides_model", x.reference_hides_model != y.reference_hides_model),
        ("tangent_edges", x.tangent_edges != y.tangent_edges),
        ("reference_margin_mm", a.reference_margin_mm != b.reference_margin_mm),
        ("view", a.kind != b.kind),
        ("place", a.place != b.place),
    ];
    fields.into_iter().filter(|(_, changed)| *changed).map(|(name, _)| name.to_string()).collect()
}

/// Extents as the replies give them: measured on the display mesh, or a placeholder's and why.
fn extents_result(extents: &Extents, unmeasured: Option<Unmeasured>) -> ExtentsResult {
    ExtentsResult {
        min: extents.model.min.into(),
        max: extents.model.max.into(),
        error_mm: extents.error_mm,
        status: if unmeasured.is_some() { "placeholder" } else { "approximate" },
        provenance: unmeasured.map_or("display_mesh", Unmeasured::name),
    }
}

/// A resolved setting as inspect shows it: its value, where it comes from and what the view sets.
fn setting<T: Copy, U>(resolved: Setting<T>, own: Option<T>, show: impl Fn(T) -> U) -> SettingResult<U> {
    let (from, parent) = match resolved.set_by {
        SetBy::This => (FromResult::This, None),
        SetBy::Parent(id) => (FromResult::Parent, Some(ids::encode(id))),
        SetBy::Default => (FromResult::Default, None),
    };
    SettingResult { value: show(resolved.value), from, parent, set: own.map(&show) }
}

/// A basis as the replies give it: its name when it is exactly a named view's.
fn direction(basis: &Plane) -> ViewResult {
    ViewResult {
        named: NamedView::ALL.iter().find(|v| v.basis() == *basis).map(|v| snake_name(*v)),
        // The line of sight, -(x × y), written so no component reads -0.
        direction: basis.y.cross(basis.x).into(),
        x: basis.x.into(),
        y: basis.y.into(),
        origin: basis.origin.into(),
    }
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
