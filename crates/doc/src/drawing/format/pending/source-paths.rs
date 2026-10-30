//! A drawing's sources may have a `path` (#3890): the file of another design it draws, relative to the
//! design it is saved in once that has a file. A source with no `path` is the design it is in, and only
//! one may be that. An older drawing's source has no `path` and stays as it is. The last release refuses
//! a drawing saved with one, as a newer format.
