//! The sheet cache: each view's last lines, kept in the design file so a drawing opens with them and
//! needn't project what hasn't changed. One manifest per drawing, `cache/drawings/<id>/manifest.json`,
//! and one entry per view, `cache/drawings/<id>/views/<view>.json` (Deflate). By view, not by sheet: a
//! view's lines are centred on the view, so moving it needs no new entry.
//!
//! The lines are the engine's results, as the model cache's bodies are ([`super::native_cache`]): a
//! miss is never an error, an entry is written only when [`PackageWriter::room`] says so, a plain JSON
//! design has none, and a mismatch means drawing again, so there is no format file or migration.
//! They are shown as they are until drawn again; [`ReadLines::current`] says whether this build wrote
//! them for this very `document.json`.
//!
//! A save writes what it is given and keeps the entries of each other drawing from the file it came
//! from, so a drawing not opened this session keeps its lines. Those are read and deflated again
//! rather than copied raw: a damaged entry stops there, and a failed read costs that entry, never the
//! save.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use solidcraft_doc::ids;
use solidcraft_geom::paper::{Group, ObjectKind, Scene};
use solidcraft_geom::{Aabb2, Vec2};

use super::*;

const FORMAT: &str = "solidcraft-sheet-cache/1";
const FOLDER: &str = "cache/drawings";
/// The most views one drawing's cache holds, a manifest's size and one view's lines.
const MAX_VIEWS: usize = 10_000;
const MAX_MANIFEST: usize = 16 << 20;
const MAX_VIEW_BYTES: usize = 64 << 20;

/// What a save keeps of its drawings' last lines.
#[derive(Clone, Debug, Default)]
pub struct SheetCaches {
    /// The lines of each drawing that has lines current now.
    pub lines: Vec<SheetLines>,
    /// The file the other drawings' entries are kept from, if there is one.
    pub keep_from: Option<String>,
}

/// One drawing's lines.
#[derive(Clone, Debug)]
pub struct SheetLines {
    pub drawing: u64,
    pub views: Vec<SavedView>,
}

/// One view's lines, centred on the view.
#[derive(Clone, Debug, PartialEq)]
pub struct SavedView {
    pub view: u64,
    /// What it was drawn from, opaque here: the engine compares it.
    pub key: String,
    pub group: Arc<Group>,
    /// Its boundary, centred as its lines are.
    pub boundary: Aabb2,
    /// Where its plane's origin falls on paper, centred as its lines are: its frame, with its basis
    /// and scale.
    pub origin_mm: Vec2,
}

/// A drawing's lines as a file has them.
#[derive(Clone, Debug, PartialEq)]
pub struct ReadLines {
    /// This build wrote them for the `document.json` asked about.
    pub current: bool,
    pub views: Vec<SavedView>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    format: String,
    build: String,
    document: String,
    /// Each view's id and key.
    views: Vec<(String, String)>,
}

#[derive(Serialize)]
struct ViewOut<'a> {
    group: &'a Group,
    boundary: [f64; 4],
    origin_mm: [f64; 2],
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ViewIn {
    group: Group,
    boundary: [f64; 4],
    origin_mm: [f64; 2],
}

fn folder(drawing: u64) -> String {
    format!("{FOLDER}/{}/", ids::encode(drawing))
}

/// Writes `caches` into the container of `doc`, whose `document.json` is `document`.
pub(super) fn write(writer: &mut PackageWriter, doc: &Document, caches: &SheetCaches, document: &[u8], budget: usize) -> Result<()> {
    let in_design = |id: u64| doc.drawings.iter().any(|d| d.id == id);
    let mut written = BTreeSet::new();
    for lines in caches.lines.iter().filter(|lines| in_design(lines.drawing)) {
        if write_drawing(writer, lines, document, budget)? {
            written.insert(lines.drawing);
        }
    }
    let keep: Vec<u64> = doc.drawings.iter().map(|d| d.id).filter(|id| !written.contains(id)).collect();
    if keep.is_empty() {
        return Ok(());
    }
    let Some(mut old) = caches.keep_from.as_deref().and_then(|path| open_regular_file(path).ok()).and_then(|source| PackageReader::new(source).ok()) else { return Ok(()) };
    keep.into_iter().try_for_each(|drawing| keep_drawing(writer, &mut old, drawing, budget))
}

/// Writes one drawing's lines, or none of them when they don't fit: whether it did.
fn write_drawing(writer: &mut PackageWriter, lines: &SheetLines, document: &[u8], budget: usize) -> Result<bool> {
    if lines.views.is_empty() || lines.views.len() > MAX_VIEWS {
        return Ok(false);
    }
    let mut parts = Vec::with_capacity(lines.views.len());
    for v in &lines.views {
        let [min, max] = [v.boundary.min, v.boundary.max];
        let out = ViewOut { group: &v.group, boundary: [min.x, min.y, max.x, max.y], origin_mm: [v.origin_mm.x, v.origin_mm.y] };
        let bytes = serde_json::to_vec(&out).map_err(invalid)?;
        if bytes.len() > MAX_VIEW_BYTES {
            return Ok(false);
        }
        parts.push(bytes);
    }
    let views = lines.views.iter().map(|v| (ids::encode(v.view), v.key.clone())).collect();
    let manifest = Manifest { format: FORMAT.into(), build: native_cache::build(), document: digest(document), views };
    let manifest = serde_json::to_vec(&manifest).map_err(invalid)?;
    let size = parts.iter().map(Vec::len).fold(manifest.len(), usize::saturating_add);
    if manifest.len() > MAX_MANIFEST || !writer.room(parts.len() + 1, size, budget) {
        log::info!("the last lines of drawing {} do not fit the design file: saved without them", ids::encode(lines.drawing));
        return Ok(false);
    }
    let folder = folder(lines.drawing);
    writer.part(&format!("{folder}manifest.json"), &manifest, true)?;
    for (v, bytes) in lines.views.iter().zip(&parts) {
        writer.part(&format!("{folder}views/{}.json", ids::encode(v.view)), bytes, true)?;
    }
    Ok(true)
}

