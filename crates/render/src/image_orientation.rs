//! A picture's decoded pixels turned upright by its EXIF orientation (#19106).
//!
//! What a PNG or JPEG file's header says (its kind, its stored size, its orientation and the size it
//! shows at) is read by the doc crate's `image_orientation`, which the model cache is keyed by (#21702):
//! a canvas's height and a sheet picture's size are part of the model. It reads the tag by the same
//! `image` crate's EXIF rule as the decoders here, and doc's tests check the two against each other, so
//! a picture's size and its decoded pixels always agree.
//!
//! - Decoded pixels turn in one place, [`turn`]; a file's own tag is read through its decoder in one
//!   place, [`upright`]. Phones and cameras often save a portrait photo as landscape pixels plus an
//!   orientation tag, and the `image` crate decodes the pixels as stored (#19138): everything that
//!   reads a picture file to show it or to measure it goes through [`upright`] or [`decode_upright`].

use std::io::{BufRead, Seek};

use image::metadata::Orientation;
use image::{DynamicImage, ImageDecoder, ImageReader, ImageResult};

/// Decoded pixels turned and mirrored as `orientation` says; a value outside 1 to 8 leaves them.
pub fn turn(image: &mut DynamicImage, orientation: u8) {
    if let Some(orientation) = Orientation::from_exif(orientation) {
        image.apply_orientation(orientation);
    }
}

/// The picture `decoder` holds, decoded and turned upright by the EXIF orientation it reads, and
/// whether it was turned: when it was, the file's own bytes still carry the tag, so a copy of them
/// would show the picture on its side to a reader that ignores it.
pub fn upright(mut decoder: impl ImageDecoder) -> ImageResult<(DynamicImage, bool)> {
    let orientation = decoder.orientation()?.to_exif();
    let mut picture = DynamicImage::from_decoder(decoder)?;
    turn(&mut picture, orientation);
    Ok((picture, orientation != 1))
}

/// The picture in `reader`, decoded under the reader's limits and turned upright ([`upright`]).
pub fn decode_upright<R: BufRead + Seek>(reader: ImageReader<R>) -> ImageResult<(DynamicImage, bool)> {
    upright(reader.into_decoder()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sheet_image::tests::{app1, with_segment};

    /// A JPEG of `width` x `height` pixels, red on its left half and blue on its right half.
    fn test_jpeg(width: u32, height: u32, orientation: Option<u8>) -> Vec<u8> {
        let rgb = image::RgbImage::from_fn(width, height, |x, _| if x < width / 2 { image::Rgb([255, 0, 0]) } else { image::Rgb([0, 0, 255]) });
        let mut jpeg = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 95).encode_image(&rgb).expect("a JPEG of a few pixels encodes");
        orientation.map_or(jpeg.clone(), |o| with_segment(&jpeg, 0xE1, &app1(u16::from(o))))
    }

    fn decoded(bytes: &[u8]) -> (DynamicImage, bool) {
        decode_upright(ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format().unwrap()).unwrap()
    }

    /// An orientation of 6 stands a 60 x 40 file up as 40 x 60; a file with none is as stored.
    #[test]
    fn an_exif_orientation_turns_the_picture_upright() {
        let (plain, turned) = decoded(&test_jpeg(60, 40, None));
        assert_eq!((plain.width(), plain.height(), turned), (60, 40, false));
        let (upright, turned) = decoded(&test_jpeg(60, 40, Some(6)));
        assert_eq!((upright.width(), upright.height(), turned), (40, 60, true));
        // Orientation 1 is "as stored": nothing turns.
        let (same, turned) = decoded(&test_jpeg(60, 40, Some(1)));
        assert_eq!((same.width(), same.height(), turned), (60, 40, false));
    }
}
