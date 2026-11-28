//! The message box: what you type, and what you can reach from it.
//!
//! `/` opens the skills devd can load ("/review", "/frontend"), `@` opens the
//! team ("@qa check the checkout flow"), `+` attaches files and folders, and the
//! row underneath says which model, how much effort and which mode the next
//! message goes out with. Every choice here is passed to `chat.send`; nothing is
//! drawn that does not do something.
//!
//! A skill or a member is written into the message as plain text. devd's
//! assistant reads `/name` as "load this skill first" and `@name` as "hand this
//! to that member" (its prompt says so), so the same words work typed by hand,
//! pasted, or sent by an agent over the API.

use std::path::{Path, PathBuf};

use devd_app_core::nav::Route;
use devd_app_core::sync::Mutation;
use gpui::{
    AnyElement, ClickEvent, Context, FontWeight, MouseButton, PathPromptOptions, deferred, div,
    prelude::*, px, relative,
};
use serde_json::{json, Value};

use crate::shell::Shell;
use crate::ui::input::TextInput;
use crate::state::{devd, q};
use crate::theme::{mono, size, theme};
use crate::ui::{self, Button, ButtonKind, icon};

/// Which popover is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Open {
    /// None.
    #[default]
    Nothing,
    /// The attach menu.
    Plus,
    /// Model and effort.
    Model,
    /// How much it may do without asking.
    Mode,
}

/// The modes `chat.send` accepts, with what each means.
pub const MODES: &[(&str, &str, &str)] = &[
    (
        "auto",
        "Auto",
        "Works on its own and asks before anything risky",
    ),
    ("manual", "Ask first", "Asks before every change it makes"),
    ("plan", "Plan", "Reads and plans; changes nothing"),
    (
        "bypass",
        "Bypass",
        "Never asks. Use it only in a copy you can throw away",
    ),
];

/// The efforts `chat.send` accepts.
pub const EFFORTS: &[(&str, &str)] = &[
    ("none", "Off"),
    ("low", "Low"),
    ("medium", "Medium"),
    ("high", "High"),
    ("extra", "Extra"),
    ("max", "Max"),
];

/// The effort level a conversation's stored choice names, as the composer holds it.
///
/// A conversation that never chose one has no level, and the chip then says what the default is
/// rather than what was last picked somewhere else.
pub fn level(effort: Option<&str>) -> Option<&'static str> {
    let effort = effort?;
    EFFORTS
        .iter()
        .find(|(id, _)| *id == effort)
        .map(|(id, _)| *id)
}

/// What the message box has chosen, beyond its text.
#[derive(Debug, Default)]
pub struct Compose {
    /// The popover that is open.
    pub open: Open,
    /// The mode chosen, or None to leave it as the conversation has it.
    pub mode: Option<&'static str>,
    /// The effort chosen, or None for the ordinary default.
    pub effort: Option<&'static str>,
    /// The conversation these choices belong to, so the effort can be read from it when it opens
    /// instead of showing whatever was last picked in this window.
    pub thread: Option<String>,
    /// The provider and model chosen, or None for the default one.
    pub model: Option<(String, String)>,
    /// Files and folders attached to the next message.
    pub files: Vec<PathBuf>,
    /// The highlighted row of the `/` or `@` menu.
    pub selected: usize,
    /// The popover a press outside it just closed, and when: the same press lands on the button that opened it.
    pub dismissed: Option<(Open, std::time::Instant)>,
    /// What scrolls the `/` and `@` menu, so every match can be reached and the highlighted one stays in view.
    pub list_scroll: gpui::ScrollHandle,
    /// The buttons have moved under the text. Once a message needs a second row it keeps the stacked layout until
    /// the box is emptied: the two layouts give the text different widths, and switching back and forth at the
    /// point where it just fits would make the box flicker as you type.
    pub stacked: std::cell::Cell<bool>,
}

impl Compose {
    /// What pressing a popover's button does: open it, or close it when it is open. A press that has just closed
    /// it from outside is that same press arriving at its button, and must not open it again.
    pub fn toggle(&mut self, which: Open) {
        let just_closed = self.dismissed.take().is_some_and(|(was, at)| {
            was == which && at.elapsed() < std::time::Duration::from_millis(300)
        });
        self.open = if self.open == which || just_closed {
            Open::Nothing
        } else {
            which
        };
    }
}

/// A popover that closes when the window is pressed anywhere outside it.
fn dismissable(menu: gpui::Div, which: Open, cx: &mut Context<Shell>) -> impl IntoElement {
    let name = format!("popover.{which:?}");
    let menu = menu
        .id(gpui::ElementId::Name(name.clone().into()))
        .on_mouse_down_out(cx.listener(move |this, _, _, cx| {
            let compose = this.compose_of(this.front_screen());
            if compose.open == which {
                compose.open = Open::Nothing;
                compose.dismissed = Some((which, std::time::Instant::now()));
                cx.notify();
            }
        }));
    // Fade, not rise: the menu is positioned from the box it belongs to, and moving it would move that.
    ui::motion::fade(
        menu,
        gpui::ElementId::Name(format!("{name}.enter").into()),
        ui::motion::FAST,
    )
}

