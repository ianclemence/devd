//! The Assistant: conversations on the left, the one you are in on the right.
//!
//! A conversation is a log that only grows, so it is read incrementally (the
//! sync engine asks for what came after the last event it holds) and drawn
//! through a virtual list, which lays out only the rows on screen. A thousand
//! events cost the same to open as ten.
//!
//! The log's events are folded into what a person reads: your message, devd's
//! words, and each tool call as one quiet line with its result, not as a
//! request and a response a hundred lines apart.

use std::collections::HashMap;
use std::rc::Rc;

use devd_app_core::markdown::{self, Block};
use devd_app_core::nav::Route;
use devd_app_core::sync::Mutation;
use devd_app_core::toolname::{self, Phase};
use gpui::{
    AnyElement, App, ClickEvent, Context, FontWeight, ListAlignment, ListOffset, ListState, Window,
    div, list, prelude::*, px,
};
use serde_json::{Value, json};

use crate::shell::Shell;
use crate::state::{devd, q};
use crate::theme::{Tone, mono, size, theme};
use crate::ui::{self, Button, ButtonKind, eyebrow, icon, live_dot, markdown as md};

/// One thing a person reads in a conversation.
#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    /// What the person wrote. `pending` until devd has stored it.
    You { text: String, pending: bool },
    /// What devd wrote, parsed for drawing.
    Devd { blocks: Vec<Block>, model: String },
    /// A tool call as the model made it and, once it has one, its result.
    /// It is put into words when it is drawn, because the words depend on how it ended.
    Tool {
        name: String,
        input: Value,
        ok: Option<bool>,
        detail: String,
    },
    /// Something devd's harness said: a pause, a limit.
    Notice(String),
    /// A failure that ended the turn.
    Failed(String),
    /// An operation waiting for the person's approval.
    Waiting { capability: String, summary: String },
    /// A run of finished steps, folded into one line ("Read 3 files and ran a command") that opens when pressed.
    Steps(Vec<Item>),
    /// The files one step changed, with a way back: the `changes` event's id, each file's status and path,
    /// and whether it has been undone.
    Changes { event: i64, files: Vec<(String, String)>, undone: bool },
}

/// Fold a conversation's events into the items a person reads.
pub fn fold(events: &[Value], pending: &[&str]) -> Vec<Item> {
    let mut items: Vec<Item> = Vec::new();
    // Where each tool call landed, so its result can be attached to it.
    let mut calls: HashMap<String, usize> = HashMap::new();
    // Output tokens, cost and time of the model's replies since the person last spoke.
    let mut turn: (u64, f64, u64) = (0, 0.0, 0);
    for event in events {
        let data = &event["data"];
        if event["kind"] == "user" && data["gate"] != json!(true) && data["drawing"] != json!(true) {
            turn = (0, 0.0, 0);
        }
        match event["kind"].as_str().unwrap_or_default() {
            // devd's own reminder before it lets a turn end is not something the person said.
            "user" if data["gate"] == json!(true) => {
                let n = data["objections"].as_u64().unwrap_or(1);
                items.push(Item::Notice(format!(
                    "devd sent it back to finish: {n} thing{} still open",
                    if n == 1 { " was" } else { "s were" }
                )));
            }
            // The same for a drawing devd sent back because it could not be drawn.
            "user" if data["drawing"] == json!(true) => {
                let n = data["problems"].as_u64().unwrap_or(1);
                items.push(Item::Notice(format!(
                    "devd asked it to draw that again: {n} problem{} with the drawing",
                    if n == 1 { "" } else { "s" }
                )));
            }
            "user" => items.push(Item::You {
                text: data["text"].as_str().unwrap_or_default().to_string(),
                pending: data["pending"] == json!(true),
            }),
            "assistant" => {
                // What the turn has cost so far: the label under its last words carries the whole turn.
                turn.0 += data["usage"]["output"].as_u64().unwrap_or(0);
                turn.1 += data["usage"]["cost_usd"].as_f64().unwrap_or(0.0);
                turn.2 += data["took_ms"].as_u64().unwrap_or(0);
                let model = reply_label(data["model"].as_str().unwrap_or_default(), turn);
                for block in data["content"].as_array().map(Vec::as_slice).unwrap_or(&[]) {
                    match block["type"].as_str().unwrap_or_default() {
                        "text" => {
                            let text = block["text"].as_str().unwrap_or_default();
                            if !text.trim().is_empty() {
                                items.push(Item::Devd {
                                    blocks: markdown::parse(text),
                                    model: model.clone(),
                                });
                            }
                        }
                        "tool_use" => {
                            calls.insert(
                                block["id"].as_str().unwrap_or_default().to_string(),
                                items.len(),
                            );
                            items.push(Item::Tool {
                                name: block["name"].as_str().unwrap_or_default().to_string(),
                                input: block["input"].clone(),
                                ok: None,
                                detail: String::new(),
                            });
                        }
                        _ => {}
                    }
                }
            }
            "tool_result" => {
                let failed = data["is_error"] == json!(true);
                let text = data["content"].as_str().unwrap_or_default();
                if let Some(Item::Tool { ok, detail, .. }) = calls
                    .get(data["tool_use_id"].as_str().unwrap_or_default())
                    .and_then(|at| items.get_mut(*at))
                {
                    *ok = Some(!failed);
                    *detail = result_line(text, failed);
                }
            }
            "changes" => items.push(Item::Changes {
                event: event["id"].as_i64().unwrap_or_default(),
                files: data["files"]
                    .as_array()
                    .map(|f| {
                        f.iter()
                            .map(|c| (c["status"].as_str().unwrap_or("M").to_string(), c["path"].as_str().unwrap_or_default().to_string()))
                            .collect()
                    })
                    .unwrap_or_default(),
                undone: false,
            }),
            "undone" => {
                let which = data["event"].as_i64();
                for item in items.iter_mut() {
                    if let Item::Changes { event, undone, .. } = item
                        && Some(*event) == which
                    {
                        *undone = true;
                    }
                }
            }
            "notice" => items.push(Item::Notice(
                data["text"].as_str().unwrap_or_default().to_string(),
            )),
            "error" => items.push(Item::Failed(
                data["message"].as_str().unwrap_or_default().to_string(),
            )),
            // The log keeps every request ever made. Only one that is still
            // undecided is something the person is being asked.
            "approval" if pending.contains(&data["request_id"].as_str().unwrap_or_default()) => {
                items.push(Item::Waiting {
                    capability: data["capability"].as_str().unwrap_or_default().to_string(),
                    summary: data["plan"]["summary"]
                        .as_str()
                        .unwrap_or_default()
                        .to_string(),
                })
            }
            _ => {}
        }
    }
    one_name_per_turn(&mut items);
    fold_steps(items)
}

