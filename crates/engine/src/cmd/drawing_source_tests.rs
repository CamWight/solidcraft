use std::sync::Arc;

use serde_json::json;

use super::*;
use crate::schema::tests::conforms;

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
}

/// A model command run from the open drawing: its reply, checked against the schema.
fn model(s: &mut Session, command: &str, params: Value) -> Value {
    let reply = run(s, EXECUTE, json!({"command": command, "params": params}));
    let schema = output_schema_for::<ExecuteResult>();
    assert!(conforms(schema.as_value(), schema.as_value(), &reply), "{reply}");
    reply
}

/// `p` is refused, leaving the design, the open drawing, the sketch and the steps as they were: why.
fn refused(s: &mut Session, p: Value) -> String {
    let (doc, open, sketch, steps, runs) = (s.doc.clone(), s.open_drawing, s.active_sketch, s.undo.len(), s.evaluations);
    let why = s.execute(EXECUTE, &p).err().map(|e| e.to_string()).unwrap_or_else(|| panic!("accepted {p}"));
    assert!(Arc::ptr_eq(&doc, &s.doc) || *doc == *s.doc, "{p}: the design changed");
    assert_eq!((s.open_drawing, s.active_sketch, s.undo.len()), (open, sketch, steps), "{p}");
    assert!(s.evaluations <= runs + 1, "{p}");
    why
}

/// A box and a drawing of it, open.
fn drawn() -> Session {
    let mut s = Session::default();
    run(&mut s, "solid.box", json!({"length": 40, "width": 30, "height": 10, "body_name": "Plate"}));
    run(&mut s, "drawing.create", json!({}));
    s
}

/// A model command from the drawing is one step of the model and one evaluation. The drawing stays
/// open, its record the same `Arc`. Its reply is the command's.
#[test]
fn a_model_command_runs_with_the_drawing_open() {
    let mut s = drawn();
    let (open, record) = (s.open_drawing, s.drawing("test").unwrap().clone());
    let (steps, runs, features) = (s.undo.len(), s.evaluations, s.doc.features.len());
    let reply = model(&mut s, "solid.box", json!({"length": 5, "width": 5, "height": 5, "corner": [100, 0, 0], "operation": "new"}));
    assert_eq!(reply["command"], json!("solid.box"), "{reply}");
    assert_eq!(reply["source"], json!(ids::encode(record.sources[0].id)));
    assert!(reply["result"].is_object());
    assert_eq!(s.doc.features.len(), features + 1);
    assert_eq!((s.undo.len(), s.undo.last().map(|step| step.part)), (steps + 1, Some(None)), "a step of the model");
    assert_eq!(s.evaluations, runs + 1, "one evaluation");
    assert_eq!(s.open_drawing, open);
    assert!(Arc::ptr_eq(&record, s.drawing("test").unwrap()), "the drawing's record is untouched");

    let named = run(&mut s, EXECUTE, json!({"source": ids::encode(record.sources[0].id), "command": "parameters.list"}));
    assert_eq!(named["command"], json!("parameters.list"), "a read takes no step: {named}");
    assert_eq!(s.undo.len(), steps + 1);
}

/// A drawing's undo passes the model's newer step and says so, and leaves it. `model: true` takes it
/// back with the drawing open and its edits as they are; redo brings it back.
#[test]
fn the_drawings_undo_passes_a_model_step_and_says_so() {
    let mut s = drawn();
    run(&mut s, "drawing.set", json!({"name": "First"}));
    let features = s.doc.features.len();
    model(&mut s, "solid.box", json!({"length": 5, "width": 5, "height": 5, "corner": [100, 0, 0], "operation": "new"}));
    run(&mut s, "drawing.set", json!({"name": "Second"}));

    let newest = run(&mut s, "edit.undo", json!({}));
    assert_eq!(newest.get("skipped"), None, "nothing newer: {newest}");
    let passed = run(&mut s, "edit.undo", json!({}));
    assert_eq!(passed["skipped"].as_array().map(Vec::len), Some(1), "{passed}");
    assert_eq!(s.doc.features.len(), features + 1, "the model's step stays");

    let runs = s.evaluations;
    let undone = run(&mut s, "edit.undo", json!({"model": true}));
    assert_eq!(undone.get("skipped"), None, "{undone}");
    assert_eq!(s.doc.features.len(), features, "taken back");
    assert_eq!(s.evaluations, runs + 1);
    assert!(s.open_drawing.is_some(), "still open");
    run(&mut s, "edit.redo", json!({"model": true}));
    assert_eq!(s.doc.features.len(), features + 1);
    assert!(s.open_drawing.is_some());
    assert!(s.execute("edit.undo", &json!({"model": "yes"})).is_err());
    assert!(s.execute("edit.redo", &json!({"model": 1})).is_err());
}

