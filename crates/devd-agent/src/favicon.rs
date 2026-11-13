//! `web.favicon`: the small picture a site uses for itself, for the window to draw before a link.
//!
//! Claude and ChatGPT put a site's icon before each link in an answer. Doing it the usual way, by asking a
//! third party's icon service, would tell that service every site the assistant mentions. Here the icon
//! comes from the site itself, through the one guarded fetch devd has ([`crate::docs`]): https only, public
//! hosts only, every redirect re-checked, nothing sent but the request. It is looked up when a link is on
//! screen, kept on disk, and a miss is remembered too, so a site is asked about at most once a day.
//!
//! There is no image decoder here. A file is accepted only if its first bytes say it is a picture the
//! window can draw (PNG, JPEG, GIF, WebP or SVG), or an `.ico` that holds a PNG, whose PNG is taken out of
//! it. Anything else is a miss, and the window shows a lettered tile instead of a broken image.
//!
//! The parts that decide things are pure functions with tests; the network is passed in.

use std::path::Path;
use std::time::{Duration, SystemTime};

use serde_json::{json, Value};

/// How long a found icon is believed.
pub const FRESH_FOR: Duration = Duration::from_secs(14 * 24 * 3600);
/// How long a miss is believed.
pub const MISS_FOR: Duration = Duration::from_secs(24 * 3600);
/// The biggest icon file accepted. Real ones are a few kilobytes.
pub const MAX_BYTES: usize = 128 * 1024;

/// A picture format the window can draw.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// PNG.
    Png,
    /// JPEG.
    Jpeg,
    /// GIF.
    Gif,
    /// WebP.
    Webp,
    /// SVG.
    Svg,
}

impl Kind {
    /// The file extension it is kept under.
    pub fn ext(self) -> &'static str {
        match self {
            Kind::Png => "png",
            Kind::Jpeg => "jpg",
            Kind::Gif => "gif",
            Kind::Webp => "webp",
            Kind::Svg => "svg",
        }
    }
}

/// A host name as a safe file name and a fetchable host, or `None` when it is neither. Lower case, no
/// port, no trailing dot, only letters, digits, dots and hyphens, and at least one dot.
pub fn normalise(host: &str) -> Option<String> {
    let host = host.trim().trim_end_matches('.').to_ascii_lowercase();
    let host = host.rsplit_once(':').filter(|(_, port)| port.chars().all(|c| c.is_ascii_digit())).map(|(h, _)| h.to_string()).unwrap_or(host);
    let ok = !host.is_empty() && host.len() <= 253 && host.contains('.') && !host.starts_with(['.', '-']) && host.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '-') && !host.contains("..");
    ok.then_some(host)
}

/// What a file is, and the bytes to keep. An `.ico` is opened and its largest PNG taken out; one that holds
/// only bitmaps is not usable.
pub fn sniff(bytes: &[u8]) -> Option<(Kind, Vec<u8>)> {
    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n";
    if bytes.starts_with(PNG) {
        return Some((Kind::Png, bytes.to_vec()));
    }
    if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        return Some((Kind::Jpeg, bytes.to_vec()));
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return Some((Kind::Gif, bytes.to_vec()));
    }
    if bytes.len() > 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return Some((Kind::Webp, bytes.to_vec()));
    }
    // An ICO starts 00 00 01 00, a count, then 16-byte entries: width, height, colours, 0, planes, bits, size, offset.
    if bytes.len() > 6 && bytes[..4] == [0, 0, 1, 0] {
        return ico(bytes);
    }
    // SVG is text: look for the element near the start, after any XML header, comment or doctype.
    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(2048)]).to_ascii_lowercase();
    (head.trim_start().starts_with("<svg") || (head.trim_start().starts_with("<?xml") && head.contains("<svg"))).then(|| (Kind::Svg, bytes.to_vec()))
}