/// One row of the `/` or `@` menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// What goes into the message: `/review`, `@qa`.
    pub insert: String,
    /// The name shown.
    pub title: String,
    /// What it is for.
    pub detail: String,
}

/// The word being typed, when it is a `/` or `@` something.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Trigger {
    /// `/` or `@`.
    pub mark: char,
    /// What follows the mark.
    pub query: String,
    /// Where the word starts in the text.
    pub start: usize,
}

/// The `/` or `@` word at the end of the text, if that is what is being typed.
pub fn trigger(text: &str) -> Option<Trigger> {
    let start = text
        .char_indices()
        .rev()
        .find(|(_, c)| c.is_whitespace())
        .map(|(i, c)| i + c.len_utf8())
        .unwrap_or(0);
    let word = &text[start..];
    let mark = word.chars().next().filter(|c| *c == '/' || *c == '@')?;
    let query = &word[1..];
    // A path ("/usr/bin") or an address is not a skill.
    if query
        .chars()
        .any(|c| !(c.is_alphanumeric() || matches!(c, '-' | '_' | ':' | '.')))
    {
        return None;
    }
    Some(Trigger {
        mark,
        query: query.to_string(),
        start,
    })
}

/// The entries that fit what has been typed: names that begin with it first, then names
/// that contain it, then descriptions that do.
pub fn matching(entries: &[Entry], query: &str) -> Vec<Entry> {
    let q = query.to_lowercase();
    let name = |e: &Entry| e.insert[1..].to_lowercase();
    let mut out: Vec<Entry> = entries
        .iter()
        .filter(|e| name(e).starts_with(&q))
        .cloned()
        .collect();
    out.extend(
        entries
            .iter()
            .filter(|e| !name(e).starts_with(&q) && name(e).contains(&q))
            .cloned(),
    );
    out.extend(
        entries
            .iter()
            .filter(|e| {
                !name(e).contains(&q)
                    && (e.title.to_lowercase().contains(&q) || e.detail.to_lowercase().contains(&q))
            })
            .cloned(),
    );
    out
}

/// The text with the word being typed replaced by the chosen entry.
pub fn accept(text: &str, trigger: &Trigger, insert: &str) -> String {
    format!("{}{insert} ", &text[..trigger.start])
}

/// The message as it is sent: what was typed, and the files that go with it.
pub fn with_files(message: &str, files: &[PathBuf]) -> String {
    if files.is_empty() {
        return message.to_string();
    }
    let list: Vec<String> = files.iter().map(|f| format!("- {}", f.display())).collect();
    // The marker is the one the agent reads pictures by, and the list goes last so
    // every path is the last thing in the message whatever was typed above it.
    format!(
        "{message}\n\nAttached files:\n{}\n",
        list.join("\n")
    )
}

/// The menu for the word being typed: where it starts and what it offers.
pub fn completion(text: &str, cx: &gpui::App) -> Option<(Trigger, Vec<Entry>)> {
    let trig = trigger(text)?;
    let sync = &devd(cx).sync;
    let entries: Vec<Entry> = if trig.mark == '/' {
        sync.watch(&q::skills())
            .list("")
            .iter()
            .filter_map(|s| {
                let name = s["name"].as_str()?;
                Some(Entry {
                    insert: format!("/{name}"),
                    title: name.to_string(),
                    detail: s["description"].as_str().unwrap_or_default().to_string(),
                })
            })
            .collect()
    } else {
        sync.watch(&q::roles())
            .list("")
            .iter()
            .filter_map(|r| {
                let id = r["id"].as_str()?;
                Some(Entry {
                    insert: format!("@{id}"),
                    title: r["title"].as_str().unwrap_or(id).to_string(),
                    detail: r["summary"].as_str().unwrap_or_default().to_string(),
                })
            })
            .collect()
    };
    let found = matching(&entries, &trig.query);
    (!found.is_empty()).then_some((trig, found))
}

/// The model the next message uses, in words.
fn model_label(held: &Compose, providers: &[Value]) -> String {
    if let Some((_, model)) = &held.model {
        return model.clone();
    }
    providers
        .iter()
        .find(|p| p["is_default"] == serde_json::json!(true))
        .and_then(|p| p["default_model"].as_str())
        .filter(|m| !m.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| "No model yet".to_string())
}

