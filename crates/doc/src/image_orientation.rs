//! What a PNG or JPEG file's header says (#19106): its kind, its pixels as the file stores them, and the
//! EXIF orientation every viewer turns them by. One reader, so a picture's size and its decoded pixels
//! always agree. It is doc's (#21702): a canvas's height and corners and a sheet picture's size come
//! from it, so it is part of what the model cache is keyed by. Decoded pixels are turned in the render
//! crate's `image_orientation` (`turn`, `upright`), by the same `image` crate's EXIF rule.
//!
//! - No pixels are decoded. The walk reads a JPEG's segments up to its scan; a PNG's size is its IHDR's
//!   and its chunks up to its pixel data are read by the PNG decoder's own `read_info`, so its CRC checks
//!   are the decoder's. Input is hostile, so every read is checked.
//! - The orientation is the EXIF block's: a JPEG's APP1 segment that starts `Exif\0\0` (of two, the last
//!   before its scan, as the decoder keeps it), or a PNG's `eXIf` chunk, which comes before IDAT (PNG 3rd
//!   edition, table 7; with a bad CRC it is skipped, and of two the first counts, as the decoder does).
//!   The `image` crate reads its IFD0. None, a damaged one or a value outside 1 to 8 is 1: the picture
//!   shows as stored, as a browser shows it (CSS Images 3, `image-orientation: from-image`).

use image::metadata::Orientation;

/// A PNG or JPEG file's header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header {
    /// `png` or `jpeg`.
    pub format: &'static str,
    /// Its width and height in pixels as the file stores them.
    pub stored: [u32; 2],
    /// Its EXIF orientation, 1 to 8.
    pub orientation: u8,
}

/// The header of the PNG or JPEG file `b`; `None` for anything else or a picture with no pixels.
pub fn header(b: &[u8]) -> Option<Header> {
    if b.len() > 24 && b.starts_with(b"\x89PNG\r\n\x1a\n") {
        let w = u32::from_be_bytes([*b.get(16)?, *b.get(17)?, *b.get(18)?, *b.get(19)?]);
        let h = u32::from_be_bytes([*b.get(20)?, *b.get(21)?, *b.get(22)?, *b.get(23)?]);
        return (w > 0 && h > 0).then(|| Header { format: "png", stored: [w, h], orientation: png_orientation(b) });
    }
    if b.starts_with(&[0xFF, 0xD8]) {
        return jpeg(b);
    }
    None
}

/// `stored` as it shows after `orientation`: its sides swap for the quarter turns, 5 to 8.
pub fn shown(stored: [u32; 2], orientation: u8) -> [u32; 2] {
    use Orientation::{Rotate90, Rotate90FlipH, Rotate270, Rotate270FlipH};
    let [w, h] = stored;
    if matches!(Orientation::from_exif(orientation), Some(Rotate90 | Rotate270 | Rotate90FlipH | Rotate270FlipH)) { [h, w] } else { [w, h] }
}

/// The orientation an EXIF block says, 1 when there is none or it says none.
fn orientation(exif: Option<&[u8]>) -> u8 {
    exif.and_then(Orientation::from_exif_chunk).map_or(1, Orientation::to_exif)
}

/// A PNG's orientation as its decoder reads it. The PNG decoder's own `read_info` reads the chunks up to
/// the first IDAT by its rules: an `eXIf` with a bad CRC is skipped, and of two the first counts. A file
/// it refuses shows as stored, and its decoder refuses it too. No pixels are decoded.
fn png_orientation(b: &[u8]) -> u8 {
    let Ok(reader) = png::Decoder::new(std::io::Cursor::new(b)).read_info() else { return 1 };
    orientation(reader.info().exif_metadata.as_deref())
}

/// A JPEG's header: the size of its first frame, and the last EXIF block before its scan.
fn jpeg(b: &[u8]) -> Option<Header> {
    let (mut stored, mut exif, mut scanned) = (None, None, false);
    // Walk the markers: the frame's size as `canvas::image_info` always read it, then on to the scan.
    let mut i = 2;
    while i + 9 < b.len() {
        if *b.get(i)? != 0xFF {
            i += 1;
            continue;
        }
        let m = *b.get(i + 1)?;
        if m == 0xD8 || m == 0x01 || (0xD0..=0xD7).contains(&m) || m == 0xFF {
            i += if m == 0xFF { 1 } else { 2 };
            continue;
        }
        let len = usize::from(u16::from_be_bytes([*b.get(i + 2)?, *b.get(i + 3)?]));
        if stored.is_none() && matches!(m, 0xC0..=0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF) {
            let h = u32::from(u16::from_be_bytes([*b.get(i + 5)?, *b.get(i + 6)?]));
            let w = u32::from(u16::from_be_bytes([*b.get(i + 7)?, *b.get(i + 8)?]));
            if w == 0 || h == 0 {
                return None;
            }
            stored = Some([w, h]);
        } else if m == 0xE1
            && !scanned
            && let Some(data) = b.get(i + 4..i + 2 + len).and_then(|data| data.strip_prefix(b"Exif\0\0"))
        {
            // Of two, the last counts, as the decoder keeps it; another APP1 (XMP) is no EXIF block.
            exif = Some(data);
        }
        if matches!(m, 0xDA | 0xD9) {
            // Whatever follows the scan is no header.
            scanned = true;
            if stored.is_some() {
                break;
            }
        }
        i += 2 + len;
    }
    Some(Header { format: "jpeg", stored: stored?, orientation: orientation(exif) })
}

#[cfg(test)]
#[path = "image_orientation_tests.rs"]
mod tests;
