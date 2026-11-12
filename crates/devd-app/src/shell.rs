//! The window's frame: where you are, how you get anywhere else, and what is
//! waiting for you.
//!
//! Three fixed parts surround every screen. The **rail** on the left is the
//! eight places there are. The **top bar** says where you are and opens the
//! command menu, which reaches everything else. The **status strip** along the
//! bottom is the machine's pulse: health, who is working, which devd.
//!
//! Everything the frame can do is a command in one table (see
//! `devd_app_core::command`), so the rail, the keys and the menu cannot
//! disagree about what exists.

use devd_app_core::command::{Command, Commands, Section};
use devd_app_core::fuzzy;
use devd_app_core::nav::{History, Route};
use devd_app_core::sync::Mutation;
use gpui::{
    actions, div, prelude::*, px, AnyElement, App, ClickEvent, Context, Entity, FocusHandle, Focusable, FontWeight, KeyBinding,
    MouseButton, Subscription, Window,
};
use serde_json::json;

use crate::screens;
use crate::state::{devd, q, Devd};
use crate::theme::{mono, size, theme, Theme, Tone};
use crate::ui::input::{InputEvent, TextInput};
use crate::ui::{self, face, icon, kbd, live_dot, tint, Button, ButtonKind};

actions!(
    devd,
    [
        OpenPalette,
        Back,
        Forward,
        NewConversation,
        ToggleTheme,
        Go1,
        Go2,
        Go3,
        Go4,
        Go5,
        Go6,
        Go7,
        Go8,
        Go9,
        PaletteUp,
        PaletteDown,
        Refresh,
    ]
);

/// The keys the frame answers to. All of them use a modifier, so none can be
/// typed by accident into a text field.
pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("secondary-k", OpenPalette, None),
        KeyBinding::new("secondary-p", OpenPalette, None),
        KeyBinding::new("secondary-n", NewConversation, None),
        KeyBinding::new("secondary-r", Refresh, None),
        KeyBinding::new("secondary-[", Back, None),
        KeyBinding::new("secondary-]", Forward, None),
        KeyBinding::new("secondary-shift-l", ToggleTheme, None),
        KeyBinding::new("secondary-1", Go1, None),
        KeyBinding::new("secondary-2", Go2, None),
        KeyBinding::new("secondary-3", Go3, None),
        KeyBinding::new("secondary-4", Go4, None),
        KeyBinding::new("secondary-5", Go5, None),
        KeyBinding::new("secondary-6", Go6, None),
        KeyBinding::new("secondary-7", Go7, None),
        KeyBinding::new("secondary-8", Go8, None),
        KeyBinding::new("secondary-9", Go9, None),
        KeyBinding::new("up", PaletteUp, Some("Palette")),
        KeyBinding::new("down", PaletteDown, Some("Palette")),
    ]);
}

/// The table of everything the frame can do.
pub fn commands() -> Commands {
    let mut table = Commands::new();
    for (n, route) in Route::primary().iter().enumerate() {
        let id = format!("go.{}", route.segment());
        let detail = match route {
            Route::Home => "What needs you, and what is running",
            Route::Inbox => "Approvals, questions and proposals waiting for a decision",
            Route::Assistant(_) => "Conversations with devd",
            Route::Runs(_) => "Team runs: plans, tasks and evidence",
            Route::Projects(_) => "Every project devd knows",
            Route::Sites(_) => "Addresses, HTTPS and what serves them",
            Route::Team => "The roster: who does what, and what each must prove",
            Route::Server => "Services, runtimes and health",
            Route::Intelligence => "Providers, models, and what the work costs",
            _ => "",
        };
        table.add(Command::new(&id, route.title(), detail, Section::Navigate).key(&format!("secondary-{}", n + 1)));
    }
    table.add(Command::new("go.activity", "Activity", "Everything that ran, and who approved it", Section::Navigate).keywords(&["audit", "log", "history"]));
    table.add(Command::new("go.settings", "Settings", "Preferences and diagnostics", Section::Navigate).keywords(&["preferences", "doctor"]));
    table.add(Command::new("go.back", "Back", "Return to where you were", Section::Navigate).key("secondary-["));
    table.add(Command::new("go.forward", "Forward", "Undo Back", Section::Navigate).key("secondary-]"));
    table.add(Command::new("conversation.new", "New conversation", "Ask devd something, or give it work", Section::Create).key("secondary-n").keywords(&["chat", "ask", "prompt"]));
    table.add(Command::new("server.start", "Start what is needed", "Start every service a project needs", Section::Machine).keywords(&["services", "nginx", "boot", "machine"]));
    table.add(Command::new("server.hosts", "Publish hostnames", "Write the .test names to the hosts file (asks first)", Section::Machine).keywords(&["hosts", "dns", "sync"]).consequential());
    table.add(Command::new("skills.remotion", "Install the Remotion skills", "Fetch Remotion's official agent skills for the motion designer", Section::Machine).keywords(&["video", "film", "animation", "motion", "remotion"]));
    table.add(Command::new("intelligence.catalogue", "Update the model catalogue", "Fetch current models and prices from models.dev", Section::Machine).keywords(&["models", "prices", "refresh", "providers"]));
    table.add(Command::new("intelligence.fetch", "Fetch every provider's models", "Ask each provider what its key may use", Section::Machine).keywords(&["models", "list", "refresh"]));
    table.add(Command::new("view.palette", "Command menu", "Search every action and place", Section::View).key("secondary-k"));
    table.add(Command::new("view.theme", "Switch light and dark", "Change the window's palette", Section::View).key("secondary-shift-l").keywords(&["appearance", "colour"]));
    table.add(Command::new("view.refresh", "Refresh this screen", "Ask devd again for what this screen shows", Section::View).key("secondary-r").keywords(&["reload"]));
    table
}

