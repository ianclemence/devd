//! The scenes behind the window: smooth, anti-aliased pictures in a theme's own two colours.
//!
//! Each is made here as an RGBA PNG whose colour is the theme's and whose transparency is the shape, so the window's
//! own colour shows through wherever the scene has nothing to say. Lines are drawn by their distance in pixels from
//! the point, which keeps them thin and clean at any size, and a little grain is added to the transparency so a slow
//! fade does not band on a dark window. Every scene is quiet toward the left and the middle, where text is read.

use super::dither::{Pattern, chunk, fbm, hash2, smooth, zlib_stored};

/// The picture's size in pixels. It is scaled to cover the window.
pub const WIDTH: u32 = 1280;
/// The picture's size in pixels.
pub const HEIGHT: u32 = 720;

const ASPECT: f32 = WIDTH as f32 / HEIGHT as f32;

fn mix(a: [u8; 3], b: [u8; 3], t: f32) -> [f32; 3] {
    let t = t.clamp(0.0, 1.0);
    [0, 1, 2].map(|i| a[i] as f32 * (1.0 - t) + b[i] as f32 * t)
}

/// Layers painted one over another: a colour and how much of it shows.
#[derive(Default, Clone, Copy)]
struct Paint {
    colour: [f32; 3],
    alpha: f32,
}

impl Paint {
    /// Lay `colour` over what is already there.
    fn over(&mut self, colour: [f32; 3], alpha: f32) {
        let a = alpha.clamp(0.0, 1.0);
        if a <= 0.0 {
            return;
        }
        let total = a + self.alpha * (1.0 - a);
        for (mine, theirs) in self.colour.iter_mut().zip(colour) {
            *mine = (theirs * a + *mine * self.alpha * (1.0 - a)) / total;
        }
        self.alpha = total;
    }
}

/// How much of a line of half-width `half` (in pixels) covers a point `distance` pixels from its middle.
fn line(distance: f32, half: f32) -> f32 {
    (half + 0.5 - distance).clamp(0.0, 1.0)
}

fn gauss(x: f32) -> f32 {
    (-(x * x)).exp()
}

fn distance(px: f32, py: f32, at: (f32, f32)) -> f32 {
    ((px - at.0).powi(2) + (py - at.1).powi(2)).sqrt()
}

/// A field of fine lines rolling like water seen from a height, their crests catching the light. Sling.
fn ripples(px: f32, py: f32, base: [u8; 3], glint: [u8; 3], seed: u32) -> Paint {
    let mut paint = Paint::default();
    let across = px / ASPECT;
    paint.over(mix(base, glint, 0.95), gauss(distance(px, py, (0.94 * ASPECT, 0.02)) / 0.46) * 0.30);
    // The field rises out of nothing on the left, where text is read.
    let reach = smooth((across - 0.16) / 0.66);
    if reach <= 0.0 || py < 0.30 {
        return paint;
    }
    const LINES: u32 = 30;
    let start = seed as f32 * 1.7;
    for k in 0..LINES {
        let t = k as f32 / (LINES - 1) as f32;
        let (swell, ripple) = (0.15 * (1.0 - 0.35 * t), 0.045);
        let (a, b) = (px * 2.1 + t * 2.6 + 0.9 + start, px * 5.3 - t * 4.2 + start);
        let height = 0.56 + 0.50 * t + swell * a.sin() + ripple * b.sin();
        // The distance to the line is measured square to it, so a steep part is as thin as a flat one.
        let slope = swell * 2.1 * a.cos() + ripple * 5.3 * b.cos();
        let from = (py - height).abs() / (1.0 + slope * slope).sqrt() * HEIGHT as f32;
        if from > 2.0 {
            continue;
        }
        let crest = (0.5 - 0.5 * a.sin()).powf(1.6);
        paint.over(mix(base, glint, 0.30 + 0.70 * crest), line(from, 0.55) * reach * (0.55 + 0.45 * crest) * (0.65 + 0.35 * t));
    }
    paint
}

