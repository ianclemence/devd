//! The sync engine: what the window knows, and how it stays true.
//!
//! The web interface asked the server for everything on every screen: Home made
//! seventy requests, and a long conversation was read whole each time it was
//! opened. A native window can do better, because it can keep what it has
//! already been told.
//!
//! The engine holds one cache of **queries**. A query is a read capability and
//! its arguments (`site.list {}`, `team.get {"run": "…"}`), and its value is
//! whatever that capability returned. Screens never call the backend; they
//! [`watch`](Engine::watch) a query and draw what comes back, which is one of:
//!
//! * nothing yet, and a fetch has been started;
//! * the last known value, immediately, while a newer one is fetched in the
//!   background (stale-while-revalidate);
//! * the current value.
//!
//! Three things keep the cache true:
//!
//! 1. **Events.** Every mutation in devd publishes an event on the bus. An
//!    event's kind names the domain that changed (`site.created`), and
//!    [`Engine::on_event`] marks the queries of that domain stale. Queries a
//!    screen is looking at are refetched at once; the rest wait until somebody
//!    looks.
//! 2. **Optimistic mutations.** A write is shown before it is confirmed. The
//!    caller describes the change as [`Patch`]es over cached queries; the engine
//!    lays them over the confirmed values until the backend answers, then drops
//!    them, either because the refetched truth now includes the change or
//!    because it failed and the screen must go back. Confirmed data is never
//!    edited in place, so a rollback is the removal of an overlay, not an undo.
//! 3. **Logs.** A conversation or a run timeline only grows. A [`LogSpec`]
//!    query keeps its rows and asks only for what came after the last one.
//!
//! The engine knows nothing about gpui, HTTP or the registry. It is given a
//! [`Backend`] and a callback to wake the window, which is what makes every
//! behaviour here testable with a fake backend and no window.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

/// What the engine needs from devd.
pub trait Backend: Send + Sync + 'static {
    /// Run a capability. A read returns its data; a write may instead return the
    /// plan a person has to approve.
    fn invoke(&self, capability: &str, args: &Value) -> Result<Reply, Failure>;
}

/// What a capability answered.
#[derive(Debug, Clone, PartialEq)]
pub enum Reply {
    /// It ran. The value is the capability's data.
    Done(Value),
    /// It changed nothing and waits for a person's decision.
    NeedsApproval {
        /// The single-use token that applies exactly this invocation.
        token: String,
        /// What would happen, as the capability's planner described it.
        plan: Value,
    },
}

/// A refused or failed call, in devd's own words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    /// Machine-readable code: `not_found`, `permission_denied`, `conflict`…
    pub code: String,
    /// The sentence a person reads.
    pub message: String,
}

impl Failure {
    /// A failure with a code and a message.
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

/// A read capability and its arguments: the identity of a cached value.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct QueryKey {
    /// Capability id, e.g. `site.list`.
    pub capability: String,
    /// The arguments as canonical JSON text, so equal arguments are equal keys
    /// whatever order their fields were written in.
    args: String,
}

impl QueryKey {
    /// A query with arguments.
    pub fn new(capability: impl Into<String>, args: Value) -> Self {
        Self {
            capability: capability.into(),
            args: canonical(&args),
        }
    }

    /// A query that takes no arguments.
    pub fn bare(capability: impl Into<String>) -> Self {
        Self::new(capability, json!({}))
    }

    /// The arguments, parsed back.
    pub fn args(&self) -> Value {
        serde_json::from_str(&self.args).unwrap_or(Value::Null)
    }

    /// The domain a capability belongs to: `site` for `site.list`.
    pub fn domain(&self) -> &str {
        self.capability
            .split('.')
            .next()
            .unwrap_or(&self.capability)
    }
}

/// JSON text with object keys sorted, recursively.
fn canonical(v: &Value) -> String {
    fn sorted(v: &Value) -> Value {
        match v {
            Value::Object(m) => {
                let ordered: BTreeMap<&String, Value> =
                    m.iter().map(|(k, v)| (k, sorted(v))).collect();
                Value::Object(ordered.into_iter().map(|(k, v)| (k.clone(), v)).collect())
            }
            Value::Array(a) => Value::Array(a.iter().map(sorted).collect()),
            other => other.clone(),
        }
    }
    sorted(v).to_string()
}

/// How a growing log is read in pieces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogSpec {
    /// The argument that carries the last row already held, e.g. `after`.
    pub cursor_arg: &'static str,
    /// JSON pointer to the array of rows in the reply, e.g. `/events`.
    pub rows: &'static str,
    /// The field of a row that orders it, e.g. `id`.
    pub id: &'static str,
    /// The most rows one reply carries. A reply that is full means there is
    /// more, and the engine asks again at once until one comes back short.
    pub page: usize,
}

