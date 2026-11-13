//! Drawing templates (#3875): what a new drawing starts from. A template is a drawing with no
//! model: its sheets with their borders, title blocks and the text typed over their cells, its
//! projection, units, language, properties and revision numbering, and nothing that draws or
//! documents a design ([`check`]). So it is saved and read as a drawing is, under the same caps
//! and damage rules, and a new drawing is a copy of it with ids of its own ([`instantiate`]).
//!
//! A drawing saved as a template ([`from_drawing`]) keeps its format and leaves its model behind; a
//! template file reads as a drawing does, then is held to the template rules ([`read`]).
//!
//! A template keeps what sits on its sheets, such as a note, a revision table or a placed symbol (a
//! company mark in a corner), and its symbols (#13063); what sits on a view goes with the view. A new
//! drawing has them on its sheets' new ids, each symbol and placement with an id of its own and each
//! placement naming its symbol's new id.
//!
//! A template applied to a drawing ([`apply`]) gives each sheet its paper, drawing border and title
//! block, and moves nothing on it. A logo in a block travels with it, its file brought into the drawing.
//! Its symbols come in too: one with the name of the drawing's own replaces it where it stands, so an
//! updated company mark reaches every place the old one sat.
//!
//! The built-ins ([`BUILT_INS`]) are made in code when asked for, never stored: ISO A4 to A0, each
//! for a part and for an assembly, one sheet with the ISO border and the built-in title block
//! ([`default_block`](title_block::default_block)), the kind's general tolerances typed over its
//! cell ([`PART_NOTE`], [`ASSEMBLY_NOTE`]). A part's drawing starts from ISO A3 and an assembly's from
//! ISO A2 ([`default_for`]). A drawing saved as a template replaces or changes them: a copy of a
//! built-in with a company's changes, its own note among them, is a template like any other. A later
//! built-in never changes a drawing made from an earlier one: a drawing keeps copies, never names.
//! An older name (`iso-a3`, before the built-ins came in two kinds) reads as that size's part one.

use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use solidcraft_geom::cell::Part;
use solidcraft_geom::drawing_sheet::{self, DrawingBorderProfile, Orientation, PaperSize};
use solidcraft_geom::revision::RevisionRecord;
use solidcraft_geom::title_block::{self, CellKind, TitleBlock};

use super::Drawing;
use super::by_id::ById;
use super::image::ImageFile;
use super::note::Anchor;
use super::symbol::{PlacedSymbol, Symbol};
use crate::format::DRAWING;
use crate::ids::{self, IdSource};
use crate::{DocError, Document, Result};

/// The built-in templates, by name: each ISO size for a part and for an assembly.
pub const BUILT_INS: &[&str] = &[
    "iso-a4-part",
    "iso-a4-assembly",
    "iso-a3-part",
    "iso-a3-assembly",
    "iso-a2-part",
    "iso-a2-assembly",
    "iso-a1-part",
    "iso-a1-assembly",
    "iso-a0-part",
    "iso-a0-assembly",
];
/// Where every built-in's ids come from, so one built-in is always the same bytes.
const BUILT_IN_SEED: u64 = 3875;
/// The ISO sizes the built-ins come in, as their names and titles write them.
const SIZES: [(PaperSize, &str); 5] =
    [(PaperSize::A4, "A4"), (PaperSize::A3, "A3"), (PaperSize::A2, "A2"), (PaperSize::A1, "A1"), (PaperSize::A0, "A0")];

/// The general tolerances a part's built-ins carry.
pub const PART_NOTE: &str = "Unless stated: linear ±0.2 mm, angles ±0.5°. Break sharp edges 0.2 mm.";
/// The general tolerances an assembly's built-ins carry.
pub const ASSEMBLY_NOTE: &str = "Unless stated: assembled positions ±0.5 mm, angles ±1°. Part tolerances are on the part drawings.";
/// The title block cell the notes are typed over.
const TOLERANCES_CELL: &str = "general_tolerances";

/// The built-in a new drawing starts from when nothing else is chosen: ISO A3 for a part, ISO A2 for
/// an assembly.
pub fn default_for(assembly: bool) -> &'static str {
    if assembly { "iso-a2-assembly" } else { "iso-a3-part" }
}

/// Whether `doc`'s drawings are an assembly's, for the template they start from: it places components.
pub fn is_assembly(doc: &Document) -> bool {
    !doc.occurrences.is_empty()
}

