//! The browser the assistant and the person share.
//!
//! The assistant can open a page, read it, click and type in it, and take a picture of it. This is where the
//! person sees the same tab: which tabs are open, what the page looks like now, and the means to take over: an
//! address, back, forward and reload, a click on the page, the wheel to scroll it, the keyboard to type into it.
//! Nothing here is a second browser: every control is one of the `browser.*` capabilities the assistant uses, so a
//! tab the assistant opened appears here and a page the person loads is the page the assistant reads next.
//!
//! The browser is started only when it is asked for. Looking at this never launches a Chromium.

use std::cell::Cell;

use std::rc::Rc;
use std::time::{Duration, Instant};

use devd_app_core::surface;
use gpui::{
    AnyElement, Bounds, ClickEvent, Context, Entity, FocusHandle, KeyDownEvent, MouseButton,
    MouseDownEvent, ObjectFit, Pixels, ScrollWheelEvent, Subscription, canvas, div, img,
    prelude::*, px,
};
use serde_json::{Value, json};

use super::pages::{Row, list_or, read_call, text};
use crate::shell::Shell;
use crate::state::devd;
use crate::theme::{Tone, mono, size, theme};
use crate::ui::input::{InputEvent, TextInput};
use crate::ui::{self, Button, ButtonKind, favicon};

/// What the page remembers between draws.
#[derive(Default)]
pub struct BrowserUi {
    /// The address being typed.
    address: Option<Entity<TextInput>>,
    /// The address last copied into the box from the page, so a page that moved on by itself is followed
    /// and one the person is typing over is left alone.
    address_for: String,
    /// A picture is wanted as soon as nothing else is running.
    want_shot: bool,
    /// When a scroll was last sent, so a spinning wheel is a few scrolls and not hundreds.
    scrolled: Option<Instant>,
    /// Where the picture is on screen, so a click on it can be turned into a point on the page.
    bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    /// Where keys go while the page is being typed into.
    focus: Option<FocusHandle>,
    /// Whether the console is open.
    console: bool,
    /// Whether the page's structure is open.
    outline: bool,
    /// The screen's scale where the page is drawn, so the page is laid out for it.
    scale: Rc<Cell<f32>>,
    /// The page as it is now, and its size in CSS pixels.
    live: Option<std::sync::Arc<gpui::RenderImage>>,
    live_size: (f32, f32),
    /// The tab the picture is of. A frame of a tab that was closed or switched away from is never shown as the
    /// tab that is open now.
    live_tab: String,
    /// What keeps the picture moving while the pane is open.
    frames: Option<gpui::Task<()>>,
    /// The pane size last given to the page: width, height and the scale in hundredths.
    sized: Option<(u32, u32, u32)>,
    /// When the pointer was last sent, so moving it is a few events a second, not one per pixel.
    hovered: Option<Instant>,
    /// Wheel turns not yet sent.
    wheel: f64,
    _watch: Option<Subscription>,
}

impl BrowserUi {
    /// Where the person is looking, for the assistant: the address of the tab on screen.
    pub fn looking_at(&self) -> Option<String> {
        (!self.address_for.is_empty()).then(|| self.address_for.clone())
    }
}

/// What the browser is made of, so the pane can arrange it: a chip for each tab, a button for another, and the body.
pub struct Parts {
    /// One chip per tab.
    pub tabs: Vec<AnyElement>,
    /// Opens another tab.
    pub new: AnyElement,
    /// The address bar and the page, or what to do when nothing is open.
    pub body: AnyElement,
}

