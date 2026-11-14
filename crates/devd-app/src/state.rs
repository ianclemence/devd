//! What the whole window shares: the connection, the cache, who is present.
//!
//! Screens read this and draw. They do not hold copies of devd's data, so two
//! screens cannot disagree about it, and a screen that is opened for the first
//! time already has whatever another screen fetched.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use devd_app_core::presence::{Doing, Place, Presence, Room, Who};
use devd_app_core::sync::{Engine, LogSpec, Mutation, MutationId, Outcome, QueryKey, Settled};
use futures::StreamExt;
use futures::channel::mpsc::{UnboundedSender, unbounded};
use gpui::{App, Global};
use serde_json::{Value, json};

use crate::backend::Connection;

/// Everything shared.
pub struct Devd {
    /// The running devd this window talks to.
    pub conn: Arc<Connection>,
    /// What devd has told the window, kept true.
    pub sync: Engine,
    /// Who is where.
    pub room: Room,
    /// The text of replies being written right now, by conversation.
    pub writing: HashMap<String, String>,
    /// What a model is thinking before it writes, by conversation: when it began and the latest of it.
    pub thinking: HashMap<String, (Instant, String)>,
    /// Things to tell the person once: a write that failed, one that worked.
    pub toasts: Vec<Toast>,
    /// Work that is still running, named while it runs. A scaffold can take a
    /// minute, and a closed dialog must not look like nothing happened.
    pub running: Vec<Running>,
    /// Writes that returned a plan and wait for the person's decision.
    pub decisions: Vec<Decision>,
    /// Whether the event stream is connected.
    pub online: bool,
    /// A conversation devd just created for a first message, to be opened.
    pub opened_thread: Option<String>,
    /// A place an agent asked the window to show, waiting for the window to go there.
    pub pending_go: Option<String>,
    /// A file a link pointed at, inside a project devd knows: the project, the file within it, and the line.
    pub pending_file: Option<(String, String, Option<u32>)>,
    /// Folded runs of steps the person opened, as `<conversation>:<row>`.
    pub open_steps: std::collections::HashSet<String>,
    /// A row whose height changed when it was opened or closed, for the conversation's list to measure again.
    pub steps_toggled: Option<usize>,
    /// Buttons the assistant drew that were pressed, waiting for the shell to carry them out or ask.
    pub pending_ui: Vec<crate::ui::genui::Act>,
    /// Where the slow pulse of every "happening now" mark is, 0 to 3.
    pub beat: u8,
    /// Whether such a mark was drawn since the last beat. Only then is the
    /// window redrawn for the next one.
    pub live_seen: std::cell::Cell<bool>,
    /// Whether the conversation on screen is running, as of the last draw. A run started from a terminal,
    /// a script or CI lives in another process, so this window's own server never hears its events: while
    /// this is set, the conversation is read again on every beat instead of waiting to be told.
    pub chat_running: std::cell::Cell<bool>,
    /// The folder the open conversation is about: relative paths in its replies are read against it.
    pub link_root: Option<std::path::PathBuf>,
    /// Projects whose tasks are shown. A project with many tasks would otherwise
    /// turn the page into a wall of commands.
    pub open_projects: std::collections::HashSet<String>,
    /// Capability groups whose rows are shown on the API and tools page.
    pub open_groups: std::collections::HashSet<String>,
    /// Whether the window may close even while a scaffold runs. Set once the
    /// person has answered the warning.
    pub allow_close: std::cell::Cell<bool>,
    /// Whether the close button was pressed and the warning is owed.
    pub close_asked: std::cell::Cell<bool>,
    /// Whether a terminal was drawn since the last quick look. Only then are the terminals read again, so a
    /// screen nobody is looking at costs nothing.
    pub terminal_seen: std::cell::Cell<bool>,
    /// The same for the browser's tabs.
    pub browser_seen: std::cell::Cell<bool>,
    /// Whether a run that is still going was drawn since the last look. A run may be driven by another process (a
    /// terminal, a script, a window that has since closed), whose events this window's server never hears, so while one
    /// is on screen it is read again every second or two instead of waiting to be told.
    pub run_seen: std::cell::Cell<bool>,
    /// The latest picture of the shared browser's page, and where it was taken.
    pub shot: Option<Shot>,
}

