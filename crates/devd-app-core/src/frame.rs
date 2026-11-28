//! What the window's frame says when a screen is in front: where you are, what is selected,
//! whether Back and Forward would go anywhere.
//!
//! The native window keeps one history and reads its crumb straight from it. The browser client is
//! a page with the browser's history: it reports the screen it is showing, and the server answers
//! with the same crumb and the same enabled arrows the window would draw (`GET /api/chrome`). The
//! rules are here, once, so "Team › Schedules" and a greyed Back mean the same in both.

use serde::Serialize;
use serde_json::Value;

use crate::nav::Route;

/// The frame, for a screen.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Frame {
    /// Which screen is lit on the rail: its segment (`projects` for a project's page).
    pub rail: String,
    /// What the top bar says first: the area's name when the screen is one of its pages, else the
    /// screen's own name.
    pub title: String,
    /// The area, when the screen is one of its pages: the part before the `›`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    /// What is selected on the screen, as a person reads it: a project's folder name, a site's host,
    /// a conversation's title.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selection: Option<String>,
    /// The address the frame describes, as it was given.
    pub path: String,
    /// The rail entry this screen belongs to, when it belongs to one: a page of an area lights the
    /// area's screen, as the window's rail does.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rail_entry: Option<String>,
}

/// The frame for `path`, with `selection` the name of whatever is selected there (a folder name, a
/// conversation's title): the page knows it, the route cannot always say it.
pub fn of(path: &str, selection: Option<&str>) -> Frame {
    let Some(route) = Route::parse(path) else {
        return Frame { rail: "home".into(), title: "Home".into(), parent: None, selection: None, path: path.to_string(), rail_entry: Some("home".into()) };
    };
    let rail = route.segment().to_string();
    // A page that is one tab of an area says the area first, "Team › Schedules", as the area's own
    // tab does: on its own, "Schedules" did not say where in the window you were.
    if let Route::Page(id) = &route {
        if let Some(page) = crate::pages::find(id) {
            return Frame {
                // The area's screen is what the rail lights for one of its pages.
                rail: page.area.home().segment().to_string(),
                title: page.title.to_string(),
                parent: Some(page.area.home().title().to_string()),
                selection: None,
                path: path.to_string(),
                rail_entry: Some(rail),
            };
        }
    }
    let selected = selection.map(str::to_string).or_else(|| route.selection().map(str::to_string));
    Frame {
        rail: rail.clone(),
        title: route.title().to_string(),
        parent: None,
        selection: selected,
        path: path.to_string(),
        rail_entry: Some(rail),
    }
}

/// The name a selected project is shown by: its folder, not its whole path.
pub fn select(value: &Value, field: &str) -> Option<String> {
    let v = value[field].as_str()?;
    Some(
        v.trim_end_matches(['/', '\\'])
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or(v)
            .to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_page_says_its_area_first_and_a_screen_says_its_own_name() {
        let f = of("/schedules", None);
        assert_eq!(f.title, "Schedules");
        assert_eq!(f.parent.as_deref(), Some("Team"));
        assert_eq!(f.rail, "team");
        let home = of("/", None);
        assert_eq!((home.title.as_str(), home.rail.as_str()), ("Home", "home"));
    }

    #[test]
    fn a_project_shows_its_folder_name_and_lights_projects() {
        let f = of(&format!("/projects/{}", crate::nav::encode(r"D:\www\luna")), None);
        assert_eq!(f.rail, "projects");
        assert_eq!(f.selection.as_deref(), Some(r"D:\www\luna"));
        let named = of(&format!("/projects/{}", crate::nav::encode(r"D:\www\luna")), Some("luna"));
        assert_eq!(named.selection.as_deref(), Some("luna"));
        assert_eq!(select(&serde_json::json!({ "p": r"D:\www\luna\" }), "p").as_deref(), Some("luna"));
    }
}
