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
use gpui::{div, list, prelude::*, px, AnyElement, App, ClickEvent, Context, FontWeight, ListAlignment, ListOffset, ListState, Window};
use serde_json::{json, Value};

use crate::shell::Shell;
use crate::state::{devd, q};
use crate::theme::{mono, size, theme, Tone};
use crate::ui::{self, eyebrow, icon, live_dot, markdown as md, pill, Button};

/// One thing a person reads in a conversation.
#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    /// What the person wrote. `pending` until devd has stored it.
    You { text: String, pending: bool },
    /// What devd wrote, parsed for drawing.
    Devd { blocks: Vec<Block>, model: String },
    /// A tool call as the model made it and, once it has one, its result.
    /// It is put into words when it is drawn, because the words depend on how it ended.
    Tool { name: String, input: Value, ok: Option<bool>, detail: String },
    /// Something devd's harness said: a pause, a limit.
    Notice(String),
    /// A failure that ended the turn.
    Failed(String),
    /// An operation waiting for the person's approval.
    Waiting { capability: String, summary: String },
}

/// Fold a conversation's events into the items a person reads.
pub fn fold(events: &[Value], pending: &[&str]) -> Vec<Item> {
    let mut items: Vec<Item> = Vec::new();
    // Where each tool call landed, so its result can be attached to it.
    let mut calls: HashMap<String, usize> = HashMap::new();
    for event in events {
        let data = &event["data"];
        match event["kind"].as_str().unwrap_or_default() {
            // devd's own reminder before it lets a turn end is not something the person said.
            "user" if data["gate"] == json!(true) => {
                let n = data["objections"].as_u64().unwrap_or(1);
                items.push(Item::Notice(format!("devd sent it back to finish: {n} thing{} still open", if n == 1 { " was" } else { "s were" })));
            }
            "user" => items.push(Item::You { text: data["text"].as_str().unwrap_or_default().to_string(), pending: data["pending"] == json!(true) }),
            "assistant" => {
                let model = data["model"].as_str().unwrap_or_default().to_string();
                for block in data["content"].as_array().map(Vec::as_slice).unwrap_or(&[]) {
                    match block["type"].as_str().unwrap_or_default() {
                        "text" => {
                            let text = block["text"].as_str().unwrap_or_default();
                            if !text.trim().is_empty() {
                                items.push(Item::Devd { blocks: markdown::parse(text), model: model.clone() });
                            }
                        }
                        "tool_use" => {
                            calls.insert(block["id"].as_str().unwrap_or_default().to_string(), items.len());
                            items.push(Item::Tool { name: block["name"].as_str().unwrap_or_default().to_string(), input: block["input"].clone(), ok: None, detail: String::new() });
                        }
                        _ => {}
                    }
                }
            }
            "tool_result" => {
                let failed = data["is_error"] == json!(true);
                let text = data["content"].as_str().unwrap_or_default();
                if let Some(Item::Tool { ok, detail, .. }) = calls.get(data["tool_use_id"].as_str().unwrap_or_default()).and_then(|at| items.get_mut(*at)) {
                    *ok = Some(!failed);
                    *detail = result_line(text, failed);
                }
            }
            "notice" => items.push(Item::Notice(data["text"].as_str().unwrap_or_default().to_string())),
            "error" => items.push(Item::Failed(data["message"].as_str().unwrap_or_default().to_string())),
            // The log keeps every request ever made. Only one that is still
            // undecided is something the person is being asked.
            "approval" if pending.contains(&data["request_id"].as_str().unwrap_or_default()) => items.push(Item::Waiting {
                capability: data["capability"].as_str().unwrap_or_default().to_string(),
                summary: data["plan"]["summary"].as_str().unwrap_or_default().to_string(),
            }),
            _ => {}
        }
    }
    items
}