/// A picture of the shared browser's page.
#[derive(Debug, Clone, PartialEq)]
pub struct Shot {
    /// The file, in devd's uploads.
    pub file: String,
    /// The page's width in its own pixels.
    pub width: f32,
    /// The page's height in its own pixels.
    pub height: f32,
    /// The address it was taken at.
    pub url: String,
}

impl Global for Devd {}

/// A short message that leaves by itself.
#[derive(Debug, Clone)]
pub struct Toast {
    /// What happened.
    pub text: String,
    /// Whether it is a failure.
    pub failed: bool,
    /// When it was shown.
    pub at: Instant,
    /// A place to open when the toast is clicked, when there is somewhere to look.
    pub go: Option<String>,
}

impl Toast {
    /// Good news, or a note that is not a failure.
    pub fn new(text: impl Into<String>, failed: bool) -> Self {
        Self {
            text: text.into(),
            failed,
            at: Instant::now(),
            go: None,
        }
    }

    /// A message that opens a screen when clicked.
    pub fn opening(text: impl Into<String>, go: impl Into<String>) -> Self {
        Self {
            go: Some(go.into()),
            ..Self::new(text, false)
        }
    }
}

/// A write that has not answered yet, and what to call it while it runs.
#[derive(Debug, Clone)]
pub struct Running {
    /// Which write.
    pub id: MutationId,
    /// What it is doing, in the person's words.
    pub text: String,
    /// The stack run it started, when it can be followed or cancelled.
    pub run_id: Option<String>,
}

/// How often the pulse of a live mark steps.
pub const BEAT: Duration = Duration::from_millis(420);

/// How long a toast stays, when it is good news. A failure has to be read, and often
/// acted on, so it stays longer: see [`Toast::stays`].
pub const TOAST_FOR: Duration = Duration::from_secs(5);

impl Toast {
    /// How long this one stays.
    pub fn stays(&self) -> Duration {
        if self.failed {
            // Time to read a sentence or two, and then some.
            Duration::from_secs(12)
                .max(Duration::from_millis(self.text.chars().count() as u64 * 70))
        } else if self.go.is_some() {
            // It invites a click; give the person time to reach it.
            Duration::from_secs(12)
        } else {
            TOAST_FOR
        }
    }
}

/// An operation that changes nothing until the person approves its plan.
#[derive(Debug, Clone)]
pub struct Decision {
    /// The capability asked for.
    pub capability: String,
    /// The single-use token that applies exactly that invocation.
    pub token: String,
    /// The server's description: summary, plan, tier.
    pub plan: Value,
}

/// Shared state, read.
pub fn devd(cx: &App) -> &Devd {
    cx.global::<Devd>()
}

/// What a worker thread tells the window's thread.
enum Signal {
    /// Something in the cache changed.
    Changed,
    /// devd published an event.
    Event(String, Value),
    /// The event stream connected or dropped.
    Online(bool),
}

