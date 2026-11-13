//! Motion: films made by writing a page and rendering it, with nothing to install.
//!
//! A composition is an HTML page that is a pure function of time (see the
//! runtime it loads, `motion/devd-motion.js`). devd serves the folder on a
//! loopback port, opens the page in its own browser at the composition's size,
//! asks for each frame's time in turn, and photographs the page. The result is a
//! numbered sequence of PNG frames that is the same on every render, and an MP4
//! when ffmpeg is present.
//!
//! Everything here is built in: the runtime, the template, the file server and
//! the frame capture ship inside devd. The one thing that does not is the
//! encoder. Turning frames into a video needs an encoder, and devd does not
//! carry one: it uses an `ffmpeg` already on the machine (or the path in
//! `DEVD_FFMPEG`), and when there is none it says so and still delivers the
//! frames, which every video tool can assemble.
//!
//! The render is deterministic because the page never runs on its own clock:
//! nothing animates between two frames, so nothing can be caught half way.

use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};

use devd_core::{DevdError, Result};

/// The runtime every composition loads.
pub const RUNTIME: &str = include_str!("../motion/devd-motion.js");
const TEMPLATE: &str = include_str!("../motion/template.html");

/// The most frames one render may make: an hour of video is not a thing to start by accident.
pub const MAX_FRAMES: usize = 18_000;

/// What a new composition looks like.
#[derive(Debug, Clone)]
pub struct Spec {
    /// What it is called, and the first words on screen.
    pub title: String,
    /// Frame width in pixels.
    pub width: u32,
    /// Frame height in pixels.
    pub height: u32,
    /// Frames a second.
    pub fps: u32,
    /// How long it runs.
    pub seconds: f64,
}

impl Default for Spec {
    fn default() -> Self {
        Self { title: "Untitled".into(), width: 1920, height: 1080, fps: 30, seconds: 6.0 }
    }
}

fn html_escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// Create a composition folder: `index.html` and the runtime beside it.
///
/// Refuses to overwrite anything: a person's film is worth more than a fresh
/// template.
pub fn scaffold(dir: &Path, spec: &Spec) -> Result<Value> {
    if spec.width < 64 || spec.height < 64 || spec.width > 7680 || spec.height > 4320 {
        return Err(DevdError::invalid("the size must be between 64x64 and 7680x4320"));
    }
    if spec.fps == 0 || spec.fps > 120 {
        return Err(DevdError::invalid("fps must be between 1 and 120"));
    }
    if !(spec.seconds > 0.0 && spec.seconds * f64::from(spec.fps) <= MAX_FRAMES as f64) {
        return Err(DevdError::invalid(format!("the length must be more than 0 and at most {MAX_FRAMES} frames")));
    }
    let index = dir.join("index.html");
    let runtime = dir.join("devd-motion.js");
    for existing in [&index, &runtime] {
        if existing.exists() {
            return Err(DevdError::conflict(format!("{} already exists; open it instead of starting over", existing.display())));
        }
    }
    std::fs::create_dir_all(dir)?;
    let page = TEMPLATE
        .replace("__TITLE__", &html_escape(&spec.title))
        .replace("__WIDTH__", &spec.width.to_string())
        .replace("__HEIGHT__", &spec.height.to_string())
        .replace("__FPS__", &spec.fps.to_string())
        .replace("__SECONDS__", &spec.seconds.to_string());
    std::fs::write(&index, page)?;
    std::fs::write(&runtime, RUNTIME)?;
    Ok(json!({
        "dir": dir.display().to_string(),
        "composition": index.display().to_string(),
        "runtime": runtime.display().to_string(),
        "width": spec.width, "height": spec.height, "fps": spec.fps, "seconds": spec.seconds,
        "frames": (spec.seconds * f64::from(spec.fps)).round() as u64,
    }))
}

fn content_type(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref() {
        Some("html") | Some("htm") => "text/html; charset=utf-8",
        Some("js") | Some("mjs") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("json") => "application/json",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("webp") => "image/webp",
        Some("gif") => "image/gif",
        Some("woff2") => "font/woff2",
        Some("woff") => "font/woff",
        Some("ttf") => "font/ttf",
        Some("otf") => "font/otf",
        Some("wav") => "audio/wav",
        Some("mp3") => "audio/mpeg",
        Some("mp4") => "video/mp4",
        Some("webm") => "video/webm",
        _ => "application/octet-stream",
    }
}

