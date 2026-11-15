//! Smooth backgrounds: pools of light and meshes of colour, drawn as a real picture, not as dots.
//!
//! Each is made here, small (the window scales it up, which only makes it softer), as an RGBA PNG whose colour
//! is the theme's own and whose transparency is the shape. A little grain is added to the transparency so a
//! slow gradient does not show bands on a dark window.

use super::dither::{Pattern, chunk, hash2, zlib_stored};

/// The picture's size in pixels. It is scaled to the window, so it is small on purpose.
pub const WIDTH: u32 = 480;
/// The picture's size in pixels.
pub const HEIGHT: u32 = 270;

fn mix(a: [u8; 3], b: [u8; 3], t: f32) -> [f32; 3] {
    let t = t.clamp(0.0, 1.0);
    [0, 1, 2].map(|i| a[i] as f32 * (1.0 - t) + b[i] as f32 * t)
}

fn smooth(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// A repeatable number in -1..1 that varies smoothly with the point.
fn wobble(x: f32, y: f32, seed: u32) -> f32 {
    let (xi, yi) = (x.floor(), y.floor());
    let (fx, fy) = (smooth(x - xi), smooth(y - yi));
    let h = |dx: i32, dy: i32| hash2(xi as i32 + dx, yi as i32 + dy, seed) * 2.0 - 1.0;
    let top = h(0, 0) + (h(1, 0) - h(0, 0)) * fx;
    let bottom = h(0, 1) + (h(1, 1) - h(0, 1)) * fx;
    top + (bottom - top) * fy
}

/// One pixel: its colour and how much of it shows, both from 0 to 255.
fn pixel(pattern: Pattern, u: f32, v: f32, base: [u8; 3], glint: [u8; 3], seed: u32) -> ([f32; 3], f32) {
    let aspect = WIDTH as f32 / HEIGHT as f32;
    match pattern {
        Pattern::Glow => {
            // Four pools of light, each with its own colour, size and place. Where they overlap they blend; where
            // none reaches, nothing shows, so the window's own colour is left alone.
            let mut colour = [0.0f32; 3];
            let mut weight = 0.0f32;
            let mut clear = 1.0f32;
            for k in 0..4u32 {
                let at = |salt: u32| hash2(k as i32, salt as i32, 7 + seed);
                // The first pool comes from the top right (or wherever the corner says); the rest are scattered.
                let (cx, cy) = if k == 0 { (0.88, 0.12) } else { (0.15 + 0.8 * at(1), 0.10 + 0.8 * at(2)) };
                let radius = if k == 0 { 0.62 } else { 0.28 + 0.30 * at(3) };
                let (dx, dy) = ((u - cx) * aspect, v - cy);
                let d = (dx * dx + dy * dy).sqrt() / (radius * aspect.min(1.4));
                let a = (-(d * d) * 2.2).exp() * if k == 0 { 0.95 } else { 0.55 + 0.35 * at(4) };
                let tone = mix(base, glint, if k % 2 == 0 { 0.85 } else { 0.25 + 0.4 * at(5) });
                for i in 0..3 {
                    colour[i] += tone[i] * a;
                }
                weight += a;
                clear *= 1.0 - a.min(0.98);
            }
            let colour = if weight > 0.0 { [colour[0] / weight, colour[1] / weight, colour[2] / weight] } else { [0.0; 3] };
            (colour, (1.0 - clear) * 255.0)
        }
        Pattern::Mesh => {
            // Four colours, one at each corner, blended across the window through a gently warped field so the
            // seams curve instead of running straight.
            let (wu, wv) = (u + 0.10 * wobble(u * 2.1, v * 2.1, 11 + seed), v + 0.10 * wobble(u * 2.1 + 5.0, v * 2.1, 12 + seed));
            let (wu, wv) = (smooth(wu), smooth(wv));
            let corner = |n: u32| -> [f32; 3] {
                // Some corners are deep and some are bright, so the mesh has depth instead of one muddy middle tone.
                let tone = mix(base, glint, [0.15, 0.95, 0.6, 0.9][((n + seed) % 4) as usize]);
                let depth = 0.35 + 0.65 * hash2(n as i32, 9, 21 + seed);
                tone.map(|c| c * depth)
            };
            let (a, b, c, d) = (corner(0), corner(1), corner(2), corner(3));
            let colour = [0, 1, 2].map(|i| (a[i] * (1.0 - wu) + b[i] * wu) * (1.0 - wv) + (c[i] * (1.0 - wu) + d[i] * wu) * wv);
            // Strongest where the mesh is lit, fading toward the opposite corner so it never ends at an edge.
            let lit = (1.0 - ((1.0 - u) * 0.55 + v * 0.55)).clamp(0.0, 1.0);
            (colour, (0.55 + 0.45 * smooth(lit)) * 255.0)
        }
        _ => ([0.0; 3], 0.0),
    }
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
            // Grain: up to one step either way, so a slow fade does not band.
            let grain = hash2(x as i32, y as i32, 5) * 2.0 - 1.0;
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

    #[test]
    fn a_soft_scene_is_a_real_picture_that_changes_with_its_colours_corner_and_number() {
        let (ink, glint) = ([0xc0, 0xb0, 0xe0], [0xf0, 0xa0, 0x50]);
        for pattern in [Pattern::Glow, Pattern::Mesh] {
            let a = png(ink, glint, pattern, (false, false), 0);
            assert_eq!(&a[..8], b"\x89PNG\r\n\x1a\n");
            assert_ne!(a, png(ink, glint, pattern, (true, false), 0), "{pattern:?}: another corner");
            assert_ne!(a, png(ink, glint, pattern, (false, false), 3), "{pattern:?}: another number");
            assert_ne!(a, png(glint, ink, pattern, (false, false), 0), "{pattern:?}: other colours");
        }
    }

    #[test]
    fn a_pool_of_light_is_brightest_at_its_centre_and_leaves_the_far_corner_clear() {
        let (_, centre) = pixel(Pattern::Glow, 0.88, 0.12, [255; 3], [255; 3], 0);
        let (_, far) = pixel(Pattern::Glow, 0.02, 0.98, [255; 3], [255; 3], 0);
        assert!(centre > 150.0, "{centre}");
        assert!(far < centre, "{far} < {centre}");
    }
}