/// Where a cached value stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Freshness {
    /// Never fetched; a fetch is on its way.
    Loading,
    /// The newest answer the engine has, with nothing known to have changed.
    Fresh,
    /// A value is shown, and a newer one is being fetched or will be on the next look.
    Stale,
    /// The last fetch failed. An older value may still be shown beside the error.
    Failed,
}

/// What a screen draws for one query.
#[derive(Debug, Clone)]
pub struct Snapshot {
    /// The value, with any pending optimistic changes laid over it.
    pub data: Option<Arc<Value>>,
    /// Where it stands.
    pub freshness: Freshness,
    /// The last fetch's failure, if it failed.
    pub error: Option<Failure>,
    /// Whether an unconfirmed change is part of `data`.
    pub optimistic: bool,
    /// Whether a fetch is running now.
    pub fetching: bool,
}

impl Snapshot {
    /// A list inside the value, or empty when it is absent or not a list.
    pub fn list(&self, pointer: &str) -> &[Value] {
        self.data
            .as_deref()
            .and_then(|v| {
                if pointer.is_empty() {
                    Some(v)
                } else {
                    v.pointer(pointer)
                }
            })
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// True the first time only: nothing to show and no error to show instead.
    pub fn is_first_load(&self) -> bool {
        self.data.is_none() && self.error.is_none()
    }
}

/// One change to one cached value, applied before the backend confirms it.
#[derive(Clone)]
pub struct Patch {
    key: QueryKey,
    apply: Arc<dyn Fn(&mut Value) + Send + Sync>,
}

impl Patch {
    /// Any change, as a function over the cached value.
    pub fn new(key: QueryKey, apply: impl Fn(&mut Value) + Send + Sync + 'static) -> Self {
        Self {
            key,
            apply: Arc::new(apply),
        }
    }

    /// Set the value at a JSON pointer inside the cached value.
    pub fn set(key: QueryKey, pointer: &'static str, value: Value) -> Self {
        Self::new(key, move |root| {
            if let Some(slot) = root.pointer_mut(pointer) {
                *slot = value.clone();
            }
        })
    }

    /// Remove the row whose `field` equals `id` from the list at `pointer`.
    pub fn remove_row(
        key: QueryKey,
        pointer: &'static str,
        field: &'static str,
        id: Value,
    ) -> Self {
        Self::new(key, move |root| {
            if let Some(rows) = list_mut(root, pointer) {
                rows.retain(|row| row.get(field) != Some(&id));
            }
        })
    }

    /// Merge `fields` into the row whose `field` equals `id`.
    pub fn update_row(
        key: QueryKey,
        pointer: &'static str,
        field: &'static str,
        id: Value,
        fields: Value,
    ) -> Self {
        Self::new(key, move |root| {
            let Some(rows) = list_mut(root, pointer) else {
                return;
            };
            let Some(row) = rows.iter_mut().find(|row| row.get(field) == Some(&id)) else {
                return;
            };
            if let (Some(row), Some(fields)) = (row.as_object_mut(), fields.as_object()) {
                for (k, v) in fields {
                    row.insert(k.clone(), v.clone());
                }
            }
        })
    }

    /// Put a new row at the front of the list at `pointer`.
    pub fn prepend_row(key: QueryKey, pointer: &'static str, row: Value) -> Self {
        Self::new(key, move |root| {
            if let Some(rows) = list_mut(root, pointer) {
                rows.insert(0, row.clone());
            }
        })
    }

    /// Put a new row at the end of the list at `pointer`.
    pub fn append_row(key: QueryKey, pointer: &'static str, row: Value) -> Self {
        Self::new(key, move |root| {
            if let Some(rows) = list_mut(root, pointer) {
                rows.push(row.clone());
            }
        })
    }
}

fn list_mut<'a>(root: &'a mut Value, pointer: &str) -> Option<&'a mut Vec<Value>> {
    let slot = if pointer.is_empty() {
        Some(root)
    } else {
        root.pointer_mut(pointer)
    };
    slot.and_then(Value::as_array_mut)
}

/// A write, with the way to show it before it is confirmed.
#[derive(Clone)]
pub struct Mutation {
    /// The capability to run.
    pub capability: String,
    /// Its arguments.
    pub args: Value,
    /// What the screens should show meanwhile.
    pub patches: Vec<Patch>,
    /// Extra domains this write changes, beyond its own. A write to `hosts.sync`
    /// changes what `site.list` returns, and only the caller knows that.
    pub also_invalidates: Vec<&'static str>,
}

impl Mutation {
    /// A write with no optimistic change: the screens update when it is confirmed.
    pub fn new(capability: impl Into<String>, args: Value) -> Self {
        Self {
            capability: capability.into(),
            args,
            patches: Vec::new(),
            also_invalidates: Vec::new(),
        }
    }

    /// Show this change at once.
    pub fn optimistic(mut self, patch: Patch) -> Self {
        self.patches.push(patch);
        self
    }

