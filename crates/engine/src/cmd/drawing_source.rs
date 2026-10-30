//! `drawing.source.execute` (#3890): a model command run with a drawing open, which stays open.
//! The caller's command owns the undo step and the evaluation ([`CommandSpec::run_nested`]), so it is
//! one step of the model and one evaluation, and costs what the model command costs. Only a model
//! command runs here, so files, checkpoints, undo and `drawing.*` need no list.
//!
//! A drawing's undo passes that step, and says so; `edit.undo {model: true}` takes it back from here.
//! A command that would leave a sketch open or a move uncaptured waits for the drawing to close: both
//! are the model's own modes, with nothing here to finish them. A linked design is read-only from a
//! drawing, so only the design the drawing is in takes one.
//!
//! `drawing.sources` lists where each source stands, as `component.links` lists links. `drawing.source.add`
//! and `.relink` read the new file before they change anything, and are one undo step of the drawing.
//! `.locate` finds a source's file for this session only, so it changes nothing to save. `.remove` is
//! refused while anything uses the source ([`solidcraft_doc::drawing::Drawing::source_users`]).

use std::path::Path;

use serde_json::Value;
use solidcraft_doc::drawing::{Drawing, MAX_SOURCE_PATH_BYTES, Source};
use solidcraft_doc::ids;

use super::design_link::absolute;
use super::{CommandSpec, Part, find_command};
use crate::drawing_sources::SourceState;
use crate::params::bad;
use crate::schema::drawing_source::{
    AddParams, ExecuteParams, ExecuteResult, RemoveParams, RemoveResult, RepointParams, SourceResult, SourcesResult,
};
use crate::schema::{TypedContract, output_schema_for};
use crate::{EngineError, Result, Session, frames};

const EXECUTE: &str = "drawing.source.execute";
const EXECUTE_CONTRACT: TypedContract<ExecuteParams> = TypedContract::new();
const LIST: &str = "drawing.sources";
const ADD: &str = "drawing.source.add";
const ADD_CONTRACT: TypedContract<AddParams> = TypedContract::new();
const RELINK: &str = "drawing.source.relink";
const RELINK_CONTRACT: TypedContract<RepointParams> = TypedContract::new();
const LOCATE: &str = "drawing.source.locate";
const LOCATE_CONTRACT: TypedContract<RepointParams> = TypedContract::new();
const REMOVE: &str = "drawing.source.remove";
const REMOVE_CONTRACT: TypedContract<RemoveParams> = TypedContract::new();

pub static COMMANDS: &[CommandSpec] = &[
    CommandSpec::new(EXECUTE, "Change the Model", crate::checked!("drawing.source.execute", execute))
        .any_part()
        .icon("edit")
        .contract(&EXECUTE_CONTRACT)
        .output(output_schema_for::<ExecuteResult>)
        .params(
            "source?: its id (default the design the drawing is in); command: a model command's id; params?: what it takes → {source, \
         command, result: its answer}. Runs with the drawing open and leaves it open. One undo step of the model and one evaluation; \
         edit.undo {model: true} takes it back from the drawing. Refused: anything but a model command; live: true; a command that \
         leaves a sketch open or a move uncaptured; a linked design, which is read-only from a drawing.",
        ),
    CommandSpec::new(LIST, "List Sources", crate::checked!("drawing.sources", list))
        .icon("list")
        .in_drawing()
        .noundo()
        .doc_only()
        .output(output_schema_for::<SourcesResult>)
        .params(
            "→ sources: [{source, path?, file?: what it is read from now, state: up_to_date, pending, missing, unreadable, not_allowed or newer, \
         reason?}]. A design that couldn't be opened is tried again. The design the drawing is in has no path.",
        ),
    CommandSpec::new(ADD, "Add Source", crate::checked!("drawing.source.add", add))
        .icon("new")
        .in_drawing()
        .doc_only()
        .contract(&ADD_CONTRACT)
        .output(output_schema_for::<SourceResult>)
        .params("path: a saved .sc design → as one of drawing.sources. Read before anything changes. One undo step; this design isn't evaluated."),
    CommandSpec::new(RELINK, "Relink Source", crate::checked!("drawing.source.relink", relink))
        .icon("chain")
        .in_drawing()
        .doc_only()
        .contract(&RELINK_CONTRACT)
        .output(output_schema_for::<SourceResult>)
        .params(
            "source: its id; path: a saved .sc design → as one of drawing.sources, with warnings? when the design has another name. Read before \
         anything changes. One undo step; this design isn't evaluated.",
        ),
    CommandSpec::new(LOCATE, "Locate Source", crate::checked!("drawing.source.locate", locate))
        .icon("open")
        .in_drawing()
        .noundo()
        .doc_only()
        .contract(&LOCATE_CONTRACT)
        .output(output_schema_for::<SourceResult>)
        .params(
            "source: its id; path: a saved .sc design → as one of drawing.sources. Finds the source's file for this session only: the drawing \
         keeps its path and has nothing new to save.",
        ),
    CommandSpec::new(REMOVE, "Remove Source", crate::checked!("drawing.source.remove", remove))
        .icon("delete")
        .in_drawing()
        .doc_only()
        .contract(&REMOVE_CONTRACT)
        .output(output_schema_for::<RemoveResult>)
        .params(
            "source: its id → {removed}. Refused while a view, a sheet or a parts list uses it, and for the design the drawing is in. One undo step.",
        ),
];

