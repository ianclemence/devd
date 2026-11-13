//! The pages behind each screen of the rail.
//!
//! The rail is short on purpose: what needs me, the assistant, the team, projects, sites, the
//! server. Behind most of those are more pages (a site's hosts and certificates, the server's
//! runtimes and databases, the team's schedules and skills). They are grouped into **areas**, and
//! each area's screens carry the same row of tabs, so any page is one click from any other in its
//! area and nothing the interface can do is out of reach.
//!
//! This is only the table and the arithmetic of it, so that every page is declared once, every
//! page is reachable, and a test fails if one is added without a place to be found.

use crate::nav::Route;

/// A part of the rail that has pages behind it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Area {
    /// Projects, and the ways to make and work on one.
    Projects,
    /// Sites, and what serves and protects them.
    Sites,
    /// The team, and the work it does on its own.
    Team,
    /// The machine devd runs on.
    Server,
    /// Settings, diagnostics and what devd can do.
    Settings,
}

impl Area {
    /// The screen of the rail this area belongs to.
    pub fn home(self) -> Route {
        match self {
            Area::Projects => Route::Projects(None),
            Area::Sites => Route::Sites(None),
            Area::Team => Route::Team,
            Area::Server => Route::Server,
            Area::Settings => Route::Settings,
        }
    }

    /// What the area's own first tab is called.
    pub fn first_tab(self) -> &'static str {
        match self {
            Area::Projects => "All projects",
            Area::Sites => "All sites",
            Area::Team => "The team",
            Area::Server => "Overview",
            Area::Settings => "Settings",
        }
    }
}

/// One page.
#[derive(Debug, Clone, Copy)]
pub struct Page {
    /// Its name in a path and in the command menu.
    pub id: &'static str,
    /// What it is called.
    pub title: &'static str,
    /// Where it is found.
    pub area: Area,
    /// One line: what it is for.
    pub summary: &'static str,
}

/// Every page, in the order of the tabs.
pub const PAGES: &[Page] = &[
    Page { id: "new-project", title: "New project", area: Area::Projects, summary: "Start from a stack, start empty, or add a folder you already have" },
    Page { id: "stacks", title: "Stacks", area: Area::Projects, summary: "Every template devd can create, and what each one makes" },
    Page { id: "hosts", title: "Hosts", area: Area::Sites, summary: "The names this machine answers to, and what is not published yet" },
    Page { id: "certificates", title: "Certificates", area: Area::Sites, summary: "HTTPS for every site, and whether this machine trusts devd's authority" },
    Page { id: "mail", title: "Mail", area: Area::Sites, summary: "What your sites tried to send, caught here instead of delivered" },
    Page { id: "schedules", title: "Schedules", area: Area::Team, summary: "Work the team does on a timetable, and cron jobs for sites" },
    Page { id: "automations", title: "Automations", area: Area::Team, summary: "What devd does by itself when something happens" },
    Page { id: "skills", title: "Skills", area: Area::Team, summary: "The instructions agents load before a kind of work" },
    Page { id: "mcp", title: "MCP servers", area: Area::Team, summary: "Outside tools the assistant may use, and how far it may trust each" },
    Page { id: "notes", title: "Notes", area: Area::Team, summary: "What the team has learned and proposes to remember" },
    Page { id: "runtimes", title: "Runtimes", area: Area::Server, summary: "PHP, Node.js and Go: which versions are installed and in use" },
    Page { id: "databases", title: "Databases", area: Area::Server, summary: "MariaDB databases and their users, and the backups taken of them" },
    Page { id: "tools", title: "Tools", area: Area::Server, summary: "Git, Composer, editors and the other tools devd can install for you" },
    Page { id: "remote", title: "Remote", area: Area::Server, summary: "Share a site on the internet for a while, with a public link" },
    Page { id: "diagnostics", title: "Diagnostics", area: Area::Settings, summary: "What this machine is doing, what devd needs, and updates" },
    Page { id: "capabilities", title: "API and tools", area: Area::Settings, summary: "Everything devd can do, by name, as the assistant and scripts see it" },
    Page { id: "connect", title: "Connect agents", area: Area::Settings, summary: "Let Claude Code, Cursor and others use devd" },
    Page { id: "flags", title: "Flags", area: Area::Settings, summary: "Switches for new behaviour, and where each value came from" },
];

