//! Navigation: where the window is, as a value.
//!
//! A location is a [`Route`], and a route has one textual form (`/projects`,
//! `/assistant/8642e0b2`). That text is what a deep link carries, what the
//! window remembers between sessions, and what an agent passes to `app.navigate`,
//! so a person, a link and an agent all name a place the same way.

use serde::Serialize;

/// A place in the window.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case", tag = "screen", content = "id")]
pub enum Route {
    /// The overview: what needs you, what is running, recent work.
    Home,
    /// Approvals, questions and proposals waiting for a decision.
    Inbox,
    /// The assistant, on a conversation or on a new one.
    Assistant(Option<String>),
    /// Agent runs, on one run or the list.
    Runs(Option<String>),
    /// The team: roster, charter, schedules, skills.
    Team,
    /// Projects, on one project (by directory) or the list.
    Projects(Option<String>),
    /// Sites, on one site (by name) or the list.
    Sites(Option<String>),
    /// The server on this machine: services, runtimes, databases and tools.
    Server,
    /// Providers, models, which model does which work, and what it costs.
    Intelligence,
    /// Everything that happened: the audit trail.
    Activity,
    /// Preferences and diagnostics.
    Settings,
}

impl Route {
    /// The screens of the rail, in order, each with its unselected route.
    ///
    /// The order is the order of the questions: what needs me, who is doing the
    /// thinking (the assistant, its runs, the team, and the intelligence behind
    /// them), what am I building, and what is it running on.
    pub fn primary() -> [Route; 9] {
        [
            Route::Home,
            Route::Inbox,
            Route::Assistant(None),
            Route::Runs(None),
            Route::Team,
            Route::Intelligence,
            Route::Projects(None),
            Route::Sites(None),
            Route::Server,
        ]
    }

    /// The screen's name.
    pub fn title(&self) -> &'static str {
        match self {
            Self::Home => "Home",
            Self::Inbox => "Inbox",
            Self::Assistant(_) => "Assistant",
            Self::Runs(_) => "Runs",
            Self::Team => "Team",
            Self::Projects(_) => "Projects",
            Self::Sites(_) => "Sites",
            Self::Server => "Server",
            Self::Intelligence => "Intelligence",
            Self::Activity => "Activity",
            Self::Settings => "Settings",
        }
    }

    /// The first segment of the path, which also names the screen for an agent.
    pub fn segment(&self) -> &'static str {
        match self {
            Self::Home => "home",
            Self::Inbox => "inbox",
            Self::Assistant(_) => "assistant",
            Self::Runs(_) => "runs",
            Self::Team => "team",
            Self::Projects(_) => "projects",
            Self::Sites(_) => "sites",
            Self::Server => "server",
            Self::Intelligence => "intelligence",
            Self::Activity => "activity",
            Self::Settings => "settings",
        }
    }

    /// The thing selected on the screen, when one is.
    pub fn selection(&self) -> Option<&str> {
        match self {
            Self::Assistant(id) | Self::Runs(id) | Self::Projects(id) | Self::Sites(id) => id.as_deref(),
            _ => None,
        }
    }

    /// Whether two routes are the same screen, whatever is selected on it.
    pub fn same_screen(&self, other: &Route) -> bool {
        self.segment() == other.segment()
    }

    /// The route as text: `/sites/blog`.
    pub fn path(&self) -> String {
        match self.selection() {
            Some(id) => format!("/{}/{}", self.segment(), encode(id)),
            None => format!("/{}", self.segment()),
        }
    }

    /// Read a route from text. Unknown text is `None`, never a guess.
    pub fn parse(path: &str) -> Option<Route> {
        let trimmed = path.trim().trim_start_matches("devd://").trim_matches('/');
        let (segment, rest) = match trimmed.split_once('/') {
            Some((segment, rest)) => (segment, Some(decode(rest)).filter(|r| !r.is_empty())),
            None => (trimmed, None),
        };
        Some(match segment {
            "" | "home" => Route::Home,
            "inbox" => Route::Inbox,
            "assistant" | "chat" => Route::Assistant(rest),
            "runs" => Route::Runs(rest),
            "team" => Route::Team,
            "projects" => Route::Projects(rest),
            "sites" => Route::Sites(rest),
            // `machine` and `models` are what these screens were called before.
            "server" | "machine" | "environment" => Route::Server,
            "intelligence" | "models" | "providers" => Route::Intelligence,
            "activity" => Route::Activity,
            "settings" | "diagnostics" => Route::Settings,
            _ => return None,
        })
    }
}

