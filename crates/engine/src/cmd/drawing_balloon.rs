//! `drawing.balloon.add`, `.edit` and `.inspect` (W26, #3874): a balloon on a saved view of the
//! open drawing, pointing at a placed part by an edge of it, and where it stands with the model
//! now. The edge is picked as a dimension's is and saved by name
//! ([`solidcraft_doc::Document::edge_reference`]); its path is the part the balloon numbers.
//! Adding and editing are one undo step of the drawing each, and none of the commands evaluates
//! the model.
//!
//! The number it shows is its row's item in the parts list it reads
//! ([`solidcraft_doc::drawing::Drawing::parts_list_for`]), worked out on each read and never saved.
//! One list is made for a read, with each row's members, then one lookup per level of its path.

use std::sync::Arc;

use serde_json::Value;
use solidcraft_doc::drawing::Drawing;
use solidcraft_doc::drawing::balloon::{Balloon, BalloonPlace, Stacked};
use solidcraft_doc::ids;

use super::CommandSpec;
use super::drawing_dimension::{located_result, set_field};
use crate::params::bad;
use crate::schema::drawing_balloon::{AddParams, BalloonResult, EditParams, EditResult, InspectParams, StackAddParams};
use crate::schema::drawing_dimension::EdgeParam;
use crate::schema::{TypedContract, output_schema_for};
use crate::{Result, Session};

const ADD: &str = "drawing.balloon.add";
const ADD_CONTRACT: TypedContract<AddParams> = TypedContract::new();
const STACK_ADD: &str = "drawing.balloon.stack.add";
const STACK_ADD_CONTRACT: TypedContract<StackAddParams> = TypedContract::new();
const EDIT: &str = "drawing.balloon.edit";
const EDIT_CONTRACT: TypedContract<EditParams> = TypedContract::new();
const INSPECT: &str = "drawing.balloon.inspect";
const INSPECT_CONTRACT: TypedContract<InspectParams> = TypedContract::new();

pub static COMMANDS: &[CommandSpec] = &[
    CommandSpec::new(ADD, "Add Balloon", crate::checked!("drawing.balloon.add", add))
        .icon("new")
        .in_drawing()
        .doc_only()
        .contract(&ADD_CONTRACT)
        .output(output_schema_for::<BalloonResult>)
        .params(
            "view: a saved view's id; edge: the edge its arrow points at, a point on it [x, y, z] in the design's frame, {point} or \
             {body, index}, with path: the occurrences its body is placed through, from the top (the part the balloon numbers; at \
             least one); at: [x, y], the balloon's centre on paper in mm from the view's centre; parts_list?: the table it reads \
             (default its sheet's only parts list, else the drawing's) → what drawing.balloon.inspect \
             answers for it. Saved by the edge's name, never the point picked. One undo step; no model is evaluated.",
        ),
    CommandSpec::new(STACK_ADD, "Stack Balloon", crate::checked!("drawing.balloon.stack.add", stack_add))
        .icon("new")
        .in_drawing()
        .doc_only()
        .contract(&STACK_ADD_CONTRACT)
        .output(output_schema_for::<BalloonResult>)
        .params(
            "on: the balloon it stands on and touches; edge: as drawing.balloon.add's, the part it numbers; angle?: the way to it from that \
             balloon, in radians from the paper's +x (default 0) → what drawing.balloon.inspect answers for it. On that balloon's view and \
             reading its list. Only a stack's first draws its arrow. At most 64 in a stack. One undo step; no model is evaluated.",
        ),
    CommandSpec::new(EDIT, "Edit Balloon", crate::checked!("drawing.balloon.edit", edit))
        .icon("edit")
        .in_drawing()
        .doc_only()
        .contract(&EDIT_CONTRACT)
        .output(output_schema_for::<EditResult>)
        .params(
            "balloon: its id; set: {at?: [x, y]: its own place, out of any stack, angle?: radians, the way to it from the balloon it stands on, parts_list?: a table's id, or null to read the only list} → {balloon, changed: [fields]}. What it points at stays: to number another part, add \
             a new one. One undo step, none when nothing changes; no model is evaluated.",
        ),
    CommandSpec::new(INSPECT, "Inspect Balloon", crate::checked!("drawing.balloon.inspect", inspect))
        .icon("inspect")
        .in_drawing()
        .noundo()
        .doc_only()
        .contract(&INSPECT_CONTRACT)
        .output(output_schema_for::<BalloonResult>)
        .params(
            "balloon: its id → {balloon, view, record: as saved, occurrence: the part it numbers, status: ok, ambiguous, suppressed or \
             lost, why?, point?: where its arrow ends (its edge's middle, in the design's frame), exact, candidates?: [{edge: {body, \
             index}}], parts_list?: the list it reads, choices?: with none to go by, those it could name, item?: its row's item (\"1.2\"), \
             row_status: ok, no_list, reference, no_row or not_found, stack?: its stack's balloons, warnings?}. A lost row never removes it. Read-only; no model is \
             evaluated.",
        ),
];