/// The provider of the model the next message uses: its id and its name.
fn model_provider(held: &Compose, providers: &[Value]) -> (String, String) {
    let found = match &held.model {
        Some((id, _)) => providers.iter().find(|p| p["id"].as_str() == Some(id)),
        None => providers
            .iter()
            .find(|p| p["is_default"] == serde_json::json!(true)),
    };
    found
        .map(|p| {
            (
                p["preset"]
                    .as_str()
                    .or(p["id"].as_str())
                    .unwrap_or_default()
                    .to_string(),
                p["label"].as_str().unwrap_or_default().to_string(),
            )
        })
        .unwrap_or_default()
}

/// One row of a popover.
fn option(
    id: gpui::ElementId,
    title: &str,
    detail: &str,
    current: bool,
    on: bool,
    cx: &gpui::App,
) -> gpui::Stateful<gpui::Div> {
    option_with(id, None, title, detail, current, on, cx)
}

/// A row that starts with a vendor's mark, after the tick.
fn option_with(
    id: gpui::ElementId,
    mark: Option<gpui::Div>,
    title: &str,
    detail: &str,
    current: bool,
    on: bool,
    cx: &gpui::App,
) -> gpui::Stateful<gpui::Div> {
    let t = theme(cx);
    ui::row(id, on, cx)
        .min_h(px(34.0))
        .child(
            div()
                .w(px(14.0))
                .flex_none()
                .when(current, |d| d.child(icon("check", 13.0, t.accent))),
        )
        .children(mark)
        .child(
            div()
                .flex_none()
                .text_size(px(size::TEXT))
                .font_weight(FontWeight::MEDIUM)
                .child(title.to_string()),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_size(px(size::TEXT_SM))
                .text_color(t.muted)
                .child(ui::Clip::new(detail.to_string())),
        )
}

/// A row whose words matter as much as its name: the name, and under it what it means, in full and wrapped.
/// What a mode allows is a decision about risk, so none of it is cut short.
fn choice(id: gpui::ElementId, title: &str, detail: &str, current: bool, cx: &gpui::App) -> gpui::Stateful<gpui::Div> {
    let t = theme(cx);
    ui::row(id, false, cx)
        .items_start()
        .py(px(7.0))
        .child(
            div()
                .w(px(14.0))
                .h(px(20.0))
                .flex_none()
                .flex()
                .items_center()
                .when(current, |d| d.child(icon("check", 13.0, t.accent))),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(1.0))
                .child(div().text_size(px(size::TEXT)).line_height(px(20.0)).font_weight(FontWeight::MEDIUM).child(title.to_string()))
                .child(div().text_size(px(size::TEXT_SM)).text_color(t.muted).child(detail.to_string())),
        )
}

/// What stands before a row's name: a picture, or the symbol that calls it up.
enum Lead {
    /// An icon from the table.
    Icon(&'static str),
    /// A character typed to reach this: `/` for a skill, `@` for a teammate.
    Symbol(char),
}

/// A row of the attach menu or of the `/` and `@` menu. Its first element is a small tile holding the
/// icon or symbol, so every row reads the same way: what it is, then its name, then a word about it.
fn menu_row(
    id: gpui::ElementId,
    lead: Lead,
    title: &str,
    detail: &str,
    on: bool,
    cx: &gpui::App,
) -> gpui::Stateful<gpui::Div> {
    let t = theme(cx);
    // The icon or symbol stands on its own, in a fixed column so the names line up: no tile around it.
    let tile = div()
        .w(px(14.0))
        .h(px(20.0))
        .flex_none()
        .flex()
        .items_center()
        .justify_center();
    let tile = match lead {
        Lead::Icon(name) => tile.child(icon(name, 14.0, t.muted)),
        Lead::Symbol(mark) => tile.child(
            div()
                .font_family(crate::theme::mono())
                .text_size(px(size::TEXT))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(t.accent)
                .child(mark.to_string()),
        ),
    };
    // The mark belongs to its name, so it sits close to it; the description keeps its distance.
    ui::row(id, on, cx)
        .min_h(px(38.0))
        .gap(px(10.0))
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap(px(7.0))
                .child(tile)
                .child(
                    div()
                        .text_size(px(size::TEXT))
                        .font_weight(FontWeight::MEDIUM)
                        .child(title.to_string()),
                ),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_size(px(size::TEXT_SM))
                .text_color(t.muted)
                .child(ui::Clip::new(detail.to_string())),
        )
}

/// A floating list tied to the message box. It opens above the box, or below it where the box sits too
/// near the top of the window to leave room, as it does on Home.
fn popover(cx: &gpui::App, below: bool) -> gpui::Div {
    let t = theme(cx);
    let at = div().absolute().left(px(0.0));
    (if below {
        at.top(relative(1.0)).mt(px(8.0))
    } else {
        at.bottom(relative(1.0)).mb(px(8.0))
    })
    .p(px(5.0))
    .flex()
    .flex_col()
    .gap(px(1.0))
    .rounded(px(size::RADIUS_L))
    .border_1()
    .border_color(t.line_strong)
    .bg(t.raised)
    .shadow_lg()
    .occlude()
}