/// The file a drawing's source named `given` keeps, whole: a saved design that isn't this one, whose
/// path may be reached. All of it is judged from the text, before anything asks the disk or the network.
pub(super) fn source_file(s: &Session, cmd: &str, given: &str) -> Result<String> {
    if given.trim().is_empty() || given.len() >= MAX_SOURCE_PATH_BYTES || given.contains('\0') {
        return Err(bad(cmd, "`source` is the path of a saved .sc design"));
    }
    let file = absolute(Path::new(given));
    if !solidcraft_io::is_design_container_path(&file) {
        return Err(bad(cmd, "choose a saved .sc design"));
    }
    if s.path.as_deref().is_some_and(|own| absolute(Path::new(own)) == file) {
        return Err(bad(cmd, "that is the design the drawing is in"));
    }
    super::component_link::link_refused(s.path.as_deref(), given).map_err(|e| bad(cmd, e.to_string()))?;
    Ok(file.to_string_lossy().into_owned())
}

/// The linked source `given` names.
fn linked<'d>(d: &'d Drawing, cmd: &str, given: &str) -> Result<&'d Source> {
    let id = ids::decode(given).ok_or_else(|| bad(cmd, "`source` must be a source's id (drawing.sources)"))?;
    let source = d.sources.iter().find(|source| source.id == id).ok_or_else(|| bad(cmd, format!("the drawing has no source {given}")))?;
    if source.path.is_none() {
        return Err(bad(cmd, "that is the design the drawing is in"));
    }
    Ok(source)
}

/// Where `source` of `d` stands now.
fn standing(s: &Session, d: &Drawing, source: &Source) -> SourceResult {
    let (state, reason) = match s.drawn(d, source.id) {
        Ok(_) => (SourceState::UpToDate, None),
        Err(why) => (why.state, Some(why.reason)),
    };
    SourceResult {
        source: ids::encode(source.id),
        path: source.path.clone(),
        file: source.path.as_deref().map(|path| s.linked.file(path).to_string()),
        state: state.word().to_string(),
        reason,
        warnings: Vec::new(),
        param_warnings: Vec::new(),
    }
}

fn reply<T: serde::Serialize>(answer: T) -> Result<Value> {
    serde_json::to_value(answer).map_err(|e| EngineError::Other(e.to_string()))
}

fn list(s: &mut Session, _p: &Value) -> Result<Value> {
    s.linked.forget_unopened();
    s.open_sources();
    let d = s.drawing(LIST)?.clone();
    reply(SourcesResult { sources: d.sources.iter().map(|source| standing(s, &d, source)).collect(), param_warnings: Vec::new() })
}

fn add(s: &mut Session, p: &Value) -> Result<Value> {
    let request: AddParams = crate::schema::read(ADD, p)?;
    let file = source_file(s, ADD, &request.path)?;
    let record = s.drawing(ADD)?.clone();
    if record.sources.iter().any(|source| source.path.as_deref() == Some(file.as_str())) {
        return Err(bad(ADD, "the drawing already draws that design"));
    }
    s.open_source_file(&file).map_err(|why| bad(ADD, why.to_string()))?;
    let id = record.allocate_id(&mut s.ids).map_err(|e| bad(ADD, e.to_string()))?;
    let source = Source { id, path: Some(file) };
    let added = source.clone();
    s.edit_drawing(ADD, |d| {
        d.sources.push(added);
        Ok(())
    })?;
    let d = s.drawing(ADD)?.clone();
    reply(standing(s, &d, &source))
}