fn add(s: &mut Session, p: &Value) -> Result<Value> {
    let request: AddParams = crate::schema::read(ADD, p)?;
    let record = s.drawing(ADD)?.clone();
    let view = request.view.0;
    record.view(view).ok_or_else(|| bad(ADD, format!("the drawing has no view {}", ids::encode(view))))?;
    placed(s, ADD, &record, view, &request.edge, request.at.into(), request.parts_list.map(|list| list.0))
}

fn stack_add(s: &mut Session, p: &Value) -> Result<Value> {
    let request: StackAddParams = crate::schema::read(STACK_ADD, p)?;
    let record = s.drawing(STACK_ADD)?.clone();
    let on = request.on.0;
    let under = record.balloons.by_id(on).ok_or_else(|| bad(STACK_ADD, format!("the drawing has no balloon {}", ids::encode(on))))?;
    let at = BalloonPlace::On(Stacked { on, angle: request.angle.unwrap_or(0.0) });
    placed(s, STACK_ADD, &record, under.view, &request.edge, at, under.parts_list)
}

/// A new balloon of `record` on `view`, pointing at `edge`, one undo step.
fn placed(s: &mut Session, cmd: &str, record: &Drawing, view: u64, edge: &EdgeParam, at: BalloonPlace, parts_list: Option<u64>) -> Result<Value> {
    let from = s.drawn_by_view(record, view).map_err(|why| bad(cmd, why.to_string()))?;
    let edge = from.doc.edge_reference(&from.model, &edge.pick, &edge.path).map_err(|why| bad(cmd, why.to_string()))?;
    let id = record.allocate_id(&mut s.ids).map_err(|e| bad(cmd, e.to_string()))?;
    let balloon = Balloon { id, view, edge, at, parts_list };
    s.edit_drawing(cmd, |d| {
        d.balloons.add([Arc::new(balloon)]);
        Ok(())
    })?;
    answer(s, cmd, id)
}

fn edit(s: &mut Session, p: &Value) -> Result<Value> {
    let request: EditParams = crate::schema::read(EDIT, p)?;
    let id = request.balloon.0;
    let changed = s.edit_drawing(EDIT, |d| {
        let at = d.balloons.index_of(id).ok_or_else(|| bad(EDIT, format!("the drawing has no balloon {}", ids::encode(id))))?;
        let Some(balloon) = d.balloons.get_mut(at).map(Arc::make_mut) else { return Ok(Vec::new()) };
        let mut changed = Vec::new();
        set_field(&mut balloon.at, request.set.at.map(BalloonPlace::from), "at", &mut changed);
        if let Some(angle) = request.set.angle {
            let BalloonPlace::On(stacked) = &mut balloon.at else { return Err(bad(EDIT, "a balloon at its own place has no angle")) };
            set_field(&mut stacked.angle, Some(angle), "angle", &mut changed);
        }
        set_field(&mut balloon.parts_list, request.set.parts_list.map(|list| list.map(|id| id.0)), "parts_list", &mut changed);
        Ok(changed)
    })?;
    let reply = EditResult { balloon: ids::encode(id), changed, param_warnings: Vec::new() };
    crate::schema::reply(reply)
}

fn inspect(s: &mut Session, p: &Value) -> Result<Value> {
    let request: InspectParams = crate::schema::read(INSPECT, p)?;
    answer(s, INSPECT, request.balloon.0)
}

/// Balloon `id` of the open drawing as the model has it now.
fn answer(s: &Session, cmd: &str, id: u64) -> Result<Value> {
    let d = s.drawing(cmd)?;
    let balloon = d.balloons.by_id(id).ok_or_else(|| bad(cmd, format!("the drawing has no balloon {}", ids::encode(id))))?;
    let located = crate::drawing_sources::only(crate::drawing_sources::resolve_balloons(s, d, &[id]), cmd, "balloon", id)?;
    let row = crate::drawing_sources::balloon_rows(s, d, &[balloon]).pop().unwrap_or_default();
    let stack = d.stack_of(balloon);
    let stack = if stack.len() > 1 { stack.into_iter().map(ids::encode).collect() } else { Vec::new() };
    let warnings = if row.status == "reference" { vec!["it points at a reference, which no parts list counts".to_string()] } else { Vec::new() };
    let reply = BalloonResult {
        balloon: ids::encode(id),
        view: ids::encode(balloon.view),
        record: crate::schema::reply(balloon)?,
        occurrence: balloon.occurrence().to_vec(),
        located: located_result(located),
        parts_list: row.list.map(ids::encode),
        choices: row.choices.into_iter().map(ids::encode).collect(),
        item: row.item,
        row_status: row.status.to_string(),
        stack,
        warnings,
        param_warnings: Vec::new(),
    };
    crate::schema::reply(reply)
}

#[cfg(test)]
#[path = "drawing_balloon_tests.rs"]
mod tests;