/// Two or more finished steps in a row are one line, the way a person reports them. A step still running is
/// never folded: it is what is happening now.
fn fold_steps(items: Vec<Item>) -> Vec<Item> {
    let mut out: Vec<Item> = Vec::with_capacity(items.len());
    let mut run: Vec<Item> = Vec::new();
    let flush = |run: &mut Vec<Item>, out: &mut Vec<Item>| match run.len() {
        0 => {}
        1 => out.append(run),
        _ => out.push(Item::Steps(std::mem::take(run))),
    };
    for item in items {
        match &item {
            Item::Tool { ok: Some(_), .. } => run.push(item),
            _ => {
                flush(&mut run, &mut out);
                out.push(item);
            }
        }
    }
    flush(&mut run, &mut out);
    out
}

/// The quiet line under a reply: the model, then what the turn took, such as "deepseek-flash · 2.3k tokens ·
/// 41/s · $0.012". A figure that was not recorded is left out rather than shown as zero.
fn reply_label(model: &str, (tokens, cost, ms): (u64, f64, u64)) -> String {
    let mut parts = vec![model.to_string()];
    if tokens > 0 {
        parts.push(format!("{} tokens", ui::compact(tokens)));
        if ms > 0 {
            parts.push(format!("{:.0}/s", tokens as f64 / (ms as f64 / 1000.0)));
        }
    }
    if cost > 0.0 {
        parts.push(if cost < 0.01 { format!("${cost:.4}") } else { format!("${cost:.2}") });
    }
    parts.retain(|p| !p.is_empty());
    parts.join(" · ")
}

/// An agent's turn is many model calls: a sentence, a tool, a sentence, a tool. Read together they are one
/// reply, so the model that wrote it is named once, under the last thing it wrote before the person spoke
/// again, not under every step.
fn one_name_per_turn(items: &mut [Item]) {
    let mut last_in_turn: Option<usize> = None;
    let mut named: Vec<usize> = Vec::new();
    for (n, item) in items.iter().enumerate() {
        match item {
            Item::Devd { .. } => last_in_turn = Some(n),
            Item::You { .. } => named.extend(last_in_turn.take()),
            _ => {}
        }
    }
    named.extend(last_in_turn);
    for (n, item) in items.iter_mut().enumerate() {
        if let Item::Devd { model, .. } = item
            && !named.contains(&n)
        {
            model.clear();
        }
    }
}

/// The one line of a tool result worth showing beside the call.
fn result_line(text: &str, failed: bool) -> String {
    let parsed: Value = serde_json::from_str(text).unwrap_or(Value::Null);
    let line = if failed {
        parsed["error"]["message"]
            .as_str()
            .or(parsed["error"].as_str())
            .unwrap_or(text)
    } else {
        parsed["result"]["outcome"]["message"]
            .as_str()
            .or(parsed["result"]["message"].as_str())
            .or(parsed["message"].as_str())
            .unwrap_or_default()
    };
    let line = line.lines().next().unwrap_or_default().trim();
    line.chars().take(160).collect()
}

/// The conversation on screen, prepared for drawing.
pub struct Transcript {
    thread: Option<String>,
    /// How many events the items were folded from, to know when to fold again.
    events: usize,
    items: Rc<Vec<Item>>,
    /// The reply being written, parsed, with the length of the text it was parsed from.
    writing: Option<(usize, Rc<Vec<Block>>)>,
    list: ListState,
    /// The failure that offers Resume, if any. It is taller than a plain one, so the list measures it again when this changes.
    resume_at: Option<usize>,
}

impl Transcript {
    /// Nothing open.
    pub fn new() -> Self {
        Self {
            thread: None,
            events: 0,
            items: Rc::new(Vec::new()),
            writing: None,
            list: ListState::new(0, ListAlignment::Bottom, px(600.0)),
            resume_at: None,
        }
    }

    /// Bring the prepared items up to date with the events and the live text.
    fn update(&mut self, thread: &str, events: &[Value], pending: &[&str], writing: Option<&str>) {
        let switched = self.thread.as_deref() != Some(thread);
        let before = self.items.len() + usize::from(self.writing.is_some());
        if switched || events.len() + pending.len() != self.events {
            self.items = Rc::new(fold(events, pending));
            self.events = events.len() + pending.len();
            self.thread = Some(thread.to_string());
        }
        let live_changed = match (writing.filter(|w| !w.trim().is_empty()), &self.writing) {
            (None, None) => false,
            (Some(text), Some((len, _))) if *len == text.len() => false,
            (Some(text), _) => {
                self.writing = Some((text.len(), Rc::new(markdown::parse(text))));
                true
            }
            (None, Some(_)) => {
                self.writing = None;
                true
            }
        };
        let after = self.items.len() + usize::from(self.writing.is_some());
        if switched {
            self.list.reset(after);
            // A conversation opens on its newest message.
            self.list.scroll_to(ListOffset {
                item_ix: after,
                offset_in_item: px(0.0),
            });
        } else if after != before {
            // Reading the newest messages means following new ones as they
            // arrive; having scrolled back to read means staying put.
            let following = self.list.logical_scroll_top().item_ix + 12 >= before;
            // Everything from the first changed row on is laid out again.
            let keep = before.min(after).saturating_sub(1);
            self.list.splice(keep..before, after - keep);
            if following {
                self.list.scroll_to(ListOffset {
                    item_ix: after,
                    offset_in_item: px(0.0),
                });
            }
        } else if live_changed && after > 0 {
            // The row being written grew; only it is measured again.
            self.list.splice(after - 1..after, 1);
        }
    }
}

impl Default for Transcript {
    fn default() -> Self {
        Self::new()
    }
}

/// The screen.
pub fn render(
    shell: &mut Shell,
    thread: Option<String>,
    _window: &mut Window,
    cx: &mut Context<Shell>,
) -> AnyElement {
    let t = theme(cx).clone();
    // The strip first: it opens the pane when the assistant has just started the browser.
    let strip = super::pane::strip(shell, cx);
    let current = thread.clone();
    let main = match thread {
        Some(id) => conversation(shell, &id, cx),
        None => start(shell, cx),
    };
    div()
        .size_full()
        .flex()
        .child(sidebar(current.as_deref(), cx))
        .child(main)
        .children(super::pane::pane(shell, cx))
        .child(strip)
        .bg(t.bg)
        .into_any_element()
}