/// Connect the cache and the event stream to the window.
pub fn start(conn: Arc<Connection>, cx: &mut App) {
    let sync = Engine::new(conn.clone(), 6);
    let (tx, mut rx) = unbounded::<Signal>();

    let wake = tx.clone();
    sync.on_change(move || {
        let _ = wake.unbounded_send(Signal::Changed);
    });

    follow_events(conn.clone(), sync.clone(), tx);

    let mut room = Room::new();
    room.seen(
        "you",
        Presence {
            who: Who::You,
            place: Place::default(),
            doing: Doing::Viewing,
        },
        Instant::now(),
    );
    cx.set_global(Devd {
        conn,
        sync,
        room,
        writing: HashMap::new(),
        thinking: HashMap::new(),
        toasts: Vec::new(),
        running: Vec::new(),
        decisions: Vec::new(),
        online: false,
        opened_thread: None,
        pending_go: None,
        pending_file: None,
        open_steps: Default::default(),
        steps_toggled: None,
        pending_ui: Vec::new(),
        beat: 0,
        live_seen: std::cell::Cell::new(false),
        link_root: None,
        chat_running: std::cell::Cell::new(false),
        open_projects: std::collections::HashSet::new(),
        open_groups: std::collections::HashSet::new(),
        allow_close: std::cell::Cell::new(false),
        close_asked: std::cell::Cell::new(false),
        terminal_seen: std::cell::Cell::new(false),
        browser_seen: std::cell::Cell::new(false),
        run_seen: std::cell::Cell::new(false),
        shot: None,
    });

    // One task on the window's thread turns signals into redraws. A burst of
    // signals becomes one redraw: everything waiting is taken before drawing.
    cx.spawn(async move |cx| {
        while let Some(first) = rx.next().await {
            let mut batch = vec![first];
            while let Ok(more) = rx.try_recv() {
                batch.push(more);
            }
            let alive = cx.update(|cx| {
                for signal in batch {
                    apply(signal, cx);
                }
                settle_writes(cx);
                cx.refresh_windows();
            });
            if alive.is_err() {
                break;
            }
        }
    })
    .detach();

    // A shell and a browser tab change whenever their programs do, and nothing tells the window: so while one is
    // on screen it is read again, several times a second. Off screen, not at all.
    cx.spawn(async move |cx| {
        loop {
            cx.background_executor()
                .timer(Duration::from_millis(160))
                .await;
            let alive = cx.update(|cx| {
                let state = devd(cx);
                if state.terminal_seen.replace(false) {
                    state.sync.invalidate("terminal");
                }
            });
            if alive.is_err() {
                break;
            }
        }
    })
    .detach();
    cx.spawn(async move |cx| {
        let mut looks = 0u32;
        loop {
            cx.background_executor()
                .timer(Duration::from_millis(900))
                .await;
            looks = looks.wrapping_add(1);
            let alive = cx.update(|cx| {
                let state = devd(cx);
                if state.browser_seen.replace(false) {
                    state.sync.invalidate("browser");
                }
                // A run is read again every second look, about every two seconds.
                if looks.is_multiple_of(2) && state.run_seen.replace(false) {
                    state.sync.invalidate("team");
                }
                // The machine changes without devd being told: a service stops, a database server is started from
                // elsewhere, a port is taken. What shows it is read again about every five seconds. Only what is on
                // screen is fetched, so a screen that shows none of it costs nothing.
                if looks.is_multiple_of(5) {
                    for domain in ["service", "health", "db", "mail"] {
                        state.sync.invalidate(domain);
                    }
                }
            });
            if alive.is_err() {
                break;
            }
        }
    })
    .detach();

    // Themes from files: the mods folder is looked at about once a second, which costs a directory
    // listing. When something in it changed, it is read again, and a theme in use is repainted in place.
    cx.spawn(async move |cx| {
        loop {
            cx.background_executor().timer(Duration::from_millis(1000)).await;
            let alive = cx.update(|cx| {
                if !crate::theme::Library::of(cx).is_stale() {
                    return;
                }
                let home = devd(cx).conn.home.clone();
                let library = crate::theme::Library::read(std::path::Path::new(&home));
                refresh_theme(&library, &home, cx);
                cx.set_global(library);
                cx.refresh_windows();
            });
            if alive.is_err() {
                break;
            }
        }
    })
    .detach();

    // Presence fades and toasts leave on their own, so something has to look.
    // The same look moves the pulse of whatever is happening now. A mark that
    // animated by itself would redraw the whole window on every frame for as
    // long as an agent worked, which on a fast display is hundreds of redraws a
    // second to say "still going". This is two or three, and none at all when
    // no such mark is on screen.
    cx.spawn(async move |cx| {
        loop {
            cx.background_executor().timer(BEAT).await;
            let alive = cx.update(|cx| {
                let now = Instant::now();
                let state = cx.global_mut::<Devd>();
                let before = state.toasts.len();
                state
                    .toasts
                    .retain(|t| now.duration_since(t.at) < t.stays());
                state.beat = (state.beat + 1) % 4;
                let pulsing = state.live_seen.replace(false);
                // A conversation that is running may be running somewhere else: ask again, not wait.
                if state.chat_running.replace(false) {
                    state.sync.invalidate("chat");
                }
                let changed = state.room.sweep(now) || state.toasts.len() != before || pulsing;
                if changed {
                    cx.refresh_windows();
                }
            });
            if alive.is_err() {
                break;
            }
        }
    })
    .detach();
}