/// The message box, its menus and the row of choices under it.
pub fn view(
    shell: &Shell,
    screen: crate::shell::ComposerScreen,
    hint: &'static str,
    widest: f32,
    cx: &mut Context<Shell>,
) -> AnyElement {
    let t = theme(cx).clone();
    // The screen this box is drawn on, kept where the box's own buttons can find it: the Assistant's
    // box and Home's are separate controls with separate attachments and choices.
    shell.composer_screen.set(screen);
    let input = shell.composer_of(screen);
    let text = input.read(cx).text().to_string();
    let compose = match screen {
        crate::shell::ComposerScreen::Home => &shell.compose,
        crate::shell::ComposerScreen::Assistant => &shell.chat_compose,
    };
    let inventory = devd(cx).sync.watch(&q::inventory());
    let providers: Vec<Value> = inventory.list("/providers").to_vec();
    let sendable = !text.trim().is_empty();
    let below = matches!(shell.history.current(), devd_app_core::nav::Route::Home);

    let mut root = div()
        .relative()
        .w_full()
        .max_w(px(widest))
        .mx_auto()
        .flex()
        .flex_col()
        .gap(px(6.0));

    // What `/` or `@` offers, above the box.
    if let Some((trig, found)) = completion(&text, cx) {
        let at = compose.selected.min(found.len() - 1);
        let mut menu = popover(cx, below).w_full();
        let heading = format!(
            "{} · {}",
            if trig.mark == '/' { "SKILLS" } else { "THE TEAM" },
            found.len()
        );
        menu = menu.child(div().px(px(10.0)).py(px(4.0)).child(ui::eyebrow(heading, cx)));
        // Every match, in a list about eight rows tall that scrolls; the highlighted row is kept in view as the
        // arrow keys move it.
        let mut list = div()
            .id("compose.completions")
            .max_h(px(336.0))
            .overflow_y_scroll()
            .track_scroll(&compose.list_scroll)
            .flex()
            .flex_col();
        for (n, entry) in found.iter().enumerate() {
            let (insert, from) = (entry.insert.clone(), trig.clone());
            list = list.child(
                div()
                    .child(
                        menu_row(
                            ui::nid(
                                if trig.mark == '/' {
                                    "compose.skill"
                                } else {
                                    "compose.member"
                                },
                                n,
                            ),
                            Lead::Symbol(trig.mark),
                            entry.insert.trim_start_matches(trig.mark),
                            &entry.detail,
                            n == at,
                            cx,
                        )
                        .on_click(cx.listener(
                            move |this, _: &ClickEvent, _, cx| {
                                let now = holding(this).read(cx).text().to_string();
                                let text = accept(&now, &from, &insert);
                                this.composer
                                    .update(cx, |input, cx| input.set_text(text, cx));
                            },
                        )),
                    )
                    .into_any_element(),
            );
        }
        menu = menu.child(list);
        root = root.child(deferred(menu).with_priority(10));
    }

    match compose.open {
        Open::Plus => {
            let menu = popover(cx, below)
                .w(px(330.0))
                .child(
                    menu_row(
                        "compose.add.files".into(),
                        Lead::Icon("file"),
                        "Add files",
                        "Attach them to this message",
                        false,
                        cx,
                    )
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.attach(false, cx))),
                )
                .child(
                    menu_row(
                        "compose.add.folder".into(),
                        Lead::Icon("folder"),
                        "Add a folder",
                        "Give it everything inside",
                        false,
                        cx,
                    )
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.attach(true, cx))),
                )
                .child(div().h(px(1.0)).mx(px(8.0)).my(px(3.0)).bg(t.line))
                .child(
                    menu_row(
                        "compose.add.skill".into(),
                        Lead::Symbol('/'),
                        "Use a skill",
                        "Load its instructions first",
                        false,
                        cx,
                    )
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.start_word("/", cx))),
                )
                .child(
                    menu_row(
                        "compose.add.member".into(),
                        Lead::Symbol('@'),
                        "Ask a team member",
                        "Hand it to one teammate",
                        false,
                        cx,
                    )
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.start_word("@", cx))),
                );
            root = root.child(deferred(dismissable(menu, Open::Plus, cx)).with_priority(10));
        }
        Open::Model => {
            // Wide enough for the effort row: its label and six choices did not fit in 340px, and "Max" was cut off.
            let mut menu = popover(cx, below).w(px(420.0));
            menu = menu.child(
                div()
                    .px(px(10.0))
                    .py(px(4.0))
                    .child(ui::eyebrow("MODEL", cx)),
            );
            let current = model_label(compose, &providers);
            let mut shown = 0;
            // Providers with a key, and the default one even without: its models are what
            // the next message would use, and a row that says it needs a key beats an empty list.
            for provider in providers.iter().filter(|p| {
                p["configured"] == serde_json::json!(true)
                    || p["is_default"] == serde_json::json!(true)
            }) {
                let pid = provider["id"].as_str().unwrap_or_default().to_string();
                let ready = provider["configured"] == serde_json::json!(true);
                for model in provider["models"]
                    .as_array()
                    .map(Vec::as_slice)
                    .unwrap_or(&[])
                    .iter()
                    .take(6)
                {
                    let mid = model["model_id"].as_str().unwrap_or_default().to_string();
                    let name = provider["label"].as_str().unwrap_or_default().to_string();
                    let label = if ready {
                        name.clone()
                    } else {
                        format!("{name} · needs a key")
                    };
                    let (p2, m2) = (pid.clone(), mid.clone());
                    menu = menu.child(
                        option_with(
                            ui::nid("compose.model", shown),
                            Some(ui::brand(
                                provider["preset"].as_str().unwrap_or(&pid),
                                &name,
                                18.0,
                                cx,
                            )),
                            &mid,
                            &label,
                            mid == current,
                            false,
                            cx,
                        )
                        .on_click(cx.listener(
                            move |this, _: &ClickEvent, _, cx| {
                                held(this).open = Open::Nothing;
                                if ready {
                                    held(this).model = Some((p2.clone(), m2.clone()));
                                    cx.notify();
                                } else {
                                    // It cannot answer without a key: go where one is added.
                                    this.go(Route::Intelligence, cx);
                                }
                            },
                        )),
                    );
                    shown += 1;
                }
            }
            if shown == 0 {
                menu = menu.child(
                    div()
                        .px(px(10.0))
                        .py(px(8.0))
                        .text_size(px(size::TEXT_SM))
                        .text_color(t.muted)
                        .child("No model is ready. Add a key in Intelligence."),
                );
            }
            menu = menu.child(
                option(
                    "compose.model.more".into(),
                    "More models",
                    "in Intelligence",
                    false,
                    false,
                    cx,
                )
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    held(this).open = Open::Nothing;
                    this.go(Route::Intelligence, cx);
                })),
            );
            let mut efforts = div()
                .px(px(10.0))
                .py(px(6.0))
                .flex()
                .items_center()
                .gap(px(4.0))
                .child(
                    div()
                        .w(px(56.0))
                        .text_size(px(size::TEXT_SM))
                        .text_color(t.muted)
                        .child("Effort"),
                );
            for (n, (id, label)) in EFFORTS.iter().enumerate() {
                let on =
                    compose.effort == Some(*id) || (compose.effort.is_none() && *id == "medium");
                let pick = *id;
                efforts = efforts.child(
                    Button::new(ui::nid("compose.effort", n), *label)
                        .kind(if on {
                            ButtonKind::Secondary
                        } else {
                            ButtonKind::Ghost
                        })
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            held(this).effort = Some(pick);
                            cx.notify();
                        })),
                );
            }
            let menu = menu.child(div().h(px(1.0)).mx(px(8.0)).my(px(4.0)).bg(t.line)).child(efforts);
            root = root.child(deferred(dismissable(menu, Open::Model, cx)).with_priority(10));
        }
        Open::Mode => {
            let mut menu = popover(cx, below).w(px(340.0));
            menu = menu.child(
                div()
                    .px(px(10.0))
                    .py(px(4.0))
                    .child(ui::eyebrow("WHAT IT MAY DO", cx)),
            );
            for (n, (id, label, detail)) in MODES.iter().enumerate() {
                let pick = *id;
                menu = menu.child(
                    choice(
                        ui::nid("compose.mode", n),
                        label,
                        detail,
                        compose.mode.unwrap_or("auto") == *id,
                        cx,
                    )
                    .on_click(cx.listener(
                        move |this, _: &ClickEvent, _, cx| {
                            held(this).mode = Some(pick);
                            held(this).open = Open::Nothing;
                            cx.notify();
                        },
                    )),
                );
            }
            root = root.child(deferred(dismissable(menu, Open::Mode, cx)).with_priority(10));
        }
        Open::Nothing => {}
    }

    // The box: files attached, the text, and the buttons. A picture is shown as itself, the way
    // the page's composer shows it: a thumbnail with its name and a press to open it full size.
    // Anything else keeps its file mark and its name.
    let mut files = div().flex().flex_wrap().gap(px(6.0));
    for (n, file) in compose.files.iter().enumerate() {
        let name = file
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| file.display().to_string());
        let attach = Button::icon_only(ui::nid("compose.unattach", n), "x").on_click(
            cx.listener(move |this, _: &ClickEvent, _, cx| {
                if n < held(this).files.len() {
                    held(this).files.remove(n);
                }
                cx.notify();
            }),
        );
        let chip = match preview(file) {
            Some(picture) => {
                let open = file.clone();
                div()
                    .w(px(74.0))
                    .rounded(px(size::RADIUS))
                    .border_1()
                    .border_color(t.line)
                    .bg(t.surface)
                    .overflow_hidden()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .w_full()
                            .h(px(56.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .bg(t.bg)
                            .cursor_pointer()
                            .on_mouse_down(MouseButton::Left, cx.listener(move |_this, _: &gpui::MouseDownEvent, _, cx| {
                                cx.open_with_system(&open);
                            }))
                            .child(gpui::img(gpui::ImageSource::Render(picture)).size_full().object_fit(gpui::ObjectFit::Contain)),
                    )
                    .child(
                        div()
                            .px(px(4.0))
                            .py(px(2.0))
                            .flex()
                            .items_center()
                            .gap(px(2.0))
                            .text_size(px(size::TEXT_XS))
                            .child(div().flex_1().min_w_0().child(ui::Clip::new(name)))
                            .child(attach),
                    )
            }
            None => {
                let kind = if file.is_dir() { "folder" } else { "file" };
                div()
                    .h(px(26.0))
                    .pl(px(8.0))
                    .pr(px(4.0))
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .rounded(px(size::RADIUS))
                    .border_1()
                    .border_color(t.line)
                    .bg(t.surface)
                    .text_size(px(size::TEXT_SM))
                    .child(icon(kind, 13.0, t.faint))
                    .child(div().max_w(px(180.0)).child(ui::Clip::new(name)))
                    .child(attach)
            }
        };
        files = files.child(chip);
    }
    let mut boxed = div()
        .px(px(8.0))
        .py(px(8.0))
        .flex()
        .flex_col()
        .gap(px(8.0))
        .rounded(px(size::RADIUS_L))
        .border_1()
        .border_color(t.line_strong)
        .bg(t.raised)
        .text_size(px(size::TEXT_MD))
        .line_height(px(22.0));
    if !compose.files.is_empty() {
        boxed = boxed.child(div().px(px(4.0)).child(files));
    }
    let plus = Button::icon_only("compose.plus", "plus").on_click(cx.listener(
        |this, _: &ClickEvent, _, cx| {
            held(this).toggle(Open::Plus);
            cx.notify();
        },
    ));
    let field = div()
        .flex_1()
        .min_w_0()
        .py(px(3.0))
        .child(input.clone());
    // Next to send: hand this message to the team instead. A project-shaped request used to
    // become a single agent's conversation with no plan, no worktree and no verdict, because
    // the only way to start a run was to know the phrase "have the team ..." - this is the
    // one visible way to choose it. The plan still comes back for approval before any work.
    let team = Button::new("composer.team", "Team")
                    .disabled(!sendable)
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                        let goal = holding(this).read(cx).text().trim().to_string();
                        if goal.is_empty() {
                            return;
                        }
                        this.confirm(
                            "Hand this to the team?",
                            format!(
                                "A planner will propose a plan for:\n\n{}\n\nNothing is built until you approve that plan.",
                                goal
                            ),
                            "Start a run",
                            false,
                            Mutation::new("team.start", json!({ "goal": goal })),
                            window,
                            cx,
                        );
                    }));
    let send = Button::icon_only("composer.send", "arrow-up")
        .kind(ButtonKind::Primary)
        .disabled(!sendable)
        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
            this.submit_composer(window, cx)
        }));
    // A one-line message is one row: attach, the text, send. Once it runs to a second row the text takes the
    // box's whole width and the buttons sit in a row of their own under it, as in every editor people write long
    // prompts in: beside the text, they took a column out of every line of a long message.
    let stacked = !text.is_empty()
        && (compose.stacked.get() || text.contains('\n') || input.read(cx).rows() > 1);
    compose.stacked.set(stacked);
    if stacked {
        boxed = boxed.child(field).child(
            div()
                .flex()
                .items_center()
                .gap(px(8.0))
                .child(plus)
                .child(div().flex_1())
                .child(team)
                .child(send),
        );
    } else {
        boxed = boxed.child(
            div()
                .flex()
                .items_end()
                .gap(px(8.0))
                .child(plus)
                .child(field)
                .child(team)
                .child(send),
        );
    }
    root = root.child(boxed);

    // Under the box: the model, the effort and the mode the next message goes out with.
    let chip = |id: &'static str, label: String, open: Open, cx: &mut Context<Shell>| {
        Button::new(id, label)
            .icon(if open == Open::Model {
                "models"
            } else {
                "lock"
            })
            .kind(ButtonKind::Ghost)
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                held(this).toggle(open);
                cx.notify();
            }))
    };
    let effort = EFFORTS
        .iter()
        .find(|(id, _)| Some(*id) == compose.effort)
        .map(|(_, l)| *l)
        .unwrap_or("Medium");
    let effort = if effort == "Off" { "Thinking off".to_string() } else { effort.to_string() };
    let mode = MODES
        .iter()
        .find(|(id, _, _)| Some(*id) == compose.mode)
        .map(|(_, l, _)| *l)
        .unwrap_or("Auto");
    let mut under = div().px(px(2.0)).flex().items_center().gap(px(4.0));
    // The model chip carries its vendor's mark, so which model is answering is seen at a glance.
    let (provider_id, provider_name) = model_provider(compose, &providers);
    let model_chip = div()
        .id("compose.model.chip")
        .h(px(size::CONTROL))
        .px(px(8.0))
        .flex()
        .flex_none()
        .items_center()
        .gap(px(7.0))
        .rounded(px(size::RADIUS))
        .text_size(px(size::TEXT_SM))
        .font_weight(FontWeight::MEDIUM)
        .text_color(t.muted)
        .cursor_pointer()
        .hover(|s| s.bg(t.hover))
        .child(ui::brand(&provider_id, &provider_name, 18.0, cx))
        .child(format!("{} · {effort}", model_label(compose, &providers)))
        .child(icon("chevron-down", 12.0, t.faint))
        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
            held(this).toggle(Open::Model);
            cx.notify();
        }));
    under =
        under
            .child(model_chip)
            .child(chip("compose.mode.chip", mode.to_string(), Open::Mode, cx));
    under = under.child(
        div()
            .flex_1()
            .min_w_0()
            .flex()
            .justify_end()
            .text_size(px(size::TEXT_XS))
            .text_color(t.faint)
            .font_family(mono())
            .child(ui::Clip::new(hint)),
    );
    root.child(under).into_any_element()
}

