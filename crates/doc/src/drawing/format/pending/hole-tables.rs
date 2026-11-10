//! A table can list holes with their places (`kind: {hole: {view, origin, holes: [{hole, tag}]}}`, #13056):
//! the view they are measured on, the point they are measured from, and each hole by its feature (and
//! sketch point) with the tag it was given. An older drawing has no hole tables. The last release
//! refuses a drawing with this format's number.
