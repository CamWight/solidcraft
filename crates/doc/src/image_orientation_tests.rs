use image::{DynamicImage, ImageReader};

use super::*;

/// The colours of a photo's stored quarters: top-left, top-right, bottom-left, bottom-right.
const QUARTERS: [[u8; 3]; 4] = [[255, 0, 0], [0, 255, 0], [0, 0, 255], [255, 255, 0]];

/// A big-endian TIFF block whose IFD0 holds `entries`, each a tag, a type and one value.
fn tiff(entries: &[[u16; 3]]) -> Vec<u8> {
    let mut t = b"MM\0\x2a\0\0\0\x08".to_vec();
    t.extend((entries.len() as u16).to_be_bytes());
    for [tag, kind, value] in entries {
        t.extend([tag.to_be_bytes(), kind.to_be_bytes(), [0, 0], [0, 1], value.to_be_bytes(), [0, 0]].concat());
    }
    t.extend([0; 4]);
    t
}

/// A JPEG of 32 by 16 pixels in [`QUARTERS`], with `exif` as its EXIF block.
fn photo(exif: Option<Vec<u8>>) -> Vec<u8> {
    use image::ImageEncoder;
    let pixels = image::RgbImage::from_fn(32, 16, |x, y| image::Rgb(QUARTERS[usize::from(y >= 8) * 2 + usize::from(x >= 16)]));
    let mut out = Vec::new();
    let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 95);
    if let Some(exif) = exif {
        encoder.set_exif_metadata(exif).unwrap();
    }
    encoder.write_image(pixels.as_raw(), 32, 16, image::ExtendedColorType::Rgb8).unwrap();
    out
}

/// `jpeg` with one more segment, `marker` holding `data`, first after its start.
fn with_segment(jpeg: &[u8], marker: u8, data: &[u8]) -> Vec<u8> {
    let length = u16::try_from(data.len() + 2).unwrap().to_be_bytes();
    [&jpeg[..2], &[0xFF, marker], &length, data, &jpeg[2..]].concat()
}

/// The EXIF APP1 segment's data for orientation `value`.
fn app1(value: u16) -> Vec<u8> {
    [b"Exif\0\0".as_slice(), &tiff(&[[0x0112, 3, value]])].concat()
}

/// A PNG of 32 by 16 pixels, with `exif` as an `eXIf` chunk, before or after its pixels.
fn png(exif: Option<&[u8]>, after: bool) -> Vec<u8> {
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, 32, 16);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().unwrap();
    let chunk = |writer: &mut png::Writer<&mut Vec<u8>>| {
        if let Some(exif) = exif {
            writer.write_chunk(png::chunk::ChunkType(*b"eXIf"), exif).unwrap();
        }
    };
    if !after {
        chunk(&mut writer);
    }
    writer.write_image_data(&[90; 32 * 16 * 3]).unwrap();
    if after {
        chunk(&mut writer);
    }
    writer.finish().unwrap();
    out
}

/// Where a [`png`]'s first `eXIf` chunk sits, whole: its length, kind, data and CRC.
fn exif_chunk(file: &[u8]) -> std::ops::Range<usize> {
    let at = file.windows(4).position(|w| w == b"eXIf").unwrap() - 4;
    let length = u32::from_be_bytes(file[at..at + 4].try_into().unwrap()) as usize;
    at..at + 12 + length
}

fn read(b: &[u8]) -> Option<(&'static str, [u32; 2], u8)> {
    header(b).map(|h| (h.format, h.stored, h.orientation))
}

/// `bytes` decoded and turned upright as a canvas or a sheet picture is drawn: by the render crate.
fn decoded(bytes: &[u8]) -> (DynamicImage, bool) {
    solidcraft_render::image_orientation::decode_upright(ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format().unwrap()).unwrap()
}

/// The header's orientation and the decoder's are one: for JPEGs and PNGs with no EXIF block, one,
/// two that disagree, a damaged one, a PNG eXIf with a broken CRC, and values 0 to 9 and 255, the size the header gives is
/// the size `decode_upright` turns the pixels to, and they turn exactly when the header names a transform.
#[test]
fn the_header_and_the_decoder_agree_on_a_files_orientation() {
    let mut files = vec![photo(None), png(None, false)];
    for value in (0..=9u16).chain([255]) {
        let exif = tiff(&[[0x0112, 3, value]]);
        files.push(photo(Some(exif.clone())));
        files.push(png(Some(&exif), false));
        for other in [1, 6] {
            files.push(with_segment(&photo(Some(tiff(&[[0x0112, 3, other]]))), 0xE1, &app1(value)));
        }
    }
    let damaged = [tiff(&[[0x0112, 3, 6]])[..12].to_vec(), b"MM\0\x2a\xff\xff\xff\xf0".to_vec(), tiff(&[[0x0112, 4, 6]])];
    for exif in damaged {
        files.push(photo(Some(exif.clone())));
        files.push(png(Some(&exif), false));
        files.push(with_segment(&photo(Some(tiff(&[[0x0112, 3, 6]]))), 0xE1, &[b"Exif\0\0".as_slice(), &exif].concat()));
    }
    // A PNG's eXIf as its decoder reads it: one whose CRC is broken is skipped, and of two the first counts.
    let six = png(Some(&tiff(&[[0x0112, 3, 6]])), false);
    let mut broken = six.clone();
    broken[exif_chunk(&six).end - 1] ^= 0xFF;
    files.push(broken);
    let eight = png(Some(&tiff(&[[0x0112, 3, 8]])), false);
    let end = exif_chunk(&six).end;
    files.push([&six[..end], &eight[exif_chunk(&eight)], &six[end..]].concat());
    for (case, file) in files.iter().enumerate() {
        let h = header(file).unwrap();
        let (picture, turned) = decoded(file);
        assert_eq!([picture.width(), picture.height()], shown(h.stored, h.orientation), "case {case}: {h:?}");
        let transforms = Orientation::from_exif(h.orientation).is_some_and(|o| o != Orientation::NoTransforms);
        assert_eq!(turned, transforms, "case {case}: {h:?}");
    }
}