fn relink(s: &mut Session, p: &Value) -> Result<Value> {
    let request: RepointParams = crate::schema::read(RELINK, p)?;
    let record = s.drawing(RELINK)?.clone();
    let id = linked(&record, RELINK, &request.source)?.id;
    let file = source_file(s, RELINK, &request.path)?;
    if record.sources.iter().any(|source| source.id != id && source.path.as_deref() == Some(file.as_str())) {
        return Err(bad(RELINK, "another source already draws that design"));
    }
    let was = s.drawn(&record, id).ok().map(|from| from.doc.name.clone());
    let name = s.open_source_file(&file).map_err(|why| bad(RELINK, why.to_string()))?.doc.name.clone();
    let source = Source { id, path: Some(file) };
    let moved = source.path.clone();
    s.edit_drawing(RELINK, |d| {
        if let Some(found) = d.sources.iter_mut().find(|known| known.id == id) {
            found.path = moved;
        }
        Ok(())
    })?;
    let d = s.drawing(RELINK)?.clone();
    let mut answer = standing(s, &d, &source);
    if let Some(was) = was.filter(|was| *was != name) {
        answer.warnings.push(format!("this design is called `{name}`, and the one it replaces `{was}`: check the views"));
    }
    reply(answer)
}

fn locate(s: &mut Session, p: &Value) -> Result<Value> {
    let request: RepointParams = crate::schema::read(LOCATE, p)?;
    let record = s.drawing(LOCATE)?.clone();
    let source = linked(&record, LOCATE, &request.source)?.clone();
    let file = source_file(s, LOCATE, &request.path)?;
    s.open_source_file(&file).map_err(|why| bad(LOCATE, why.to_string()))?;
    if let Some(path) = source.path.clone() {
        s.linked.locate(path, file);
    }
    reply(standing(s, &record, &source))
}

fn remove(s: &mut Session, p: &Value) -> Result<Value> {
    let request: RemoveParams = crate::schema::read(REMOVE, p)?;
    let record = s.drawing(REMOVE)?.clone();
    let id = linked(&record, REMOVE, &request.source)?.id;
    let users = record.source_users(id);
    if !users.is_empty() {
        let named: Vec<String> = users.iter().map(|user| ids::encode(*user)).collect();
        return Err(bad(REMOVE, format!("the source is in use by {}", named.join(", "))));
    }
    s.edit_drawing(REMOVE, |d| {
        d.sources.retain(|source| source.id != id);
        Ok(())
    })?;
    reply(RemoveResult { removed: ids::encode(id), param_warnings: Vec::new() })
}

fn execute(s: &mut Session, p: &Value) -> Result<Value> {
    let request: ExecuteParams = crate::schema::read(EXECUTE, p)?;
    let source = {
        let d = s.drawing(EXECUTE)?;
        let design = d.sources.first().map(|source| source.id);
        let named = request.source.as_deref().map(|id| ids::decode(id).ok_or_else(|| bad(EXECUTE, "`source` must be a source's id"))).transpose()?;
        named.or(design).filter(|id| d.sources.iter().any(|source| source.id == *id)).ok_or_else(|| bad(EXECUTE, "the drawing has no such source"))?
    };
    if s.drawing(EXECUTE)?.sources.iter().any(|known| known.id == source && known.path.is_some()) {
        return Err(bad(EXECUTE, "a linked design is read-only from a drawing: open it to change it"));
    }
    let id = request.command.as_str();
    let spec = find_command(id).ok_or_else(|| bad(EXECUTE, format!("no command `{id}`")))?;
    if spec.part != Part::Model {
        return Err(bad(EXECUTE, format!("`{id}` isn't a model command")));
    }
    if request.params.get("live").is_some_and(|live| live != &Value::Bool(false)) {
        return Err(bad(EXECUTE, "a live gesture is the model's own: `live` isn't taken here"));
    }
    let params = if request.params.is_null() { Value::Object(Default::default()) } else { request.params };

    let _frame = frames::mapped_as(id);
    let mapped = frames::to_local(s, id, &params);
    let (open, sketch, moves) = (s.open_drawing.take(), s.active_sketch, s.pending_moves.clone());
    let result = spec.run_nested(s, mapped.as_ref().unwrap_or(&params));
    s.open_drawing = open;
    let result = result?;
    // A sketch opened changes no document, so nothing else would put it back.
    let left_open = s.active_sketch != sketch;
    s.active_sketch = sketch;
    if left_open || s.pending_moves != moves {
        return Err(bad(EXECUTE, format!("`{id}` leaves a sketch open or a move uncaptured: close the drawing to do that")));
    }
    let reply = ExecuteResult { source: ids::encode(source), command: id.to_string(), result };
    serde_json::to_value(reply).map_err(|e| EngineError::Other(e.to_string()))
}

#[cfg(test)]
#[path = "drawing_source_tests.rs"]
mod tests;
