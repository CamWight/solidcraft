//! `drawing.dimension.add`, `.edit` and `.inspect` (W19d, #4896): a dimension on a saved view of
//! the open drawing, its settings, and what it reads now. What a dimension measures is picked (a
//! point, or an edge's index) and saved by name, never by the point picked
//! ([`solidcraft_doc::Document::edge_reference`]); its value comes from the model each time it is
//! read ([`solidcraft_doc::Document::resolve_dimensions`]). Adding and editing are one undo step
//! of the drawing each, and none of the commands evaluates the model.

use std::sync::Arc;

use serde_json::Value;
use solidcraft_doc::drawing::Drawing;
use solidcraft_doc::drawing::dimension::{Candidate, Dimension, DimensionKind, Status, TextPlace};
use solidcraft_doc::ids;
use solidcraft_geom::drawing_measure::{LinearDirection, Reading};

use super::CommandSpec;
use crate::params::bad;
use crate::schema::drawing_dimension::{
    AddParams, CandidateResult, CornerParam, DimensionResult, DimensionSet, EdgeParam, EditParams, EditResult, InspectParams, KindParam,
};
use crate::schema::{TypedContract, output_schema_for};
use crate::{EngineError, Result, Session};

const ADD: &str = "drawing.dimension.add";
const ADD_CONTRACT: TypedContract<AddParams> = TypedContract::new();
const EDIT: &str = "drawing.dimension.edit";
const EDIT_CONTRACT: TypedContract<EditParams> = TypedContract::new();
const INSPECT: &str = "drawing.dimension.inspect";
const INSPECT_CONTRACT: TypedContract<InspectParams> = TypedContract::new();

/// How far a new dimension's line is from what it measures, on paper, until a dimension style
/// gives it (D1).
const DEFAULT_OFFSET_MM: f64 = 10.0;
/// How many decimals a new dimension shows unless told.
const DEFAULT_DECIMALS: u8 = 2;

pub static COMMANDS: &[CommandSpec] = &[
    CommandSpec::new(ADD, "Add Drawing Dimension", crate::checked!("drawing.dimension.add", ADD_CONTRACT, add))
        .icon("dimension")
        .in_drawing()
        .doc_only()
        .contract(&ADD_CONTRACT)
        .output(output_schema_for::<DimensionResult>)
        .params(
            "view: a saved view's id; kind: {linear: {a, b: corners, direction?: aligned (default), horizontal, vertical or {along: [x, y]}, \
             reading?: projected (default) or true}}, {length: {edge, direction?, reading?}} or {angle: {a, b: straight edges, reading?, \
             other_side?}}; an edge is a point on it [x, y, z] in the design's frame, {point} or {body, index}, and a corner a point, \
             {point} or {edges: [2 to 16 edges]}, each with path?: the occurrences its body is placed through, from the top; offset_mm?: \
             the dimension line's distance on paper (default 10); text?: centred (default) or {at: [x, y]} from the view's centre; \
             decimals?: 0..6 (default 2); override_text? → what drawing.dimension.inspect answers for it. Saved by the edges' names, \
             never the points picked. One undo step; no model is evaluated.",
        ),
    CommandSpec::new(EDIT, "Edit Drawing Dimension", crate::checked!("drawing.dimension.edit", EDIT_CONTRACT, edit))
        .icon("dimension")
        .in_drawing()
        .doc_only()
        .contract(&EDIT_CONTRACT)
        .output(output_schema_for::<EditResult>)
        .params(
            "dimension: its id; set: {offset_mm?, text?: centred or {at: [x, y]}, decimals?: 0..6, override_text?: text, or null to show \
             the number again; direction?, reading?: a linear or length dimension's, reading? and other_side?: an angle's} → \
             {dimension, changed: [fields]}. What it measures stays: to measure something else, add a new one. One undo step, none \
             when nothing changes; no model is evaluated.",
        ),
    CommandSpec::new(INSPECT, "Inspect Drawing Dimension", crate::checked!("drawing.dimension.inspect", INSPECT_CONTRACT, inspect))
        .icon("inspect")
        .in_drawing()
        .noundo()
        .doc_only()
        .contract(&INSPECT_CONTRACT)
        .output(output_schema_for::<DimensionResult>)
        .params(
            "dimension: its id → {dimension, view, record: as saved, status: ok, overridden, ambiguous, suppressed, lost or unmeasurable, \
             why?, value? (model mm, or degrees for an angle), foreshortened?, text?, last_good? (the value it last read this session, \
             when it has none now), exact, candidates?: [{edge: {body, index}} or {corner: [x, y, z]}]}. Read-only; no model is \
             evaluated.",
        ),
];

