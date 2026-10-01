//! Media helpers for the sending binaries (#64): the chunked SendMedia stream,
//! generated test images, and a size-capped URL fetch.

use std::io::{Cursor, Read};

use wamux::proto::v1 as pb;

/// Inline media goes over the socket in chunks of this size after the header.
pub const MEDIA_CHUNK_BYTES: usize = 64 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum MediaKitError {
    #[error("cannot encode a {width}x{height} test image: {reason}")]
    Encode {
        width: u32,
        height: u32,
        reason: String,
    },
    #[error("cannot fetch {url}: {reason}")]
    Fetch { url: String, reason: String },
    #[error("{url} is larger than the {cap} byte cap")]
    TooLarge { url: String, cap: usize },
}

/// The SendMedia request stream: the header first, then `bytes` in
/// `MEDIA_CHUNK_BYTES` pieces.
pub fn media_chunks(header: pb::SendMediaHeader, bytes: &[u8]) -> Vec<pb::SendMediaChunk> {
    let first = pb::SendMediaChunk {
        part: Some(pb::send_media_chunk::Part::Header(header)),
    };
    let rest = bytes
        .chunks(MEDIA_CHUNK_BYTES)
        .map(|piece| pb::SendMediaChunk {
            part: Some(pb::send_media_chunk::Part::Chunk(piece.to_vec())),
        });
    std::iter::once(first).chain(rest).collect()
}

fn encode_image(
    img: &image::RgbImage,
    format: image::ImageFormat,
) -> Result<Vec<u8>, MediaKitError> {
    let mut buf: Vec<u8> = Vec::new();
    img.write_to(&mut Cursor::new(&mut buf), format)
        .map_err(|e| MediaKitError::Encode {
            width: img.width(),
            height: img.height(),
            reason: e.to_string(),
        })?;
    Ok(buf)
}

/// A gradient PNG of the given size, so a sent image is recognisable in a chat.
pub fn png_bytes(width: u32, height: u32) -> Result<Vec<u8>, MediaKitError> {
    let mut img = image::RgbImage::new(width, height);
    for (x, y, pixel) in img.enumerate_pixels_mut() {
        *pixel = image::Rgb([((x / 2) % 256) as u8, (y % 256) as u8, 200]);
    }
    encode_image(&img, image::ImageFormat::Png)
}

/// A square JPEG, the shape WhatsApp wants for a profile picture.
pub fn jpeg_bytes(side: u32) -> Result<Vec<u8>, MediaKitError> {
    let mut img = image::RgbImage::new(side, side);
    for (x, y, pixel) in img.enumerate_pixels_mut() {
        *pixel = image::Rgb([(x % 256) as u8, (y % 256) as u8, 120]);
    }
    encode_image(&img, image::ImageFormat::Jpeg)
}

/// GET `url` off the async runtime (ureq is blocking) and refuse a body over `cap`.
pub async fn fetch_url_capped(url: &str, cap: usize) -> Result<Vec<u8>, MediaKitError> {
    let owned: String = url.to_string();
    let joined = tokio::task::spawn_blocking(move || fetch_blocking(&owned, cap)).await;
    joined.map_err(|e| MediaKitError::Fetch {
        url: url.to_string(),
        reason: format!("fetch task failed: {e}"),
    })?
}

fn fetch_blocking(url: &str, cap: usize) -> Result<Vec<u8>, MediaKitError> {
    let failed = |reason: String| MediaKitError::Fetch {
        url: url.to_string(),
        reason,
    };
    let response = ureq::get(url).call().map_err(|e| failed(e.to_string()))?;
    let mut body: Vec<u8> = Vec::new();
    // One byte past the cap is enough to know the body is too large.
    response
        .into_reader()
        .take(cap as u64 + 1)
        .read_to_end(&mut body)
        .map_err(|e| failed(e.to_string()))?;
    if body.len() > cap {
        return Err(MediaKitError::TooLarge {
            url: url.to_string(),
            cap,
        });
    }
    Ok(body)
}

/// A 512x512 WebP sticker (the size WhatsApp renders stickers at).
pub fn webp_sticker_bytes() -> Result<Vec<u8>, MediaKitError> {
    const SIDE: u32 = 512;
    let mut img = image::RgbaImage::new(SIDE, SIDE);
    for (x, y, pixel) in img.enumerate_pixels_mut() {
        *pixel = image::Rgba([(x / 2) as u8, (y / 2) as u8, 180, 255]);
    }
    let mut buf: Vec<u8> = Vec::new();
    image::DynamicImage::ImageRgba8(img)
        .write_to(&mut Cursor::new(&mut buf), image::ImageFormat::WebP)
        .map_err(|e| MediaKitError::Encode {
            width: SIDE,
            height: SIDE,
            reason: e.to_string(),
        })?;
    Ok(buf)
}
