//! The window's background art: one scene per theme, drawn as an ordered dither.
//!
//! An ordered (Bayer) dither turns a smooth field into two states, a dot or no
//! dot, so the art is sharp at any size, has no banding, and looks printed or
//! engraved, which suits a tool that is meant to be exact. The fields behind it
//! are not flat gradients: they are made from layered noise, so every scene has
//! the irregular, organic grain of a risograph print, and a second, brighter
//! tone (the theme's signal colour) is scattered through the places the scene
//! wants the eye to go.
//!
//! * **Stone**: ripples. A stone dropped into still water at the top-right
//!   corner; the rings spread, wobble and dissolve.
//! * **Shepherd**: dawn over the pasture. A low sun, and ridge after ridge of
//!   hills, each darker and nearer than the last, with a few stars still up.
//! * **Psalm**: silk. Strands of light drift and fold across the dark, a thread of
//!   gold running through them, the way a harp's strings catch a lamp.
//!
//! The picture is made here rather than shipped: a PNG written by hand (no
//! image library, no file to keep in step with the theme) that the window
//! serves to itself as `art/<theme>-<dark|light>.png`. Every scene fades to
//! nothing on the sides that face into the window, so there is never an edge.

use std::f32::consts::{PI, TAU};

/// The picture's size in pixels.
pub const WIDTH: u32 = 1280;
/// The picture's size in pixels.
pub const HEIGHT: u32 = 720;
/// How many pixels one dot covers. Two keeps the dots crisp on a scaled display.
const CELL: u32 = 2;
const CELLS_W: u32 = WIDTH / CELL;
const CELLS_H: u32 = HEIGHT / CELL;
/// The picture is wider than tall by this much; distances are measured with it so rings stay round.
const ASPECT: f32 = WIDTH as f32 / HEIGHT as f32;

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

/// The scene a theme draws behind its screens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pattern {
    /// Rings spreading from the top-right corner. Sling.
    Ripples,
    /// A low sun over layered hills. Dawn.
    Pasture,
    /// Folds of silk: warped strands of light. Psalm.
    Silk,
    /// The contour lines of a hand-drawn map, in rose. Sharon.
    Contours,
    /// Rays of morning light fanning from the corner. Daybreak.
    Rays,
}

impl Pattern {
    /// Whether the scene stands on the bottom edge of the window rather than hanging from the top.
    pub fn stands_on_the_bottom(self) -> bool {
        matches!(self, Pattern::Pasture)
    }

    /// How strongly the picture shows, as an opacity. The scenes differ in how much ink they carry.
    pub fn strength(self, dark: bool) -> f32 {
        let base = match self {
            Pattern::Ripples => 0.13,
            Pattern::Pasture => 0.16,
            Pattern::Silk => 0.18,
            Pattern::Contours => 0.14,
            Pattern::Rays => 0.14,
        };
        // Ink on a pale window carries more than light on a dark one, so the pale themes are shown a little quieter.
        if dark { base } else { base * 0.92 }
    }
}

// ---------------------------------------------------------------------------------------------- noise

/// A repeatable number in 0..1 for a point and a seed.
fn hash2(x: i32, y: i32, seed: u32) -> f32 {
    let mut h = (x as u32).wrapping_mul(0x85eb_ca6b) ^ (y as u32).wrapping_mul(0xc2b2_ae35) ^ seed.wrapping_mul(0x27d4_eb2f);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2c1b_3c6d);
    h ^= h >> 12;
    h = h.wrapping_mul(0x297a_2d39);
    h ^= h >> 15;
    (h & 0xffff) as f32 / 65_535.0
}

