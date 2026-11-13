//! A drawing's records pasted into another drawing, or into the same one (#13065): each record gets
//! a new id, and every link between them follows it through one old-to-new map
//! ([`Drawing::paste_records`]). What a copy keeps and what it drops is said here once, for every
//! kind of record a sheet holds:
//!
//! - a view keeps its settings and goes on its pasted sheet, or on the sheet pasted onto; a
//!   projected view whose parent isn't pasted, or every one when the two drawings' projection
//!   angles differ, becomes a base view that looks as it did ([`View::as_base`]);
//! - a dimension measures in its view's copy and shows no model dimension: the one it was copied
//!   from still does, and each shows once;
//! - a balloon names its parts list's copy when the list is pasted too, the list itself when this
//!   drawing has it (a paste into the drawing it came from), else none, so it reads the lists it
//!   can, as after a delete;
//! - a note goes on its sheet's copy, or on its view's copy;
//! - an image file with the same bytes as one this drawing has is used again;
//! - a sheet keeps its name when no sheet has it, else gets the next free one, and names this
//!   drawing's design; a parts list too;
//! - revision tags are never pasted: each shows a row of its own drawing's revision record;
//! - each kind's copies keep their originals' order, which is their id order: the copies' new ids
//!   are in that order too, so they paint as the originals do.
//!
//! The model references stay as they were: moving them onto another part is the caller's.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use solidcore_record::Vec2;

use super::balloon::Balloon;
use super::dimension::Dimension;
use super::image::{ImageFile, SheetImage};
use super::note::{Anchor, Note};
use super::table::{Kind, SheetTable};
use super::view::{View, ViewKind, shifted};
use super::{Drawing, OnSheet, Sheet, SheetComponent};
use crate::ids::IdSource;
use crate::{DocError, Result, ids};

/// Where a paste puts what it copies.
#[derive(Clone, Copy, Debug)]
pub struct Placement {
    /// The sheet the views and tables copied without their sheet go on.
    pub onto: u64,
    /// How far every copied view moves on paper.
    pub by: Vec2,
    /// This drawing's source: the design a pasted base view draws, and a pasted sheet or parts list
    /// names.
    pub source: u64,
}

/// What [`Drawing::paste_records`] made.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Pasted {
    /// Each record copied, by its kind, the id it had and its copy's, in the order they were made.
    pub ids: Vec<(&'static str, u64, u64)>,
    /// The views' copies, in the copy's order.
    pub views: Vec<u64>,
    /// The base views among them, by the ids they were copied from.
    pub old_bases: BTreeSet<u64>,
    /// The same base views, by their own ids.
    pub bases: BTreeSet<u64>,
    /// The new sheets, in the copy's order.
    pub sheets: Vec<u64>,
    /// The new tables, in the copy's order.
    pub tables: Vec<u64>,
    /// How many projected views became base views because the projection angles differ.
    pub based_across: usize,
}

