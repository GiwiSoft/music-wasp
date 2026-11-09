//! Bounded decoding for images that come from users or downloaded tags.
//!
//! Skin backgrounds and icons are user files, and cover art is read from tags
//! that any downloaded audio file can carry. A tiny PNG can declare a huge
//! canvas, so every decode here sets explicit limits instead of relying on the
//! crate defaults. The default allocation cap is non-strict and says nothing
//! about dimensions.

use image::{DynamicImage, ImageError, ImageReader, ImageResult, Limits};
use std::io::{BufRead, Cursor, Seek};
use std::path::Path;

/// Largest accepted width or height. Covers are a few thousand pixels, and a
/// skin background at this size is already 256 MiB of RGBA.
pub const MAX_DIMENSION: u32 = 8192;

/// Strict budget for the decoder's allocations, including the output buffer.
pub const MAX_ALLOC: u64 = 512 * 1024 * 1024;

fn limits() -> Limits {
    let mut limits = Limits::no_limits();
    limits.max_image_width = Some(MAX_DIMENSION);
    limits.max_image_height = Some(MAX_DIMENSION);
    limits.max_alloc = Some(MAX_ALLOC);
    limits
}

/// Decode an image file from disk within the limits.
pub fn open(path: &Path) -> ImageResult<DynamicImage> {
    decode(ImageReader::open(path)?)
}

/// Decode an in-memory image within the limits.
pub fn from_bytes(bytes: &[u8]) -> ImageResult<DynamicImage> {
    decode(ImageReader::new(Cursor::new(bytes)))
}

fn decode<R: BufRead + Seek>(reader: ImageReader<R>) -> ImageResult<DynamicImage> {
    let mut reader = reader.with_guessed_format().map_err(ImageError::IoError)?;
    reader.limits(limits());
    reader.decode()
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ExtendedColorType, ImageEncoder};

    fn png(width: u32, height: u32) -> Vec<u8> {
        let pixels = vec![128u8; (width * height * 3) as usize];
        let mut out = Vec::new();
        image::codecs::png::PngEncoder::new(&mut out)
            .write_image(&pixels, width, height, ExtendedColorType::Rgb8)
            .unwrap();
        out
    }

    #[test]
    fn decodes_images_within_the_limits() {
        let image = from_bytes(&png(64, 32)).unwrap();
        assert_eq!((image.width(), image.height()), (64, 32));
    }

    #[test]
    fn rejects_width_over_the_limit() {
        let error = from_bytes(&png(MAX_DIMENSION + 1, 1)).unwrap_err();
        assert!(matches!(error, ImageError::Limits(_)), "{error:?}");
    }

    #[test]
    fn rejects_height_over_the_limit() {
        let error = from_bytes(&png(1, MAX_DIMENSION + 1)).unwrap_err();
        assert!(matches!(error, ImageError::Limits(_)), "{error:?}");
    }

    #[test]
    fn accepts_the_dimension_limit_exactly() {
        assert!(from_bytes(&png(MAX_DIMENSION, 1)).is_ok());
    }

    #[test]
    fn rejects_garbage() {
        assert!(from_bytes(b"not an image").is_err());
    }
}
