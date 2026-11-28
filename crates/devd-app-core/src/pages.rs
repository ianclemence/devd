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
    /// Runs, and the recreations made from what a user can see.
    Runs,
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
            Area::Runs => Route::Runs(None),
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
            Area::Runs => "All runs",
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
    Page {
        id: "new-project",
        title: "New project",
        area: Area::Projects,
        summary: "Start from a stack, start empty, or add a folder you already have",
    },
    Page {
        id: "stacks",
        title: "Stacks",
        area: Area::Projects,
        summary: "Every template devd can create, and what each one makes",
    },
    Page {
        id: "hosts",
        title: "Hosts",
        area: Area::Sites,
        summary: "The names this machine answers to, and what is not published yet",
    },
    Page {
        id: "certificates",
        title: "Certificates",
        area: Area::Sites,
        summary: "HTTPS for every site, and whether this machine trusts devd's authority",
    },
    Page {
        id: "mail",
        title: "Mail",
        area: Area::Sites,
        summary: "What your sites tried to send, caught here instead of delivered",
    },
    Page {
        id: "recreate",
        title: "Recreate",
        area: Area::Runs,
        summary: "A study, a build and a check, kept as one record; clean room by default",
    },
    Page {
        id: "schedules",
        title: "Schedules",
        area: Area::Team,
        summary: "Work the team does on a timetable, and cron jobs for sites",
    },
    Page {
        id: "automations",
        title: "Automations",
        area: Area::Team,
        summary: "What devd does by itself when something happens",
    },
    Page {
        id: "skills",
        title: "Skills",
        area: Area::Team,
        summary: "The instructions agents load before a kind of work",
    },
    Page {
        id: "mcp",
        title: "MCP servers",
        area: Area::Team,
        summary: "Outside tools the assistant may use, and how far it may trust each",
    },
    Page {
        id: "notes",
        title: "Notes",
        area: Area::Team,
        summary: "What the team has learned and proposes to remember",
    },
    Page {
        id: "services",
        title: "Services",
        area: Area::Server,
        summary: "Nginx, MariaDB, Redis and the rest: what is needed, what is running, and what to start",
    },
    Page {
        id: "operations",
        title: "Operations",
        area: Area::Server,
        summary: "Restarts, upgrades and the checks that keep every service where it should be",
    },
    Page {
        id: "preview",
        title: "Preview",
        area: Area::Server,
        summary: "Open a site or a service in devd's own browser",
    },
    Page {
        id: "runtimes",
        title: "Runtimes",
        area: Area::Server,
        summary: "PHP, Node.js and Go: which versions are installed and in use",
    },
    Page {
        id: "databases",
        title: "Databases",
        area: Area::Server,
        summary: "MariaDB databases and their users, and the backups taken of them",
    },
    Page {
        id: "tools",
        title: "Tools",
        area: Area::Server,
        summary: "Git, Composer, editors and the other tools devd can install for you",
    },
    Page {
        id: "remote",
        title: "Remote",
        area: Area::Server,
        summary: "Share a site on the internet for a while, with a public link",
    },
    Page {
        id: "diagnostics",
        title: "Diagnostics",
        area: Area::Settings,
        summary: "What this machine is doing, what devd needs, and updates",
    },
    Page {
        id: "capabilities",
        title: "API and tools",
        area: Area::Settings,
        summary: "Everything devd can do, by name, as the assistant and scripts see it",
    },
    Page {
        id: "connect",
        title: "Connect agents",
        area: Area::Settings,
        summary: "Let Claude Code, Cursor and others use devd",
    },
    Page {
        id: "flags",
        title: "Flags",
        area: Area::Settings,
        summary: "Switches for new behaviour, and where each value came from",
    },
];

/// One screen on the rail, in the order it is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RailItem {
    /// What it says.
    pub label: &'static str,
    /// Its name in a path.
    pub segment: &'static str,
    /// The icon the window draws beside it (an `icons/<name>.svg` in the window's set).
    pub icon: &'static str,
    /// The area behind it, when it has pages.
    pub area: Option<Area>,
}

/// The rail, top to bottom. The window and the browser client both draw this list, so neither
/// can have a screen the other does not, or in a different order.
pub const RAIL: &[RailItem] = &[
    RailItem { label: "Home", segment: "home", icon: "home", area: None },
    RailItem { label: "Inbox", segment: "inbox", icon: "inbox", area: None },
    RailItem { label: "Assistant", segment: "assistant", icon: "assistant", area: None },
    RailItem { label: "Runs", segment: "runs", icon: "runs", area: Some(Area::Runs) },
    RailItem { label: "Team", segment: "team", icon: "team", area: Some(Area::Team) },
    RailItem { label: "Intelligence", segment: "intelligence", icon: "models", area: None },
    RailItem { label: "Projects", segment: "projects", icon: "folder", area: Some(Area::Projects) },
    RailItem { label: "Sites", segment: "sites", icon: "globe", area: Some(Area::Sites) },
    RailItem { label: "Server", segment: "server", icon: "machine", area: Some(Area::Server) },
    RailItem { label: "Activity", segment: "activity", icon: "clock", area: None },
    RailItem { label: "Settings", segment: "settings", icon: "settings", area: Some(Area::Settings) },
];

