//! A drawing's images (W38e, #5426): a PNG or JPEG placed on a sheet, such as a logo or a photo.
//!
//! - **The bytes** travel in the drawing's record (base64, as a canvas's do), so the design opens
//!   without the original file, and the file's path is never saved.
//! - **The caps** are checked from the file's bytes and header, before anything decodes its pixels:
//!   a small file can claim to hold more pixels than memory has. The header can lie in the record
//!   too, so reading a drawing compares the two ([`Drawing::check_image_files`]); an edit, which only
//!   ever adds a checked file, doesn't decode them again.
//!
//! The rules, beside the record's: its sheet is one of the drawing's; its rectangle is on paper and
//! has a size; at most [`MAX_IMAGES`] images of at most [`MAX_IMAGE_BYTES`] and
//! [`MAX_IMAGE_PIXELS`] each, and [`MAX_IMAGES_BYTES`] between them, listed by id.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use solidcraft_geom::drawing_view::is_paper_coordinate;
use solidcraft_geom::paper::MAX_SIDE_MM;

use super::{Drawing, add_by_id, in_id_order, require};
use crate::canvas::{base64_decode, base64_encode, image_info};
use crate::{DocError, Result, ids};

/// The most images a drawing may have.
pub const MAX_IMAGES: usize = 1_000;
/// The largest file an image may be.
pub const MAX_IMAGE_BYTES: usize = 4 << 20;
/// The most a drawing's images may be between them: two of the largest and some, whose 12 MiB of
/// text leaves room under [`super::MAX_DRAWING_BYTES`] for the rest of the record.
pub const MAX_IMAGES_BYTES: usize = 9 << 20;
/// The most pixels an image may have: 8,000 by 8,000, 256 MiB once decoded.
pub const MAX_IMAGE_PIXELS: u64 = 64_000_000;

/// One image on a sheet of a drawing.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SheetImage {
    #[serde(with = "crate::ids")]
    pub id: u64,
    #[serde(with = "crate::ids")]
    pub sheet: u64,
    /// Where it sits, `[x0, y0, x1, y1]` in millimetres from the sheet's lower-left corner.
    pub rect_mm: [f64; 4],
    /// `png` or `jpeg`, as the file's header says.
    pub format: String,
    /// Its width and height in pixels, as the file's header says.
    pub pixels: [u32; 2],
    /// The file, base64.
    pub data: String,
}

/// The length of `bytes` bytes in base64.
const fn encoded(bytes: usize) -> usize {
    bytes.div_ceil(3) * 4
}

/// What a file's bytes and header say it is, when its caps allow it. Nothing is decoded.
fn file_info(bytes: &[u8]) -> Result<(&'static str, [u32; 2])> {
    require(bytes.len() <= MAX_IMAGE_BYTES, || format!("the file is over {} MiB", MAX_IMAGE_BYTES >> 20))?;
    let (format, pixels) = image_info(bytes).ok_or_else(|| DocError::Invalid("the file isn't a PNG or JPEG".into()))?;
    let [w, h] = pixels;
    require(u64::from(w) * u64::from(h) <= MAX_IMAGE_PIXELS, || format!("the file has {w} by {h} pixels, over {MAX_IMAGE_PIXELS}"))?;
    Ok((format, pixels))
}

impl SheetImage {
    /// The image of a file's `bytes`, when its caps allow it: nothing is decoded.
    pub fn from_file(id: u64, sheet: u64, rect_mm: [f64; 4], bytes: &[u8]) -> Result<SheetImage> {
        let (format, pixels) = file_info(bytes)?;
        Ok(SheetImage { id, sheet, rect_mm, format: format.into(), pixels, data: base64_encode(bytes) })
    }
}

impl Drawing {
    /// Adds `images`, each where its id puts it, as [`Drawing::add_notes`] adds notes.
    pub fn add_images(&mut self, images: impl IntoIterator<Item = Arc<SheetImage>>) {
        add_by_id(&mut self.images, images, |i| i.id);
    }

    /// Image `id`, found by halves in the images' id order.
    pub fn image(&self, id: u64) -> Option<&SheetImage> {
        let at = self.images.binary_search_by_key(&id, |i| i.id).ok()?;
        self.images.get(at).map(|i| &**i)
    }

    pub(super) fn check_images(&self) -> Result<()> {
        require(self.images.len() <= MAX_IMAGES, || format!("a drawing has at most {MAX_IMAGES} images"))?;
        require(in_id_order(&self.images, |i| i.id), || "a drawing lists its images by id".into())?;
        let total = self.images.iter().fold(0, |total: usize, i| total.saturating_add(i.data.len()));
        require(total <= encoded(MAX_IMAGES_BYTES), || format!("a drawing's images are at most {} MiB between them", MAX_IMAGES_BYTES >> 20))?;
        for image in &self.images {
            self.check_image(image).map_err(|e| DocError::Invalid(format!("image {}: {e}", ids::encode(image.id))))?;
        }
        Ok(())
    }

    fn check_image(&self, i: &SheetImage) -> Result<()> {
        require(self.sheets.iter().any(|s| s.id == i.sheet), || format!("it is on sheet {}, which the drawing doesn't have", ids::encode(i.sheet)))?;
        let [x0, y0, x1, y1] = i.rect_mm;
        let on_paper = [x0, y0, x1, y1].into_iter().all(is_paper_coordinate);
        require(on_paper && x0 < x1 && y0 < y1, || format!("its rectangle has a size and is at most {MAX_SIDE_MM} mm on paper"))?;
        require(matches!(i.format.as_str(), "png" | "jpeg"), || "it is a PNG or a JPEG".into())?;
        let [w, h] = i.pixels;
        require(w > 0 && h > 0 && u64::from(w) * u64::from(h) <= MAX_IMAGE_PIXELS, || format!("it has 1 to {MAX_IMAGE_PIXELS} pixels"))?;
        require(i.data.len() <= encoded(MAX_IMAGE_BYTES), || format!("its file is at most {} MiB", MAX_IMAGE_BYTES >> 20))
    }

    /// Each image's bytes are the file its record says: what reading a drawing checks once, as it
    /// decodes the base64, so that nothing later trusts a size the file doesn't have.
    pub(super) fn check_image_files(&self) -> Result<()> {
        self.images.iter().try_for_each(|i| {
            let bytes = base64_decode(&i.data).ok_or_else(|| DocError::Invalid(format!("image {}: its file isn't base64", ids::encode(i.id))))?;
            let (format, pixels) = file_info(&bytes).map_err(|e| DocError::Invalid(format!("image {}: {e}", ids::encode(i.id))))?;
            require(format == i.format && pixels == i.pixels, || format!("image {}: its record and its file disagree", ids::encode(i.id)))
        })
    }
}

#[cfg(test)]
#[path = "image_tests.rs"]
mod tests;
