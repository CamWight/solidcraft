//! Drawings save where their own symbols are placed (`placed_symbols`, #13063): each on a sheet,
//! a view or a view's model as a note is, with its place, scale, angle and its prompts' answers.
//! An older drawing has none. The last release warns about the newer format number, opens what it
//! can read and keeps the rest: it sets the placements aside and writes them back when it saves,
//! and refuses to delete a sheet, a view or a symbol a placement names.