/// The rail as a script (`window.DEVD_RAIL`), for the page to draw it from the same list.
pub fn rail_script() -> String {
    let mut out = String::from("window.DEVD_RAIL=[");
    for (i, item) in RAIL.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        let area = match item.area {
            Some(a) => format!("{:?}", a.home().path()),
            None => "null".to_string(),
        };
        out.push_str(&format!(
            "{{\"label\":{:?},\"segment\":{:?},\"icon\":{:?},\"area\":{area}}}",
            item.label,
            item.segment,
            item.icon
        ));
    }
    out.push_str("];");
    out
}

/// The page with this id.
pub fn find(id: &str) -> Option<&'static Page> {
    PAGES.iter().find(|p| p.id == id)
}

/// The area a route is in, when it has pages behind it.
pub fn area_of(route: &Route) -> Option<Area> {
    match route {
        Route::Projects(_) | Route::Project(..) => Some(Area::Projects),
        Route::Sites(_) => Some(Area::Sites),
        // Runs has pages behind it (Recreate), so it is an area like the others: its screen and
        // its pages share a row of tabs.
        Route::Runs(_) => Some(Area::Runs),
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
    let mut out = vec![Tab {
        label: area.first_tab(),
        route: area.home(),
    }];
    out.extend(PAGES.iter().filter(|p| p.area == area).map(|p| Tab {
        label: p.title,
        route: Route::Page(p.id.to_string()),
    }));
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
            let route = Route::parse(&format!("/{}", page.id))
                .unwrap_or_else(|| panic!("{} cannot be reached by its path", page.id));
            assert_eq!(route, Route::Page(page.id.to_string()));
            assert_eq!(route.path(), format!("/{}", page.id));
            assert_eq!(route.title(), page.title);
            assert_eq!(area_of(&route), Some(page.area));
            assert!(
                tabs(page.area).iter().any(|t| t.route == route),
                "{} has no tab",
                page.id
            );
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
        assert!(
            Route::Home.on_rail(&Route::Home),
            "a screen with no pages is lit as itself"
        );
    }

    #[test]
    fn an_areas_tabs_start_with_its_own_screen_and_mark_the_current_one() {
        let tabs = tabs(Area::Sites);
        assert_eq!(tabs[0].route, Route::Sites(None));
        assert_eq!(
            tabs.iter().map(|t| t.label).collect::<Vec<_>>(),
            ["All sites", "Hosts", "Certificates", "Mail"]
        );
        let on_hosts = Route::Page("hosts".into());
        assert_eq!(tabs.iter().filter(|t| is_current(t, &on_hosts)).count(), 1);
        assert!(
            is_current(&tabs[0], &Route::Sites(Some("blog".into()))),
            "a site selected is still the sites tab"
        );
        assert!(!is_current(&tabs[0], &on_hosts));
    }

    #[test]
    fn an_unknown_page_is_not_a_route() {
        assert!(Route::parse("/not-a-page").is_none());
        assert!(find("not-a-page").is_none());
    }

    #[test]
    fn the_rail_is_every_screen_once_and_each_leads_somewhere() {
        // The window draws this list and the browser client draws the same one, so a screen added
        // to one is a screen in both.
        let mut labels: Vec<&str> = RAIL.iter().map(|r| r.label).collect();
        let n = labels.len();
        labels.sort_unstable();
        labels.dedup();
        assert_eq!(labels.len(), n, "a screen is listed twice");
        for item in RAIL {
            let route = Route::parse(item.segment).expect("a rail item names a screen");
            // Home is the root (`/`); every other screen is its segment.
        let path = if item.segment == "home" { "/".to_string() } else { format!("/{}", item.segment) };
        assert_eq!(route.path(), path, "{}", item.label);
            assert_eq!(route.title(), item.label, "{}", item.segment);
            assert_eq!(area_of(&route).is_some(), item.area.is_some(), "{}", item.label);
            assert!(!item.icon.is_empty(), "{} has no icon", item.label);
        }
        // Every area the pages belong to has a place on the rail.
        for area in [Area::Projects, Area::Sites, Area::Runs, Area::Team, Area::Server, Area::Settings] {
            assert!(RAIL.iter().any(|r| r.area == Some(area)), "{area:?} is not on the rail");
        }
        let script = rail_script();
        assert!(script.starts_with("window.DEVD_RAIL=["));
        assert!(script.contains("\"label\":\"Home\""));
        assert!(script.contains("\"segment\":\"projects\""));
        assert!(script.contains("\"area\":\"/projects\""));
    }
}