/// A low sun over long, soft dunes, far ones pale and near ones deep. Dawn.
fn pasture(px: f32, py: f32, base: [u8; 3], glint: [u8; 3], seed: u32) -> Paint {
    let mut paint = Paint::default();
    let sun = (0.80 * ASPECT, 0.56);
    let d = distance(px, py, sun);
    // The sky warms toward the horizon.
    paint.over(mix(base, glint, 0.8), smooth((py - 0.15) / 0.6) * gauss((px - sun.0) / 1.1) * 0.28);
    paint.over(mix(base, glint, 0.9), gauss(d / 0.28) * 0.34);
    paint.over(mix(base, glint, 1.0), smooth((0.075 - d) / 0.03 + 0.5) * 0.5);
    // Each dune: where its crest is, how tall its swell, how fast it turns, its tint and how deep it is.
    const DUNES: [(f32, f32, f32, f32, f32); 4] =
        [(0.68, 0.10, 0.9, 0.80, 0.16), (0.77, 0.11, 1.2, 0.86, 0.20), (0.86, 0.10, 1.5, 0.92, 0.24), (0.95, 0.07, 1.9, 1.00, 0.30)];
    for (k, (crest, swell, turn, tint, depth)) in DUNES.iter().enumerate() {
        let k32 = k as f32;
        let ridge = crest + swell * (fbm(px * turn + k32 * 5.7, k32 * 3.1, 40 + seed + k as u32) - 0.5) * 2.0;
        let inside = ((py - ridge) * HEIGHT as f32 + 0.5).clamp(0.0, 1.0);
        if inside > 0.0 {
            let down = ((py - ridge) / 0.22).clamp(0.0, 1.0);
            paint.over(mix(base, glint, *tint), inside * depth * (1.0 - 0.55 * down));
        }
        // The crest catches the light, most where it passes under the sun.
        let lit = gauss((px - sun.0) / 0.55);
        paint.over(mix(base, glint, 1.0), line((py - ridge).abs() * HEIGHT as f32, 0.5) * lit * (0.75 - 0.12 * k32));
    }
    paint
}

/// Ribbons of light drifting across the dark, each with a bright thread in it. Psalm.
fn silk(px: f32, py: f32, base: [u8; 3], glint: [u8; 3], seed: u32) -> Paint {
    let mut paint = Paint::default();
    // Where each ribbon runs, how it sways, where its sway starts, how wide it is and how bright.
    const RIBBONS: [(f32, f32, f32, f32, f32); 3] =
        [(0.20, 1.9, 0.0, 0.095, 0.62), (0.36, 1.4, 2.1, 0.140, 0.40), (0.09, 2.6, 4.2, 0.060, 0.50)];
    let across = px / ASPECT;
    let reach = smooth((across - 0.12) / 0.75);
    for (k, (at, sway, phase, width, bright)) in RIBBONS.iter().enumerate() {
        let drift = (fbm(px * 0.9 + k as f32 * 9.0, k as f32, 30 + seed) - 0.5) * 0.26;
        let centre = at + 0.12 * (px * sway + phase + seed as f32).sin() + drift - (across - 0.5) * 0.10;
        let off = py - centre;
        // Soft above, sharper below, like light hanging from its upper edge.
        let spread = if off < 0.0 { *width } else { width * 0.55 };
        let tone = if k == 1 { 0.35 } else { 0.9 };
        paint.over(mix(base, glint, tone), gauss(off / spread) * bright * reach);
        for strand in 0..3 {
            let lift = (strand as f32 - 1.0) * width * 0.30 + 0.012 * (px * (4.0 + strand as f32) + phase).sin();
            let from = (off - lift).abs() * HEIGHT as f32;
            let much = if strand == 1 { 0.95 } else { 0.45 };
            paint.over(mix(base, glint, 1.0), line(from, 0.35) * reach * bright * much);
        }
    }
    paint
}