/// The one line of a tool result worth showing beside the call.
fn result_line(text: &str, failed: bool) -> String {
    let parsed: Value = serde_json::from_str(text).unwrap_or(Value::Null);
    let line = if failed {
        parsed["error"]["message"].as_str().or(parsed["error"].as_str()).unwrap_or(text)
    } else {
        parsed["result"]["outcome"]["message"].as_str().or(parsed["result"]["message"].as_str()).or(parsed["message"].as_str()).unwrap_or_default()
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
}

impl Transcript {
    /// Nothing open.
    pub fn new() -> Self {
        Self { thread: None, events: 0, items: Rc::new(Vec::new()), writing: None, list: ListState::new(0, ListAlignment::Bottom, px(600.0)) }
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
            self.list.scroll_to(ListOffset { item_ix: after, offset_in_item: px(0.0) });
        } else if after != before {
            // Reading the newest messages means following new ones as they
            // arrive; having scrolled back to read means staying put.
            let following = self.list.logical_scroll_top().item_ix + 12 >= before;
            // Everything from the first changed row on is laid out again.
            let keep = before.min(after).saturating_sub(1);
            self.list.splice(keep..before, after - keep);
            if following {
                self.list.scroll_to(ListOffset { item_ix: after, offset_in_item: px(0.0) });
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
pub fn render(shell: &mut Shell, thread: Option<String>, _window: &mut Window, cx: &mut Context<Shell>) -> AnyElement {
    let t = theme(cx).clone();
    div()
        .size_full()
        .flex()
        .child(sidebar(thread.as_deref(), cx))
        .child(match thread {
            Some(id) => conversation(shell, &id, cx),
            None => start(shell, cx),
        })
        .bg(t.bg)
        .into_any_element()
}

fn sidebar(current: Option<&str>, cx: &mut Context<Shell>) -> impl IntoElement {
    let t = theme(cx).clone();
    let threads = devd(cx).sync.watch(&q::threads());
    let mut column = div().flex().flex_col().gap(px(1.0)).p(px(8.0));
    if threads.is_first_load() {
        for n in 0..6 {
            column = column.child(div().h(px(44.0)).px(px(10.0)).flex().items_center().child(ui::skeleton(ui::nid("threads.skeleton", n), 150.0 + (n as f32 * 23.0) % 70.0, cx)));
        }
    }
    // A specialist's conversation belongs to its run, and is reached from there.
    let own: Vec<&Value> = threads.list("").iter().filter(|th| !th["team"]["role"].is_string()).collect();
    let mut last_project: Option<String> = None;
    for thread in own {
        let id = thread["id"].as_str().unwrap_or_default().to_string();
        let project = thread["project"].as_str().map(|p| ui::leaf(p).to_string()).unwrap_or_else(|| "No project".into());
        if last_project.as_ref() != Some(&project) {
            column = column.child(div().px(px(10.0)).pt(px(12.0)).pb(px(4.0)).child(eyebrow(project.to_uppercase(), cx)));
            last_project = Some(project);
        }
        let state = thread["state"].as_str().unwrap_or_default();
        let said = devd_app_core::status::of(state);
        // A conversation at rest has no mark: the list is quiet until something needs a look.
        let marker: AnyElement = if said.label.is_empty() || said.level == devd_app_core::status::Level::Neutral {
            div().size(px(7.0)).flex_none().into_any_element()
        } else {
            ui::state_mark(gpui::ElementId::Name(format!("thread.live.{}", thread["id"].as_str().unwrap_or_default()).into()), &said, cx)
        };
        let go = id.clone();
        column = column.child(
            ui::row(gpui::ElementId::Name(format!("thread.{id}").into()), current == Some(id.as_str()), cx)
                .h(px(34.0))
                .child(marker)
                .child(div().flex_1().min_w_0().child(ui::Clip::new(thread["title"].as_str().unwrap_or("Conversation").to_string())))
                .child(div().flex_none().text_size(px(size::TEXT_XS)).text_color(t.faint).child(ui::ago(thread["updated_at"].as_str().unwrap_or_default())))
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.go(Route::Assistant(Some(go.clone())), cx))),
        );
    }
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
                .child(Button::new("thread.new", "New").icon("plus").keys("secondary-n").on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.run("conversation.new", window, cx)))),
        )
        .child(div().id("threads").flex_1().overflow_y_scroll().child(column))
}

