//! Writing library folders (#12982): a new library, and new, changed, copied and removed entries.
//! Each write is one file and crash-safe ([`crate::safe_write`]): a new file never replaces one,
//! a changed file replaces only the one a scan read, and a link at its name is never followed.
//! A change names the revision it was made from and is refused when the entry on disk has another.
//! A removed entry moves to the library's own bin, `.removed/<kind>/`, which no scan reads.
//! Read-only places, the built-in library and unavailable libraries are never written. Every path
//! follows the network rules a design's links follow, as a scan's reads do.
//!
//! After a write, the library written is read again with a scan's reads and caps, and the
//! [`Libraries`] returned hold it, so the next command sees the write.
use super::{Kind, LIMITS, Libraries, Library, Listed, MANIFEST_SUFFIX, Place, Reason, Scanned, Seen, Status, checked, list, read_library};
use crate::catalog::{Definition, Entry, Provenance, identifier, text};
use std::path::Path;
use std::sync::Arc;

/// The library's own bin for removed entries; a scan reads only the kind folders, never this.
pub const BIN: &str = ".removed";

/// What a write made or changed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Written {
    /// The library's id.
    pub library: String,
    /// The entry's id, when the write was an entry's.
    pub entry: Option<String>,
    pub revision: Option<String>,
    /// The file written, or moved to the bin, from the library's folder, with `/` between names.
    pub file: String,
}

fn join(dir: &str, name: &str) -> String {
    Path::new(dir).join(name).to_string_lossy().into_owned()
}

/// `e`, a failed read or write at `path`, said with the path once: a folder check's refusal names
/// the path it refused already, so it is left as it is.
fn at(path: &str, e: impl std::fmt::Display) -> String {
    let e = e.to_string();
    if e.contains(path) { e } else { format!("{path}: {e}") }
}

/// The reads and writes a write makes, each checked first as a scan's reads are.
mod checked_write {
    use crate::vfs::{self, check_design_path, check_new_design_path};
    use std::io::Result;
    use std::path::Path;

    pub(super) fn create(path: &str, bytes: &[u8]) -> Result<()> {
        check_new_design_path(path, None)?;
        crate::safe_write::create_atomic(Path::new(path), bytes)
    }
    /// `still` is asked again just before the new file takes the name.
    pub(super) fn replace(path: &str, bytes: &[u8], still: impl FnOnce() -> Result<()>) -> Result<()> {
        check_design_path(path, None)?;
        crate::safe_write::write_atomic_no_follow_if(Path::new(path), bytes, still)
    }
    pub(super) fn move_to(from: &str, to: &str) -> Result<()> {
        check_design_path(from, None)?;
        check_new_design_path(to, None)?;
        crate::safe_write::move_no_replace(Path::new(from), Path::new(to))
    }
    pub(super) fn make_dirs(path: &str) -> Result<()> {
        check_new_design_path(path, None)?;
        vfs::create_new_dir_all(path)
    }
    pub(super) fn real_dir(path: &str) -> Result<bool> {
        check_new_design_path(path, None)?;
        vfs::real_dir(path)
    }
}

/// The folders a write in `folder` passes inside its place, each a real folder or not made yet, as
/// a scan reads only real folders ([`crate::vfs::real_dir`]): the library's folder itself in a folder
/// place (a scan lists it as one of the place's folders), then `folder/below[0]`, `…/below[1]`, …
/// A folder not made yet ends the check: nothing is below it.
fn real_folders(place: &Place, folder: &str, below: &[&str]) -> Result<(), String> {
    let mut dirs: Vec<String> = matches!(place, Place::Folder(_)).then(|| folder.to_string()).into_iter().collect();
    let mut dir = folder.to_string();
    for name in below {
        dir = join(&dir, name);
        dirs.push(dir.clone());
    }
    for dir in &dirs {
        if !checked_write::real_dir(dir).map_err(|e| at(dir, e))? {
            break;
        }
    }
    Ok(())
}