fn sidebar(current: Option<&str>, cx: &mut Context<Shell>) -> impl IntoElement {
    let t = theme(cx).clone();
    let threads = devd(cx).sync.watch(&q::threads());
    let mut column = div().flex().flex_col().gap(px(1.0)).p(px(8.0));
    if threads.is_first_load() {
        for n in 0..6 {
            column = column.child(div().h(px(44.0)).px(px(10.0)).flex().items_center().child(
                ui::skeleton(
                    ui::nid("threads.skeleton", n),
                    150.0 + (n as f32 * 23.0) % 70.0,
                    cx,
                ),
            ));
        }
    }
    // A specialist's conversation belongs to its run, and is reached from there.
    let own: Vec<&Value> = threads
        .list("")
        .iter()
        .filter(|th| !th["team"]["role"].is_string())
        .collect();
    let mut last_project: Option<String> = None;
    for thread in own {
        let id = thread["id"].as_str().unwrap_or_default().to_string();
        // A conversation with no project is listed without a heading; a heading says which project the ones under it are about.
        let project = thread["project"].as_str().map(|p| ui::leaf(p).to_string());
        if project != last_project {
            if let Some(name) = &project {
                column = column.child(
                    div()
                        .px(px(10.0))
                        .pt(px(12.0))
                        .pb(px(4.0))
                        .child(eyebrow(name.to_uppercase(), cx)),
                );
            }
            last_project = project;
        }
        let state = thread["state"].as_str().unwrap_or_default();
        let said = devd_app_core::status::of(state);
        // A conversation at rest has no mark: the list is quiet until something needs a look.
        let marker: AnyElement =
            if said.label.is_empty() || said.level == devd_app_core::status::Level::Neutral {
                div().size(px(7.0)).flex_none().into_any_element()
            } else {
                ui::state_mark(
                    gpui::ElementId::Name(
                        format!("thread.live.{}", thread["id"].as_str().unwrap_or_default()).into(),
                    ),
                    &said,
                    cx,
                )
            };
        let go = id.clone();
        let (doomed, doomed_title, was_open) = (
            id.clone(),
            thread["title"]
                .as_str()
                .unwrap_or("this conversation")
                .to_string(),
            current == Some(id.as_str()),
        );
        column = column.child(
            ui::row(
                gpui::ElementId::Name(format!("thread.{id}").into()),
                current == Some(id.as_str()),
                cx,
            )
            .group("thread.row")
            .h(px(34.0))
            .child(marker)
            .child(
                div().flex_1().min_w_0().child(ui::Clip::new(
                    thread["title"]
                        .as_str()
                        .unwrap_or("Conversation")
                        .to_string(),
                )),
            )
            .child(
                div()
                    .flex_none()
                    .text_size(px(size::TEXT_XS))
                    .text_color(t.faint)
                    .child(ui::ago(thread["updated_at"].as_str().unwrap_or_default())),
            )
            .child(ui::row_action(
                gpui::ElementId::Name(format!("thread.delete.{id}").into()),
                "thread.row",
                "trash",
                "Delete conversation",
                true,
                cx.listener(move |this, _: &ClickEvent, window, cx| {
                    this.delete_conversation(
                        doomed.clone(),
                        doomed_title.clone(),
                        was_open,
                        window,
                        cx,
                    )
                }),
                cx,
            ))
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.go(Route::Assistant(Some(go.clone())), cx)
            })),
        );
    }
    let clear = div().flex_none().p(px(8.0)).border_t_1().border_color(t.line).child(
        Button::new("threads.clear", "Clear finished").icon("trash").kind(ButtonKind::Ghost).on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
            this.confirm(
                "Delete every finished conversation?".to_string(),
                "Every conversation that is not in the middle of something is removed, with its messages and log. Files in your projects stay. This cannot be undone.".to_string(),
                "Delete finished",
                true,
                Mutation::new("chat.prune", json!({})).invalidating("chat"),
                window,
                cx,
            )
        })),
    );
    div()
        .w(px(size::SIDEBAR))
        .h_full()
        .flex()
        .flex_col()
        .flex_none()
        .border_r_1()
        .border_color(t.line)
        .child(
            div()
                .h(px(44.0))
                .px(px(14.0))
                .flex()
                .flex_none()
                .items_center()
                .justify_between()
                .child(div().font_weight(FontWeight::MEDIUM).child("Conversations"))
                .child(
                    Button::new("thread.new", "New")
                        .icon("plus")
                        .keys("secondary-n")
                        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                            this.run("conversation.new", window, cx)
                        })),
                ),
        )
        .child(
            div()
                .id("threads")
                .flex_1()
                .overflow_y_scroll()
                .child(column),
        )
        .child(clear)
}

impl Shell {
    /// Ask before deleting a conversation, and delete it. Its messages and its log go; nothing else does.
    pub fn delete_conversation(
        &mut self,
        id: String,
        title: String,
        _was_open: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.confirm(
            format!("Delete \"{}\"?", ui::clip_chars(&title, 48)),
            "The conversation, its messages and its log are removed. Anything it changed in your projects stays. This cannot be undone.",
            "Delete conversation",
            true,
            Mutation::new("chat.delete", json!({ "thread": id })).invalidating("chat"),
            window,
            cx,
        );
    }
}

/// The message box, with its menus: see [`super::compose`].
fn composer(
    shell: &Shell,
    hint: &'static str,
    running: bool,
    cx: &mut Context<Shell>,
) -> AnyElement {
    super::compose::view(shell, hint, running, size::READING, cx)
}

/// No conversation open: the box, and where to begin.
fn start(shell: &mut Shell, cx: &mut Context<Shell>) -> AnyElement {
    let t = theme(cx).clone();
    // A new conversation's first message creates it; go there once devd says which it is.
    if let Some(id) = cx.global_mut::<crate::state::Devd>().opened_thread.take() {
        shell.history.select(Route::Assistant(Some(id)));
        cx.notify();
    }
    div()
        .relative()
        .flex_1()
        .min_w_0()
        .h_full()
        .flex()
        .flex_col()
        .justify_center()
        .gap(px(22.0))
        .px(px(32.0))
        .child(
            div()
                .w_full()
                .max_w(px(size::READING))
                .mx_auto()
                .flex()
                .flex_col()
                .gap(px(6.0))
                .child(div().text_size(px(size::TEXT_DISPLAY)).font_weight(FontWeight::SEMIBOLD).child("What are we working on?"))
                .child(div().text_size(px(size::TEXT_MD)).text_color(t.muted).child("Ask a question, describe a change, or hand the team a goal. devd reads the project before it answers.")),
        )
        .child(composer(shell, "Enter sends · Shift+Enter adds a line", false, cx))
        .into_any_element()
}