/// The contour lines of a map, every fifth one drawn stronger. Sharon.
fn contours(px: f32, py: f32, base: [u8; 3], glint: [u8; 3], seed: u32) -> Paint {
    let mut paint = Paint::default();
    let height = |x: f32, y: f32| (fbm(x * 1.25 + 0.4, y * 1.25, 61 + seed) * 0.88 + 0.12 * fbm(x * 3.0, y * 3.0, 62 + seed)) * 20.0;
    let band = height(px, py);
    // How fast the height changes per pixel, so a line is the same weight on a steep slope as on a flat one.
    let e = 1.0 / HEIGHT as f32;
    let slope = ((height(px + e, py) - band).powi(2) + (height(px, py + e) - band).powi(2)).sqrt().max(1e-4);
    let from_line = (band - band.round()).abs() / slope;
    let index = (band.round() as i32).rem_euclid(5) == 0;
    let reach = smooth(1.0 - distance(px, py, (0.86 * ASPECT, 0.22)) / 1.35);
    // High ground is tinted a little, in steps, as a printed map is.
    paint.over(mix(base, glint, 0.7), (band / 5.0).floor().max(0.0) * 0.045 * reach);
    let (tone, half, much) = if index { (1.0, 0.9, 0.95) } else { (0.15, 0.4, 0.6) };
    paint.over(mix(base, glint, tone), line(from_line, half) * reach * much);
    paint
}

/// Beams of morning light from beyond the upper right corner. Daybreak.
fn rays(px: f32, py: f32, base: [u8; 3], glint: [u8; 3], seed: u32) -> Paint {
    let mut paint = Paint::default();
    let source = (1.01 * ASPECT, -0.08);
    let r = distance(px, py, source);
    let angle = (py - source.1).atan2(px - source.0);
    let fade = gauss(r / 1.15);
    paint.over(mix(base, glint, 0.85), gauss(r / 0.55) * 0.60);
    for k in 0..9 {
        let at = std::f32::consts::PI * (0.50 + 0.052 * k as f32) + (hash2(k, 3, 71 + seed) - 0.5) * 0.045;
        let width = 0.008 + 0.030 * hash2(k, 5, 72 + seed);
        let strength = 0.20 + 0.40 * hash2(k, 7, 73 + seed);
        let tone = if k % 3 == 0 { 0.95 } else { 0.35 };
        paint.over(mix(base, glint, tone), gauss((angle - at) / width) * strength * fade);
    }
    paint
}

/// Four pools of light, each with its own colour, size and place; where none reaches, nothing shows.
fn glow(u: f32, v: f32, base: [u8; 3], glint: [u8; 3], seed: u32) -> Paint {
    let mut paint = Paint::default();
    for k in 0..4i32 {
        let at = |salt: i32| hash2(k, salt, 7 + seed);
        // The first pool comes from the top right (or wherever the corner says); the rest are scattered.
        let (cx, cy) = if k == 0 { (0.88, 0.12) } else { (0.15 + 0.8 * at(1), 0.10 + 0.8 * at(2)) };
        let radius = if k == 0 { 0.62 } else { 0.28 + 0.30 * at(3) };
        let d = (((u - cx) * ASPECT).powi(2) + (v - cy).powi(2)).sqrt() / (radius * ASPECT.min(1.4));
        let a = (-(d * d) * 2.2).exp() * if k == 0 { 0.95 } else { 0.55 + 0.35 * at(4) };
        let tone = if k % 2 == 0 { 0.85 } else { 0.25 + 0.4 * at(5) };
        paint.over(mix(base, glint, tone), a);
    }
    paint
}

