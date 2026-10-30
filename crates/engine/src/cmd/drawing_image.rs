//! `drawing.image.add` (W38e, #5426): a PNG or JPEG on a sheet, its bytes saved in the design. One
//! undo step, and no model is evaluated. An image is deleted as a drawing's other objects are
//! (`selection.delete`), and goes with its sheet.

use std::sync::Arc;

use serde_json::Value;
use solidcraft_doc::drawing::image::{MAX_IMAGE_BYTES, SheetImage};
use solidcraft_doc::ids;

use super::CommandSpec;
use crate::params::bad;
use crate::schema::drawing_image::{AddParams, AddResult};
use crate::schema::{TypedContract, output_schema_for};
use crate::{EngineError, Result, Session};

const ADD: &str = "drawing.image.add";
const ADD_CONTRACT: TypedContract<AddParams> = TypedContract::new();

pub static COMMANDS: &[CommandSpec] = &[CommandSpec::new(ADD, "Add Image", crate::checked!("drawing.image.add", add))
    .icon("new")
    .in_drawing()
    .doc_only()
    .contract(&ADD_CONTRACT)
    .output(output_schema_for::<AddResult>)
    .params(
        "sheet: id or name; file: a PNG or JPEG, at most 4 MiB and 64 million pixels, read from the file's header before anything decodes it; \
         rect_mm: [x0, y0, x1, y1] from the sheet's lower-left corner → {image, format, pixels}. Its bytes are saved in the design and its path \
         isn't. At most 9 MiB of images a drawing. One undo step; no model is evaluated.",
    )];

fn add(s: &mut Session, p: &Value) -> Result<Value> {
    let request: AddParams = crate::schema::read(ADD, p)?;
    let record = s.drawing(ADD)?.clone();
    let sheet = crate::drawing_scene::sheet_id(&record, &request.sheet, ADD)?;
    let file = request.file.as_str();
    if file.trim().is_empty() || file.len() >= 4096 {
        return Err(bad(ADD, "`file` is the path of a PNG or JPEG"));
    }
    let bytes = solidcraft_io::vfs::read(file, MAX_IMAGE_BYTES).map_err(|e| EngineError::Other(format!("{file}: {e}")))?;
    let id = record.allocate_id(&mut s.ids).map_err(|e| bad(ADD, e.to_string()))?;
    let image = SheetImage::from_file(id, sheet, request.rect_mm, &bytes).map_err(|e| bad(ADD, format!("{file}: {e}")))?;
    let reply = AddResult { image: ids::encode(id), format: image.format.clone(), pixels: image.pixels, param_warnings: Vec::new() };
    s.edit_drawing(ADD, |d| {
        d.add_images([Arc::new(image)]);
        // Every drawing saved opens again, so one that couldn't be saved is refused here, not at the save.
        d.to_json().map_err(|e| bad(ADD, e.to_string()))?;
        Ok(())
    })?;
    serde_json::to_value(reply).map_err(|e| EngineError::Other(e.to_string()))
}

#[cfg(test)]
#[path = "drawing_image_tests.rs"]
mod tests;
