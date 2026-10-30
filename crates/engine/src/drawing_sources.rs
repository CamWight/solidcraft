//! The designs a drawing's linked sources name, opened read-only beside it (#3890, L2). Each is a
//! [`Session`] of its own, which nothing runs a command on and nothing saves: at most one for each
//! file, however many sources name it, held only while a drawing open names it. A linked design's own
//! drawings aren't opened, so a drawing of B in A, and one of A in B, open each design once.
//!
//! Every read goes through the checks a component link's does, so a path is judged from its text
//! before anything asks the disk or the network. [`Session::open_sources`] opens what the drawing open
//! names, and a command on the drawing does so first. A view drawn from a design not open yet, or from
//! one that can't be opened, says so ([`Session::drawn`]).

use std::collections::{BTreeMap, BTreeSet};
use std::io::{Cursor, ErrorKind};
use std::path::Path;
use std::sync::Arc;

use solidcraft_doc::drawing::Drawing;
use solidcraft_doc::{DocError, ModelState, ids};
use solidcraft_io::vfs;

use crate::Session;

/// Where a linked design stands. The words are a link listing's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceState {
    /// It is open.
    UpToDate,
    /// It isn't open yet.
    Pending,
    /// There is no file at its path.
    Missing,
    /// The file is there but isn't a design this build can read, or its path is a device.
    Unreadable,
    /// The file is outside the folders this agent may use, or on a network location not allowed.
    NotAllowed,
    /// It was saved by a newer SolidCraft: updating SolidCraft opens it.
    Newer,
}

impl SourceState {
    /// The word a listing reports.
    pub fn word(self) -> &'static str {
        match self {
            SourceState::UpToDate => "up_to_date",
            SourceState::Pending => "pending",
            SourceState::Missing => "missing",
            SourceState::Unreadable => "unreadable",
            SourceState::NotAllowed => "not_allowed",
            SourceState::Newer => "newer",
        }
    }
}

/// Why a source can't be drawn: where it stands, and the reason to show.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unopened {
    pub state: SourceState,
    pub reason: String,
}

impl std::fmt::Display for Unopened {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.reason)
    }
}

/// One linked design, as the last attempt to open it left it.
struct Linked {
    opened: Result<Box<Session>, Unopened>,
    /// The world state a view of it is drawn from while it isn't open, so a new attempt is a new one.
    nothing: Arc<ModelState>,
}

/// The linked designs of a session, and where this session has found a source's file.
#[derive(Default)]
pub struct LinkedDesigns {
    /// By file.
    designs: BTreeMap<String, Linked>,
    /// The file found for a source's saved path, this session only: `drawing.source.locate`.
    located: BTreeMap<String, String>,
}

impl LinkedDesigns {
    /// The file a source saved with `path` is read from now.
    pub(crate) fn file<'a>(&'a self, path: &'a str) -> &'a str {
        self.located.get(path).map_or(path, String::as_str)
    }

    /// A [`Session::scratch`] copy: each design shared, as a scratch copy of a session shares its own.
    pub(crate) fn scratch(&self) -> LinkedDesigns {
        let copy = |l: &Linked| Linked { opened: l.opened.as_ref().map(|s| Box::new(s.scratch())).map_err(Clone::clone), nothing: l.nothing.clone() };
        LinkedDesigns { designs: self.designs.iter().map(|(file, l)| (file.clone(), copy(l))).collect(), located: self.located.clone() }
    }

    /// Drops every design, keeping what was located: nothing open names them.
    pub(crate) fn close(&mut self) {
        self.designs.clear();
    }

    /// Forgets each design that couldn't be opened, so the next attempt reads it again.
    pub(crate) fn forget_unopened(&mut self) {
        self.designs.retain(|_, l| l.opened.is_ok());
    }

    /// The file `path` is found at from now on, this session only.
    pub(crate) fn locate(&mut self, path: String, file: String) {
        self.located.insert(path, file);
    }
}

/// The world state of a source no design is open for yet.
fn not_open() -> Arc<ModelState> {
    static NOT_OPEN: std::sync::OnceLock<Arc<ModelState>> = std::sync::OnceLock::new();
    NOT_OPEN.get_or_init(Arc::default).clone()
}

/// Whether `file` is the design saved at `own`.
fn is_own(own: Option<&str>, file: &str) -> bool {
    own.is_some_and(|own| crate::cmd::absolute_path(Path::new(own)) == crate::cmd::absolute_path(Path::new(file)))
}

fn problem(state: SourceState, reason: String) -> Unopened {
    Unopened { state, reason }
}

