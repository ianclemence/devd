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
use crate::screens::modal::{Field, FormSpec};
use crate::state::{devd, q};
use crate::theme::{Tone, mono, size, theme};
use crate::ui::{self, tint, Button, ButtonKind, eyebrow, icon, live_dot, markdown as md};

/// One thing a person reads in a conversation.
#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    /// What the person wrote. `pending` until devd has stored it.
    You { text: String, pending: bool },
    /// What devd wrote, parsed for drawing.
    ///
    /// `role` is who said it when the conversation is a team room: `implementer`, `reviewer`, or
    /// nothing when it is the assistant itself. The room draws a name over each bubble, the way a
    /// group chat does, so the reader knows who is speaking without opening a conversation per agent.
    Devd {
        blocks: Vec<Block>,
        model: String,
        role: Option<String>,
        /// Whether this is a teammate's turn rather than the assistant's: it is drawn as a bubble.
        bubble: bool,
        /// The conversation it was said in, when the item came from a room's merged log.
        thread: Option<String>,
    },
    /// A message said in a team room, by an agent to the room or to one teammate.
    Said {
        from: String,
        to: Option<String>,
        about: Option<String>,
        text: String,
    },
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

/// Fold a conversation's events into the items a person reads. `running` says the newest turn is still
/// being written: its model, tokens and cost are not shown until the turn is done.
pub fn fold(events: &[Value], pending: &[&str], running: bool) -> Vec<Item> {
    let mut items: Vec<Item> = Vec::new();
    // Where each tool call landed, so its result can be attached to it.
    let mut calls: HashMap<String, usize> = HashMap::new();
    // Who spoke last, so consecutive messages from one member collapse under one heading the way a
    // group chat does. A tool line between them does not change who is talking; another member
    // speaking, or the person writing, does.
    let mut speaker: Option<String> = None;
    for event in events {
        let data = &event["data"];
        let data_event_id = event["id"].as_i64().unwrap_or_default();
        match event["kind"].as_str().unwrap_or_default() {
            // devd's own reminders to the model (finish this, draw that again) are between it and the model.
            "user" if data["gate"] == json!(true) || data["drawing"] == json!(true) => {}
            // The goal is still open: devd asked the model again whether it is met.
            "user" if data["goal"] == json!(true) => {
                items.push(Item::Notice(format!(
                    "devd asked again whether the goal is met \u{b7} {} of {}",
                    data["step"].as_u64().unwrap_or(1),
                    data["of"].as_u64().unwrap_or(25)
                )));
            }
            "user" => items.push(Item::You {
                text: data["text"].as_str().unwrap_or_default().to_string(),
                pending: data["pending"] == json!(true),
            }),
            // A message said in the room: an agent to the room, or to one teammate about a task.
            "agent" => items.push(Item::Said {
                from: data["from"].as_str().unwrap_or("devd").to_string(),
                to: data["to"].as_str().map(str::to_string),
                about: data["about"].as_str().map(str::to_string),
                text: data["text"].as_str().unwrap_or_default().to_string(),
            }),
            "assistant" => {
                let model = data["model"].as_str().unwrap_or_default().to_string();
                for block in data["content"].as_array().map(Vec::as_slice).unwrap_or(&[]) {
                    match block["type"].as_str().unwrap_or_default() {
                        "text" => {
                            let text = block["text"].as_str().unwrap_or_default();
                            if !text.trim().is_empty() {
                                let role = event["role"].as_str().map(str::to_string);
                                let heading = role.is_some() && speaker != role;
                                speaker = role.clone();
                                items.push(Item::Devd {
                                    blocks: markdown::parse(text),
                                    model: model.clone(),
                                    role: if heading { role } else { None },
                                    // A teammate's turn is a bubble even when its heading is folded
                                    // into the one above it.
                                    bubble: event["role"].is_string(),
                                    // The conversation it was said in, so a bubble in a room can
                                    // open the specialist's own log when it is pressed.
                                    thread: event["thread_id"].as_str().map(str::to_string),
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
            // A run of edits is one line, not one line per edit: consecutive change rows are
            // merged into the row above, each file keeping its latest state. Three "Changed 1 file"
            // rows in a row read as history nobody asked for; what a person wants is the shape of
            // what the turn has touched, and Undo still takes back the most recent one.
            "changes" => {
                let files: Vec<(String, String)> = data["files"]
                    .as_array()
                    .map(|f| {
                        f.iter()
                            .map(|c| (c["status"].as_str().unwrap_or("M").to_string(), c["path"].as_str().unwrap_or_default().to_string()))
                            .collect()
                    })
                    .unwrap_or_default();
                if let Some(Item::Changes { event, files: seen, .. }) = items.last_mut() {
                    for (status, path) in files {
                        match seen.iter_mut().find(|(_, seen)| *seen == path) {
                            Some(slot) => slot.0 = status,
                            None => seen.push((status, path)),
                        }
                    }
                    *event = (*event).max(data_event_id);
                } else {
                    items.push(Item::Changes {
                        event: data_event_id,
                        files,
                        undone: false,
                    });
                }
            }
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
            "notice" => {
                let text = data["text"].as_str().unwrap_or_default();
                // One closing note: "not verified" says again what "left open" has just said.
                let said = matches!(items.last(), Some(Item::Notice(before)) if before.starts_with("It stopped with"));
                if !(said && text.starts_with("Finished, but not verified")) {
                    items.push(Item::Notice(text.to_string()));
                }
            }
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
    // A turn is many model calls, so naming its last step names it after every couple of sentences: the
    // label moves down the page as the reply is written, with a token count that is not final yet. The
    // person reads the model and the turn's cost when the turn is over, not while it is still arriving.
    if running {
        let after = items
            .iter()
            .rposition(|i| matches!(i, Item::You { .. }))
            .map_or(0, |at| at + 1);
        for item in &mut items[after..] {
            if let Item::Devd { model, .. } = item {
                model.clear();
            }
        }
    }
    fold_steps(items)
}

/// Two or more finished steps in a row are one line, the way a person reports them. A step still running is
/// never folded: it is what is happening now.
fn fold_steps(items: Vec<Item>) -> Vec<Item> {
    let mut out: Vec<Item> = Vec::with_capacity(items.len());
    let mut run: Vec<Item> = Vec::new();
    // What stands apart from the folded line: a step that failed, and the files a step changed. The write
    // that changed them is not shown twice, so the step before a change is its line.
    let mut apart: Vec<Item> = Vec::new();
    fn flush(run: &mut Vec<Item>, apart: &mut Vec<Item>, out: &mut Vec<Item>) {
        match run.as_slice() {
            [] => {}
            [Item::Tool { ok: Some(true), .. }] => out.append(run),
            _ => out.push(Item::Steps(std::mem::take(run))),
        }
        out.append(apart);
    }
    // Whether the agent went on after this step in the same turn: a failure it got past is part of the work, and
    // only a failure the turn ends on is something the person has to see.
    let went_on: Vec<bool> = (0..items.len())
        .map(|at| {
            items[at + 1..]
                .iter()
                .take_while(|later| !matches!(later, Item::You { .. }))
                .any(|later| matches!(later, Item::Devd { .. } | Item::Tool { ok: Some(true), .. }))
        })
        .collect();
    for (at, item) in items.into_iter().enumerate() {
        match &item {
            Item::Tool { ok: Some(true), .. } => run.push(item),
            Item::Tool { ok: Some(false), .. } if went_on[at] => run.push(item),
            Item::Tool { ok: Some(false), .. } => apart.push(item),
            Item::Changes { .. } => {
                if matches!(run.last(), Some(Item::Tool { .. })) {
                    run.pop();
                }
                apart.push(item);
            }
            _ => {
                flush(&mut run, &mut apart, &mut out);
                out.push(item);
            }
        }
    }
    flush(&mut run, &mut apart, &mut out);
    out
}

thread_local! {
    /// Whether the conversation on screen is running, so a step in progress is shown once, in the status line.
    static RUNNING_LINE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Who a row belongs to, for spacing: every row of one turn shares a value, and a new value starts
/// a new turn. The person and the assistant are different turns; the assistant and its own tool
/// steps are one.
fn turn_of(item: &Item) -> u8 {
    match item {
        Item::You { .. } => 0,
        _ => 1,
    }
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
    // A reason is read whole: cut at a word, never in the middle of one.
    if line.chars().count() <= 320 {
        return line.to_string();
    }
    let cut: String = line.chars().take(320).collect();
    let cut = cut.rsplit_once(' ').map(|(head, _)| head).unwrap_or(&cut);
    format!("{cut}…")
}

/// The conversation on screen, prepared for drawing.
pub struct Transcript {
    thread: Option<String>,
    /// How many events the items were folded from, to know when to fold again.
    events: usize,
    /// Whether the newest turn was still running when the items were folded.
    running: bool,
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
            running: false,
            items: Rc::new(Vec::new()),
            writing: None,
            list: ListState::new(0, ListAlignment::Bottom, px(600.0)),
            resume_at: None,
        }
    }

    /// Bring the prepared items up to date with the events and the live text.
    fn update(&mut self, thread: &str, events: &[Value], pending: &[&str], writing: Option<&str>, running: bool) {
        let switched = self.thread.as_deref() != Some(thread);
        let before = self.items.len() + usize::from(self.writing.is_some());
        // The turn ending changes a label without adding an event, so it is part of what is folded.
        if switched || events.len() + pending.len() != self.events || running != self.running {
            self.items = Rc::new(fold(events, pending, running));
            self.events = events.len() + pending.len();
            self.running = running;
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
    window: &mut Window,
    cx: &mut Context<Shell>,
) -> AnyElement {
    use super::pane::{CONVERSATION_MIN, NARROWEST, STRIP};
    // The strip first: it opens the pane when the assistant has just started the browser.
    let strip = super::pane::strip(shell, cx);
    // A conversation that is gone is gone: deleted from this window, from another, or by an agent. The
    // page behind it went on showing its last transcript, so a deleted conversation stayed on screen
    // while its row had already left the list. It goes back to the start instead, as New does.
    let asked = thread.is_some();
    let current = thread.filter(|id| exists(id, cx));
    if current.is_none() && asked {
        shell.history.select(Route::Assistant(None));
    }
    let main = match &current {
        Some(id) => conversation(shell, id, cx),
        None => start(shell, cx),
    };
    // A browser that has taken the window is the thing being looked at: the conversation and the
    // list step aside so the page has every pixel, and the strip stays so it can be given back.
    let maximised = shell.pane.browser_maximised && shell.pane.browser;
    // The room a pane may take is what is left after the conversation keeps its minimum. When the window is too
    // narrow for the list, the conversation and a usable pane, the list steps aside first: it is one click away,
    // the conversation is what is being read.
    let row_width = f32::from(window.viewport_size().width) - size::RAIL;
    let collapsed = cx.global::<crate::state::Devd>().sidebar_collapsed;
    let list_width = if collapsed { size::COLLAPSED } else { cx.global::<crate::state::Devd>().list_width };
    let room_with_list = row_width - list_width - STRIP - CONVERSATION_MIN;
    let show_list = !shell.pane.any() || room_with_list >= NARROWEST;
    let room = if show_list { room_with_list } else { row_width - STRIP - CONVERSATION_MIN };
    let mut row = div().size_full().flex();
    if !maximised {
        if show_list {
            row = row.child(sidebar(current.as_deref(), cx));
        }
        row = row.child(main);
    }
    row = row
        .children(super::pane::pane(shell, row_width, room, cx))
        .child(strip)
        .bg(ui::ground(cx));
    row.into_any_element()
}

/// Whether a conversation is still among the ones devd has.
///
/// The list is what the window already keeps true: a conversation leaves it the moment it is
/// deleted, here or anywhere else. A list nobody has read yet says nothing either way, so the
/// conversation is allowed to draw until the first list arrives. A conversation that is in the
/// list is taken as present: whether it is a team member's (made by its run rather than typed)
/// is that page's own business.
fn exists(id: &str, cx: &Context<Shell>) -> bool {
    // The list is read without asking for it, so merely drawing a conversation does not make the
    // window wait on a fetch; arriving at the Assistant has already asked for it.
    let Some(threads) = devd(cx).sync.peek(&q::threads()) else {
        return true;
    };
    // Nothing read yet, or a list that failed to load: not evidence of anything.
    if threads.data.is_none() || threads.error.is_some() {
        return true;
    }
    threads.list("").iter().any(|t| t["id"].as_str() == Some(id))
}

fn sidebar(current: Option<&str>, cx: &mut Context<Shell>) -> AnyElement {
    let collapsed = cx.global::<crate::state::Devd>().sidebar_collapsed;
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
    let mut own: Vec<&Value> = threads
        .list("")
        .iter()
        .filter(|th| !th["team"]["role"].is_string())
        .collect();
    // What the person pinned is at the top, under a heading of its own, in the order it was pinned.
    own.sort_by_key(|th| th["pinned_at"].as_str().map(|at| (0, at.to_string())).unwrap_or((1, String::new())));
    // Working conversations first, in their project's group. Snoozed work has its own shelf, and
    // settled work the last one: both are out of the way, still one click from being opened, and
    // never mixed into active work.
    let (mut snoozed, rest): (Vec<&Value>, Vec<&Value>) = own
        .into_iter()
        .partition(|th| snoozed_now(th));
    snoozed.sort_by(|a, b| a["snoozed_until"].as_str().cmp(&b["snoozed_until"].as_str()));
    let (mut settled, working): (Vec<&Value>, Vec<&Value>) =
        rest.into_iter().partition(|th| th["settled_at"].is_string());
    settled.sort_by(|a, b| b["settled_at"].as_str().cmp(&a["settled_at"].as_str()));
    let mut last_project: Option<String> = None;
    for thread in working {
        let project = if thread["pinned_at"].is_string() {
            Some("Pinned".to_string())
        } else {
            thread["project"].as_str().map(|p| ui::leaf(p).to_string())
        };
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
        column = column.child(thread_row(thread, current, false, false, cx));
    }
    if !snoozed.is_empty() {
        let count = snoozed.len();
        column = column.child(
            div()
                .px(px(10.0))
                .pt(px(14.0))
                .pb(px(4.0))
                .child(eyebrow(
                    if count == 1 {
                        "SNOOZED".to_string()
                    } else {
                        format!("SNOOZED · {count}")
                    },
                    cx,
                )),
        );
        for thread in snoozed {
            column = column.child(thread_row(thread, current, true, true, cx));
        }
    }
    if !settled.is_empty() {
        let count = settled.len();
        column = column.child(
            div()
                .px(px(10.0))
                .pt(px(14.0))
                .pb(px(4.0))
                .child(eyebrow(
                    if count == 1 {
                        "SETTLED".to_string()
                    } else {
                        format!("SETTLED · {count}")
                    },
                    cx,
                )),
        );
        for thread in settled {
            column = column.child(thread_row(thread, current, true, false, cx));
        }
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
    // Put away: a narrow column holding only the way back and a new conversation. The list beside
    // it keeps its own layout, so toggling does not reflow the reply you are reading.
    if collapsed {
        return div()
            .w(px(size::COLLAPSED))
            .h_full()
            .flex()
            .flex_col()
            .flex_none()
            .items_center()
            .gap(px(4.0))
            .py(px(8.0))
            .border_r_1()
            .border_color(t.line)
            .child(
                Button::icon_only("sidebar.expand", "chevron-right")
                    .kind(ButtonKind::Ghost)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.toggle_sidebar(cx))),
            )
            .child(
                Button::icon_only("sidebar.new", "plus")
                    .kind(ButtonKind::Ghost)
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                        this.run("conversation.new", window, cx)
                    })),
            )
            .into_any_element();
    }
    let wide = cx.global::<crate::state::Devd>().list_width;
    let grip = ui::grip("sidebar.grip", crate::state::Edge::List, wide, false, cx);
    div()
        .w(px(wide))
        .relative()
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
        .child(grip)
        .into_any_element()
}

/// One conversation in the list.
///
/// A parked conversation — settled or snoozed — reads as parked: dimmer, never live, and with the
/// same actions, because a parked conversation can still be opened, woken or deleted. A snoozed one
/// says when it comes back instead of how long ago it moved.
fn thread_row(
    thread: &Value,
    current: Option<&str>,
    parked: bool,
    snoozed: bool,
    cx: &mut Context<Shell>,
) -> AnyElement {
    let t = theme(cx).clone();
    let id = thread["id"].as_str().unwrap_or_default().to_string();
    let said = devd_app_core::status::of(thread["state"].as_str().unwrap_or_default());
    let marker: AnyElement =
        if parked || said.label.is_empty() || said.level == devd_app_core::status::Level::Neutral {
            div().size(px(7.0)).flex_none().into_any_element()
        } else {
            ui::state_mark(
                gpui::ElementId::Name(format!("thread.live.{id}").into()),
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
    // A conversation that started a run is named by what the run is for, in the list as well as in
    // its header: "A team run (id 6548ec1b-…)" is a database key wearing a sentence, and the same
    // conversation must not have two names in two places. A member's own conversation keeps its
    // task's title. The run is found the same two ways `run_of` finds it, because a room's own row
    // does not always carry which run it belongs to.
    let run_here = thread["team"]["run"]
        .as_str()
        .map(str::to_string)
        .or_else(|| {
            devd(cx)
                .sync
                .watch(&q::runs())
                .list("")
                .iter()
                .find(|r| r["thread_id"].as_str() == Some(id.as_str()))
                .and_then(|r| r["id"].as_str().map(str::to_string))
        });
    let title = match (run_here, thread["team"]["role"].as_str()) {
        (Some(run), None) => devd(cx)
            .sync
            .watch(&q::runs())
            .list("")
            .iter()
            .find(|r| r["id"].as_str() == Some(run.as_str()))
            .and_then(|r| r["goal"].as_str())
            .map(|goal| {
                let short: String = goal.chars().take(48).collect();
                if short.len() < goal.len() { format!("{short}…") } else { short }
            })
            .unwrap_or_else(|| thread["title"].as_str().unwrap_or("Conversation").to_string()),
        _ => thread["title"]
            .as_str()
            .unwrap_or("Conversation")
            .to_string(),
    };
    ui::row(
        gpui::ElementId::Name(format!("thread.{id}").into()),
        current == Some(id.as_str()),
        cx,
    )
    .group("thread.row")
    .h(px(34.0))
    .child(marker)
    .child(
        div()
            .flex_1()
            .min_w_0()
            .text_color(if parked { t.muted } else { t.text })
            .child(ui::Clip::new(title)),
    )
    .child(
        div()
            .flex_none()
            .text_size(px(size::TEXT_XS))
            .text_color(t.faint)
            .child(if snoozed {
                ui::due_at(thread["snoozed_until"].as_str().unwrap_or_default())
            } else {
                ui::ago(thread["updated_at"].as_str().unwrap_or_default())
            }),
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
    }))
    .into_any_element()
}

/// The conversation's own menu: everything about the conversation rather than the work.
///
/// It is drawn inside the header, anchored under the button that opens it, and closes on any press
/// outside it or on choosing something. The items are the capabilities the window already had, in
/// one place: fork, pin, settle, rename, the pull request link, delete.
/// What the menu needs to know about the conversation it serves.
struct PanelFacts<'a> {
    id: &'a str,
    title: &'a str,
    pinned: bool,
    settled: bool,
    snoozed: bool,
    last_event: Option<i64>,
    pr_url: Option<&'a str>,
}

fn conversation_panel(facts: PanelFacts<'_>, cx: &mut Context<Shell>) -> AnyElement {
    let PanelFacts {
        id,
        title,
        pinned,
        settled,
        snoozed,
        last_event,
        pr_url,
    } = facts;
    type Act = Box<dyn Fn(&mut Shell, &mut Window, &mut Context<Shell>)>;
    // A third field says the row is destructive: it is the only one that is, and it says so in the
    // colour rather than in the position.
    let mut items: Vec<(&'static str, bool, Act)> = Vec::new();

    if let Some(event) = last_event {
        let (tid, ttitle) = (id.to_string(), title.to_string());
        items.push((
            "Fork from here",
            false,
            Box::new(move |_, _, cx| {
                devd(cx).sync.mutate(
                    Mutation::new(
                        "chat.fork",
                        json!({ "thread": tid, "event": event, "title": format!("Fork of {ttitle}") }),
                    )
                    .invalidating("chat"),
                );
            }),
        ));
    }
    {
        let tid = id.to_string();
        items.push((
            if pinned { "Unpin" } else { "Pin to the top" },
            false,
            Box::new(move |_, _, cx| {
                devd(cx).sync.mutate(
                    Mutation::new("chat.pin_session", json!({ "thread": tid, "pinned": !pinned }))
                        .invalidating("chat"),
                );
            }),
        ));
    }
    {
        let tid = id.to_string();
        items.push((
            if settled { "Reopen" } else { "Mark as settled" },
            false,
            Box::new(move |_, _, cx| {
                devd(cx).sync.mutate(
                    Mutation::new("chat.settle", json!({ "thread": tid, "settled": !settled }))
                        .invalidating("chat"),
                );
            }),
        ));
    }
    {
        let tid = id.to_string();
        items.push((
            "Rename",
            false,
            Box::new(move |this, window, cx| {
                this.open_form(
                    FormSpec {
                        title: "Rename the conversation",
                        intro: "The name it has in the list. Nothing else changes.",
                        verb: "Rename",
                        capability: "chat.update",
                        domain: "chat",
                        fixed: json!({ "thread": tid }),
                        fields: vec![Field::text("title", "Name", "The checkout bug")],
                        busy: None,
                    },
                    window,
                    cx,
                );
            }),
        ));
    }
    if snoozed {
        let tid = id.to_string();
        items.push((
            "Wake it now",
            false,
            Box::new(move |_, _, cx| {
                devd(cx).sync.mutate(
                    Mutation::new("chat.snooze", json!({ "thread": tid })).invalidating("chat"),
                );
            }),
        ));
    } else {
        for (label, until) in [
            ("Snooze for an hour", snooze_in(1)),
            ("Snooze until tomorrow", tomorrow_morning()),
        ] {
            let tid = id.to_string();
            items.push((
                label,
                false,
                Box::new(move |_, _, cx| {
                    devd(cx).sync.mutate(
                        Mutation::new(
                            "chat.snooze",
                            json!({ "thread": tid, "until": until.clone() }),
                        )
                        .invalidating("chat"),
                    );
                }),
            ));
        }
    }
    {
        let tid = id.to_string();
        items.push((
            if pr_url.is_some() {
                "Change the pull request"
            } else {
                "Link a pull request"
            },
            false,
            Box::new(move |this, window, cx| {
                this.open_form(
                    FormSpec {
                        title: "Link a pull request",
                        intro: "The review this conversation is about. It shows beside the title and in the list.",
                        verb: "Link",
                        capability: "chat.link_pr",
                        domain: "chat",
                        fixed: json!({ "thread": tid }),
                        fields: vec![Field::text(
                            "url",
                            "Address",
                            "https://github.com/owner/repo/pull/1",
                        )],
                        busy: None,
                    },
                    window,
                    cx,
                );
            }),
        ));
    }
    if pr_url.is_some() {
        let tid = id.to_string();
        items.push((
            "Unlink the pull request",
            false,
            Box::new(move |_, _, cx| {
                devd(cx).sync.mutate(
                    Mutation::new("chat.link_pr", json!({ "thread": tid })).invalidating("chat"),
                );
            }),
        ));
    }
    {
        let (tid, ttitle) = (id.to_string(), title.to_string());
        items.push((
            "Delete",
            true,
            Box::new(move |this, window, cx| {
                this.delete_conversation(tid.clone(), ttitle.clone(), true, window, cx);
            }),
        ));
    }

    let t = theme(cx).clone();
    let (text, danger, hover) = (t.text, t.danger, t.hover);
    let mut rows = div().flex().flex_col().gap(px(1.0));
    for (n, (label, destructive, action)) in items.into_iter().enumerate() {
        rows = rows.child(
            div()
                .id(ui::nid("conversation.menu", n))
                .h(px(30.0))
                .px(px(10.0))
                .flex()
                .items_center()
                .rounded(px(6.0))
                .cursor_pointer()
                .hover(move |s| s.bg(hover))
                .child(
                    div()
                        .text_size(px(size::TEXT_SM))
                        .text_color(if destructive { danger } else { text })
                        .child(label),
                )
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    this.conversation_menu = false;
                    action(this, window, cx);
                })),
        );
    }

    div()
        .absolute()
        .top(px(44.0))
        // Lined up with the right edge of the header's last button, so the menu looks attached to it.
        .right(px(24.0))
        .w(px(232.0))
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
        .on_mouse_down_out(cx.listener(|this, _, _, cx| {
            this.conversation_menu = false;
            cx.notify();
        }))
        .child(rows)
        .into_any_element()
}

/// Whether a conversation is put aside right now.
///
/// The stored instant is compared, not the string: a snooze a person set in another time zone is
/// still a moment, and comparing text would put it on the shelf for the wrong length of time.
fn snoozed_now(thread: &Value) -> bool {
    thread["snoozed_until"]
        .as_str()
        .and_then(|at| chrono::DateTime::parse_from_rfc3339(at).ok())
        .is_some_and(|at| at > chrono::Utc::now())
}

/// An instant some hours from now, in the form `chat.snooze` takes.
fn snooze_in(hours: i64) -> String {
    (chrono::Utc::now() + chrono::Duration::hours(hours)).to_rfc3339()
}

/// Nine in the morning on the next day, in the machine's own time zone.
fn tomorrow_morning() -> String {
    use chrono::TimeZone;
    let tomorrow = chrono::Local::now().date_naive() + chrono::Duration::days(1);
    tomorrow
        .and_hms_opt(9, 0, 0)
        .and_then(|nine| chrono::Local.from_local_datetime(&nine).single())
        .map(|at| at.to_rfc3339())
        .unwrap_or_else(|| snooze_in(12))
}

/// What a linked pull request is called in a conversation's header: `PR 42`, `MR 7`, or the words
/// alone when the address does not end in a number.
fn pr_label(url: &str) -> String {
    let tail = url.trim_end_matches('/').rsplit('/').next().unwrap_or_default();
    let kind = if url.contains("merge_requests") { "MR" } else { "PR" };
    if !tail.is_empty() && tail.chars().all(|c| c.is_ascii_digit()) {
        format!("{kind} {tail}")
    } else {
        "Pull request".to_string()
    }
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
    cx: &mut Context<Shell>,
) -> AnyElement {
    super::compose::view(shell, crate::shell::ComposerScreen::Assistant, hint, size::READING, cx)
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
        .child(composer(shell, "Enter sends · Shift+Enter adds a line", cx))
        .into_any_element()
}

/// The run a conversation belongs to, when it belongs to one: its origin, or an agent's own.
fn run_of(shell: &Shell, id: &str, cx: &mut Context<Shell>) -> Option<String> {
    let _ = shell;
    run_of_self(id, cx)
}

/// The same, for a caller that has no shell at hand.
pub fn run_of_self(id: &str, cx: &App) -> Option<String> {
    devd(cx)
        .sync
        .watch(&q::threads())
        .list("")
        .iter()
        .find(|th| th["id"].as_str() == Some(id))
        .and_then(|th| th["team"]["run"].as_str().map(str::to_string))
        .or_else(|| {
            devd(cx)
                .sync
                .watch(&q::runs())
                .list("")
                .iter()
                .find(|r| r["thread_id"].as_str() == Some(id))
                .and_then(|r| r["id"].as_str().map(str::to_string))
        })
}

/// Whether a run can still take a direction: a finished one would refuse it, and the person would
/// get an error for having spoken in a conversation they were reading.
pub fn run_open(run: &str, cx: &App) -> bool {
    devd(cx)
        .sync
        .watch(&q::runs())
        .list("")
        .iter()
        .find(|r| r["id"].as_str() == Some(run))
        .map(|r| !matches!(r["state"].as_str().unwrap_or_default(), "done" | "failed" | "cancelled"))
        .unwrap_or(false)
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
    // The conversation a run was started from is the **room**: the person's messages, the
    // planner's and every specialist's, in one transcript. A specialist's own conversation is
    // still that specialist's: it keeps its task, its branch and its own turns, and points back at
    // the room. Reading Backend's chat and being shown what Implementer said is not a room, it is a
    // wrong answer to "what did Backend do".
    let room = run_of(shell, id, cx);
    let seat = room.as_ref().map(|_| devd(cx).sync.watch(&q::team_log(id)));
    let room_data = seat.as_ref().and_then(|s| s.data.as_deref());
    let origin = room_data
        .and_then(|d| d["origin"].as_str())
        .map(str::to_string);
    let in_room = origin.as_deref() == Some(id);
    let room_events: Vec<Value> = if in_room {
        room_data
            .and_then(|d| d["events"].as_array().cloned())
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    let events: &[Value] = if room_events.is_empty() { events } else { &room_events };
    let pending: Vec<&str> = data["pending"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[])
        .iter()
        .filter_map(|p| p["id"].as_str().or(p["request_id"].as_str()))
        .collect();
    // Running in this window's server, or in another process (a terminal, a script, CI) that writes to the
    // same record: the record says so, and is what to believe.
    let running = data["running"] == json!(true) || data["thread"]["state"].as_str() == Some("running");
    shell
        .transcript
        .update(id, events, &pending, writing.as_deref(), running);

    let thread = &data["thread"];
    // The composer's effort is this conversation's, not the window's. Opening a conversation reads the
    // level it has been running at, so the chip cannot say "Thinking off" above an agent that is
    // thinking at the default, and a conversation set to Off does not come back as Medium.
    // The Assistant's own box carries this: Home's box has no conversation to follow.
    let held = &mut shell.chat_compose;
    if held.thread.as_deref() != Some(id) {
        held.thread = Some(id.to_string());
        held.effort = super::compose::level(thread["effort"].as_str());
    }
    // What this conversation's working tree has changed, for the branch beside the title and the
    // count on the Changes action. A conversation with no project, or one whose project is not a
    // repository, simply has nothing to say here.
    let changes = devd(cx)
        .sync
        .watch(&q::thread_changes(id))
        .data
        .clone()
        .map(|v| (*v).clone())
        .unwrap_or(Value::Null);
    let branch = changes["branch"].as_str().map(str::to_string);
    let dirty = changes["clean"] == json!(false);
    let changed = changes["files"].as_array().map(Vec::len).unwrap_or(0);
    if running {
        devd(cx).chat_running.set(true);
    }
    // The provider's prompt count includes the cache hits. Shown as one number it reads as fresh
    // work, which it is not: on a tool-using conversation most of it is the same prefix read again
    // at a tenth of the price. Say each part, so the fresh work is what the eye lands on.
    // Relative paths in this conversation's replies are read against its project.
    cx.global_mut::<crate::state::Devd>().link_root =
        thread["project"].as_str().map(std::path::PathBuf::from);
    let project = thread["project"].as_str().map(project_name);
    // A room is the run, so it is titled with what the run is for rather than with the id it was
    // given: a person reading the conversation should see the goal, not a database key. A
    // specialist's conversation keeps its own title — its task — because that is what it is.
    let run_data = if in_room {
        room
            .as_deref()
            .and_then(|run| devd(cx).sync.watch(&q::run(run)).data.clone())
    } else {
        None
    };
    let title = match run_data
        .as_ref()
        .and_then(|d| d["run"]["goal"].as_str())
        .map(str::to_string)
    {
        Some(goal) => {
            let short: String = goal.chars().take(90).collect();
            if short.len() < goal.len() { format!("{short}…") } else { short }
        }
        None => thread["title"]
            .as_str()
            .unwrap_or("Conversation")
            .to_string(),
    };

    // The heading: the title, then one quiet line of facts, then the actions. Facts are words in the page's secondary colour
    // separated by a middle dot, not badges; an error is not squeezed in here but given its own line below.
    let mut facts: Vec<String> = Vec::new();
    if let Some(project) = project {
        facts.push(project);
    }
    // Who is in the room and where the work stands: the roster is what makes a run a team rather
    // than a queue, and it belongs beside the goal it is working toward. In a member's own
    // conversation the useful facts are the task and its branch, which are already here.
    if let Some(data) = run_data.as_ref() {
        let mut roles: Vec<String> = data["tasks"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or(&[])
            .iter()
            .filter_map(|t| t["role"].as_str())
            .map(role_title)
            .collect();
        roles.sort();
        roles.dedup();
        if let Some(state) = data["run"]["state"].as_str() {
            facts.push(state.to_string());
        }
        if !roles.is_empty() {
            facts.push(roles.join(", "));
        }
    }
    if let Some(snap) = devd(cx).sync.peek(&q::threads()) {
        let origin = snap
            .list("")
            .iter()
            .find(|t| t["id"].as_str() == Some(id))
            .and_then(|t| t["forked_from"].as_str().map(str::to_string))
            .and_then(|from| snap.list("").iter().find(|t| t["id"].as_str() == Some(from.as_str())).and_then(|t| t["title"].as_str().map(str::to_string)));
        if let Some(origin) = origin {
            facts.push(format!("forked from {}", origin.chars().take(40).collect::<String>()));
        }
    }
    let mut actions = div().flex().items_center().gap(px(2.0));
    if running {
        let stop = id.to_string();
        actions = actions
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
    // Pin keeps the conversation at the top of the list; fork branches it into a new one from where it stands.
    let marks = devd(cx)
        .sync
        .peek(&q::threads())
        .and_then(|snap| snap.list("").iter().find(|t| t["id"].as_str() == Some(id)).cloned());
    let pinned = marks.as_ref().is_some_and(|m| m["pinned_at"].is_string());
    let settled_now = thread["settled_at"].is_string();
    // Where this conversation's files are: its own worktree, or the project's folder.
    let in_worktree = thread["workdir"]
        .as_str()
        .filter(|w| !w.is_empty())
        .map(str::to_string);
    if thread["project"].as_str().is_some() || in_worktree.is_some() {
        let tid = id.to_string();
        let tooltip = if in_worktree.is_some() {
            "Working in its own worktree — click to go back to the project"
        } else {
            "Give this conversation its own worktree, so its changes cannot collide with other work"
        };
        let back = in_worktree.is_some();
        actions = actions.child(
            div()
                .id("conversation.worktree.wrap")
                .tooltip(ui::tip(tooltip))
                .child(
                    Button::icon_only("conversation.worktree", "branch")
                        .kind(if in_worktree.is_some() {
                            ButtonKind::Secondary
                        } else {
                            ButtonKind::Ghost
                        })
                        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                            if back {
                                devd(cx).sync.mutate(
                                    Mutation::new(
                                        "chat.worktree",
                                        json!({ "thread": tid, "action": "detach" }),
                                    )
                                    .invalidating("chat"),
                                );
                            } else {
                                this.confirm(
                                    "Give this conversation its own worktree?".to_string(),
                                    "Its tools will work in a separate folder on its own branch, in the same repository, so this conversation's changes cannot collide with another piece of work. Nothing is deleted, and going back keeps the folder.".to_string(),
                                    "Start a worktree",
                                    false,
                                    Mutation::new(
                                        "chat.worktree",
                                        json!({ "thread": tid, "action": "create" }),
                                    )
                                    .invalidating("chat"),
                                    window,
                                    cx,
                                );
                            }
                        })),
                ),
        );
    }
    // Review what this conversation changed, in the project's own Git surface: staging, per-file
    // diffs, the commit, pull and push. A second changes screen would only drift from that one.
    if let Some(dir) = thread["workdir"]
        .as_str()
        .filter(|w| !w.is_empty())
        .or_else(|| thread["project"].as_str())
        .map(str::to_string)
    {
        let open = dir.clone();
        actions = actions.child(
            div()
                .id("conversation.changes.wrap")
                .tooltip(ui::tip(match changed {
                    0 => "Review the changes, then commit and push".to_string(),
                    1 => "1 file changed — review it, then commit and push".to_string(),
                    n => format!("{n} files changed — review them, then commit and push"),
                }))
                .child(
                    div()
                        .relative()
                        .child(
                            Button::icon_only("conversation.changes", "diff")
                                .kind(if dirty {
                                    ButtonKind::Secondary
                                } else {
                                    ButtonKind::Ghost
                                })
                                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                    this.go(
                                        Route::project(
                                            open.clone(),
                                            devd_app_core::project::ProjectTab::Changes,
                                        ),
                                        cx,
                                    )
                                })),
                        )
                        // A dot, not a badge: the count is in the tooltip and the Changes screen.
                        .children(dirty.then(|| {
                            div()
                                .absolute()
                                .top(px(2.0))
                                .right(px(2.0))
                                .size(px(6.0))
                                .rounded_full()
                                .bg(t.warn)
                        })),
                ),
        );
    }
    // The last step of the loop, on a branch of its own: a pushed branch becomes a review.
    if in_worktree.is_some() && thread["pr_url"].as_str().is_none_or(str::is_empty) {
        let tid = id.to_string();
        actions = actions.child(
            div()
                .id("conversation.pr.open.wrap")
                .tooltip(ui::tip("Open a pull request for this branch, once it is pushed"))
                .child(
                    Button::icon_only("conversation.pr.open", "external")
                        .kind(ButtonKind::Ghost)
                        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                            this.confirm(
                                "Open a pull request for this branch?".to_string(),
                                "The branch must already be pushed — a review is of commits the host can see. Its title and body are filled from the commits, and the pull request is linked to this conversation.".to_string(),
                                "Open the pull request",
                                false,
                                Mutation::new("chat.pr", json!({ "thread": tid })).invalidating("chat"),
                                window,
                                cx,
                            );
                        })),
                ),
        );
    }
    let last_event = events.last().and_then(|e| e["id"].as_i64());
    // Everything about the conversation rather than the work — pin, settle, fork, rename, the pull
    // request link, delete — lives in one menu, so the bar keeps only what a person does while
    // working: stop, where the files are, the changes, and the review.
    actions = actions.child(
        div()
            .id("conversation.more.wrap")
            // No tooltip while the menu is open: it would sit on top of the menu it opened.
            .when(!shell.conversation_menu, |d| d.tooltip(ui::tip("Conversation menu")))
            .child(
                Button::icon_only("conversation.more", "more")
                    .kind(if shell.conversation_menu {
                        ButtonKind::Secondary
                    } else {
                        ButtonKind::Ghost
                    })
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.conversation_menu = !this.conversation_menu;
                        cx.notify();
                    })),
            ),
    );
    let head = div()
        .h(px(48.0))
        .px(px(20.0))
        .flex()
        .flex_none()
        .items_center()
        .gap(px(16.0))
        .relative()
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
                        .child(ui::Clip::new(title.clone())),
                )
                // A member's own conversation is not the room, but the room is one press away: a
                // specialist's log is where its work happened, and the conversation it belongs to is
                // where the team talked about it.
                .children(origin.as_deref().filter(|_| !in_room).map(|org| {
                    let (target, hover) = (org.to_string(), t.hover);
                    div()
                        .id("conversation.room")
                        .flex_none()
                        .flex()
                        .items_center()
                        .gap(px(4.0))
                        .px(px(7.0))
                        .py(px(2.0))
                        .rounded(px(size::RADIUS))
                        .cursor_pointer()
                        .text_size(px(size::TEXT_XS))
                        .text_color(t.muted)
                        .hover(move |s| s.bg(hover))
                        .child("In the room")
                        .child(icon("arrow-right", 11.0, t.faint))
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.go(Route::Assistant(Some(target.clone())), cx);
                        }))
                }))
                // The branch, when this conversation works on one of its own: a quiet fact beside
                // the title, the same weight as the pull request link under it.
                .children(branch.map(|b| {
                    div()
                        .flex_none()
                        .text_size(px(size::TEXT_XS))
                        .text_color(t.faint)
                        .child(b)
                }))
                // The pull request this conversation is about, when one is linked: a quiet link
                // beside the title, never a badge competing with it.
                .children(thread["pr_url"].as_str().map(|url| {
                    let (url, label) = (url.to_string(), pr_label(url));
                    let (hover, muted, faint) = (t.text, t.muted, t.faint);
                    div()
                        .id("conversation.pr")
                        .flex_none()
                        .flex()
                        .items_center()
                        .gap(px(4.0))
                        .text_size(px(size::TEXT_SM))
                        .text_color(muted)
                        .cursor_pointer()
                        .hover(move |s| s.text_color(hover))
                        .child(ui::icon("external", 11.0, faint))
                        .child(label)
                        .tooltip(ui::tip("The pull request this conversation is about"))
                        .on_click(move |_, _, _| {
                            let _ = open::that(&url);
                        })
                }))
                .children((!facts.is_empty()).then(|| {
                    div()
                        .min_w_0()
                        .overflow_hidden()
                        .text_size(px(size::TEXT_SM))
                        .text_color(t.faint)
                        .child(ui::Clip::new(facts.join("  ·  ")))
                })),
        )
        .child(actions);


    // A conversation that stopped on an error says so once. Its last failure in the transcript is where that is read,
    // beside the message box, and that is where it is resumed from. Only when the transcript holds no such failure is
    // it said above instead.
    let items = shell.transcript.items.clone();
    // What the status line says devd is doing: thinking, writing, or the step that is running.
    let is_writing = writing.as_deref().is_some_and(|w| !w.trim().is_empty());
    // The thought itself is never put in the status line. It is there for the person to open, and
    // until they do the line says only that the model is thinking.
    let thought = match (&thinking, is_writing) {
        (Some((_, text)), false) if !text.trim().is_empty() => Some(text.clone()),
        _ => None,
    };
    let doing = match (&thinking, is_writing) {
        (Some(_), false) => "Thinking".to_string(),
        (_, true) => "Writing".to_string(),
        // What is being done right now, said as the step's own row would say it: "Reading a.rs", "Running cargo test".
        _ => {
            let step = items.iter().rev().find_map(|i| match i {
                Item::Tool { name, input, ok: None, .. } => {
                    let said = toolname::describe(name, input, Phase::Running);
                    Some(if said.object.is_empty() { said.action } else { format!("{} {}", said.action, said.object) })
                }
                _ => None,
            });
            step.unwrap_or_else(|| "Working".to_string())
        }
    };
    // The step that is running is this line; its own row in the transcript would say the same thing twice.
    RUNNING_LINE.with(|c| c.set(running));
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
                Some(blocks) => devd_writes(&format!("{key}.live"), blocks, "", TurnStyle::default(), true, cx),
                None => div().into_any_element(),
            },
        };
        // A new speaker starts a turn, and a turn is worth more air than a step inside one. Every
        // row used to get the same 7px above and below, so the fifth tool call of a turn and the
        // first sentence of the next one looked equally far apart and the conversation read as a
        // single block.
        let opens_turn = match (items.get(index.saturating_sub(1)), items.get(index)) {
            (None, _) => true,
            (Some(previous), Some(current)) => turn_of(previous) != turn_of(current),
            _ => false,
        };
        div()
            .w_full()
            .flex()
            .justify_center()
            .px(px(24.0))
            .pt(px(if opens_turn { 22.0 } else { 7.0 }))
            .pb(px(7.0))
            // `min_w_0`: in a narrow column a row shrinks with it. Without it a row is at least as wide as its
            // content, and a centred row wider than the column was cut off at the left edge.
            .child(div().w_full().min_w_0().max_w(px(size::READING)).child(content))
            .into_any_element()
    })
    .size_full();

    // A goal that is open: what it is, how many times devd has asked, and the three things a person does with it.
    let goal_banner = data["goal"]
        .as_object()
        .filter(|g| matches!(g.get("state").and_then(Value::as_str), Some("active" | "paused")))
        .map(|g| {
            let active = g.get("state").and_then(Value::as_str) == Some("active");
            let (toggle, clear) = (id.to_string(), id.to_string());
            let objective = g.get("objective").and_then(Value::as_str).unwrap_or_default().to_string();
            let steps = g.get("steps").and_then(Value::as_u64).unwrap_or(0);
            div().px(px(24.0)).pt(px(10.0)).child(div().w_full().max_w(px(size::READING)).mx_auto().child(
                ui::Banner::new(if active { Tone::Info } else { Tone::Warn })
                    .title(if active { "Working toward a goal" } else { "Goal paused" })
                    .text(format!("{objective} \u{b7} asked {steps} of 25 times"))
                    .action(
                        Button::new("goal.toggle", if active { "Pause" } else { "Resume" })
                            .icon(if active { "stop" } else { "play" })
                            .kind(ButtonKind::Secondary)
                            .on_click(move |_, _, cx| {
                                devd(cx).sync.mutate(
                                    Mutation::new("chat.goal", json!({ "thread": toggle, "action": if active { "pause" } else { "resume" } }))
                                        .invalidating("chat"),
                                );
                            }),
                    )
                    .action(Button::new("goal.clear", "Clear").kind(ButtonKind::Ghost).on_click(move |_, _, cx| {
                        devd(cx).sync.mutate(
                            Mutation::new("chat.goal", json!({ "thread": clear, "action": "clear" })).invalidating("chat"),
                        );
                    }))
                    .render(cx),
            ))
        });
    div()
        .relative()
        .flex_1()
        .min_w_0()
        .h_full()
        .flex()
        .flex_col()
        .child(head)
        .children(goal_banner)
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
                .children(running.then(|| {
                    // One row says what is happening. When there is a thought to read, that same row opens it.
                    let open = cx.global::<crate::state::Devd>().thinking_open.contains(id);
                    activity_line(id, doing.clone(), turn_began, thought.clone(), open, cx)
                }))
                .child(composer(
                    shell,
                    if running {
                        "Sends at the next step"
                    } else {
                        "Enter sends · Shift+Enter adds a line"
                    },
                    cx,
                )),
        )
        // The conversation's menu is the last child of the column, drawn over the transcript rather
        // than under it: it belongs to the header but it has to be above everything, and painting
        // later is the only way to be.
        .children(shell.conversation_menu.then(|| {
            conversation_panel(
                PanelFacts {
                    id,
                    title: &title,
                    pinned,
                    settled: settled_now,
                    snoozed: snoozed_now(thread),
                    last_event,
                    pr_url: thread["pr_url"].as_str(),
                },
                cx,
            )
        }))
        .into_any_element()
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

