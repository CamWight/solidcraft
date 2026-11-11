//! Drawings save a whole copy of the drafting profile their numbers follow (`profile`, #4896): its
//! id, version, name and edition, number format, the decimals a new length, angle or tolerance gets,
//! and how a repeat is written. An older drawing has none and reads the default built-in, ISO with a
//! decimal point; its dimensions keep their own decimals. The last release refuses a drawing saved
//! with one, as a newer format.
