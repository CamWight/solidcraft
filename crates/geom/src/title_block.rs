//! Title blocks: the block of cells that identifies a drawing and what its sheet documents, as
//! data in sheet millimetres.
//!
//! A [`TitleBlock`] is a definition: its size, its two line widths and its cells. Each [`Cell`] is
//! a rectangle measured from the block's lower-right corner (x at most 0, y at least 0), with a
//! label and a value: text from cell content ([`crate::cell`]), or the projection symbol. The block
//! sits on the drawing border's lower-right corner, so one definition serves every sheet it fits.
//! [`TitleBlock::validate`] checks a definition on its own, as when one is read, and
//! [`TitleBlock::layout`] places it on one sheet by the cell text rule: each cell's rectangle, where
//! its label goes and the box its value goes in. Values are set in their boxes once the paper scene
//! has text; nothing here is saved or drawn yet.
//!
//! The outline is drawn `outline_line_mm` wide and the lines between cells `cell_line_mm`, each
//! centred on its edge.

use serde::{Deserialize, Serialize};

use crate::cell::{self, Content, Fit, HAlign};
use crate::drawing_sheet::SheetLayout;
use crate::paper::MAX_LINE_MM;
use crate::{Aabb2, Vec2};

/// The most cells a block may have.
pub const MAX_CELLS: usize = 64;
const MIN_BLOCK_SIDE_MM: f64 = 1.0;
const MAX_BLOCK_SIDE_MM: f64 = 1000.0;
const MAX_NAME_BYTES: usize = 64;

/// A title block's definition.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TitleBlock {
    /// The block's width and height.
    pub size_mm: Vec2,
    pub outline_line_mm: f64,
    pub cell_line_mm: f64,
    pub cells: Vec<Cell>,
}

/// One cell of a block.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Cell {
    /// Snake case, and no other cell of the block has it.
    pub name: String,
    /// `[x0, y0, x1, y1]` from the block's lower-right corner: `-width <= x0 < x1 <= 0` and
    /// `0 <= y0 < y1 <= height`.
    pub rect_mm: [f64; 4],
    /// None: the value has the whole cell.
    pub label: Option<Label>,
    pub kind: CellKind,
}

/// A cell's label, such as "Created by": one line at its top left.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Label {
    pub text: String,
    pub cap_mm: f64,
}

/// What a cell's value is. Externally tagged (`{"text": {...}}`, `"projection_symbol"`), as serde
/// ignores extra fields on an internally tagged unit variant.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum CellKind {
    Text(TextCell),
    /// The projection symbol, drawn from the drawing's projection (`drawing.projection`), which the
    /// views read too: never fixed in the block.
    ProjectionSymbol,
}

/// A text value: its content, its cap height, where its lines sit across the value box (left when
/// not given) and what it should fit (nothing when not given).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TextCell {
    pub content: Content,
    pub cap_mm: f64,
    #[serde(default)]
    pub align: HAlign,
    #[serde(default)]
    pub fit: Fit,
}

/// A block placed on one sheet: what a writer draws, before the values are set.
#[derive(Clone, Debug, PartialEq)]
pub struct TitleBlockLayout {
    pub outline: Aabb2,
    pub outline_line_mm: f64,
    pub cell_line_mm: f64,
    /// One for each of the definition's cells, in its order.
    pub cells: Vec<CellLayout>,
}