/// The picture inside an `.ico`: the entry closest to 32 pixels wide (not under 16 when there is a choice),
/// as a PNG. An entry that is already a PNG is used as it is; one that is a bitmap is re-encoded.
fn ico(bytes: &[u8]) -> Option<(Kind, Vec<u8>)> {
    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n";
    let count = u16::from_le_bytes([bytes[4], bytes[5]]) as usize;
    let mut best: Option<((bool, i32), Vec<u8>)> = None;
    for n in 0..count.min(32) {
        let at = 6 + n * 16;
        let Some(entry) = bytes.get(at..at + 16) else { break };
        let width = if entry[0] == 0 { 256 } else { entry[0] as i32 };
        let size = u32::from_le_bytes([entry[8], entry[9], entry[10], entry[11]]) as usize;
        let offset = u32::from_le_bytes([entry[12], entry[13], entry[14], entry[15]]) as usize;
        let Some(data) = offset.checked_add(size).and_then(|end| bytes.get(offset..end)) else { continue };
        let png = if data.starts_with(PNG) { Some(data.to_vec()) } else { bmp_to_png(data) };
        let Some(png) = png else { continue };
        let key = (width < 16, (width - 32).abs());
        if best.as_ref().is_none_or(|(k, _)| key < *k) {
            best = Some((key, png));
        }
    }
    best.map(|(_, png)| (Kind::Png, png))
}