fn conversation(shell: &mut Shell, id: &str, cx: &mut Context<Shell>) -> AnyElement {
    cx.global_mut::<crate::state::Devd>().open_thread = Some(id.to_string());
    let t = theme(cx).clone();
    let state = devd(cx);
    let snap = state.sync.watch_log(&q::thread(id), q::thread_log());
    let writing = state.writing.get(id).cloned();
    let thinking = state.thinking.get(id).cloned();
    let Some(data) = snap.data.clone() else {
        let body = match &snap.error {
            Some(failure) if failure.code == "not_found" => div()
                .flex()
                .flex_col()
                .items_center()
                .gap(px(14.0))
                .child(ui::empty(
                    "assistant",
                    "This conversation is gone",
                    "It was deleted, here or somewhere else.",
                    cx,
                ))
                .child(
                    Button::new("conversation.gone", "Start a new one")
                        .icon("plus")
                        .kind(ButtonKind::Primary)
                        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                            this.run("conversation.new", window, cx)
                        })),
                )
                .into_any_element(),
            Some(failure) => ui::empty(
                "alert",
                "This conversation could not be read",
                failure.message.clone(),
                cx,
            )
            .into_any_element(),
            None => div()
                .p(px(32.0))
                .flex()
                .flex_col()
                .gap(px(14.0))
                .children((0..5).map(|n| {
                    ui::skeleton(
                        ui::nid("conversation.skeleton", n),
                        220.0 + n as f32 * 60.0,
                        cx,
                    )
                }))
                .into_any_element(),
        };
        return div()
            .flex_1()
            .min_w_0()
            .h_full()
            .child(body)
            .into_any_element();
    };
    let events = data["events"].as_array().map(Vec::as_slice).unwrap_or(&[]);
    let pending: Vec<&str> = data["pending"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[])
        .iter()
        .filter_map(|p| p["id"].as_str().or(p["request_id"].as_str()))
        .collect();
    shell
        .transcript
        .update(id, events, &pending, writing.as_deref());

    let thread = &data["thread"];
    // Running in this window's server, or in another process (a terminal, a script, CI) that writes to the
    // same record: the record says so, and is what to believe.
    let running = data["running"] == json!(true) || thread["state"].as_str() == Some("running");
    if running {
        devd(cx).chat_running.set(true);
    }
    let tokens = thread["input_tokens"].as_u64().unwrap_or(0)
        + thread["output_tokens"].as_u64().unwrap_or(0);
    // Relative paths in this conversation's replies are read against its project.
    cx.global_mut::<crate::state::Devd>().link_root =
        thread["project"].as_str().map(std::path::PathBuf::from);
    let project = thread["project"].as_str().map(project_name);
    let title = thread["title"]
        .as_str()
        .unwrap_or("Conversation")
        .to_string();

    // The heading: the title, then one quiet line of facts, then the actions. Facts are words in the page's secondary colour
    // separated by a middle dot, not badges; an error is not squeezed in here but given its own line below.
    let mut facts: Vec<String> = Vec::new();
    if let Some(project) = project {
        facts.push(project);
    }
    if tokens > 0 {
        facts.push(format!("{} tokens", ui::compact(tokens)));
    }
    let mut actions = div().flex().items_center().gap(px(2.0));
    if running {
        let stop = id.to_string();
        actions = actions
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .pr(px(6.0))
                    .child(live_dot("conversation.live", Tone::Info, cx))
                    .child(
                        div()
                            .text_size(px(size::TEXT_SM))
                            .text_color(t.muted)
                            .child("Working"),
                    ),
            )
            .child(
                Button::new("conversation.stop", "Stop")
                    .icon("stop")
                    .kind(ButtonKind::Ghost)
                    .on_click(move |_, _, cx| {
                        devd(cx)
                            .sync
                            .mutate(Mutation::new("chat.stop", json!({ "thread": stop })));
                    }),
            );
    }
    let (doomed, doomed_title) = (id.to_string(), title.clone());
    actions = actions.child(
        div()
            .id("conversation.delete.wrap")
            .tooltip(ui::tip("Delete conversation"))
            .child(
                Button::icon_only("conversation.delete", "trash").on_click(cx.listener(
                    move |this, _: &ClickEvent, window, cx| {
                        this.delete_conversation(
                            doomed.clone(),
                            doomed_title.clone(),
                            true,
                            window,
                            cx,
                        )
                    },
                )),
            ),
    );
    let head = div()
        .h(px(48.0))
        .px(px(20.0))
        .flex()
        .flex_none()
        .items_center()
        .gap(px(16.0))
        .border_b_1()
        .border_color(t.line)
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .items_baseline()
                .gap(px(10.0))
                .child(
                    div()
                        .min_w_0()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(t.text)
                        .child(ui::Clip::new(title)),
                )
                .children((!facts.is_empty()).then(|| {
                    div()
                        .flex_none()
                        .text_size(px(size::TEXT_SM))
                        .text_color(t.faint)
                        .child(facts.join("  ·  "))
                })),
        )
        .child(actions);

    // A conversation that stopped on an error says so once. Its last failure in the transcript is where that is read,
    // beside the message box, and that is where it is resumed from. Only when the transcript holds no such failure is
    // it said above instead.
    let items = shell.transcript.items.clone();
    // What the status line says devd is doing: thinking, writing, or the step that is running.
    let is_writing = writing.as_deref().is_some_and(|w| !w.trim().is_empty());
    let (doing, glimpse) = match (&thinking, is_writing) {
        (Some((_, tail)), false) => ("Thinking".to_string(), Some(tail.clone())),
        (_, true) => ("Writing".to_string(), None),
        _ => {
            let step = items.iter().rev().find_map(|i| match i {
                Item::Tool { name, input, ok: None, .. } => {
                    let said = toolname::describe(name, input, Phase::Running);
                    Some(if said.object.is_empty() { said.action } else { format!("{} {}", said.action, said.object) })
                }
                _ => None,
            });
            (step.unwrap_or_else(|| "Working".to_string()), None)
        }
    };
    // The turn began when the person last spoke.
    let turn_began = data["events"]
        .as_array()
        .and_then(|events| events.iter().rev().find(|e| e["kind"] == "user"))
        .and_then(|e| e["created_at"].as_str())
        .and_then(|at| chrono::DateTime::parse_from_rfc3339(at).ok())
        .map(|at| at.with_timezone(&chrono::Utc));
    let failure = (!running).then(|| thread["error"].as_str().map(str::to_string)).flatten();
    let resume_at = failure
        .as_ref()
        .and_then(|_| items.iter().rposition(|i| matches!(i, Item::Failed(_))))
        .filter(|at| items[at + 1..].iter().all(|i| matches!(i, Item::Notice(_))));
    // A folded run of steps that was opened or closed changed its height: the list measures that row again.
    if let Some(row) = cx.global_mut::<crate::state::Devd>().steps_toggled.take()
        && row < shell.transcript.list.item_count()
    {
        shell.transcript.list.splice(row..row + 1, 1);
    }
    if shell.transcript.resume_at != resume_at {
        let rows = shell.transcript.list.item_count();
        for at in [shell.transcript.resume_at, resume_at].into_iter().flatten() {
            if at < rows {
                shell.transcript.list.splice(at..at + 1, 1);
            }
        }
        shell.transcript.resume_at = resume_at;
    }
    let stopped = failure
        .filter(|_| resume_at.is_none())
        .map(|error| {
            div().w_full().max_w(px(size::READING)).mx_auto().pb(px(10.0)).child(
                ui::Banner::new(Tone::Danger)
                    .title("This conversation stopped")
                    .body(div().line_clamp(4).child(error))
                    .action(resume_button("thread.resume.bottom", id))
                    .render(cx),
            )
        });

    let live = shell
        .transcript
        .writing
        .as_ref()
        .map(|(_, blocks)| blocks.clone());
    let key = id.to_string();
    let body = list(shell.transcript.list.clone(), move |index, _window, cx| {
        let content = match items.get(index) {
            Some(item) => draw(&key, index, item, resume_at == Some(index), cx),
            None => match &live {
                Some(blocks) => devd_writes(&format!("{key}.live"), blocks, "", true, cx),
                None => div().into_any_element(),
            },
        };
        div()
            .w_full()
            .flex()
            .justify_center()
            .px(px(24.0))
            .py(px(7.0))
            .child(div().w_full().max_w(px(size::READING)).child(content))
            .into_any_element()
    })
    .size_full();

    let chain = chain(shell, id, thread, cx);
    div()
        .flex_1()
        .min_w_0()
        .h_full()
        .flex()
        .flex_col()
        .child(head)
        .children(chain)
        .child(div().flex_1().min_h_0().child(body))
        .child(
            // Pinned to the column's width, as the transcript rows are, so the box keeps its place and its width
            // whatever sits above it (a chain opened or closed must not move it).
            div()
                .flex_none()
                .w_full()
                .px(px(24.0))
                .pb(px(14.0))
                .pt(px(8.0))
                .children(stopped)
                .children(waiting_rows(id, &data, cx))
                .children(running.then(|| activity_line(doing.clone(), glimpse.clone(), turn_began, cx)))
                .child(composer(
                    shell,
                    if running {
                        "A message now is read at its next step"
                    } else {
                        "Enter sends · Shift+Enter adds a line"
                    },
                    running,
                    cx,
                )),
        )
        .into_any_element()
}