/// One row of the command menu.
#[derive(Clone)]
struct Entry {
    icon: &'static str,
    title: String,
    detail: String,
    key: Option<String>,
    highlight: Vec<usize>,
    run: Target,
}

/// What choosing a row does.
#[derive(Clone)]
enum Target {
    Command(String),
    Route(Route),
}

/// The open command menu.
struct Palette {
    input: Entity<TextInput>,
    selected: usize,
    _watch: Subscription,
}

/// The root view.
pub struct Shell {
    /// Where the window is, and has been.
    pub history: History,
    focus: FocusHandle,
    commands: Commands,
    palette: Option<Palette>,
    /// The message box, shared by Home and the Assistant.
    pub composer: Entity<TextInput>,
    /// What the message box has chosen besides its text.
    pub compose: screens::compose::Compose,
    /// The conversation on screen, prepared for drawing.
    pub transcript: screens::assistant::Transcript,
    /// What the Intelligence screen has open.
    pub intel: screens::intelligence::View,
    /// The form for adding a member of the team.
    pub role_form: screens::roles::Form,
    /// Whether Activity also lists what only looked at things.
    pub activity_reads: bool,
    _watch: Subscription,
}

impl Shell {
    /// The frame, opened on Home.
    pub fn new(start: Route, window: &mut Window, cx: &mut Context<Self>) -> Self {
        // The words this window spell-checks against: its own, any fuller dictionary
        // on this machine, and the person's, one word per line, in devd's data folder.
        let personal = std::env::var_os("DEVD_HOME").map(std::path::PathBuf::from).unwrap_or_default().join("dictionary.txt");
        let dictionary = std::sync::Arc::new(devd_app_core::spell::Dictionary::for_this_machine(&personal));
        let composer = cx.new(|cx| TextInput::new("Ask devd anything. / for skills, @ for a teammate", cx).multiline(8).spelling(dictionary).linking());
        let watch = cx.subscribe_in(&composer, window, |this, _, event: &InputEvent, window, cx| match event {
            // Enter takes the highlighted skill or member when a menu is showing, and sends otherwise.
            InputEvent::Submitted => {
                if !this.compose_pick(cx) {
                    this.send(window, cx);
                }
            }
            InputEvent::Changed => this.compose_changed(cx),
            InputEvent::Up => this.compose_step(-1, cx),
            InputEvent::Down => this.compose_step(1, cx),
            InputEvent::Cancelled => this.compose_cancel(cx),
        });
        let focus = cx.focus_handle();
        window.focus(&focus);
        let intel = screens::intelligence::View::new(window, cx);
        let role_form = screens::roles::Form::new(cx);
        Self {
            intel,
            role_form,
            activity_reads: false,
            history: History::new(start),
            focus,
            commands: commands(),
            palette: None,
            composer,
            compose: Default::default(),
            transcript: screens::assistant::Transcript::new(),
            _watch: watch,
        }
    }

    /// Go to a place.
    pub fn go(&mut self, route: Route, cx: &mut Context<Self>) {
        self.history.select(route);
        self.present(cx);
        cx.notify();
    }