impl Libraries {
    /// The library `id` as this set uses it, and the scan it came from.
    fn find(&self, id: &str) -> Result<(&Arc<Scanned>, &Library), String> {
        let mut statuses = self.set.libraries().map(|(_, status)| status);
        for scan in self.set.places() {
            for library in &scan.libraries {
                let status = statuses.next();
                if library.id() != Some(id) {
                    continue;
                }
                return match status {
                    Some(Status::Available) => Ok((scan, library)),
                    Some(Status::Unavailable(why)) => Err(format!("the library {id} at {} {why}", library.shown_at())),
                    None => Err(format!("no library {id} is listed")),
                };
            }
        }
        Err(format!("no library {id} is listed"))
    }

    /// The library `id`, when it may be written: never the built-in one, nor one in a read-only
    /// place, nor one the scan found read-only.
    fn writable(&self, id: &str) -> Result<(&Arc<Scanned>, &Library, &str), String> {
        let (scan, library) = self.find(id)?;
        let Some(folder) = library.folder() else { return Err(Reason::BuiltIn.to_string()) };
        if self.my_library.iter().chain(&self.rows).any(|row| row.place == scan.place && row.read_only) {
            return Err(format!("the library {id} is in a read-only place; allow changes to the place first"));
        }
        if let Some(why) = library.read_only() {
            return Err(format!("the library {id} can't be changed: {why}"));
        }
        Ok((scan, library, folder))
    }

    /// The entry `id` with its library, and the file it was read from.
    fn listed_entry(&self, id: &str) -> Result<(&Library, &Listed, Entry), String> {
        let library_id = id.split_once('/').map(|(library, _)| library).ok_or_else(|| format!("{id}: an entry id is <library id>/<entry id>"))?;
        let (_, library) = self.find(library_id)?;
        let listed = library
            .entries
            .iter()
            .find(|l| l.entry().is_ok_and(|e| e.id == id))
            .ok_or_else(|| format!("no entry {id} is listed in {}", library.shown_at()))?;
        let entry = listed.entry().map_err(|why| why.to_string())?.clone();
        Ok((library, listed, entry))
    }

    /// These libraries with the one in `folder` (inside `scan`) read again, or with `scan`'s place
    /// scanned again when `folder` is `None` (a new library).
    fn read_again(&self, scan: &Arc<Scanned>, folder: Option<&str>) -> Libraries {
        let fresh = match folder {
            None => super::scan(&scan.place),
            Some(folder) => {
                let mut fresh = Scanned::clone(scan);
                let (library, seen) = read_folder(folder);
                if let Some(old) = fresh.libraries.iter_mut().find(|l| l.folder() == Some(folder)) {
                    *old = library;
                    // Its listings too, so the check after a write finds the place as it left it.
                    fresh.seen.replace(folder, seen);
                }
                fresh
            }
        };
        let fresh = Arc::new(fresh);
        let places = self.set.places().iter().map(|p| if Arc::ptr_eq(p, scan) { Arc::clone(&fresh) } else { Arc::clone(p) }).collect();
        Libraries { my_library: self.my_library.clone(), rows: self.rows.clone(), states: self.states.clone(), set: super::Set::shared(places) }
    }