/// The team behind this conversation, as a chain under its heading: every agent in the run, joined by what each waits for.
///
/// A conversation belongs to a run in one of two ways: it is the one the person started the run from, or it is one
/// of the agents' own. Either way the whole run is shown, with the agent whose conversation this is marked.
fn chain(
    shell: &mut Shell,
    id: &str,
    thread: &Value,
    cx: &mut Context<Shell>,
) -> Option<AnyElement> {
    let t = theme(cx).clone();
    // The conversation's own record may not say which run it belongs to; the list of conversations always does.
    let run_id = thread["team"]["run"]
        .as_str()
        .map(str::to_string)
        .or_else(|| {
            devd(cx)
                .sync
                .watch(&q::threads())
                .list("")
                .iter()
                .find(|th| th["id"].as_str() == Some(id))
                .and_then(|th| th["team"]["run"].as_str().map(str::to_string))
        })
        .or_else(|| {
            devd(cx)
                .sync
                .watch(&q::runs())
                .list("")
                .iter()
                .find(|r| r["thread_id"].as_str() == Some(id))
                .and_then(|r| r["id"].as_str().map(str::to_string))
        })?;
    let run = devd(cx).sync.watch(&q::run(&run_id));
    let data = run.data.clone()?;
    let tasks: Vec<Value> = data["tasks"].as_array().cloned().unwrap_or_default();
    if tasks.is_empty() {
        return None;
    }
    let open = !shell.chain_collapsed;
    let go = run_id.clone();
    let header = div()
        .id("chain.header")
        .h(px(36.0))
        .flex()
        .flex_none()
        .items_center()
        .gap(px(10.0))
        .px(px(20.0))
        .cursor_pointer()
        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
            this.chain_collapsed = !this.chain_collapsed;
            cx.notify();
        }))
        .child(icon(
            if open {
                "chevron-down"
            } else {
                "chevron-right"
            },
            13.0,
            t.faint,
        ))
        .child(icon("team", 14.0, t.muted))
        .child(
            div()
                .text_size(px(size::TEXT_SM))
                .font_weight(FontWeight::MEDIUM)
                .child("The team"),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_size(px(size::TEXT_SM))
                .text_color(t.muted)
                .child(ui::Clip::new(format!(
                    "{} · {}",
                    ui::flow::summary(&tasks),
                    ui::flow::status_line(
                        data["run"]["state"].as_str().unwrap_or("running"),
                        &tasks
                    )
                ))),
        )
        .child(
            div()
                .id("chain.open")
                .flex()
                .items_center()
                .gap(px(5.0))
                .px(px(8.0))
                .py(px(3.0))
                .rounded(px(size::RADIUS))
                .cursor_pointer()
                .text_size(px(size::TEXT_SM))
                .text_color(t.muted)
                .hover(|s| s.bg(t.hover))
                .child("Open run")
                .child(icon("arrow-right", 12.0, t.faint))
                .on_click(cx.listener(move |this, ev: &ClickEvent, _, cx| {
                    cx.stop_propagation();
                    let _ = ev;
                    this.go(Route::Runs(Some(go.clone())), cx)
                })),
        );
    let mut column = div()
        .flex_none()
        .flex()
        .flex_col()
        .border_b_1()
        .border_color(t.line)
        .child(header);
    if open {
        column = column.child(
            div().px(px(20.0)).pb(px(12.0)).child(
                div()
                    .id("chain.scroll")
                    .max_h(px(300.0))
                    .overflow_y_scroll()
                    .child(ui::flow::graph("chain", &tasks, Some(id), cx)),
            ),
        );
    }
    Some(column.into_any_element())
}

