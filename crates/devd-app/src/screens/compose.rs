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
use gpui::{
    AnyElement, ClickEvent, Context, FontWeight, PathPromptOptions, deferred, div, prelude::*, px,
    relative,
};
use serde_json::Value;

use crate::shell::Shell;
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
    ("low", "Low"),
    ("medium", "Medium"),
    ("high", "High"),
    ("extra", "Extra"),
    ("max", "Max"),
];

/// What the message box has chosen, beyond its text.
#[derive(Debug, Default)]
pub struct Compose {
    /// The popover that is open.
    pub open: Open,
    /// The mode chosen, or None to leave it as the conversation has it.
    pub mode: Option<&'static str>,
    /// The effort chosen, or None for the ordinary default.
    pub effort: Option<&'static str>,
    /// The provider and model chosen, or None for the default one.
    pub model: Option<(String, String)>,
    /// Files and folders attached to the next message.
    pub files: Vec<PathBuf>,
    /// The highlighted row of the `/` or `@` menu.
    pub selected: usize,
    /// The popover a press outside it just closed, and when: the same press lands on the button that opened it.
    pub dismissed: Option<(Open, std::time::Instant)>,
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
            if this.compose.open == which {
                this.compose.open = Open::Nothing;
                this.compose.dismissed = Some((which, std::time::Instant::now()));
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
    format!(
        "{message}\n\nAttached (read them from disk):\n{}",
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
fn model_label(shell: &Shell, providers: &[Value]) -> String {
    if let Some((_, model)) = &shell.compose.model {
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
fn model_provider(shell: &Shell, providers: &[Value]) -> (String, String) {
    let found = match &shell.compose.model {
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
    let tile = div()
        .size(px(24.0))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(size::RADIUS))
        .bg(t.hover)
        .border_1()
        .border_color(t.line);
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
    ui::row(id, on, cx)
        .min_h(px(38.0))
        .gap(px(10.0))
        .child(tile)
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
    hint: &'static str,
    running: bool,
    widest: f32,
    cx: &mut Context<Shell>,
) -> AnyElement {
    let t = theme(cx).clone();
    let text = shell.composer.read(cx).text().to_string();
    let compose = &shell.compose;
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
        menu = menu.child(div().px(px(10.0)).py(px(4.0)).child(ui::eyebrow(
            if trig.mark == '/' {
                "SKILLS"
            } else {
                "THE TEAM"
            },
            cx,
        )));
        for (n, entry) in found.iter().take(8).enumerate() {
            let (insert, from) = (entry.insert.clone(), trig.clone());
            menu = menu.child(
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
                                let now = this.composer.read(cx).text().to_string();
                                let text = accept(&now, &from, &insert);
                                this.composer
                                    .update(cx, |input, cx| input.set_text(text, cx));
                            },
                        )),
                    )
                    .into_any_element(),
            );
        }
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
            let mut menu = popover(cx, below).w(px(340.0));
            menu = menu.child(
                div()
                    .px(px(10.0))
                    .py(px(4.0))
                    .child(ui::eyebrow("MODEL", cx)),
            );
            let current = model_label(shell, &providers);
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
                                this.compose.open = Open::Nothing;
                                if ready {
                                    this.compose.model = Some((p2.clone(), m2.clone()));
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
                    this.compose.open = Open::Nothing;
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
                            this.compose.effort = Some(pick);
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
                            this.compose.mode = Some(pick);
                            this.compose.open = Open::Nothing;
                            cx.notify();
                        },
                    )),
                );
            }
            root = root.child(deferred(dismissable(menu, Open::Mode, cx)).with_priority(10));
        }
        Open::Nothing => {}
    }

    // The box: files attached, the text, and the buttons.
    let mut files = div().flex().flex_wrap().gap(px(6.0));
    for (n, file) in compose.files.iter().enumerate() {
        let name = file
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| file.display().to_string());
        let kind = if file.is_dir() { "folder" } else { "file" };
        files = files.child(
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
                .child(
                    Button::icon_only(ui::nid("compose.unattach", n), "x").on_click(cx.listener(
                        move |this, _: &ClickEvent, _, cx| {
                            if n < this.compose.files.len() {
                                this.compose.files.remove(n);
                            }
                            cx.notify();
                        },
                    )),
                ),
        );
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
    // One row: attach, the text, send. A long message grows the row upward-anchored,
    // so the buttons stay beside its last line.
    boxed = boxed.child(
        div()
            .flex()
            .items_end()
            .gap(px(8.0))
            .child(
                Button::icon_only("compose.plus", "plus").on_click(cx.listener(
                    |this, _: &ClickEvent, _, cx| {
                        this.compose.toggle(Open::Plus);
                        cx.notify();
                    },
                )),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .py(px(3.0))
                    .child(shell.composer.clone()),
            )
            .child(
                Button::icon_only("composer.send", "arrow-up")
                    .kind(ButtonKind::Primary)
                    .disabled(!sendable)
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                        this.submit_composer(window, cx)
                    })),
            ),
    );
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
                this.compose.toggle(open);
                cx.notify();
            }))
    };
    let effort = EFFORTS
        .iter()
        .find(|(id, _)| Some(*id) == compose.effort)
        .map(|(_, l)| *l)
        .unwrap_or("Medium");
    let mode = MODES
        .iter()
        .find(|(id, _, _)| Some(*id) == compose.mode)
        .map(|(_, l, _)| *l)
        .unwrap_or("Auto");
    let mut under = div().px(px(2.0)).flex().items_center().gap(px(4.0));
    // The model chip carries its vendor's mark, so which model is answering is seen at a glance.
    let (provider_id, provider_name) = model_provider(shell, &providers);
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
        .child(format!("{} · {effort}", model_label(shell, &providers)))
        .child(icon("chevron-down", 12.0, t.faint))
        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
            this.compose.toggle(Open::Model);
            cx.notify();
        }));
    under =
        under
            .child(model_chip)
            .child(chip("compose.mode.chip", mode.to_string(), Open::Mode, cx));
    if running {
        under = under.child(
            div()
                .flex()
                .items_center()
                .gap(px(6.0))
                .ml(px(6.0))
                .child(ui::live_dot("compose.live", crate::theme::Tone::Info, cx))
                .child(
                    div()
                        .text_size(px(size::TEXT_SM))
                        .text_color(t.info)
                        .child("working"),
                ),
        );
    }
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

    /// The text changed: close any popover, restart the menu at its first row, and
    /// give the arrows to the menu while one is showing.
    pub fn compose_changed(&mut self, cx: &mut Context<Self>) {
        let text = self.composer.read(cx).text().to_string();
        let showing = completion(&text, cx).is_some();
        self.composer.update(cx, |input, _| input.steer(showing));
        self.compose.selected = 0;
        self.compose.open = Open::Nothing;
        cx.notify();
    }

    /// Move the highlight in the `/` or `@` menu.
    pub fn compose_step(&mut self, by: isize, cx: &mut Context<Self>) {
        let text = self.composer.read(cx).text().to_string();
        if let Some((_, found)) = completion(&text, cx) {
            let n = found.len().min(8) as isize;
            self.compose.selected = (self.compose.selected as isize + by).rem_euclid(n) as usize;
            cx.notify();
        }
    }

    /// Enter in the box: take the highlighted entry if a menu is showing. True when it did.
    pub fn compose_pick(&mut self, cx: &mut Context<Self>) -> bool {
        let text = self.composer.read(cx).text().to_string();
        let Some((trig, found)) = completion(&text, cx) else {
            return false;
        };
        let entry = &found[self.compose.selected.min(found.len() - 1)];
        let next = accept(&text, &trig, &entry.insert);
        self.composer
            .update(cx, |input, cx| input.set_text(next, cx));
        true
    }

    /// Escape in the box: close what is open.
    pub fn compose_cancel(&mut self, cx: &mut Context<Self>) {
        self.compose.open = Open::Nothing;
        cx.notify();
    }

    /// Begin a `/` or `@` word at the end of the message.
    pub fn start_word(&mut self, mark: &str, cx: &mut Context<Self>) {
        let mut text = self.composer.read(cx).text().to_string();
        if !text.is_empty() && !text.ends_with(char::is_whitespace) {
            text.push(' ');
        }
        text.push_str(mark);
        self.composer
            .update(cx, |input, cx| input.set_text(text, cx));
    }

    /// Ask for files or folders and attach what is chosen.
    pub fn attach(&mut self, folders: bool, cx: &mut Context<Self>) {
        self.compose.open = Open::Nothing;
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
                        if !this.compose.files.contains(&path) {
                            this.compose.files.push(path);
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
            said.starts_with("look\n\nAttached")
                && said.contains("- a.txt")
                && said.contains("- src")
        );
    }
}