    /// Also refetch the queries of another domain when this succeeds.
    pub fn invalidating(mut self, domain: &'static str) -> Self {
        self.also_invalidates.push(domain);
        self
    }
}

/// A handle to a write in flight.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MutationId(u64);

/// How a write ended.
#[derive(Debug, Clone, PartialEq)]
pub enum Settled {
    /// Applied. The value is the capability's data.
    Done(Value),
    /// Nothing was applied; a person must approve the plan first.
    NeedsApproval {
        /// The token to approve or decline with.
        token: String,
        /// What would happen.
        plan: Value,
    },
    /// Refused or failed. The optimistic change has been taken back.
    Failed(Failure),
}

/// A write that finished, for the window to report.
#[derive(Debug, Clone, PartialEq)]
pub struct Outcome {
    /// Which write.
    pub id: MutationId,
    /// The capability it ran.
    pub capability: String,
    /// How it ended.
    pub settled: Settled,
}

struct Entry {
    /// The confirmed value.
    base: Option<Arc<Value>>,
    /// The confirmed value with overlays applied, and the overlay ids it was built from.
    composed: Option<(Vec<u64>, Arc<Value>)>,
    stale: bool,
    fetching: bool,
    /// A change arrived while a fetch was running; the answer may predate it.
    refetch: bool,
    error: Option<Failure>,
    last_watched: Option<Instant>,
    log: Option<LogSpec>,
}

impl Entry {
    fn new(log: Option<LogSpec>) -> Self {
        Self {
            base: None,
            composed: None,
            stale: true,
            fetching: false,
            refetch: false,
            error: None,
            last_watched: None,
            log,
        }
    }
}

struct Overlay {
    id: u64,
    patches: Vec<Patch>,
}

enum Job {
    Fetch(QueryKey),
    Mutate(u64, Mutation),
}

struct State {
    entries: HashMap<QueryKey, Entry>,
    overlays: Vec<Overlay>,
    queue: VecDeque<Job>,
    running: usize,
    next_mutation: u64,
    outcomes: Vec<Outcome>,
    revision: u64,
    closed: bool,
}

/// How long after a screen last looked at a query it still counts as on screen.
///
/// A window redraws what it shows at least this often while it is visible, so a
/// query not watched for this long is on a screen that has been left.
const ON_SCREEN: Duration = Duration::from_secs(20);

struct Inner {
    backend: Arc<dyn Backend>,
    state: Mutex<State>,
    work: Condvar,
    idle: Condvar,
    wake: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
}

/// The cache and its workers. Cloning shares one engine.
#[derive(Clone)]
pub struct Engine {
    inner: Arc<Inner>,
}

impl Engine {
    /// Start an engine over a backend, with `workers` threads doing the calls.
    pub fn new(backend: Arc<dyn Backend>, workers: usize) -> Self {
        let inner = Arc::new(Inner {
            backend,
            state: Mutex::new(State {
                entries: HashMap::new(),
                overlays: Vec::new(),
                queue: VecDeque::new(),
                running: 0,
                next_mutation: 1,
                outcomes: Vec::new(),
                revision: 0,
                closed: false,
            }),
            work: Condvar::new(),
            idle: Condvar::new(),
            wake: Mutex::new(None),
        });
        for n in 0..workers.max(1) {
            let inner = inner.clone();
            let _ = std::thread::Builder::new()
                .name(format!("devd-sync-{n}"))
                .spawn(move || worker(&inner));
        }
        Self { inner }
    }

    /// Call `wake` whenever something a screen might draw has changed.
    ///
    /// It is called from a worker thread and must only schedule a redraw.
    pub fn on_change(&self, wake: impl Fn() + Send + Sync + 'static) {
        *self.inner.wake.lock().expect("wake lock") = Some(Arc::new(wake));
    }

    /// A number that grows whenever any snapshot may have changed.
    pub fn revision(&self) -> u64 {
        self.inner.state.lock().expect("sync state").revision
    }

    /// Read a query for drawing, fetching it if it is missing or stale.
    pub fn watch(&self, key: &QueryKey) -> Snapshot {
        self.watch_as(key, None)
    }

    /// Read a growing log for drawing. Later fetches ask only for newer rows.
    pub fn watch_log(&self, key: &QueryKey, spec: LogSpec) -> Snapshot {
        self.watch_as(key, Some(spec))
    }

    fn watch_as(&self, key: &QueryKey, log: Option<LogSpec>) -> Snapshot {
        let mut st = self.inner.state.lock().expect("sync state");
        let entry = st
            .entries
            .entry(key.clone())
            .or_insert_with(|| Entry::new(log));
        entry.last_watched = Some(Instant::now());
        let start = entry.stale && !entry.fetching && entry.error.is_none();
        if start {
            entry.fetching = true;
            st.queue.push_back(Job::Fetch(key.clone()));
            self.inner.work.notify_one();
        }
        snapshot(&mut st, key)
    }