/// The page with this id.
pub fn find(id: &str) -> Option<&'static Page> {
    PAGES.iter().find(|p| p.id == id)
}

/// The area a route is in, when it has pages behind it.
pub fn area_of(route: &Route) -> Option<Area> {
    match route {
        Route::Projects(_) | Route::Project(..) => Some(Area::Projects),
        Route::Sites(_) => Some(Area::Sites),
        Route::Team => Some(Area::Team),
        Route::Server => Some(Area::Server),
        Route::Settings => Some(Area::Settings),
        Route::Page(id) => find(id).map(|p| p.area),
        _ => None,
    }
}

/// One tab of an area's row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tab {
    /// What it says.
    pub label: &'static str,
    /// Where it goes.
    pub route: Route,
}

/// The tabs of an area: its own screen first, then its pages.
pub fn tabs(area: Area) -> Vec<Tab> {
    let mut out = vec![Tab { label: area.first_tab(), route: area.home() }];
    out.extend(PAGES.iter().filter(|p| p.area == area).map(|p| Tab { label: p.title, route: Route::Page(p.id.to_string()) }));
    out
}

/// Whether a tab is the one on screen.
pub fn is_current(tab: &Tab, current: &Route) -> bool {
    match (&tab.route, current) {
        (Route::Page(a), Route::Page(b)) => a == b,
        (Route::Page(_), _) | (_, Route::Page(_)) => false,
        (a, b) => a.same_screen(b),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_page_is_reachable_by_its_path_and_belongs_to_an_area_with_a_tab_for_it() {
        for page in PAGES {
            let route = Route::parse(&format!("/{}", page.id)).unwrap_or_else(|| panic!("{} cannot be reached by its path", page.id));
            assert_eq!(route, Route::Page(page.id.to_string()));
            assert_eq!(route.path(), format!("/{}", page.id));
            assert_eq!(route.title(), page.title);
            assert_eq!(area_of(&route), Some(page.area));
            assert!(tabs(page.area).iter().any(|t| t.route == route), "{} has no tab", page.id);
        }
    }

    #[test]
    fn page_ids_are_unique_and_do_not_shadow_a_screen() {
        let mut ids: Vec<&str> = PAGES.iter().map(|p| p.id).collect();
        ids.sort_unstable();
        let n = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), n, "two pages share an id");
        for id in ids {
            let route = Route::parse(id).unwrap();
            assert!(matches!(route, Route::Page(_)), "{id} is already a screen");
        }
    }

    #[test]
    fn a_page_lights_up_its_areas_place_on_the_rail() {
        let hosts = Route::Page("hosts".into());
        assert!(hosts.on_rail(&Route::Sites(None)));
        assert!(Route::Page("runtimes".into()).on_rail(&Route::Server));
        assert!(!hosts.on_rail(&Route::Server));
        assert!(Route::Page("flags".into()).on_rail(&Route::Settings));
        assert!(Route::Home.on_rail(&Route::Home), "a screen with no pages is lit as itself");
    }

    #[test]
    fn an_areas_tabs_start_with_its_own_screen_and_mark_the_current_one() {
        let tabs = tabs(Area::Sites);
        assert_eq!(tabs[0].route, Route::Sites(None));
        assert_eq!(tabs.iter().map(|t| t.label).collect::<Vec<_>>(), ["All sites", "Hosts", "Certificates", "Mail"]);
        let on_hosts = Route::Page("hosts".into());
        assert_eq!(tabs.iter().filter(|t| is_current(t, &on_hosts)).count(), 1);
        assert!(is_current(&tabs[0], &Route::Sites(Some("blog".into()))), "a site selected is still the sites tab");
        assert!(!is_current(&tabs[0], &on_hosts));
    }

    #[test]
    fn an_unknown_page_is_not_a_route() {
        assert!(Route::parse("/not-a-page").is_none());
        assert!(find("not-a-page").is_none());
    }
}