/// Keeps one drawing's entries from the file it came from, all of them or none.
fn keep_drawing<R: Read + Seek>(writer: &mut PackageWriter, old: &mut PackageReader<R>, drawing: u64, budget: usize) -> Result<()> {
    let folder = folder(drawing);
    let Ok(names) = old.names() else { return Ok(()) };
    let names: Vec<String> = names.into_iter().filter(|name| name.starts_with(&folder)).collect();
    if names.is_empty() || names.len() > MAX_VIEWS + 1 {
        return Ok(());
    }
    let mut parts = Vec::with_capacity(names.len());
    for name in &names {
        let Ok(bytes) = old.limited_part(name, MAX_VIEW_BYTES) else { return Ok(()) };
        parts.push(bytes);
    }
    let size = parts.iter().map(Vec::len).fold(0, usize::saturating_add);
    if !writer.room(parts.len(), size, budget) {
        return Ok(());
    }
    names.iter().zip(&parts).try_for_each(|(name, bytes)| writer.part(name, bytes, true))
}

/// The lines of `drawing` in the file at `path`, with `document` the digest of the `document.json`
/// they would have to be for ([`design_file_digest`]). Any failure is a miss.
pub fn read_sheet_lines(path: &str, drawing: u64, document: &str) -> Option<ReadLines> {
    let mut reader = PackageReader::new(open_regular_file(path).ok()?).ok()?;
    read(&mut reader, drawing, document).ok()
}

fn read<R: Read + Seek>(reader: &mut PackageReader<R>, drawing: u64, document: &str) -> Result<ReadLines> {
    let folder = folder(drawing);
    let manifest: Manifest = serde_json::from_slice(&reader.limited_part(&format!("{folder}manifest.json"), MAX_MANIFEST)?).map_err(invalid)?;
    if manifest.format != FORMAT || manifest.views.len() > MAX_VIEWS {
        return Err(invalid("sheet cache key"));
    }
    // A development build of unknown source never trusts a cache.
    let current = manifest.build == native_cache::build() && manifest.document == document && env!("SOLIDCRAFT_SOURCE_COMMIT") != "unknown";
    let mut views = Vec::with_capacity(manifest.views.len());
    for (id, key) in manifest.views {
        // A damaged view is that view's miss: the others still show.
        let read = ids::decode(&id).and_then(|view| {
            let bytes = reader.limited_part(&format!("{folder}views/{id}.json"), MAX_VIEW_BYTES).ok()?;
            view_from(&bytes, view, key)
        });
        views.extend(read);
    }
    Ok(ReadLines { current, views })
}

/// One view's lines, when they keep what a scene takes and are that view's.
fn view_from(bytes: &[u8], view: u64, key: String) -> Option<SavedView> {
    let saved: ViewIn = serde_json::from_slice(bytes).ok()?;
    let [x0, y0, x1, y1] = saved.boundary;
    let [ox, oy] = saved.origin_mm;
    if ![x0, y0, x1, y1, ox, oy].into_iter().all(f64::is_finite) || saved.group.object != view || saved.group.kind != ObjectKind::View {
        return None;
    }
    let mut scene =
        Scene { size_mm: Vec2 { x: solidcraft_geom::paper::MAX_SIDE_MM, y: solidcraft_geom::paper::MAX_SIDE_MM }, groups: vec![saved.group] };
    scene.validate().ok()?;
    let group = Arc::new(scene.groups.pop()?);
    Some(SavedView { view, key, group, boundary: Aabb2 { min: Vec2 { x: x0, y: y0 }, max: Vec2 { x: x1, y: y1 } }, origin_mm: Vec2 { x: ox, y: oy } })
}

/// The digest of a container's `document.json`: what its sheet caches are for.
pub fn design_bytes_digest(bytes: &[u8]) -> Option<String> {
    let mut reader = PackageReader::new(Cursor::new(bytes)).ok()?;
    reader.document(16 << 20).ok().map(|document| digest(&document))
}

/// [`design_bytes_digest`] of the file at `path`, without reading the rest of it.
pub fn design_file_digest(path: &str) -> Option<String> {
    let mut reader = PackageReader::new(open_regular_file(path).ok()?).ok()?;
    reader.document(16 << 20).ok().map(|document| digest(&document))
}

#[cfg(test)]
#[path = "sheet_cache_tests.rs"]
mod tests;
