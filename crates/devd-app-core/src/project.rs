//! The tabs of one project's page.
//!
//! A project page has six places: its overview, its files, its git history and changes, what it can run, a
//! shell in its folder, and what the team has to say about it. Git is large, so it is one place with its own
//! row of eight views. A tab is named in a path (`/projects/<folder>/changes`), so Back returns to it, a link
//! can point at it, and an agent can be told to go there.

use serde::Serialize;

/// A tab of a project's page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub enum ProjectTab {
    /// What the project is: stack, branch, what needs doing, what is running.
    Overview,
    /// The project's folder, as a tree, with files that can be read and changed.
    Files,
    /// Git: what has changed and is not committed.
    Changes,
    /// Git: the commits.
    History,
    /// Git: the branches.
    Branches,
    /// Git: work put aside.
    Stashes,
    /// Git: tags.
    Tags,
    /// Git: the remotes this repository talks to.
    Remotes,
    /// Git: other working folders of this repository.
    Worktrees,
    /// Git: where HEAD has been, the way back from a mistake.
    Recover,
    /// Tasks devd can run, and the processes it keeps going.
    Run,
    /// A shell in the project's folder.
    Terminal,
    /// To-dos, reviews and what the team remembers.
    Agents,
}

/// A place on the project page's first row. Git is several tabs behind one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Group {
    /// The overview.
    Overview,
    /// The files.
    Files,
    /// Everything about git.
    Git,
    /// Tasks and processes.
    Run,
    /// The shell.
    Terminal,
    /// The team's view.
    Agents,
}

impl Group {
    /// Every group, in the order of the row.
    pub const ALL: [Group; 6] = [
        Group::Overview,
        Group::Files,
        Group::Git,
        Group::Run,
        Group::Terminal,
        Group::Agents,
    ];

    /// What it says.
    pub fn label(self) -> &'static str {
        match self {
            Group::Overview => "Overview",
            Group::Files => "Files",
            Group::Git => "Git",
            Group::Run => "Run",
            Group::Terminal => "Terminal",
            Group::Agents => "Team",
        }
    }

    /// The tab it opens on: the first of its own, or the last one visited in the group.
    pub fn first(self) -> ProjectTab {
        match self {
            Group::Overview => ProjectTab::Overview,
            Group::Files => ProjectTab::Files,
            Group::Git => ProjectTab::Changes,
            Group::Run => ProjectTab::Run,
            Group::Terminal => ProjectTab::Terminal,
            Group::Agents => ProjectTab::Agents,
        }
    }
}

impl ProjectTab {
    /// Every tab.
    pub const ALL: [ProjectTab; 13] = [
        ProjectTab::Overview,
        ProjectTab::Files,
        ProjectTab::Changes,
        ProjectTab::History,
        ProjectTab::Branches,
        ProjectTab::Stashes,
        ProjectTab::Tags,
        ProjectTab::Remotes,
        ProjectTab::Worktrees,
        ProjectTab::Recover,
        ProjectTab::Run,
        ProjectTab::Terminal,
        ProjectTab::Agents,
    ];

    /// The git views, in the order of their row.
    pub const GIT: [ProjectTab; 8] = [
        ProjectTab::Changes,
        ProjectTab::History,
        ProjectTab::Branches,
        ProjectTab::Stashes,
        ProjectTab::Tags,
        ProjectTab::Remotes,
        ProjectTab::Worktrees,
        ProjectTab::Recover,
    ];

    /// Its name in a path.
    pub fn id(self) -> &'static str {
        match self {
            ProjectTab::Overview => "overview",
            ProjectTab::Files => "files",
            ProjectTab::Changes => "changes",
            ProjectTab::History => "history",
            ProjectTab::Branches => "branches",
            ProjectTab::Stashes => "stashes",
            ProjectTab::Tags => "tags",
            ProjectTab::Remotes => "remotes",
            ProjectTab::Worktrees => "worktrees",
            ProjectTab::Recover => "recover",
            ProjectTab::Run => "run",
            ProjectTab::Terminal => "terminal",
            ProjectTab::Agents => "team",
        }
    }

    /// What it says.
    pub fn label(self) -> &'static str {
        match self {
            ProjectTab::Overview => "Overview",
            ProjectTab::Files => "Files",
            ProjectTab::Changes => "Changes",
            ProjectTab::History => "History",
            ProjectTab::Branches => "Branches",
            ProjectTab::Stashes => "Stashes",
            ProjectTab::Tags => "Tags",
            ProjectTab::Remotes => "Remotes",
            ProjectTab::Worktrees => "Worktrees",
            ProjectTab::Recover => "Recover",
            ProjectTab::Run => "Run",
            ProjectTab::Terminal => "Terminal",
            ProjectTab::Agents => "Team",
        }
    }

    /// The tab with this name in a path. `agents` is what the team tab was called while it was being built.
    pub fn from_id(id: &str) -> Option<ProjectTab> {
        if id == "agents" {
            return Some(ProjectTab::Agents);
        }
        Self::ALL.into_iter().find(|t| t.id() == id)
    }

    /// The place on the first row this tab belongs to.
    pub fn group(self) -> Group {
        match self {
            ProjectTab::Overview => Group::Overview,
            ProjectTab::Files => Group::Files,
            ProjectTab::Changes
            | ProjectTab::History
            | ProjectTab::Branches
            | ProjectTab::Stashes
            | ProjectTab::Tags
            | ProjectTab::Remotes
            | ProjectTab::Worktrees
            | ProjectTab::Recover => Group::Git,
            ProjectTab::Run => Group::Run,
            ProjectTab::Terminal => Group::Terminal,
            ProjectTab::Agents => Group::Agents,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_tab_has_a_unique_name_a_label_and_one_group_that_lists_it() {
        let mut ids: Vec<&str> = ProjectTab::ALL.iter().map(|t| t.id()).collect();
        ids.sort_unstable();
        let n = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), n);
        for tab in ProjectTab::ALL {
            assert!(!tab.label().is_empty());
            assert_eq!(ProjectTab::from_id(tab.id()), Some(tab));
            assert!(Group::ALL.contains(&tab.group()), "{tab:?}");
        }
        assert_eq!(
            ProjectTab::GIT.len(),
            ProjectTab::ALL
                .iter()
                .filter(|t| t.group() == Group::Git)
                .count()
        );
        assert!(ProjectTab::GIT.iter().all(|t| t.group() == Group::Git));
    }

    #[test]
    fn a_group_opens_on_a_tab_that_belongs_to_it() {
        for group in Group::ALL {
            assert_eq!(group.first().group(), group, "{group:?}");
        }
        assert_eq!(
            ProjectTab::from_id("agents"),
            Some(ProjectTab::Agents),
            "the old name still finds the tab"
        );
        assert_eq!(ProjectTab::from_id("nope"), None);
    }
}