    /// Tell the room where the person is, so agents can know what they are looking at.
    fn present(&self, cx: &mut Context<Self>) {
        use devd_app_core::presence::{Doing, Place, Presence, Who};
        let route = self.history.current().clone();
        let place = Place { screen: Some(route.path()), thread: matches!(route, Route::Assistant(_)).then(|| route.selection().map(str::to_string)).flatten(), ..Place::default() };
        cx.global_mut::<Devd>().room.seen("you", Presence { who: Who::You, place, doing: Doing::Viewing }, std::time::Instant::now());
        // So an agent asking "what is the person looking at?" gets an answer.
        devd(cx).sync.mutate(Mutation::new("app.report", json!({ "route": route.path() })));
    }

    /// Run a command by id. The rail, the keys, the menu and an agent all end here.
    pub fn run(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.commands.used(id);
        match id {
            "go.back" => {
                self.history.back();
            }
            "go.forward" => {
                self.history.forward();
            }
            "conversation.new" => {
                self.history.go(Route::Assistant(None));
                window.focus(&self.composer.focus_handle(cx));
            }
            "view.palette" => return self.open_palette(window, cx),
            "view.theme" => {
                let next = if theme(cx).dark { Theme::light() } else { Theme::dark() };
                cx.set_global(next);
                cx.refresh_windows();
            }
            "view.refresh" => {
                let sync = devd(cx).sync.clone();
                for domain in ["site", "service", "health", "chat", "team", "inbox", "project", "provider", "models", "runtime", "approval"] {
                    sync.invalidate(domain);
                }
            }
            "server.start" => {
                devd(cx).sync.mutate(Mutation::new("stack.start", json!({})).invalidating("service"));
            }
            "skills.remotion" => {
                devd(cx).sync.mutate(Mutation::new("skills.install", json!({ "source": "remotion-dev/skills" })));
            }
            "intelligence.catalogue" => {
                devd(cx).sync.mutate(Mutation::new("models.refresh", json!({})).invalidating("models"));
            }
            "intelligence.fetch" => {
                let sync = devd(cx).sync.clone();
                let ids: Vec<String> = sync.watch(&q::inventory()).list("/providers").iter().filter(|p| p["configured"] == json!(true)).filter_map(|p| p["id"].as_str().map(str::to_string)).collect();
                for id in ids {
                    sync.mutate(Mutation::new("provider.models", json!({ "id": id })));
                }
            }
            "server.hosts" => {
                devd(cx).sync.mutate(Mutation::new("hosts.sync", json!({})).invalidating("site"));
            }
            other => {
                if let Some(route) = other.strip_prefix("go.").and_then(Route::parse) {
                    self.history.go(route);
                }
            }
        }
        self.present(cx);
        cx.notify();
    }

    /// Send what is in the message box: to the open conversation, or as a new one.
    pub(crate) fn send(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let text = self.composer.read(cx).text().trim().to_string();
        if text.is_empty() {
            return;
        }
        let thread = match self.history.current() {
            Route::Assistant(Some(id)) => Some(id.clone()),
            _ => None,
        };
        let text = screens::compose::with_files(&text, &self.compose.files);
        let mut args = json!({ "message": text });
        // Only what was chosen is sent; the rest stays as the conversation has it.
        if let Some(mode) = self.compose.mode {
            args["permission"] = json!(mode);
        }
        if let Some(effort) = self.compose.effort {
            args["effort"] = json!(effort);
        }
        if let Some((provider, model)) = &self.compose.model {
            args["provider"] = json!(provider);
            args["model"] = json!(model);
        }
        let mut mutation = Mutation::new("chat.send", json!({}));
        if let Some(id) = &thread {
            args["thread"] = json!(id);
            // The message appears in the conversation before devd has stored it.
            mutation = mutation.optimistic(devd_app_core::sync::Patch::append_row(
                q::thread(id),
                "/events",
                json!({ "id": i64::MAX, "kind": "user", "data": { "text": text, "pending": true } }),
            ));
        }
        mutation.args = args;
        devd(cx).sync.mutate(mutation);
        self.compose.files.clear();
        self.composer.update(cx, |input, cx| input.clear(cx));
        if thread.is_none() {
            self.history.go(Route::Assistant(None));
        }
        cx.notify();
    }

