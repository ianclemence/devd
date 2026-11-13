//! Commands: everything the window can do, declared once.
//!
//! devd's first rule is that an action is declared once and every surface is a
//! view of it. The window follows the same rule for its own actions. Going to a
//! screen, opening the command menu, starting a conversation, toggling the
//! terminal: each is a [`Command`] in one [`Commands`] table. The command menu
//! lists that table, the keymap binds keys to it, the menus are built from it,
//! and an agent reads and runs it through `app.commands` and `app.run`.
//!
//! So nothing the window can do is reachable only by a mouse, and nothing an
//! agent can do in the window is hidden from a person: the same id, the same
//! title and the same effect, whoever asks.

use serde::Serialize;
use serde_json::Value;

use crate::fuzzy;

/// Where a command belongs, which is also how the menu groups it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Section {
    /// Moving between screens.
    Navigate,
    /// Starting something new.
    Create,
    /// The conversation in front of you.
    Conversation,
    /// The project in front of you.
    Project,
    /// Services, runtimes and the server on this machine.
    Machine,
    /// The window itself: panes, theme, zoom.
    View,
    /// A capability from the registry, run from the menu.
    Capability,
}

impl Section {
    /// The heading shown above the group.
    pub fn title(self) -> &'static str {
        match self {
            Self::Navigate => "Go to",
            Self::Create => "Create",
            Self::Conversation => "Conversation",
            Self::Project => "Project",
            Self::Machine => "Server",
            Self::View => "View",
            Self::Capability => "Capabilities",
        }
    }
}

/// One thing the window can do.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Command {
    /// Stable, dotted: `go.home`, `conversation.new`, `view.toggle_terminal`.
    pub id: String,
    /// What a person reads in the menu.
    pub title: String,
    /// One line saying what happens, shown under the title and to an agent.
    pub detail: String,
    /// The group it is listed under.
    pub section: Section,
    /// Other words a person might type for it.
    pub keywords: Vec<String>,
    /// The default key, in the keymap's notation (`ctrl-k`, `g h`).
    pub key: Option<String>,
    /// Whether it changes or removes something a person may want back.
    pub consequential: bool,
    /// Arguments it accepts, as a JSON Schema, when it takes any.
    pub args: Option<Value>,
}

impl Command {
    /// A command with an id, a title and what it does.
    pub fn new(id: &str, title: &str, detail: &str, section: Section) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            detail: detail.into(),
            section,
            keywords: Vec::new(),
            key: None,
            consequential: false,
            args: None,
        }
    }

    /// Bind a default key.
    pub fn key(mut self, key: &str) -> Self {
        self.key = Some(key.into());
        self
    }

    /// Add words that should also find it.
    pub fn keywords(mut self, words: &[&str]) -> Self {
        self.keywords = words.iter().map(|w| w.to_string()).collect();
        self
    }

    /// Mark it as something to confirm before running for an agent.
    pub fn consequential(mut self) -> Self {
        self.consequential = true;
        self
    }

    /// Declare the arguments it takes.
    pub fn args(mut self, schema: Value) -> Self {
        self.args = Some(schema);
        self
    }

    /// The text the menu matches a query against.
    fn haystack(&self) -> String {
        let mut text = format!("{}: {}", self.section.title(), self.title);
        for word in &self.keywords {
            text.push(' ');
            text.push_str(word);
        }
        text
    }
}

/// A command the menu is showing, with why it matched.
#[derive(Debug, Clone, PartialEq)]
pub struct Hit<'a> {
    /// The command.
    pub command: &'a Command,
    /// Byte offsets into the command's title to highlight.
    pub highlight: Vec<usize>,
}

/// The table of commands.
#[derive(Debug, Default, Clone)]
pub struct Commands {
    all: Vec<Command>,
    /// Ids of recently run commands, most recent first.
    recent: Vec<String>,
}

/// How many recently used commands lead the menu before anything is typed.
const RECENT: usize = 5;

impl Commands {
    /// An empty table.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a command. A second command with the same id replaces the first.
    pub fn add(&mut self, command: Command) {
        match self.all.iter_mut().find(|c| c.id == command.id) {
            Some(existing) => *existing = command,
            None => self.all.push(command),
        }
    }

    /// Every command, in the order declared.
    pub fn all(&self) -> &[Command] {
        &self.all
    }

    /// One command by id.
    pub fn get(&self, id: &str) -> Option<&Command> {
        self.all.iter().find(|c| c.id == id)
    }

    /// Record that a command ran, so the menu offers it first next time.
    pub fn used(&mut self, id: &str) {
        self.recent.retain(|r| r != id);
        self.recent.insert(0, id.to_string());
        self.recent.truncate(RECENT);
    }

