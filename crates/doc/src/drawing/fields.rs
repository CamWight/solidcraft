//! A title block's fields (#3875): the keys each source has, and what they read. Stateless: the
//! drawing gives its own fields, its sheets' and its newest revision's, and the caller hands in the
//! component a sheet documents with what the model says of it (its material and mass, worked out
//! by the bill of materials). A key its source doesn't have is refused when a block is given or
//! read; a value that isn't there, or is empty, shows [`MISSING`]. Sheet tables' cells (B3) read
//! the same fields.
//!
//! - **component:** `name`, `part_number`, `description`, `revision` (its part properties),
//!   `material`, `mass`, and `properties.<name>` (its free-text properties, such as a finish);
//! - **drawing:** `name`, `number` (its `part_number` property: the drawing's identification
//!   number), `organization`, `author`, `project`, and `properties.<key>` (any of its properties);
//! - **sheet:** `name`, `number`, `count`: sheets left out of the count are neither counted nor
//!   numbered, and such a sheet shows neither;
//! - **revision:** the newest revision row's fields ([`Field`]);
//! - **prompt:** any key; answers come with templates, so a prompt shows [`MISSING`] until then.

use std::borrow::Cow;

use solidcraft_geom::cell::{Content, Format, MISSING, Part, Source};
use solidcraft_geom::revision::Field;
use solidcraft_geom::title_block::{CellKind, TitleBlock};

use super::Drawing;
use crate::{DocError, Metadata, Result};

const COMPONENT: [&str; 6] = ["name", "part_number", "description", "revision", "material", "mass"];
const DRAWING: [&str; 5] = ["name", "number", "organization", "author", "project"];
const SHEET: [&str; 3] = ["name", "number", "count"];
/// Before a property's name: the field reads that property.
const PROPERTIES: &str = "properties.";
/// The units a mass may be shown in, with the grams in one, kilograms first: what a mass shows
/// in unless its field's format says, until #4867 gives drawings their units.
const MASS_UNITS: [(&str, f64); 2] = [("kg", 1000.0), ("g", 1.0)];
/// The most decimals a number may be shown to.
pub const MAX_DECIMALS: u8 = 6;
/// The decimals a mass is shown to unless its field's format says.
const MASS_DECIMALS: u8 = 2;

/// The component a sheet documents, as its fields read it.
#[derive(Clone, Copy, Debug)]
pub struct Component<'a> {
    pub name: &'a str,
    /// Its part properties.
    pub metadata: &'a Metadata,
    /// Its free-text properties (the design's own: none).
    pub properties: Option<&'a std::collections::BTreeMap<String, String>>,
    /// What it is made of, as its bill of materials row would say: none when nothing is assigned.
    pub material: Option<&'a str>,
    /// What one of it weighs, in grams: none when it can't be weighed.
    pub mass: Option<f64>,
}

/// Whether `source` has the field `key`.
pub fn has_key(source: Source, key: &str) -> bool {
    let property = key.strip_prefix(PROPERTIES).is_some_and(|name| !name.is_empty());
    match source {
        Source::Component => COMPONENT.contains(&key) || property,
        Source::Drawing => DRAWING.contains(&key) || property,
        Source::Sheet => SHEET.contains(&key),
        Source::Revision => Field::ALL.iter().any(|f| f.key() == key),
        Source::Prompt => !key.is_empty(),
        Source::Row => false,
    }
}

/// Whether `block` has a text cell `name`: what a sheet's override may name.
pub fn overridable(block: Option<&TitleBlock>, name: &str) -> bool {
    block.is_some_and(|block| block.cells.iter().any(|cell| cell.name == name && matches!(cell.kind, CellKind::Text(_))))
}

