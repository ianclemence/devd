//! The window's background art: a field of dots that thins out from a corner.
//!
//! An ordered (Bayer) dither turns a smooth fade into two states, a dot or no dot,
//! so the art is plain, sharp at any size, and has no banding. It is the look of
//! something engraved or printed, which suits a tool that is meant to be exact.
//!
//! The picture is made here rather than shipped: a small PNG written by hand
//! (no image library, no file to keep in step with the theme) that the window
//! serves to itself as `art/dither-dark.png` and `art/dither-light.png`. The
//! pixels are one colour and the dots are the only thing drawn; how faint it is
//! is the opacity it is shown at.

/// The picture's size in pixels.
pub const WIDTH: u32 = 640;
/// The picture's size in pixels.
pub const HEIGHT: u32 = 360;
/// How many pixels one dot covers. Two keeps the dots crisp on a scaled display.
const CELL: u32 = 2;

const BAYER: [[u8; 8]; 8] = [
    [0, 32, 8, 40, 2, 34, 10, 42],
    [48, 16, 56, 24, 50, 18, 58, 26],
    [12, 44, 4, 36, 14, 46, 6, 38],
    [60, 28, 52, 20, 62, 30, 54, 22],
    [3, 35, 11, 43, 1, 33, 9, 41],
    [51, 19, 59, 27, 49, 17, 57, 25],
    [15, 47, 7, 39, 13, 45, 5, 37],
    [63, 31, 55, 23, 61, 29, 53, 21],
];

/// The shape of the field. Each theme gets its own, so the background says
/// which theme is in use even before a colour is read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pattern {
    /// Dense at the top-right corner, thinning toward the bottom-left. Stone.
    Corner,
    /// A diagonal weave, like cloth. Shepherd.
    Woven,
    /// Vertical bands, like the strings of an instrument. Harp.
    Strings,
}

/// How dense the dots are at a point, from 0 (none) to 1.
fn density(x: u32, y: u32, pattern: Pattern) -> f32 {
    match pattern {
        Pattern::Corner => {
            let (dx, dy) = ((WIDTH - x) as f32 / WIDTH as f32, y as f32 / HEIGHT as f32);
            let d = (dx * dx + dy * dy).sqrt() / std::f32::consts::SQRT_2;
            (1.0 - d * 1.35).clamp(0.0, 1.0).powf(1.6)
        }
        Pattern::Woven => {
            let (dx, dy) = ((WIDTH - x) as f32 / WIDTH as f32, (HEIGHT - y) as f32 / HEIGHT as f32);
            let d = (dx * dx + dy * dy).sqrt() / std::f32::consts::SQRT_2;
            let base = (1.0 - d * 1.25).clamp(0.0, 1.0);
            let weave = 0.58 + 0.42 * (0.5 + 0.5 * ((x + y) as f32 / 24.0).sin());
            (base * weave).powf(1.35)
        }
        Pattern::Strings => {
            let base = (1.0 - (y as f32 / HEIGHT as f32) * 1.15).clamp(0.0, 1.0);
            let strings = 0.38 + 0.62 * (0.5 + 0.5 * (x as f32 / 10.0).sin());
            (base * strings).powf(1.45)
        }
    }
}

/// Whether a dot is drawn at this pixel.
fn dot(x: u32, y: u32, pattern: Pattern) -> bool {
    let (cx, cy) = (x / CELL, y / CELL);
    density(cx * CELL, cy * CELL, pattern) > (BAYER[(cy % 8) as usize][(cx % 8) as usize] as f32 + 0.5) / 64.0
}

/// The picture as a PNG: dots in `rgb`, everything else transparent.
#[cfg(test)]
pub fn png(rgb: [u8; 3]) -> Vec<u8> {
    png_as(rgb, Pattern::Corner)
}

