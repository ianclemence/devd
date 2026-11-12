//! Presence: who is where, doing what, right now.
//!
//! devd is one person and a team of agents working on the same machine. The
//! window shows where each of them is the way a shared document shows cursors:
//! a face on the project an agent is working in, on the conversation it is
//! writing, on the file it is editing.
//!
//! Nothing here is invented or remembered. Presence is derived from what devd
//! already reports: a conversation's state, a team task's heartbeat, a tool
//! call that started and has not finished. An actor nobody has heard from for
//! [`STALE_AFTER`] is dropped, so a crashed worker fades from the screen
//! instead of looking busy for ever.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use serde::Serialize;

/// How long an actor stays shown after the last sign of life.
pub const STALE_AFTER: Duration = Duration::from_secs(90);

/// Somebody who can be somewhere.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Who {
    /// The person at the window.
    You,
    /// devd's own assistant, in a conversation.
    Assistant,
    /// A member of the team, by role id (`implementer`, `qa`).
    Member(String),
    /// An agent in another program, driving devd over MCP.
    Outside(String),
}

impl Who {
    /// The name shown beside a face.
    pub fn label(&self) -> String {
        match self {
            Self::You => "You".into(),
            Self::Assistant => "devd".into(),
            Self::Member(role) => {
                let mut name = role.replace(['-', '_'], " ");
                if let Some(first) = name.get_mut(0..1) {
                    first.make_ascii_uppercase();
                }
                name
            }
            Self::Outside(client) => client.clone(),
        }
    }

    /// One or two letters for a small face.
    pub fn initials(&self) -> String {
        let label = self.label();
        let mut words = label.split_whitespace();
        let first = words.next().and_then(|w| w.chars().next());
        let second = words.next().and_then(|w| w.chars().next());
        first.into_iter().chain(second).flat_map(char::to_uppercase).collect()
    }
}

/// Where somebody is.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize)]
pub struct Place {
    /// The project's directory, when they are in one.
    pub project: Option<String>,
    /// The conversation, when they are in one.
    pub thread: Option<String>,
    /// The team run, when they are part of one.
    pub run: Option<String>,
    /// The file they are reading or changing.
    pub file: Option<String>,
    /// The screen of the window, for the person.
    pub screen: Option<String>,
}

/// What somebody is doing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "detail")]
pub enum Doing {
    /// Looking, not changing anything.
    Viewing,
    /// Waiting for the model.
    Thinking,
    /// Writing a reply.
    Writing,
    /// Running a capability, by id.
    Calling(String),
    /// Stopped until a person decides something.
    Waiting(String),
}

impl Doing {
    /// A few words for a tooltip: "running project.run".
    pub fn phrase(&self) -> String {
        match self {
            Self::Viewing => "looking".into(),
            Self::Thinking => "thinking".into(),
            Self::Writing => "writing".into(),
            Self::Calling(capability) => format!("running {capability}"),
            Self::Waiting(reason) => format!("waiting: {reason}"),
        }
    }

    /// Whether to draw the face as active (a moving ring) or idle.
    pub fn active(&self) -> bool {
        !matches!(self, Self::Viewing | Self::Waiting(_))
    }
}

/// One actor's presence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Presence {
    /// Who.
    pub who: Who,
    /// Where.
    pub place: Place,
    /// Doing what.
    pub doing: Doing,
}

/// Everybody's presence, keyed by a stable id for the actor's session.
#[derive(Debug, Default)]
pub struct Room {
    present: HashMap<String, (Presence, Instant)>,
}

impl Room {
    /// An empty room.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a sign of life. `session` identifies one actor in one place: a
    /// conversation id for the assistant, a task id for a team member.
    pub fn seen(&mut self, session: &str, presence: Presence, now: Instant) {
        self.present.insert(session.to_string(), (presence, now));
    }

    /// The actor has finished or left.
    pub fn left(&mut self, session: &str) {
        self.present.remove(session);
    }

    /// Drop everybody not heard from recently. Returns whether anybody was dropped.
    pub fn sweep(&mut self, now: Instant) -> bool {
        let before = self.present.len();
        // The person is present for as long as the window is open.
        self.present.retain(|_, (p, at)| p.who == Who::You || now.duration_since(*at) < STALE_AFTER);
        self.present.len() != before
    }

