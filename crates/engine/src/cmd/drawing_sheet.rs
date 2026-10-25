//! A drawing's sheets (#3875, W18 S4): add, change, reorder and delete them, each one undo step of
//! the drawing. A sheet is named by its id or, when only one has it, its name.
//!
//! A new sheet takes the paper, border and resolution of the sheet it follows, so a drawing's
//! sheets match unless told otherwise. Changing a sheet's paper changes only the paper: regrowing
//! its border and moving what is on it is S5's.

use std::sync::Arc;

use serde_json::{Value, json};
use solidcraft_doc::drawing::{Drawing, Excluded, Sheet, SheetComponent};
use solidcraft_doc::ids::{self, IdSource};
use solidcraft_geom::drawing_sheet::{Orientation, SheetSize};

use super::CommandSpec;
use crate::params::{bad, num, str_};
use crate::{Result, Session};

pub static COMMANDS: &[CommandSpec] = &[
    CommandSpec::new("drawing.sheet.add", "Add Sheet", add).icon("new").drawing_file().doc_only().params(
        "after?: a sheet (id or name; default the last); name?; size?: {standard: a3} or {custom: {sides_mm: [w, h]}}; orientation?: landscape|portrait; dpi?: 25..2400 (each defaulting to the sheet it follows) → the new sheet's id and the drawing's sheets. One undo step.",
    ),
    CommandSpec::new("drawing.sheet.set", "Sheet Settings", set).icon("settings").drawing_file().doc_only().params(
        "sheet: id or name; name?, size?, orientation?, dpi?; documents?: {source: id, component: id (0 the design)} or null; excluded?: {count?, print?} → the drawing's sheets. One undo step.",
    ),
    CommandSpec::new("drawing.sheet.reorder", "Reorder Sheets", reorder)
        .icon("reorder")
        .in_drawing()
        .doc_only()
        .params("sheets: every sheet's id, in the new order → the drawing's sheets. One undo step."),
    CommandSpec::new("drawing.sheet.delete", "Delete Sheet", delete)
        .icon("delete")
        .in_drawing()
        .doc_only()
        .params("sheet: id or name (a drawing keeps at least one) → the drawing's sheets. One undo step."),
];

/// The drawing a command changes, as a copy to change.
fn drawing(s: &Session, cmd: &str) -> Result<Drawing> {
    s.drawing(cmd).map(|d| (**d).clone())
}

/// Keeps a changed drawing when it passes its rules, and answers with its sheets.
fn keep(s: &mut Session, cmd: &str, d: Drawing) -> Result<Value> {
    s.edit_drawing(cmd, |open| {
        *open = d;
        Ok(())
    })?;
    let sheets: Vec<Value> = s.drawing(cmd)?.sheets.iter().map(|sheet| json!({"id": ids::encode(sheet.id), "name": sheet.name})).collect();
    Ok(json!({"sheets": sheets}))
}

/// The position of the sheet `p[key]` names: by id, else by a name only one sheet has.
fn sheet_at(cmd: &str, d: &Drawing, p: &Value, key: &str) -> Result<usize> {
    let named = str_(p, key).ok_or_else(|| bad(cmd, format!("`{key}` must be a sheet's id or name")))?;
    if let Some(i) = ids::decode(named).and_then(|id| d.sheets.iter().position(|s| s.id == id)) {
        return Ok(i);
    }
    let mut called = d.sheets.iter().enumerate().filter(|(_, s)| s.name == named).map(|(i, _)| i);
    match (called.next(), called.next()) {
        (Some(i), None) => Ok(i),
        (Some(_), Some(_)) => Err(bad(cmd, format!("more than one sheet is called `{named}`: give its id"))),
        (None, _) => Err(bad(cmd, format!("no sheet `{named}`"))),
    }
}

