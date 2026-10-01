//! QR rendering for the pairing binaries (#64): one PNG for a viewer, and an
//! ASCII fallback for a terminal with no display.

use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum QrError {
    #[error("cannot encode the pairing code as a QR: {0}")]
    Encode(String),
    #[error("cannot write the QR image to {path}: {reason}")]
    Write { path: String, reason: String },
}

/// Pixels per module in the PNG: big enough for a phone camera on a screen.
const PNG_SCALE: usize = 8;
/// Modules of white border, the spec's quiet zone.
const QUIET_MODULES: usize = 4;

fn encode(code: &str) -> Result<qrcode::QrCode, QrError> {
    qrcode::QrCode::new(code.as_bytes()).map_err(|e| QrError::Encode(e.to_string()))
}

/// The QR as terminal text, two characters per module so it scans square.
pub fn ascii_qr(code: &str) -> Result<String, QrError> {
    let qr = encode(code)?;
    let width: usize = qr.width();
    let border: usize = QUIET_MODULES;
    let side: usize = width + border * 2;
    let colors = qr.to_colors();
    let dark = |x: usize, y: usize| {
        let inside = x >= border && y >= border && x < border + width && y < border + width;
        inside && colors[(y - border) * width + (x - border)] == qrcode::Color::Dark
    };
    let mut text = String::with_capacity(side * (side * 2 + 1));
    for y in 0..side {
        for x in 0..side {
            text.push_str(if dark(x, y) { "\u{2588}\u{2588}" } else { "  " });
        }
        text.push('\n');
    }
    Ok(text)
}

/// The QR as a PNG at `path`, scaled up so a phone camera reads it off a screen.
pub fn write_qr_png(code: &str, path: &Path) -> Result<(), QrError> {
    let qr = encode(code)?;
    let width: usize = qr.width();
    let colors = qr.to_colors();
    let side = ((width + QUIET_MODULES * 2) * PNG_SCALE) as u32;
    let mut img = image::GrayImage::from_pixel(side, side, image::Luma([255u8]));
    for (index, color) in colors.iter().enumerate() {
        if *color != qrcode::Color::Dark {
            continue;
        }
        let (x, y) = (index % width + QUIET_MODULES, index / width + QUIET_MODULES);
        paint_module(&mut img, x, y);
    }
    img.save(path).map_err(|e| QrError::Write {
        path: path.display().to_string(),
        reason: e.to_string(),
    })
}

fn paint_module(img: &mut image::GrayImage, x: usize, y: usize) {
    for dy in 0..PNG_SCALE {
        for dx in 0..PNG_SCALE {
            img.put_pixel(
                (x * PNG_SCALE + dx) as u32,
                (y * PNG_SCALE + dy) as u32,
                image::Luma([0u8]),
            );
        }
    }
}

/// Hand `path` to the desktop's viewer (`xdg-open`). Best effort: a headless
/// run still has the ASCII QR, so a missing viewer is only printed.
pub fn open_in_viewer(path: &Path) {
    match std::process::Command::new("xdg-open").arg(path).spawn() {
        Ok(_) => println!("[qr] opened {} with xdg-open", path.display()),
        Err(e) => eprintln!(
            "[qr] xdg-open failed ({e}); open {} manually",
            path.display()
        ),
    }
}
