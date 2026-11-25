//! A drawing can save a title block label, a table heading, a view label's words or the text over a
//! title block cell (`title_block_overrides`) as a built-in message
//! (`{"message": "drawing.paper.cell.title"}`, #19884) in place of its English text, so the words
//! print in the drawing's language and follow a change of it. New title blocks, tables and view
//! labels save their words so, and so do the note and document type a built-in template gives its
//! first sheet's cells. Text someone typed is still saved as the text itself. An older drawing has
//! typed text only: it reads and prints exactly as it did, in English, and no step rewrites it, as
//! its English can't be told from English typed on purpose. The last release warns about the newer
//! format number, opens what it can read and keeps the rest.