    /// Read what is cached without starting anything.
    pub fn peek(&self, key: &QueryKey) -> Option<Snapshot> {
        let mut st = self.inner.state.lock().expect("sync state");
        st.entries.contains_key(key).then(|| snapshot(&mut st, key))
    }

    /// Fetch a query again now, including one whose last fetch failed.
    pub fn refresh(&self, key: &QueryKey) {
        let mut st = self.inner.state.lock().expect("sync state");
        let entry = st
            .entries
            .entry(key.clone())
            .or_insert_with(|| Entry::new(None));
        entry.error = None;
        entry.stale = true;
        if entry.fetching {
            entry.refetch = true;
        } else {
            entry.fetching = true;
            st.queue.push_back(Job::Fetch(key.clone()));
            self.inner.work.notify_one();
        }
    }

    /// Mark every query of a domain stale, refetching the ones on screen.
    pub fn invalidate(&self, domain: &str) {
        let mut st = self.inner.state.lock().expect("sync state");
        invalidate_locked(&self.inner, &mut st, &[domain]);
    }

    /// Tell the engine something changed in devd.
    ///
    /// `kind` is an event's kind from the bus. Its first segment is the domain
    /// whose queries are now out of date; [`related`] adds the domains that a
    /// change in that one is known to reach.
    pub fn on_event(&self, kind: &str) {
        let domain = kind.split('.').next().unwrap_or(kind);
        let mut domains = vec![domain];
        domains.extend(related(domain));
        let mut st = self.inner.state.lock().expect("sync state");
        invalidate_locked(&self.inner, &mut st, &domains);
    }

    /// Run a write, showing its patches until the backend answers.
    pub fn mutate(&self, mutation: Mutation) -> MutationId {
        let mut st = self.inner.state.lock().expect("sync state");
        let id = st.next_mutation;
        st.next_mutation += 1;
        if !mutation.patches.is_empty() {
            st.overlays.push(Overlay {
                id,
                patches: mutation.patches.clone(),
            });
            st.revision += 1;
        }
        st.queue.push_back(Job::Mutate(id, mutation));
        self.inner.work.notify_one();
        MutationId(id)
    }

    /// The writes that finished since this was last called.
    pub fn take_outcomes(&self) -> Vec<Outcome> {
        std::mem::take(&mut self.inner.state.lock().expect("sync state").outcomes)
    }

    /// How many writes have not been answered yet.
    pub fn pending_mutations(&self) -> usize {
        let st = self.inner.state.lock().expect("sync state");
        st.overlays.len()
            + st.queue
                .iter()
                .filter(|j| matches!(j, Job::Mutate(..)))
                .count()
    }

    /// Wait until nothing is queued or running. For tests and for shutdown.
    pub fn settle(&self) {
        let mut st = self.inner.state.lock().expect("sync state");
        while !st.queue.is_empty() || st.running > 0 {
            st = self.inner.idle.wait(st).expect("sync state");
        }
    }

    /// Every confirmed value, for writing to disk so the next start is instant.
    pub fn export(&self) -> Value {
        let st = self.inner.state.lock().expect("sync state");
        let rows: Vec<Value> = st
            .entries
            .iter()
            .filter_map(|(key, entry)| {
                let base = entry.base.as_ref()?;
                Some(json!({ "capability": key.capability, "args": key.args(), "data": **base }))
            })
            .collect();
        json!({ "version": 1, "queries": rows })
    }

    /// Load values written by [`export`](Engine::export). They are shown at once
    /// and treated as stale, so the first look at each refetches it.
    pub fn import(&self, saved: &Value) {
        if saved["version"] != json!(1) {
            return;
        }
        let mut st = self.inner.state.lock().expect("sync state");
        for row in saved["queries"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or(&[])
        {
            let Some(capability) = row["capability"].as_str() else {
                continue;
            };
            let key = QueryKey::new(capability, row["args"].clone());
            let entry = st.entries.entry(key).or_insert_with(|| Entry::new(None));
            if entry.base.is_none() {
                entry.base = Some(Arc::new(row["data"].clone()));
                entry.stale = true;
            }
        }
        st.revision += 1;
    }

    /// Stop the workers. Queued work is dropped.
    pub fn close(&self) {
        let mut st = self.inner.state.lock().expect("sync state");
        st.closed = true;
        st.queue.clear();
        self.inner.work.notify_all();
        self.inner.idle.notify_all();
    }
}

/// Domains that go out of date when another one changes.
///
/// A site's row carries whether its hostname is published and whether it has a
/// certificate, so a change to hosts or TLS changes what `site.list` returns.
/// These are the edges the event's own name does not show.
pub fn related(domain: &str) -> &'static [&'static str] {
    match domain {
        "hosts" | "tls" | "dns" => &["site", "health", "activity"],
        "site" => &["hosts", "tls", "health", "project", "activity"],
        // A database server starting or stopping changes what the database screens can read.
        "service" | "toolchain" | "php" | "web" => &["services", "health", "site", "db", "backup", "mail"],
        "db" => &["backup", "health"],
        "runtime" => &["site", "health", "project"],
        "chat" => &["team", "inbox", "approval", "todo", "activity"],
        "team" => &["chat", "inbox", "approval", "runs", "activity"],
        "approval" => &["inbox", "chat", "team"],
        "inbox" => &["approval"],
        // A write to a file changes what git says; a git command changes the files. A shell can change either.
        "fs" => &["git", "project", "activity"],
        "git" => &["fs", "project", "review", "activity"],
        "terminal" => &["fs", "git", "project"],
        "project" | "stack" => &["git", "site", "todo", "activity"],
        "provider" | "models" | "model" => &["provider", "models", "health"],
        "environment" | "settings" => &["health", "session"],
        _ => &[],
    }
}