/// One cell placed on a sheet.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CellLayout {
    pub rect: Aabb2,
    /// Where the label's baseline starts, when the cell has one.
    pub label_at: Option<Vec2>,
    pub value_box: Aabb2,
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("title block: {0}")]
pub struct Invalid(pub &'static str);

type Result<T> = std::result::Result<T, Invalid>;

fn require(condition: bool, message: &'static str) -> Result<()> {
    if condition { Ok(()) } else { Err(Invalid(message)) }
}

/// A lowercase letter, then lowercase letters, digits and underscores: at most 64 in all.
fn is_snake_case(name: &str) -> bool {
    let mut bytes = name.bytes();
    name.len() <= MAX_NAME_BYTES
        && bytes.next().is_some_and(|b| b.is_ascii_lowercase())
        && bytes.all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

/// Whether two rectangles share more than an edge.
fn overlap(a: Aabb2, b: Aabb2) -> bool {
    a.min.x < b.max.x && b.min.x < a.max.x && a.min.y < b.max.y && b.min.y < a.max.y
}

impl Cell {
    /// The cell's rectangle with the block's lower-right corner at `corner`.
    fn rect(&self, corner: Vec2) -> Aabb2 {
        let [x0, y0, x1, y1] = self.rect_mm;
        Aabb2 { min: corner + Vec2::new(x0, y0), max: corner + Vec2::new(x1, y1) }
    }

    fn value_box(&self, rect: Aabb2) -> Aabb2 {
        cell::value_box(rect, self.label.as_ref().map(|label| label.cap_mm))
    }
}

impl TitleBlock {
    /// Checks a definition on its own, counting its cells first: call it when one is read.
    /// [`Self::layout`] calls it too.
    pub fn validate(&self) -> Result<()> {
        require(self.cells.len() <= MAX_CELLS, "a title block has at most 64 cells")?;
        let Vec2 { x: width, y: height } = self.size_mm;
        let sides = MIN_BLOCK_SIDE_MM..=MAX_BLOCK_SIDE_MM;
        require(sides.contains(&width) && sides.contains(&height), "each side of a title block is 1 to 1000 mm")?;
        let lines = [self.outline_line_mm, self.cell_line_mm];
        require(lines.into_iter().all(|line| line > 0.0 && line <= MAX_LINE_MM), "a title block's lines are above 0 and at most 10 mm wide")?;
        for (i, cell) in self.cells.iter().enumerate() {
            let earlier = || self.cells.iter().take(i);
            require(is_snake_case(&cell.name), "a cell's name is snake case and at most 64 characters")?;
            require(earlier().all(|other| other.name != cell.name), "no two cells have the same name")?;
            let [x0, y0, x1, y1] = cell.rect_mm;
            require(-width <= x0 && x0 < x1 && x1 <= 0.0 && 0.0 <= y0 && y0 < y1 && y1 <= height, "a cell lies inside its block")?;
            let rect = cell.rect(Vec2::ZERO);
            require(earlier().all(|other| !overlap(other.rect(Vec2::ZERO), rect)), "no two cells overlap")?;
            if let Some(label) = &cell.label {
                require(label.cap_mm.is_finite() && label.cap_mm > 0.0, "a label's cap height is finite and above 0")?;
                let one_line = label.text.len() <= cell::MAX_TEXT_BYTES && !label.text.chars().any(char::is_control);
                require(one_line, "a label is one line of at most 4096 bytes")?;
            }
            let room = cell.value_box(rect).size();
            require(room.x > 0.0 && room.y > 0.0, "a cell has room for its value below its label")?;
            if let CellKind::Text(text) = &cell.kind {
                text.content.validate().map_err(|cell::Invalid(message)| Invalid(message))?;
                require(text.cap_mm > 0.0 && text.cap_mm <= room.y, "a value's cap height is above 0 and fits its value box")?;
            }
        }
        Ok(())
    }

    /// The block on the lower-right corner of `sheet`'s drawing border, which must hold it.
    pub fn layout(&self, sheet: &SheetLayout) -> Result<TitleBlockLayout> {
        self.validate()?;
        let border = sheet.drawing_border;
        let (size, room) = (self.size_mm, border.size());
        let fits = border.min.is_finite() && border.max.is_finite() && size.x <= room.x && size.y <= room.y;
        require(fits, "a title block fits inside the drawing border")?;
        let corner = Vec2::new(border.max.x, border.min.y);
        let cells = self
            .cells
            .iter()
            .map(|cell| {
                let rect = cell.rect(corner);
                let label_at = cell.label.as_ref().map(|label| cell::label_at(rect, label.cap_mm));
                CellLayout { rect, label_at, value_box: cell.value_box(rect) }
            })
            .collect();
        Ok(TitleBlockLayout {
            outline: Aabb2 { min: corner - Vec2::new(size.x, 0.0), max: corner + Vec2::new(0.0, size.y) },
            outline_line_mm: self.outline_line_mm,
            cell_line_mm: self.cell_line_mm,
            cells,
        })
    }
}

#[cfg(test)]
#[path = "title_block_tests.rs"]
mod tests;