/// Draw the browser, `wide` pixels across.
pub fn parts(shell: &mut Shell, wide: f32, cx: &mut Context<Shell>) -> Parts {
    let t = theme(cx).clone();
    devd(cx).browser_seen.set(true);
    if shell.browser_ui.address.is_none() {
        let address = cx.new(|cx| TextInput::new("Type a URL", cx));
        let watch = cx.subscribe(&address, |this, input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Submitted) {
                let address = input.read(cx).text().trim().to_string();
                this.browser_go(&address, cx);
            }
        });
        shell.browser_ui.address = Some(address);
        shell.browser_ui._watch = Some(watch);
    }
    if shell.browser_ui.focus.is_none() {
        shell.browser_ui.focus = Some(cx.focus_handle());
    }

    let status = read_call(cx, &surface::browser::status());
    let data = status.data.as_deref().cloned().unwrap_or(Value::Null);
    let running = data["running"] == json!(true);
    let tabs: Vec<Value> = data["tabs"].as_array().cloned().unwrap_or_default();
    let focused = tabs
        .iter()
        .find(|tab| tab["focused"] == json!(true))
        .or(tabs.first())
        .cloned()
        .unwrap_or(Value::Null);
    let (tab_id, url, title) = (
        text(&focused, "id"),
        text(&focused, "url"),
        text(&focused, "title"),
    );

    // Follow the page: its address goes in the box, and a page that changed gets a new picture.
    if let Some(address) = shell.browser_ui.address.clone() {
        if running && url != shell.browser_ui.address_for {
            shell.browser_ui.address_for = url.clone();
            let shown = if url == "about:blank" {
                String::new()
            } else {
                url.clone()
            };
            address.update(cx, |i, cx| i.set_text(shown, cx));
        }
        if !running {
            shell.browser_ui.address_for.clear();
        }
    }
    let _ = (&tab_id, &title);
    // The page is drawn live while the pane is open; a screenshot is taken only when asked for, as evidence.
    keep_live(shell, running, cx);
    if running && shell.browser_ui.want_shot && devd(cx).sync.pending_mutations() == 0 {
        shell.browser_ui.want_shot = false;
        crate::state::start_write(cx, surface::browser::screenshot().mutation(), None, None);
    }

    // The chips: one for each tab, or the one a browser that is not running will open.
    let mut chips = Vec::new();
    for (n, tab) in tabs.iter().enumerate() {
        let (id, url) = (text(tab, "id"), text(tab, "url"));
        let name = {
            let title = text(tab, "title");
            if !title.is_empty() {
                title
            } else if url.is_empty() || url == "about:blank" {
                "New tab".to_string()
            } else {
                url.clone()
            }
        };
        let (focus_id, close_id) = (id.clone(), id.clone());
        let host = favicon::host_of(&url).unwrap_or_default();
        let lead = favicon::tile(&host, cx).into_any_element();
        chips.push(super::pane::chip(
            ui::nid("browser.tab", n),
            name,
            Some(lead),
            tab["focused"] == json!(true),
            move |_this, _, cx| {
                crate::state::start_write(
                    cx,
                    surface::browser::tab_focus(&focus_id).mutation(),
                    None,
                    None,
                );
            },
            move |_this, _, cx| {
                crate::state::start_write(
                    cx,
                    surface::browser::tab_close(&close_id).mutation(),
                    None,
                    None,
                );
            },
            cx,
        ));
    }
    if !running {
        chips.push(super::pane::chip(
            ui::nid("browser.tab", 0),
            "New tab".to_string(),
            None,
            true,
            |_, _, _| {},
            |_, _, _| {},
            cx,
        ));
    }
    let new = super::pane::icon_button(
        "browser.tab.new",
        "plus",
        cx.listener(move |_this, _: &ClickEvent, _, cx| {
            let running = devd(cx)
                .sync
                .peek(&devd_app_core::sync::QueryKey::bare("browser.status"))
                .and_then(|s| s.data.as_deref().map(|d| d["running"] == json!(true)))
                .unwrap_or(false);
            crate::state::start_write(
                cx,
                if running {
                    surface::browser::tab_open(None)
                } else {
                    surface::browser::open(None)
                }
                .mutation(),
                None,
                None,
            );
        }),
        &t,
    );

    let toolbar = toolbar(shell, running, cx);
    let content: AnyElement = if status.is_first_load() {
        div()
            .p(px(16.0))
            .child(ui::skeleton("browser.sk", 200.0, cx))
            .into_any_element()
    } else if !running || tabs.is_empty() {
        // No browser, or every tab closed: the start screen, not a page that will never arrive.
        if let Some(old) = shell.browser_ui.live.take() {
            cx.drop_image(old, None);
        }
        shell.browser_ui.live_tab.clear();
        empty(cx)
    } else {
        // The page takes the room; the console and what the assistant sees fold under it.
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(picture(shell, &tab_id, cx))
            .child(panels(shell, cx))
            .into_any_element()
    };
    let _ = wide;
    Parts {
        tabs: chips,
        new,
        body: div()
            .size_full()
            .flex()
            .flex_col()
            .child(toolbar)
            .child(content)
            .into_any_element(),
    }
}

