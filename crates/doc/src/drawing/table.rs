//! A drawing's saved tables (#3874): a parts list or a revision table on one of its sheets. The
//! record says what the table lists, how it reads ([`Table`]) and where it sits. Only a general
//! table's rows are saved: every other kind reads the design and the revision record as they are
//! when it is drawn.
//!
//! - **A parts list** lists one bill of materials view of a component of the drawing's design,
//!   with the options [`crate::bom::ItemNumbers`] saves. Its source is checked against the
//!   drawing's sources; a component deleted since is for what draws it to report.
//!   [`PartsList::rows`] gives a list's rows as a table reads them.
//! - **A family table** lists the design's configurations, a row each, with a column per
//!   configuration column ([`Family::table`]). Only the design has configurations: one of another
//!   component is for what draws it to report.
//! - **A general table** holds cells typed in ([`General`]), as plain text. Its cells are the
//!   table's, so an override over one is refused: the cell itself is typed.
//! - **A hole table** lists holes picked on a view ([`Hole`]), each with the tag it was given
//!   ([`hole_tags`]: by size, then in order), measured from one point of the model as the view shows
//!   it. Its view must be one of the drawing's, and it goes with its view when that is deleted
//!   ([`Drawing::with_contents`]). The places and sizes are read from the model when it is drawn.
//! - **Where it sits** is a corner of the sheet's drawing space with an offset, so a change of
//!   paper takes it along and moves nothing else, or a free place on paper.
//!
//! The rules, beside the record's: a table is on one of the drawing's sheets; its definition keeps
//! [`Table::validate`]; its place is on paper ([`is_paper_coordinate`]); at most [`MAX_TABLES`],
//! listed by id. A general table's cells keep a table's caps (`General::check`); a hole table's
//! holes and tags theirs (`Hole::check`).

use std::collections::{BTreeMap, HashSet};

use serde::{Deserialize, Serialize};
use solidcore_record::Vec3;
use solidcraft_geom::cell::{self, Content, Format, HAlign, MAX_NAME_BYTES, Part, Source};
use solidcraft_geom::drawing_measure::on_view;
use solidcraft_geom::drawing_view::is_paper_coordinate;
use solidcraft_geom::paper::MAX_SIDE_MM;
use solidcraft_geom::table::{self, BALLOONED, Column, Filter, Heading, Table};

use super::by_id::Record;
use super::dimension::{DEFAULT_DECIMALS, LabelKind, Lookup, ModelPoint, Status, check_point, reading_label};
use super::hole_callout::{self, HoleRef, Resolution, TextStyle};
use super::{Drawing, SheetComponent, require};
use crate::assembly::BomType;
use crate::bom::{self, Bom, Group, Mass, Quantity, Request, View};
use crate::config::Column as ConfigColumn;
use crate::display_units::{NumberFormat, format_length, format_number};
use crate::{DocError, Document, Model, Result, expr, ids};

/// The most tables a drawing may have.
pub const MAX_TABLES: usize = 1_000;
/// The most decimals a mass may show.
const MAX_MASS_DECIMALS: u8 = 6;
/// The row field that marks a family table's active configuration, which no new table prints.
pub const ACTIVE: &str = "active";
/// The most holes a hole table lists.
pub const MAX_HOLES: usize = 1_000;
/// The most characters a hole's tag has.
pub const MAX_TAG_CHARS: usize = 8;

/// A table on a sheet.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SheetTable {
    #[serde(with = "crate::ids")]
    pub id: u64,
    /// The sheet it is on.
    #[serde(with = "crate::ids")]
    pub sheet: u64,
    pub kind: Kind,
    pub table: Table,
    pub place: Place,
}

/// What a table lists.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Kind {
    PartsList(PartsList),
    /// The drawing's revision rows ([`Drawing::revisions`]).
    Revision,
    Family(Family),
    General(General),
    Hole(Hole),
}