/// The file a request path names, inside `root`, or nothing.
///
/// The server exists to serve one composition's folder to the browser devd
/// started, on a loopback port. A request that leaves the folder, by `..`, a
/// drive letter, an encoded separator or a link, is refused.
fn resolve(root: &Path, request_path: &str) -> Option<PathBuf> {
    let path = request_path.split(['?', '#']).next().unwrap_or("");
    let decoded = percent_decode(path)?;
    let relative = decoded.trim_start_matches('/');
    let relative = if relative.is_empty() { "index.html" } else { relative };
    if relative.contains('\\') || relative.contains('\0') {
        return None;
    }
    let mut full = root.to_path_buf();
    for part in Path::new(relative).components() {
        match part {
            Component::Normal(p) => full.push(p),
            _ => return None,
        }
    }
    let canonical = full.canonicalize().ok()?;
    let base = root.canonicalize().ok()?;
    (canonical.starts_with(&base) && canonical.is_file()).then_some(canonical)
}

fn percent_decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = text.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

fn serve_one(mut stream: TcpStream, root: &Path) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let mut buffer = [0u8; 4096];
    let Ok(n) = stream.read(&mut buffer) else { return };
    let head = String::from_utf8_lossy(&buffer[..n]);
    let mut first = head.lines().next().unwrap_or("").split_whitespace();
    let (method, target) = (first.next().unwrap_or(""), first.next().unwrap_or("/"));
    let reply = |stream: &mut TcpStream, status: &str, kind: &str, body: &[u8]| {
        let header = format!("HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n", body.len());
        let _ = stream.write_all(header.as_bytes());
        let _ = stream.write_all(body);
    };
    if method != "GET" {
        reply(&mut stream, "405 Method Not Allowed", "text/plain", b"GET only");
    } else {
        match resolve(root, target).and_then(|p| std::fs::read(&p).ok().map(|b| (p, b))) {
            Some((path, body)) => reply(&mut stream, "200 OK", content_type(&path), &body),
            None => reply(&mut stream, "404 Not Found", "text/plain", b"not found"),
        }
    }
    let _ = stream.shutdown(Shutdown::Both);
}

/// A loopback file server for one folder, stopped when dropped.
pub struct Server {
    /// The address the browser opens.
    pub base: String,
    stop: Arc<AtomicBool>,
}

impl Server {
    /// Serve `root` on an unused loopback port.
    pub fn start(root: &Path) -> Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", 0))?;
        listener.set_nonblocking(true)?;
        let port = listener.local_addr()?.port();
        let stop = Arc::new(AtomicBool::new(false));
        let (flag, folder) = (stop.clone(), root.to_path_buf());
        std::thread::Builder::new().name("devd-motion-files".into()).spawn(move || {
            while !flag.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let folder = folder.clone();
                        std::thread::spawn(move || {
                            let _ = stream.set_nonblocking(false);
                            serve_one(stream, &folder);
                        });
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(15)),
                    Err(_) => break,
                }
            }
        })?;
        Ok(Self { base: format!("http://127.0.0.1:{port}"), stop })
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// What to render.
#[derive(Debug, Clone, Default)]
pub struct Render {
    /// Frames a second; the composition's own when absent.
    pub fps: Option<u32>,
    /// Only these moments, in seconds, as stills: for looking at a film
    /// without waiting for all of it.
    pub at: Vec<f64>,
    /// The video to write, inside the composition folder. `out/<name>.mp4` by default.
    pub out: Option<String>,
    /// A sound file inside the composition folder to put under the picture.
    pub audio: Option<String>,
    /// H.264 quality, lower is better (default 18).
    pub crf: Option<u32>,
}

/// An `ffmpeg` this machine has.
pub fn find_ffmpeg() -> Option<PathBuf> {
    let candidate = |p: PathBuf| Command::new(&p).arg("-version").stdout(Stdio::null()).stderr(Stdio::null()).status().ok().filter(|s| s.success()).map(|_| p);
    if let Some(explicit) = std::env::var_os("DEVD_FFMPEG") {
        return candidate(PathBuf::from(explicit));
    }
    candidate(PathBuf::from("ffmpeg"))
}