/// The picture for one theme's pattern.
pub fn png_as(rgb: [u8; 3], pattern: Pattern) -> Vec<u8> {
    let mut raw = Vec::with_capacity((HEIGHT * (WIDTH * 4 + 1)) as usize);
    for y in 0..HEIGHT {
        raw.push(0); // no filter on this row
        for x in 0..WIDTH {
            if dot(x, y, pattern) {
                raw.extend_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
            } else {
                raw.extend_from_slice(&[0, 0, 0, 0]);
            }
        }
    }
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut header = Vec::new();
    header.extend_from_slice(&WIDTH.to_be_bytes());
    header.extend_from_slice(&HEIGHT.to_be_bytes());
    header.extend_from_slice(&[8, 6, 0, 0, 0]); // 8-bit RGBA, no interlace
    chunk(&mut out, b"IHDR", &header);
    chunk(&mut out, b"IDAT", &zlib_stored(&raw));
    chunk(&mut out, b"IEND", &[]);
    out
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let start = out.len();
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let crc = crc32(&out[start..]);
    out.extend_from_slice(&crc.to_be_bytes());
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for &b in bytes {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xedb8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

/// A zlib stream that stores the data uncompressed: valid, simple, and the picture is small.
fn zlib_stored(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x78, 0x01];
    let mut blocks = data.chunks(65_535).peekable();
    while let Some(block) = blocks.next() {
        out.push(u8::from(blocks.peek().is_none()));
        out.extend_from_slice(&(block.len() as u16).to_le_bytes());
        out.extend_from_slice(&(!(block.len() as u16)).to_le_bytes());
        out.extend_from_slice(block);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for &x in data {
        a = (a + x as u32) % 65_521;
        b = (b + a) % 65_521;
    }
    out.extend_from_slice(&((b << 16) | a).to_be_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_checksum_is_the_one_png_defines() {
        // The published CRC of an empty IEND chunk.
        assert_eq!(crc32(b"IEND"), 0xae42_6082);
    }

    #[test]
    fn the_picture_is_a_png_of_the_stated_size() {
        let bytes = png([200, 200, 190]);
        assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
        assert_eq!(&bytes[12..16], b"IHDR");
        assert_eq!(u32::from_be_bytes(bytes[16..20].try_into().unwrap()), WIDTH);
        assert_eq!(u32::from_be_bytes(bytes[20..24].try_into().unwrap()), HEIGHT);
        assert!(bytes.ends_with(&[0, 0, 0, 0, b'I', b'E', b'N', b'D', 0xae, 0x42, 0x60, 0x82]));
    }

    #[test]
    fn the_dots_gather_at_the_corner_and_thin_out() {
        let count = |x0: u32, y0: u32, pattern| (0..60).flat_map(|y| (0..60).map(move |x| (x0 + x, y0 + y))).filter(|&(x, y)| dot(x, y, pattern)).count();
        let corner = count(WIDTH - 60, 0, Pattern::Corner);
        let far = count(0, HEIGHT - 60, Pattern::Corner);
        assert!(corner > 1_000, "{corner}");
        assert_eq!(far, 0, "nothing reaches the far corner");
    }

    #[test]
    fn every_pattern_draws_its_own_field() {
        for pattern in [Pattern::Corner, Pattern::Woven, Pattern::Strings] {
            let bytes = png_as([200, 200, 190], pattern);
            assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
            assert!(bytes.len() > 100, "{pattern:?}");
        }
        let dots = |pattern| (0..120).flat_map(|y| (0..120).map(move |x| (x, y))).filter(|&(x, y)| dot(x, y, pattern)).count();
        let (corner, woven, strings) = (dots(Pattern::Corner), dots(Pattern::Woven), dots(Pattern::Strings));
        assert!(corner > 0 && woven > 0 && strings > 0, "{corner} {woven} {strings}");
        assert_ne!(woven, strings, "the two new fields are not the same picture");
    }

    #[test]
    fn the_same_picture_every_time() {
        assert_eq!(png([1, 2, 3]), png([1, 2, 3]));
    }
}
