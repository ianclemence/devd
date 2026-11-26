//! The window's background art: which scene a theme wears, and the small tools that make a picture.
//!
//! Every scene is drawn smoothly across the whole window (see [`super::soft`]): thin anti-aliased lines, layered
//! light and slow gradients, in the theme's own colours. The picture is made here rather than shipped, a PNG written
//! by hand that the window serves to itself, so there is no file to keep in step with a theme.

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
    /// Soft pools of coloured light.
    Glow,
    /// A mesh of colour across the window.
    Mesh,
}

impl Pattern {
    /// Every scene, by the name a theme file and a picture's path use.
    pub const ALL: [(&'static str, Pattern); 7] = [
        ("ripples", Pattern::Ripples),
        ("pasture", Pattern::Pasture),
        ("silk", Pattern::Silk),
        ("contours", Pattern::Contours),
        ("rays", Pattern::Rays),
        ("glow", Pattern::Glow),
        ("mesh", Pattern::Mesh),
    ];

    /// The scene's name.
    pub fn name(self) -> &'static str {
        Self::ALL.iter().find(|(_, p)| *p == self).map(|(n, _)| *n).unwrap_or("ripples")
    }

    /// The scene a name means.
    pub fn named(name: &str) -> Option<Pattern> {
        Self::ALL.iter().find(|(n, _)| *n == name).map(|(_, p)| *p)
    }

    /// Whether the scene stands on the bottom edge of the window rather than hanging from the top.
    pub fn stands_on_the_bottom(self) -> bool {
        matches!(self, Pattern::Pasture)
    }

    /// How strongly the picture shows, as an opacity. The scenes differ in how much ink they carry.
    pub fn strength(self, dark: bool) -> f32 {
        let base = match self {
            Pattern::Ripples => 0.34,
            Pattern::Pasture => 0.42,
            Pattern::Silk => 0.42,
            Pattern::Contours => 0.34,
            Pattern::Rays => 0.38,
            Pattern::Glow => 0.45,
            Pattern::Mesh => 0.34,
        };
        // Ink on a pale window carries more than light on a dark one, so the pale themes are shown a little quieter.
        if dark { base } else { base * 0.92 }
    }
}

// ---------------------------------------------------------------------------------------------- noise

thread_local! {
    /// The picture's own number, which changes every pattern in it without changing its kind. Set for the length of
    /// one picture, on the thread making it.
    static STIR: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// A repeatable number in 0..1 for a point and a seed.
pub(super) fn hash2(x: i32, y: i32, seed: u32) -> f32 {
    let seed = seed ^ STIR.with(|s| s.get()).wrapping_mul(0x9e37_79b9);
    let mut h = (x as u32).wrapping_mul(0x85eb_ca6b)
        ^ (y as u32).wrapping_mul(0xc2b2_ae35)
        ^ seed.wrapping_mul(0x27d4_eb2f);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2c1b_3c6d);
    h ^= h >> 12;
    h = h.wrapping_mul(0x297a_2d39);
    h ^= h >> 15;
    (h & 0xffff) as f32 / 65_535.0
}

pub(super) fn smooth(t: f32) -> f32 {
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
pub(super) fn fbm(x: f32, y: f32, seed: u32) -> f32 {
    let (mut sum, mut amp, mut freq, mut norm) = (0.0, 0.5, 1.0, 0.0);
    for octave in 0..5 {
        sum += amp * noise(x * freq, y * freq, seed.wrapping_add(octave * 31));
        norm += amp;
        amp *= 0.5;
        freq *= 2.03;
    }
    sum / norm
}


/// The picture as a PNG in `base` and `glint`.
pub fn png_as(base: [u8; 3], glint: [u8; 3], pattern: Pattern) -> Vec<u8> {
    super::soft::png(base, glint, pattern, (false, false), 0)
}

/// The picture again, mirrored across and down when `flip` says so, with the pattern's own number `seed`.
pub fn png_with(base: [u8; 3], glint: [u8; 3], pattern: Pattern, flip: (bool, bool), seed: u32) -> Vec<u8> {
    super::soft::png(base, glint, pattern, flip, seed)
}

pub(super) fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
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
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xedb8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

/// A zlib stream that stores the data uncompressed: valid, simple, and the picture is cut once per run.
pub(super) fn zlib_stored(data: &[u8]) -> Vec<u8> {
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
        assert_eq!(crc32(b"IEND"), 0xae42_6082);
    }
}