/// While devd works: one row at the foot of the reply, in the column and aligned with the steps above it, saying
/// what it is doing and for how long. A step running ("Running go test"), the words being written, or the model
/// thinking; while it thinks the row opens the thought beneath itself, so there is never a second "Thinking".
fn activity_line(
    thread: &str,
    doing: String,
    since: Option<chrono::DateTime<chrono::Utc>>,
    thought: Option<String>,
    open: bool,
    cx: &App,
) -> AnyElement {
    let t = theme(cx);
    let took = since.map(|at| took((chrono::Utc::now() - at).num_seconds().max(0) as u64));
    let mut row = div()
        .id("thread.activity")
        .flex()
        .items_center()
        .gap(px(6.0))
        .pl(px(2.0))
        .min_h(px(24.0))
        .overflow_hidden()
        .text_size(px(size::TEXT_SM))
        .child(div().w(px(16.0)).flex().flex_none().justify_center().child(ui::motion::spinner("thread.spinner", t.accent)))
        .child(div().min_w_0().text_color(t.muted).child(doing))
        .children(took.map(|took| div().flex_none().text_color(t.faint).child(format!("· {took}"))));
    if thought.is_some() {
        let key = thread.to_string();
        row = row
            .cursor_pointer()
            .hover(|s| s.opacity(0.8))
            .child(div().flex_none().child(icon(if open { "chevron-down" } else { "chevron-right" }, 12.0, t.faint)))
            .on_click(move |_, _, cx| {
                let devd = cx.global_mut::<crate::state::Devd>();
                if !devd.thinking_open.remove(&key) {
                    devd.thinking_open.insert(key.clone());
                }
                cx.refresh_windows();
            });
    }
    div()
        .w_full()
        .max_w(px(size::READING))
        .mx_auto()
        .pb(px(8.0))
        .flex()
        .flex_col()
        .items_start()
        .gap(px(4.0))
        .child(row)
        .children(thought.filter(|_| open).map(|text| {
            div()
                .id(ui::nid("thinking.body", 0))
                .w_full()
                .max_h(px(220.0))
                .overflow_y_scroll()
                .ml(px(9.0))
                .pl(px(14.0))
                .border_l_1()
                .border_color(t.line_strong)
                .text_size(px(size::TEXT_SM))
                .line_height(px(19.0))
                .text_color(t.faint)
                .child(text)
        }))
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
            .min_w_0()
            .child(
                div()
                    .min_w_0()
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
        Item::Devd {
            blocks,
            model,
            role,
            bubble,
            thread: said_in,
        } => devd_writes(
            &format!("{thread}.{index}"),
            blocks,
            model,
            TurnStyle {
                role: role.as_deref(),
                bubble: *bubble,
                // A teammate's bubble in a room is a door: pressing it opens the conversation that
                // message came from, so the detail is one press away instead of a search in the list.
                open: said_in.as_deref().filter(|said_in| *said_in != thread),
            },
            false,
            cx,
        ),
        // One teammate speaking to the room: a name, who it is for, and what it is about, because
        // in a room the reply target is half the message.
        Item::Said {
            from,
            to,
            about,
            text,
        } => {
            let name = if from == "devd" {
                "devd".to_string()
            } else {
                role_title(from)
            };
            let initials: String = name
                .split_whitespace()
                .filter_map(|w| w.chars().next())
                .take(2)
                .collect();
            let mut head = div()
                .flex()
                .items_center()
                .gap(px(6.0))
                .child(ui::face(initials, role_tone(from), false, cx))
                .child(
                    div()
                        .text_size(px(size::TEXT_XS))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(t.muted)
                        .child(name),
                );
            if let Some(to) = to {
                head = head.child(
                    div()
                        .text_size(px(size::TEXT_XS))
                        .text_color(t.accent)
                        .child(format!("@{to}")),
                );
            }
            if let Some(about) = about {
                head = head.child(
                    div()
                        .text_size(px(size::TEXT_XS))
                        .text_color(t.faint)
                        .child(format!("· {about}")),
                );
            }
            div()
                .flex()
                .flex_col()
                .gap(px(6.0))
                .p(px(11.0))
                .rounded(px(size::RADIUS_L))
                .bg(t.hover)
                .child(head)
                .child(
                    div()
                        .text_size(px(size::TEXT_MD))
                        .line_height(px(22.0))
                        .text_color(t.text)
                        .child(text.clone()),
                )
                .into_any_element()
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
            // While the turn runs, the status line above the message box says what is running, with its clock.
            if phase == Phase::Running && RUNNING_LINE.with(|c| c.get()) {
                return div().into_any_element();
            }
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
            // What was done reads as prose; what it was done to is the code. A step that failed
            // wraps, because its reason is the whole point of the line: a message cut at the
            // column's edge is a message that cannot be acted on.
            let mut line = div()
                .flex()
                .items_start()
                .gap(px(8.0))
                .pl(px(2.0))
                .text_size(px(size::TEXT))
                .child(
                    div()
                        .w(px(16.0))
                        .flex()
                        .flex_none()
                        .justify_center()
                        .mt(px(2.0))
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
                        .min_w_0().line_clamp(2)
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
                                .child(said.object)
                                .into_any_element()
                        }
                        None => object.child(said.object).into_any_element(),
                    }
                });
            if phase == Phase::Failed && !detail.is_empty() {
                // Why it failed goes on its own line under the step, at the row's full width. Beside a long
                // command it was squeezed to a column one letter wide.
                let reason = div()
                    .pl(px(26.0))
                    .min_w_0()
                    .text_size(px(size::TEXT_SM))
                    .line_height(px(19.0))
                    .text_color(t.muted)
                    .child(detail.clone());
                return div()
                    .flex()
                    .flex_col()
                    .gap(px(4.0))
                    .px(px(10.0))
                    .py(px(8.0))
                    .rounded(px(size::RADIUS_L))
                    .bg(tint(t.danger, 0.08))
                    .border_1()
                    .border_color(tint(t.danger, 0.22))
                    .child(line)
                    .child(reason)
                    .into_any_element();
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
        // A notice is prose: it wraps inside the transcript's column, and the icon sits with its
        // first line rather than centred against four lines of text.
        Item::Notice(text) => div()
            .flex()
            .items_start()
            .gap(px(8.0))
            .text_size(px(size::TEXT_SM))
            .text_color(t.muted)
            .child(div().flex_none().mt(px(1.0)).child(icon("info", 13.0, t.faint)))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .line_height(px(19.0))
                    .child(text.clone()),
            )
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
                    // Undo puts files back: it removes what an agent wrote, so it is said in the colour of that.
                    Button::new(ui::nid("changes.undo", index), "Undo")
                        .kind(ButtonKind::Danger)
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
/// A role's name as a person reads it: `implementer` is "Implementer", `qa` is "QA".
fn role_title(role: &str) -> String {
    if role == "qa" {
        return "QA".to_string();
    }
    let words = role.replace(['-', '_'], " ");
    let mut chars = words.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// A role's face colour, stable across a room: the same member keeps the same colour in every
/// bubble and every run, so a long transcript is scanned by identity rather than read by name.
///
/// The palette is the four tones that mean nothing bad. Red and amber are reserved for failure and
/// warning, and a person must never have to work out whether green means "passed" or "is the QA
/// engineer".
fn role_tone(role: &str) -> Tone {
    let mut hash: u32 = 0;
    for byte in role.bytes() {
        hash = hash.wrapping_mul(31).wrapping_add(u32::from(byte));
    }
    match hash % 4 {
        0 => Tone::Accent,
        1 => Tone::Info,
        2 => Tone::Ok,
        _ => Tone::Neutral,
    }
}

/// How a turn is drawn: who said it (when it opens a speaker's run of messages), whether it is a
/// member's bubble rather than the assistant's own voice, and where pressing the name leads.
#[derive(Default, Clone, Copy)]
struct TurnStyle<'a> {
    role: Option<&'a str>,
    bubble: bool,
    open: Option<&'a str>,
}