/// The holes a hole table lists: picked on `view` and measured on it from `origin`, each with its tag.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Hole {
    /// The view the holes are measured on and their tags stand on.
    #[serde(with = "crate::ids")]
    pub view: u64,
    /// The point of the model the places are measured from: a corner, or a circle's centre.
    pub origin: ModelPoint,
    /// In the order they were given.
    pub holes: Vec<HoleTag>,
}

/// One hole of a hole table and its tag, given once ([`hole_tags`]), so a model edit never renames it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HoleTag {
    pub hole: HoleRef,
    pub tag: String,
}

/// The cells a general table holds, typed in: each row its cells by column key, so a column moved
/// keeps its cells. An empty cell isn't saved, so a cell has one form; one no column reads is kept
/// and not drawn, as a parts list's field no column reads isn't.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct General {
    pub rows: Vec<BTreeMap<String, String>>,
}

/// How many configuration columns a new family table leaves out, by why.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LeftOut {
    /// Appearance columns: a look can't be printed as text.
    pub looks: usize,
    /// Those a table has no room for, that no row could fill or whose heading no label can show.
    pub others: usize,
}

/// The configurations a family table lists.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Family {
    /// The component whose configurations it lists (0: the design, the only one that has them).
    pub of: SheetComponent,
}

/// The bill of materials view a parts list lists, in the words [`crate::bom::ItemNumbers`] saves.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PartsList {
    /// The component listed (0: the design).
    pub of: SheetComponent,
    #[serde(default, rename = "levels")]
    pub view: View,
    #[serde(default, rename = "group_by")]
    pub group: Group,
    #[serde(default, rename = "group_children", skip_serializing_if = "std::ops::Not::not")]
    pub fold: bool,
    #[serde(default, rename = "show_purchased_contents", skip_serializing_if = "std::ops::Not::not")]
    pub purchased_contents: bool,
    #[serde(default)]
    pub mass: MassFormat,
}

/// How a parts list shows a mass.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MassFormat {
    pub unit: MassUnit,
    pub decimals: u8,
}