/// Percent-encode what cannot sit in one path segment (a project is a directory).
fn encode(id: &str) -> String {
    let mut out = String::with_capacity(id.len());
    for byte in id.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(byte as char),
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

fn decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = (bytes[i] == b'%' && i + 2 < bytes.len()).then(|| text.get(i + 1..i + 3)).flatten();
        match hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
            Some(byte) => {
                out.push(byte);
                i += 3;
            }
            None => {
                out.push(bytes[i]);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Where the window has been, for Back and Forward.
#[derive(Debug, Clone)]
pub struct History {
    entries: Vec<Route>,
    at: usize,
}

/// How many places Back can return to.
const DEPTH: usize = 100;

impl History {
    /// A history that starts at `route`.
    pub fn new(route: Route) -> Self {
        Self { entries: vec![route], at: 0 }
    }

    /// Where the window is.
    pub fn current(&self) -> &Route {
        &self.entries[self.at]
    }

    /// Go somewhere. Anything Forward would have returned to is forgotten, and
    /// going to where you already are records nothing.
    pub fn go(&mut self, route: Route) -> bool {
        if self.current() == &route {
            return false;
        }
        self.entries.truncate(self.at + 1);
        self.entries.push(route);
        if self.entries.len() > DEPTH {
            self.entries.remove(0);
        }
        self.at = self.entries.len() - 1;
        true
    }

    /// Change what is selected on the current screen without adding a step, so
    /// clicking through a list does not fill Back with every row.
    pub fn select(&mut self, route: Route) {
        if self.current().same_screen(&route) {
            self.entries[self.at] = route;
        } else {
            self.go(route);
        }
    }

    /// Step back. Returns whether there was anywhere to go.
    pub fn back(&mut self) -> bool {
        let can = self.at > 0;
        if can {
            self.at -= 1;
        }
        can
    }

    /// Step forward again.
    pub fn forward(&mut self) -> bool {
        let can = self.at + 1 < self.entries.len();
        if can {
            self.at += 1;
        }
        can
    }

    /// Whether Back would do anything.
    pub fn can_back(&self) -> bool {
        self.at > 0
    }

    /// Whether Forward would do anything.
    pub fn can_forward(&self) -> bool {
        self.at + 1 < self.entries.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_route_reads_back_from_its_own_text() {
        let routes = [
            Route::Home,
            Route::Inbox,
            Route::Assistant(None),
            Route::Assistant(Some("8642e0b2-3201".into())),
            Route::Runs(Some("2d0a232b".into())),
            Route::Team,
            Route::Projects(Some(r"D:\devd\www\nairobi unwind".into())),
            Route::Sites(Some("blog".into())),
            Route::Server,
            Route::Intelligence,
            Route::Activity,
            Route::Settings,
        ];
        for route in routes {
            assert_eq!(Route::parse(&route.path()).as_ref(), Some(&route), "{}", route.path());
        }
    }

    #[test]
    fn a_directory_is_one_path_segment() {
        let path = Route::Projects(Some(r"D:\www\shop".into())).path();
        assert_eq!(path, "/projects/D%3A%5Cwww%5Cshop");
    }

    #[test]
    fn links_and_old_names_are_understood_and_nonsense_is_not() {
        assert_eq!(Route::parse("devd://sites/blog"), Some(Route::Sites(Some("blog".into()))));
        assert_eq!(Route::parse("/machine"), Some(Route::Server), "it was called Machine");
        assert_eq!(Route::parse("/models"), Some(Route::Intelligence), "it was called Models");
        assert_eq!(Route::parse(""), Some(Route::Home));
        assert_eq!(Route::parse("/nowhere"), None);
    }

    #[test]
    fn back_and_forward() {
        let mut h = History::new(Route::Home);
        assert!(!h.go(Route::Home), "going where you are records nothing");
        h.go(Route::Projects(None));
        h.go(Route::Server);
        assert!(h.back());
        assert_eq!(h.current(), &Route::Projects(None));
        assert!(h.can_forward());
        h.go(Route::Sites(None));
        assert!(!h.can_forward(), "a new step forgets the old future");
        assert!(h.back() && h.back());
        assert!(!h.back());
        assert_eq!(h.current(), &Route::Home);
    }

    #[test]
    fn clicking_through_a_list_is_one_step() {
        let mut h = History::new(Route::Home);
        h.go(Route::Sites(None));
        h.select(Route::Sites(Some("blog".into())));
        h.select(Route::Sites(Some("shop".into())));
        assert_eq!(h.current(), &Route::Sites(Some("shop".into())));
        assert!(h.back());
        assert_eq!(h.current(), &Route::Home);
    }

    #[test]
    fn the_rail_lists_each_screen_once() {
        let rail = Route::primary();
        for (i, a) in rail.iter().enumerate() {
            assert!(rail[i + 1..].iter().all(|b| !a.same_screen(b)));
            assert_eq!(Route::parse(&a.path()).as_ref(), Some(a));
        }
    }
}
