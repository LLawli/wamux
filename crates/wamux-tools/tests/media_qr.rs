//! #64: the shared media and QR helpers the sending and pairing binaries use.

use std::io::Cursor;

use wamux::proto::v1 as pb;
use wamux_tools::media_kit::{MEDIA_CHUNK_BYTES, jpeg_bytes, media_chunks, png_bytes};
use wamux_tools::qr::{ascii_qr, write_qr_png};

fn header() -> pb::SendMediaHeader {
    pb::SendMediaHeader {
        mime_type: "image/png".into(),
        media_type: pb::MediaType::Image as i32,
        ..Default::default()
    }
}

#[test]
fn media_chunks_put_the_header_first_then_the_bytes_in_order() {
    let bytes: Vec<u8> = (0..(MEDIA_CHUNK_BYTES * 2 + 10)).map(|i| i as u8).collect();
    let chunks = media_chunks(header(), &bytes);
    assert_eq!(
        chunks.len(),
        4,
        "header + 3 chunks for 2 full and 1 partial"
    );
    assert!(matches!(
        chunks[0].part,
        Some(pb::send_media_chunk::Part::Header(_))
    ));
    let mut joined = Vec::new();
    for chunk in &chunks[1..] {
        let Some(pb::send_media_chunk::Part::Chunk(piece)) = &chunk.part else {
            panic!("only data after the header");
        };
        assert!(piece.len() <= MEDIA_CHUNK_BYTES);
        joined.extend_from_slice(piece);
    }
    assert_eq!(joined, bytes);
}

#[test]
fn media_chunks_of_nothing_is_just_the_header() {
    assert_eq!(media_chunks(header(), &[]).len(), 1);
}

#[test]
fn png_bytes_decode_at_the_requested_size() {
    let bytes = png_bytes(320, 200).unwrap();
    let image = image::load(Cursor::new(&bytes), image::ImageFormat::Png).unwrap();
    assert_eq!((image.width(), image.height()), (320, 200));
}

#[test]
fn jpeg_bytes_decode_as_a_square() {
    let bytes = jpeg_bytes(640).unwrap();
    let image = image::load(Cursor::new(&bytes), image::ImageFormat::Jpeg).unwrap();
    assert_eq!((image.width(), image.height()), (640, 640));
}

#[test]
fn ascii_qr_renders_square_rows() {
    let text = ascii_qr("2@pairing-ref,keyA,keyB,adv").unwrap();
    let rows: Vec<&str> = text.lines().filter(|l| !l.is_empty()).collect();
    assert!(
        rows.len() >= 21,
        "a QR is at least 21 modules tall: {}",
        rows.len()
    );
    let width = rows[0].chars().count();
    assert!(rows.iter().all(|r| r.chars().count() == width));
}

#[test]
fn write_qr_png_writes_a_decodable_image() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("qr.png");
    write_qr_png("2@pairing-ref,keyA,keyB,adv", &path).unwrap();
    let image = image::open(&path).unwrap();
    assert!(
        image.width() >= 200,
        "scaled for a phone camera: {}",
        image.width()
    );
    assert_eq!(image.width(), image.height());
}

#[test]
fn write_qr_png_to_a_missing_directory_is_an_error_naming_the_path() {
    let err = write_qr_png("code", std::path::Path::new("/nonexistent-64/qr.png")).unwrap_err();
    assert!(err.to_string().contains("/nonexistent-64/qr.png"), "{err}");
}