/// The ffmpeg arguments that turn a numbered PNG sequence into H.264.
pub fn encode_args(frames: &Path, fps: u32, audio: Option<&Path>, crf: u32, out: &Path) -> Vec<String> {
    let mut args: Vec<String> = ["-y", "-hide_banner", "-loglevel", "error", "-framerate"].iter().map(|s| s.to_string()).collect();
    args.push(fps.to_string());
    args.extend(["-i".into(), frames.join("frame_%05d.png").display().to_string()]);
    if let Some(audio) = audio {
        args.extend(["-i".into(), audio.display().to_string()]);
    }
    args.extend(["-c:v".into(), "libx264".into(), "-pix_fmt".into(), "yuv420p".into(), "-crf".into(), crf.to_string(), "-movflags".into(), "+faststart".into()]);
    if audio.is_some() {
        args.extend(["-c:a".into(), "aac".into(), "-b:a".into(), "192k".into(), "-shortest".into()]);
    }
    args.push(out.display().to_string());
    args
}

/// A path inside `dir`, from a relative name.
fn inside(dir: &Path, name: &str) -> Result<PathBuf> {
    // The same rule on every system: a backslash separates folders, and a colon (a drive, or a Windows stream) is never
    // part of a plain name. Otherwise `C:\x.mp4` would be a harmless file name on one system and an escape on another.
    let plain = name.replace('\\', "/");
    let p = Path::new(&plain);
    if name.contains(':') || p.is_absolute() || p.components().any(|c| !matches!(c, Component::Normal(_))) {
        return Err(DevdError::invalid(format!("`{name}` must be a plain path inside the composition's folder")));
    }
    Ok(dir.join(p))
}