thread_local! {
    /// When each running tool call was first drawn, so a long one can say how long.
    static RUNNING_SINCE: std::cell::RefCell<HashMap<(String, usize), std::time::Instant>> = std::cell::RefCell::new(HashMap::new());
}

/// Seconds a tool call has been seen running.
fn running_for(thread: &str, index: usize) -> Option<u64> {
    RUNNING_SINCE.with(|m| {
        Some(
            m.borrow_mut()
                .entry((thread.to_string(), index))
                .or_insert_with(std::time::Instant::now)
                .elapsed()
                .as_secs(),
        )
    })
}

/// The call is no longer running: stop counting it.
fn forget_running(thread: &str, index: usize) -> Option<u64> {
    RUNNING_SINCE.with(|m| m.borrow_mut().remove(&(thread.to_string(), index)));
    None
}

/// While devd works: what it is doing now, and how long since the person last spoke. It sits at the foot of
/// the reply, above the message box, for the whole turn: a step running ("Running go test"), the model thinking
/// (with the latest of its thought, faint), or the words being written. Nothing about a long turn should look
/// like a hang.
fn activity_line(doing: String, glimpse: Option<String>, since: Option<chrono::DateTime<chrono::Utc>>, cx: &App) -> AnyElement {
    let t = theme(cx);
    let took = since.map(|at| took((chrono::Utc::now() - at).num_seconds().max(0) as u64));
    div()
        .w_full()
        .max_w(px(size::READING))
        .mx_auto()
        .pb(px(10.0))
        .flex()
        .items_center()
        .gap(px(10.0))
        .text_size(px(size::TEXT_SM))
        .child(live_dot("thread.activity", Tone::Accent, cx))
        .child(div().flex_none().max_w(px(420.0)).text_color(t.muted).font_weight(FontWeight::MEDIUM).child(ui::Clip::new(doing)))
        .children(took.map(|took| div().flex_none().font_family(mono()).text_size(px(size::TEXT_XS)).text_color(t.faint).child(took)))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .italic()
                .text_color(t.faint)
                .children(glimpse.map(|g| ui::Clip::tail(g.split_whitespace().collect::<Vec<_>>().join(" ")))),
        )
        .into_any_element()
}

/// Messages written while the agent was working, waiting for its next step: each on a line with a way to
/// take it back before it is read.
fn waiting_rows(thread: &str, data: &Value, cx: &App) -> Vec<AnyElement> {
    let t = theme(cx);
    data["waiting"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[])
        .iter()
        .enumerate()
        .map(|(n, m)| {
            let (thread, id) = (thread.to_string(), m["id"].as_i64().unwrap_or_default());
            let text = m["text"].as_str().unwrap_or_default().split_whitespace().collect::<Vec<_>>().join(" ");
            div()
                .w_full()
                .max_w(px(size::READING))
                .mx_auto()
                .pb(px(8.0))
                .flex()
                .items_center()
                .gap(px(8.0))
                .text_size(px(size::TEXT_SM))
                .child(div().flex_none().text_color(t.faint).child("Waiting"))
                .child(div().flex_1().min_w_0().text_color(t.muted).child(ui::Clip::new(text)))
                .child(
                    Button::new(ui::nid("queued.remove", n), "Remove")
                        .kind(ButtonKind::Ghost)
                        .on_click(move |_, _, cx| {
                            devd(cx).sync.mutate(
                                Mutation::new("chat.unqueue", json!({ "thread": thread, "id": id })).invalidating("chat"),
                            );
                        }),
                )
                .into_any_element()
        })
        .collect()
}

/// A duration as a person says it: "45s", "7m 17s", "1h 3m".
fn took(secs: u64) -> String {
    match secs {
        0..=59 => format!("{secs}s"),
        60..=3599 => format!("{}m {}s", secs / 60, secs % 60),
        _ => format!("{}h {}m", secs / 3600, (secs % 3600) / 60),
    }
}

/// Pick a stopped conversation back up. Any message does that, from where it stopped; this one asks for nothing new.
fn resume_button(id: &'static str, thread: &str) -> Button {
    let thread = thread.to_string();
    Button::new(id, "Resume").icon("play").kind(ButtonKind::Primary).on_click(move |_, _, cx| {
        devd(cx).sync.mutate(
            Mutation::new("chat.send", json!({ "thread": thread, "message": "Carry on from where you stopped." }))
                .invalidating("chat"),
        );
    })
}