/// The message box, with its menus: see [`super::compose`].
fn composer(shell: &Shell, hint: &'static str, running: bool, cx: &mut Context<Shell>) -> AnyElement {
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
        .child(ui::art(cx))
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
    let t = theme(cx).clone();
    let state = devd(cx);
    let snap = state.sync.watch_log(&q::thread(id), q::thread_log());
    let writing = state.writing.get(id).cloned();
    let Some(data) = snap.data.clone() else {
        let body = match &snap.error {
            Some(failure) => ui::empty("alert", "This conversation could not be read", failure.message.clone(), cx),
            None => div().p(px(32.0)).flex().flex_col().gap(px(14.0)).children((0..5).map(|n| ui::skeleton(ui::nid("conversation.skeleton", n), 220.0 + n as f32 * 60.0, cx))),
        };
        return div().flex_1().min_w_0().h_full().child(body).into_any_element();
    };
    let events = data["events"].as_array().map(Vec::as_slice).unwrap_or(&[]);
    let pending: Vec<&str> = data["pending"].as_array().map(Vec::as_slice).unwrap_or(&[]).iter().filter_map(|p| p["id"].as_str().or(p["request_id"].as_str())).collect();
    shell.transcript.update(id, events, &pending, writing.as_deref());

    let thread = &data["thread"];
    let running = data["running"] == json!(true);
    let tokens = thread["input_tokens"].as_u64().unwrap_or(0) + thread["output_tokens"].as_u64().unwrap_or(0);
    // Relative paths in this conversation's replies are read against its project.
    cx.global_mut::<crate::state::Devd>().link_root = thread["project"].as_str().map(std::path::PathBuf::from);
    let project = thread["project"].as_str().map(|p| ui::leaf(p).to_string());
    let mut head = div()
        .h(px(44.0))
        .px(px(20.0))
        .flex()
        .flex_none()
        .items_center()
        .gap(px(10.0))
        .border_b_1()
        .border_color(t.line)
        .child(div().flex_1().min_w_0().font_weight(FontWeight::MEDIUM).child(ui::Clip::new(thread["title"].as_str().unwrap_or("Conversation").to_string())));
    if let Some(project) = project {
        head = head.child(div().flex().items_center().gap(px(5.0)).text_size(px(size::TEXT_SM)).text_color(t.muted).child(icon("folder", 13.0, t.faint)).child(project));
    }
    if tokens > 0 {
        head = head.child(div().font_family(mono()).text_size(px(size::TEXT_XS)).text_color(t.faint).child(format!("{} tokens", ui::compact(tokens))));
    }
    if running {
        let stop = id.to_string();
        head = head
            .child(div().flex().items_center().gap(px(6.0)).child(live_dot("conversation.live", Tone::Info, cx)).child(div().text_size(px(size::TEXT_SM)).text_color(t.info).child("working")))
            .child(Button::new("conversation.stop", "Stop").icon("stop").on_click(move |_, _, cx| {
                devd(cx).sync.mutate(Mutation::new("chat.stop", json!({ "thread": stop })));
            }));
    } else if let Some(error) = thread["error"].as_str() {
        head = head.child(pill("stopped on an error", Tone::Danger, cx)).child(div().max_w(px(320.0)).text_size(px(size::TEXT_SM)).text_color(t.muted).child(ui::Clip::new(error.to_string())));
    }

    let items = shell.transcript.items.clone();
    let live = shell.transcript.writing.as_ref().map(|(_, blocks)| blocks.clone());
    let key = id.to_string();
    let body = list(shell.transcript.list.clone(), move |index, _window, cx| {
        let content = match items.get(index) {
            Some(item) => draw(&key, index, item, cx),
            None => match &live {
                Some(blocks) => devd_writes(&format!("{key}.live"), blocks, "", true, cx),
                None => div().into_any_element(),
            },
        };
        div().w_full().flex().justify_center().px(px(24.0)).py(px(7.0)).child(div().w_full().max_w(px(size::READING)).child(content)).into_any_element()
    })
    .size_full();

    div()
        .flex_1()
        .min_w_0()
        .h_full()
        .flex()
        .flex_col()
        .child(head)
        .child(div().flex_1().min_h_0().child(body))
        .child(div().flex_none().px(px(24.0)).pb(px(14.0)).pt(px(8.0)).child(composer(
            shell,
            if running { "A message now is read at its next step" } else { "Enter sends · Shift+Enter adds a line" },
            running,
            cx,
        )))
        .into_any_element()
}