impl Shell {
    /// Send what is in the box.
    pub fn submit_composer(&mut self, window: &mut gpui::Window, cx: &mut Context<Self>) {
        self.send(window, cx);
    }

    /// The text of a screen's box changed: close any popover, restart the menu at its first row,
    /// and give the arrows to the menu while one is showing.
    pub fn compose_changed(&mut self, screen: crate::shell::ComposerScreen, cx: &mut Context<Self>) {
        let input = self.composer_of(screen).clone();
        let text = input.read(cx).text().to_string();
        let showing = completion(&text, cx).is_some();
        input.update(cx, |input, _| input.steer(showing));
        let compose = self.compose_of(screen);
        compose.selected = 0;
        compose.list_scroll.scroll_to_item(0);
        compose.open = Open::Nothing;
        cx.notify();
    }

    /// Move the highlight in the `/` or `@` menu.
    pub fn compose_step(&mut self, screen: crate::shell::ComposerScreen, by: isize, cx: &mut Context<Self>) {
        let text = self.composer_of(screen).read(cx).text().to_string();
        if let Some((_, found)) = completion(&text, cx) {
            let n = found.len() as isize;
            let compose = self.compose_of(screen);
            compose.selected = (compose.selected as isize + by).rem_euclid(n) as usize;
            compose.list_scroll.scroll_to_item(compose.selected);
            cx.notify();
        }
    }

