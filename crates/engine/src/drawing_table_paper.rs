//! A sheet table placed (#3874 P8b): its rows read, its columns fitted to what the fonts really set,
//! and the whole where its record says. A table's corner can only be named once its size is known, so
//! it is laid out at the origin first and placed second; one measure serves both.
//!
//! **Cost:** a parts list reads the bill of materials, weighed only when a column shows a mass (about
//! 135 ms on a bolted bracket against 0.2 ms unweighed). The provider keeps the answer until the table,
//! the model or the paper changes, so this never runs per frame.

use std::cell::RefCell;
use std::collections::HashMap;

use solidcraft_doc::bom::{self, Tree};
use solidcraft_doc::drawing::table::{Corner, Kind, Place, SheetTable};
use solidcraft_doc::drawing::{Drawing, Sheet};
use solidcraft_geom::cell::{HAlign, Part};
use solidcraft_geom::paper::text::TextRun;
use solidcraft_geom::paper::{Item, ObjectKind};
use solidcraft_geom::table::{Heading, Row, Table, TableLayout};
use solidcraft_geom::{Aabb2, Vec2};

use crate::Session;
use crate::drawing_annotation_paper::{OnPaper, Text, text_width};

/// `layout`'s lines and its cells' words with capitals `cap_mm` high. Each line stands on its baseline,
/// its left, middle or right at its anchor as its cell aligns. A word that can't be set says why.
pub(crate) fn on_paper(layout: &TableLayout, cap_mm: f64, object: u64, kind: ObjectKind) -> OnPaper {
    let mut group = layout.paper(object, kind);
    let (mut set, mut fonts, mut why) = (HashMap::new(), Vec::new(), None);
    for cell in std::iter::once(&layout.heading).chain(&layout.rows).flat_map(|row| &row.cells) {
        for line in &cell.lines {
            let text = set.entry(line.text.as_str()).or_insert_with(|| {
                let text = Text::set(Some(&line.text), cap_mm);
                fonts.extend(text.fonts.iter().cloned());
                why = why.take().or_else(|| text.why.clone());
                text
            });
            let before = match cell.align {
                HAlign::Left => 0.0,
                HAlign::Centre => text.half.x,
                HAlign::Right => text.half.x * 2.0,
            };
            let start = line.at - Vec2::X * before;
            group.items.extend(text.runs.iter().map(|run| Item::Text(TextRun { origin: start + run.origin, ..run.clone() })));
        }
    }
    OnPaper { group, fonts, why }
}

/// A table laid out and placed, and the rows it lists before its filter.
pub(crate) struct Placed {
    pub rows: Vec<Row>,
    pub layout: TableLayout,
}

/// Where a sheet's tables are anchored: inside its drawing border, or its paper without one.
pub(crate) fn drawing_space(sheet: &Sheet) -> Aabb2 {
    let border = sheet.border.as_ref().and_then(|border| border.layout(&sheet.sheet).ok()).map(|layout| layout.drawing_border);
    border.unwrap_or_else(|| {
        let size = sheet.sheet.size_mm().unwrap_or(Vec2::ZERO);
        Aabb2 { min: Vec2::ZERO, max: size }
    })
}

/// `record` of `d` on a sheet with drawing space `space`, or why it can't be.
pub(crate) fn place(s: &Session, d: &Drawing, record: &SheetTable, space: Aabb2) -> Result<Placed, String> {
    let rows = rows_of(s, d, record)?;
    let table = &record.table;
    // A table repeats its words, and a word costs a font lookup: each one is set once.
    let widths = RefCell::new(HashMap::<String, f64>::new());
    let measure = |text: &str, cap_mm: f64| *widths.borrow_mut().entry(text.to_string()).or_insert_with(|| text_width(text, cap_mm));
    let sized = table.layout(&rows, Vec2::ZERO, &measure).map_err(|e| e.to_string())?;
    let corner = corner_of(record.place, table.heading, sized.outline.size(), space);
    let layout = table.layout(&rows, corner, &measure).map_err(|e| e.to_string())?;
    Ok(Placed { rows, layout })
}

/// The point [`Table::layout`] takes for a table of `size` placed at `place`: the right end of its
/// heading's outer edge.
fn corner_of(place: Place, heading: Heading, size: Vec2, space: Aabb2) -> Vec2 {
    // The outline's top left, whichever way it is named.
    let top_left = match place {
        Place::At([x, y]) => Vec2::new(x, y),
        Place::Anchor { corner, offset_mm: [dx, dy] } => {
            let (x, y) = match corner {
                Corner::TopLeft => (space.min.x, space.max.y),
                Corner::TopRight => (space.max.x - size.x, space.max.y),
                Corner::BottomLeft => (space.min.x, space.min.y + size.y),
                Corner::BottomRight => (space.max.x - size.x, space.min.y + size.y),
            };
            Vec2::new(x + dx, y + dy)
        }
    };
    let edge = if heading == Heading::Top { top_left.y } else { top_left.y - size.y };
    Vec2::new(top_left.x + size.x, edge)
}

fn rows_of(s: &Session, d: &Drawing, record: &SheetTable) -> Result<Vec<Row>, String> {
    let list = match &record.kind {
        Kind::Revision => return Ok(d.revisions.table_rows(d.revisions.per_sheet.then_some(record.sheet))),
        Kind::PartsList(list) => list,
    };
    let from = s.drawn(d, list.of.source).map_err(|why| why.to_string())?;
    let state = from.model.state();
    let mut tree = Tree::new(&from.doc, &state);
    let request = bom::Request { mass: shows_mass(&record.table), ..list.request() };
    let listed = bom::list(&mut tree, &request).map_err(|e| e.to_string())?;
    Ok(list.rows(&listed, s.doc.drawing_units(d)))
}

/// Whether a column shows a row's mass, which is what makes a list weigh its parts.
fn shows_mass(table: &Table) -> bool {
    table.columns.iter().flat_map(|column| &column.content.0).any(|part| matches!(part, Part::Field { key, .. } if key == "mass"))
}

#[cfg(test)]
#[path = "drawing_table_paper_tests.rs"]
mod tests;