/// Keep the event stream open, reconnecting with a growing pause.
///
/// The pause starts at half a second and doubles to thirty, and goes back to
/// the start once a connection has delivered something. A devd that is
/// restarting is found again quickly; one that is gone is not hammered.
fn follow_events(conn: Arc<Connection>, sync: Engine, tx: UnboundedSender<Signal>) {
    let _ = std::thread::Builder::new()
        .name("devd-events".into())
        .spawn(move || {
            let mut last_id = None;
            let mut pause = Duration::from_millis(500);
            loop {
                let mut heard = false;
                let announce = tx.clone();
                let result = conn.follow(&mut last_id, |kind, payload| {
                    if !heard {
                        heard = true;
                        let _ = announce.unbounded_send(Signal::Online(true));
                    }
                    // A reply being written is not a change to any query.
                    if kind != "chat.delta" {
                        sync.on_event(kind);
                    }
                    let _ =
                        announce.unbounded_send(Signal::Event(kind.to_string(), payload.clone()));
                });
                if tx.unbounded_send(Signal::Online(false)).is_err() {
                    return;
                }
                if heard || result.is_ok() {
                    pause = Duration::from_millis(500);
                }
                std::thread::sleep(pause);
                pause = (pause * 2).min(Duration::from_secs(30));
                // Whatever happened while the stream was down, the screens may be stale.
                for domain in [
                    "chat", "team", "inbox", "approval", "site", "service", "health", "project",
                    "runtime", "provider", "models",
                ] {
                    sync.invalidate(domain);
                }
            }
        });
}

/// After the mods folder changed: repaint a theme from a file that is in use. A theme that was
/// removed gives way to Sling; one that a save broke stays as it last was, and Settings says why.
fn refresh_theme(library: &crate::theme::Library, home: &str, cx: &mut App) {
    let current = crate::theme::theme(cx).clone();
    if !current.is_mod() {
        return;
    }
    let folder = current.id.trim_start_matches("mod:");
    let next = match library.found.iter().find(|f| f.id == folder) {
        None => crate::theme::Theme::by_id("sling"),
        Some(found) => match &found.theme {
            Ok(m) => crate::theme::Theme::from_mod(m),
            Err(_) => return,
        },
    };
    if next != current {
        let _ = crate::theme::save(std::path::Path::new(home), &next);
        cx.set_global(next);
    }
}

/// Wear a theme an agent asked for with `theme.use`. The folder is read first, so a theme saved a
/// moment ago is found.
fn wear_theme(id: &str, cx: &mut App) {
    let home = devd(cx).conn.home.clone();
    let library = crate::theme::Library::read(std::path::Path::new(&home));
    let next = crate::theme::Theme::resolve(id, &library);
    let _ = crate::theme::save(std::path::Path::new(&home), &next);
    cx.set_global(library);
    cx.set_global(next);
    cx.refresh_windows();
}