/// One item of a transcript. `resume` marks the failure that stopped the conversation: it says so and offers Resume.
fn draw(thread: &str, index: usize, item: &Item, resume: bool, cx: &App) -> AnyElement {
    let t = theme(cx);
    match item {
        Item::You { text, pending } => div()
            .flex()
            .justify_end()
            .child(
                div()
                    .max_w(px(560.0))
                    .px(px(13.0))
                    .py(px(9.0))
                    .rounded(px(size::RADIUS_L))
                    .bg(t.raised)
                    .border_1()
                    .border_color(t.line)
                    .text_size(px(size::TEXT_MD))
                    .line_height(px(21.0))
                    .opacity(if *pending { 0.6 } else { 1.0 })
                    .child(md::linked(&format!("{thread}.{index}.you"), text, cx)),
            )
            .into_any_element(),
        Item::Devd { blocks, model } => {
            devd_writes(&format!("{thread}.{index}"), blocks, model, false, cx)
        }
        Item::Tool {
            name,
            input,
            ok,
            detail,
        } => {
            let phase = match ok {
                Some(true) => Phase::Done,
                Some(false) => Phase::Failed,
                None => Phase::Running,
            };
            let said = toolname::describe(name, input, phase);
            // What only looked recedes; what changed something, and what failed, is read at full strength.
            let quiet = phase == Phase::Done && toolname::only_looks(name, input);
            // A call still running says how long: silence is the one thing a feed must not do.
            let waiting = if phase == Phase::Running {
                running_for(thread, index)
            } else {
                forget_running(thread, index)
            };
            let glyph = match said.kind {
                "file" => "file",
                "search" => "search",
                "terminal" => "terminal",
                "git" => "branch",
                "browser" => "globe",
                "team" => "team",
                "plan" => "check",
                "server" => "machine",
                _ => "bolt",
            };
            let mark: AnyElement = match phase {
                Phase::Running => {
                    live_dot(ui::nid("tool.live", index), Tone::Info, cx).into_any_element()
                }
                Phase::Done => icon(glyph, 13.0, t.faint).into_any_element(),
                Phase::Failed => icon("x", 13.0, t.danger).into_any_element(),
            };
            // What was done reads as prose; what it was done to is the code.
            let mut line = div()
                .flex()
                .items_center()
                .gap(px(8.0))
                .pl(px(2.0))
                .text_size(px(size::TEXT))
                .child(
                    div()
                        .w(px(16.0))
                        .flex()
                        .flex_none()
                        .justify_center()
                        .child(mark),
                )
                .child(
                    div()
                        .flex_none()
                        .text_color(if phase == Phase::Failed {
                            t.danger
                        } else if quiet {
                            t.faint
                        } else {
                            t.muted
                        })
                        .child(said.action),
                )
                .child({
                    let object = div()
                        .min_w_0()
                        .font_family(mono())
                        .text_size(px(size::TEXT_SM))
                        .text_color(t.text);
                    // A file the agent touched opens on a click. The path it was given is used, not the
                    // shortened name shown, so a file in a subfolder is found.
                    let root = ui::links::root(cx);
                    let target = if said.kind == "file" {
                        ["path", "file", "from", "to"]
                            .iter()
                            .filter_map(|k| input[*k].as_str())
                            .chain(std::iter::once(said.object.as_str()))
                            .find_map(|p| ui::links::resolve_token(p, root.as_deref()))
                    } else {
                        None
                    };
                    match target {
                        Some(link) => {
                            let hover = t.info;
                            object
                                .id(ui::nid("tool.open", index))
                                .cursor_pointer()
                                .hover(move |s| s.text_color(hover))
                                .on_click(move |_, _, cx| ui::links::open(&link, cx))
                                .child(ui::Clip::new(said.object))
                                .into_any_element()
                        }
                        None => object.child(ui::Clip::new(said.object)).into_any_element(),
                    }
                });
            if phase == Phase::Failed && !detail.is_empty() {
                line = line.child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_size(px(size::TEXT_SM))
                        .text_color(t.faint)
                        .child(ui::Clip::new(detail.clone())),
                );
            }
            if let Some(took) = waiting.filter(|s| *s >= 15) {
                line = line.child(
                    div()
                        .flex_none()
                        .font_family(mono())
                        .text_size(px(size::TEXT_XS))
                        .text_color(if took >= 120 { t.warn } else { t.faint })
                        .child(format!("still running · {}", ui::age(took))),
                );
            }
            line.into_any_element()
        }
        Item::Notice(text) => div()
            .flex()
            .items_center()
            .gap(px(8.0))
            .text_size(px(size::TEXT_SM))
            .text_color(t.muted)
            .child(icon("info", 13.0, t.faint))
            .child(text.clone())
            .into_any_element(),
        Item::Failed(text) => {
            let words = md::linked(&format!("{thread}.{index}.failed"), text, cx);
            let banner = ui::Banner::new(Tone::Danger).body(words);
            let banner = if resume {
                banner
                    .title("This conversation stopped")
                    .action(resume_button("thread.resume", thread))
            } else {
                banner
            };
            banner.render(cx).into_any_element()
        }
        Item::Steps(steps) => {
            let key = format!("{thread}:{index}");
            let open = cx.try_global::<crate::state::Devd>().is_some_and(|d| d.open_steps.contains(&key));
            let listed: Vec<(&str, &Value, bool)> = steps
                .iter()
                .filter_map(|s| match s {
                    Item::Tool { name, input, ok, .. } => Some((name.as_str(), input, ok.unwrap_or(true))),
                    _ => None,
                })
                .collect();
            let failed = listed.iter().any(|(_, _, ok)| !ok);
            let line = devd_app_core::toolname::summary(&listed);
            let toggle = key.clone();
            let head = div()
                .id(ui::nid("steps.head", index))
                .flex()
                .items_start()
                .gap(px(6.0))
                .pl(px(2.0))
                .text_size(px(size::TEXT))
                .line_height(px(20.0))
                .text_color(if failed { t.muted } else { t.faint })
                .cursor_pointer()
                .hover(|s| s.opacity(0.8))
                .child(div().w(px(16.0)).h(px(20.0)).flex().flex_none().items_center().justify_center().child(icon(if open { "chevron-down" } else { "chevron-right" }, 12.0, t.faint)))
                .child(div().flex_1().min_w_0().child(line))
                .on_click(move |_, _, cx| {
                    let devd = cx.global_mut::<crate::state::Devd>();
                    if !devd.open_steps.remove(&toggle) {
                        devd.open_steps.insert(toggle.clone());
                    }
                    devd.steps_toggled = Some(index);
                    cx.refresh_windows();
                });
            let mut column = div().flex().flex_col().gap(px(10.0)).child(head);
            if open {
                for (k, step) in steps.iter().enumerate() {
                    column = column.child(draw(thread, 1_000_000 + index * 100 + k, step, false, cx));
                }
            }
            column.into_any_element()
        }
        Item::Changes { event, files, undone } => {
            let n = files.len();
            let names: Vec<String> = files
                .iter()
                .take(3)
                .map(|(_, path)| path.rsplit('/').next().unwrap_or(path).to_string())
                .collect();
            let more = n.saturating_sub(3);
            let list = format!("{}{}", names.join(", "), if more > 0 { format!(" and {more} more") } else { String::new() });
            let plural = if n == 1 { "" } else { "s" };
            let mut row = div()
                .flex()
                .items_center()
                .gap(px(8.0))
                .pl(px(2.0))
                .text_size(px(size::TEXT))
                .child(div().w(px(16.0)).flex().flex_none().justify_center().child(icon(if *undone { "refresh" } else { "file" }, 13.0, t.faint)))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_color(t.faint)
                        .child(if *undone { format!("Undid the changes to {n} file{plural}: {list}") } else { format!("Changed {n} file{plural}: {list}") }),
                );
            if !*undone {
                let (thread, event) = (thread.to_string(), *event);
                row = row.child(
                    Button::new(ui::nid("changes.undo", index), "Undo")
                        .kind(ButtonKind::Ghost)
                        .on_click(move |_, _, cx| {
                            devd(cx).sync.mutate(
                                Mutation::new("chat.undo", json!({ "thread": thread, "event": event })).invalidating("chat"),
                            );
                        }),
                );
            }
            row.into_any_element()
        }
        Item::Waiting { summary, .. } => ui::Banner::new(Tone::Warn)
            .title("Waiting for your approval")
            .text(if summary.is_empty() { "devd will carry on once you answer.".to_string() } else { summary.clone() })
            .render(cx)
            .into_any_element(),
    }
}