/// Four colours, one at each corner, blended through a warped field so the seams curve.
fn mesh(u: f32, v: f32, base: [u8; 3], glint: [u8; 3], seed: u32) -> Paint {
    let wu = smooth(u + 0.20 * (fbm(u * 2.1, v * 2.1, 11 + seed) - 0.5));
    let wv = smooth(v + 0.20 * (fbm(u * 2.1 + 5.0, v * 2.1, 12 + seed) - 0.5));
    let corner = |n: u32| -> [f32; 3] {
        // Some corners are deep and some bright, so the mesh has depth instead of one muddy middle tone.
        let tone = mix(base, glint, [0.15, 0.95, 0.6, 0.9][((n + seed) % 4) as usize]);
        let depth = 0.35 + 0.65 * hash2(n as i32, 9, 21 + seed);
        tone.map(|c| c * depth)
    };
    let (a, b, c, d) = (corner(0), corner(1), corner(2), corner(3));
    let colour = [0, 1, 2].map(|i| (a[i] * (1.0 - wu) + b[i] * wu) * (1.0 - wv) + (c[i] * (1.0 - wu) + d[i] * wu) * wv);
    // Strongest where the mesh is lit, fading toward the opposite corner so it never ends at an edge.
    let lit = (1.0 - ((1.0 - u) * 0.55 + v * 0.55)).clamp(0.0, 1.0);
    Paint { colour, alpha: 0.55 + 0.45 * smooth(lit) }
}

/// One pixel: its colour and how much of it shows, both from 0 to 255. `u` and `v` run 0..1 across and down.
fn pixel(pattern: Pattern, u: f32, v: f32, base: [u8; 3], glint: [u8; 3], seed: u32) -> ([f32; 3], f32) {
    let (px, py) = (u * ASPECT, v);
    let paint = match pattern {
        Pattern::Ripples => ripples(px, py, base, glint, seed),
        Pattern::Pasture => pasture(px, py, base, glint, seed),
        Pattern::Silk => silk(px, py, base, glint, seed),
        Pattern::Contours => contours(px, py, base, glint, seed),
        Pattern::Rays => rays(px, py, base, glint, seed),
        Pattern::Glow => glow(u, v, base, glint, seed),
        Pattern::Mesh => mesh(u, v, base, glint, seed),
    };
    (paint.colour, paint.alpha * 255.0)
}

