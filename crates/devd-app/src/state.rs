//! What the whole window shares: the connection, the cache, who is present.
//!
//! Screens read this and draw. They do not hold copies of devd's data, so two
//! screens cannot disagree about it, and a screen that is opened for the first
//! time already has whatever another screen fetched.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use devd_app_core::presence::{Doing, Place, Presence, Room, Who};
use devd_app_core::sync::{Engine, LogSpec, Outcome, QueryKey, Settled};
use futures::channel::mpsc::{unbounded, UnboundedSender};
use futures::StreamExt;
use gpui::{App, Global};
use serde_json::{json, Value};

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
    /// Things to tell the person once: a write that failed, one that worked.
    pub toasts: Vec<Toast>,
    /// Writes that returned a plan and wait for the person's decision.
    pub decisions: Vec<Decision>,
    /// Whether the event stream is connected.
    pub online: bool,
    /// A conversation devd just created for a first message, to be opened.
    pub opened_thread: Option<String>,
    /// Where the slow pulse of every "happening now" mark is, 0 to 3.
    pub beat: u8,
    /// Whether such a mark was drawn since the last beat. Only then is the
    /// window redrawn for the next one.
    pub live_seen: std::cell::Cell<bool>,
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
}

/// How often the pulse of a live mark steps.
pub const BEAT: Duration = Duration::from_millis(420);

/// How long a toast stays.
pub const TOAST_FOR: Duration = Duration::from_secs(5);

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
    room.seen("you", Presence { who: Who::You, place: Place::default(), doing: Doing::Viewing }, Instant::now());
    cx.set_global(Devd { conn, sync, room, writing: HashMap::new(), toasts: Vec::new(), decisions: Vec::new(), online: false, opened_thread: None, beat: 0, live_seen: std::cell::Cell::new(false) });

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

    // Presence fades and toasts leave on their own, so something has to look.
    // The same look moves the pulse of whatever is happening now. A mark that
    // animated by itself would redraw the whole window on every frame for as
    // long as an agent worked, which on a fast display is hundreds of redraws a
    // second to say "still going". This is two or three, and none at all when
    // no such mark is on screen.
    cx.spawn(async move |cx| loop {
        cx.background_executor().timer(BEAT).await;
        let alive = cx.update(|cx| {
            let now = Instant::now();
            let state = cx.global_mut::<Devd>();
            let before = state.toasts.len();
            state.toasts.retain(|t| now.duration_since(t.at) < TOAST_FOR);
            state.beat = (state.beat + 1) % 4;
            let pulsing = state.live_seen.replace(false);
            let changed = state.room.sweep(now) || state.toasts.len() != before || pulsing;
            if changed {
                cx.refresh_windows();
            }
        });
        if alive.is_err() {
            break;
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
    let _ = std::thread::Builder::new().name("devd-events".into()).spawn(move || {
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
                let _ = announce.unbounded_send(Signal::Event(kind.to_string(), payload.clone()));
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
            for domain in ["chat", "team", "inbox", "approval", "site", "service", "health", "project", "runtime", "provider", "models"] {
                sync.invalidate(domain);
            }
        }
    });
}

fn apply(signal: Signal, cx: &mut App) {
    let state = cx.global_mut::<Devd>();
    match signal {
        Signal::Changed => {}
        Signal::Online(online) => state.online = online,
        Signal::Event(kind, payload) => {
            let data = &payload["data"];
            let thread = data["thread"].as_str().unwrap_or_default().to_string();
            match kind.as_str() {
                "chat.delta" if !thread.is_empty() => {
                    let text = state.writing.entry(thread.clone()).or_default();
                    if data["start"] == json!(true) {
                        text.clear();
                    }
                    text.push_str(data["text"].as_str().unwrap_or_default());
                    let place = Place { thread: Some(thread.clone()), ..Place::default() };
                    state.room.seen(&thread, Presence { who: Who::Assistant, place, doing: Doing::Writing }, Instant::now());
                }
                "chat.event" if !thread.is_empty() => {
                    // The logged event replaces the live text in the same place.
                    state.writing.remove(&thread);
                }
                _ => {}
            }
        }
    }
}

/// Report the writes that finished: a toast for a failure, a decision for a plan.
fn settle_writes(cx: &mut App) {
    let outcomes: Vec<Outcome> = devd(cx).sync.take_outcomes();
    if outcomes.is_empty() {
        return;
    }
    let state = cx.global_mut::<Devd>();
    for outcome in outcomes {
        match outcome.settled {
            Settled::Done(value) => {
                // A write whose result is not otherwise visible says that it worked.
                if let Some(text) = devd_app_core::status::confirmation(&outcome.capability, &value) {
                    // A check that ran and found a problem is a result, not a success.
                    let failed = value["ok"] == json!(false);
                    state.toasts.push(Toast { text, failed, at: Instant::now() });
                }
                if outcome.capability == "chat.send" {
                    let id = value["id"].as_str().or(value["thread"]["id"].as_str()).or(value["thread"].as_str());
                    state.opened_thread = id.map(str::to_string);
                }
            }
            Settled::Failed(failure) => state.toasts.push(Toast { text: failure.message, failed: true, at: Instant::now() }),
            Settled::NeedsApproval { token, plan } => state.decisions.push(Decision { capability: outcome.capability, token, plan }),
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
        LogSpec { cursor_arg: "after", rows: "/events", id: "id", page: 500 }
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

    /// The team's roles: what each is for and what it must prove.
    pub fn roles() -> QueryKey {
        QueryKey::bare("team.roles")
    }

    /// What every specialist is doing right now, across projects.
    pub fn overview() -> QueryKey {
        QueryKey::bare("team.overview")
    }
}