/// Nothing is open yet.
fn empty(cx: &mut Context<Shell>) -> AnyElement {
    let t = theme(cx).clone();
    div()
        .flex()
        .flex_col()
        .items_center()
        .gap(px(8.0))
        .px(px(32.0))
        .pt(px(90.0))
        .child(ui::icon("globe", 24.0, t.faint))
        .child(div().text_size(px(size::TEXT)).font_weight(gpui::FontWeight::MEDIUM).child("Browse with the assistant"))
        .child(div().max_w(px(300.0)).text_center().text_size(px(size::TEXT_SM)).text_color(t.muted).child("Type a URL, or ask the assistant to open a site. It can read the page, click, and type, and you see what it sees."))
        .into_any_element()
}

/// Back, forward, reload and the address, in one row.
fn toolbar(shell: &Shell, running: bool, cx: &mut Context<Shell>) -> AnyElement {
    let t = theme(cx).clone();
    let nav =
        |id: &'static str, icon: &'static str, which: &'static str, cx: &mut Context<Shell>| {
            super::pane::icon_button(
                id,
                icon,
                cx.listener(move |_this, _: &ClickEvent, _, cx| {
                    if devd(cx)
                        .sync
                        .peek(&devd_app_core::sync::QueryKey::bare("browser.status"))
                        .and_then(|s| s.data.as_deref().map(|d| d["running"] == json!(true)))
                        .unwrap_or(false)
                    {
                        crate::state::start_write(
                            cx,
                            surface::browser::history(which).mutation(),
                            None,
                            None,
                        );
                    }
                }),
                &t,
            )
        };
    let address = shell.browser_ui.address.clone();
    div()
        .flex()
        .items_center()
        .gap(px(2.0))
        .px(px(10.0))
        .h(px(44.0))
        .flex_none()
        .border_b_1()
        .border_color(t.line)
        .child(nav("browser.back", "arrow-left", "back", cx))
        .child(nav("browser.forward", "arrow-right", "forward", cx))
        .child(nav("browser.reload", "refresh", "reload", cx))
        .children(address.map(|a| {
            div()
                .flex_1()
                .min_w_0()
                .mx(px(6.0))
                .px(px(12.0))
                .py(px(6.0))
                .rounded(px(size::RADIUS))
                .bg(t.hover)
                .child(a)
        }))
        .children(running.then(|| {
            super::pane::icon_button(
                "browser.shot",
                "camera",
                cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.browser_ui.want_shot = true;
                    cx.notify();
                }),
                &t,
            )
        }))
        .into_any_element()
}