/// The picture as a PNG. `flip` mirrors it across and down, so the light can come from another corner; `seed`
/// rearranges it.
pub fn png(base: [u8; 3], glint: [u8; 3], pattern: Pattern, flip: (bool, bool), seed: u32) -> Vec<u8> {
    let mut rows = Vec::with_capacity((HEIGHT * (WIDTH * 4 + 1)) as usize);
    for y in 0..HEIGHT {
        rows.push(0);
        for x in 0..WIDTH {
            let (fx, fy) = (if flip.0 { WIDTH - 1 - x } else { x }, if flip.1 { HEIGHT - 1 - y } else { y });
            let (u, v) = ((fx as f32 + 0.5) / WIDTH as f32, (fy as f32 + 0.5) / HEIGHT as f32);
            let (colour, alpha) = pixel(pattern, u, v, base, glint, seed);
            // Grain: up to one step either way, so a slow fade does not band. Nothing is added where nothing is.
            let grain = if alpha > 1.0 { hash2(x as i32, y as i32, 5) * 2.0 - 1.0 } else { 0.0 };
            let alpha = (alpha + grain * 1.2).clamp(0.0, 255.0);
            rows.extend_from_slice(&[colour[0] as u8, colour[1] as u8, colour[2] as u8, alpha as u8]);
        }
    }
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut header = Vec::new();
    header.extend_from_slice(&WIDTH.to_be_bytes());
    header.extend_from_slice(&HEIGHT.to_be_bytes());
    header.extend_from_slice(&[8, 6, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &header);
    chunk(&mut out, b"IDAT", &zlib_stored(&rows));
    chunk(&mut out, b"IEND", &[]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const INK: [u8; 3] = [0xc0, 0xb0, 0xe0];
    const GLINT: [u8; 3] = [0xf0, 0xa0, 0x50];

    /// The average transparency of a patch, sampled every few pixels.
    fn weight(pattern: Pattern, u: (f32, f32), v: (f32, f32)) -> f32 {
        let mut sum = 0.0;
        for i in 0..40 {
            for j in 0..40 {
                let at = |r: (f32, f32), k: i32| r.0 + (r.1 - r.0) * (k as f32 + 0.5) / 40.0;
                sum += pixel(pattern, at(u, i), at(v, j), INK, GLINT, 0).1;
            }
        }
        sum / 1600.0
    }

    /// Every built-in theme's scene over its own background, written to `DEVD_SCENES` to be looked at.
    #[test]
    #[ignore = "writes pictures for a person to look at"]
    fn write_the_themes_for_looking_at() {
        let Ok(dir) = std::env::var("DEVD_SCENES") else {
            return;
        };
        for id in crate::theme::Theme::ORDER {
            let theme = crate::theme::Theme::by_id(id);
            let bg = gpui::Rgba::from(theme.bg);
            let ground = [bg.r, bg.g, bg.b].map(|c| c * 255.0);
            let mut rows = Vec::new();
            for y in 0..HEIGHT {
                rows.push(0);
                for x in 0..WIDTH {
                    let (u, v) = ((x as f32 + 0.5) / WIDTH as f32, (y as f32 + 0.5) / HEIGHT as f32);
                    // A scene that stands on the bottom is drawn as it is; the window never flips a built-in.
                    let (colour, alpha) = pixel(theme.pattern, u, v, theme.art_rgb(), theme.art_glint_rgb(), 0);
                    let a = alpha / 255.0 * theme.art_strength;
                    for (under, over) in ground.iter().zip(colour) {
                        rows.push((under * (1.0 - a) + over * a) as u8);
                    }
                    rows.push(255);
                }
            }
            let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
            let mut header = Vec::new();
            header.extend_from_slice(&WIDTH.to_be_bytes());
            header.extend_from_slice(&HEIGHT.to_be_bytes());
            header.extend_from_slice(&[8, 6, 0, 0, 0]);
            chunk(&mut out, b"IHDR", &header);
            chunk(&mut out, b"IDAT", &zlib_stored(&rows));
            chunk(&mut out, b"IEND", &[]);
            std::fs::write(format!("{dir}/{id}.png"), out).unwrap();
        }
    }

    #[test]
    fn every_scene_is_its_own_picture_and_changes_with_colours_corner_and_number() {
        let mut seen = Vec::new();
        for (_, pattern) in Pattern::ALL {
            let a = png(INK, GLINT, pattern, (false, false), 0);
            assert_eq!(&a[..8], b"\x89PNG\r\n\x1a\n");
            assert_ne!(a, png(INK, GLINT, pattern, (true, false), 0), "{pattern:?}: another corner");
            assert_ne!(a, png(GLINT, INK, pattern, (false, false), 0), "{pattern:?}: other colours");
            assert!(!seen.contains(&a), "{pattern:?} is another scene's picture");
            seen.push(a);
        }
    }

    #[test]
    fn the_lit_scenes_gather_where_they_come_from_and_stay_quiet_where_text_is_read() {
        for pattern in [Pattern::Ripples, Pattern::Silk, Pattern::Contours, Pattern::Rays] {
            let source = weight(pattern, (0.7, 1.0), (0.0, 0.45));
            let reading = weight(pattern, (0.0, 0.3), (0.65, 1.0));
            assert!(source > reading * 2.0 + 5.0, "{pattern:?}: {source} against {reading}");
        }
        let (dunes, sky) = (weight(Pattern::Pasture, (0.0, 1.0), (0.8, 1.0)), weight(Pattern::Pasture, (0.0, 0.5), (0.0, 0.25)));
        assert!(dunes > sky * 3.0, "{dunes} against {sky}");
    }
}