fn add(s: &mut Session, p: &Value) -> Result<Value> {
    let request: AddParams = crate::schema::read(ADD, p)?;
    let record = s.drawing(ADD)?.clone();
    let view = request.view.0;
    record.view(view).ok_or_else(|| bad(ADD, format!("the drawing has no view {}", ids::encode(view))))?;
    let (doc, model) = (&s.doc, &s.model);
    let edge = |e: &EdgeParam| doc.edge_reference(model, &e.pick, &e.path).map_err(|why| bad(ADD, why.to_string()));
    let corner = |c: &CornerParam| doc.corner_reference(model, &c.pick, &c.path).map_err(|why| bad(ADD, why.to_string()));
    let kind = match request.kind {
        KindParam::Linear { a, b, direction, reading } => DimensionKind::Linear {
            a: corner(&a)?,
            b: corner(&b)?,
            direction: direction.unwrap_or(LinearDirection::Aligned),
            reading: reading.unwrap_or(Reading::Projected),
        },
        KindParam::Length { edge: e, direction, reading } => DimensionKind::Length {
            edge: edge(&e)?,
            direction: direction.unwrap_or(LinearDirection::Aligned),
            reading: reading.unwrap_or(Reading::Projected),
        },
        KindParam::Angle { a, b, reading, other_side } => {
            DimensionKind::Angle { a: edge(&a)?, b: edge(&b)?, reading: reading.unwrap_or(Reading::Projected), other_side }
        }
    };
    let id = record.allocate_id(&mut s.ids).map_err(|e| bad(ADD, e.to_string()))?;
    let dimension = Dimension {
        id,
        view,
        kind,
        offset_mm: request.offset_mm.unwrap_or(DEFAULT_OFFSET_MM),
        text: request.text.unwrap_or(TextPlace::Centred),
        decimals: request.decimals.unwrap_or(DEFAULT_DECIMALS),
        override_text: request.override_text,
    };
    s.edit_drawing(ADD, |d| {
        d.add_dimensions([Arc::new(dimension)]);
        Ok(())
    })?;
    answer(s, ADD, id)
}

fn edit(s: &mut Session, p: &Value) -> Result<Value> {
    let request: EditParams = crate::schema::read(EDIT, p)?;
    let id = request.dimension.0;
    let changed = s.edit_drawing(EDIT, |d| change(d, id, &request.set))?;
    let reply = EditResult { dimension: ids::encode(id), changed, param_warnings: Vec::new() };
    serde_json::to_value(reply).map_err(|e| EngineError::Other(e.to_string()))
}

/// `set` made on dimension `id` of `d`: the fields it changes, by name.
fn change(d: &mut Drawing, id: u64, set: &DimensionSet) -> Result<Vec<String>> {
    let at = d.dimension_index(id).ok_or_else(|| bad(EDIT, format!("the drawing has no dimension {}", ids::encode(id))))?;
    let Some(dimension) = d.dimensions.get_mut(at).map(Arc::make_mut) else { return Ok(Vec::new()) };
    let before = dimension.clone();
    if let Some(offset) = set.offset_mm {
        dimension.offset_mm = offset;
    }
    if let Some(text) = set.text {
        dimension.text = text;
    }
    if let Some(decimals) = set.decimals {
        dimension.decimals = decimals;
    }
    if let Some(text) = &set.override_text {
        dimension.override_text = text.clone();
    }
    match &mut dimension.kind {
        DimensionKind::Linear { direction, reading, .. } | DimensionKind::Length { direction, reading, .. } => {
            if set.other_side.is_some() {
                return Err(bad(EDIT, "`other_side` is an angle's"));
            }
            *direction = set.direction.unwrap_or(*direction);
            *reading = set.reading.unwrap_or(*reading);
        }
        DimensionKind::Angle { reading, other_side, .. } => {
            if set.direction.is_some() {
                return Err(bad(EDIT, "an angle has no `direction`: it measures between its edges' lines"));
            }
            *reading = set.reading.unwrap_or(*reading);
            *other_side = set.other_side.unwrap_or(*other_side);
        }
    }
    Ok(changed_fields(&before, dimension))
}