    fn open_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.palette.is_some() {
            return self.close_palette(window, cx);
        }
        let input = cx.new(|cx| TextInput::new("Search actions, conversations, sites and projects", cx));
        let watch = cx.subscribe_in(&input, window, |this, _, event: &InputEvent, window, cx| match event {
            InputEvent::Changed => {
                if let Some(palette) = this.palette.as_mut() {
                    palette.selected = 0;
                }
                cx.notify();
            }
            InputEvent::Submitted => this.choose(None, window, cx),
            InputEvent::Cancelled => this.close_palette(window, cx),
            InputEvent::Up | InputEvent::Down => {}
        });
        window.focus(&input.focus_handle(cx));
        self.palette = Some(Palette { input, selected: 0, _watch: watch });
        cx.notify();
    }

    fn close_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.palette = None;
        window.focus(&self.focus);
        cx.notify();
    }

    /// What the menu lists for what has been typed: commands first, then the
    /// conversations, sites and projects devd already told the window about.
    fn entries(&self, cx: &App) -> Vec<Entry> {
        let Some(palette) = &self.palette else { return Vec::new() };
        let query = palette.input.read(cx).text().to_string();
        let mut out: Vec<Entry> = self
            .commands
            .search(&query, 8)
            .into_iter()
            .map(|hit| Entry {
                icon: match hit.command.section {
                    Section::Navigate => Route::parse(hit.command.id.trim_start_matches("go.")).as_ref().map(ui::screen_icon).unwrap_or("arrow-right"),
                    Section::Create => "plus",
                    Section::Machine => "machine",
                    _ => "command",
                },
                title: hit.command.title.clone(),
                detail: hit.command.detail.clone(),
                key: hit.command.key.clone(),
                highlight: hit.highlight,
                run: Target::Command(hit.command.id.clone()),
            })
            .collect();
        if query.trim().is_empty() {
            return out;
        }
        let sync = &devd(cx).sync;
        let mut places: Vec<(i32, Entry)> = Vec::new();
        let mut consider = |icon: &'static str, title: String, detail: String, route: Route| {
            if let Some(m) = fuzzy::matches(&query, &title) {
                places.push((m.score, Entry { icon, title, detail, key: None, highlight: m.positions, run: Target::Route(route) }));
            }
        };
        if let Some(snap) = sync.peek(&q::threads()) {
            for thread in snap.list("") {
                if thread["team"]["role"].is_string() {
                    continue;
                }
                let id = thread["id"].as_str().unwrap_or_default().to_string();
                let project = ui::leaf(thread["project"].as_str().unwrap_or("no project")).to_string();
                consider("assistant", thread["title"].as_str().unwrap_or("Conversation").to_string(), project, Route::Assistant(Some(id)));
            }
        }
        if let Some(snap) = sync.peek(&q::sites()) {
            for site in snap.list("") {
                let name = site["site"]["name"].as_str().unwrap_or_default().to_string();
                consider("globe", site["site"]["host"].as_str().unwrap_or(&name).to_string(), "Site".into(), Route::Sites(Some(name)));
            }
        }
        if let Some(snap) = sync.peek(&q::projects()) {
            for project in snap.list("") {
                let path = project["path"].as_str().unwrap_or_default().to_string();
                consider("folder", project["name"].as_str().unwrap_or(ui::leaf(&path)).to_string(), path.clone(), Route::Projects(Some(path)));
            }
        }
        places.sort_by_key(|place| std::cmp::Reverse(place.0));
        out.extend(places.into_iter().take(8).map(|(_, entry)| entry));
        out
    }

    fn choose(&mut self, index: Option<usize>, window: &mut Window, cx: &mut Context<Self>) {
        let entries = self.entries(cx);
        let index = index.or(self.palette.as_ref().map(|p| p.selected)).unwrap_or(0);
        let Some(entry) = entries.get(index).cloned() else { return };
        self.close_palette(window, cx);
        match entry.run {
            Target::Command(id) => self.run(&id, window, cx),
            Target::Route(route) => self.go(route, cx),
        }
    }

    fn step_palette(&mut self, by: isize, cx: &mut Context<Self>) {
        let count = self.entries(cx).len();
        if let Some(palette) = self.palette.as_mut().filter(|_| count > 0) {
            palette.selected = (palette.selected as isize + by).rem_euclid(count as isize) as usize;
        }
        cx.notify();
    }

    fn rail(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let current = self.history.current().clone();
        let waiting = devd(cx).sync.watch(&q::inbox()).list("").iter().filter(|m| m["state"] == json!("open")).count() + devd(cx).decisions.len();
        let mut rail = div()
            .w(px(size::RAIL))
            .h_full()
            .flex()
            .flex_col()
            .flex_none()
            .items_center()
            .gap(px(2.0))
            .py(px(10.0))
            .bg(t.surface)
            .border_r_1()
            .border_color(t.line);
        // The mark: the same artwork the installed application carries, so the window
        // and the app on the taskbar are one thing.
        rail = rail.child(gpui::img(gpui::SharedString::from("brand/devd-logo-rail.png")).mb(px(10.0)).size(px(28.0)).flex_none().rounded(px(7.0)));
        let item = |route: Route, badge: usize, cx: &mut Context<Self>| {
            let selected = current.same_screen(&route);
            let color = if selected { t.text } else { t.faint };
            let hover = t.hover;
            let id = format!("go.{}", route.segment());
            let mut el = div()
                .id(gpui::ElementId::Name(format!("rail.{}", route.segment()).into()))
                .relative()
                .size(px(38.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(size::RADIUS + 2.0))
                .cursor_pointer()
                .child(icon(ui::screen_icon(&route), 17.0, color))
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| this.run(&id, window, cx)));
            el = if selected { el.bg(t.active) } else { el.hover(move |s| s.bg(hover)) };
            if selected {
                // Where you are: a short bar of the signal colour at the rail's edge.
                el = el.child(div().absolute().left(px(-9.0)).top(px(11.0)).w(px(3.0)).h(px(16.0)).rounded_full().bg(t.accent));
            }
            if badge > 0 {
                el = el.child(
                    div()
                        .absolute()
                        .top(px(3.0))
                        .right(px(3.0))
                        .min_w(px(14.0))
                        .h(px(14.0))
                        .px(px(3.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_full()
                        .bg(t.warn)
                        .text_size(px(9.0))
                        .font_weight(FontWeight::BOLD)
                        .text_color(t.bg)
                        .child(badge.to_string()),
                );
            }
            el
        };
        for route in Route::primary() {
            let badge = if route == Route::Inbox { waiting } else { 0 };
            rail = rail.child(item(route, badge, cx));
        }
        rail.child(div().flex_1()).child(item(Route::Activity, 0, cx)).child(item(Route::Settings, 0, cx))
    }

    fn topbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let route = self.history.current().clone();
        let arrow = |name: &'static str, enabled: bool, command: &'static str, cx: &mut Context<Self>| {
            let hover = t.hover;
            let el = div()
                .id(command)
                .size(px(26.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(size::RADIUS))
                .child(icon(name, 14.0, if enabled { t.muted } else { tint(t.faint, 0.5) }));
            if enabled {
                el.cursor_pointer().hover(move |s| s.bg(hover)).on_click(cx.listener(move |this, _: &ClickEvent, window, cx| this.run(command, window, cx)))
            } else {
                el
            }
        };
        let mut crumbs = div().flex().items_center().gap(px(7.0)).text_size(px(size::TEXT)).child(div().font_weight(FontWeight::MEDIUM).text_color(t.text).child(route.title()));
        if let Some(selection) = route.selection() {
            crumbs = crumbs
                .child(icon("chevron-right", 11.0, t.faint))
                .child(div().max_w(px(360.0)).text_color(t.muted).child(ui::Clip::new(self.selection_label(&route, selection, cx))));
        }
        let hover = t.hover;
        let search = div()
            .id("topbar.search")
            .h(px(size::CONTROL))
            .w(px(280.0))
            .px(px(9.0))
            .flex()
            .items_center()
            .gap(px(8.0))
            .rounded(px(size::RADIUS))
            .border_1()
            .border_color(t.line)
            .bg(t.surface)
            .cursor_pointer()
            .hover(move |s| s.bg(hover))
            .child(icon("search", 13.0, t.faint))
            .child(div().flex_1().text_size(px(size::TEXT_SM)).text_color(t.faint).child("Search or run a command"))
            .child(kbd("secondary-k", cx))
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.open_palette(window, cx)));
        let mut faces = div().flex().items_center().gap(px(4.0));
        for presence in devd(cx).room.everyone().into_iter().filter(|p| p.who != devd_app_core::presence::Who::You).take(5) {
            faces = faces.child(face(presence.who.initials(), Tone::Info, presence.doing.active(), cx));
        }
        div()
            .h(px(size::TOPBAR))
            .flex()
            .flex_none()
            .items_center()
            .gap(px(10.0))
            .px(px(12.0))
            .border_b_1()
            .border_color(t.line)
            .child(arrow("arrow-left", self.history.can_back(), "go.back", cx))
            .child(arrow("arrow-right", self.history.can_forward(), "go.forward", cx))
            .child(crumbs)
            .child(div().flex_1())
            .child(faces)
            .child(search)
    }

    /// The name of what is selected, as devd knows it: a title, not an id.
    fn selection_label(&self, route: &Route, selection: &str, cx: &App) -> String {
        let sync = &devd(cx).sync;
        match route {
            Route::Assistant(_) => sync
                .peek(&q::threads())
                .and_then(|s| s.list("").iter().find(|t| t["id"] == json!(selection)).and_then(|t| t["title"].as_str().map(str::to_string)))
                .unwrap_or_else(|| "Conversation".into()),
            Route::Runs(_) => sync
                .peek(&q::runs())
                .and_then(|s| s.list("").iter().find(|r| r["id"] == json!(selection)).and_then(|r| r["title"].as_str().map(str::to_string)))
                .unwrap_or_else(|| "Run".into()),
            Route::Projects(_) => ui::leaf(selection).to_string(),
            _ => selection.to_string(),
        }
    }

    fn status(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let state = devd(cx);
        let health = state.sync.watch(&q::health());
        let (tone, headline) = match health.data.as_deref() {
            _ if !state.online && health.data.is_none() => (Tone::Neutral, "Connecting to devd…".to_string()),
            Some(data) => {
                let tone = match data["state"].as_str().unwrap_or_default() {
                    "healthy" => Tone::Ok,
                    "critical" => Tone::Danger,
                    "degraded" | "setup_incomplete" => Tone::Warn,
                    _ => Tone::Neutral,
                };
                (tone, data["headline"].as_str().unwrap_or("Checking").to_string())
            }
            None => (Tone::Neutral, "Checking the machine…".to_string()),
        };
        let working = state.room.working();
        let pending = state.sync.pending_mutations();
        let mut strip = div()
            .h(px(size::STATUS))
            .flex()
            .flex_none()
            .items_center()
            .gap(px(14.0))
            .px(px(12.0))
            .border_t_1()
            .border_color(t.line)
            .bg(t.surface)
            .font_family(mono())
            .text_size(px(size::TEXT_XS))
            .text_color(t.faint)
            .child(div().flex().items_center().gap(px(6.0)).child(ui::dot(tone, cx)).child(div().text_color(t.muted).child(headline)));
        if working > 0 {
            strip = strip.child(
                div().flex().items_center().gap(px(6.0)).child(live_dot("status.working", Tone::Info, cx)).child(format!("{working} agent{} working", if working == 1 { "" } else { "s" })),
            );
        }
        if pending > 0 {
            strip = strip.child(div().text_color(t.muted).child(format!("saving {pending}…")));
        }
        if !state.online {
            strip = strip.child(div().text_color(t.warn).child("live updates paused, reconnecting"));
        }
        strip
            .child(div().flex_1())
            .child(if state.conn.owner { "this window runs devd" } else { "attached to a running devd" })
            .child(format!("127.0.0.1:{}", state.conn.port))
            .child(format!("devd {}", state.conn.version))
    }

    fn palette_overlay(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let palette = self.palette.as_ref()?;
        let t = theme(cx).clone();
        let entries = self.entries(cx);
        let mut list = div().flex().flex_col().p(px(6.0)).gap(px(1.0));
        if entries.is_empty() {
            list = list.child(div().px(px(10.0)).py(px(18.0)).text_size(px(size::TEXT)).text_color(t.muted).child("Nothing matches. Try a screen, a site or a conversation title."));
        }
        for (n, entry) in entries.iter().enumerate() {
            let selected = n == palette.selected;
            let mut title = div().flex();
            for (offset, ch) in entry.title.char_indices() {
                let lit = entry.highlight.contains(&offset);
                title = title.child(div().text_color(if lit { t.accent } else { t.text }).font_weight(if lit { FontWeight::SEMIBOLD } else { FontWeight::NORMAL }).child(ch.to_string()));
            }
            let mut line = ui::row(gpui::ElementId::Name(format!("palette.{n}").into()), selected, cx)
                .child(icon(entry.icon, 15.0, if selected { t.accent } else { t.faint }))
                .child(div().flex_none().whitespace_nowrap().child(title))
                .child(div().flex_1().min_w_0().text_size(px(size::TEXT_SM)).text_color(t.faint).child(ui::Clip::new(entry.detail.clone())))
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| this.choose(Some(n), window, cx)));
            if let Some(key) = &entry.key {
                line = line.child(kbd(key, cx));
            }
            list = list.child(line);
        }
        let panel = div()
            .w(px(620.0))
            .max_h(px(460.0))
            .flex()
            .flex_col()
            .rounded(px(size::RADIUS_L + 2.0))
            .border_1()
            .border_color(t.line_strong)
            .bg(t.raised)
            .shadow_lg()
            .overflow_hidden()
            .key_context("Palette")
            .on_action(cx.listener(|this, _: &PaletteUp, _, cx| this.step_palette(-1, cx)))
            .on_action(cx.listener(|this, _: &PaletteDown, _, cx| this.step_palette(1, cx)))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .h(px(46.0))
                    .px(px(14.0))
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(10.0))
                    .border_b_1()
                    .border_color(t.line)
                    .text_size(px(size::TEXT_MD))
                    .line_height(px(22.0))
                    .text_color(t.text)
                    .child(icon("search", 15.0, t.faint))
                    .child(palette.input.clone()),
            )
            .child(div().id("palette.list").flex_1().overflow_y_scroll().child(list))
            .child(
                div()
                    .h(px(30.0))
                    .px(px(12.0))
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(14.0))
                    .border_t_1()
                    .border_color(t.line)
                    .text_size(px(size::TEXT_XS))
                    .text_color(t.faint)
                    .child(div().flex().items_center().gap(px(5.0)).child(kbd("↑ ↓", cx)).child("move"))
                    .child(div().flex().items_center().gap(px(5.0)).child(kbd("enter", cx)).child("run"))
                    .child(div().flex().items_center().gap(px(5.0)).child(kbd("escape", cx)).child("close")),
            );
        Some(
            div()
                .absolute()
                .inset_0()
                .flex()
                .justify_center()
                .pt(px(96.0))
                .bg(t.scrim)
                .occlude()
                .on_mouse_down(MouseButton::Left, cx.listener(|this, _, window, cx| this.close_palette(window, cx)))
                .child(panel)
                .into_any_element(),
        )
    }

    /// The plan of an operation that waits for the person, with the two answers.
    fn decision_overlay(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let decision = devd(cx).decisions.first()?.clone();
        let t = theme(cx).clone();
        let plan = &decision.plan["plan"];
        let summary = plan["summary"].as_str().or(decision.plan["summary"].as_str()).unwrap_or("This operation needs your approval.").to_string();
        let mut body = div().flex().flex_col().gap(px(6.0));
        for step in plan["steps"].as_array().map(Vec::as_slice).unwrap_or(&[]).iter().chain(plan["effects"].as_array().map(Vec::as_slice).unwrap_or(&[])) {
            let text = step.as_str().map(str::to_string).unwrap_or_else(|| step.to_string());
            body = body.child(div().flex().items_start().gap(px(8.0)).child(div().mt(px(7.0)).size(px(4.0)).flex_none().rounded_full().bg(t.faint)).child(div().flex_1().text_size(px(size::TEXT)).text_color(t.muted).child(text)));
        }
        let destructive = plan["destructive"] == json!(true) || matches!(decision.plan["tier"].as_str(), Some("destructive" | "admin"));
        let (approve_token, deny_token) = (decision.token.clone(), decision.token.clone());
        let answer = |approve: bool, token: String| {
            move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                let conn = devd(cx).conn.clone();
                let sync = devd(cx).sync.clone();
                let token = token.clone();
                // The server replays its own record of the invocation, so what
                // was shown here is exactly what runs.
                std::thread::spawn(move || {
                    let _ = if approve { conn.approve(&token) } else { conn.deny(&token) };
                    for domain in ["site", "service", "hosts", "git", "project", "chat", "team", "approval", "inbox", "health"] {
                        sync.invalidate(domain);
                    }
                });
                cx.global_mut::<Devd>().decisions.remove(0);
                cx.refresh_windows();
            }
        };
        let panel = div()
            .w(px(520.0))
            .flex()
            .flex_col()
            .gap(px(14.0))
            .p(px(20.0))
            .rounded(px(size::RADIUS_L + 2.0))
            .border_1()
            .border_color(t.line_strong)
            .bg(t.raised)
            .shadow_lg()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(icon(if destructive { "alert" } else { "lock" }, 16.0, if destructive { t.danger } else { t.warn }))
                    .child(div().text_size(px(size::TEXT_LG)).font_weight(FontWeight::SEMIBOLD).text_color(t.text).child("Approve this?"))
                    .child(div().flex_1())
                    .child(ui::pill(decision.capability.clone(), Tone::Neutral, cx)),
            )
            .child(div().text_size(px(size::TEXT_MD)).text_color(t.text).child(summary))
            .child(body)
            .child(div().text_size(px(size::TEXT_SM)).text_color(t.faint).child("Nothing has changed yet. Declining is recorded too."))
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(px(8.0))
                    .child(Button::new("decision.deny", "Decline").on_click(answer(false, deny_token)))
                    .child(Button::new("decision.approve", "Approve").kind(if destructive { ButtonKind::Danger } else { ButtonKind::Primary }).on_click(answer(true, approve_token))),
            );
        Some(div().absolute().inset_0().flex().items_center().justify_center().bg(t.scrim).occlude().child(panel).into_any_element())
    }

    fn toasts(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let t = theme(cx).clone();
        let toasts = &devd(cx).toasts;
        if toasts.is_empty() {
            return None;
        }
        let mut column = div().absolute().bottom(px(size::STATUS + 14.0)).right(px(16.0)).flex().flex_col().gap(px(8.0));
        for toast in toasts.iter().rev().take(3) {
            column = column.child(
                div()
                    .id(ui::nid("toast", toast.at.elapsed().as_millis() as usize))
                    .w(px(420.0))
                    .p(px(12.0))
                    .flex()
                    .items_start()
                    .cursor_pointer()
                    // A click puts it away: a failure stays long enough to read, not forever.
                    .on_click(|_, _, cx| {
                        cx.global_mut::<Devd>().toasts.clear();
                        cx.refresh_windows();
                    })
                    .gap(px(9.0))
                    .rounded(px(size::RADIUS_L))
                    .border_1()
                    .border_color(if toast.failed { tint(t.danger, 0.4) } else { t.line_strong })
                    .bg(t.raised)
                    .shadow_md()
                    .child(icon(if toast.failed { "alert" } else { "check" }, 15.0, if toast.failed { t.danger } else { t.ok }))
                    .child(div().flex_1().min_w_0().text_size(px(size::TEXT)).line_height(px(19.0)).text_color(t.text).child(toast.text.clone())),
            );
        }
        Some(column.into_any_element())
    }
}