    /// A new library called `name` in the listed place at `place`: in a folder place, a new
    /// subfolder named after it; in a library place, that folder itself, which must hold nothing
    /// yet. The manifest is written last, so a write stopped part way leaves no library.
    pub fn create_library(&self, place: &str, name: &str, provenance: Provenance) -> Result<(Libraries, Written), String> {
        if !text(name, 128) {
            return Err("a library needs a 1-128 byte name".into());
        }
        provenance.validate().map_err(|e| e.to_string())?;
        let row = self.listed(place).ok_or_else(|| format!("no place is listed at {place}"))?;
        if row.read_only {
            return Err(format!("{place} is read-only; allow changes to the place first"));
        }
        let scan = self.set.places().iter().find(|s| s.place == row.place).ok_or_else(|| format!("{place} hasn't been scanned yet"))?;
        // The place as it is now decides, not the session's scan: another app may have made a library
        // there since.
        let now = super::scan(&row.place);
        // A missing place is made; any other reason it can't be read stops the write.
        if let Status::Unavailable(why) = &now.status
            && *why != Reason::Missing
        {
            return Err(format!("{place} {why}"));
        }
        let folder = match &row.place {
            Place::Folder(dir) => {
                let taken = match checked::dirs_in(dir, LIMITS.libraries_per_folder) {
                    Ok(taken) => taken,
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
                    Err(e) => return Err(at(dir, e)),
                };
                if taken.len() >= LIMITS.libraries_per_folder {
                    return Err(format!("{dir} holds the most folders a place reads ({})", LIMITS.libraries_per_folder));
                }
                let names: Vec<String> = taken.iter().map(|d| super::file_label(d)).collect();
                // Taken whatever the case, but a folder of this very name that a stopped create left
                // ([`unused`]) is used again, not passed over.
                let left = |folder: &String| unused(&join(dir, folder)).unwrap_or(false);
                let taken = |candidate: &str| names.iter().any(|n| n.to_lowercase() == candidate.to_lowercase() && !(n == candidate && left(n)));
                let stem = unique(&stem(name), taken).ok_or_else(|| format!("{dir}: every folder name for {name} is taken"))?;
                join(dir, &stem)
            }
            Place::Library(dir) => {
                if !now.libraries.iter().all(|l| l.status == Status::Unavailable(Reason::NoManifest)) {
                    return Err(format!("{dir} holds a library already; a library place is one library"));
                }
                dir.clone()
            }
            Place::BuiltIn => return Err(Reason::BuiltIn.to_string()),
        };
        // The folder must be empty: never turn someone's folder into a library.
        if !unused(&folder)? {
            return Err(format!("{folder} isn't empty; a new library needs an empty folder"));
        }
        let others = self.set.places().iter().filter(|p| !Arc::ptr_eq(p, scan)).flat_map(|p| &p.libraries);
        let ids: Vec<&str> = others.chain(&now.libraries).filter_map(Library::id).collect();
        let id = new_library_id(&ids, name);
        let header = super::Header { format: super::FORMAT.into(), id: id.clone(), name: name.into(), provenance };
        let bytes = serde_json::to_vec_pretty(&header).map_err(|e| e.to_string())?;
        let manifest = join(&folder, &format!("{}{MANIFEST_SUFFIX}", stem(name)));
        for kind in [Kind::Material, Kind::Appearance] {
            real_folders(&row.place, &folder, &[kind.folder()])?;
            checked_write::make_dirs(&join(&folder, kind.folder())).map_err(|e| at(&folder, e))?;
        }
        checked_write::create(&manifest, &bytes).map_err(|e| at(&manifest, e))?;
        let written = Written { library: id, entry: None, revision: None, file: super::file_label(&manifest) };
        Ok((self.read_again(scan, None), written))
    }

    /// A new entry in the library `library`, under a new id made from its name.
    pub fn add_entry(&self, library: &str, definition: Definition, provenance: Provenance) -> Result<(Libraries, Written), String> {
        let (scan, _, folder) = self.writable(library)?;
        let now = on_disk_now(library, folder)?;
        let local = new_entry_id(&now, &definition);
        let entry = Entry::with_id(library, &local, definition, provenance).map_err(|e| e.to_string())?;
        let file = self.write_new(&scan.place, &now, folder, &entry)?;
        let written = Written { library: library.into(), entry: Some(entry.id), revision: Some(entry.revision), file };
        Ok((self.read_again(scan, Some(folder)), written))
    }

    /// A copy of the entry `id` (from any library, the built-in one included) in the library `to`,
    /// under a new id there; it keeps its provenance.
    pub fn copy_entry(&self, id: &str, to: &str) -> Result<(Libraries, Written), String> {
        let (_, _, source) = self.listed_entry(id)?;
        self.add_entry(to, source.definition, source.provenance)
    }