/// The page, live. It fills the pane, is laid out at the pane's own size (so it is read at its real size, not a
/// desktop page shrunk into a column), and is redrawn many times a second while the pane is open. A click on it is
/// a click on the page at the same point, the pointer moving over it is the pointer moving over the page, the wheel
/// scrolls it as far as the wheel turned, and keys typed on it go to it.
fn picture(shell: &Shell, tab: &str, cx: &mut Context<Shell>) -> AnyElement {
    let t = theme(cx).clone();
    let bounds = shell.browser_ui.bounds.clone();
    let scale = shell.browser_ui.scale.clone();
    // Only a picture of the tab that is open: after a tab closes or another is chosen, the old page is not left
    // standing while the new one loads.
    let live = shell
        .browser_ui
        .live
        .clone()
        .filter(|_| !tab.is_empty() && shell.browser_ui.live_tab == tab);
    let (page_w, page_h) = shell.browser_ui.live_size;
    let (click_bounds, move_bounds, wheel_bounds) = (bounds.clone(), bounds.clone(), bounds.clone());
    let focus = shell.browser_ui.focus.clone();
    let click_focus = focus.clone();
    // A point in the pane, as a point in the page. The page is laid out at the pane's size, so this is the same
    // point; while a resize is on its way the frame on screen is still the old size, and it is scaled to it.
    let at = move |bounds: &Rc<Cell<Option<Bounds<Pixels>>>>,
                   x: Pixels,
                   y: Pixels|
          -> Option<(f64, f64)> {
        let b = bounds.get()?;
        let (w, h) = (f64::from(b.size.width), f64::from(b.size.height));
        if w <= 0.0 || h <= 0.0 {
            return None;
        }
        let (px_x, px_y) = (f64::from(x - b.origin.x), f64::from(y - b.origin.y));
        let (pw, ph) = if page_w > 0.0 && page_h > 0.0 {
            (page_w as f64, page_h as f64)
        } else {
            (w, h)
        };
        ((0.0..=w).contains(&px_x) && (0.0..=h).contains(&px_y)).then(|| (px_x / w * pw, px_y / h * ph))
    };
    let mut view = div()
        .id("browser.picture")
        .key_context("Terminal")
        .relative()
        .w_full()
        .flex_1()
        .min_h(px(200.0))
        .bg(gpui::rgb(0xffffff))
        .overflow_hidden()
        .cursor_pointer();
    if let Some(f) = &focus {
        view = view.track_focus(f);
    }
    let shown: AnyElement = match live {
        Some(frame) => img(gpui::ImageSource::Render(frame))
            .size_full()
            .object_fit(ObjectFit::Fill)
            .into_any_element(),
        None => div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(t.bg)
            .text_size(px(size::TEXT_SM))
            .text_color(t.muted)
            .child("Loading the page…")
            .into_any_element(),
    };
    view.child(shown)
        .child(
            canvas(
                move |b, window, _| {
                    bounds.set(Some(b));
                    scale.set(window.scale_factor());
                },
                |_, _, _, _| {},
            )
            // Pinned to the corner: an absolute box with no inset sits where it would have flowed, below the page,
            // and every point in the page then measured as outside it.
            .absolute()
            .top_0()
            .left_0()
            .size_full(),
        )
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |_this, ev: &MouseDownEvent, window, cx| {
                if let Some(f) = &click_focus {
                    window.focus(f);
                }
                if let Some((x, y)) = at(&click_bounds, ev.position.x, ev.position.y) {
                    crate::state::send_call(cx, "browser.mouse", json!({ "x": x, "y": y }));
                    cx.notify();
                }
            }),
        )
        .on_mouse_move(cx.listener(move |this, ev: &gpui::MouseMoveEvent, _, cx| {
            // Hover answers within a frame or two; the pointer is sent at most every 50ms, not every pixel.
            let due = this
                .browser_ui
                .hovered
                .is_none_or(|last| last.elapsed() > Duration::from_millis(50));
            if due && let Some((x, y)) = at(&move_bounds, ev.position.x, ev.position.y) {
                this.browser_ui.hovered = Some(Instant::now());
                crate::state::send_call(cx, "browser.hover", json!({ "x": x, "y": y }));
            }
        }))
        .on_scroll_wheel(cx.listener(move |this, ev: &ScrollWheelEvent, window, cx| {
            // As far as the wheel turned, at the pointer, like a browser: the turns that arrive within 40ms are one
            // scroll, so a fast wheel or a touchpad is a few scrolls and not hundreds, and none of it is lost.
            let step = -f64::from(ev.delta.pixel_delta(window.line_height()).y);
            this.browser_ui.wheel += step;
            let due = this
                .browser_ui
                .scrolled
                .is_none_or(|last| last.elapsed() > Duration::from_millis(40));
            if due
                && this.browser_ui.wheel != 0.0
                && let Some((x, y)) = at(&wheel_bounds, ev.position.x, ev.position.y)
            {
                let delta = this.browser_ui.wheel;
                this.browser_ui.wheel = 0.0;
                this.browser_ui.scrolled = Some(Instant::now());
                crate::state::send_call(cx, "browser.scroll", json!({ "x": x, "y": y, "delta_y": delta }));
            }
            cx.stop_propagation();
        }))
        .on_key_down(cx.listener(move |_this, ev: &KeyDownEvent, _, cx| {
            let k = &ev.keystroke;
            let named = match k.key.as_str() {
                "enter" => Some("Enter"),
                "tab" => Some("Tab"),
                "backspace" => Some("Backspace"),
                "escape" => Some("Escape"),
                "up" => Some("ArrowUp"),
                "down" => Some("ArrowDown"),
                "left" => Some("ArrowLeft"),
                "right" => Some("ArrowRight"),
                "delete" => Some("Delete"),
                "home" => Some("Home"),
                "end" => Some("End"),
                "pageup" => Some("PageUp"),
                "pagedown" => Some("PageDown"),
                _ => None,
            };
            let call = if k.modifiers.control && k.key == "v" {
                cx.read_from_clipboard()
                    .and_then(|c| c.text())
                    .map(|words| ("browser.key", json!({ "text": words })))
            } else if let Some(name) = named {
                Some(("browser.key", json!({ "key": name })))
            } else if k.key == "space" {
                Some(("browser.key", json!({ "text": " " })))
            } else if !k.modifiers.control && !k.modifiers.alt {
                k.key_char
                    .as_deref()
                    .filter(|c| !c.is_empty())
                    .map(|c| ("browser.key", json!({ "text": c })))
            } else {
                None
            };
            if let Some((capability, args)) = call {
                crate::state::send_call(cx, capability, args);
                cx.stop_propagation();
            }
        }))
        .into_any_element()
}