/// Render a composition. `progress` is called with how many frames are done.
pub fn render(root: &Path, dir: &Path, options: &Render, progress: &mut dyn FnMut(usize, usize)) -> Result<Value> {
    if !dir.join("index.html").is_file() {
        return Err(DevdError::not_found(format!("{} has no index.html; start one with motion.new", dir.display())));
    }
    let server = Server::start(dir)?;
    let url = format!("{}/index.html?render", server.base);
    let stills = !options.at.is_empty();
    let frames_dir = dir.join("out").join(if stills { "stills" } else { "frames" });

    let (width, height, fps, seconds, written, count) = crate::browser::with_capture(root, &url, |capture| {
        let info = capture.info()?;
        let (width, height) = (info["width"].as_u64().unwrap_or(1920) as u32, info["height"].as_u64().unwrap_or(1080) as u32);
        let fps = options.fps.unwrap_or(info["fps"].as_u64().unwrap_or(30) as u32).clamp(1, 120);
        let seconds = info["seconds"].as_f64().unwrap_or(5.0);
        let total = (seconds * f64::from(fps)).round() as usize;
        if total == 0 || total > MAX_FRAMES {
            return Err(DevdError::invalid(format!("{total} frames is outside 1..{MAX_FRAMES}; shorten the composition or lower the frame rate")));
        }
        // Frames from an earlier render must not leak into this film.
        if frames_dir.exists() {
            std::fs::remove_dir_all(&frames_dir)?;
        }
        std::fs::create_dir_all(&frames_dir)?;
        let times: Vec<f64> = if stills { options.at.iter().map(|t| t.clamp(0.0, seconds)).collect() } else { (0..total).map(|f| f as f64 / f64::from(fps)).collect() };
        capture.resize(width, height)?;
        let mut written = 0usize;
        for (index, t) in times.iter().enumerate() {
            let png = capture.frame(*t)?;
            let name = if stills { format!("still_{:07.3}s.png", t) } else { format!("frame_{:05}.png", index + 1) };
            std::fs::write(frames_dir.join(name), png)?;
            written += 1;
            progress(written, times.len());
        }
        Ok((width, height, fps, seconds, written, times.len()))
    })?;
    drop(server);

    if stills {
        return Ok(json!({ "state": "ok", "stills": count, "dir": frames_dir.display().to_string(), "width": width, "height": height }));
    }
    let mut result = json!({
        "state": "ok", "frames": written, "fps": fps, "seconds": seconds, "width": width, "height": height,
        "frames_dir": frames_dir.display().to_string(),
    });
    match find_ffmpeg() {
        None => {
            result["video"] = Value::Null;
            result["note"] = json!("The frames are rendered. There is no ffmpeg on this machine to encode them: install one, or set DEVD_FFMPEG to its path, and run the render again. Any video tool can also assemble frame_%05d.png at the frame rate above.");
        }
        Some(ffmpeg) => {
            let name = options.out.clone().unwrap_or_else(|| "out/film.mp4".to_string());
            let out = inside(dir, &name)?;
            if let Some(parent) = out.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let audio = options.audio.as_deref().map(|a| inside(dir, a)).transpose()?;
            if let Some(a) = &audio {
                if !a.is_file() {
                    return Err(DevdError::not_found(format!("the audio file {} does not exist", a.display())));
                }
            }
            let args = encode_args(&frames_dir, fps, audio.as_deref(), options.crf.unwrap_or(18).clamp(0, 51), &out);
            let run = Command::new(ffmpeg).args(&args).stdout(Stdio::null()).stderr(Stdio::piped()).output()?;
            if !run.status.success() {
                return Err(DevdError::internal(format!("ffmpeg could not encode the frames: {}", String::from_utf8_lossy(&run.stderr).trim())));
            }
            result["video"] = json!(out.display().to_string());
            result["bytes"] = json!(std::fs::metadata(&out).map(|m| m.len()).unwrap_or(0));
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_composition_is_a_page_and_its_runtime_and_never_overwrites() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("film");
        let made = scaffold(&dir, &Spec { title: "A <b>title</b> & more".into(), ..Spec::default() }).unwrap();
        assert_eq!(made["frames"], 180);
        let page = std::fs::read_to_string(dir.join("index.html")).unwrap();
        assert!(page.contains("A &lt;b&gt;title&lt;/b&gt; &amp; more"), "the title is text, not markup");
        assert!(page.contains("Motion.config({ width: 1920, height: 1080, fps: 30, seconds: 6 })"));
        assert!(std::fs::read_to_string(dir.join("devd-motion.js")).unwrap().contains("window.__motion"));
        assert!(!page.contains("__"), "every placeholder is filled");
        // Starting again over an existing film is refused, and nothing is changed.
        std::fs::write(dir.join("index.html"), "mine").unwrap();
        assert!(scaffold(&dir, &Spec::default()).unwrap_err().to_string().contains("already exists"));
        assert_eq!(std::fs::read_to_string(dir.join("index.html")).unwrap(), "mine");
    }

    #[test]
    fn a_film_cannot_be_started_at_a_size_or_length_that_is_a_mistake() {
        let d = tempfile::tempdir().unwrap();
        for spec in [
            Spec { width: 10, ..Spec::default() },
            Spec { fps: 0, ..Spec::default() },
            Spec { seconds: 0.0, ..Spec::default() },
            Spec { seconds: 10_000.0, ..Spec::default() },
        ] {
            assert!(scaffold(&d.path().join("x"), &spec).is_err(), "{spec:?}");
        }
    }

    #[test]
    fn the_file_server_gives_out_the_folder_and_nothing_beside_it() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().join("film");
        std::fs::create_dir_all(root.join("assets")).unwrap();
        std::fs::write(root.join("index.html"), "<p>hi</p>").unwrap();
        std::fs::write(root.join("assets/a.png"), [1, 2, 3]).unwrap();
        std::fs::write(d.path().join("secret.txt"), "no").unwrap();
        assert!(resolve(&root, "/").unwrap().ends_with("index.html"));
        assert!(resolve(&root, "/assets/a.png?x=1").is_some());
        for bad in ["/../secret.txt", "/%2e%2e/secret.txt", "/assets/../../secret.txt", "/..%2fsecret.txt", "/assets\\..\\..\\secret.txt", "/C:/Windows/win.ini", "//etc/passwd", "/missing.png", "/assets"] {
            assert!(resolve(&root, bad).is_none(), "{bad} escaped the folder");
        }
    }

    #[test]
    fn the_server_answers_a_browser_over_a_real_socket() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("index.html"), "<p>hello</p>").unwrap();
        std::fs::write(d.path().join("devd-motion.js"), "x").unwrap();
        let server = Server::start(d.path()).unwrap();
        let get = |path: &str, method: &str| {
            let addr = server.base.trim_start_matches("http://").to_string();
            let mut s = TcpStream::connect(&addr).unwrap();
            write!(s, "{method} {path} HTTP/1.1\r\nHost: {addr}\r\n\r\n").unwrap();
            let mut out = String::new();
            s.read_to_string(&mut out).unwrap();
            out
        };
        let page = get("/index.html", "GET");
        assert!(page.starts_with("HTTP/1.1 200") && page.contains("text/html") && page.ends_with("<p>hello</p>"), "{page}");
        assert!(get("/../Cargo.toml", "GET").starts_with("HTTP/1.1 404"));
        assert!(get("/index.html", "POST").starts_with("HTTP/1.1 405"));
        assert!(server.base.starts_with("http://127.0.0.1:"), "loopback only");
    }

    #[test]
    fn the_encoder_is_asked_for_a_player_friendly_video_with_sound_when_there_is_sound() {
        let args = encode_args(Path::new("/f"), 30, None, 18, Path::new("/o.mp4"));
        let line = args.join(" ");
        assert!(line.contains("-framerate 30") && line.contains("frame_%05d.png") && line.contains("-pix_fmt yuv420p") && line.contains("+faststart"));
        assert!(!line.contains("aac"));
        let with = encode_args(Path::new("/f"), 24, Some(Path::new("/a.wav")), 16, Path::new("/o.mp4")).join(" ");
        assert!(with.contains("-i /a.wav") && with.contains("-c:a aac") && with.contains("-shortest") && with.contains("-crf 16"));
    }

    #[test]
    fn an_output_path_cannot_leave_the_composition_folder() {
        let dir = Path::new("/film");
        assert!(inside(dir, "out/film.mp4").is_ok());
        for bad in ["../x.mp4", "/abs.mp4", "out/../../x.mp4", "C:\\x.mp4"] {
            assert!(inside(dir, bad).is_err(), "{bad}");
        }
    }
}

