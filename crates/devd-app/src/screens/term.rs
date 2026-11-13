//! A shell inside devd, drawn in the window.
//!
//! The same view serves the project's Terminal tab and the assistant's side pane. A shell is a real terminal
//! devd keeps (`terminal.*`), so it survives leaving the page, the assistant can read what it shows, and only
//! the person can type into it. This draws its screen, takes its keys and lists the shells of one folder.

use devd_app_core::surface;
use gpui::{div, point, prelude::*, px, AnyElement, App, ClickEvent, ClipboardItem, Context, FocusHandle, KeyDownEvent, MouseButton, MouseDownEvent, ScrollHandle};
use serde_json::{json, Value};

use super::pages::read_call;
use crate::shell::Shell;
use crate::state::devd;
use crate::theme::{mono, size, theme};
use crate::ui::{self, card, Button, ButtonKind};

/// Where a shell is shown, which is which state it keeps.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Home {
    /// The Terminal tab of a project.
    Project,
    /// The assistant's side pane.
    Pane,
}

/// What a place showing shells remembers.
#[derive(Default)]
pub struct TermUi {
    /// The shell shown.
    pub id: Option<String>,
    /// How many shells the folder had when last drawn, so a new one is the one shown.
    pub known: usize,
    /// Where keys go while a shell is being typed into.
    pub focus: Option<FocusHandle>,
    /// What scrolls the shell, so it can stay at the bottom as it prints.
    pub scroll: ScrollHandle,
    /// How many lines the shell showed when last drawn.
    pub lines: usize,
    /// Whether a first shell was already opened for this place, so closing the last one does not open another.
    pub requested: bool,
    /// Frames still to be pinned to the bottom: the room a new line takes is only known once it is drawn.
    pub pin: u8,
}

impl Shell {
    /// The shell state of a place.
    pub fn term_ui(&mut self, home: Home) -> &mut TermUi {
        match home {
            Home::Project => &mut self.project_ui.term,
            Home::Pane => &mut self.pane.term,
        }
    }
}

/// Whether two paths are the same folder, however each was written.
pub fn same_folder(a: &str, b: &str) -> bool {
    let norm = |p: &str| p.replace('\\', "/").trim_start_matches("//?/").trim_end_matches('/').to_lowercase();
    norm(a) == norm(b)
}

/// What the shells of a folder are made of, so a place can arrange them: a chip for each, a button for another, one
/// for copying what the screen shows, and the screen itself.
pub struct Shells {
    /// One chip per shell.
    pub tabs: Vec<AnyElement>,
    /// Opens another shell.
    pub new: AnyElement,
    /// Copies what the shown shell has printed.
    pub copy: Option<AnyElement>,
    /// The shown shell's screen, or what to do when there is none.
    pub screen: AnyElement,
}