/// The template rules: a drawing's ([`Drawing::validate`]), and nothing of a model. A template
/// with anything of one is refused, naming all it has.
pub fn check(template: &Drawing) -> Result<()> {
    template.validate()?;
    let has = [
        (!template.sources.is_empty(), "a source"),
        (!template.views.is_empty(), "views"),
        (!template.dimensions.is_empty(), "dimensions"),
        (!template.balloons.is_empty(), "balloons"),
        (template.revisions.ids().next().is_some(), "revision rows"),
        (template.sheets.iter().any(|sheet| sheet.documents.is_some()), "a sheet that documents a component"),
        (!template.sheet_images.is_empty(), "images placed on its sheets"),
    ];
    let has: Vec<&str> = has.into_iter().filter(|(has, _)| *has).map(|(_, what)| what).collect();
    if has.is_empty() {
        return Ok(());
    }
    Err(DocError::Invalid(format!("a template draws no design, and this one has {}", has.join(", "))))
}

/// A new drawing named `name` from `template`: a copy with the format this build writes, an id
/// none of `taken` has (its design's drawings'), and each sheet, symbol and placement an id of its
/// own, all from `id_source` and none of the template's. Each placement names its symbol's new id,
/// and what sits on a sheet names the sheet's.
pub fn instantiate(template: &Drawing, name: &str, taken: &BTreeSet<u64>, id_source: &mut IdSource) -> Result<Drawing> {
    check(template)?;
    let mut in_use: BTreeSet<u64> = template.ids().chain(taken.iter().copied()).collect();
    let mut fresh = || -> Result<u64> {
        let id = id_source.next(&in_use)?;
        in_use.insert(id);
        Ok(id)
    };
    let id = fresh()?;
    let sheets: BTreeMap<u64, u64> = template.sheets.iter().map(|sheet| Ok((sheet.id, fresh()?))).collect::<Result<_>>()?;
    let symbols: BTreeMap<u64, u64> = template.symbols.iter().map(|symbol| Ok((symbol.id, fresh()?))).collect::<Result<_>>()?;
    let mut placed = Vec::with_capacity(template.placed_symbols.len());
    for p in &template.placed_symbols {
        placed.push(Arc::new(PlacedSymbol {
            id: fresh()?,
            symbol: new_id(&symbols, p.symbol),
            anchor: on_new_sheet(&p.anchor, &sheets),
            ..(**p).clone()
        }));
    }
    let format = DRAWING.format_string(DRAWING.current());
    let mut d = Drawing { format, format_pending: DRAWING.pending_names(), id, name: name.into(), ..template.clone() };
    d.sheets = template.sheets.iter().map(|sheet| Arc::new(super::Sheet { id: new_id(&sheets, sheet.id), ..(**sheet).clone() })).collect();
    d.symbols = ById::default();
    d.symbols.add(template.symbols.iter().map(|s| Arc::new(Symbol { id: new_id(&symbols, s.id), ..(**s).clone() })));
    d.placed_symbols = ById::default();
    d.placed_symbols.add(placed);
    // A template holds nothing else on a sheet ([`check`]): no views, placed images or revision tags.
    for note in d.notes.iter_mut() {
        Arc::make_mut(note).anchor = on_new_sheet(&note.anchor, &sheets);
    }
    for table in d.tables.iter_mut() {
        Arc::make_mut(table).sheet = new_id(&sheets, table.sheet);
    }
    d.validate()?;
    Ok(d)
}

/// `old`'s new id in `ids` (old to new), which has every id of its kind the template has.
fn new_id(ids: &BTreeMap<u64, u64>, old: u64) -> u64 {
    ids.get(&old).copied().unwrap_or(old)
}

/// `anchor` on its sheet's new id (`sheets`, old to new), when it is on a sheet: a template's anchors
/// all are, since it has no views.
fn on_new_sheet(anchor: &Anchor, sheets: &BTreeMap<u64, u64>) -> Anchor {
    match anchor {
        Anchor::Sheet(old) => Anchor::Sheet(new_id(sheets, *old)),
        other => other.clone(),
    }
}