impl Drawing {
    /// Copies of `what` of drawing `from` (which may be this one, as it was), and of the sheets
    /// `sheets` of it whole, put in this drawing as `place` says, each with a new id from
    /// `id_source`: the rules are the module's. Every new id is allocated at once. Refused, with
    /// this drawing unchanged, when the copy names a view `from` doesn't have, or a projected view
    /// that can't be resolved; the caller checks the drawing's rules after.
    pub fn paste_records(&mut self, from: &Drawing, what: &OnSheet, sheets: &[u64], place: Placement, id_source: &mut IdSource) -> Result<Pasted> {
        // Every kind a sheet holds, with no `..`: a new kind fails to build here until it says how
        // it is pasted. Revision tags show a row of their own drawing's record: never pasted.
        let OnSheet { views, dimensions, balloons, images, tags: _, tables, notes } = what;
        let files: BTreeSet<u64> = images.iter().filter_map(|id| from.sheet_images.by_id(*id)).map(|image| image.image).collect();
        let mut new: BTreeMap<u64, u64> = BTreeMap::new();
        for file in files.iter().filter_map(|id| from.images.by_id(*id)) {
            if let Some(same) = self.images.iter().find(|x| x.format == file.format && x.data == file.data) {
                new.insert(file.id, same.id);
            }
        }
        let fresh_files: Vec<u64> = files.iter().filter(|id| !new.contains_key(id)).copied().collect();
        let kinds: [(&'static str, &[u64]); 8] = [
            ("sheet", sheets),
            ("view", views),
            ("dimension", dimensions),
            ("balloon", balloons),
            ("note", notes),
            ("image file", &fresh_files),
            ("image", images),
            ("table", tables),
        ];
        let count = kinds.iter().map(|(_, list)| list.len()).sum();
        // Given out in order, and each kind's list is in id order, so the copies keep their
        // originals' order: a drawing keeps its records, and paints them, in id order.
        let mut fresh = self.allocate_ids(count, id_source)?;
        fresh.sort_unstable();
        let mut fresh = fresh.into_iter();
        let mut made = Pasted::default();
        for (kind, old) in kinds.iter().flat_map(|(kind, list)| list.iter().map(move |old| (*kind, *old))) {
            let id = fresh.next().ok_or_else(|| DocError::Invalid("no free id".into()))?;
            new.insert(old, id);
            made.ids.push((kind, old, id));
        }
        let map = |old: u64| new.get(&old).copied();
        let sheet_of = |old: u64| map(old).unwrap_or(place.onto);

        // Views first, as the drawing still is: a refusal leaves it unchanged.
        let across = from.projection != self.projection;
        let mut copies = Vec::with_capacity(views.len());
        for old in views {
            let missing = || DocError::Invalid(format!("the copy has no view {}", ids::encode(*old)));
            let (original, id) = from.view(*old).zip(map(*old)).ok_or_else(missing)?;
            let mut view = View { id, sheet: sheet_of(original.sheet), place: shifted(original, place.by), ..original.clone() };
            match original.kind {
                ViewKind::Projected { parent, side } => match map(parent).filter(|_| !across) {
                    Some(parent) => view.kind = ViewKind::Projected { parent, side },
                    None => {
                        let resolved = from.resolve_view(*old)?;
                        view = view.as_base(&resolved, place.source, from.view_centre(*old)? + place.by);
                        made.based_across += usize::from(across);
                    }
                },
                ViewKind::Base { view: direction, .. } => view.kind = ViewKind::Base { source: place.source, view: direction },
            }
            if matches!(view.kind, ViewKind::Base { .. }) {
                made.old_bases.insert(*old);
                made.bases.insert(id);
            }
            made.views.push(id);
            copies.push(Arc::new(view));
        }
        // The parts lists this drawing had before the paste, for the balloons.
        let lists: BTreeSet<u64> = self.tables.iter().filter(|t| matches!(t.kind, Kind::PartsList(_))).map(|t| t.id).collect();
        for original in from.sheets.iter().filter(|sheet| sheets.contains(&sheet.id)) {
            let name = if self.sheets.iter().any(|x| x.name == original.name) { self.free_sheet_name() } else { original.name.clone() };
            let documents = original.documents.map(|c| SheetComponent { source: place.source, ..c });
            let id = sheet_of(original.id);
            self.sheets.push(Arc::new(Sheet { id, name, documents, ..Sheet::clone(original) }));
            made.sheets.push(id);
        }
        self.add_views(copies);
        self.add_dimensions(dimensions.iter().filter_map(|old| {
            let original = from.dimension(*old)?;
            Some(Arc::new(Dimension { id: map(*old)?, view: map(original.view)?, from_model: None, ..original.clone() }))
        }));
        self.balloons.add(balloons.iter().filter_map(|old| {
            let original = from.balloons.by_id(*old)?;
            let parts_list = original.parts_list.and_then(|list| map(list).or_else(|| lists.contains(&list).then_some(list)));
            Some(Arc::new(Balloon { id: map(*old)?, view: map(original.view)?, parts_list, ..original.clone() }))
        }));
        self.notes.add(notes.iter().filter_map(|old| {
            let original = from.notes.by_id(*old)?;
            let anchor = match original.anchor {
                Anchor::Sheet(sheet) => Anchor::Sheet(sheet_of(sheet)),
                _ => original.anchor.on_view(map(original.anchor.view()?)?),
            };
            Some(Arc::new(Note { id: map(*old)?, anchor, ..original.clone() }))
        }));
        self.images.add(fresh_files.iter().filter_map(|old| Some(Arc::new(ImageFile { id: map(*old)?, ..from.images.by_id(*old)?.clone() }))));
        self.sheet_images.add(images.iter().filter_map(|old| {
            let original = from.sheet_images.by_id(*old)?;
            Some(Arc::new(SheetImage { id: map(*old)?, sheet: sheet_of(original.sheet), image: map(original.image)?, ..original.clone() }))
        }));
        let mut pasted_tables = Vec::with_capacity(tables.len());
        for old in tables {
            let Some((original, id)) = from.tables.by_id(*old).zip(map(*old)) else { continue };
            let mut table = SheetTable { id, sheet: sheet_of(original.sheet), ..original.clone() };
            if let Kind::PartsList(list) = &mut table.kind {
                list.of.source = place.source;
            }
            made.tables.push(id);
            pasted_tables.push(Arc::new(table));
        }
        self.tables.add(pasted_tables);
        Ok(made)
    }
}

#[cfg(test)]
#[path = "paste_tests.rs"]
mod tests;