    /// Everybody, the person first, then by name, so faces do not shuffle.
    pub fn everyone(&self) -> Vec<&Presence> {
        let mut all: Vec<&Presence> = self.present.values().map(|(p, _)| p).collect();
        all.sort_by_key(|p| (p.who != Who::You, p.who.label(), p.place.thread.clone()));
        all
    }

    /// Who is in a project, other than the person.
    pub fn in_project(&self, project: &str) -> Vec<&Presence> {
        self.everyone()
            .into_iter()
            .filter(|p| p.who != Who::You && p.place.project.as_deref().is_some_and(|dir| same_project(dir, project)))
            .collect()
    }

    /// Who is in a conversation, other than the person.
    pub fn in_thread(&self, thread: &str) -> Vec<&Presence> {
        self.everyone().into_iter().filter(|p| p.who != Who::You && p.place.thread.as_deref() == Some(thread)).collect()
    }

    /// How many agents are doing something right now.
    pub fn working(&self) -> usize {
        self.present.values().filter(|(p, _)| p.who != Who::You && p.doing.active()).count()
    }
}

/// Whether `dir` is `project` or inside it. A team member works in a worktree
/// under the project (`<project>/.devd/worktrees/…`), which is still that project.
fn same_project(dir: &str, project: &str) -> bool {
    let norm = |s: &str| s.replace('\\', "/").trim_end_matches('/').to_ascii_lowercase();
    let (dir, project) = (norm(dir), norm(project));
    dir == project || dir.starts_with(&format!("{project}/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(who: Who, project: &str, doing: Doing) -> Presence {
        Presence { who, place: Place { project: Some(project.into()), ..Place::default() }, doing }
    }

    #[test]
    fn faces_have_names_and_initials() {
        assert_eq!(Who::Member("security-engineer".into()).label(), "Security engineer");
        assert_eq!(Who::Member("security-engineer".into()).initials(), "SE");
        assert_eq!(Who::Assistant.initials(), "D");
        assert_eq!(Who::You.label(), "You");
    }

    #[test]
    fn a_worktree_is_inside_its_project() {
        let mut room = Room::new();
        let now = Instant::now();
        room.seen("t1", at(Who::Member("frontend".into()), r"D:\www\shop\.devd\worktrees\a1-shell", Doing::Calling("fs.edit".into())), now);
        room.seen("t2", at(Who::Member("qa".into()), "D:/www/shopfront", Doing::Thinking), now);
        let in_shop = room.in_project("D:/www/shop");
        assert_eq!(in_shop.len(), 1, "shopfront is a different project");
        assert_eq!(in_shop[0].who, Who::Member("frontend".into()));
        assert_eq!(in_shop[0].doing.phrase(), "running fs.edit");
    }

    #[test]
    fn a_silent_agent_fades_and_the_person_does_not() {
        let mut room = Room::new();
        let start = Instant::now();
        room.seen("me", at(Who::You, "D:/www/shop", Doing::Viewing), start);
        room.seen("t1", at(Who::Member("qa".into()), "D:/www/shop", Doing::Thinking), start);
        assert!(!room.sweep(start + Duration::from_secs(30)));
        assert_eq!(room.working(), 1);
        assert!(room.sweep(start + STALE_AFTER + Duration::from_secs(1)));
        assert_eq!(room.everyone().len(), 1);
        assert_eq!(room.everyone()[0].who, Who::You);
    }

    #[test]
    fn waiting_is_present_but_not_working() {
        let mut room = Room::new();
        room.seen("t1", at(Who::Assistant, "D:/www/shop", Doing::Waiting("approval".into())), Instant::now());
        assert_eq!(room.working(), 0);
        assert_eq!(room.in_project("d:\\www\\shop\\").len(), 1, "paths compare whatever their slashes and case");
        room.left("t1");
        assert!(room.everyone().is_empty());
    }

    #[test]
    fn the_person_is_listed_first_and_the_order_is_stable() {
        let mut room = Room::new();
        let now = Instant::now();
        room.seen("b", at(Who::Member("qa".into()), "p", Doing::Thinking), now);
        room.seen("a", at(Who::Assistant, "p", Doing::Writing), now);
        room.seen("me", at(Who::You, "p", Doing::Viewing), now);
        let names: Vec<String> = room.everyone().iter().map(|p| p.who.label()).collect();
        assert_eq!(names, ["You", "Qa", "devd"]);
    }
}