/// A bitmap as stored inside an `.ico` (a header, a palette, bottom-up rows and a transparency mask), as a
/// PNG. Handles the uncompressed 1, 4, 8, 24 and 32 bit forms, which is all an icon uses.
fn bmp_to_png(d: &[u8]) -> Option<Vec<u8>> {
    let u32_at = |o: usize| d.get(o..o + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    if u32_at(0)? < 40 || u32_at(16)? != 0 {
        return None;
    }
    let header = u32_at(0)? as usize;
    let width = u32_at(4)? as usize;
    let height = (u32_at(8)? as i32).unsigned_abs() as usize / 2;
    let bits = u16::from_le_bytes([*d.get(14)?, *d.get(15)?]) as usize;
    if width == 0 || height == 0 || width > 256 || height > 256 || ![1, 4, 8, 24, 32].contains(&bits) {
        return None;
    }
    let palette_len = if bits <= 8 { match u32_at(32)? as usize { 0 => 1 << bits, n => n.min(1 << bits) } } else { 0 };
    let palette_at = header;
    let pixels_at = palette_at + palette_len * 4;
    let row = (width * bits).div_ceil(32) * 4;
    let mask_row = width.div_ceil(32) * 4;
    let mask_at = pixels_at + row * height;
    let mut rgba = vec![0u8; width * height * 4];
    let mut any_alpha = false;
    for y in 0..height {
        let src = pixels_at + (height - 1 - y) * row;
        for x in 0..width {
            let (r, g, b, a) = match bits {
                32 => {
                    let p = d.get(src + x * 4..src + x * 4 + 4)?;
                    (p[2], p[1], p[0], p[3])
                }
                24 => {
                    let p = d.get(src + x * 3..src + x * 3 + 3)?;
                    (p[2], p[1], p[0], 255)
                }
                _ => {
                    let per = 8 / bits;
                    let byte = *d.get(src + x / per)?;
                    let shift = 8 - bits * (x % per + 1);
                    let index = ((byte >> shift) as usize) & ((1 << bits) - 1);
                    let p = d.get(palette_at + index * 4..palette_at + index * 4 + 4)?;
                    (p[2], p[1], p[0], 255)
                }
            };
            any_alpha |= a != 0;
            let o = (y * width + x) * 4;
            rgba[o..o + 4].copy_from_slice(&[r, g, b, a]);
        }
    }
    // Below 32 bits, or a 32-bit picture with no alpha at all, transparency is the mask: a set bit is see-through.
    if bits < 32 || !any_alpha {
        for y in 0..height {
            let src = mask_at + (height - 1 - y) * mask_row;
            for x in 0..width {
                let Some(byte) = d.get(src + x / 8) else { break };
                let clear = byte >> (7 - x % 8) & 1 == 1;
                rgba[(y * width + x) * 4 + 3] = if clear { 0 } else { 255 };
            }
        }
    }
    Some(png_rgba(width as u32, height as u32, &rgba))
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

/// An RGBA picture as a PNG, its data stored rather than compressed: valid, and an icon is tiny.
fn png_rgba(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
    let mut raw = Vec::with_capacity(rgba.len() + height as usize);
    for row in rgba.chunks(width as usize * 4) {
        raw.push(0);
        raw.extend_from_slice(row);
    }
    let mut z = vec![0x78, 0x01];
    let mut blocks = raw.chunks(65_535).peekable();
    while let Some(block) = blocks.next() {
        z.push(u8::from(blocks.peek().is_none()));
        z.extend_from_slice(&(block.len() as u16).to_le_bytes());
        z.extend_from_slice(&(!(block.len() as u16)).to_le_bytes());
        z.extend_from_slice(block);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for &x in &raw {
        a = (a + x as u32) % 65_521;
        b = (b + a) % 65_521;
    }
    z.extend_from_slice(&((b << 16) | a).to_be_bytes());
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut chunk = |kind: &[u8; 4], data: &[u8]| {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let start = out.len();
        out.extend_from_slice(kind);
        out.extend_from_slice(data);
        let crc = crc32(&out[start..]);
        out.extend_from_slice(&crc.to_be_bytes());
    };
    let mut header = Vec::new();
    header.extend_from_slice(&width.to_be_bytes());
    header.extend_from_slice(&height.to_be_bytes());
    header.extend_from_slice(&[8, 6, 0, 0, 0]);
    chunk(b"IHDR", &header);
    chunk(b"IDAT", &z);
    chunk(b"IEND", &[]);
    out
}

/// An icon a page names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    /// Where the page says it is.
    pub href: String,
    /// How good it looks for a 14-pixel mark: higher is better.
    pub score: u32,
}

fn attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let lower = tag.to_ascii_lowercase();
    let mut from = 0;
    while let Some(found) = lower[from..].find(name) {
        let at = from + found;
        let before_ok = lower[..at].chars().last().is_none_or(|c| c.is_whitespace() || c == '"' || c == '\'');
        let rest = tag[at + name.len()..].trim_start();
        if before_ok && rest.starts_with('=') {
            let value = rest[1..].trim_start();
            return match value.chars().next()? {
                q @ ('"' | '\'') => value[1..].split(q).next(),
                _ => value.split(|c: char| c.is_whitespace() || c == '>').next(),
            };
        }
        from = at + name.len();
    }
    None
}

/// The icons a page's `<link>` tags name, best first. A vector icon scores highest, then a raster one near
/// 32 to 64 pixels; a very large touch icon scores low, because it would be shrunk to nothing.
pub fn candidates(html: &str) -> Vec<Candidate> {
    let lower = html.to_ascii_lowercase();
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(found) = lower[from..].find("<link") {
        let start = from + found;
        let end = lower[start..].find('>').map(|e| start + e).unwrap_or(lower.len());
        let tag = &html[start..end];
        from = end.max(start + 5);
        let Some(rel) = attr(tag, "rel").map(str::to_ascii_lowercase) else { continue };
        if !rel.split_whitespace().any(|r| r == "icon" || r == "apple-touch-icon" || r == "shortcut") {
            continue;
        }
        let Some(href) = attr(tag, "href").map(str::trim).filter(|h| !h.is_empty() && !h.starts_with("data:")) else { continue };
        let kind = attr(tag, "type").unwrap_or_default().to_ascii_lowercase();
        let size = attr(tag, "sizes").and_then(|s| s.split(['x', 'X']).next().and_then(|w| w.trim().parse::<u32>().ok()));
        let score = if kind.contains("svg") || href.to_ascii_lowercase().ends_with(".svg") {
            100
        } else {
            match size {
                Some(32..=96) => 80,
                Some(16..=31) => 60,
                Some(97..=256) => 50,
                Some(_) => 20,
                None => 55,
            }
        };
        out.push(Candidate { href: href.to_string(), score });
    }
    out.sort_by_key(|c| std::cmp::Reverse(c.score));
    out.dedup_by(|a, b| a.href == b.href);
    out
}

/// An address for an icon the page named. Only web addresses: `http` is upgraded, a protocol-relative one
/// takes `https`, a path is read against the site, and anything else (a `data:` address, a script) is none.
pub fn resolve(host: &str, href: &str) -> Option<String> {
    let href = href.trim();
    let url = if let Some(rest) = href.strip_prefix("https://") {
        format!("https://{rest}")
    } else if let Some(rest) = href.strip_prefix("http://") {
        format!("https://{rest}")
    } else if let Some(rest) = href.strip_prefix("//") {
        format!("https://{rest}")
    } else if href.contains(':') {
        return None;
    } else if let Some(path) = href.strip_prefix('/') {
        format!("https://{host}/{path}")
    } else {
        format!("https://{host}/{}", href.trim_start_matches("./"))
    };
    (!url.contains(char::is_whitespace)).then_some(url)
}

/// Where a host's icon is kept and whether it is still believed.
enum Cached {
    Found(std::path::PathBuf),
    Missing,
    Stale,
}

fn age(path: &Path, now: SystemTime) -> Option<Duration> {
    std::fs::metadata(path).ok()?.modified().ok().and_then(|m| now.duration_since(m).ok())
}

fn cached(dir: &Path, host: &str, now: SystemTime) -> Cached {
    for ext in ["png", "svg", "jpg", "gif", "webp"] {
        let path = dir.join(format!("{host}.{ext}"));
        if age(&path, now).is_some_and(|a| a < FRESH_FOR) {
            return Cached::Found(path);
        }
    }
    if age(&dir.join(format!("{host}.none")), now).is_some_and(|a| a < MISS_FOR) {
        return Cached::Missing;
    }
    Cached::Stale
}

/// The network, as far as this module needs it: a page's text and a small file's bytes.
pub trait Web {
    /// The page at this address, as text.
    fn page(&self, url: &str) -> Option<String>;
    /// A file at this address, if it is small enough and reachable.
    fn file(&self, url: &str) -> Option<Vec<u8>>;
}

/// Find, check and keep a site's icon. The answer is `{"found": true, "path": ..., "kind": ...}` or
/// `{"found": false, "reason": ...}`; nothing here is an error, because a site without an icon is ordinary.
pub fn lookup(dir: &Path, host: &str, web: &dyn Web, now: SystemTime) -> Value {
    let Some(host) = normalise(host) else { return json!({ "found": false, "reason": "that is not a host name" }) };
    match cached(dir, &host, now) {
        Cached::Found(path) => return json!({ "found": true, "path": path.display().to_string(), "cached": true }),
        Cached::Missing => return json!({ "found": false, "reason": "the site has no icon we could use", "cached": true }),
        Cached::Stale => {}
    }
    let _ = std::fs::create_dir_all(dir);
    let mut tried: Vec<String> = Vec::new();
    let mut urls: Vec<String> = web.page(&format!("https://{host}/")).map(|html| candidates(&html).into_iter().filter_map(|c| resolve(&host, &c.href)).take(3).collect()).unwrap_or_default();
    urls.push(format!("https://{host}/favicon.ico"));
    for url in urls {
        if tried.contains(&url) {
            continue;
        }
        tried.push(url.clone());
        let Some(bytes) = web.file(&url) else { continue };
        let Some((kind, data)) = sniff(&bytes) else { continue };
        let path = dir.join(format!("{host}.{}", kind.ext()));
        // A newer file under another extension replaces an older one.
        for ext in ["png", "svg", "jpg", "gif", "webp"] {
            let _ = std::fs::remove_file(dir.join(format!("{host}.{ext}")));
        }
        let _ = std::fs::remove_file(dir.join(format!("{host}.none")));
        if std::fs::write(&path, data).is_ok() {
            return json!({ "found": true, "path": path.display().to_string(), "kind": kind.ext(), "from": url });
        }
    }
    let _ = std::fs::write(dir.join(format!("{host}.none")), b"");
    json!({ "found": false, "reason": "the site has no icon we could use" })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashMap;

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\nrest-of-a-picture";

    #[test]
    fn a_host_is_made_safe_to_use_as_a_file_name_or_refused() {
        assert_eq!(normalise("Example.COM."), Some("example.com".into()));
        assert_eq!(normalise("docs.rs:443"), Some("docs.rs".into()));
        for bad in ["", "localhost", "a/b.com", "..", "a..b.com", "exa mple.com", "ex%41mple.com", "-x.com", "x.com/../../etc", "é.com"] {
            assert_eq!(normalise(bad), None, "{bad}");
        }
        assert_eq!(normalise(&format!("{}.com", "a".repeat(260))), None);
    }

    #[test]
    fn a_file_is_kept_only_when_its_first_bytes_say_it_is_a_picture() {
        assert_eq!(sniff(PNG).unwrap().0, Kind::Png);
        assert_eq!(sniff(&[0xff, 0xd8, 0xff, 0xe0, 1, 2]).unwrap().0, Kind::Jpeg);
        assert_eq!(sniff(b"GIF89a....").unwrap().0, Kind::Gif);
        assert_eq!(sniff(b"RIFF\x00\x00\x00\x00WEBPVP8 ").unwrap().0, Kind::Webp);
        assert_eq!(sniff(b"<?xml version=\"1.0\"?><!-- c --><svg xmlns=\"x\"></svg>").unwrap().0, Kind::Svg);
        assert_eq!(sniff(b"  <svg viewBox=\"0 0 1 1\"/>").unwrap().0, Kind::Svg);
        for not in [&b"<html><body>Not found</body></html>"[..], b"", b"MZ\x90\x00 an executable", b"{\"icon\": 1}", b"<script>alert(1)</script>"] {
            assert!(sniff(not).is_none(), "{:?}", String::from_utf8_lossy(not));
        }
    }

    /// The stored picture of a PNG made by `png_rgba`: its width, height and pixels, read back.
    fn read_back(png: &[u8]) -> (u32, u32, Vec<u8>) {
        assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
        let width = u32::from_be_bytes(png[16..20].try_into().unwrap());
        let height = u32::from_be_bytes(png[20..24].try_into().unwrap());
        let idat = png.windows(4).position(|w| w == b"IDAT").unwrap();
        let len = u32::from_be_bytes(png[idat - 4..idat].try_into().unwrap()) as usize;
        let z = &png[idat + 4..idat + 4 + len];
        // zlib header, then stored blocks (flag, len, !len, data), then the checksum.
        let (mut at, mut raw) = (2, Vec::new());
        loop {
            let last = z[at] == 1;
            let n = u16::from_le_bytes([z[at + 1], z[at + 2]]) as usize;
            raw.extend_from_slice(&z[at + 5..at + 5 + n]);
            at += 5 + n;
            if last {
                break;
            }
        }
        let pixels: Vec<u8> = raw.chunks(width as usize * 4 + 1).flat_map(|row| row[1..].to_vec()).collect();
        (width, height, pixels)
    }

    fn entry(width: u8, size: usize, offset: usize) -> Vec<u8> {
        let mut e = vec![width, width, 0, 0, 1, 0, 32, 0];
        e.extend_from_slice(&(size as u32).to_le_bytes());
        e.extend_from_slice(&(offset as u32).to_le_bytes());
        e
    }

    #[test]
    fn an_ico_gives_up_the_png_nearest_32_pixels() {
        let small = [PNG, b"small"].concat();
        let mid = [PNG, b"the-middle-one"].concat();
        let huge = [PNG, b"the-huge-one"].concat();
        let start = 6 + 3 * 16;
        let mut ico = vec![0, 0, 1, 0, 3, 0];
        ico.extend(entry(16, small.len(), start));
        ico.extend(entry(32, mid.len(), start + small.len()));
        ico.extend(entry(0, huge.len(), start + small.len() + mid.len()));
        for data in [&small, &mid, &huge] {
            ico.extend_from_slice(data);
        }
        assert_eq!(sniff(&ico), Some((Kind::Png, mid)));
    }

    #[test]
    fn a_32_bit_bitmap_icon_becomes_a_png_with_its_pixels_the_right_way_up() {
        // 2x2, bottom row first, each pixel B G R A. The top row is red then green; the bottom is blue then a half-clear white.
        let mut d = vec![40, 0, 0, 0, 2, 0, 0, 0, 4, 0, 0, 0, 1, 0, 32, 0, 0, 0, 0, 0, 16, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        d.extend_from_slice(&[255, 0, 0, 255, 255, 255, 255, 128]); // bottom row: blue, white at half alpha
        d.extend_from_slice(&[0, 0, 255, 255, 0, 255, 0, 255]); // top row: red, green
        d.extend_from_slice(&[0; 8]); // the mask, unused at 32 bits when there is alpha
        let mut ico = vec![0, 0, 1, 0, 1, 0];
        ico.extend(entry(2, d.len(), 22));
        ico.extend_from_slice(&d);
        let (kind, png) = sniff(&ico).expect("a bitmap icon is usable");
        assert_eq!(kind, Kind::Png);
        let (w, h, px) = read_back(&png);
        assert_eq!((w, h), (2, 2));
        assert_eq!(px, [255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 128], "red, green over blue, half-clear white");
    }

    #[test]
    fn a_palette_bitmap_uses_its_mask_for_transparency() {
        // 2x2 at 8 bits: a two-colour palette, indices 0 and 1, and a mask clearing the bottom-right pixel.
        let mut d = vec![40, 0, 0, 0, 2, 0, 0, 0, 4, 0, 0, 0, 1, 0, 8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0];
        d.extend_from_slice(&[0, 0, 255, 0, 255, 0, 0, 0]); // palette: red, blue (B G R reserved)
        d.extend_from_slice(&[0, 1, 0, 0]); // bottom row: indices 0, 1, padded to 4
        d.extend_from_slice(&[1, 0, 0, 0]); // top row: indices 1, 0
        d.extend_from_slice(&[0b0100_0000, 0, 0, 0, 0, 0, 0, 0]); // mask rows: bottom clears x=1; top clears nothing
        let (_, png) = ico_png(&d);
        let (_, _, px) = read_back(&png);
        assert_eq!(&px[0..8], &[0, 0, 255, 255, 255, 0, 0, 255], "top row: blue, red");
        assert_eq!(&px[8..12], &[255, 0, 0, 255], "bottom-left is red");
        assert_eq!(px[15], 0, "bottom-right is see-through");
    }

    fn ico_png(bitmap: &[u8]) -> (Kind, Vec<u8>) {
        let mut ico = vec![0, 0, 1, 0, 1, 0];
        ico.extend(entry(2, bitmap.len(), 22));
        ico.extend_from_slice(bitmap);
        sniff(&ico).expect("usable")
    }

    #[test]
    fn a_broken_or_hostile_icon_is_a_miss_and_never_a_panic() {
        // Compressed bitmaps, a huge declared size, a directory pointing past the end, and truncations of a good one.
        assert_eq!(sniff(&[0, 0, 1, 0, 9, 0, 16, 16, 0, 0, 1, 0, 32, 0, 255, 255, 255, 0, 255, 255, 255, 0]), None);
        let compressed = [40, 0, 0, 0, 2, 0, 0, 0, 4, 0, 0, 0, 1, 0, 32, 0, 1, 0, 0, 0];
        let mut ico = vec![0, 0, 1, 0, 1, 0];
        ico.extend(entry(2, compressed.len(), 22));
        ico.extend_from_slice(&compressed);
        assert_eq!(sniff(&ico), None);
        let huge = [40, 0, 0, 0, 255, 255, 255, 127, 255, 255, 255, 127, 1, 0, 32, 0, 0, 0, 0, 0];
        let mut ico = vec![0, 0, 1, 0, 1, 0];
        ico.extend(entry(2, huge.len(), 22));
        ico.extend_from_slice(&huge);
        assert_eq!(sniff(&ico), None);
        let mut good = vec![40, 0, 0, 0, 2, 0, 0, 0, 4, 0, 0, 0, 1, 0, 32, 0, 0, 0, 0, 0, 16, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        good.extend_from_slice(&[1; 24]);
        for cut in 0..good.len() {
            let mut ico = vec![0, 0, 1, 0, 1, 0];
            ico.extend(entry(2, cut, 22));
            ico.extend_from_slice(&good[..cut]);
            let _ = sniff(&ico);
        }
    }

    #[test]
    fn the_best_icon_a_page_names_comes_first() {
        let html = r#"<head>
            <link rel="icon" href="/small.png" sizes="16x16" type="image/png">
            <link rel="apple-touch-icon" sizes="180x180" href="/touch.png">
            <link rel='icon' href='/best.svg' type='image/svg+xml'>
            <link rel="icon" href="/mid.png" sizes="32x32">
            <link rel="stylesheet" href="/style.css">
            <link rel="icon" href="data:image/png;base64,AAAA">
            <LINK REL="SHORTCUT ICON" HREF=/old.ico>
        </head>"#;
        let found: Vec<String> = candidates(html).into_iter().map(|c| c.href).collect();
        assert_eq!(found[0], "/best.svg");
        assert_eq!(found[1], "/mid.png");
        assert!(found.contains(&"/old.ico".to_string()) && found.contains(&"/small.png".to_string()));
        assert!(!found.contains(&"/style.css".to_string()) && !found.iter().any(|h| h.starts_with("data:")));
    }

    #[test]
    fn an_icon_address_is_made_absolute_and_never_a_script() {
        assert_eq!(resolve("a.com", "/i.png").as_deref(), Some("https://a.com/i.png"));
        assert_eq!(resolve("a.com", "i.png").as_deref(), Some("https://a.com/i.png"));
        assert_eq!(resolve("a.com", "./i.png").as_deref(), Some("https://a.com/i.png"));
        assert_eq!(resolve("a.com", "//cdn.b.com/i.png").as_deref(), Some("https://cdn.b.com/i.png"));
        assert_eq!(resolve("a.com", "http://b.com/i.png").as_deref(), Some("https://b.com/i.png"));
        for bad in ["javascript:alert(1)", "data:image/png;base64,AA", "file:///c:/x", "ftp://x/y", "/a b.png"] {
            assert_eq!(resolve("a.com", bad), None, "{bad}");
        }
    }

    struct Fake {
        pages: HashMap<String, String>,
        files: HashMap<String, Vec<u8>>,
        asked: RefCell<Vec<String>>,
    }

    impl Web for Fake {
        fn page(&self, url: &str) -> Option<String> {
            self.asked.borrow_mut().push(url.to_string());
            self.pages.get(url).cloned()
        }
        fn file(&self, url: &str) -> Option<Vec<u8>> {
            self.asked.borrow_mut().push(url.to_string());
            self.files.get(url).cloned()
        }
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("devd-favicon-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn the_icon_is_found_from_the_page_kept_and_not_asked_for_again() {
        let dir = scratch("found");
        let web = Fake { pages: HashMap::from([("https://a.com/".into(), r#"<link rel="icon" href="/x.png" sizes="32x32">"#.into())]), files: HashMap::from([("https://a.com/x.png".into(), PNG.to_vec())]), asked: RefCell::new(Vec::new()) };
        let first = lookup(&dir, "A.com", &web, SystemTime::now());
        assert_eq!(first["found"], true, "{first}");
        assert!(std::fs::read(first["path"].as_str().unwrap()).unwrap().starts_with(b"\x89PNG"));
        let asked = web.asked.borrow().len();
        let again = lookup(&dir, "a.com", &web, SystemTime::now());
        assert_eq!((again["found"].clone(), again["cached"].clone()), (json!(true), json!(true)));
        assert_eq!(web.asked.borrow().len(), asked, "a kept icon is not fetched again");
        // After the freshness window it is asked for again.
        let later = lookup(&dir, "a.com", &web, SystemTime::now() + FRESH_FOR + Duration::from_secs(60));
        assert!(web.asked.borrow().len() > asked && later["found"] == true, "{later}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn it_falls_back_to_favicon_ico_and_remembers_a_miss() {
        let dir = scratch("fallback");
        let web = Fake { pages: HashMap::new(), files: HashMap::from([("https://b.com/favicon.ico".into(), [0u8, 0, 1, 0, 1, 0, 16, 16, 0, 0, 1, 0, 32, 0, 12, 0, 0, 0, 22, 0, 0, 0].iter().copied().chain(PNG.iter().copied().take(12)).collect())]), asked: RefCell::new(Vec::new()) };
        // A one-entry ICO whose data is the first 12 bytes of a PNG: not a whole PNG signature plus data? It is 8 + 4 bytes, so it is.
        let got = lookup(&dir, "b.com", &web, SystemTime::now());
        assert_eq!(got["found"], true, "{got}");

        let none = Fake { pages: HashMap::from([("https://c.com/".into(), "<html>no icons</html>".into())]), files: HashMap::new(), asked: RefCell::new(Vec::new()) };
        let miss = lookup(&dir, "c.com", &none, SystemTime::now());
        assert_eq!(miss["found"], false);
        let asked = none.asked.borrow().len();
        assert!(asked >= 2, "the page and /favicon.ico were tried");
        let again = lookup(&dir, "c.com", &none, SystemTime::now());
        assert_eq!(again["cached"], true);
        assert_eq!(none.asked.borrow().len(), asked, "a miss is remembered for a day");
        let tomorrow = lookup(&dir, "c.com", &none, SystemTime::now() + MISS_FOR + Duration::from_secs(60));
        assert_eq!(tomorrow["found"], false);
        assert!(none.asked.borrow().len() > asked, "and then asked again");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_page_that_answers_with_something_else_is_a_miss_and_a_bad_host_is_refused_up_front() {
        let dir = scratch("liar");
        let web = Fake { pages: HashMap::from([("https://d.com/".into(), r#"<link rel="icon" href="/i.png">"#.into())]), files: HashMap::from([("https://d.com/i.png".into(), b"<html>pretending</html>".to_vec()), ("https://d.com/favicon.ico".into(), b"<script>x</script>".to_vec())]), asked: RefCell::new(Vec::new()) };
        assert_eq!(lookup(&dir, "d.com", &web, SystemTime::now())["found"], false);
        let refused = lookup(&dir, "../../etc", &web, SystemTime::now());
        assert_eq!(refused["found"], false);
        assert!(refused["reason"].as_str().unwrap().contains("not a host name"));
        assert!(!dir.join("..").join("..").join("etc.none").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