    /// Enter in the box: take the highlighted entry if a menu is showing. True when it did.
    pub fn compose_pick(&mut self, screen: crate::shell::ComposerScreen, cx: &mut Context<Self>) -> bool {
        let input = self.composer_of(screen).clone();
        let text = input.read(cx).text().to_string();
        let Some((trig, found)) = completion(&text, cx) else {
            return false;
        };
        let at = self.compose_of(screen).selected.min(found.len() - 1);
        let next = accept(&text, &trig, &found[at].insert);
        input.update(cx, |input, cx| input.set_text(next, cx));
        true
    }

    /// Escape in the box: close what is open.
    pub fn compose_cancel(&mut self, screen: crate::shell::ComposerScreen, cx: &mut Context<Self>) {
        self.compose_of(screen).open = Open::Nothing;
        cx.notify();
    }

    /// Open a new conversation with `text` in the Assistant's box, ready to finish and send. Used
    /// where a screen hands a task to the assistant, such as Make a theme in Settings.
    pub fn ask_assistant(&mut self, text: &str, window: &mut gpui::Window, cx: &mut Context<Self>) {
        self.go(devd_app_core::nav::Route::Assistant(None), cx);
        let _ = self.sync_draft(cx);
        let input = self.chat_composer.clone();
        input.update(cx, |input, cx| input.set_text(text.to_string(), cx));
        window.focus(&gpui::Focusable::focus_handle(input.read(cx), cx));
        cx.notify();
    }