fn smooth(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Smooth value noise in 0..1.
fn noise(x: f32, y: f32, seed: u32) -> f32 {
    let (xi, yi) = (x.floor(), y.floor());
    let (fx, fy) = (smooth(x - xi), smooth(y - yi));
    let (xi, yi) = (xi as i32, yi as i32);
    let a = hash2(xi, yi, seed);
    let b = hash2(xi + 1, yi, seed);
    let c = hash2(xi, yi + 1, seed);
    let d = hash2(xi + 1, yi + 1, seed);
    let top = a + (b - a) * fx;
    let bottom = c + (d - c) * fx;
    top + (bottom - top) * fy
}

/// Layered noise: broad shapes with finer detail on them, in 0..1.
fn fbm(x: f32, y: f32, seed: u32) -> f32 {
    let (mut sum, mut amp, mut freq, mut norm) = (0.0, 0.5, 1.0, 0.0);
    for octave in 0..5 {
        sum += amp * noise(x * freq, y * freq, seed.wrapping_add(octave * 31));
        norm += amp;
        amp *= 0.5;
        freq *= 2.03;
    }
    sum / norm
}

/// 0 below `a`, 1 above `b`, smooth between. `a` may be above `b` to flip it.
fn step(a: f32, b: f32, x: f32) -> f32 {
    smooth((x - a) / (b - a))
}

// ---------------------------------------------------------------------------------------------- scenes

/// How much ink at a point (0..1), and how much of it is the bright tone (0..1).
/// `u` and `v` run 0..1 across and down the picture.
fn scene(pattern: Pattern, u: f32, v: f32) -> (f32, f32) {
    let (px, py) = (u * ASPECT, v);
    match pattern {
        Pattern::Ripples => {
            let (dx, dy) = (px - 0.94 * ASPECT, py - 0.03);
            let r = (dx * dx + dy * dy).sqrt();
            // The rings wobble more the further they have travelled.
            let wobble = (fbm(px * 2.4, py * 2.4, 11) - 0.5) * 0.30 * (0.25 + r);
            let phase = (r + wobble) * 15.5;
            let ring = (0.5 + 0.5 * (phase * TAU).cos()).powf(2.4);
            let reach = (1.0 - r / 1.40).clamp(0.0, 1.0).powf(1.45);
            let core = (1.0 - r / 0.26).clamp(0.0, 1.0).powf(1.8);
            let ink = reach * (0.07 + 0.93 * ring) + 0.75 * core;
            let bright = if ring > 0.72 { reach * 1.1 } else { 0.0 } + core * 0.4;
            (ink * step(0.0, 0.26, u), bright.clamp(0.0, 1.0))
        }
        Pattern::Pasture => {
            // The sky: a low sun with a wide glow, and stars that have not gone yet.
            let (sx, sy) = (px - 0.70 * ASPECT, py - 0.34);
            let sun_dist = (sx * sx + sy * sy).sqrt();
            let disc = step(0.105, 0.092, sun_dist) * (0.70 + 0.30 * (1.0 - sun_dist / 0.105).clamp(0.0, 1.0) + 0.10 * fbm(px * 40.0, py * 40.0, 8));
            let glow = (-(sun_dist / 0.30).powi(2)).exp() * 0.55;
            let haze = fbm(px * 1.1, py * 5.5, 5) * 0.14 * step(0.20, 0.55, v);
            let mut ink = (glow + disc + haze).min(1.0);
            let mut bright = disc + glow * 0.6;
            // Hills, far to near. Each one hides what is behind it.
            const HILLS: [(f32, f32, f32); 5] = [(0.56, 0.060, 1.7), (0.64, 0.070, 2.4), (0.72, 0.070, 3.3), (0.81, 0.060, 4.6), (0.91, 0.045, 6.4)];
            for (k, (base, amp, freq)) in HILLS.iter().enumerate() {
                let ridge = base + amp * (fbm(px * freq + k as f32 * 7.3, 3.0 + k as f32, 40 + k as u32) - 0.5) * 2.0;
                if v >= ridge {
                    let depth = v - ridge;
                    let body = (1.0 - depth / 0.20).clamp(0.0, 1.0).powf(1.5);
                    let rim = if depth < 0.0065 { 1.0 } else { 0.0 };
                    // Ploughed rows on the nearest hills.
                    let rows = if k >= 3 { 0.82 + 0.18 * (0.5 + 0.5 * (py * 120.0 + fbm(px * 3.0, py * 2.0, 70) * 14.0).sin()) } else { 1.0 };
                    ink = ((0.07 + 0.045 * k as f32 + 0.40 * body) * rows + rim * 0.55).min(1.0);
                    bright = if rim > 0.0 && k % 2 == 0 { 0.7 } else { 0.0 };
                }
            }
            // Stars, only in open sky.
            if v < 0.50 && hash2((u * CELLS_W as f32) as i32, (v * CELLS_H as f32) as i32, 9) < 0.0035 {
                ink = 1.0;
                bright = 1.0;
            }
            (ink * step(0.0, 0.30, u) * step(0.0, 0.22, v), bright.clamp(0.0, 1.0))
        }
        Pattern::Silk => {
            // Long lines that flow across the window and fold over themselves. Each line is one value of a phase
            // that runs mostly down the window, bent by two layers of slow noise: where the bend is steep the
            // lines crowd together and the cloth looks gathered; where it is gentle they spread and rest.
            let bend = fbm(px * 1.15, py * 0.9, 31) * 7.0 + fbm(px * 2.6 + 4.0, py * 2.1, 32) * 2.2;
            let phase = py * 21.0 + px * 3.4 + bend;
            let wave = phase * PI;
            // Where the cloth is gathered: brightest in the upper right, falling away toward the left and bottom.
            let (dx, dy) = (px - 0.88 * ASPECT, py - 0.10);
            let reach = (1.0 - (dx * dx + dy * dy).sqrt() / 1.45).clamp(0.0, 1.0).powf(1.05);
            // The sheen of satin: broad soft bands that follow the folds, with a thin bright line on each crest.
            let sheen = (0.5 + 0.5 * wave.sin()).powf(2.2);
            let crest = (1.0 - wave.sin().abs()).powf(7.0);
            let ink = reach * (0.05 + 0.50 * sheen) + reach * 0.85 * crest * step(0.15, 0.45, reach);
            // Gold runs along the crests where the cloth catches the light.
            let bright = if crest > 0.30 { (crest * 1.2).min(1.0) * step(0.2, 0.6, reach) } else { 0.0 };
            (ink * step(0.10, 0.34, u) * step(1.0, 0.62, v), bright)
        }
        Pattern::Contours => {
            // The height of the land, and a line at every step of it; every fifth line is an index line.
            let height = fbm(px * 1.7 + 0.4, py * 1.7, 61) * 0.78 + 0.22 * fbm(px * 4.2, py * 4.2, 62);
            let band = height * 24.0;
            let frac = band - band.floor();
            let near = frac.min(1.0 - frac) * 2.0;
            let line = (1.0 - near * 4.5).clamp(0.0, 1.0);
            let index = (band.floor() as i32).rem_euclid(5) == 0;
            let (dx, dy) = (px - 0.82 * ASPECT, py - 0.30);
            let reach = (1.0 - (dx * dx + dy * dy).sqrt() / 1.25).clamp(0.0, 1.0).powf(0.85);
            let ink = reach * (line * if index { 1.0 } else { 0.62 } + 0.04);
            let bright = if index && line > 0.25 { reach } else { 0.0 };
            (ink * step(0.0, 0.30, u) * step(1.0, 0.62, v), bright)
        }
        Pattern::Rays => {
            // Light from just beyond the top-right corner: broad beams with fine ones inside them.
            let (dx, dy) = (px - 1.03 * ASPECT, py + 0.07);
            let r = (dx * dx + dy * dy).sqrt();
            let angle = dy.atan2(dx);
            let bend = (fbm(r * 2.6, angle * 2.0, 71) - 0.5) * 0.30;
            let fine = (0.5 + 0.5 * ((angle + bend) * 34.0).sin()).powf(2.8);
            let broad = 0.5 + 0.5 * ((angle + bend * 0.8) * 7.5 + 1.3).sin();
            let reach = (1.0 - r / 1.60).clamp(0.0, 1.0).powf(0.95);
            let glow = (-(r / 0.34).powi(2)).exp();
            let ink = reach * (0.07 + 0.78 * fine * (0.35 + 0.65 * broad)) + glow * 0.75;
            let bright = if fine > 0.80 { reach } else { 0.0 } + glow * 0.5;
            (ink.min(1.0) * step(0.0, 0.30, u) * step(1.0, 0.62, v), bright.clamp(0.0, 1.0))
        }
    }
}

/// What one cell is: nothing, a dot in the base tone, or a dot in the bright tone.
fn cell(cx: u32, cy: u32, pattern: Pattern) -> Option<bool> {
    let (u, v) = ((cx as f32 + 0.5) / CELLS_W as f32, (cy as f32 + 0.5) / CELLS_H as f32);
    let (ink, bright) = scene(pattern, u, v);
    let threshold = (BAYER[(cy % 8) as usize][(cx % 8) as usize] as f32 + 0.5) / 64.0;
    if ink <= threshold {
        return None;
    }
    Some(bright > 0.0 && hash2(cx as i32, cy as i32, 99) < bright)
}

/// Whether any ink lands in the pixel at (`x`, `y`), for tests.
#[cfg(test)]
fn inked(x: u32, y: u32, pattern: Pattern) -> bool {
    cell(x / CELL, y / CELL, pattern).is_some()
}

/// The picture as a PNG: dots in `base`, brighter dots in `glint`, everything else transparent.
pub fn png_as(base: [u8; 3], glint: [u8; 3], pattern: Pattern) -> Vec<u8> {
    let mut rows = Vec::with_capacity((HEIGHT * (WIDTH * 4 + 1)) as usize);
    for cy in 0..CELLS_H {
        let mut line = Vec::with_capacity((WIDTH * 4) as usize);
        for cx in 0..CELLS_W {
            let px: [u8; 4] = match cell(cx, cy, pattern) {
                None => [0, 0, 0, 0],
                Some(false) => [base[0], base[1], base[2], 255],
                Some(true) => [glint[0], glint[1], glint[2], 255],
            };
            for _ in 0..CELL {
                line.extend_from_slice(&px);
            }
        }
        for _ in 0..CELL {
            rows.push(0); // no filter on this row
            rows.extend_from_slice(&line);
        }
    }
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut header = Vec::new();
    header.extend_from_slice(&WIDTH.to_be_bytes());
    header.extend_from_slice(&HEIGHT.to_be_bytes());
    header.extend_from_slice(&[8, 6, 0, 0, 0]); // 8-bit RGBA, no interlace
    chunk(&mut out, b"IHDR", &header);
    chunk(&mut out, b"IDAT", &zlib_stored(&rows));
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

/// A zlib stream that stores the data uncompressed: valid, simple, and the picture is cut once per run.
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

    const ALL: [Pattern; 5] = [Pattern::Ripples, Pattern::Pasture, Pattern::Silk, Pattern::Contours, Pattern::Rays];

    fn count(pattern: Pattern, x0: u32, y0: u32, w: u32, h: u32) -> usize {
        (0..h).flat_map(|y| (0..w).map(move |x| (x0 + x, y0 + y))).filter(|&(x, y)| inked(x, y, pattern)).count()
    }

    #[test]
    fn the_checksum_is_the_one_png_defines() {
        // The published CRC of an empty IEND chunk.
        assert_eq!(crc32(b"IEND"), 0xae42_6082);
    }

    #[test]
    fn every_scene_is_a_png_of_the_stated_size() {
        for pattern in ALL {
            let bytes = png_as([200, 200, 190], [220, 240, 80], pattern);
            assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
            assert_eq!(&bytes[12..16], b"IHDR");
            assert_eq!(u32::from_be_bytes(bytes[16..20].try_into().unwrap()), WIDTH);
            assert_eq!(u32::from_be_bytes(bytes[20..24].try_into().unwrap()), HEIGHT);
            assert!(bytes.ends_with(&[0, 0, 0, 0, b'I', b'E', b'N', b'D', 0xae, 0x42, 0x60, 0x82]));
        }
    }

    #[test]
    fn the_ripples_gather_at_the_corner_and_die_out_far_from_it() {
        assert!(count(Pattern::Ripples, WIDTH - 200, 0, 200, 200) > 4_000);
        assert_eq!(count(Pattern::Ripples, 0, HEIGHT - 120, 400, 120), 0, "nothing reaches the far corner");
    }

    #[test]
    fn no_scene_has_an_edge_where_it_meets_the_window() {
        // The sides that face into the window carry no ink at all, so the picture never shows as a rectangle.
        for pattern in ALL {
            assert_eq!(count(pattern, 0, 0, 24, HEIGHT), 0, "{pattern:?} has a hard left edge");
            if pattern != Pattern::Pasture {
                assert_eq!(count(pattern, 0, HEIGHT - 6, WIDTH / 2, 6), 0, "{pattern:?} has a hard bottom edge");
            }
        }
        for pattern in [Pattern::Silk, Pattern::Contours, Pattern::Rays] {
            assert_eq!(count(pattern, 0, HEIGHT - 6, WIDTH, 6), 0, "{pattern:?} has a hard bottom edge on the right");
        }
        assert_eq!(count(Pattern::Pasture, 0, 0, WIDTH, 6), 0, "the pasture has a hard top edge");
    }

    #[test]
    fn the_pasture_stands_on_the_bottom_and_the_silk_hangs_from_the_top() {
        assert!(Pattern::Pasture.stands_on_the_bottom());
        assert!(!Pattern::Silk.stands_on_the_bottom() && !Pattern::Ripples.stands_on_the_bottom());
        let hills = count(Pattern::Pasture, WIDTH / 2, HEIGHT - 160, 600, 120);
        let sky = count(Pattern::Pasture, WIDTH / 2, 0, 600, 120);
        assert!(hills > sky * 3, "{hills} {sky}");
        assert!(count(Pattern::Silk, WIDTH / 2, 0, 600, 200) > 1_500);
    }

    #[test]
    fn the_silk_is_gathered_at_the_upper_right_and_thin_at_the_lower_left() {
        let near = count(Pattern::Silk, WIDTH - 360, 0, 360, 240);
        let far = count(Pattern::Silk, 0, HEIGHT - 240, 360, 240);
        assert!(near > far * 5 + 500, "{near} {far}");
    }

    #[test]
    fn every_scene_draws_its_own_picture_with_both_tones() {
        let inks: Vec<usize> = ALL.iter().map(|&p| count(p, WIDTH / 3, 0, 400, HEIGHT)).collect();
        assert!(inks.iter().all(|&n| n > 500), "{inks:?}");
        for (i, a) in inks.iter().enumerate() {
            for b in &inks[i + 1..] {
                assert_ne!(a, b, "two scenes are the same picture: {inks:?}");
            }
        }
        for pattern in ALL {
            let glints = (0..CELLS_H).step_by(3).flat_map(|y| (0..CELLS_W).step_by(3).map(move |x| (x, y))).filter(|&(x, y)| cell(x, y, pattern) == Some(true)).count();
            assert!(glints > 20, "{pattern:?} has no bright tone ({glints})");
        }
    }

    #[test]
    fn the_rays_fan_out_from_the_corner() {
        assert!(count(Pattern::Rays, WIDTH - 300, 0, 300, 300) > 5_000);
        assert!(count(Pattern::Rays, 0, HEIGHT - 200, 300, 200) < 200, "the rays do not reach the far corner");
    }

    #[test]
    fn the_same_picture_every_time() {
        assert_eq!(png_as([1, 2, 3], [4, 5, 6], Pattern::Silk), png_as([1, 2, 3], [4, 5, 6], Pattern::Silk));
    }
}

#[cfg(test)]
mod preview {
    use super::*;

    /// Writes every scene to the folder named by `DEVD_ART_OUT`, so a person can look at them. Not a check.
    #[test]
    #[ignore]
    fn write_the_scenes_for_looking_at() {
        let Ok(dir) = std::env::var("DEVD_ART_OUT") else { return };
        for (name, pattern, base, glint) in [
            ("sling", Pattern::Ripples, [0xd9, 0xd6, 0xcc], [0xd4, 0xf2, 0x4a]),
            ("dawn", Pattern::Pasture, [0xd9, 0xc9, 0x9a], [0xe9, 0xb8, 0x5c]),
            ("psalm", Pattern::Silk, [0xc9, 0xb8, 0xd9], [0xf0, 0xc4, 0x6e]),
            ("sharon", Pattern::Contours, [0x8c, 0x3b, 0x52], [0xb0, 0x2a, 0x58]),
            ("daybreak", Pattern::Rays, [0x3a, 0x31, 0x5a], [0x86, 0x58, 0x00]),
        ] {
            std::fs::write(std::path::Path::new(&dir).join(format!("{name}.png")), png_as(base, glint, pattern)).unwrap();
        }
    }
}