impl Default for MassFormat {
    fn default() -> Self {
        MassFormat { unit: MassUnit::Kg, decimals: 2 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MassUnit {
    G,
    Kg,
    Lb,
}

/// Where a table sits.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Place {
    /// The table's `corner` at that corner of the sheet's drawing space, moved by `offset_mm`.
    Anchor {
        corner: Corner,
        #[serde(default)]
        offset_mm: [f64; 2],
    },
    /// Its outline's top left, in millimetres from the sheet's lower-left corner.
    At([f64; 2]),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Corner {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

impl PartsList {
    /// The list as the bill of materials asks for it, unweighed.
    pub fn request(&self) -> Request {
        Request {
            source: self.of.component,
            view: self.view,
            group: self.group,
            fold: self.fold,
            mass: false,
            purchased_contents: self.purchased_contents,
        }
    }

    /// Each row of `list` as a table reads it, each followed by the rows listed under it, with its
    /// values formatted and its numbers kept as numbers, so a sort orders them by value. A length is
    /// shown in `units`. A value the row doesn't have is left out, so the
    /// cell shows [`solidcraft_geom::cell::MISSING`]: a mass not weighed or not known, or a quantity that doesn't add up.
    pub fn rows(&self, list: &Bom, units: &str) -> Vec<table::Row> {
        let row = |(item, row): (Vec<u32>, &bom::Row)| {
            // Free properties first, so that none takes the place of a value below.
            let mut fields = row.properties.clone();
            let values = [
                ("item", bom::item_text(&item)),
                ("part_number", row.part_number.clone()),
                ("name", row.names.join("; ")),
                ("description", row.description.clone()),
                ("revision", row.revision.clone()),
                ("material", row.materials.join("; ")),
                ("count", row.count.to_string()),
            ];
            fields.extend(values.map(|(key, value)| (key.to_string(), value)));
            // In one of the row it is listed under, as a list reads; `count` has them all.
            let quantity = match row.quantity {
                Quantity::Count => Some(row.each.to_string()),
                Quantity::Pieces(n) => Some(n.to_string()),
                Quantity::Length(mm) => Some(format_length(mm, units, 2)),
                Quantity::Unknown(_) => None,
            };
            let values = [("quantity", quantity), ("mass", self.mass.text(row.mass))];
            fields.extend(values.into_iter().filter_map(|(key, value)| Some((key.to_string(), value?))));
            let quantity = match row.quantity {
                // A count's text is a whole number, which is exact as a float up to 2^53.
                Quantity::Count => Some(row.each as f64),
                Quantity::Pieces(n) => Some(n),
                Quantity::Length(mm) => Some(mm),
                Quantity::Unknown(_) => None,
            };
            // The values the texts above are formatted from, filled here with them (`table::Row::numbers`).
            let mass = if let Mass::Grams(grams) = row.mass { Some(grams) } else { None };
            let numbers = [("count", Some(row.count as f64)), ("quantity", quantity), ("mass", mass)];
            let numbers = numbers.into_iter().filter_map(|(key, n)| Some((key.to_string(), n.filter(|n| n.is_finite())?))).collect();
            let kind = if row.assembly { table::RowKind::Assembly } else { table::RowKind::Part };
            // The item's path names the row and those it is listed under: one deeper per member.
            let level = u32::try_from(item.len().saturating_sub(1)).unwrap_or(u32::MAX);
            table::Row { key: row.key.text(), kind, purchased: row.bom_type == BomType::Purchased, fields, level, numbers }
        };
        list.rows_deep().into_iter().map(row).collect()
    }
}

impl Family {
    /// The definition a new family table starts with: a new table's NAME column, then one for each
    /// of `doc`'s configuration columns in its order, as many as a table has room for, with those
    /// left out. An appearance column is left out: a look can't be printed as text. A column reads the cell its configuration column gives each row
    /// ([`Document::config_key`]); its heading is the parameter's name, or the column's kind and
    /// target, in capitals, as a parts list's headings are; its name is the key in snake case,
    /// numbered on a clash, so a filter, an override or a sort can name it.
    pub fn table(&self, doc: &Document) -> (Table, LeftOut) {
        let mut table = Kind::Family(*self).default_table();
        let mut taken: HashSet<String> = table.columns.iter().map(|c| c.name.clone()).collect();
        let mut left_out = LeftOut::default();
        for config in &doc.configs.columns {
            if matches!(config, ConfigColumn::Appearance(_) | ConfigColumn::ComponentAppearance(_) | ConfigColumn::FaceAppearance(_)) {
                left_out.looks += 1;
                continue;
            }
            if table.columns.len() == table::MAX_COLUMNS {
                left_out.others += 1;
                continue;
            }
            // A column no row could fill, or with a heading no label can show, is left out.
            let key = doc.config_key(config);
            let heading = heading_of(&key);
            if key.len() > table::MAX_KEY_BYTES || cell::check_label(&heading).is_err() {
                left_out.others += 1;
                continue;
            }
            let name = unique(snake_case(&key), &mut taken);
            table.columns.push(Column { name, ..column(&key, &heading, HAlign::Left) });
        }
        (table, left_out)
    }

    /// Each of `doc`'s configurations as a table reads it, in its table's order: its name, the cell
    /// of each configuration column, and [`ACTIVE`] `yes` on the active one. A parameter's cell
    /// carries the value it gives the configuration ([`Document::configuration_values`]) as its
    /// number, so it sorts by value; one worked out from other parameters prints that value, a
    /// length in `units`, and one typed as a value prints as typed. Any other cell is text.
    pub fn rows(doc: &Document, units: &str) -> Vec<table::Row> {
        let configs = &doc.configs;
        let keys: Vec<String> = configs.columns.iter().map(|c| doc.config_key(c)).collect();
        let reads_values = configs.columns.iter().any(|c| matches!(c, ConfigColumn::Param(_)));
        let row = |config: &crate::config::ConfigRow| {
            let values = if reads_values { doc.configuration_values(config) } else { BTreeMap::new() };
            let (mut fields, mut numbers) = (BTreeMap::new(), BTreeMap::new());
            for ((key, column), cell) in keys.iter().zip(&configs.columns).zip(&config.cells) {
                // A key longer than a row's field key can be names no column, so it is left out.
                if key.len() > table::MAX_KEY_BYTES {
                    continue;
                }
                let value = match column {
                    ConfigColumn::Param(name) => values.get(name).copied().filter(|v| v.v.is_finite() && v.kind().is_some()),
                    _ => None,
                };
                let text = match value {
                    Some(v) if !expr::references(cell).is_empty() => shown(v, units).unwrap_or_else(|| cell.clone()),
                    _ => cell.clone(),
                };
                if let Some(v) = value {
                    numbers.insert(key.clone(), v.v);
                }
                fields.insert(key.clone(), text);
            }
            fields.insert("name".into(), config.name.clone());
            if configs.active.as_deref() == Some(config.name.as_str()) {
                fields.insert(ACTIVE.into(), "yes".into());
            }
            table::Row { key: config.name.clone(), kind: table::RowKind::Part, purchased: false, fields, level: 0, numbers }
        };
        configs.rows.iter().map(row).collect()
    }
}

/// A parameter's value as a table prints it, with two decimals: a length in `units`, as a parts
/// list's lengths are, an angle in degrees, a number as it is.
fn shown(value: expr::Value, units: &str) -> Option<String> {
    let number = |x: f64| format_number(x, 2, &NumberFormat::default()).ok().map(|shown| shown.text);
    match value.kind()? {
        expr::Kind::Length => Some(format_length(value.v, units, 2)),
        expr::Kind::Angle => number(value.v.to_degrees()).map(|n| format!("{n}°")),
        expr::Kind::Unitless => number(value.v),
    }
}

/// A configuration column's heading: a parameter's name, or the column's kind and target, in
/// capitals. [`Document::config_key`] names a column `<kind>:<target>`.
fn heading_of(key: &str) -> String {
    let heading = match key.split_once(':') {
        Some(("param", name)) => name.to_string(),
        Some((kind, target)) => format!("{} {target}", kind.replace('_', " ")),
        None => key.to_string(),
    };
    heading.to_uppercase()
}

/// `key` as a column's name: its letters and digits in lower case, each run of anything else one
/// `_`, short enough for a `_N` to follow. A configuration key starts with its kind's letters.
fn snake_case(key: &str) -> String {
    let mut name = String::new();
    for c in key.chars() {
        if c.is_ascii_alphanumeric() {
            name.push(c.to_ascii_lowercase());
        } else if !name.is_empty() && !name.ends_with('_') {
            name.push('_');
        }
    }
    // All ASCII, so any length is a character boundary.
    name.truncate(MAX_NAME_BYTES - 8);
    name.trim_end_matches('_').to_string()
}

/// `base`, or `base_2`, `base_3`... the first that isn't `taken`, which it then is.
fn unique(base: String, taken: &mut HashSet<String>) -> String {
    let mut name = base.clone();
    let mut n = 2;
    while !taken.insert(name.clone()) {
        name = format!("{base}_{n}");
        n += 1;
    }
    name
}

impl General {
    /// Each row as `table` reads it: its key its number from 1, its fields its cells, and each cell
    /// typed as a number also as that number ([`table::typed_number`]), so `1.2` sorts before `1.10`.
    /// A cell left empty is blank, not missing: every field a column reads is there.
    pub fn rows(&self, table: &Table) -> Vec<table::Row> {
        let read = table.columns.iter().flat_map(|c| &c.content.0).filter_map(|part| match part {
            Part::Field { key, .. } => Some((key.clone(), String::new())),
            Part::Text(_) => None,
        });
        let blank: BTreeMap<String, String> = read.collect();
        let row = |(at, cells): (usize, &BTreeMap<String, String>)| {
            let numbers = cells.iter().filter_map(|(key, text)| Some((key.clone(), table::typed_number(text)?))).collect();
            let mut fields = blank.clone();
            fields.extend(cells.clone());
            let key = at.saturating_add(1).to_string();
            table::Row { key, kind: table::RowKind::Part, purchased: false, fields, level: 0, numbers }
        };
        self.rows.iter().enumerate().map(row).collect()
    }

    /// The rules: a table's caps on its rows, its columns and its text, each cell's text by the
    /// cell text rule and not empty, each key one line, and no overrides on `table`.
    fn check(&self, table: &Table) -> Result<()> {
        require(self.rows.len() <= table::MAX_ROWS, || format!("a general table has at most {} rows", table::MAX_ROWS))?;
        require(table.overrides.is_empty(), || "a general table's cells are typed in: type into the cell instead (`general.cells`)".into())?;
        let mut bytes = 0usize;
        for cells in &self.rows {
            require(cells.len() <= table::MAX_COLUMNS, || format!("a general table's row has at most {} cells", table::MAX_COLUMNS))?;
            for (key, text) in cells {
                let one_line = !key.is_empty() && key.len() <= table::MAX_KEY_BYTES && cell::check_label(key).is_ok();
                require(one_line, || format!("a cell's column key is one line of 1 to {} bytes", table::MAX_KEY_BYTES))?;
                require(!text.is_empty(), || format!("the cell `{key}` is empty: an empty cell isn't saved"))?;
                cell::check_text([text.as_str()]).map_err(|e| DocError::Invalid(e.to_string()))?;
                bytes = bytes.saturating_add(key.len()).saturating_add(text.len());
                require(bytes <= table::MAX_TEXT_BYTES, || "a general table's cells hold at most 1 MiB of text".into())?;
            }
        }
        Ok(())
    }
}

impl Hole {
    /// The rules: its view is one of `d`'s; its origin a point a dimension could name; 1 to
    /// [`MAX_HOLES`] holes, each a hole reference by its own rule and none twice; each tag 1 to
    /// [`MAX_TAG_CHARS`] letters and digits, no two the same.
    fn check(&self, d: &Drawing) -> Result<()> {
        require(d.view(self.view).is_some(), || format!("its view {} isn't one of the drawing's views", ids::encode(self.view)))?;
        check_point(&self.origin)?;
        check_hole_count(self.holes.len())?;
        let (mut holes, mut tags) = (HashSet::new(), HashSet::new());
        for one in &self.holes {
            one.hole.validate()?;
            require(holes.insert((one.hole.feature, one.hole.point.as_deref())), || format!("it lists hole {} twice", hole_name(&one.hole)))?;
            let tag_ok = (1..=MAX_TAG_CHARS).contains(&one.tag.chars().count()) && one.tag.chars().all(|c| c.is_ascii_alphanumeric());
            require(tag_ok, || format!("a hole's tag is 1 to {MAX_TAG_CHARS} letters and digits"))?;
            require(tags.insert(one.tag.as_str()), || format!("two of its holes have the tag `{}`", one.tag))?;
        }
        Ok(())
    }
}

/// A hole table read from the model: what it shows, and where its tags stand.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HoleRead {
    /// A row a hole, in tag order: TAG; X and Y, its centre's place from the origin as the view shows
    /// it, signed, in the drawing's unit with a dimension's decimals; SIZE, its callout's lines. A hole
    /// that can't be read keeps its row and its tag, with no values.
    pub rows: Vec<table::Row>,
    /// Each row with no values, by tag, and why.
    pub missing: Vec<(String, String)>,
    /// Each hole read, by tag: its centre on the face it starts on, in the design's frame.
    pub centres: Vec<(String, Vec3)>,
}

impl Document {
    /// What `hole`, a hole table of `drawing` (one of this design's), shows of `model` (this
    /// design's) now. `Err` only for a view the drawing doesn't have.
    pub fn read_holes(&self, model: &Model, drawing: &Drawing, hole: &Hole) -> Result<HoleRead> {
        let basis = drawing.resolve_view(hole.view)?.basis;
        let style = hole_text_style(self, drawing);
        let origin = Lookup::new(self, model).corner(&hole.origin).map(|(at, _)| at).map_err(|status| format!("its origin: {}", why_of(&status)));
        let mut tagged: Vec<&HoleTag> = hole.holes.iter().collect();
        tagged.sort_by_key(|one| {
            let (letters, number) = split_tag(&one.tag);
            (letters.len(), letters.to_string(), number)
        });
        let mut out = HoleRead::default();
        for one in tagged {
            let read = origin.clone().and_then(|origin| {
                let (callout, size) = read_hole(self, model, &one.hole, &style)?;
                let centre = Vec3::new(callout.start[0], callout.start[1], callout.start[2]);
                Ok((on_view(&basis, centre - origin), size, centre))
            });
            let mut fields = BTreeMap::from([("tag".to_string(), one.tag.clone())]);
            let mut numbers = BTreeMap::new();
            match read {
                Ok((place, size, centre)) => {
                    for (key, mm) in [("x", place.x), ("y", place.y)] {
                        let text = signed_length(mm, &style.units, style.decimals);
                        numbers.extend(table::typed_number(&text).map(|n| (key.to_string(), n)));
                        fields.insert(key.to_string(), text);
                    }
                    fields.insert("size".to_string(), size);
                    out.centres.push((one.tag.clone(), centre));
                }
                // No values: the table shows its own mark for a field a row lacks.
                Err(why) => out.missing.push((one.tag.clone(), why)),
            }
            out.rows.push(table::Row { key: one.tag.clone(), kind: table::RowKind::Part, purchased: false, fields, level: 0, numbers });
        }
        Ok(out)
    }

    /// The sizes of `holes` on `model` (this design's), as a hole table on `drawing` (one of this
    /// design's) prints them and tags them by ([`hole_tags`]): each one's callout text. A hole that
    /// can't be read now is refused, with why: a new tag needs its size.
    pub fn hole_sizes(&self, model: &Model, drawing: &Drawing, holes: &[HoleRef]) -> Result<Vec<String>> {
        let style = hole_text_style(self, drawing);
        holes
            .iter()
            .map(|hole| {
                read_hole(self, model, hole, &style)
                    .map(|(_, size)| size)
                    .map_err(|why| DocError::Invalid(format!("hole {}: {why}", hole_name(hole))))
            })
            .collect()
    }
}

/// A hole table's text: the drawing's unit and language, with a dimension's decimals, so a table and a
/// dimension on one sheet read alike.
fn hole_text_style(doc: &Document, drawing: &Drawing) -> TextStyle {
    TextStyle { units: doc.drawing_units(drawing).to_string(), decimals: DEFAULT_DECIMALS, language: drawing.language.clone() }
}

/// `hole`'s callout on `model` and its text as one size, or why it can't be read.
fn read_hole(doc: &Document, model: &Model, hole: &HoleRef, style: &TextStyle) -> std::result::Result<(hole_callout::HoleCallout, String), String> {
    match hole_callout::resolve(doc, model, hole).map_err(|e| e.to_string())? {
        Resolution::Resolved { callout } => {
            let size = hole_callout::text(&callout, style).map_err(|e| e.to_string())?.join("\n");
            Ok((callout, size))
        }
        Resolution::Suppressed => Err(format!("feature {} is suppressed", hole.feature)),
        Resolution::Failed { reason } | Resolution::Lost { reason } => Err(reason),
    }
}

/// A length shown as a dimension shows it ([`reading_label`]), but signed: one that rounds to zero
/// shows no sign, so a hole on the origin reads `0.00`, never `-0.00`.
fn signed_length(mm: f64, units: &str, decimals: u8) -> String {
    let text = reading_label(LabelKind::Length, mm, units, decimals);
    match text.strip_prefix('-') {
        Some(magnitude) if magnitude.chars().all(|c| c == '0' || c == '.') => magnitude.to_string(),
        _ => text,
    }
}

/// Why a point can't be found, in words.
fn why_of(status: &Status) -> String {
    match status {
        Status::Ambiguous(_) => "its edges meet at more than one point".into(),
        Status::Suppressed => "a feature it needs is suppressed".into(),
        Status::Failed(why) | Status::Lost(why, _) | Status::Unmeasurable(why) => why.clone(),
        Status::Ok | Status::Overridden => "it can't be found".into(),
    }
}

/// A hole table lists 1 to [`MAX_HOLES`] holes. A command checks a list's count with it before
/// reading any hole, as each read costs a lookup in the model.
pub fn check_hole_count(count: usize) -> Result<()> {
    require((1..=MAX_HOLES).contains(&count), || format!("a hole table lists 1 to {MAX_HOLES} holes"))
}

/// A hole as a message names it: its feature, and its sketch point if it has one.
pub fn hole_name(hole: &HoleRef) -> String {
    match &hole.point {
        Some(point) => format!("{} at point `{point}`", hole.feature),
        None => hole.feature.to_string(),
    }
}

/// The tags of holes whose sizes are `sizes`, given in that order, beside `placed` (each kept hole's
/// size and tag). Holes of one size share a letter, and each is numbered after the last number its
/// letter has. A size is a hole's callout text, compared as text, so holes that read alike share a
/// letter and no tolerance is invented. A size no kept tag has takes the first letter no kept tag
/// uses: A to Z, then AA, AB, as columns are lettered.
pub fn hole_tags<'a>(placed: impl IntoIterator<Item = (&'a str, &'a str)>, sizes: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let mut letter_of: BTreeMap<&str, String> = BTreeMap::new();
    let mut last: BTreeMap<String, u64> = BTreeMap::new();
    for (size, tag) in placed {
        let (letters, number) = split_tag(tag);
        letter_of.entry(size).or_insert_with(|| letters.to_string());
        let at = last.entry(letters.to_string()).or_insert(0);
        *at = (*at).max(number);
    }
    let mut next = 1;
    sizes
        .into_iter()
        .map(|size| {
            let letters = letter_of.entry(size).or_insert_with(|| {
                loop {
                    let letters = column_letters(next);
                    next += 1;
                    if !last.contains_key(&letters) {
                        break letters;
                    }
                }
            });
            let number = last.entry(letters.clone()).or_insert(0);
            *number = number.saturating_add(1);
            format!("{letters}{number}")
        })
        .collect()
}

/// A tag's letters and its number: `B12` is `B` and 12. A tag with no number has 0.
fn split_tag(tag: &str) -> (&str, u64) {
    let digits = tag.find(|c: char| c.is_ascii_digit()).unwrap_or(tag.len());
    let (letters, number) = tag.split_at(digits);
    (letters, number.parse().unwrap_or(0))
}

/// The `n`th set of letters, from 1, as columns are lettered: A to Z, then AA.
fn column_letters(mut n: usize) -> String {
    let mut letters = Vec::new();
    while n > 0 {
        n -= 1;
        letters.extend(b"ABCDEFGHIJKLMNOPQRSTUVWXYZ".get(n % 26).map(|b| char::from(*b)));
        n /= 26;
    }
    letters.iter().rev().collect()
}

impl MassFormat {
    /// A mass as this format shows it, without its unit: the column's heading says that.
    fn text(self, mass: Mass) -> Option<String> {
        let Mass::Grams(grams) = mass else { return None };
        let in_grams = match self.unit {
            MassUnit::G => 1.0,
            MassUnit::Kg => 1000.0,
            MassUnit::Lb => 453.592_37,
        };
        format_number(grams / in_grams, self.decimals, &NumberFormat::default()).ok().map(|shown| shown.text)
    }
}

impl Kind {
    /// The definition a new table of this kind starts with: headings on top, rows of 7 mm, each
    /// column as wide as its text. A parts list's last column says how many balloons point at each
    /// row, for its editor alone: it isn't printed.
    pub fn default_table(&self) -> Table {
        let columns = match self {
            Kind::PartsList(_) => vec![
                column("item", "ITEM", HAlign::Centre),
                column("quantity", "QTY", HAlign::Centre),
                column("name", "NAME", HAlign::Left),
                column("description", "DESCRIPTION", HAlign::Left),
                column("material", "MATERIAL", HAlign::Left),
                column("mass", "MASS", HAlign::Right),
                Column { printed: false, ..column(BALLOONED, "BALLOONED", HAlign::Centre) },
            ],
            Kind::Family(_) => vec![column("name", "NAME", HAlign::Left)],
            Kind::General(_) => vec![column("a", "A", HAlign::Left), column("b", "B", HAlign::Left), column("c", "C", HAlign::Left)],
            Kind::Hole(_) => vec![
                column("tag", "TAG", HAlign::Centre),
                column("x", "X", HAlign::Right),
                column("y", "Y", HAlign::Right),
                column("size", "SIZE", HAlign::Left),
            ],
            Kind::Revision => vec![
                column("revision", "REV", HAlign::Centre),
                column("description", "DESCRIPTION", HAlign::Left),
                column("designer", "DRAWN", HAlign::Left),
                column("checker", "CHECKED", HAlign::Left),
                column("date", "DATE", HAlign::Left),
            ],
        };
        Table {
            columns,
            heading: Heading::Top,
            row_height_mm: 7.0,
            cap_mm: 3.5,
            outline_line_mm: 0.7,
            cell_line_mm: 0.35,
            filter: Filter::default(),
            overrides: Vec::new(),
            split: None,
            sort: Vec::new(),
        }
    }

    /// Where a new table of this kind goes: a parts list and a hole table at the top right, a revision
    /// table at the bottom left, a family table and a general table at the top left.
    pub fn default_place(&self) -> Place {
        let corner = match self {
            Kind::PartsList(_) | Kind::Hole(_) => Corner::TopRight,
            Kind::Revision => Corner::BottomLeft,
            Kind::Family(_) | Kind::General(_) => Corner::TopLeft,
        };
        Place::Anchor { corner, offset_mm: [0.0, 0.0] }
    }

    /// The component it lists, if it lists one.
    pub fn of(&self) -> Option<SheetComponent> {
        match self {
            Kind::PartsList(list) => Some(list.of),
            Kind::Family(family) => Some(family.of),
            Kind::Revision | Kind::General(_) | Kind::Hole(_) => None,
        }
    }
}

fn column(key: &str, heading: &str, align: HAlign) -> Column {
    let content = Content(vec![Part::Field { source: Source::Row, key: key.into(), format: Format::default() }]);
    Column { name: key.into(), heading: heading.into(), content, width_mm: None, align, printed: true }
}

impl Record for SheetTable {
    fn id(&self) -> u64 {
        self.id
    }
}

impl Drawing {
    /// The tables' rules (the module's notes).
    pub(super) fn check_tables(&self) -> Result<()> {
        self.tables.check(MAX_TABLES, "table", |t| self.check_table(t))
    }

    fn check_table(&self, t: &SheetTable) -> Result<()> {
        require(self.sheets.iter().any(|s| s.id == t.sheet), || format!("it is on sheet {}, which the drawing doesn't have", ids::encode(t.sheet)))?;
        t.table.validate().map_err(|e| DocError::Invalid(e.to_string()))?;
        if let Some(of) = t.kind.of() {
            let named = || format!("it lists source {}, which the drawing doesn't have", ids::encode(of.source));
            require(self.sources.iter().any(|s| s.id == of.source), named)?;
        }
        match &t.kind {
            Kind::PartsList(list) => {
                require(list.mass.decimals <= MAX_MASS_DECIMALS, || format!("a mass shows at most {MAX_MASS_DECIMALS} decimals"))?;
            }
            Kind::General(general) => general.check(&t.table)?,
            Kind::Hole(hole) => hole.check(self)?,
            Kind::Revision | Kind::Family(_) => {}
        }
        let [x, y] = match t.place {
            Place::Anchor { offset_mm, .. } => offset_mm,
            Place::At(at) => at,
        };
        require(is_paper_coordinate(x) && is_paper_coordinate(y), || format!("its place is at most {MAX_SIDE_MM} mm on paper"))
    }
}

#[cfg(test)]
#[path = "table_tests.rs"]
mod tests;