fn apply(signal: Signal, cx: &mut App) {
    if let Signal::Event(kind, payload) = &signal
        && kind == "app.theme"
    {
        if let Some(id) = payload["data"]["theme"].as_str() {
            wear_theme(id, cx);
        }
        return;
    }
    let state = cx.global_mut::<Devd>();
    match signal {
        Signal::Changed => {}
        Signal::Online(online) => state.online = online,
        Signal::Event(kind, payload) => {
            let data = &payload["data"];
            let thread = data["thread"].as_str().unwrap_or_default().to_string();
            match kind.as_str() {
                "app.go" => state.pending_go = data["route"].as_str().map(str::to_string),
                "chat.delta" if !thread.is_empty() => {
                    if data["start"] == json!(true) {
                        state.thinking.remove(&thread);
                    }
                    // Thinking comes before the words. Only the latest of it is kept: it is shown as a glimpse
                    // that the model is working, not as something to read back.
                    if let Some(piece) = data["thinking"].as_str() {
                        let (_, tail) = state.thinking.entry(thread.clone()).or_insert_with(|| (Instant::now(), String::new()));
                        tail.push_str(piece);
                        let keep = tail.char_indices().rev().nth(280).map(|(i, _)| i).unwrap_or(0);
                        if keep > 0 {
                            tail.drain(..keep);
                        }
                    }
                    if data["text"].as_str().is_some_and(|t| !t.is_empty()) {
                        state.thinking.remove(&thread);
                    }
                    let text = state.writing.entry(thread.clone()).or_default();
                    if data["start"] == json!(true) {
                        text.clear();
                    }
                    text.push_str(data["text"].as_str().unwrap_or_default());
                    let place = Place {
                        thread: Some(thread.clone()),
                        ..Place::default()
                    };
                    // A team member writes in a conversation of its own: its face is that member's (the
                    // Implementer's "I"), not devd's, and pressing it opens that conversation.
                    let who = state
                        .sync
                        .peek(&q::threads())
                        .and_then(|snap| {
                            snap.list("")
                                .iter()
                                .find(|t| t["id"].as_str() == Some(thread.as_str()))
                                .and_then(|t| t["team"]["role"].as_str().map(str::to_string))
                        })
                        .map(Who::Member)
                        .unwrap_or(Who::Assistant);
                    state.room.seen(
                        &thread,
                        Presence {
                            who,
                            place,
                            doing: Doing::Writing,
                        },
                        Instant::now(),
                    );
                }
                "chat.event" if !thread.is_empty() => {
                    // The logged event replaces the live text in the same place.
                    state.writing.remove(&thread);
                    state.thinking.remove(&thread);
                }
                _ => {}
            }
        }
    }
}

/// Start a write and, when it is long enough to notice, name it while it runs.
pub fn start_write(
    cx: &mut App,
    mutation: Mutation,
    text: Option<String>,
    run_id: Option<String>,
) -> MutationId {
    let id = devd(cx).sync.mutate(mutation);
    if let Some(text) = text {
        cx.global_mut::<Devd>()
            .running
            .push(Running { id, text, run_id });
    }
    id
}

/// A project was created and registered, so the toast can open the list.
fn log_created_project(capability: &str, value: &Value) -> bool {
    matches!(capability, "project.init" | "project.create")
        && value["status"].as_str() == Some("ok")
}

/// Report the writes that finished: a toast for a failure, a decision for a plan.
fn settle_writes(cx: &mut App) {
    let outcomes: Vec<Outcome> = devd(cx).sync.take_outcomes();
    if outcomes.is_empty() {
        return;
    }
    let state = cx.global_mut::<Devd>();
    for outcome in outcomes {
        // A write that was named while it ran stops being named now.
        state.running.retain(|run| run.id != outcome.id);
        match outcome.settled {
            Settled::Done(value) => {
                // A write whose result is not otherwise visible says that it worked.
                if let Some(text) = devd_app_core::status::confirmation(&outcome.capability, &value)
                {
                    // A check that ran and found a problem is a result, not a success.
                    let failed = value["ok"] == json!(false)
                        && value["status"].as_str() != Some("cancelled");
                    let toast = if log_created_project(&outcome.capability, &value) {
                        Toast::opening(text, "/projects")
                    } else {
                        Toast::new(text, failed)
                    };
                    state.toasts.push(toast);
                }
                if outcome.capability == "browser.screenshot"
                    && let Some(file) = value["file"].as_str()
                {
                    state.shot = Some(Shot {
                        file: file.to_string(),
                        width: value["width"].as_f64().unwrap_or(0.0) as f32,
                        height: value["height"].as_f64().unwrap_or(0.0) as f32,
                        url: value["url"].as_str().unwrap_or_default().to_string(),
                    });
                }
                if outcome.capability == "chat.send" {
                    let id = value["id"]
                        .as_str()
                        .or(value["thread"]["id"].as_str())
                        .or(value["thread"].as_str());
                    state.opened_thread = id.map(str::to_string);
                }
            }
            Settled::Failed(failure) => {
                let text = devd_app_core::status::failure(&failure.message);
                state.toasts.push(Toast::new(text, true));
            }
            Settled::NeedsApproval { token, plan } => state.decisions.push(Decision {
                capability: outcome.capability,
                token,
                plan,
            }),
        }
    }
}

/// The queries the screens read, named once.
pub mod q {
    use super::*;

    /// Every site, with what serves it and whether its name resolves.
    pub fn sites() -> QueryKey {
        QueryKey::bare("site.list")
    }