thread_local! {
    /// When each running tool call was first drawn, so a long one can say how long.
    static RUNNING_SINCE: std::cell::RefCell<HashMap<(String, usize), std::time::Instant>> = std::cell::RefCell::new(HashMap::new());
}

/// Seconds a tool call has been seen running.
fn running_for(thread: &str, index: usize) -> Option<u64> {
    RUNNING_SINCE.with(|m| Some(m.borrow_mut().entry((thread.to_string(), index)).or_insert_with(std::time::Instant::now).elapsed().as_secs()))
}

/// The call is no longer running: stop counting it.
fn forget_running(thread: &str, index: usize) -> Option<u64> {
    RUNNING_SINCE.with(|m| m.borrow_mut().remove(&(thread.to_string(), index)));
    None
}

fn draw(thread: &str, index: usize, item: &Item, cx: &App) -> AnyElement {
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
        Item::Devd { blocks, model } => devd_writes(&format!("{thread}.{index}"), blocks, model, false, cx),
        Item::Tool { name, input, ok, detail } => {
            let phase = match ok {
                Some(true) => Phase::Done,
                Some(false) => Phase::Failed,
                None => Phase::Running,
            };
            let said = toolname::describe(name, input, phase);
            // What only looked recedes; what changed something, and what failed, is read at full strength.
            let quiet = phase == Phase::Done && toolname::only_looks(name, input);
            // A call still running says how long: silence is the one thing a feed must not do.
            let waiting = if phase == Phase::Running { running_for(thread, index) } else { forget_running(thread, index) };
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
                Phase::Running => live_dot(ui::nid("tool.live", index), Tone::Info, cx).into_any_element(),
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
                .child(div().w(px(16.0)).flex().flex_none().justify_center().child(mark))
                .child(div().flex_none().text_color(if phase == Phase::Failed { t.danger } else if quiet { t.faint } else { t.muted }).child(said.action))
                .child({
                    let object = div().min_w_0().font_family(mono()).text_size(px(size::TEXT_SM)).text_color(t.text);
                    // A file the agent touched opens on a click. The path it was given is used, not the
                    // shortened name shown, so a file in a subfolder is found.
                    let root = ui::links::root(cx);
                    let target = if said.kind == "file" {
                        ["path", "file", "from", "to"].iter().filter_map(|k| input[*k].as_str()).chain(std::iter::once(said.object.as_str())).find_map(|p| ui::links::resolve_token(p, root.as_deref()))
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
                line = line.child(div().flex_1().min_w_0().text_size(px(size::TEXT_SM)).text_color(t.faint).child(ui::Clip::new(detail.clone())));
            }
            if let Some(took) = waiting.filter(|s| *s >= 15) {
                line = line.child(div().flex_none().font_family(mono()).text_size(px(size::TEXT_XS)).text_color(if took >= 120 { t.warn } else { t.faint }).child(format!("still running · {}", ui::age(took))));
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
        Item::Failed(text) => div()
            .p(px(12.0))
            .flex()
            .items_start()
            .gap(px(9.0))
            .rounded(px(size::RADIUS_L))
            .border_1()
            .border_color(ui::tint(t.danger, 0.35))
            .bg(ui::tint(t.danger, 0.08))
            .child(icon("alert", 15.0, t.danger))
            .child(div().flex_1().min_w_0().text_size(px(size::TEXT)).line_height(px(20.0)).child(md::linked(&format!("{thread}.{index}.failed"), text, cx)))
            .into_any_element(),
        Item::Waiting { capability, summary } => div()
            .p(px(12.0))
            .flex()
            .items_center()
            .gap(px(9.0))
            .rounded(px(size::RADIUS_L))
            .border_1()
            .border_color(ui::tint(t.warn, 0.35))
            .bg(ui::tint(t.warn, 0.07))
            .child(icon("lock", 15.0, t.warn))
            .child(div().flex_1().min_w_0().text_size(px(size::TEXT)).child(ui::Clip::new(if summary.is_empty() { format!("Waiting for your approval: {capability}") } else { summary.clone() })))
            .child(pill(capability.clone(), Tone::Warn, cx))
            .into_any_element(),
    }
}

/// devd's words: reading text at reading width, with the model named once, quietly.
fn devd_writes(id: &str, blocks: &[Block], model: &str, live: bool, cx: &App) -> AnyElement {
    let t = theme(cx);
    let mut column = div().flex().flex_col().gap(px(6.0)).text_size(px(size::TEXT_MD)).line_height(px(22.0)).child(md::document(id, blocks, cx));
    if live {
        column = column.child(div().flex().items_center().gap(px(6.0)).child(live_dot("writing", Tone::Accent, cx)).child(div().text_size(px(size::TEXT_XS)).text_color(t.faint).child("writing")));
    } else if !model.is_empty() {
        column = column.child(div().font_family(mono()).text_size(px(10.5)).text_color(t.faint).child(model.to_string()));
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
    fn a_tool_call_and_its_result_are_one_item() {
        let items = fold(&[
            event("user", json!({ "text": "what changed?" })),
            event("assistant", json!({ "model": "m", "content": [
                { "type": "text", "text": "Checking the working tree." },
                { "type": "tool_use", "id": "c1", "name": "git_status", "input": { "repo": "D:\\www\\shop" } },
            ]})),
            event("tool_result", json!({ "tool_use_id": "c1", "is_error": false, "content": "{\"ok\":true,\"result\":{\"outcome\":{\"message\":\"3 changed files\"}}}" })),
            event("assistant", json!({ "model": "m", "content": [{ "type": "text", "text": "Three files changed." }] })),
        ], &[]);
        assert_eq!(items.len(), 4);
        assert_eq!(items[2], Item::Tool { name: "git_status".into(), input: json!({ "repo": "D:\\www\\shop" }), ok: Some(true), detail: "3 changed files".into() });
    }

    #[test]
    fn a_failed_call_carries_the_reason_and_an_unanswered_one_is_still_running() {
        let items = fold(&[
            event("assistant", json!({ "content": [
                { "type": "tool_use", "id": "a", "name": "read_file", "input": { "path": "missing.rs" } },
                { "type": "tool_use", "id": "b", "name": "run_task", "input": { "task": "test" } },
            ]})),
            event("tool_result", json!({ "tool_use_id": "a", "is_error": true, "content": "{\"error\":{\"code\":\"not_found\",\"message\":\"missing.rs does not exist\\nmore\"}}" })),
        ], &[]);
        assert_eq!(items[0], Item::Tool { name: "read_file".into(), input: json!({ "path": "missing.rs" }), ok: Some(false), detail: "missing.rs does not exist".into() });
        assert!(matches!(&items[1], Item::Tool { ok: None, name, .. } if name == "run_task"));
    }

    #[test]
    fn notices_errors_and_undecided_approvals_are_shown_and_empty_text_is_not() {
        let items = fold(&[
            event("assistant", json!({ "content": [{ "type": "text", "text": "  " }] })),
            event("notice", json!({ "text": "Paused after 40 steps." })),
            event("error", json!({ "message": "the provider refused the key" })),
            event("approval", json!({ "capability": "fs.read", "request_id": "old", "plan": { "summary": "read a log" } })),
            event("approval", json!({ "capability": "git.push", "request_id": "r1", "plan": { "summary": "push main to origin" } })),
        ], &["r1"]);
        assert_eq!(items, vec![
            Item::Notice("Paused after 40 steps.".into()),
            Item::Failed("the provider refused the key".into()),
            Item::Waiting { capability: "git.push".into(), summary: "push main to origin".into() },
        ]);
    }
}
