//! `drawing.source.execute` (#3890): a model command run with a drawing open, which stays open.
//! The caller's command owns the undo step and the evaluation ([`CommandSpec::run_nested`]), so it is
//! one step of the model and one evaluation, and costs what the model command costs. Only a model
//! command runs here, so files, checkpoints, undo and `drawing.*` need no list.
//!
//! A drawing's undo passes that step, and says so; `edit.undo {model: true}` takes it back from here.
//! A command that would leave a sketch open or a move uncaptured waits for the drawing to close: both
//! are the model's own modes, with nothing here to finish them.

use serde_json::Value;
use solidcraft_doc::ids;

use super::{CommandSpec, Part, find_command};
use crate::params::bad;
use crate::schema::drawing_source::{ExecuteParams, ExecuteResult};
use crate::schema::{TypedContract, output_schema_for};
use crate::{EngineError, Result, Session, frames};

const EXECUTE: &str = "drawing.source.execute";
const EXECUTE_CONTRACT: TypedContract<ExecuteParams> = TypedContract::new();

pub static COMMANDS: &[CommandSpec] =
    &[CommandSpec::new(EXECUTE, "Change the Model", crate::checked!("drawing.source.execute", EXECUTE_CONTRACT, execute))
        .any_part()
        .icon("edit")
        .contract(&EXECUTE_CONTRACT)
        .output(output_schema_for::<ExecuteResult>)
        .params(
            "source?: its id (default the design the drawing is in); command: a model command's id; params?: what it takes → {source, \
         command, result: its answer}. Runs with the drawing open and leaves it open. One undo step of the model and one evaluation; \
         edit.undo {model: true} takes it back from the drawing. Refused: anything but a model command; live: true; a command that \
         leaves a sketch open or a move uncaptured.",
        )];

fn execute(s: &mut Session, p: &Value) -> Result<Value> {
    let request: ExecuteParams = crate::schema::read(EXECUTE, p)?;
    let source = {
        let d = s.drawing(EXECUTE)?;
        let design = d.sources.first().map(|source| source.id);
        let named = request.source.as_deref().map(|id| ids::decode(id).ok_or_else(|| bad(EXECUTE, "`source` must be a source's id"))).transpose()?;
        named.or(design).filter(|id| d.sources.iter().any(|source| source.id == *id)).ok_or_else(|| bad(EXECUTE, "the drawing has no such source"))?
    };
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