#[cfg(test)]
mod render_tests {
    use super::*;

    /// A real render through devd's own browser. Skipped where no Chromium-based browser is installed.
    #[test]
    fn a_composition_renders_the_same_frames_every_time() {
        if !crate::browser::available() {
            eprintln!("skipped: no Chromium-based browser on this machine");
            return;
        }
        let d = tempfile::tempdir().unwrap();
        let home = d.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        let dir = d.path().join("film");
        scaffold(&dir, &Spec { title: "Same every time".into(), width: 640, height: 360, fps: 10, seconds: 1.0 }).unwrap();
        let frames = |out: &Value| {
            let dir = PathBuf::from(out["frames_dir"].as_str().unwrap());
            let mut names: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().path()).collect();
            names.sort();
            names.iter().map(|p| std::fs::read(p).unwrap()).collect::<Vec<_>>()
        };
        let mut progress = Vec::new();
        let first = match render(&home, &dir, &Render::default(), &mut |done, total| progress.push((done, total))) {
            Ok(first) => first,
            // A browser that is installed but cannot start here (a build machine without a usable sandbox, say) is the
            // machine, not the renderer; the reason is printed so it is not hidden.
            Err(e) if e.to_string().contains("the browser stopped before it was ready") || e.to_string().contains("the browser did not start") => {
                eprintln!("skipped: {e}");
                return;
            }
            Err(e) => panic!("{e}"),
        };
        assert_eq!((first["frames"].as_u64(), first["width"].as_u64(), first["height"].as_u64()), (Some(10), Some(640), Some(360)));
        assert_eq!(progress.last(), Some(&(10, 10)));
        let one = frames(&first);
        assert_eq!(one.len(), 10);
        assert!(one[0].starts_with(&[0x89, b'P', b'N', b'G']), "a PNG");
        assert_ne!(one[0], one[9], "the film changes over time");
        // A second render of the same composition is byte-for-byte the first.
        let second = render(&home, &dir, &Render::default(), &mut |_, _| {}).unwrap();
        assert_eq!(one, frames(&second), "a render is deterministic");
        // Stills of chosen moments, without the whole film.
        let still = render(&home, &dir, &Render { at: vec![0.0, 0.5, 99.0], ..Render::default() }, &mut |_, _| {}).unwrap();
        assert_eq!(still["stills"], 3, "a time past the end is clamped to the last frame");
        // A folder with no composition says what to do.
        let none = render(&home, d.path(), &Render::default(), &mut |_, _| {}).unwrap_err().to_string();
        assert!(none.contains("motion.new"), "{none}");
        let _ = crate::browser::close();
    }
}