/// The shells of one folder, each a real terminal kept by devd: the same screen the assistant can read. `cols` is how
/// many characters fit across; `tall` is the screen's height, or `None` to fill the room it is given.
pub fn shells(shell: &mut Shell, home: Home, dir: &str, cols: u16, tall: Option<f32>, cx: &mut Context<Shell>) -> Shells {
    let t = theme(cx).clone();
    devd(cx).terminal_seen.set(true);
    let list = read_call(cx, &surface::terminal::list());
    let mine: Vec<Value> = list.list("/terminals").iter().filter(|s| same_folder(s["cwd"].as_str().unwrap_or_default(), dir)).cloned().collect();
    let open_dir = dir.to_string();
    let open = move |cx: &mut App| {
        crate::state::start_write(cx, surface::terminal::open(&open_dir, cols, 26).mutation(), None, None);
    };
    let new = {
        let open = open.clone();
        super::pane::icon_button("terminal.new", "plus", cx.listener(move |_this, _: &ClickEvent, _, cx| open(cx)), &t)
    };
    fn sized<E: Styled>(el: E, tall: Option<f32>) -> E {
        match tall {
            Some(h) => el.h(px(h)),
            None => el.flex_1().min_h_0(),
        }
    }

    if mine.is_empty() {
        shell.term_ui(home).known = 0;
        if list.is_first_load() {
            return Shells { tabs: vec![], new, copy: None, screen: sized(div().w_full(), tall).child(ui::skeleton("terminal.sk", 240.0, cx)).into_any_element() };
        }
        // Opening the panel opens a shell, as a terminal panel should; closing the last one leaves it closed.
        if !shell.term_ui(home).requested {
            shell.term_ui(home).requested = true;
            open(cx);
        }
        let start = Button::new("terminal.open", "Open a terminal").icon("terminal").kind(ButtonKind::Ghost).on_click(cx.listener(move |_this, _: &ClickEvent, _, cx| open(cx)));
        let screen = sized(div().w_full().flex().flex_col().items_center().justify_center().gap(px(10.0)), tall)
            .child(ui::icon("terminal", 22.0, t.faint))
            .child(div().text_size(px(size::TEXT)).text_color(t.muted).child("No terminal is open"))
            .child(start)
            .into_any_element();
        return Shells { tabs: vec![], new, copy: None, screen };
    }
    shell.term_ui(home).requested = true;

    // A shell just opened is the one to show; otherwise the one that was.
    if mine.len() != shell.term_ui(home).known {
        if mine.len() > shell.term_ui(home).known {
            shell.term_ui(home).id = mine.last().and_then(|s| s["id"].as_str()).map(str::to_string);
        }
        shell.term_ui(home).known = mine.len();
    }
    let chosen = shell.term_ui(home).id.clone().filter(|id| mine.iter().any(|s| s["id"].as_str() == Some(id.as_str()))).or_else(|| mine.first().and_then(|s| s["id"].as_str()).map(str::to_string)).unwrap_or_default();

    let screen = read_call(cx, &surface::terminal::read(&chosen, 1500));
    let body = screen.data.as_ref().map(|d| d["screen"].as_str().unwrap_or_default().to_string()).unwrap_or_default();
    let running = screen.data.as_ref().map(|d| d["running"] != json!(false)).unwrap_or(true);
    let exit = screen.data.as_ref().and_then(|d| d["exit_code"].as_i64());
    let lines: Vec<String> = devd_app_core::terminal::lines(&body).into_iter().map(str::to_string).collect();

    let focus = shell.term_ui(home).focus.get_or_insert_with(|| cx.focus_handle()).clone();
    let scroll = shell.term_ui(home).scroll.clone();
    if lines.len() != shell.term_ui(home).lines {
        shell.term_ui(home).lines = lines.len();
        shell.term_ui(home).pin = 4;
    }
    if shell.term_ui(home).pin > 0 {
        shell.term_ui(home).pin -= 1;
        let max = scroll.max_offset().height;
        scroll.set_offset(point(px(0.0), -max));
    }

    // One chip per shell, each with its own way to end it.
    let mut tabs = Vec::new();
    for (n, s) in mine.iter().enumerate() {
        let id = s["id"].as_str().unwrap_or_default().to_string();
        let (shown, ended) = (id.clone(), id.clone());
        tabs.push(super::pane::chip(
            ui::nid("terminal.tab", n),
            format!("Terminal {}", n + 1),
            None,
            id == chosen,
            move |this, window, cx| {
                this.term_ui(home).id = Some(shown.clone());
                if let Some(f) = &this.term_ui(home).focus {
                    window.focus(f);
                }
                cx.notify();
            },
            move |_this, _, cx| {
                crate::state::start_write(cx, surface::terminal::close(&ended).mutation(), None, None);
            },
            cx,
        ));
    }
    let copy_text = body.clone();
    let copy = super::pane::icon_button(
        "terminal.copy",
        "copy",
        cx.listener(move |_this, _: &ClickEvent, _, cx| {
            cx.write_to_clipboard(ClipboardItem::new_string(copy_text.clone()));
            crate::state::push_toast(cx, "Copied what the terminal shows.");
        }),
        &t,
    );

    // The screen: what the shell has printed, with a block where typing goes while it runs.
    let key_id = chosen.clone();
    let click_focus = focus.clone();
    let mut col = div().flex().flex_col().p(px(12.0)).font_family(mono()).text_size(px(size::TEXT_SM)).line_height(px(18.0)).text_color(t.text);
    let last = lines.len().saturating_sub(1);
    for (n, line) in lines.iter().enumerate() {
        let mut row = div().flex().items_center().min_h(px(18.0)).whitespace_nowrap().child(if line.is_empty() { " ".to_string() } else { line.clone() });
        if n == last && running {
            row = row.child(div().ml(px(1.0)).w(px(8.0)).h(px(15.0)).bg(t.accent));
        }
        col = col.child(row);
    }
    if lines.is_empty() && screen.is_first_load() {
        col = col.child(div().text_color(t.faint).child("Starting…"));
    }
    if !running {
        col = col.child(div().pt(px(8.0)).text_color(t.faint).child(format!("The shell ended{}.", exit.map(|c| format!(" with exit code {c}")).unwrap_or_default())));
    }
    let view = sized(div().id("terminal.screen").w_full(), tall)
        .key_context("Terminal")
        .track_focus(&focus)
        .overflow_y_scroll()
        .track_scroll(&scroll)
        .cursor_text()
        .on_mouse_down(MouseButton::Left, cx.listener(move |_this, _: &MouseDownEvent, window, _| window.focus(&click_focus)))
        .on_key_down(cx.listener(move |_this, ev: &KeyDownEvent, _, cx| {
            let k = &ev.keystroke;
            let paste = (k.modifiers.control && k.key == "v") || (k.modifiers.shift && k.key == "insert");
            let data = if paste {
                cx.read_from_clipboard().and_then(|c| c.text()).map(|text| devd_app_core::terminal::pasted(&text))
            } else {
                devd_app_core::terminal::bytes(devd_app_core::terminal::Press { key: &k.key, text: k.key_char.as_deref(), ctrl: k.modifiers.control, alt: k.modifiers.alt, shift: k.modifiers.shift })
            };
            if let Some(data) = data {
                crate::state::send_keys(cx, &key_id, data);
                cx.stop_propagation();
            }
        }))
        .child(col)
        .into_any_element();
    Shells { tabs, new, copy: Some(copy), screen: view }
}

/// The project's Terminal tab: the same shells as the assistant's pane, under the same kind of header.
pub fn view(shell: &mut Shell, home: Home, dir: &str, cols: u16, tall: f32, cx: &mut Context<Shell>) -> AnyElement {
    let t = theme(cx).clone();
    let parts = shells(shell, home, dir, cols, Some(tall), cx);
    let name = ui::leaf(dir).to_string();
    let ask_dir = dir.to_string();
    let ask = Button::new("terminal.ask", "Ask the assistant to run something").icon("assistant").kind(ButtonKind::Ghost).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.ask_about(format!("In the project {name} ({ask_dir}), please run: "), cx)));
    let header = super::pane::header("Terminal", parts.tabs, parts.new, parts.copy.into_iter().collect(), &t);
    div().flex().flex_col().gap(px(10.0)).child(card(cx).overflow_hidden().child(header).child(parts.screen)).child(div().flex().child(ask)).into_any_element()
}