fn invalidate_locked(inner: &Inner, st: &mut State, domains: &[&str]) {
    let now = Instant::now();
    let mut start = Vec::new();
    for (key, entry) in st.entries.iter_mut() {
        if !domains.contains(&key.domain()) {
            continue;
        }
        entry.stale = true;
        let on_screen = entry
            .last_watched
            .is_some_and(|t| now.duration_since(t) < ON_SCREEN);
        // A failure on screen stays on screen while it is asked again, and is replaced by the answer: clearing it
        // first would flash the loading state on every refresh. One off screen is cleared, so that the next look
        // asks again rather than showing an old failure.
        if !on_screen {
            entry.error = None;
        }
        if entry.fetching {
            entry.refetch = true;
        } else if on_screen {
            entry.fetching = true;
            start.push(key.clone());
        }
    }
    for key in start {
        st.queue.push_back(Job::Fetch(key));
        inner.work.notify_one();
    }
}

fn snapshot(st: &mut State, key: &QueryKey) -> Snapshot {
    let ids: Vec<u64> = st
        .overlays
        .iter()
        .filter(|o| o.patches.iter().any(|p| &p.key == key))
        .map(|o| o.id)
        .collect();
    let patches: Vec<Patch> = st
        .overlays
        .iter()
        .flat_map(|o| o.patches.iter().filter(|p| &p.key == key).cloned())
        .collect();
    let entry = st.entries.get_mut(key).expect("snapshot of a known query");
    let data = match (&entry.base, ids.is_empty()) {
        (None, _) => None,
        (Some(base), true) => Some(base.clone()),
        (Some(base), false) => match &entry.composed {
            Some((built_from, value)) if built_from == &ids => Some(value.clone()),
            _ => {
                let mut value = (**base).clone();
                for patch in &patches {
                    (patch.apply)(&mut value);
                }
                let value = Arc::new(value);
                entry.composed = Some((ids.clone(), value.clone()));
                Some(value)
            }
        },
    };
    let freshness = if entry.error.is_some() {
        Freshness::Failed
    } else if entry.base.is_none() {
        Freshness::Loading
    } else if entry.stale {
        Freshness::Stale
    } else {
        Freshness::Fresh
    };
    Snapshot {
        data,
        freshness,
        error: entry.error.clone(),
        optimistic: !ids.is_empty(),
        fetching: entry.fetching,
    }
}

fn worker(inner: &Inner) {
    loop {
        let job = {
            let mut st = inner.state.lock().expect("sync state");
            loop {
                if st.closed {
                    return;
                }
                if let Some(job) = st.queue.pop_front() {
                    st.running += 1;
                    break job;
                }
                st = inner.work.wait(st).expect("sync state");
            }
        };
        match job {
            Job::Fetch(key) => fetch(inner, &key),
            Job::Mutate(id, mutation) => mutate(inner, id, mutation),
        }
        let mut st = inner.state.lock().expect("sync state");
        st.running -= 1;
        st.revision += 1;
        if st.queue.is_empty() && st.running == 0 {
            inner.idle.notify_all();
        }
        drop(st);
        let wake = inner.wake.lock().expect("wake lock").clone();
        if let Some(wake) = wake {
            wake();
        }
    }
}