/// The step of the model that made the open drawing can't be undone from it: nothing could be open.
#[test]
fn a_model_undo_never_takes_away_the_open_drawing() {
    let mut s = drawn();
    let open = s.open_drawing;
    let why = s.execute("edit.undo", &json!({"model": true})).unwrap_err().to_string();
    assert!(why.contains("close it first"), "{why}");
    assert_eq!(s.open_drawing, open);
    assert!(s.drawing("test").is_ok());
    assert_eq!(s.undo.len(), 2, "the box and the drawing's making stay");
    run(&mut s, "drawing.close", json!({}));
    run(&mut s, "edit.undo", json!({"model": true}));
    assert!(s.doc.drawings.is_empty(), "with it closed, as ever");
}

/// Everything but a model command is refused with nothing changed, and a failing one leaves everything
/// as it was. No drawing open, no drawing to stand in.
#[test]
fn what_isnt_a_model_edit_is_refused() {
    let mut s = drawn();
    for command in ["edit.undo", "file.save", "edit.checkpoint", "drawing.close", "drawing.set", "drawing.source.execute", "no.such"] {
        let why = refused(&mut s, json!({"command": command, "params": {}}));
        assert!(why.contains("model command") || why.contains("no command"), "{command}: {why}");
    }
    refused(&mut s, json!({"source": "0000000000000", "command": "parameters.list"}));
    refused(&mut s, json!({"source": "x", "command": "parameters.list"}));
    refused(&mut s, json!({"command": 7}));
    refused(&mut s, json!({}));
    refused(&mut s, json!({"command": "solid.box", "params": {"live": true}}));
    refused(&mut s, json!({"command": "solid.box", "params": {"length": -5}}));
    refused(&mut s, json!({"command": "solid.box", "params": {"length": "not a number", "width": 5, "height": 5}}));
    refused(&mut s, json!({"command": "solid.box", "params": [1, 2]}));

    run(&mut s, "drawing.close", json!({}));
    assert!(s.execute(EXECUTE, &json!({"command": "parameters.list"})).is_err(), "open a drawing first");
}

/// A sketch is the model's own mode: opening one from here is refused with none left open.
#[test]
fn a_sketch_stays_out_of_a_drawing() {
    let mut s = Session::default();
    let made = run(&mut s, "sketch.create", json!({"plane": "XY"}));
    let sketch = made["sketch"].clone();
    run(&mut s, "sketch.rectangle.two_point", json!({"p0": [0, 0], "p1": [10, 10]}));
    run(&mut s, "sketch.finish", json!({}));
    run(&mut s, "drawing.create", json!({}));
    let why = refused(&mut s, json!({"command": "sketch.edit", "params": {"sketch": sketch}}));
    assert!(why.contains("sketch open"), "{why}");
    assert_eq!(s.active_sketch, None);
    let why = refused(&mut s, json!({"command": "sketch.create", "params": {"plane": "XY"}}));
    assert!(why.contains("sketch open"), "{why}");
    assert_eq!(s.active_sketch, None);
}

/// A numbered view's new row takes its number in the same step, as a model edit's does.
#[test]
fn a_new_row_takes_its_item_number() {
    let mut s = Session::default();
    run(&mut s, "component.create", json!({"name": "Plate"}));
    run(&mut s, "solid.box", json!({"length": 40, "width": 20, "height": 3}));
    run(&mut s, "component.activate", json!({"component": 0}));
    run(&mut s, "assembly.bom.renumber", json!({"levels": "flat"}));
    run(&mut s, "drawing.create", json!({}));
    let count = |s: &Session| s.doc.assembly.item_numbers[0].items.len();
    let before = count(&s);
    let steps = s.undo.len();
    model(&mut s, "component.create", json!({"name": "Bolt", "activate": false}));
    assert_eq!(count(&s), before + 1, "numbered");
    assert_eq!(s.undo.len(), steps + 1, "in the one step");
}

/// A folder of this test's own, and a design saved in it as `a.sc`: a box.
fn saved_design(test: &str) -> (std::path::PathBuf, String, Session) {
    let dir = std::env::temp_dir().join(format!("solidcraft-drawing-source-{}-{test}", std::process::id()));
    std::fs::create_dir_all(dir.join("parts")).unwrap();
    std::fs::create_dir_all(dir.join("moved").join("deep")).unwrap();
    let path = dir.join("a.sc").to_string_lossy().into_owned();
    let mut s = Session::default();
    run(&mut s, "solid.box", json!({"length": 10, "width": 10, "height": 10}));
    run(&mut s, "file.save", json!({"path": path}));
    (dir, path, s)
}