impl Render for Shell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // An agent asked to show the person something.
        if let Some(route) = cx.global_mut::<Devd>().pending_go.take().and_then(|r| Route::parse(&r)) {
            self.history.go(route);
            self.present(cx);
        }
        let t = theme(cx).clone();
        let screen = match self.history.current().clone() {
            Route::Home => screens::home::render(self, window, cx),
            Route::Assistant(thread) => screens::assistant::render(self, thread, window, cx),
            Route::Inbox => screens::lists::inbox(self, cx),
            Route::Runs(run) => screens::lists::runs(self, run, cx),
            Route::Projects(_) => screens::lists::projects(self, cx),
            Route::Sites(_) => screens::lists::sites(self, cx),
            Route::Server => screens::lists::server(self, cx),
            Route::Intelligence => screens::intelligence::render(self, cx),
            Route::Team => screens::lists::team(self, cx),
            Route::Activity => screens::activity::activity(self, cx),
            Route::Settings => screens::activity::settings(self, cx),
        };
        let go = |n: usize| move |this: &mut Self, window: &mut Window, cx: &mut Context<Self>| {
            let id = format!("go.{}", Route::primary()[n].segment());
            this.run(&id, window, cx);
        };
        div()
            .size_full()
            .relative()
            .flex()
            .bg(t.bg)
            .text_color(t.text)
            .text_size(px(size::TEXT))
            .font_family(crate::theme::SANS)
            .key_context("Shell")
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &OpenPalette, window, cx| this.open_palette(window, cx)))
            .on_action(cx.listener(|this, _: &NewConversation, window, cx| this.run("conversation.new", window, cx)))
            .on_action(cx.listener(|this, _: &ToggleTheme, window, cx| this.run("view.theme", window, cx)))
            .on_action(cx.listener(|this, _: &Refresh, window, cx| this.run("view.refresh", window, cx)))
            .on_action(cx.listener(|this, _: &Back, window, cx| this.run("go.back", window, cx)))
            .on_action(cx.listener(|this, _: &Forward, window, cx| this.run("go.forward", window, cx)))
            .on_action(cx.listener({ let f = go(0); move |this, _: &Go1, window, cx| f(this, window, cx) }))
            .on_action(cx.listener({ let f = go(1); move |this, _: &Go2, window, cx| f(this, window, cx) }))
            .on_action(cx.listener({ let f = go(2); move |this, _: &Go3, window, cx| f(this, window, cx) }))
            .on_action(cx.listener({ let f = go(3); move |this, _: &Go4, window, cx| f(this, window, cx) }))
            .on_action(cx.listener({ let f = go(4); move |this, _: &Go5, window, cx| f(this, window, cx) }))
            .on_action(cx.listener({ let f = go(5); move |this, _: &Go6, window, cx| f(this, window, cx) }))
            .on_action(cx.listener({ let f = go(6); move |this, _: &Go7, window, cx| f(this, window, cx) }))
            .on_action(cx.listener({ let f = go(7); move |this, _: &Go8, window, cx| f(this, window, cx) }))
            .on_action(cx.listener({ let f = go(8); move |this, _: &Go9, window, cx| f(this, window, cx) }))
            .child(self.rail(cx))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .flex()
                    .flex_col()
                    .child(self.topbar(cx))
                    .child(div().flex_1().min_h_0().child(screen))
                    .child(self.status(cx)),
            )
            .children(self.toasts(cx))
            .children(self.decision_overlay(cx))
            .children(self.palette_overlay(cx))
    }
}