/// The design at `file`, read-only, or why it can't be opened. `own` is the file of the design asking,
/// whose own server is allowed.
fn open(file: &str, own: Option<&str>) -> Result<Session, Unopened> {
    if let Some(remote) = vfs::design_path_refusal(file, own) {
        let reason = vfs::check_design_path(file, own).err().map_or_else(|| file.to_string(), |e| e.to_string());
        return Err(match remote {
            vfs::Remote::Network(_) => problem(SourceState::NotAllowed, reason),
            vfs::Remote::Device => problem(SourceState::Unreadable, reason),
        });
    }
    if !solidcraft_io::is_design_container_path(Path::new(file)) {
        return Err(problem(SourceState::Unreadable, format!("{file}: not a saved design")));
    }
    let bytes = vfs::read_design_path(file, own, solidcraft_io::limits::max_design_bytes()).map_err(|e| match e.kind() {
        // The folder scope's refusal says which folders the agent may use.
        ErrorKind::PermissionDenied => problem(SourceState::NotAllowed, e.to_string()),
        ErrorKind::NotFound => problem(SourceState::Missing, format!("{file}: not found")),
        _ => problem(SourceState::Unreadable, format!("{file}: {e}")),
    })?;
    let unreadable = |e: solidcraft_io::IoError| match e {
        solidcraft_io::IoError::Doc(DocError::Newer(why)) => problem(SourceState::Newer, format!("{file}: {why}")),
        e => problem(SourceState::Unreadable, format!("{file}: {e}")),
    };
    let (doc, saved) = if solidcraft_io::is_design_container(&bytes) {
        let mut reader = solidcraft_io::DesignContainerReader::new(Cursor::new(&bytes)).map_err(unreadable)?;
        let saved = reader.geometry();
        (reader.read_document().map_err(unreadable)?, saved)
    } else {
        (solidcraft_io::read_design(&bytes).map_err(unreadable)?, None)
    };
    let mut session = Session::opened_at(doc, saved.as_ref(), Some(file));
    session.read_only = true;
    Ok(session)
}

impl Session {
    /// Opens each linked design the drawing open names and isn't open yet, and drops those no longer
    /// named. With no linked source this is one loop over the drawing's sources.
    pub fn open_sources(&mut self) {
        let Some(drawing) = self.open_drawing.and_then(|id| self.doc.drawing(id)) else {
            self.linked.close();
            return;
        };
        let named = drawing.sources.iter().filter_map(|source| source.path.as_deref());
        let own = self.path.as_deref();
        let files: BTreeSet<String> = named.map(|path| self.linked.file(path)).filter(|file| !is_own(own, file)).map(str::to_string).collect();
        self.linked.designs.retain(|file, _| files.contains(file));
        for file in files {
            if !self.linked.designs.contains_key(&file) {
                let opened = open(&file, self.path.as_deref()).map(Box::new);
                self.linked.designs.insert(file, Linked { opened, nothing: Arc::default() });
            }
        }
    }

    /// The session `source` of `d` draws from: this one for the design the drawing is in, or the design
    /// its path names; or why that can't be drawn. Nothing is read here.
    pub fn drawn(&self, d: &Drawing, source: u64) -> Result<&Session, Unopened> {
        let Some(found) = d.sources.iter().find(|s| s.id == source) else {
            return Err(problem(SourceState::Unreadable, format!("the drawing has no source {}", ids::encode(source))));
        };
        let Some(path) = found.path.as_deref() else { return Ok(self) };
        let file = self.linked.file(path);
        if is_own(self.path.as_deref(), file) {
            return Ok(self);
        }
        match self.linked.designs.get(file) {
            Some(Linked { opened: Ok(session), .. }) => Ok(session),
            Some(Linked { opened: Err(why), .. }) => Err(why.clone()),
            None => Err(problem(SourceState::Pending, format!("{file}: not open yet"))),
        }
    }

    /// The world state a view of `source` is drawn from now: what makes its lines current.
    pub(crate) fn drawn_state(&self, d: &Drawing, source: u64) -> Arc<ModelState> {
        match self.drawn(d, source) {
            Ok(from) => from.world_state(),
            Err(_) => {
                let path = d.sources.iter().find(|s| s.id == source).and_then(|s| s.path.as_deref());
                let linked = path.and_then(|path| self.linked.designs.get(self.linked.file(path)));
                linked.map_or_else(not_open, |l| l.nothing.clone())
            }
        }
    }
}

#[cfg(test)]
#[path = "drawing_sources_tests.rs"]
mod tests;