    /// The entry `id` with a new definition (and provenance, when given), when the file still
    /// holds `revision`, the one the caller read.
    pub fn edit_entry(
        &self,
        id: &str,
        revision: &str,
        definition: Definition,
        provenance: Option<Provenance>,
    ) -> Result<(Libraries, Written), String> {
        let (library, listed, old) = self.listed_entry(id)?;
        let library_id = library.id().unwrap_or_default().to_string();
        let (scan, _, folder) = self.writable(&library_id)?;
        if Kind::of(&definition) != listed.kind() {
            return Err(format!("{id} is a {}; its kind can't change", old.definition.kind()));
        }
        real_folders(&scan.place, folder, &[listed.kind().folder()])?;
        let path = join(&join(folder, listed.kind().folder()), listed.label());
        let on_disk = unchanged(&path, listed.kind(), &library_id, revision)?;
        let local = id.split_once('/').map(|(_, local)| local).unwrap_or_default();
        let entry = Entry::with_id(&library_id, local, definition, provenance.unwrap_or(old.provenance)).map_err(|e| e.to_string())?;
        let bytes = encode(&entry)?;
        within_library_cap(&on_disk_now(&library_id, folder)?, on_disk, bytes.len() as u64)?;
        // Asked again once the new file is written beside it, so a change another app made since the
        // first check is kept, not written over.
        let still = || unchanged(&path, listed.kind(), &library_id, revision).map(drop).map_err(std::io::Error::other);
        checked_write::replace(&path, &bytes, still).map_err(|e| at(&path, e))?;
        let file = listed.file();
        let written = Written { library: library_id, entry: Some(entry.id), revision: Some(entry.revision), file };
        Ok((self.read_again(scan, Some(folder)), written))
    }

    /// Move the entry `id`'s file to its library's bin, when the file still holds `revision`.
    pub fn remove_entry(&self, id: &str, revision: &str) -> Result<(Libraries, Written), String> {
        let (library, listed, _) = self.listed_entry(id)?;
        let library_id = library.id().unwrap_or_default().to_string();
        let (scan, _, folder) = self.writable(&library_id)?;
        real_folders(&scan.place, folder, &[listed.kind().folder()])?;
        let path = join(&join(folder, listed.kind().folder()), listed.label());
        unchanged(&path, listed.kind(), &library_id, revision)?;
        let to = to_bin(&scan.place, &path, folder, listed.kind(), listed.label())?;
        let written = Written { library: library_id, entry: Some(id.into()), revision: Some(revision.into()), file: to };
        Ok((self.read_again(scan, Some(folder)), written))
    }

    /// Write `entry` to a new file in its kind folder, named after it, never over another file.
    fn write_new(&self, place: &Place, library: &Library, folder: &str, entry: &Entry) -> Result<String, String> {
        let kind = Kind::of(&entry.definition);
        real_folders(place, folder, &[kind.folder()])?;
        let dir = join(folder, kind.folder());
        let files = match checked::files_in(&dir, LIMITS.entries_per_kind) {
            Ok(files) => files,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) if e.kind() == std::io::ErrorKind::InvalidData => {
                return Err(format!("{dir} holds the most entries a library reads ({})", LIMITS.entries_per_kind));
            }
            Err(e) => return Err(at(&dir, e)),
        };
        if files.len() >= LIMITS.entries_per_kind {
            return Err(format!("{dir} holds the most entries a library reads ({})", LIMITS.entries_per_kind));
        }
        let taken: Vec<String> = files.iter().map(|f| super::file_label(f).to_lowercase()).collect();
        let name = file_name(entry.definition.name(), |n| taken.iter().any(|t| t == n)).ok_or_else(|| format!("{dir}: every file name is taken"))?;
        let bytes = encode(entry)?;
        within_library_cap(library, 0, bytes.len() as u64)?;
        checked_write::make_dirs(&dir).map_err(|e| at(&dir, e))?;
        let path = join(&dir, &name);
        checked_write::create(&path, &bytes).map_err(|e| at(&path, e))?;
        Ok(super::entry_file(kind, &name))
    }
}