fn fetch(inner: &Inner, key: &QueryKey) {
    // For a log, ask only for what came after the last row held.
    let (args, log) = {
        let st = inner.state.lock().expect("sync state");
        let entry = st.entries.get(key);
        let log = entry.and_then(|e| e.log.clone());
        let mut args = key.args();
        if let (Some(spec), Some(base)) = (&log, entry.and_then(|e| e.base.as_ref())) {
            let last = base
                .pointer(spec.rows)
                .and_then(Value::as_array)
                .and_then(|rows| rows.last())
                .map(|row| row[spec.id].clone());
            if let (Some(last), Some(map)) = (last, args.as_object_mut()) {
                map.insert(spec.cursor_arg.to_string(), last);
            }
        }
        (args, log)
    };
    let result = inner.backend.invoke(&key.capability, &args);
    let mut st = inner.state.lock().expect("sync state");
    let Some(entry) = st.entries.get_mut(key) else {
        return;
    };
    entry.fetching = false;
    match result {
        Ok(Reply::Done(value)) => {
            // A full page is not the end of the log.
            if let Some(spec) = &log {
                let rows = value
                    .pointer(spec.rows)
                    .and_then(Value::as_array)
                    .map_or(0, Vec::len);
                if spec.page > 0 && rows >= spec.page {
                    entry.refetch = true;
                }
            }
            entry.base = Some(Arc::new(match (&log, entry.base.take()) {
                (Some(spec), Some(old)) => append_rows(&old, value, spec),
                _ => value,
            }));
            entry.composed = None;
            entry.stale = false;
            entry.error = None;
        }
        Ok(Reply::NeedsApproval { .. }) => {
            entry.error = Some(Failure::new(
                "approval_required",
                "this needs a person's approval and cannot be read as a query",
            ));
        }
        Err(failure) => entry.error = Some(failure),
    }
    if entry.refetch {
        entry.refetch = false;
        entry.stale = true;
        entry.fetching = true;
        st.queue.push_back(Job::Fetch(key.clone()));
        inner.work.notify_one();
    }
}

/// The old log with the new reply's rows added. Everything else in the reply
/// (the thread's state, its title) replaces what was there.
fn append_rows(old: &Value, mut new: Value, spec: &LogSpec) -> Value {
    let mut rows = old
        .pointer(spec.rows)
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let fresh = new
        .pointer(spec.rows)
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let last = rows.last().map(|row| row[spec.id].clone());
    for row in fresh {
        // A reply that ignored the cursor repeats rows; keep each once.
        let newer = match (&last, &row[spec.id]) {
            (Some(Value::Number(a)), Value::Number(b)) => b.as_f64() > a.as_f64(),
            (Some(a), b) => !rows.iter().any(|held| &held[spec.id] == b) && a != b,
            (None, _) => true,
        };
        if newer {
            rows.push(row);
        }
    }
    if let Some(slot) = new.pointer_mut(spec.rows) {
        *slot = Value::Array(rows);
    }
    new
}