/// Keep the page laid out at the pane's size and its picture moving, while the pane is open and the browser runs.
///
/// A frame is asked for, decoded off the window's thread, and drawn; then the next, about eight a second. The frame
/// before is taken off the GPU as the next replaces it, so a pane left open all day holds two pictures, not
/// thousands. When the pane closes the loop ends and pages get the browser's own window back.
fn keep_live(shell: &mut Shell, running: bool, cx: &mut Context<Shell>) {
    if !running {
        return;
    }
    // The pane's size, once it has been drawn, as the page's size.
    if let Some(b) = shell.browser_ui.bounds.get() {
        let scale = shell.browser_ui.scale.get().max(1.0);
        let size = (
            f32::from(b.size.width).round() as u32,
            f32::from(b.size.height).round() as u32,
            (scale * 100.0).round() as u32,
        );
        if size.0 >= 200 && size.1 >= 150 && shell.browser_ui.sized != Some(size) {
            shell.browser_ui.sized = Some(size);
            crate::state::send_call(
                cx,
                "browser.viewport",
                json!({ "width": size.0, "height": size.1, "scale": size.2 as f64 / 100.0 }),
            );
        }
    }
    if shell.browser_ui.frames.is_some() {
        return;
    }
    let conn = devd(cx).conn.clone();
    shell.browser_ui.frames = Some(cx.spawn(async move |this, cx| {
        loop {
            let conn = conn.clone();
            let got = cx
                .background_spawn(async move { next_frame(&*conn) })
                .await;
            let go_on = this
                .update(cx, |this, cx| {
                    if !this.pane.browser {
                        // The pane closed: stop drawing, and give pages their own window back.
                        this.browser_ui.frames = None;
                        this.browser_ui.sized = None;
                        crate::state::send_call(cx, "browser.viewport", json!({}));
                        if let Some(old) = this.browser_ui.live.take() {
                            cx.drop_image(old, None);
                        }
                        return false;
                    }
                    if let Some((image, width, height, tab)) = got {
                        if let Some(old) = this.browser_ui.live.replace(image) {
                            cx.drop_image(old, None);
                        }
                        this.browser_ui.live_size = (width, height);
                        this.browser_ui.live_tab = tab;
                        cx.notify();
                    }
                    true
                })
                .unwrap_or(false);
            if !go_on {
                break;
            }
            cx.background_executor()
                .timer(Duration::from_millis(120))
                .await;
        }
    }));
}