/// The settings `p` gives a sheet, over the ones it has.
fn apply(cmd: &str, sheet: &mut Sheet, p: &Value) -> Result<()> {
    if let Some(name) = p.get("name") {
        sheet.name = name.as_str().ok_or_else(|| bad(cmd, "`name` must be text"))?.to_string();
    }
    if let Some(size) = p.get("size") {
        sheet.sheet.size = serde_json::from_value::<SheetSize>(size.clone())
            .map_err(|_| bad(cmd, "`size` is {\"standard\": \"a3\"} or {\"custom\": {\"sides_mm\": [w, h]}}"))?;
    }
    if let Some(orientation) = p.get("orientation") {
        sheet.sheet.orientation =
            serde_json::from_value::<Orientation>(orientation.clone()).map_err(|_| bad(cmd, "`orientation` is landscape or portrait"))?;
    }
    if p.get("dpi").is_some() {
        sheet.dpi = num(p, "dpi").ok_or_else(|| bad(cmd, "`dpi` must be a number"))?;
    }
    match p.get("documents") {
        None => {}
        Some(Value::Null) => sheet.documents = None,
        Some(documents) => {
            let source = str_(documents, "source").and_then(ids::decode).ok_or_else(|| bad(cmd, "`documents.source` must be a source's id"))?;
            let component =
                documents.get("component").map_or(Some(0), Value::as_u64).ok_or_else(|| bad(cmd, "`documents.component` must be a component id"))?;
            sheet.documents = Some(SheetComponent { source, component });
        }
    }
    if let Some(excluded) = p.get("excluded") {
        let flag = |key: &str, was: bool| match excluded.get(key) {
            None => Ok(was),
            Some(Value::Bool(b)) => Ok(*b),
            Some(_) => Err(bad(cmd, format!("`excluded.{key}` is true or false"))),
        };
        sheet.excluded = Excluded { count: flag("count", sheet.excluded.count)?, print: flag("print", sheet.excluded.print)? };
    }
    Ok(())
}

fn add(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "drawing.sheet.add";
    let mut d = drawing(s, cmd)?;
    let after = if p.get("after").is_some() { sheet_at(cmd, &d, p, "after")? } else { d.sheets.len().saturating_sub(1) };
    let model = d.sheets.get(after).ok_or_else(|| bad(cmd, "the drawing has no sheet to follow"))?;
    let id = d.allocate_id(&mut IdSource::from_entropy()).map_err(|e| bad(cmd, e.to_string()))?;
    let free = (d.sheets.len() + 1..).map(|n| format!("Sheet {n}")).find(|name| d.sheets.iter().all(|s| s.name != *name)).unwrap_or_default();
    let mut sheet = Sheet { id, name: free, documents: None, excluded: Excluded::default(), ..(**model).clone() };
    apply(cmd, &mut sheet, p)?;
    d.sheets.insert(after + 1, Arc::new(sheet));
    let mut answer = keep(s, cmd, d)?;
    answer["sheet"] = json!(ids::encode(id));
    Ok(answer)
}

fn set(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "drawing.sheet.set";
    let mut d = drawing(s, cmd)?;
    let at = sheet_at(cmd, &d, p, "sheet")?;
    let sheet = d.sheets.get_mut(at).ok_or_else(|| bad(cmd, "no such sheet"))?;
    apply(cmd, Arc::make_mut(sheet), p)?;
    keep(s, cmd, d)
}

fn reorder(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "drawing.sheet.reorder";
    let mut d = drawing(s, cmd)?;
    let every = || bad(cmd, "`sheets` must list every sheet's id once");
    let order: Vec<u64> = p
        .get("sheets")
        .and_then(Value::as_array)
        .ok_or_else(every)?
        .iter()
        .map(|v| v.as_str().and_then(ids::decode))
        .collect::<Option<_>>()
        .ok_or_else(every)?;
    let mut sheets = Vec::with_capacity(order.len());
    for id in &order {
        let at = d.sheets.iter().position(|s| s.id == *id).ok_or_else(every)?;
        sheets.push(d.sheets.remove(at));
    }
    if !d.sheets.is_empty() {
        return Err(every());
    }
    d.sheets = sheets;
    keep(s, cmd, d)
}

fn delete(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "drawing.sheet.delete";
    let mut d = drawing(s, cmd)?;
    let at = sheet_at(cmd, &d, p, "sheet")?;
    if d.sheets.len() <= 1 {
        return Err(bad(cmd, "a drawing keeps at least one sheet"));
    }
    d.sheets.remove(at);
    keep(s, cmd, d)
}

#[cfg(test)]
#[path = "drawing_sheet_tests.rs"]
mod tests;