/// `drawing` as a template: its format, and nothing it draws or documents (its sources, views,
/// dimensions, balloons, placed images, revision rows, and the component each sheet documents),
/// nor its number, which drawings made from it get their own of. Its other properties, its revision
/// numbering, its logos and the text typed over its cells of plain text stay; text typed over a cell
/// that shows a field (a parameter, a value or a reference) goes with the design that field reads.
/// Its symbols stay, and so do the placements and notes on its sheets; those on a view, or on a view's
/// model, go with the views.
pub fn from_drawing(drawing: &Drawing) -> Result<Drawing> {
    let mut template = drawing.clone();
    template.sources.clear();
    template.views.clear();
    template.notes.retain(|note| note.anchor.view().is_none());
    template.placed_symbols.retain(|placed| placed.anchor.view().is_none());
    template.dimensions.clear();
    template.balloons.clear();
    template.sheet_images.clear();
    template.drop_unused_images();
    Arc::make_mut(&mut template.properties).remove(crate::metadata::PART_NUMBER);
    let mut revisions = RevisionRecord::default();
    (revisions.numbering, revisions.per_sheet) = (template.revisions.numbering, template.revisions.per_sheet);
    template.revisions = Arc::new(revisions);
    for sheet in &mut template.sheets {
        let sheet = Arc::make_mut(sheet);
        sheet.documents = None;
        let shows_a_field = |name: &String| {
            let cell = sheet.title_block.as_ref().and_then(|block| block.cells.iter().find(|cell| cell.name == *name));
            cell.is_some_and(
                |cell| matches!(&cell.kind, CellKind::Text(text) if text.content.0.iter().any(|part| matches!(part, Part::Field { .. }))),
            )
        };
        sheet.title_block_overrides.retain(|name, _| !shows_a_field(name));
    }
    check(&template)?;
    Ok(template)
}

/// A template file's text, read as a drawing is ([`Drawing::from_json`]: its size cap, the format
/// check and the damage rules), then held to the template rules.
pub fn read(text: &str) -> Result<Drawing> {
    let template = Drawing::from_json(text)?;
    check(&template)?;
    Ok(template)
}

/// What applying a template changed on one sheet ([`apply`]).
#[derive(Clone, Debug, PartialEq)]
pub struct Restyled {
    pub sheet: u64,
    pub paper: bool,
    pub border: bool,
    pub title_block: bool,
    /// The cells whose typed text is dropped: the new block has no text cell of that name.
    pub lost: Vec<String>,
}

/// What applying a template did with one of its symbols ([`apply`]).
#[derive(Clone, Debug, PartialEq)]
pub struct Brought {
    /// Its id in the drawing.
    pub symbol: u64,
    pub name: String,
    /// It replaced the drawing's symbol of its name, keeping that one's id, so its placements show it.
    pub replaced: bool,
    /// The prompts the placements answered that it doesn't ask for: those answers are dropped.
    pub lost: Vec<String>,
}

/// `drawing` in `template`'s sheet formats: each sheet takes the paper, drawing border and title
/// block of the template's sheet in its place (its last for the sheets past its own), and keeps the
/// text typed over a cell while the new block has a text cell of that name. The template's symbols
/// come in (`symbols_in`). Nothing on a sheet moves, and no placement comes. Answers the drawing,
/// what changed on each sheet that did, and each symbol brought.
pub fn apply(drawing: &Drawing, template: &Drawing, id_source: &mut IdSource) -> Result<(Drawing, Vec<Restyled>, Vec<Brought>)> {
    check(template)?;
    let mut d = drawing.clone();
    let logos = logos_in(&mut d, template, id_source)?;
    let brought = symbols_in(&mut d, template, id_source)?;
    let mut changed = Vec::new();
    for at in 0..d.sheets.len() {
        let Some(from) = template.sheets.get(at).or(template.sheets.last()) else { continue };
        let block = from.title_block.as_ref().map(|block| with_logos(block, &logos));
        let Some(sheet) = d.sheets.get_mut(at) else { continue };
        let paper = sheet.sheet != from.sheet;
        let (border, title_block) = (sheet.border != from.border, sheet.title_block != block);
        // A sheet's typed text is always its own block's ([`Drawing::validate`]), so the same block loses none.
        if !(paper || border || title_block) {
            continue;
        }
        let sheet = Arc::make_mut(sheet);
        sheet.sheet = from.sheet;
        sheet.border = from.border;
        let lost = sheet.set_title_block(block);
        changed.push(Restyled { sheet: sheet.id, paper, border, title_block, lost });
    }
    // A logo goes with the block that showed it.
    d.drop_unused_images();
    d.validate()?;
    Ok((d, changed, brought))
}