/// Ask devd for a frame and decode it into a picture the GPU draws: its pixels, and the page's size in CSS pixels.
fn next_frame(
    backend: &dyn devd_app_core::sync::Backend,
) -> Option<(std::sync::Arc<gpui::RenderImage>, f32, f32, String)> {
    let reply = backend.invoke("browser.frame", &json!({})).ok()?;
    let value = match reply {
        devd_app_core::sync::Reply::Done(value) => value,
        _ => return None,
    };
    let path = value["path"].as_str()?;
    let bytes = std::fs::read(path).ok()?;
    let mut rgba = image::load_from_memory_with_format(&bytes, image::ImageFormat::Jpeg)
        .ok()?
        .into_rgba8();
    // The GPU takes its pixels blue-first, as gpui's own loader gives them.
    for pixel in rgba.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
    }
    let (px_w, px_h) = (rgba.width() as f32, rgba.height() as f32);
    let width = value["width"].as_f64().filter(|w| *w > 0.0).map_or(px_w, |w| w as f32);
    let height = value["height"].as_f64().filter(|h| *h > 0.0).map_or(px_h, |h| h as f32);
    let frame = image::Frame::new(rgba);
    let tab = value["tab"].as_str().unwrap_or_default().to_string();
    Some((std::sync::Arc::new(gpui::RenderImage::new([frame])), width, height, tab))
}

