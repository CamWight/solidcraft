//! Drawings save their images (`images`, #5426): each on a sheet, with its rectangle, its format and
//! pixel size, and the file's bytes (base64). The file's path is never saved. An older drawing has no
//! images. The last release refuses a drawing saved with them, as a newer format.