    /// Begin a `/` or `@` word at the end of the message, in the box of the screen in front.
    pub fn start_word(&mut self, mark: &str, cx: &mut Context<Self>) {
        let input = self.composer_of(self.front_screen()).clone();
        let mut text = input.read(cx).text().to_string();
        if !text.is_empty() && !text.ends_with(char::is_whitespace) {
            text.push(' ');
        }
        text.push_str(mark);
        input.update(cx, |input, cx| input.set_text(text, cx));
    }
    /// Ask for files or folders and attach what is chosen.
    pub fn attach(&mut self, folders: bool, cx: &mut Context<Self>) {
        self.compose_of(self.front_screen()).open = Open::Nothing;
        let chosen = cx.prompt_for_paths(PathPromptOptions {
            files: !folders,
            directories: folders,
            multiple: true,
            prompt: None,
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = chosen.await {
                this.update(cx, |this, cx| {
                    for path in paths {
                        if !held(this).files.contains(&path) {
                            held(this).files.push(path);
                        }
                    }
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
        cx.notify();
    }
}

/// A picture read from a path, ready to draw.
///
/// Only what really is a picture by its first bytes is decoded, so a file merely named `.png` is
/// shown as a file rather than as a broken image. Each path is decoded once and kept: a redraw of
/// the composer must not read and decode the same photograph again.
fn preview(path: &Path) -> Option<std::sync::Arc<gpui::RenderImage>> {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex, OnceLock};
    static SEEN: OnceLock<Mutex<HashMap<std::path::PathBuf, Option<Arc<gpui::RenderImage>>>>> = OnceLock::new();
    let seen = SEEN.get_or_init(|| Mutex::new(HashMap::new()));
    if let Ok(map) = seen.lock()
        && let Some(found) = map.get(path)
    {
        return found.clone();
    }
    let found = read_picture(path);
    if let Ok(mut map) = seen.lock() {
        map.insert(path.to_path_buf(), found.clone());
    }
    found
}

fn read_picture(path: &Path) -> Option<std::sync::Arc<gpui::RenderImage>> {
    if !path.is_file() {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    // The GPU takes its pixels blue-first, as gpui's own loader gives them.
    let mut rgba = image::load_from_memory(&bytes).ok()?.into_rgba8();
    for pixel in rgba.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
    }
    Some(std::sync::Arc::new(gpui::RenderImage::new([image::Frame::new(rgba)])))
}

/// What the box of the screen in front has chosen.
fn held(shell: &mut Shell) -> &mut Compose {
    shell.compose_of(shell.front_screen())
}

/// The box of the screen in front.
fn holding(shell: &Shell) -> gpui::Entity<TextInput> {
    shell.composer_of(shell.front_screen()).clone()
}

/// The name a path is shown by.
#[allow(dead_code)]
fn leaf(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, detail: &str) -> Entry {
        Entry {
            insert: format!("/{name}"),
            title: name.to_string(),
            detail: detail.to_string(),
        }
    }

    #[test]
    fn a_slash_or_at_word_at_the_end_is_a_trigger() {
        assert_eq!(
            trigger("/rev").unwrap(),
            Trigger {
                mark: '/',
                query: "rev".into(),
                start: 0
            }
        );
        assert_eq!(
            trigger("ask @qa").unwrap(),
            Trigger {
                mark: '@',
                query: "qa".into(),
                start: 4
            }
        );
        assert_eq!(trigger("fix it\n/").unwrap().start, 7);
        assert_eq!(trigger("/").unwrap().query, "");
    }

    #[test]
    fn a_path_a_finished_word_or_plain_text_is_not() {
        assert!(trigger("look in /usr/bin").is_none());
        assert!(trigger("/review ").is_none(), "a space ends the word");
        assert!(trigger("mail me@example.com").is_none());
        assert!(trigger("hello").is_none());
        assert!(trigger("").is_none());
    }

    #[test]
    fn names_that_begin_with_the_word_come_first() {
        let all = [
            entry("code-review", "reads a diff"),
            entry("review", "reviews work"),
            entry("ship", "release it; review first"),
        ];
        let names: Vec<String> = matching(&all, "rev").into_iter().map(|e| e.title).collect();
        assert_eq!(names, ["review", "code-review", "ship"]);
        assert_eq!(matching(&all, "").len(), 3);
        assert!(matching(&all, "zzz").is_empty());
    }

    #[test]
    fn accepting_replaces_only_the_word() {
        let text = "please /rev";
        let trig = trigger(text).unwrap();
        assert_eq!(accept(text, &trig, "/review"), "please /review ");
    }

    #[test]
    fn attached_files_are_listed_for_the_agent() {
        assert_eq!(with_files("hi", &[]), "hi");
        let said = with_files("look", &[PathBuf::from("a.txt"), PathBuf::from("src")]);
        assert!(
            said.starts_with("look\n\nAttached files:\n- a.txt\n- src"),
            "{said}"
        );
        // The same marker the agent reads pictures by: a picture attached from
        // the window reached the model as a bare path when the words were its own.
        assert!(said.trim_end().ends_with("- src"), "the list ends the message");
    }
}