/// Each symbol `template` defines, brought into `d` (#13063, Q3): one with the name of one of `d`'s
/// replaces that one's pieces where it stands, so its placements show it, and drops their answers to
/// prompts it doesn't ask for; one `d` hasn't got comes with an id of its own; one the same as `d`'s
/// changes nothing.
fn symbols_in(d: &mut Drawing, template: &Drawing, id_source: &mut IdSource) -> Result<Vec<Brought>> {
    let named: BTreeMap<String, u64> = d.symbols.iter().map(|own| (own.name.clone(), own.id)).collect();
    // Collected once, not per new symbol as `allocate_id` would.
    let mut in_use: BTreeSet<u64> = d.ids().collect();
    let mut brought = Vec::new();
    for symbol in &template.symbols {
        let (id, replaced) = match named.get(&symbol.name) {
            Some(&own) if d.symbols.by_id(own).is_some_and(|own| own.pieces == symbol.pieces) => continue,
            Some(&own) => (own, true),
            None => {
                let id = id_source.next(&in_use)?;
                in_use.insert(id);
                (id, false)
            }
        };
        let new = Symbol { id, ..(**symbol).clone() };
        let prompts = new.prompts();
        let mut lost = BTreeSet::new();
        for placed in d.placed_symbols.iter_mut().filter(|placed| placed.symbol == id) {
            let stale: Vec<String> = placed.answers.keys().filter(|key| !prompts.contains(key.as_str())).cloned().collect();
            if !stale.is_empty() {
                Arc::make_mut(placed).answers.retain(|key, _| prompts.contains(key.as_str()));
                lost.extend(stale);
            }
        }
        match d.symbols.index_of(id).and_then(|at| d.symbols.get_mut(at)) {
            Some(own) => *own = Arc::new(new),
            None => d.symbols.add([Arc::new(new)]),
        }
        brought.push(Brought { symbol: id, name: symbol.name.clone(), replaced, lost: lost.into_iter().collect() });
    }
    Ok(brought)
}

/// Each logo file `template` shows, brought into `d` once however many of its sheets show it: the
/// template's id for it, to `d`'s. One only a sheet past `d`'s last shows goes with what's unused.
fn logos_in(d: &mut Drawing, template: &Drawing, id_source: &mut IdSource) -> Result<BTreeMap<u64, u64>> {
    let mut brought = BTreeMap::new();
    for logo in template.logos() {
        if let Entry::Vacant(entry) = brought.entry(logo) {
            let file = template.images.by_id(logo).ok_or_else(|| DocError::Invalid(format!("the template has no image {}", ids::encode(logo))))?;
            let id = d.allocate_id(id_source)?;
            entry.insert(d.keep_file(ImageFile { id, ..file.clone() }));
        }
    }
    Ok(brought)
}

/// `block` with each logo cell naming its file's id in the drawing (`logos`, from [`logos_in`]).
fn with_logos(block: &TitleBlock, logos: &BTreeMap<u64, u64>) -> TitleBlock {
    let mut block = block.clone();
    for cell in &mut block.cells {
        if let CellKind::Text(text) = &mut cell.kind {
            text.image = text.image.and_then(|logo| logos.get(&logo).copied());
        }
    }
    block
}

/// The built-in `name`'s paper size, its title ("ISO A3") and whether it is an assembly's; none for
/// another name. An older name without its kind is the part's.
fn parse(name: &str) -> Option<(PaperSize, String, bool)> {
    let rest = name.strip_prefix("iso-")?;
    let (size, assembly) = match rest.split_once('-') {
        None => (rest, false),
        Some((size, "part")) => (size, false),
        Some((size, "assembly")) => (size, true),
        Some(_) => return None,
    };
    let (paper, label) = SIZES.into_iter().find(|(_, label)| label.to_ascii_lowercase() == size)?;
    Some((paper, format!("ISO {label}"), assembly))
}

/// The title of the built-in template `name` ("ISO A3"), without making it; none for another name.
pub fn title(name: &str) -> Option<String> {
    parse(name).map(|(_, title, _)| title)
}

/// Whether the built-in template `name` is an assembly's; none for another name.
pub fn assembly(name: &str) -> Option<bool> {
    parse(name).map(|(.., assembly)| assembly)
}

/// The built-in template `name` ([`BUILT_INS`]), made now; none for another name.
pub fn built_in(name: &str) -> Option<Result<Drawing>> {
    let (size, title, assembly) = parse(name)?;
    // A4 is used upright (ISO 5457); the larger sizes lie flat.
    let orientation = if size == PaperSize::A4 { Orientation::Portrait } else { Orientation::Landscape };
    Some(made(&title, drawing_sheet::Sheet::standard(size, orientation), if assembly { ASSEMBLY_NOTE } else { PART_NOTE }))
}

/// One sheet of `paper` with the ISO border and the built-in title block, `note` typed over its
/// general tolerances.
fn made(title: &str, paper: drawing_sheet::Sheet, note: &str) -> Result<Drawing> {
    let mut id_source = IdSource::seeded(BUILT_IN_SEED);
    let mut d = Drawing::new(title, "en", "Sheet 1", paper, Some(DrawingBorderProfile::iso()), &BTreeSet::new(), &mut id_source)?;
    if let Some(first) = d.sheets.first_mut() {
        let first = Arc::make_mut(first);
        first.title_block = Some(title_block::default_block());
        first.title_block_overrides.insert(TOLERANCES_CELL.into(), note.into());
    }
    check(&d)?;
    Ok(d)
}

#[cfg(test)]
#[path = "template_tests.rs"]
mod tests;