/// The path each drawing's linked source has in the file at `path`.
fn in_file(path: &str) -> Vec<Option<String>> {
    let (doc, _) = solidcraft_io::read_design_file(path, None::<fn(solidcraft_io::DesignPreview)>).unwrap();
    doc.drawings.iter().map(|d| d.sources[0].path.clone()).collect()
}

/// #6860's reproduction, with a linked source: Save As moves the design, and undo, redo and a checkpoint
/// bring back a design whose source is still the same file. In memory it is whole; each file keeps it
/// relative to where that file is. A save rewrites nothing in the session's history.
#[test]
fn a_source_path_follows_save_as_through_undo_redo_and_checkpoints() {
    let (dir, path, mut s) = saved_design("save-as");
    let part = dir.join("parts").join("plate.sc").to_string_lossy().into_owned();
    run(&mut s, "drawing.create", json!({"source": part}));
    let drawing = s.open_drawing.unwrap();
    let held = |s: &Session| s.doc.drawing(drawing).and_then(|d| d.sources[0].path.clone());
    assert_eq!(held(&s).as_deref(), Some(part.as_str()), "whole in memory");
    run(&mut s, "file.save", json!({"path": path}));
    assert_eq!(in_file(&path), [Some("parts/plate.sc".to_string())]);

    run(&mut s, "drawing.close", json!({}));
    run(&mut s, "edit.checkpoint", json!({"name": "before"}));
    let moved = dir.join("moved").join("deep").join("a.sc").to_string_lossy().into_owned();
    run(&mut s, "file.save_as", json!({"path": moved}));
    assert_eq!(in_file(&moved), [Some("../../parts/plate.sc".to_string())]);
    assert_eq!(in_file(&path), [Some("parts/plate.sc".to_string())], "the file it came from is as it was");
    assert_eq!(held(&s).as_deref(), Some(part.as_str()), "a save changes nothing in memory");

    run(&mut s, "solid.box", json!({"length": 5, "width": 5, "height": 5}));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(held(&s).as_deref(), Some(part.as_str()));
    run(&mut s, "edit.redo", json!({}));
    assert_eq!(held(&s).as_deref(), Some(part.as_str()));
    run(&mut s, "edit.restore_checkpoint", json!({"name": "before"}));
    assert_eq!(held(&s).as_deref(), Some(part.as_str()));
    run(&mut s, "file.save", json!({}));
    assert_eq!(in_file(&moved), [Some("../../parts/plate.sc".to_string())]);

    // Opened again from where it moved to: whole in memory, and the same file.
    run(&mut s, "doc.open", json!({"path": moved}));
    assert_eq!(s.doc.drawings[0].sources[0].path.as_deref(), Some(part.as_str()));
    std::fs::remove_dir_all(&dir).unwrap();
}

/// A drawing of another design may be made in an empty one, and nothing about the source is asked of
/// the disk or the network. A path that can't be a source is refused from its text alone.
#[test]
fn a_source_is_judged_from_its_text() {
    let (dir, path, mut s) = saved_design("text");
    let calls = solidcraft_io::vfs::file_calls();
    let mut hostile = vec![String::new(), " ".into(), "a\0b.sc".into(), "x".repeat(4096), "plate.step".into(), path.clone(), r"\\.\COM1\plate.sc".into()];
    if cfg!(windows) {
        hostile.push(r"\\evil.example\share\plate.sc".into());
    }
    let before = s.doc.clone();
    for given in hostile {
        let refused = s.execute("drawing.create", &json!({"source": given}));
        assert!(refused.is_err(), "{given:?}");
        assert!(Arc::ptr_eq(&s.doc, &before), "{given:?}");
    }
    assert_eq!(solidcraft_io::vfs::file_calls(), calls, "a file call was made");

    let mut empty = Session::default();
    assert!(empty.execute("drawing.create", &json!({})).is_err(), "a drawing of nothing");
    let part = dir.join("parts").join("missing.sc").to_string_lossy().into_owned();
    let reply = run(&mut empty, "drawing.create", json!({"source": part}));
    assert_eq!(reply["sources"][0]["path"], json!(part), "{reply}");
    std::fs::remove_dir_all(&dir).unwrap();
}