fn devd_writes(
    id: &str,
    blocks: &[Block],
    model: &str,
    style: TurnStyle<'_>,
    live: bool,
    cx: &App,
) -> AnyElement {
    let TurnStyle { role, bubble, open } = style;
    let t = theme(cx);
    let mut column = div()
        .flex()
        .flex_col()
        .gap(px(6.0))
        .text_size(px(size::TEXT_MD))
        .line_height(px(22.0));
    // In a team room every bubble says who is speaking: a face and a name, the way a group chat
    // does, so the room reads without opening a conversation per agent.
    if let Some(role) = role {
        let name = role_title(role);
        let initials: String = name
            .split_whitespace()
            .filter_map(|w| w.chars().next())
            .take(2)
            .collect();
        let mut head = div()
            .id(gpui::ElementId::Name(format!("{id}.head").into()))
            .flex()
            .items_center()
            .gap(px(6.0))
            .child(ui::face(initials, role_tone(role), false, cx))
            .child(
                div()
                    .text_size(px(size::TEXT_XS))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(t.muted)
                    .child(name),
            );
        // The name is the way in: it says whose words these are and where the rest of them live.
        if let Some(open) = open {
            let (target, hover) = (open.to_string(), t.hover);
            head = head
                .px(px(4.0))
                .py(px(1.0))
                .ml(px(-4.0))
                .rounded(px(size::RADIUS))
                .cursor_pointer()
                .hover(move |s| s.bg(hover))
                .on_click(move |_, _, cx| {
                    cx.global_mut::<crate::state::Devd>().opened_thread = Some(target.clone());
                });
        }
        column = column.child(head);
    }
    // The assistant speaks as a member of the room too: a face and its name at the start of each reply, and nothing else
    // beside it. A reply that is still being written has its face ringed and says so there, in the
    // place the eye already is, instead of under the text.
    let own_head = role.is_none() && (live || !model.is_empty());
    if own_head {
        let mut head = div()
            .flex()
            .items_center()
            .gap(px(6.0))
            .child(ui::face("d", Tone::Accent, live, cx))
            .child(
                div()
                    .text_size(px(size::TEXT_XS))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(t.muted)
                    .child("devd"),
            );
        if live {
            head = head.child(div().text_size(px(size::TEXT_XS)).text_color(t.accent).child("writing…"));
        }
        column = column.child(head);
    }
    column = column.child(md::document(id, blocks, cx));
    if live && role.is_some() {
        column = column.child(
            div()
                .flex()
                .items_center()
                .gap(px(6.0))
                .child(live_dot("writing", Tone::Accent, cx))
                .child(div().text_size(px(size::TEXT_XS)).text_color(t.faint).child("writing")),
        );
    }
    // A teammate's turn is a bubble, not a wall: the card is what makes a room read as a room.
    // The assistant's own turns stay flat, because they are the conversation's own voice.
    if bubble {
        column = column
            .p(px(12.0))
            .rounded(px(size::RADIUS_L))
            .bg(t.raised);
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
    fn devd_asking_again_about_a_goal_is_a_notice_not_something_the_person_said() {
        let items = fold(
            &[
                event("user", json!({ "text": "/goal make the tests pass" })),
                event("user", json!({ "text": "[devd] The goal is still open: make the tests pass", "from": "devd", "goal": true, "step": 3, "of": 25 })),
            ],
            &[],
            false,
        );
        assert!(matches!(&items[0], Item::You { .. }));
        assert_eq!(items[1], Item::Notice("devd asked again whether the goal is met · 3 of 25".into()));
        assert_eq!(items.len(), 2);
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
        let items = fold(&[event("user", json!({ "text": "go" })), call("a"), done("a"), call("b"), done("b"), call("c")], &[], false);
        assert!(matches!(&items[1], Item::Steps(steps) if steps.len() == 2), "{items:?}");
        assert!(matches!(&items[2], Item::Tool { ok: None, .. }), "the running step is what is happening now: {items:?}");
    }

    #[test]
    fn a_write_is_one_line_a_failure_stays_visible_and_reads_still_fold() {
        let call = |id: &str, name: &str| event("assistant", json!({ "model": "m", "content": [{ "type": "tool_use", "id": id, "name": name, "input": {} }] }));
        let done = |id: &str, bad: bool| event("tool_result", json!({ "tool_use_id": id, "is_error": bad, "content": "{}" }));
        let changes = event("changes", json!({ "files": [{ "status": "M", "path": "a.rs" }] }));
        let items = fold(
            &[
                event("user", json!({ "text": "go" })),
                call("a", "read_file"), done("a", false),
                call("b", "read_file"), done("b", false),
                call("c", "write_file"), done("c", false), changes,
                call("d", "run_command"), done("d", true),
            ],
            &[],
            false,
        );
        assert!(matches!(&items[1], Item::Steps(s) if s.len() == 2), "{items:?}");
        assert!(matches!(&items[2], Item::Changes { .. }), "the write is its changes line, once: {items:?}");
        assert!(matches!(&items[3], Item::Tool { ok: Some(false), .. }), "{items:?}");
        assert_eq!(items.len(), 4);
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
            false,
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

    /// Three edits in a row are one line, not three: a file keeps its latest status, and Undo still
    /// points at the most recent change.
    #[test]
    fn a_run_of_edits_is_one_line_with_each_file_at_its_latest_state() {
        let changes = |id: i64, files: Value| {
            json!({ "id": id, "kind": "changes", "data": { "files": files } })
        };
        let items = fold(
            &[
                event("user", json!({ "text": "go" })),
                changes(1, json!([{ "status": "M", "path": "a.go" }])),
                changes(2, json!([{ "status": "M", "path": "b.go" }, { "status": "A", "path": "a.go" }])),
                changes(3, json!([{ "status": "M", "path": "c.go" }])),
            ],
            &[],
            false,
        );
        let rows: Vec<&Item> = items.iter().filter(|i| matches!(i, Item::Changes { .. })).collect();
        assert_eq!(rows.len(), 1, "one line for the run of edits: {items:?}");
        match rows[0] {
            Item::Changes { event, files, .. } => {
                assert_eq!(*event, 3, "Undo takes back the most recent change");
                assert_eq!(
                    files,
                    &vec![
                        ("A".to_string(), "a.go".to_string()),
                        ("M".to_string(), "b.go".to_string()),
                        ("M".to_string(), "c.go".to_string())
                    ],
                    "each file once, at its latest state"
                );
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn a_failure_the_agent_got_past_folds_away_and_one_that_ends_the_turn_stays() {
        let call = |id: &str| event("assistant", json!({ "model": "m", "content": [{ "type": "tool_use", "id": id, "name": "run_command", "input": {} }] }));
        let done = |id: &str, bad: bool| event("tool_result", json!({ "tool_use_id": id, "is_error": bad, "content": "{}" }));
        let say = event("assistant", json!({ "model": "m", "content": [{ "type": "text", "text": "Done." }] }));
        let go = event("user", json!({ "text": "go" }));
        let gate = event("user", json!({ "text": "[devd] finish", "gate": true, "objections": 1 }));
        let got_past = fold(&[go.clone(), call("a"), done("a", true), gate, say], &[], false);
        assert!(matches!(&got_past[1], Item::Steps(s) if s.len() == 1), "{got_past:?}");
        assert_eq!(got_past.len(), 3, "devd's reminder to the model is not shown: {got_past:?}");
        let ended = fold(&[go, call("a"), done("a", true)], &[], false);
        assert!(matches!(&ended[1], Item::Tool { ok: Some(false), .. }), "{ended:?}");
    }

    /// A member keeps their colour: the same role always reads the same, and the palette never
    /// borrows the two tones that mean something is wrong.
    #[test]
    fn a_members_colour_is_theirs_and_never_a_warning() {
        assert_eq!(role_tone("backend"), role_tone("backend"), "a member keeps their colour");
        assert!(
            role_tone("backend") != role_tone("reviewer"),
            "two members are told apart without reading their names"
        );
        for role in ["backend", "reviewer", "qa", "implementer", "planner", "docs", "designer"] {
            let tone = role_tone(role);
            assert!(
                !matches!(tone, Tone::Danger | Tone::Warn),
                "{role} must not wear a colour that means a problem"
            );
        }
    }

    /// A room is one conversation: a teammate's turn is tagged with the role that said it, and a
    /// message one teammate addresses to another carries who said it and who it is for.
    #[test]
    fn a_room_tags_each_turn_with_its_speaker_and_keeps_what_a_message_answers() {
        let events = [
            event("user", json!({ "text": "build it" })),
            json!({
                "kind": "assistant",
                "role": "backend",
                "data": { "model": "deepseek-flash", "content": [{ "type": "text", "text": "Done: the scheduler." }] }
            }),
            json!({
                "kind": "agent",
                "role": "backend",
                "data": { "from": "backend", "to": "reviewer", "about": "scheduler", "text": "Can you look at the retry loop?" }
            }),
            json!({
                "kind": "assistant",
                "role": null,
                "data": { "model": "deepseek-flash", "content": [{ "type": "text", "text": "The reviewer has it." }] }
            }),
        ];
        let items = fold(&events, &[], false);
        // The assistant's own turn keeps no role: it is the room's voice, not a member of it.
        assert!(
            matches!(&items[1], Item::Devd { role: Some(role), bubble: true, .. } if role == "backend"),
            "{items:?}"
        );
        assert!(
            matches!(&items[2], Item::Said { from, to: Some(to), about: Some(about), .. }
                if from == "backend" && to == "reviewer" && about == "scheduler"),
            "{items:?}"
        );
        assert!(matches!(&items[3], Item::Devd { role: None, bubble: false, .. }), "{items:?}");
    }

    /// A member who says three things in a row is one voice in the room, not three: the heading is
    /// stated once and the rest are its bubbles.
    #[test]
    fn consecutive_messages_from_one_member_share_one_heading() {
        let said = |role: &str, text: &str| {
            json!({
                "kind": "assistant",
                "role": role,
                "data": { "model": "deepseek-flash", "content": [{ "type": "text", "text": text }] }
            })
        };
        let items = fold(
            &[
                said("implementer", "The host is Windows."),
                // A tool line between two messages does not change who is talking.
                json!({ "kind": "tool_result", "data": { "tool_use_id": "t1", "content": "ok" } }),
                said("implementer", "Committing the code first."),
                said("reviewer", "Looks right."),
                said("implementer", "Final checks."),
            ],
            &[],
            false,
        );
        let headings: Vec<Option<&str>> = items
            .iter()
            .filter_map(|i| match i {
                Item::Devd { role, bubble: true, .. } => Some(role.as_deref()),
                _ => None,
            })
            .collect();
        assert_eq!(
            headings,
            vec![Some("implementer"), None, Some("reviewer"), Some("implementer")],
            "one heading per speaker run: {items:?}"
        );
    }

    #[test]
    fn a_turn_still_being_written_names_its_model_only_after_it_is_done() {
        let say = |text: &str| event("assistant", json!({ "model": "deepseek-flash", "content": [{ "type": "text", "text": text }] }));
        let events = [
            event("user", json!({ "text": "build it" })),
            say("First, the module."),
            say("Now the tests."),
        ];
        // While the turn runs, a label under the latest step would show a model and a cost for a reply
        // that is still arriving, and move down the page on every step.
        let running = fold(&events, &[], true);
        assert!(
            running.iter().all(|i| !matches!(i, Item::Devd { model, .. } if !model.is_empty())),
            "a running turn carries no label: {running:?}"
        );
        // A finished turn before it keeps the label it earned.
        let with_earlier = fold(
            &[
                event("user", json!({ "text": "the first thing" })),
                say("The first answer."),
                event("user", json!({ "text": "the second thing" })),
                say("Still working on it."),
            ],
            &[],
            true,
        );
        let named: Vec<&str> = with_earlier
            .iter()
            .filter_map(|i| match i {
                Item::Devd { model, .. } => Some(model.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(named, ["deepseek-flash", ""], "only the running turn is quiet");
        // The same events, finished, name the turn.
        let done = fold(&events, &[], false);
        assert!(matches!(done.last(), Some(Item::Devd { model, .. }) if model == "deepseek-flash"));
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
            false,
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
            false,
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
            false,
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