    /// Every service, with whether it answers.
    pub fn services() -> QueryKey {
        QueryKey::bare("service.list")
    }

    /// The state of the machine, and what needs attention.
    pub fn health() -> QueryKey {
        QueryKey::bare("health.status")
    }

    /// Every conversation, newest first.
    pub fn threads() -> QueryKey {
        QueryKey::bare("chat.list")
    }

    /// One conversation: its events, its run, whether it is working.
    pub fn thread(id: &str) -> QueryKey {
        QueryKey::new("chat.get", json!({ "thread": id }))
    }

    /// How a conversation's events are read in pieces.
    pub fn thread_log() -> LogSpec {
        // `chat.get` returns at most 500 events for one call.
        LogSpec {
            cursor_arg: "after",
            rows: "/events",
            id: "id",
            page: 500,
        }
    }

    /// Messages and questions waiting for the person.
    pub fn inbox() -> QueryKey {
        QueryKey::bare("inbox.list")
    }

    /// Every team run.
    pub fn runs() -> QueryKey {
        QueryKey::bare("team.list")
    }

    /// One team run with its tasks.
    pub fn run(id: &str) -> QueryKey {
        QueryKey::new("team.get", json!({ "run": id }))
    }

    /// Registered projects.
    pub fn projects() -> QueryKey {
        QueryKey::bare("project.list")
    }

    /// Every provider with its state and every model it offers, priced.
    pub fn inventory() -> QueryKey {
        QueryKey::bare("models.inventory")
    }

    /// Tokens and spend, by model and by who used them.
    pub fn usage() -> QueryKey {
        QueryKey::bare("models.usage")
    }

    /// The providers devd knows how to set up.
    pub fn presets() -> QueryKey {
        QueryKey::bare("provider.presets")
    }

    /// What was done on this machine, and by whom.
    pub fn activity() -> QueryKey {
        QueryKey::new("activity.list", json!({ "limit": 150 }))
    }

    /// Whether the audit trail's hash chain still holds.
    pub fn audit() -> QueryKey {
        QueryKey::bare("audit.verify")
    }

    /// The privacy and spend rules agents run under.
    pub fn policy() -> QueryKey {
        QueryKey::bare("policy.get")
    }

    /// The skills an agent can load, for the `/` menu.
    pub fn skills() -> QueryKey {
        QueryKey::bare("skills.list")
    }

    /// The team's roles: what each is for and what it must prove.
    pub fn roles() -> QueryKey {
        QueryKey::bare("team.roles")
    }

    /// What every specialist is doing right now, across projects.
    pub fn overview() -> QueryKey {
        QueryKey::bare("team.overview")
    }

    /// The team's record: acceptance, rework, checks and time, by role and model.
    pub fn team_stats() -> QueryKey {
        QueryKey::bare("team.stats")
    }
}

/// Type into a shell. Keys go in the order they were pressed, on one thread, and do not wake the screens
/// that depend on the folder: a keystroke has changed nothing yet, and the shell's own screen is read on
/// its own clock.
pub fn send_keys(cx: &App, id: &str, data: String) {
    send_call(cx, "terminal.write", json!({ "id": id, "data": data }));
}

/// Make a call in order with the others sent this way, without waiting for it: what is typed arrives as typed.
pub fn send_call(cx: &App, capability: &'static str, args: Value) {
    use std::sync::OnceLock;
    use std::sync::mpsc::{Sender, channel};
    static LINE: OnceLock<Sender<(Arc<Connection>, &'static str, Value)>> = OnceLock::new();
    let line = LINE.get_or_init(|| {
        let (tx, rx) = channel::<(Arc<Connection>, &'static str, Value)>();
        let _ = std::thread::Builder::new()
            .name("devd-keys".into())
            .spawn(move || {
                while let Ok((conn, capability, args)) = rx.recv() {
                    let _ = devd_app_core::sync::Backend::invoke(&*conn, capability, &args);
                }
            });
        tx
    });
    let _ = line.send((devd(cx).conn.clone(), capability, args));
}

/// Tell the person something once.
pub fn push_toast(cx: &mut App, text: &str) {
    cx.global_mut::<Devd>().toasts.push(Toast::new(text, false));
}