fn mutate(inner: &Inner, id: u64, mutation: Mutation) {
    let result = inner.backend.invoke(&mutation.capability, &mutation.args);
    let mut st = inner.state.lock().expect("sync state");
    st.overlays.retain(|o| o.id != id);
    for entry in st.entries.values_mut() {
        entry.composed = None;
    }
    let settled = match result {
        Ok(Reply::Done(value)) => {
            let own = mutation
                .capability
                .split('.')
                .next()
                .unwrap_or("")
                .to_string();
            let mut domains: Vec<&str> = vec![own.as_str()];
            domains.extend(related(&own));
            domains.extend(mutation.also_invalidates.iter().copied());
            invalidate_locked(inner, &mut st, &domains);
            Settled::Done(value)
        }
        Ok(Reply::NeedsApproval { token, plan }) => Settled::NeedsApproval { token, plan },
        Err(failure) => Settled::Failed(failure),
    };
    st.outcomes.push(Outcome {
        id: MutationId(id),
        capability: mutation.capability,
        settled,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A backend whose answers a test sets, and which counts what it was asked.
    struct Fake {
        sites: Mutex<Vec<Value>>,
        calls: AtomicUsize,
        fail_writes: Mutex<Option<Failure>>,
        seen: Mutex<Vec<(String, Value)>>,
    }

    impl Fake {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                sites: Mutex::new(vec![json!({"name": "blog"}), json!({"name": "shop"})]),
                calls: AtomicUsize::new(0),
                fail_writes: Mutex::new(None),
                seen: Mutex::new(Vec::new()),
            })
        }
    }

    impl Backend for Fake {
        fn invoke(&self, capability: &str, args: &Value) -> Result<Reply, Failure> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.seen
                .lock()
                .unwrap()
                .push((capability.to_string(), args.clone()));
            match capability {
                "site.list" => Ok(Reply::Done(json!({ "sites": *self.sites.lock().unwrap() }))),
                "site.remove" => {
                    if let Some(f) = self.fail_writes.lock().unwrap().clone() {
                        return Err(f);
                    }
                    self.sites
                        .lock()
                        .unwrap()
                        .retain(|s| s["name"] != args["name"]);
                    Ok(Reply::Done(json!({})))
                }
                "git.reset" => Ok(Reply::NeedsApproval {
                    token: "t1".into(),
                    plan: json!({"summary": "reset"}),
                }),
                "chat.events" => {
                    let after = args["after"].as_i64().unwrap_or(0);
                    let rows: Vec<Value> = (1..=3)
                        .filter(|n| *n > after)
                        .map(|n| json!({"id": n}))
                        .collect();
                    Ok(Reply::Done(json!({ "state": "idle", "events": rows })))
                }
                "run.events" => {
                    // Seven rows, served two at a time.
                    let after = args["after"].as_i64().unwrap_or(0);
                    let rows: Vec<Value> = (1..=7)
                        .filter(|n| *n > after)
                        .take(2)
                        .map(|n| json!({"id": n}))
                        .collect();
                    Ok(Reply::Done(json!({ "events": rows })))
                }
                other => Err(Failure::new("not_found", format!("no capability {other}"))),
            }
        }
    }

    fn names(snap: &Snapshot) -> Vec<String> {
        snap.list("/sites")
            .iter()
            .map(|s| s["name"].as_str().unwrap().to_string())
            .collect()
    }

    #[test]
    fn equal_arguments_are_one_query_whatever_their_order() {
        let a = QueryKey::new("team.get", json!({"run": "r1", "full": true}));
        let b = QueryKey::new("team.get", json!({"full": true, "run": "r1"}));
        assert_eq!(a, b);
        assert_eq!(a.domain(), "team");
    }

    #[test]
    fn a_first_look_starts_one_fetch_and_later_looks_reuse_it() {
        let fake = Fake::new();
        let engine = Engine::new(fake.clone(), 2);
        let key = QueryKey::bare("site.list");
        let first = engine.watch(&key);
        assert!(first.is_first_load());
        assert_eq!(first.freshness, Freshness::Loading);
        engine.settle();
        for _ in 0..5 {
            let snap = engine.watch(&key);
            assert_eq!(snap.freshness, Freshness::Fresh);
            assert_eq!(names(&snap), ["blog", "shop"]);
        }
        engine.settle();
        assert_eq!(
            fake.calls.load(Ordering::SeqCst),
            1,
            "a fresh value is not fetched again"
        );
    }

    #[test]
    fn an_event_refetches_what_is_on_screen_and_keeps_showing_the_old_value() {
        let fake = Fake::new();
        let engine = Engine::new(fake.clone(), 1);
        let key = QueryKey::bare("site.list");
        engine.watch(&key);
        engine.settle();
        fake.sites.lock().unwrap().push(json!({"name": "api"}));
        engine.on_event("site.created");
        // Whatever the worker has got to, there is always a list to draw.
        assert!(!engine.peek(&key).unwrap().list("/sites").is_empty());
        engine.settle();
        assert_eq!(names(&engine.watch(&key)), ["blog", "shop", "api"]);
    }

    #[test]
    fn an_event_in_a_related_domain_reaches_the_query() {
        let fake = Fake::new();
        let engine = Engine::new(fake.clone(), 1);
        let key = QueryKey::bare("site.list");
        engine.watch(&key);
        engine.settle();
        engine.on_event("hosts.synced");
        engine.settle();
        assert_eq!(
            fake.calls.load(Ordering::SeqCst),
            2,
            "publishing hostnames changes a site's row"
        );
        engine.on_event("mail.received");
        engine.settle();
        assert_eq!(
            fake.calls.load(Ordering::SeqCst),
            2,
            "an unrelated event fetches nothing"
        );
    }

    #[test]
    fn a_write_is_shown_before_it_is_confirmed_and_stays_after() {
        let fake = Fake::new();
        let engine = Engine::new(fake.clone(), 1);
        let key = QueryKey::bare("site.list");
        engine.watch(&key);
        engine.settle();
        engine.mutate(
            Mutation::new("site.remove", json!({"name": "blog"})).optimistic(Patch::remove_row(
                key.clone(),
                "/sites",
                "name",
                json!("blog"),
            )),
        );
        // No waiting: the row is gone from the very next read.
        let shown = engine.peek(&key).unwrap();
        assert!(!names(&shown).contains(&"blog".to_string()));
        engine.settle();
        let after = engine.watch(&key);
        assert_eq!(names(&after), ["shop"]);
        assert!(!after.optimistic);
        assert!(matches!(
            engine.take_outcomes()[0].settled,
            Settled::Done(_)
        ));
    }

    #[test]
    fn a_refused_write_is_taken_back_and_reported() {
        let fake = Fake::new();
        *fake.fail_writes.lock().unwrap() =
            Some(Failure::new("conflict", "the site is serving a request"));
        let engine = Engine::new(fake.clone(), 1);
        let key = QueryKey::bare("site.list");
        engine.watch(&key);
        engine.settle();
        engine.mutate(
            Mutation::new("site.remove", json!({"name": "blog"})).optimistic(Patch::remove_row(
                key.clone(),
                "/sites",
                "name",
                json!("blog"),
            )),
        );
        engine.settle();
        assert_eq!(
            names(&engine.watch(&key)),
            ["blog", "shop"],
            "the row comes back"
        );
        let outcomes = engine.take_outcomes();
        assert_eq!(outcomes.len(), 1);
        assert!(matches!(&outcomes[0].settled, Settled::Failed(f) if f.code == "conflict"));
        assert!(
            engine.take_outcomes().is_empty(),
            "an outcome is reported once"
        );
    }

    #[test]
    fn a_write_that_needs_approval_changes_nothing_and_carries_its_plan() {
        let engine = Engine::new(Fake::new(), 1);
        engine.mutate(Mutation::new("git.reset", json!({"mode": "hard"})));
        engine.settle();
        let outcomes = engine.take_outcomes();
        assert!(
            matches!(&outcomes[0].settled, Settled::NeedsApproval { token, .. } if token == "t1")
        );
    }

    #[test]
    fn a_log_asks_only_for_rows_after_the_last_one_it_holds() {
        let fake = Fake::new();
        let engine = Engine::new(fake.clone(), 1);
        let key = QueryKey::new("chat.events", json!({"thread": "t"}));
        let spec = LogSpec {
            cursor_arg: "after",
            rows: "/events",
            id: "id",
            page: 0,
        };
        engine.watch_log(&key, spec.clone());
        engine.settle();
        assert_eq!(engine.peek(&key).unwrap().list("/events").len(), 3);
        engine.on_event("chat.event");
        engine.settle();
        let second = fake.seen.lock().unwrap().last().cloned().unwrap();
        assert_eq!(
            second.1["after"],
            json!(3),
            "the second read starts after row 3"
        );
        assert_eq!(
            engine.watch_log(&key, spec).list("/events").len(),
            3,
            "nothing is duplicated"
        );
    }

    #[test]
    fn a_log_served_in_pages_is_read_to_its_end() {
        let fake = Fake::new();
        let engine = Engine::new(fake.clone(), 1);
        let key = QueryKey::new("run.events", json!({"run": "r"}));
        engine.watch_log(
            &key,
            LogSpec {
                cursor_arg: "after",
                rows: "/events",
                id: "id",
                page: 2,
            },
        );
        engine.settle();
        let ids: Vec<i64> = engine
            .peek(&key)
            .unwrap()
            .list("/events")
            .iter()
            .map(|e| e["id"].as_i64().unwrap())
            .collect();
        assert_eq!(ids, [1, 2, 3, 4, 5, 6, 7]);
        assert_eq!(
            fake.calls.load(Ordering::SeqCst),
            4,
            "three full pages and the short one that ends it"
        );
    }

    #[test]
    fn a_saved_cache_is_shown_at_once_and_then_refreshed() {
        let fake = Fake::new();
        let first = Engine::new(fake.clone(), 1);
        let key = QueryKey::bare("site.list");
        first.watch(&key);
        first.settle();
        let saved = first.export();

        fake.sites.lock().unwrap().clear();
        let second = Engine::new(fake.clone(), 1);
        second.import(&saved);
        let cold = second.watch(&key);
        assert_eq!(
            names(&cold),
            ["blog", "shop"],
            "the last session's value is drawn immediately"
        );
        assert_eq!(cold.freshness, Freshness::Stale);
        second.settle();
        assert!(
            second.watch(&key).list("/sites").is_empty(),
            "and replaced by the truth"
        );
    }

    #[test]
    fn a_database_server_starting_refreshes_what_the_database_screens_read() {
        for kind in ["service", "toolchain"] {
            assert!(related(kind).contains(&"db"), "{kind} must refresh db");
            assert!(related(kind).contains(&"backup"), "{kind} must refresh backup");
        }
    }

    #[test]
    fn a_failed_read_keeps_the_old_value_beside_the_error() {
        let engine = Engine::new(Fake::new(), 1);
        let key = QueryKey::bare("nope.list");
        engine.watch(&key);
        engine.settle();
        let snap = engine.watch(&key);
        assert_eq!(snap.freshness, Freshness::Failed);
        assert_eq!(snap.error.unwrap().code, "not_found");
    }

    #[test]
    fn the_window_is_woken_when_something_changes() {
        let engine = Engine::new(Fake::new(), 1);
        let woken = Arc::new(AtomicUsize::new(0));
        let count = woken.clone();
        engine.on_change(move || {
            count.fetch_add(1, Ordering::SeqCst);
        });
        let before = engine.revision();
        engine.watch(&QueryKey::bare("site.list"));
        engine.settle();
        assert!(engine.revision() > before);
        // The callback runs just after the worker releases the state.
        for _ in 0..200 {
            if woken.load(Ordering::SeqCst) > 0 {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(woken.load(Ordering::SeqCst) > 0);
    }
}