/// The library in `folder` read now, with a scan's reads and caps, and the listings it made.
fn read_folder(folder: &str) -> (Library, Seen) {
    let mut seen = Seen::default();
    let library = match list(folder, &LIMITS, &mut seen) {
        Ok(listing) => read_library(folder, listing, &LIMITS, &mut seen),
        Err(reason) => Library::unavailable(folder, reason),
    };
    (library, seen)
}

/// The library `id` in `folder` as it is on disk now. A write takes its new id and its size from
/// this, not from the session's scan, which a write by another app since may have made old.
fn on_disk_now(id: &str, folder: &str) -> Result<Library, String> {
    let (library, _) = read_folder(folder);
    match library.status() {
        Status::Unavailable(why) => Err(format!("the library {id} at {folder} {why}")),
        Status::Available if library.id() != Some(id) => Err(format!("{folder} holds another library now; list the places again")),
        Status::Available => Ok(library),
    }
}

/// An entry file's bytes, within the entry cap a scan keeps.
fn encode(entry: &Entry) -> Result<Vec<u8>, String> {
    let bytes = serde_json::to_vec_pretty(entry).map_err(|e| e.to_string())?;
    if bytes.len() as u64 > LIMITS.entry_bytes {
        return Err(format!("the entry is larger than {} bytes", LIMITS.entry_bytes));
    }
    Ok(bytes)
}

/// A write that would take the library past its cap would make it unavailable at the next
/// scan: refused before it is written. `old` is the size of the file it replaces.
fn within_library_cap(library: &Library, old: u64, new: u64) -> Result<(), String> {
    if library.bytes.saturating_sub(old).saturating_add(new) > LIMITS.library_bytes {
        return Err(format!("the library would be larger than {} bytes", LIMITS.library_bytes));
    }
    Ok(())
}

/// The file at `path` still holds the entry at `revision` (read again, as a scan reads it):
/// its size on disk, or why not.
fn unchanged(path: &str, kind: Kind, library: &str, revision: &str) -> Result<u64, String> {
    let changed = || format!("{path}: the entry changed since it was read; list it again");
    let bytes = match super::read_capped(path, super::Cap::EntryBytes, LIMITS.entry_bytes) {
        Ok(bytes) => bytes,
        Err(Reason::Missing) => return Err(format!("{path}: removed since it was read")),
        Err(_) => return Err(changed()),
    };
    match super::read_entry(&bytes, kind, library) {
        Ok(entry) if entry.revision == revision => Ok(bytes.len() as u64),
        _ => Err(changed()),
    }
}

/// Move `path` into the bin of the library in `folder` under its own name, or `name (2).json` when the
/// bin holds that name: the file's name from the library's folder. The bin is never listed (it can grow
/// past any cap): each name is tried, and a name already there refuses the move, so nothing in the bin
/// is ever replaced. The one place a removal happens, so a different bin is a change here only.
fn to_bin(place: &Place, path: &str, folder: &str, kind: Kind, label: &str) -> Result<String, String> {
    let bin = join(&join(folder, BIN), kind.folder());
    real_folders(place, folder, &[BIN, kind.folder()])?;
    checked_write::make_dirs(&bin).map_err(|e| at(&bin, e))?;
    let stem = label.get(..label.len().saturating_sub(5)).filter(|_| super::ends_with(label, ".json")).unwrap_or(label);
    for candidate in std::iter::once(stem.to_string()).chain((2..=LIMITS.entries_per_kind + 2).map(|n| format!("{stem} ({n})"))) {
        let name = format!("{candidate}.json");
        match checked_write::move_to(path, &join(&bin, &name)) {
            Ok(()) => return Ok(format!("{BIN}/{}", super::entry_file(kind, &name))),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(at(path, e)),
        }
    }
    Err(format!("{bin}: every name for {label} is taken"))
}