/// Each orientation 1 to 8 reads from a JPEG's APP1 in either byte order and from a PNG's eXIf, and
/// the sides swap for 5 to 8 only. A header with no EXIF is 1, a file of neither kind is nothing.
#[test]
fn a_files_header_gives_its_exif_orientation() {
    for value in 1..=8u16 {
        let o = value as u8;
        assert_eq!(read(&photo(Some(tiff(&[[0x0112, 3, value]])))), Some(("jpeg", [32, 16], o)), "{value}");
        let mut little = b"II\x2a\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0".to_vec();
        little.extend([value.to_le_bytes(), [0, 0], [0, 0], [0, 0]].concat());
        assert_eq!(read(&photo(Some(little))), Some(("jpeg", [32, 16], o)), "{value} little-endian");
        assert_eq!(read(&png(Some(&tiff(&[[0x0112, 3, value]])), false)), Some(("png", [32, 16], o)), "{value}");
        assert_eq!(shown([32, 16], o), if value >= 5 { [16, 32] } else { [32, 16] }, "{value}");
    }
    assert_eq!(read(&photo(None)), Some(("jpeg", [32, 16], 1)));
    assert_eq!(read(&png(None, false)), Some(("png", [32, 16], 1)));
    // Headers alone, as the size checks read them.
    let mut png_header = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
    png_header.extend([640u32.to_be_bytes(), 480u32.to_be_bytes()].concat());
    png_header.extend([8, 6, 0, 0, 0]);
    assert_eq!(read(&png_header), Some(("png", [640, 480], 1)));
    let jpeg_header = [0xFF, 0xD8, 0xFF, 0xC0, 0, 17, 8, 1, 144, 2, 88, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    assert_eq!(read(&jpeg_header), Some(("jpeg", [600, 400], 1)));
    assert_eq!(read(&with_segment(&jpeg_header, 0xE1, &app1(6))), Some(("jpeg", [600, 400], 6)));
    for neither in [&b""[..], b"GIF89a", b"\x89PNG", &[0xFF, 0xD8, 0xFF, 0xD9]] {
        assert_eq!(read(neither), None);
    }
    assert_eq!((shown([3, 2], 0), shown([3, 2], 9), shown([3, 2], 255)), ([3, 2], [3, 2], [3, 2]));
}

/// A damaged, late or second EXIF block never turns a picture the size doesn't, and no input panics:
/// a bad value, type or count, a truncated block, an IFD past its end, an eXIf after the pixels, an
/// APP1 after the scan; of two, the last counts. Mutated files give 1 to 8 or nothing.
#[test]
fn damaged_duplicate_or_late_exif_reads_as_stored() {
    let orientation = |b: &[u8]| header(b).map(|h| h.orientation);
    for exif in [
        tiff(&[[0x0112, 3, 0]]),
        tiff(&[[0x0112, 3, 9]]),
        tiff(&[[0x0112, 3, 0xFFFF]]),
        tiff(&[[0x0112, 4, 6]]),
        tiff(&[[0x0110, 3, 6]]),
        tiff(&[[0x0112, 3, 6]])[..12].to_vec(),
        b"MM\0\x2a\xff\xff\xff\xf0".to_vec(),
        b"XX\0\x2a\0\0\0\x08".to_vec(),
        Vec::new(),
    ] {
        assert_eq!(orientation(&photo(Some(exif.clone()))), Some(1), "{exif:?}");
        assert_eq!(orientation(&png(Some(&exif), false)), Some(1), "{exif:?}");
    }
    let mut count = tiff(&[[0x0112, 3, 6]]);
    count[17] = 2;
    assert_eq!(orientation(&photo(Some(count))), Some(1), "two values");
    assert_eq!(orientation(&png(Some(&tiff(&[[0x0112, 3, 6]])), true)), Some(1), "after IDAT");
    let plain = photo(None);
    let late = [&plain[..plain.len() - 2], &[0xFF, 0xE1, 0, 2 + 32], &app1(6)[..], &[0xFF, 0xD9]].concat();
    assert_eq!(orientation(&late), Some(1), "after the scan");
    assert_eq!(orientation(&with_segment(&photo(Some(tiff(&[[0x0112, 3, 1]]))), 0xE1, &app1(6))), Some(1), "the last of two");
    assert_eq!(orientation(&with_segment(&photo(Some(tiff(&[[0x0112, 3, 6]]))), 0xE1, b"http://ns.adobe.com/xap/1.0/\0")), Some(6));
    let mut fill = vec![0xFF; 16 << 20];
    fill[1] = 0xD8;
    assert_eq!(header(&fill), None);

    let mut seed = 0x0001_9106_u64;
    for (case, original) in [photo(Some(tiff(&[[0x0112, 3, 6]]))), png(Some(&tiff(&[[0x0112, 3, 6]])), false)].iter().cycle().take(4000).enumerate() {
        let mut bytes = original.clone();
        for _ in 0..1 + case % 4 {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
            let at = (seed >> 33) as usize % bytes.len().min(160);
            bytes[at] = (seed >> 13) as u8;
        }
        if let Some(h) = header(&bytes) {
            assert!((1..=8).contains(&h.orientation), "{h:?}");
        }
    }
}