/// devd's words: reading text at reading width, with the model named once, quietly.
fn devd_writes(id: &str, blocks: &[Block], model: &str, live: bool, cx: &App) -> AnyElement {
    let t = theme(cx);
    let mut column = div()
        .flex()
        .flex_col()
        .gap(px(6.0))
        .text_size(px(size::TEXT_MD))
        .line_height(px(22.0))
        .child(md::document(id, blocks, cx));
    if live {
        column = column.child(
            div()
                .flex()
                .items_center()
                .gap(px(6.0))
                .child(live_dot("writing", Tone::Accent, cx))
                .child(
                    div()
                        .text_size(px(size::TEXT_XS))
                        .text_color(t.faint)
                        .child("writing"),
                ),
        );
    } else if !model.is_empty() {
        column = column.child(
            div()
                .font_family(mono())
                .text_size(px(10.5))
                .text_color(t.faint)
                .child(model.to_string()),
        );
    }
    column.into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(kind: &str, data: Value) -> Value {
        json!({ "kind": kind, "data": data })
    }

    #[test]
    fn time_taken_reads_as_a_person_says_it() {
        assert_eq!(took(45), "45s");
        assert_eq!(took(437), "7m 17s");
        assert_eq!(took(3780), "1h 3m");
    }

    #[test]
    fn finished_steps_in_a_row_fold_into_one_and_a_running_one_stays_out() {
        let call = |id: &str| event("assistant", json!({ "model": "m", "content": [{ "type": "tool_use", "id": id, "name": "read_file", "input": { "path": "a.rs" } }] }));
        let done = |id: &str| event("tool_result", json!({ "tool_use_id": id, "is_error": false, "content": "{}" }));
        let items = fold(&[event("user", json!({ "text": "go" })), call("a"), done("a"), call("b"), done("b"), call("c")], &[]);
        assert!(matches!(&items[1], Item::Steps(steps) if steps.len() == 2), "{items:?}");
        assert!(matches!(&items[2], Item::Tool { ok: None, .. }), "the running step is what is happening now: {items:?}");
    }

    #[test]
    fn the_model_is_named_once_per_reply_not_under_every_step() {
        let say = |text: &str| event("assistant", json!({ "model": "deepseek-flash", "content": [{ "type": "text", "text": text }] }));
        let items = fold(
            &[
                event("user", json!({ "text": "build it" })),
                say("First, the module."),
                say("Now the tests."),
                say("Done."),
                event("user", json!({ "text": "thanks" })),
                say("You're welcome."),
            ],
            &[],
        );
        let named: Vec<&str> = items
            .iter()
            .filter_map(|i| match i {
                Item::Devd { model, .. } => Some(model.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(named, ["", "", "deepseek-flash", "deepseek-flash"], "under the last step of each turn only");
    }

    #[test]
    fn a_tool_call_and_its_result_are_one_item() {
        let items = fold(
            &[
                event("user", json!({ "text": "what changed?" })),
                event(
                    "assistant",
                    json!({ "model": "m", "content": [
                        { "type": "text", "text": "Checking the working tree." },
                        { "type": "tool_use", "id": "c1", "name": "git_status", "input": { "repo": "D:\\www\\shop" } },
                    ]}),
                ),
                event(
                    "tool_result",
                    json!({ "tool_use_id": "c1", "is_error": false, "content": "{\"ok\":true,\"result\":{\"outcome\":{\"message\":\"3 changed files\"}}}" }),
                ),
                event(
                    "assistant",
                    json!({ "model": "m", "content": [{ "type": "text", "text": "Three files changed." }] }),
                ),
            ],
            &[],
        );
        assert_eq!(items.len(), 4);
        assert_eq!(
            items[2],
            Item::Tool {
                name: "git_status".into(),
                input: json!({ "repo": "D:\\www\\shop" }),
                ok: Some(true),
                detail: "3 changed files".into()
            }
        );
    }

    #[test]
    fn a_failed_call_carries_the_reason_and_an_unanswered_one_is_still_running() {
        let items = fold(
            &[
                event(
                    "assistant",
                    json!({ "content": [
                        { "type": "tool_use", "id": "a", "name": "read_file", "input": { "path": "missing.rs" } },
                        { "type": "tool_use", "id": "b", "name": "run_task", "input": { "task": "test" } },
                    ]}),
                ),
                event(
                    "tool_result",
                    json!({ "tool_use_id": "a", "is_error": true, "content": "{\"error\":{\"code\":\"not_found\",\"message\":\"missing.rs does not exist\\nmore\"}}" }),
                ),
            ],
            &[],
        );
        assert_eq!(
            items[0],
            Item::Tool {
                name: "read_file".into(),
                input: json!({ "path": "missing.rs" }),
                ok: Some(false),
                detail: "missing.rs does not exist".into()
            }
        );
        assert!(matches!(&items[1], Item::Tool { ok: None, name, .. } if name == "run_task"));
    }

    #[test]
    fn notices_errors_and_undecided_approvals_are_shown_and_empty_text_is_not() {
        let items = fold(
            &[
                event(
                    "assistant",
                    json!({ "content": [{ "type": "text", "text": "  " }] }),
                ),
                event("notice", json!({ "text": "Paused after 40 steps." })),
                event(
                    "error",
                    json!({ "message": "the provider refused the key" }),
                ),
                event(
                    "approval",
                    json!({ "capability": "fs.read", "request_id": "old", "plan": { "summary": "read a log" } }),
                ),
                event(
                    "approval",
                    json!({ "capability": "git.push", "request_id": "r1", "plan": { "summary": "push main to origin" } }),
                ),
            ],
            &["r1"],
        );
        assert_eq!(
            items,
            vec![
                Item::Notice("Paused after 40 steps.".into()),
                Item::Failed("the provider refused the key".into()),
                Item::Waiting {
                    capability: "git.push".into(),
                    summary: "push main to origin".into()
                },
            ]
        );
    }
}

/// The name of the project a folder belongs to. A team member works in a copy of the project kept under
/// `<project>/.devd/worktrees/`; its conversation belongs to the project, not to the copy's generated name.
fn project_name(path: &str) -> String {
    let normal = path.replace('\\', "/");
    match normal.find("/.devd/worktrees/") {
        Some(at) => ui::leaf(&normal[..at]).to_string(),
        None => ui::leaf(path).to_string(),
    }
}

#[cfg(test)]
mod project_name_tests {
    use super::project_name;

    #[test]
    fn a_copy_of_a_project_is_named_as_the_project() {
        assert_eq!(
            project_name(r"D:\devd\www\welcome\cadence\.devd\worktrees\6548ec1b-cron"),
            "cadence"
        );
        assert_eq!(
            project_name("/home/me/shop/.devd/worktrees/abc-api"),
            "shop"
        );
        assert_eq!(project_name(r"D:\devd\www\shop"), "shop");
    }
}