/// The direction, reading and side of a dimension's kind, where it has them.
fn kind_settings(kind: &DimensionKind) -> (Option<LinearDirection>, Reading, bool) {
    match kind {
        DimensionKind::Linear { direction, reading, .. } | DimensionKind::Length { direction, reading, .. } => (Some(*direction), *reading, false),
        DimensionKind::Angle { reading, other_side, .. } => (None, *reading, *other_side),
    }
}

/// The names of the fields that differ between `a` and `b`, two states of one dimension.
fn changed_fields(a: &Dimension, b: &Dimension) -> Vec<String> {
    let (x, y) = (kind_settings(&a.kind), kind_settings(&b.kind));
    let fields = [
        ("offset_mm", a.offset_mm != b.offset_mm),
        ("text", a.text != b.text),
        ("decimals", a.decimals != b.decimals),
        ("override_text", a.override_text != b.override_text),
        ("direction", x.0 != y.0),
        ("reading", x.1 != y.1),
        ("other_side", x.2 != y.2),
    ];
    fields.into_iter().filter(|(_, changed)| *changed).map(|(name, _)| name.to_string()).collect()
}

fn inspect(s: &mut Session, p: &Value) -> Result<Value> {
    let request: InspectParams = crate::schema::read(INSPECT, p)?;
    answer(s, INSPECT, request.dimension.0)
}

/// Dimension `id` of the open drawing as the model has it now. A value read is kept as its last
/// good one ([`Session::dimension_values`]).
fn answer(s: &mut Session, cmd: &str, id: u64) -> Result<Value> {
    let d = s.drawing(cmd)?.clone();
    let dimension = d.dimension(id).ok_or_else(|| bad(cmd, format!("the drawing has no dimension {}", ids::encode(id))))?;
    let resolved = s.doc.resolve_dimensions(&s.model, &d, &[id]).map_err(|e| bad(cmd, e.to_string()))?;
    let r = resolved.into_iter().next().ok_or_else(|| bad(cmd, format!("dimension {} wasn't read", ids::encode(id))))?;
    let last_good = match r.measured {
        Some(m) => {
            s.dimension_values.insert((d.id, id), m.value);
            None
        }
        None => s.dimension_values.get(&(d.id, id)).copied(),
    };
    let (status, why, candidates) = match r.status {
        Status::Ok => ("ok", None, Vec::new()),
        Status::Overridden => ("overridden", None, Vec::new()),
        Status::Ambiguous(found) => ("ambiguous", None, found.into_iter().map(candidate).collect()),
        Status::Suppressed => ("suppressed", None, Vec::new()),
        Status::Lost(why) => ("lost", Some(why), Vec::new()),
        Status::Unmeasurable(why) => ("unmeasurable", Some(why), Vec::new()),
    };
    let angle = matches!(dimension.kind, DimensionKind::Angle { .. });
    let shown = |value: f64| if angle { value.to_degrees() } else { value };
    let reply = DimensionResult {
        dimension: ids::encode(id),
        view: ids::encode(dimension.view),
        record: serde_json::to_value(dimension).map_err(|e| EngineError::Other(e.to_string()))?,
        status: status.to_string(),
        why,
        value: r.measured.map(|m| shown(m.value)),
        foreshortened: r.measured.map(|m| m.foreshortened),
        text: r.text,
        last_good: last_good.map(shown),
        exact: r.exact,
        candidates,
        param_warnings: Vec::new(),
    };
    serde_json::to_value(reply).map_err(|e| EngineError::Other(e.to_string()))
}

fn candidate(c: Candidate) -> CandidateResult {
    match c {
        Candidate::Edge { body, index } => CandidateResult::Edge { body, index },
        Candidate::Corner(p) => CandidateResult::Corner([p.x, p.y, p.z]),
    }
}

#[cfg(test)]
#[path = "drawing_dimension_tests.rs"]
mod tests;