/// The console and the page as the assistant reads it, below the page.
fn panels(shell: &Shell, cx: &mut Context<Shell>) -> AnyElement {
    let t = theme(cx).clone();
    let toggle =
        |id: &'static str, label: &'static str, on: bool, which: u8, cx: &mut Context<Shell>| {
            Button::new(id, label)
                .icon(if on { "chevron-down" } else { "chevron-right" })
                .kind(ButtonKind::Ghost)
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    if which == 0 {
                        this.browser_ui.console = !this.browser_ui.console;
                    } else {
                        this.browser_ui.outline = !this.browser_ui.outline;
                    }
                    cx.notify();
                }))
        };
    let mut column = div().flex().flex_col().gap(px(8.0)).p(px(10.0)).child(
        div()
            .flex()
            .items_center()
            .gap(px(4.0))
            .child(toggle(
                "browser.console",
                "Console",
                shell.browser_ui.console,
                0,
                cx,
            ))
            .child(toggle(
                "browser.outline",
                "What the assistant sees",
                shell.browser_ui.outline,
                1,
                cx,
            )),
    );
    if shell.browser_ui.console {
        let log = read_call(cx, &surface::browser::console());
        let mut rows = Vec::new();
        for entry in log.list("/entries") {
            let level = text(entry, "level");
            rows.push(Row {
                title: text(entry, "text"),
                state: Some((
                    match level.as_str() {
                        "error" => Tone::Danger,
                        "warn" => Tone::Warn,
                        _ => Tone::Neutral,
                    },
                    level,
                )),
                ..Row::default()
            });
        }
        for call in log.list("/network") {
            let status = call["status"].as_u64().unwrap_or(0);
            rows.push(Row {
                title: format!("{} {}", text(call, "method"), text(call, "url")),
                sub: call["error"]
                    .as_str()
                    .map(str::to_string)
                    .unwrap_or_else(|| format!("{} ms", call["ms"].as_u64().unwrap_or(0))),
                state: Some(if status == 0 || status >= 400 {
                    (
                        Tone::Danger,
                        if status == 0 {
                            "failed".to_string()
                        } else {
                            status.to_string()
                        },
                    )
                } else {
                    (Tone::Ok, status.to_string())
                }),
                ..Row::default()
            });
        }
        column = column.child(list_or(
            &log,
            rows,
            "browser.log",
            "terminal",
            "Nothing logged",
            "What the page prints, and every request it makes, appears here.",
            cx,
        ));
    }
    if shell.browser_ui.outline {
        let tree = read_call(cx, &surface::browser::snapshot());
        let mut lines = div()
            .flex()
            .flex_col()
            .gap(px(2.0))
            .p(px(12.0))
            .font_family(mono())
            .text_size(px(size::TEXT_SM));
        let nodes = tree.list("/nodes");
        for node in nodes {
            let reference = node["ref"]
                .as_str()
                .map(|r| format!("[{r}] "))
                .unwrap_or_default();
            lines = lines.child(
                div()
                    .text_color(if reference.is_empty() {
                        t.muted
                    } else {
                        t.text
                    })
                    .child(format!(
                        "{reference}{} {}",
                        text(node, "role"),
                        text(node, "name")
                    )),
            );
        }
        if nodes.is_empty() {
            lines = lines.child(div().text_color(t.muted).child(if tree.is_first_load() {
                "Reading the page…"
            } else {
                "The page has nothing to read yet."
            }));
        }
        column = column.child(
            div()
                .max_h(px(280.0))
                .overflow_hidden()
                .rounded(px(size::RADIUS))
                .border_1()
                .border_color(t.line)
                .child(lines),
        );
    }
    column.into_any_element()
}

impl Shell {
    /// Load an address: in the open browser, or start one on it.
    pub fn browser_go(&mut self, address: &str, cx: &mut Context<Self>) {
        let address = address.trim();
        let call = if address.is_empty() {
            surface::browser::open(None)
        } else {
            let url = if address.contains("://") || address.starts_with("about:") {
                address.to_string()
            } else if address.contains(' ')
                || (!address.contains('.') && !address.starts_with("localhost"))
            {
                format!("https://duckduckgo.com/?q={}", address.replace(' ', "+"))
            } else if address.starts_with("localhost") || address.starts_with("127.") {
                format!("http://{address}")
            } else {
                format!("https://{address}")
            };
            // The first load starts the browser; later ones navigate the tab on screen, and with every tab closed a
            // new tab opens on the address instead of a navigation that has no tab to go to.
            let status = devd(cx)
                .sync
                .peek(&devd_app_core::sync::QueryKey::bare("browser.status"))
                .and_then(|s| s.data.as_deref().cloned());
            let running = status.as_ref().is_some_and(|d| d["running"] == json!(true));
            let has_tab = status
                .as_ref()
                .and_then(|d| d["tabs"].as_array().map(|t| !t.is_empty()))
                .unwrap_or(false);
            if running && has_tab {
                surface::browser::navigate(&url)
            } else if running {
                surface::browser::tab_open(Some(&url))
            } else {
                surface::browser::open(Some(&url))
            }
        };
        crate::state::start_write(
            cx,
            call.mutation(),
            Some("Loading the page…".to_string()),
            None,
        );
        devd(cx).sync.invalidate("browser");
        cx.notify();
    }
}