/// Whether a new library may be made in `folder`: it isn't there, or holds nothing, or holds only
/// the empty kind folders a create stopped before its manifest leaves, so trying again works.
fn unused(folder: &str) -> Result<bool, String> {
    let kinds = [Kind::Material, Kind::Appearance].map(|kind| join(folder, kind.folder()));
    let dirs = match checked::dirs_in(folder, kinds.len()) {
        Ok(dirs) => dirs,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::InvalidData => return Ok(false),
        Err(e) => return Err(at(folder, e)),
    };
    let no_files = |dir: &str| checked::files_in(dir, 0).is_ok_and(|files| files.is_empty());
    let empty = |dir: &String| kinds.contains(dir) && no_files(dir.as_str()) && checked::dirs_in(dir, 0).is_ok_and(|dirs| dirs.is_empty());
    Ok(no_files(folder) && dirs.iter().all(empty))
}

/// A new library id from `name`: `user.<slug>`, then `-2`, `-3`, against `ids`, every library id
/// the places list. Never `solidcraft.`, which is kept for SolidCraft's own libraries.
fn new_library_id(ids: &[&str], name: &str) -> String {
    unique_id("user.", &super::slug(name), "library", |id| ids.contains(&id))
}

/// A new entry id in `library` from the entry's kind and name: `material-<slug>`, then `-2`, `-3`,
/// against every id the library lists, refused entries' included, as the built-in ids are made.
fn new_entry_id(library: &Library, definition: &Definition) -> String {
    let taken = |local: &str| library.entries.iter().any(|l| l.entry().is_ok_and(|e| e.id.split_once('/').is_some_and(|(_, id)| id == local)));
    unique_id(&format!("{}-", definition.kind()), &super::slug(definition.name()), "entry", taken)
}

/// `prefix` + `slug` (or `fallback`) cut to an [`identifier`]'s 64 bytes, then `-2`, `-3`, … until
/// `taken` says no.
fn unique_id(prefix: &str, slug: &str, fallback: &str, taken: impl Fn(&str) -> bool) -> String {
    let base = if slug.is_empty() { fallback } else { slug };
    let fit = |suffix: &str| {
        let room = 64usize.saturating_sub(prefix.len() + suffix.len());
        let mut body: String = base.chars().take(room).collect();
        while body.ends_with('-') {
            body.pop();
        }
        format!("{prefix}{body}{suffix}")
    };
    std::iter::once(String::new())
        .chain((2..).map(|n: u32| format!("-{n}")))
        .map(|suffix| fit(&suffix))
        .find(|id| identifier(id) && !taken(id))
        .unwrap_or_else(|| fit("-x"))
}

/// A file or folder name made from `name`: a label that works on every system SolidCraft runs
/// on, never an identity. Windows' name rules are [`crate::vfs`]'s.
fn stem(name: &str) -> String {
    const MAX_STEM: usize = 100;
    let mut stem: String =
        name.chars().map(|c| if c.is_control() || r#"/\:*?"<>|"#.contains(c) || crate::vfs::invisible(c) { '_' } else { c }).collect();
    // Windows drops trailing dots and spaces; a leading dot hides the file elsewhere (and the bin's
    // name starts with one).
    stem = stem.trim_matches(|c: char| c == '.' || c.is_whitespace()).to_string();
    while stem.len() > MAX_STEM {
        stem.pop();
    }
    if stem.is_empty() {
        stem = "entry".into();
    }
    // `NAME~1` reads as another file's short name on Windows.
    if crate::vfs::short_name(&stem) {
        stem = stem.replace('~', "-");
    }
    if crate::vfs::device_name(&stem) {
        stem.insert(0, '_');
    }
    stem
}

/// `stem`, then `stem (2)`, `stem (3)`, … until `taken` says no; `None` past the entry cap.
fn unique(stem: &str, taken: impl Fn(&str) -> bool) -> Option<String> {
    std::iter::once(stem.to_string()).chain((2..=LIMITS.entries_per_kind + 2).map(|n| format!("{stem} ({n})"))).find(|candidate| !taken(candidate))
}

/// The file name a new entry called `name` gets. `taken` says whether a lowercase file name is
/// already used, as names differ only by case on some drives. `None` when every candidate is taken.
pub fn file_name(name: &str, taken: impl Fn(&str) -> bool) -> Option<String> {
    unique(&stem(name), |candidate| taken(&format!("{candidate}.json").to_lowercase())).map(|stem| format!("{stem}.json"))
}