    /// What the menu shows for a query, best first.
    ///
    /// With nothing typed, recently used commands come first and the rest follow
    /// in declared order. With a query, the ranking is by match quality, and a
    /// recently used command wins a near-tie.
    pub fn search(&self, query: &str, limit: usize) -> Vec<Hit<'_>> {
        let recency = |id: &str| self.recent.iter().position(|r| r == id);
        if query.trim().is_empty() {
            let mut ordered: Vec<&Command> = self.all.iter().collect();
            ordered.sort_by_key(|c| recency(&c.id).unwrap_or(usize::MAX));
            return ordered
                .into_iter()
                .take(limit)
                .map(|command| Hit {
                    command,
                    highlight: Vec::new(),
                })
                .collect();
        }
        let mut scored: Vec<(i32, usize, &Command, Vec<usize>)> = self
            .all
            .iter()
            .enumerate()
            .filter_map(|(i, command)| {
                // The title alone decides the highlight; the wider text decides
                // whether it matches at all (a keyword hit has nothing to light).
                let on_title = fuzzy::matches(query, &command.title);
                let wide = fuzzy::matches(query, &command.haystack());
                let best = match (&on_title, &wide) {
                    (Some(t), Some(w)) => t.score.max(w.score - 6),
                    (Some(t), None) => t.score,
                    (None, Some(w)) => w.score - 6,
                    (None, None) => return None,
                };
                let boost = recency(&command.id).map(|r| 6 - r as i32).unwrap_or(0);
                Some((
                    best + boost,
                    i,
                    command,
                    on_title.map(|m| m.positions).unwrap_or_default(),
                ))
            })
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        scored
            .into_iter()
            .take(limit)
            .map(|(_, _, command, highlight)| Hit { command, highlight })
            .collect()
    }

    /// The table as an agent reads it: every command with what it does.
    pub fn describe(&self) -> Value {
        serde_json::to_value(&self.all).unwrap_or(Value::Null)
    }
}

/// A key or a sequence of keys bound to a command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Binding {
    /// The keys, in order: `["ctrl-k"]`, or `["g", "h"]` for a sequence.
    pub keys: Vec<String>,
    /// The command it runs.
    pub command: String,
}

/// The default bindings of every command that declares a key.
pub fn bindings(commands: &Commands) -> Vec<Binding> {
    commands
        .all()
        .iter()
        .filter_map(|c| {
            let key = c.key.as_ref()?;
            Some(Binding {
                keys: key.split_whitespace().map(str::to_string).collect(),
                command: c.id.clone(),
            })
        })
        .collect()
}

/// Two commands bound to the same keys, which would make one of them unreachable.
pub fn conflicts(commands: &Commands) -> Vec<(String, String, String)> {
    let all = bindings(commands);
    let mut out = Vec::new();
    for (i, a) in all.iter().enumerate() {
        for b in &all[i + 1..] {
            let clash =
                a.keys == b.keys || a.keys.starts_with(&b.keys) || b.keys.starts_with(&a.keys);
            if clash {
                out.push((a.keys.join(" "), a.command.clone(), b.command.clone()));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table() -> Commands {
        let mut t = Commands::new();
        t.add(Command::new("go.home", "Home", "Open Home", Section::Navigate).key("g h"));
        t.add(
            Command::new("go.sites", "Sites", "Open Sites", Section::Navigate)
                .key("g s")
                .keywords(&["hosts", "domains"]),
        );
        t.add(
            Command::new(
                "go.machine",
                "Machine",
                "Services and runtimes",
                Section::Navigate,
            )
            .key("g m"),
        );
        t.add(
            Command::new(
                "conversation.new",
                "New conversation",
                "Start a conversation",
                Section::Create,
            )
            .key("ctrl-n"),
        );
        t.add(
            Command::new(
                "view.palette",
                "Command menu",
                "Open this menu",
                Section::View,
            )
            .key("ctrl-k"),
        );
        t
    }

    fn ids(hits: &[Hit<'_>]) -> Vec<String> {
        hits.iter().map(|h| h.command.id.clone()).collect()
    }

    #[test]
    fn a_keyword_finds_a_command_whose_title_does_not_contain_it() {
        let t = table();
        assert_eq!(ids(&t.search("domains", 5)), ["go.sites"]);
        assert!(
            t.search("domains", 5)[0].highlight.is_empty(),
            "nothing in the title matched"
        );
    }

    #[test]
    fn the_section_is_part_of_what_is_typed() {
        let t = table();
        assert_eq!(ids(&t.search("go ma", 3))[0], "go.machine");
    }

    #[test]
    fn recently_used_commands_lead_an_empty_menu() {
        let mut t = table();
        t.used("go.machine");
        t.used("view.palette");
        assert_eq!(
            ids(&t.search("", 3)),
            ["view.palette", "go.machine", "go.home"]
        );
    }

    #[test]
    fn a_title_match_is_highlighted_in_the_title() {
        let t = table();
        let hits = t.search("new", 3);
        assert_eq!(hits[0].command.id, "conversation.new");
        assert_eq!(hits[0].highlight, [0, 1, 2]);
    }

    #[test]
    fn redeclaring_a_command_replaces_it() {
        let mut t = table();
        t.add(Command::new(
            "go.home",
            "Start",
            "Open Home",
            Section::Navigate,
        ));
        assert_eq!(t.all().len(), 5);
        assert_eq!(t.get("go.home").unwrap().title, "Start");
    }

    #[test]
    fn a_sequence_that_begins_with_another_binding_is_a_conflict() {
        let mut t = table();
        assert!(conflicts(&t).is_empty());
        t.add(Command::new("git.open", "Git", "Open Git", Section::Navigate).key("g"));
        let found = conflicts(&t);
        assert_eq!(
            found.len(),
            3,
            "`g` shadows every `g x` sequence: {found:?}"
        );
    }

    #[test]
    fn an_agent_reads_the_same_table() {
        let t = table();
        let described = t.describe();
        assert_eq!(described[0]["id"], "go.home");
        assert_eq!(described[0]["section"], "navigate");
        assert_eq!(described[0]["key"], "g h");
    }
}