/// Each field of `block` names a key its source has, and only a number has a format, in a unit
/// and to decimals it can be shown in: what a sheet holds its block to.
pub fn check_block(block: &TitleBlock) -> Result<()> {
    for cell in &block.cells {
        let CellKind::Text(text) = &cell.kind else { continue };
        for part in &text.content.0 {
            let Part::Field { source, key, format } = part else { continue };
            let name = serde_json::to_value(source).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default();
            let refuse = |why: String| Err(DocError::Invalid(format!("cell `{}`: {why}", cell.name)));
            if !has_key(*source, key) {
                return refuse(format!("the {name} has no field `{key}`"));
            }
            let mass = *source == Source::Component && key == "mass";
            if *format != Format::default() && !mass {
                return refuse(format!("the {name} field `{key}` is text, shown as it is: only a mass has a format"));
            }
            if format.unit.as_deref().is_some_and(|unit| !MASS_UNITS.iter().any(|(u, _)| *u == unit)) {
                return refuse("not supported yet: a mass is shown in kg or g until #4867 and #4811 bring more units".into());
            }
            if format.decimals.is_some_and(|d| d > MAX_DECIMALS) {
                return refuse(format!("a number is shown to at most {MAX_DECIMALS} decimals"));
            }
        }
    }
    Ok(())
}

/// A mass of `grams` shown by `format`: in its unit, kilograms unless it says, to its decimals.
fn mass_text(grams: f64, format: &Format) -> String {
    let wanted = |(unit, _): &&(&str, f64)| format.unit.as_deref().is_none_or(|u| u == *unit);
    let (unit, per) = MASS_UNITS.iter().find(wanted).copied().unwrap_or(MASS_UNITS[0]);
    let decimals = usize::from(format.decimals.unwrap_or(MASS_DECIMALS).min(MAX_DECIMALS));
    format!("{:.decimals$} {unit}", grams / per)
}

/// What a sheet's fields read: its drawing, its place among the drawing's sheets, and the component
/// it documents.
#[derive(Clone, Copy, Debug)]
pub struct Values<'a> {
    pub drawing: &'a Drawing,
    pub sheet: usize,
    pub component: Option<Component<'a>>,
}

impl Values<'_> {
    /// `content` as its cell shows it ([`Content::shown`]): text as written, each field its value
    /// by its format, or [`MISSING`] when it has none or an empty one.
    pub fn text(&self, content: &Content) -> String {
        content.shown(|source, key, format| self.field(source, key, format).map(Cow::Owned), MISSING)
    }

    /// The field `key` of `source` shown by `format`, when it has a value.
    fn field(&self, source: Source, key: &str, format: &Format) -> Option<String> {
        let d = self.drawing;
        let property = |metadata: &Metadata| key.strip_prefix(PROPERTIES).and_then(|name| metadata.text(name)).map(str::to_string);
        match source {
            Source::Drawing => match key {
                "name" => Some(d.name.clone()),
                "number" => d.properties.part_number().map(str::to_string),
                "organization" | "author" | "project" => d.properties.text(key).map(str::to_string),
                _ => property(&d.properties),
            },
            Source::Sheet => {
                let sheet = d.sheets.get(self.sheet)?;
                if sheet.excluded.count {
                    return (key == "name").then(|| sheet.name.clone());
                }
                let counted = |s: &&std::sync::Arc<super::Sheet>| !s.excluded.count;
                match key {
                    "name" => Some(sheet.name.clone()),
                    "number" => Some((d.sheets.iter().take(self.sheet).filter(counted).count() + 1).to_string()),
                    "count" => Some(d.sheets.iter().filter(counted).count().to_string()),
                    _ => None,
                }
            }
            Source::Revision => d.revisions.rows().last()?.field(key).map(str::to_string),
            Source::Component => {
                let c = self.component?;
                match key {
                    "name" => Some(c.name.to_string()),
                    "material" => c.material.map(str::to_string),
                    "mass" => c.mass.map(|grams| mass_text(grams, format)),
                    "part_number" | "description" | "revision" => c.metadata.text(key).map(str::to_string),
                    _ => key.strip_prefix(PROPERTIES).and_then(|name| c.properties?.get(name)).cloned(),
                }
            }
            Source::Prompt | Source::Row => None,
        }
    }
}

#[cfg(test)]
#[path = "fields_tests.rs"]
mod tests;
