//! Structural checks on the interface files. They cannot see pixels, but they
//! catch the regressions that actually happen: a sidebar entry that opens
//! nothing, an action with no handler, a logo asset with transparent padding.

use base64::Engine;

const APP: &str = include_str!("../ui/app.js");
const HTML: &str = include_str!("../ui/index.html");
const CSS: &str = include_str!("../ui/style.css");
const README: &str = include_str!("../../../README.md");

fn quoted_after(src: &str, marker: &str) -> Vec<String> {
    // every "word" inside the first [...] or {...} block following `marker`
    let start = src.find(marker).unwrap_or_else(|| panic!("{marker} not found"));
    let rest = &src[start..];
    let end = rest.find("];").or_else(|| rest.find("};")).unwrap();
    let block = &rest[..end];
    let mut out = Vec::new();
    let mut it = block.split('"');
    it.next();
    while let (Some(w), _) = (it.next(), it.next()) {
        out.push(w.to_string());
    }
    out
}

#[test]
fn the_interface_text_is_not_double_encoded() {
    // The UI is embedded with include_str!; one write through a Windows ANSI
    // code page can turn every ellipsis, middot and arrow into `Ã¢â‚¬Â¦` and ship
    // silently. These are the fingerprints of that mistake, plus U+FFFD.
    for (name, src) in [("app.js", APP), ("index.html", HTML), ("style.css", CSS)] {
        for bad in ["Ã", "Â", "â€", "ï»¿", "\u{FFFD}"] {
            assert!(
                !src.contains(bad),
                "{name} contains double-encoded text ({bad:?}); write UI files as UTF-8 without a BOM"
            );
        }
    }
}

#[test]
fn the_readme_is_not_double_encoded() {
    // The same ANSI round-trip that hit the interface files had reached the
    // README. The user reads it first.
    for (name, src) in [("README.md", README)] {
        for bad in ["Ã", "Â", "â€", "ï»¿", "\u{FFFD}"] {
            assert!(!src.contains(bad), "{name} contains double-encoded text ({bad:?})");
        }
    }
}

#[test]
fn every_destination_has_a_page() {
    let mut views: Vec<String> = Vec::new();
    // the rail
    for chunk in APP.split("{ view: \"").skip(1) {
        views.push(chunk.split('"').next().unwrap().to_string());
    }
    // tabs and the search palette's "go to" list
    for (start, end) in [("var TABS = {", "var AREA_HOME"), ("var GO_TO = [", "function palActions")] {
        let block = &APP[APP.find(start).unwrap()..];
        let block = &block[..block.find(end).unwrap()];
        for chunk in block.split("[\"").skip(1) {
            let name = chunk.split('"').next().unwrap();
            if chunk[name.len()..].starts_with("\", \"") && name.chars().all(|c| c.is_ascii_lowercase()) {
                views.push(name.to_string());
            }
        }
    }
    views.sort();
    views.dedup();
    assert!(views.len() > 15, "{views:?}");
    let render = &APP[APP.find("  function render() {").unwrap()..];
    let render = &render[..render.find("// ----").unwrap()];
    let server: Vec<String> = quoted_after(APP, "var SERVER_VIEWS = [");
    for v in views {
        let handled = render.contains(&format!("state.view === \"{v}\"")) || server.contains(&v) || v == "project";
        assert!(handled, "`{v}` is in the navigation but render() has no page for it");
    }
}

#[test]
fn every_action_in_the_markup_has_a_handler() {
    let handlers: Vec<&str> = APP.split("\n    ").filter_map(|l| {
        let l = l.trim_start_matches('"');
        l.split(['"', ':']).next().filter(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_lowercase() || c == '-'))
    }).collect();
    let mut used: Vec<String> = Vec::new();
    for src in [APP, HTML] {
        for chunk in src.split("data-action=\"").skip(1) {
            let name = chunk.split('"').next().unwrap();
            if name.chars().all(|c| c.is_ascii_lowercase() || c == '-') && !name.is_empty() {
                used.push(name.to_string());
            }
        }
    }
    used.sort();
    used.dedup();
    let dynamic = ["start-service", "stop-service"];
    for u in used {
        assert!(handlers.contains(&u.as_str()) || APP.contains(&format!("\"{u}\":")) || APP.contains(&format!("    {u}: function")) || dynamic.contains(&u.as_str()), "data-action=\"{u}\" has no handler");
    }
}

#[test]
fn the_sidebar_is_short_and_search_is_not_about_capabilities() {
    assert!(!HTML.contains("Search capabilities"));
    assert!(APP.contains("id=\"health-chip\"") && !HTML.contains("health-pill") && !HTML.contains("health-chip"), "the health chip lives in the footer");
    // One rail of destinations; an area's own pages are the context panel's list, not
    // more rail entries or a second bar of tabs.
    assert!(HTML.contains("id=\"rail\"") && HTML.contains("id=\"panel\""));
    let rail = &APP[APP.find("function rail(force)").unwrap()..];
    let rail = &rail[..rail.find("function item(i, small)").unwrap()];
    let items = rail.matches("{ view: \"").count();
    assert!(items <= 14, "{items} rail entries; related pages belong in the panel");
    assert!(CSS.contains(".panel-item"));
}

#[test]
fn project_and_site_lists_do_not_repeat_the_counts_above_them() {
    // The Changes view drew "1 untracked" pills directly above a group header
    // that said "UNTRACKED 1". Hosts and Certificates repeated their summary
    // count in the section heading. A section says what it is; the summary and
    // the rows already carry the numbers.
    assert!(!APP.contains("change-summary"), "the change groups own their counts");
    assert!(!APP.contains("var summary = total"), "there is no second summary row");
    assert!(APP.contains("uiHead(\"Published names\", null)"), "Hosts does not repeat its count");
    assert!(APP.contains("uiHead(\"Issued certificates\", null)"), "Certificates does not repeat its count");
    // Branches used an empty bordered tag as a spacer, which looked like an
    // unchecked checkbox on every row.
    assert!(APP.contains("class=\"branch-name-cell\""), "the current pill stays beside the branch name");
    assert!(!APP.contains("'<span class=\"tag\"> </span>'"), "no empty tag boxes");
    // A site is named without its default port; the full URL stays in the link.
    assert!(APP.contains("function displayUrl"), "site addresses drop the default port when displayed");
}

#[test]
fn interaction_states_use_the_shared_motion_language() {
    // Tabs are pressed often, so the feedback is a short scale, not a slide.
    assert!(CSS.contains(".subnav .subtab:active,"), "the tab strip has press feedback");
    assert!(CSS.contains("transform 120ms var(--ease-out)"), "tab movement is short and eased");
    // A popover grows from its trigger, not from the middle of the viewport.
    assert!(APP.contains("node.style.transformOrigin ="), "the composer popover knows its trigger");
    assert!(CSS.contains("@keyframes popover-in"), "the popover has one entrance");
    assert!(CSS.contains("animation: popover-in 140ms var(--ease-out);"), "the entrance stays under 200ms");
    // Exact properties only: `background` would also animate images and paint.
    assert!(!CSS.contains("transition: all"), "no transition: all");
    assert!(!CSS.contains("transition: background var("), "transition background-color, not background");
}

#[test]
fn tables_keep_one_grid_and_inline_rows_stay_on_one_line() {
    // The branch list is a real table: named columns, a fixed grid, local
    // branches before remote-tracking refs, and one per-row action menu.
    assert!(APP.contains("class=\"table branch-table\""), "branches use the shared table");
    assert!(APP.contains("class=\"branch-name-cell\""), "the current pill stays beside the name");
    assert!(APP.contains("class=\"cell-actions\">Actions</th>"), "the actions column is named");
    assert!(CSS.contains(".branch-table { table-layout: fixed; }"), "branch columns do not drift");
    assert!(
        !APP.contains("'<span class=\"tag\"> </span>'"),
        "no branch is marked by an empty tag box"
    );
    // Markdown tables use the same header and padding as a panel table.
    assert!(CSS.contains(".md th, .md td { padding: 10px var(--s4) 10px 0;"), "Markdown tables share the grid");
    assert!(!CSS.contains(".md tbody tr:nth-child(even)"), "no second striping language");
    // A flex row must not inherit the margin used between stacked blocks.
    assert!(
        CSS.contains(".panel-body.inline > * + * { margin-top: 0; }"),
        "inline controls line up on one baseline"
    );
    // A data-table head is one line. The action column must never break its
    // word into one letter per row in a 1%-wide cell.
    assert!(
        CSS.contains(":is(.table, .mu, .drift-table) :is(th, td).cell-actions {\n  display: table-cell;\n  width: 1%;\n  text-align: right;\n  white-space: nowrap;"),
        "action headers are table cells, right aligned and nowrap"
    );
    assert!(
        CSS.contains("vertical-align: middle;\n  white-space: nowrap;") && CSS.contains(".md th { white-space: nowrap;"),
        "every table head stays on one line"
    );
}

#[test]
fn every_tab_strip_speaks_the_same_visual_language() {
    // Section tabs, project groups and sub-tabs, site/remote tabs, mail views,
    // activity filters, Form/JSON, and terminal sessions are one control: a row
    // of quiet pills with the current one raised. A new strip joins this list
    // instead of inventing its own height, radius and border.
    for selector in [
        ".subnav .subtab,",
        ".tabs .tab,",
        ".subtabs .subtab-pill,",
        ".segmented .seg,",
        ".ui-sort,",
        ".term-tabs .term-tab {",
    ] {
        assert!(CSS.contains(selector), "{selector} must share the tab styling");
    }
    // One shell needs no tab above it: the panel head already names the folder
    // and the shell. Session tabs appear only when there is a choice.
    assert!(APP.contains("var tabs = sessions.length > 1 ?"), "a single terminal session has no tab");
    // The group row and the sub-tab row are two strips, not two items in one
    // flex row. Wrapping them again put every tab on a single line.
    assert!(
        !APP.contains("'<div class=\"tabs\">' + tabs + \"</div>\""),
        "the project tab strips must not be nested"
    );
}

#[test]
fn every_icon_comes_from_the_same_logo() {
    // The artwork has one source: `assets/devd-logo.png`. Everything else is
    // generated from it by `scripts/make-icons.py`, so the mark in the window,
    // the icon on the taskbar and the favicon are the same picture.
    let desktop = include_bytes!("../../devd-desktop/icons/128x128.png");
    let marker = "data:image/png;base64,";
    let start = HTML.find(marker).expect("inline logo") + marker.len();
    let end = HTML[start..].find('"').expect("inline logo end") + start;
    let inline = &HTML[start..end];
    let encoded = base64::engine::general_purpose::STANDARD.encode(desktop);
    assert_eq!(inline, encoded, "the in-app mark is not the desktop icon");

    assert_eq!(&desktop[1..4], b"PNG");
    let width = u32::from_be_bytes(desktop[16..20].try_into().unwrap());
    let height = u32::from_be_bytes(desktop[20..24].try_into().unwrap());
    assert_eq!((width, height), (128, 128));
    assert_eq!(desktop[25], 2, "the artwork is opaque: colour type 2 (RGB)");
}

#[test]
fn every_element_the_script_looks_up_exists_in_the_markup() {
    // `el("id")` returns null for an id that is not in the page, and the line
    // that used it then throws - which takes the whole screen down rather than
    // the one control that is missing. Every id the script reaches for has to
    // be in the markup.
    let mut wanted: Vec<String> = Vec::new();
    let bytes = APP.as_bytes();
    let needle = b"el(\"";
    let mut at = 0usize;
    while let Some(found) = APP[at..].find("el(\"") {
        let start = at + found;
        at = start + needle.len();
        // `sel("x")` ends in `el("` too, so the call has to stand on its own.
        let before = if start == 0 { b' ' } else { bytes[start - 1] };
        if before.is_ascii_alphanumeric() || before == b'_' {
            continue;
        }
        let name = APP[at..].split('"').next().unwrap_or("");
        // `el("prefix-" + id)` is a template, not one element, so a name that
        // ends in a dash is not a real id.
        if !name.is_empty()
            && !name.ends_with('-')
            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        {
            wanted.push(name.to_string());
        }
    }
    wanted.sort();
    wanted.dedup();
    for id in &wanted {
        // Some elements are built by the script itself - a drawer, a row - so
        // their ids appear in the markup of the script rather than the page.
        let in_page = HTML.contains(&format!("id=\"{id}\""));
        let in_script = APP.contains(&format!("id=\"{id}\""));
        assert!(
            in_page || in_script,
            "the script looks up `el(\"{id}\")` but nothing ever creates an element with that id"
        );
    }
}

#[test]
fn inventory_does_not_show_services_devd_manages_or_programs_that_are_not_there() {
    // devd's MariaDB is mysqld.exe on Windows. The host inventory sees it
    // holding port 3306 and would list MariaDB and MySQL as "Not installed"
    // next to the managed MariaDB row. Missing programs also belong under
    // Available, not under "Already on this computer".
    let start = APP.find("  function visibleInventory()").expect("visibleInventory");
    let end = start + APP[start..].find("  function renderServices()").expect("renderServices");
    let body = &APP[start..end];
    assert!(APP.contains("function managedServicePid(pid)"), "inventory needs to match a port owner against managed services");
    assert!(body.contains("managedServicePid(v.port_owner.pid)"), "managed services must not appear in the host inventory");
    assert!(
        body.contains("!(v.status === \"missing\" && !v.path && !v.port_owner)"),
        "missing programs must not be counted as already on this computer"
    );
    assert!(APP.contains("  function inventoryRows("), "inventory rows are built from the visible list");
}

#[test]
fn services_come_from_one_registry_catalogue() {
    // The Services screen used a third static list with download flags. The
    // catalogue capability is now the only source of install/adopt/find.
    assert!(!APP.contains("var KNOWN_SERVICES"), "no static service catalogue");
    assert!(APP.contains("attempt(\"service.catalogue\")"), "the screen reads the catalogue capability");
    assert!(APP.contains("data-action=\"service-find\""), "an existing folder can be found");
    assert!(APP.contains("data-action=\"service-adopt\""), "a found folder can be adopted");
}

#[test]
fn remote_servers_use_the_universal_ssh_feature() {
    // There is one remote-server model: a registered SSH machine. No separate
    // control-panel integration and no product names from other local stacks.
    for gone in ["ploi", "Ploi", "XAMPP", "WampServer"] {
        assert!(!APP.contains(gone), "{gone} must not appear in the interface");
    }
    assert!(APP.contains("data-action=\"ops-srv-add\""), "servers are added from the Servers tab");
    assert!(APP.contains("server.limits_set"), "server PHP limits belong to the Servers tab");
}

#[test]
fn server_and_site_tabs_match_the_agreed_information_architecture() {
    // The server strip is the whole machine. A site's own controls belong on
    // its page, reached by Manage from the Sites list, not on this strip.
    let server = APP
        .split("server: [[")
        .nth(1)
        .expect("server tabs")
        .split("]]")
        .next()
        .unwrap();
    let labels = ["Overview", "Services", "Runtimes", "Databases", "Tools", "Remote"];
    // (The tabs are now the context panel's list; their names and order are the same.)
    let mut at = 0;
    for label in labels {
        let needle = format!("\"{label}\"");
        let found = server[at..]
            .find(&needle)
            .unwrap_or_else(|| panic!("{label} is missing or out of order in the environment tabs"));
        at += found + needle.len();
    }
    assert!(!server.contains("\"sites\""), "Sites is a top-level destination, not an Environment tab");
    assert!(!server.contains("\"operations\""), "Operations is a fallback route, not a visible Environment tab");
    assert!(!server.contains("\"preview\""), "Preview belongs to the site page");

    let sites = APP
        .split("sites: [[")
        .nth(1)
        .expect("site tabs")
        .split("]]")
        .next()
        .unwrap();
    let mut at = 0;
    for label in ["All sites", "Hosts", "Certificates", "Mail"] {
        let needle = format!("\"{label}\"");
        let found = sites[at..]
            .find(&needle)
            .unwrap_or_else(|| panic!("{label} is missing or out of order in the Sites tabs"));
        at += found + needle.len();
    }
    assert!(APP.contains("{ view: \"sites\", area: \"sites\", label: \"Sites\""), "Sites is a first-class rail destination");
    assert!(APP.contains("page(\"Environment\""), "the environment screen does not call itself a single server");

    let site = quoted_after(APP, "var SITE_TABS = [");
    assert_eq!(
        site,
        vec![
            "overview", "Overview", "env", "Environment", "deploy", "Deploy", "cron", "Cron",
            "queue", "Queue", "logs", "Logs", "laravel", "Laravel", "redirects", "Redirects",
            "nginx", "Web server", "db", "Database", "mail", "Mail", "preview", "Preview"
        ]
    );
    assert!(APP.contains("data-action=\"open-site\" data-name="), "the Sites list needs a Manage action");
    assert!(APP.contains("function renderSite()"));
    assert!(APP.contains("function renderDatabases()"));
    assert!(APP.contains("function renderRemote()"));
    assert!(APP.contains("function machineSummary(m)"), "the server Overview has a machine summary");
    assert!(APP.contains("read(\"monitor.stats\""), "the machine summary is read from monitor.stats");

    let sites = APP.find("function renderSites()").expect("renderSites");
    let sites = &APP[sites..sites + APP[sites..].find("function siteDatabase(row)").expect("siteDatabase")];
    assert!(!sites.contains("data-action=\"remove-site\""), "Remove lives on the site page, not the list");
    assert!(!sites.contains("data-action=\"share-site\""), "Share lives on the site page, not the list");

    let db = APP.find("function siteDatabase(row)").expect("siteDatabase");
    let db = &APP[db..db + APP[db..].find("function sitePreview(row)").expect("sitePreview")];
    assert!(db.contains("belong to the whole environment"));
    assert!(!db.contains("data-action=\"ops-backup-run\""), "site Database is read-only");
    assert!(!db.contains("data-action=\"ops-backup-remove\""));
}

#[test]
fn tools_and_runtimes_do_not_offer_the_same_install() {
    let start = APP.find("function toolOwnedElsewhere(t)").expect("Tools ownership filter");
    let end = start + APP[start..].find("function loadToolchain()").expect("loadToolchain");
    let body = &APP[start..end];
    assert!(body.contains("t.tool.category === \"database\""));
    assert!(body.contains("t.tool.category === \"container\""));
    assert!(body.contains("t.tool.id === \"nginx\""));
    assert!(body.contains("TOOL_RUNTIME_IDS[t.tool.id]"));
    assert!(APP.contains("opsPhp(OPS.data.php, false)"), "Runtimes shows PHP settings but not the per-site profile table");
    assert!(APP.contains("Notifications</summary>"), "alerts live in Diagnostics");
}

#[test]
fn site_detail_has_an_address_and_its_own_preview() {
    assert!(APP.contains("return SITE.name ? \"/sites/\" + encodeURIComponent(SITE.name) : \"/sites\";"));
    assert!(APP.contains("if (p.indexOf(\"/sites/\") === 0) return \"site\";"));
    assert!(APP.contains("if (p.indexOf(\"/server/sites/\") === 0) return \"site\";"), "old site links still resolve");
    assert!(APP.contains("data-action=\"site-tab\" data-tab=\"preview\""));
    assert!(APP.contains("function sitePreview(row)"));
    assert!(APP.contains("function siteDatabase(row)"));
}

#[test]
fn every_action_handler_does_something() {
    // A handler that is empty is worse than no handler: the control looks
    // real, the click does nothing, and the default action is swallowed.
    let mut offenders: Vec<String> = Vec::new();
    for line in APP.split("\n    ").filter(|l| l.trim_start().starts_with('"')) {
        let name = line.trim_start_matches('"').split('"').next().unwrap_or("").to_string();
        if name.is_empty() {
            continue;
        }
        if let Some(at) = APP.find(&format!("\"{name}\": function (")) {
            let body = &APP[at..];
            let end = body.find("\n    },").unwrap_or(body.len());
            let body = &body[..end];
            let code = body
                .split_once("function (")
                .map(|(_, rest)| rest.split_once(')').map(|(_, after)| after).unwrap_or(""))
                .unwrap_or("");
            if code.trim().trim_end_matches('{').trim().is_empty() {
                offenders.push(name);
            }
        }
    }
    assert!(offenders.is_empty(), "empty action handlers: {offenders:?}");
}

#[test]
fn every_custom_property_the_stylesheet_uses_is_defined() {
    // A `var(--name)` with no definition is not an error in CSS: the property
    // silently falls back to its initial value. That is how a brand mark ends
    // up filling a whole card, and nothing complains.
    let mut defined: Vec<String> = Vec::new();
    // A definition is `--name:` at the start of a declaration, wherever it sits on its
    // line (several tokens often share one line, and a rule may set one inline).
    let bytes = CSS.as_bytes();
    let mut at = 0;
    while let Some(off) = CSS[at..].find("--") {
        let start = at + off;
        at = start + 2;
        let before = CSS[..start].trim_end().chars().last();
        if !matches!(before, None | Some('{') | Some(';') | Some('}') | Some('/')) {
            continue;
        }
        let name: String = CSS[start + 2..]
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '-')
            .collect();
        let after = start + 2 + name.len();
        if !name.is_empty() && bytes.get(after) == Some(&b':') {
            defined.push(name);
        }
    }
    let mut used: Vec<String> = Vec::new();
    for chunk in CSS.split("var(--") {
        if chunk == CSS.split("var(--").next().unwrap_or("") {
            continue;
        }
        let name: String = chunk
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '-')
            .collect();
        if !name.is_empty() {
            used.push(name);
        }
    }
    used.sort();
    used.dedup();
    let mut missing: Vec<&String> = Vec::new();
    for name in &used {
        if !defined.contains(name) {
            missing.push(name);
        }
    }
    assert!(missing.is_empty(), "these custom properties are used but never defined: {missing:?}");
}

#[test]
fn the_stylesheet_is_balanced() {
    // A rule with a stray brace swallows everything after it, and a browser
    // says nothing: the stylesheet simply stops applying. This check is the
    // only thing standing between a typo and half the interface losing its
    // layout, including every responsive breakpoint.
    let mut depth: i32 = 0;
    for (number, line) in CSS.lines().enumerate() {
        for ch in line.chars() {
            match ch {
                '{' => depth += 1,
                '}' => depth -= 1,
                _ => {}
            }
            assert!(depth >= 0, "the stylesheet closes a rule it never opened at line {}", number + 1);
        }
    }
    assert_eq!(depth, 0, "the stylesheet ends with an unclosed rule");
}

#[test]
fn the_folder_picker_is_the_normal_way_to_choose_a_project() {
    // A person must never have to type a path to add a project. The Projects
    // screen offers the picker, and the only place a path is typed is the
    // picker's own advanced field.
    let projects = &APP[APP.find("function renderProjects()").unwrap()..];
    let projects = &projects[..projects.find("function loadProjects(").unwrap()];
    assert!(
        projects.contains("data-action=\"add-project\""),
        "the Projects screen has no Add existing project action"
    );
    assert!(
        !projects.contains("id=\"add-path\""),
        "the Projects screen still asks for a typed path"
    );
    assert!(
        HTML.contains("id=\"picker\""),
        "the folder picker is missing from the page"
    );
    for id in ["picker-title", "picker-note", "picker-up", "picker-crumbs", "picker-places", "picker-list", "picker-inspect", "picker-path-input", "picker-choose", "picker-native"] {
        assert!(HTML.contains(&format!("id=\"{id}\"")), "the picker is missing `#{id}`");
    }
}

#[test]
fn the_logo_has_no_transparent_padding_and_is_square() {
    let marker = "data:image/png;base64,";
    let start = HTML.find(marker).expect("inline logo") + marker.len();
    let b64: String = HTML[start..].chars().take_while(|c| *c != '"').collect();
    let png = base64::engine::general_purpose::STANDARD.decode(b64).unwrap();
    assert_eq!(&png[1..4], b"PNG");
    let w = u32::from_be_bytes(png[16..20].try_into().unwrap());
    let h = u32::from_be_bytes(png[20..24].try_into().unwrap());
    assert_eq!(w, h, "the logo is square, so a square box shows all of it");
    assert!(w >= 80, "crisp at 40 CSS px on a 2x display");
    // PNG colour type 2 is RGB: no alpha channel, so no transparent frame can exist.
    assert_eq!(png[25], 2, "the logo asset has no alpha channel");
    let rule = &CSS[CSS.find(".top-mark {").unwrap()..];
    let rule = &rule[..rule.find('}').unwrap()];
    assert!(rule.contains("object-fit: cover") && !rule.contains("padding") && !rule.contains("margin") && !rule.contains("box-shadow"), "{rule}");
}

#[test]
fn no_function_in_the_script_is_declared_twice() {
    // JavaScript lets the later declaration silently replace the earlier one, which is how a
    // "click a project" handler once ended up running another screen's function of the same name.
    let mut seen = std::collections::HashMap::new();
    for (n, line) in APP.lines().enumerate() {
        if let Some(rest) = line.strip_prefix("  function ") {
            let name: String = rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
            if let Some(first) = seen.insert(name.clone(), n + 1) {
                panic!("function `{name}` is declared at line {first} and again at line {}", n + 1);
            }
        }
    }
}

#[test]
fn every_navigation_button_points_at_a_page() {
    let render = &APP[APP.find("  function render() {").unwrap()..];
    let render = &render[..render.find("// ----").unwrap()];
    let server: Vec<String> = quoted_after(APP, "var SERVER_VIEWS = [");
    let mut missing = Vec::new();
    for chunk in APP.split("data-view=\\\"").chain(APP.split("data-view=\"")).skip(1) {
        let name: String = chunk.chars().take_while(|c| c.is_ascii_lowercase() || *c == '-').collect();
        if name.is_empty() || name == "project" {
            continue;
        }
        if !(render.contains(&format!("state.view === \"{name}\"")) || server.contains(&name)) && !missing.contains(&name) {
            missing.push(name);
        }
    }
    assert!(missing.is_empty(), "a button navigates to a view render() cannot draw: {missing:?}");
}

#[test]
fn every_helper_the_script_calls_is_defined() {
    // A helper deleted with the feature that used it, while another screen still called it, left
    // every conversation blank. These are the small shared helpers screens lean on.
    for name in ["shortTime", "stamp", "ago", "esc", "attr", "md", "uiRow", "uiList", "uiHead", "uiEmpty", "uiSummary", "uiSheet", "uiDialog", "uiConfirm", "uiPrompt", "devdSeries", "dot", "pill", "page"] {
        let used = APP.contains(&format!("{name}("));
        let defined = APP.contains(&format!("function {name}("));
        assert!(!used || defined, "`{name}` is called but never defined");
    }
}

#[test]
fn the_script_is_valid_javascript() {
    // A string escape lost in an edit left the whole window blank while every structural check passed. When Node is
    // installed it parses the file; on a machine without it the check says so instead of passing quietly.
    let dir = std::env::temp_dir().join(format!("devd-ui-parse-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("app.js");
    std::fs::write(&file, APP).unwrap();
    match std::process::Command::new("node").arg("--check").arg(&file).output() {
        Ok(out) => assert!(out.status.success(), "app.js does not parse:\n{}", String::from_utf8_lossy(&out.stderr)),
        Err(_) => eprintln!("node is not installed: app.js was not parsed"),
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn row_actions_are_visible_without_hover_on_touch_and_narrow_windows() {
    // A phone cannot hover. The rows use a hover-only reveal for Manage, Open,
    // Stop and Remove, so without an explicit touch rule those controls are in
    // the document but invisible and unreachable.
    // The stylesheet is compared without its line endings. A Windows checkout
    // uses CRLF and a Linux one LF, and a test that hard-codes one of them fails
    // on the other platform for a reason that has nothing to do with the rule.
    let css = CSS.replace("\r\n", "\n");
    assert!(
        css.contains("@media (hover: none) {\n  .ui-row-actions.hover { opacity: 1; }\n}"),
        "touch screens need the row actions visible"
    );
    let narrow = &css[css.find("@media (max-width: 720px)").expect("the narrow layout")..];
    assert!(
        narrow.contains(".ui-row { flex-wrap: wrap; row-gap: var(--s2); }"),
        "the row wraps on a phone"
    );
    assert!(
        narrow.contains(".ui-row-actions, .ui-row-actions.hover { flex-basis: 100%; opacity: 1;"),
        "the actions get their own line on a phone"
    );
}

#[test]
fn the_setup_sheet_reads_the_runtime_kind_where_runtime_list_puts_it() {
    // runtime.list returns each runtime as { runtime: { kind, ... } }. Reading
    // `kind` off the row itself never matches, so a machine that already had
    // PHP was told to install it again and the sheet stopped before nginx.
    for line in APP.lines().filter(|l| l.contains("runtime.install")) {
        assert!(
            !line.contains("r.kind === \"php\""),
            "a runtime row keeps its kind under `runtime`: {line}"
        );
    }
    assert!(APP.contains("r.runtime && r.runtime.kind === \"php\""), "the sheet skips PHP that is already installed");
}

#[test]
fn the_site_screens_word_what_the_server_says_and_never_assume_php() {
    // A site's runtime comes from the server's `serving`, `php_effective` and `php_source`. The
    // old screens treated "no PHP version" as "static" and "a PHP version" as the site's runtime,
    // which is how a Go module came to be listed as PHP 8.5.
    assert!(APP.contains("function servingLabel(row)"), "one place words what a site serves");
    for case in ["\"proxy\"", "\"php\"", "\"static\"", "\"no_entry\""] {
        assert!(APP.contains(&format!("case {case}:")), "servingLabel handles {case}");
    }
    assert!(
        !APP.contains("row.php_effective ? \"PHP \" + row.php_effective : \"static\""),
        "a missing PHP version does not mean static files"
    );
    assert!(
        APP.contains("row.serving === \"php\" && installed.length"),
        "the PHP version picker is offered only to a site that serves PHP"
    );
    assert!(APP.contains("php_unused_pin"), "a pin that nothing uses is shown as unused, with a way to clear it");
    assert!(APP.contains("data-change=\"site-serving\""), "what a directory serves can be chosen from the site page");
}

#[test]
fn the_model_picker_uses_the_provider_selector_pattern() {
    // A datalist input looked like a select but never behaved like the provider
    // next to it: no options until focus, no shared dismiss/keyboard behaviour,
    // and a selection made before the first message was lost on the next paint.
    assert!(APP.contains("data-chat=\"model\" aria-label=\"Model\""));
    assert!(!APP.contains("id=\"chat-models\""), "the model picker is not a datalist any more");
    assert!(APP.contains("AI.pickProvider") && APP.contains("AI.pickModel"), "a model picked before the first message is remembered");
    assert!(APP.contains("args.provider = AI.pickProvider"), "the first message carries the picked provider");
    assert!(APP.contains("args.model = AI.pickModel"), "the first message carries the picked model");
}

#[test]
fn chapters_use_a_pin_icon_not_a_layers_icon() {
    assert!(HTML.contains("symbol id=\"i-pin\""), "there is a pin symbol");
    assert!(APP.contains("data-action=\"msg-pin\""), "pinning a reply is a message action");
    assert!(APP.contains("href=\"#i-pin\""), "the pin action uses the pin");
    assert!(
        !APP[APP.find("data-action=\"msg-pin\"").unwrap()..]
            .split("</button>")
            .next()
            .unwrap_or("")
            .contains("#i-layers"),
        "the chapter action no longer uses the layers icon"
    );
}

#[test]
fn normal_message_sending_does_not_toast() {
    // The conversation's own running state is the acknowledgement. The
    // capability's message is still available to a CLI caller, but the window
    // does not turn every submission into a toast.
    assert!(APP.contains("run(\"chat.send\", args, { quiet: true, rethrow: true })"), "a send is quiet; a failure reaches the outbox instead of a toast");
}

#[test]
fn tools_do_not_show_machine_implementation_metadata() {
    assert!(
        !APP.contains("package managers: "),
        "the Tools screen is about tools, not the operating system and package-manager list"
    );
}

#[test]
fn the_terminal_action_belongs_to_the_project_not_the_develop_page() {
    // On the project page, and as a surface on the conversation's project card.
    assert_eq!(
        APP.matches("data-action=\"open-terminal\"").count(),
        1,
        "the project page still opens its own terminal"
    );
    assert!(APP[APP.find("function paintWsCard()").unwrap()..].contains("data-action=\"work-terminal\""),
        "the conversation card opens a terminal surface");
    let start = APP.find("function renderProject()").expect("project page");
    let end = start + APP[start..].find("// -------------------------------------------------------------- capabilities").expect("next section");
    assert!(APP[start..end].contains("data-action=\"open-terminal\""), "the project page offers the terminal");
    let dev = APP.find("function renderDevelop()").expect("develop page");
    let dev_end = dev + APP[dev..].find("function procPanel()").expect("procPanel");
    assert!(!APP[dev..dev_end].contains("data-action=\"open-terminal\""), "the Stacks and tasks page no longer launches a terminal");
}

#[test]
fn one_class_carries_every_dropdown() {
    // A second, shorter select style is how the forms drifted away from the
    // composer's picker. Every select the app writes resolves to the one
    // control, so the test checks that the classes in the markup are covered by
    // that rule and that nothing re-declares a smaller one.
    let rule = CSS
        .find("One dropdown in the whole application")
        .map(|i| &CSS[i..])
        .expect("the one-dropdown rule");
    let rule = &rule[..rule.find("}").map(|i| i + 1).unwrap_or(rule.len())];
    for class in ["select-field", "select.input", "select.select", "select.composer-picker", "select.role-provider"] {
        assert!(rule.contains(class), "the one-dropdown rule covers {class}");
    }
    assert!(
        !CSS.contains(".select.compact { height: 24px"),
        "compact no longer makes a different height"
    );
    assert!(
        !APP.contains("class=\"input select-field\"") || rule.contains(".select-field"),
        "a select that also carries `.input` still resolves to the one control"
    );
}

#[test]
fn the_composer_has_one_primary_action() {
    // Stop while it works, the arrow as soon as there is something to send, and
    // stop again when the draft is cleared. One function decides; nothing else
    // hides the stop behind its back.
    let start = APP.find("function syncComposerAction()").expect("syncComposerAction");
    let end = start + APP[start..].find("\n  }").expect("end of function");
    let body = &APP[start..end];
    assert!(body.contains("if (stop) stop.hidden = !busy;"), "stop is the one action while busy");
    assert!(body.contains("send.hidden = busy || !hasText;"), "send never replaces stop while busy");
    let later = &APP[end..];
    assert!(
        !later.contains("stop.hidden ="),
        "no later code hides the stop after syncComposerAction decided"
    );
}

#[test]
fn the_working_indicator_actually_moves() {
    // The dots were given staggered delays but no animation, so the wait before
    // the first word looked like three static dots. A loading indicator that
    // does not move is not a loading indicator.
    assert!(APP.contains("class=\"tdot\""), "the wait is drawn as dots");
    assert!(APP.contains("typing-label"), "the wait says what it is");
    assert!(CSS.contains("animation: typing-breathe"), "the dots move");
    assert!(CSS.contains("@keyframes typing-breathe"), "the animation exists");
    assert!(CSS.contains("prefers-reduced-motion"), "the motion can be turned off");
}

#[test]
fn the_files_tab_belongs_to_the_project_that_is_open() {
    // The tree, the folder being walked and the open file are paths under one
    // project. A project switch used to leave the previous tree in state, so
    // another project's Files tab showed the files of the one before it until
    // a reload. The state is now keyed to the open project and replaced whole,
    // so a late answer for the old project cannot land in the new one.
    assert!(APP.contains("function filesForProject()"), "files state is scoped to a project");
    assert!(APP.contains("state.files.project !== path"), "a different project gets fresh state");
    assert!(APP.contains("state.files = null;"), "opening a project drops the old tree");
    assert!(APP.contains("var fs = filesForProject();"), "the Files tab reads the scoped state");
    // The tree and the document are two columns of one pane. The generic
    // `.panel + .panel` gap pushed the document down and broke the top edge.
    assert!(CSS.contains(".files-view .panel + .panel { margin-top: 0; }"), "both files panes start at the same line");
    // A long line wraps inside its own row, so the number stays beside it and
    // the code never hides behind a horizontal scrollbar.
    assert!(APP.contains("class=\"code-line\""), "each code line is its own row");
    assert!(
        CSS.contains(".code-text { min-width: 0; padding: 0 var(--s3); white-space: pre-wrap;"),
        "long lines wrap instead of scrolling sideways"
    );
}

#[test]
fn effort_belongs_to_the_model_picker() {
    // Effort sits under the model list of the selected provider, not beside
    // the model as a second control. It is offered only where the model has
    // a reasoning dial, and it opens in place with a way back.
    assert!(APP.contains("function modelEffortRow"), "the effort row is built with the picker");
    assert!(APP.contains("data-action=\"composer-effort\""), "the row opens the effort page");
    assert!(APP.contains("function effortPickerHtml"), "the effort page is in the same picker");
    assert!(APP.contains("data-action=\"composer-effort-back\""), "there is a way back to the models");
    assert!(APP.contains("Higher effort means more thorough responses"), "the page explains the trade-off");
    assert!(APP.contains("effort-recommended"), "Medium is marked as the recommendation");
    assert!(!APP.contains("composer-choice composer-effort"), "effort is not a second chip in the bar");
    assert!(APP.contains("if (!effortSupported(rail, model)) return \"\";"), "an inert control is not offered");
    assert!(APP.contains("(p.target ? \"\" : modelEffortRow(rail))"), "provider and role pickers do not carry conversation effort");
}

#[test]
fn polling_does_not_rebuild_an_open_menu() {
    // The rail is painted by every timer. Replacing it while a menu is open
    // would close the menu, replay its animation and move the pointer's target.
    // The rail has no menus left to close, and it is patched, not replaced: focus and a
    // pressed state survive a repaint.
    assert!(APP.contains("if (force || html !== RAIL_CACHE.html) {"), "an unchanged rail is not repainted");
    assert!(APP.contains("morph(el(\"rail\"), html);"), "the rail is patched in place");
    assert!(APP.contains("RAIL_CACHE"), "the rail keeps its last markup");
    // A live refresh must not pull a page from under an open dialog either.
    assert!(APP.contains("[role=\\\"dialog\\\"]"), "an open dialog pauses live repainting");
    assert!(APP.contains("p.anchorAction"), "an open popover finds its trigger after a repaint");
}

#[test]
fn opening_a_project_from_home_opens_that_project() {
    // The card's Open button and its row are the same act: open this project.
    // It used to go to the generic Stacks and tasks screen instead.
    assert!(APP.contains("\"home-open\": function (n) { openProject(n.dataset.path); }"), "home opens the project");
    assert!(!APP.contains("\"home-open\": function (n) { DEV.path"), "home does not fall back to Develop");
}

#[test]
fn the_first_section_under_site_tabs_has_no_dead_band() {
    // A section head carries its own top margin for separating chapters. The
    // first one under a tab strip is not a new chapter, so it must not leave a
    // gap between the tabs and the content.
    assert!(CSS.contains(".segmented.site-tabs + .ui-head { margin-top: 0; }"));
    assert!(APP.contains("class=\"site-page\""), "the tab strip and its sections are one block");
    assert!(CSS.contains(".site-page {"), "the block sets its own spacing");
}

#[test]
fn file_folders_look_like_doors() {
    assert!(APP.contains("class=\"file-go\""), "a folder row has a way-in mark");
    assert!(CSS.contains(".file-go"), "the mark has its own alignment and hover state");
}

#[test]
fn agent_actions_are_grouped_into_one_collapsed_line() {
    assert!(APP.contains("class=\"msg-tools"), "steps are wrapped in one group");
    assert!(APP.contains("closeGroup()"), "the group is closed by whatever is not a step");
    // The run reads as one line of prose, collapsed by default; the calls
    // themselves are one click away rather than crowding the answer.
    assert!(APP.contains("data-action=\"tools-toggle\""), "the run opens and closes");
    assert!(APP.contains("function toolSummary"), "the heading says what happened in words");
    assert!(APP.contains("class=\"tools-list\""), "the calls sit in one list");
    assert!(APP.contains("(open ? \"\" : \" hidden\")"), "the list starts collapsed");
    assert!(CSS.contains(".tools-list[hidden]"), "a collapsed list takes no space");
    assert!(!CSS.contains(".msg-tools > .msg-used"), "no count line above the rows");
}

#[test]
fn one_question_gets_one_assistant_turn() {
    // A turn is the span from a person's message to the next one. Every model
    // call in it - the intent, the tool calls, the final answer - is part of
    // that turn, with one avatar and one attribution. A person's message is
    // its own row, which is what separates one turn from the next.
    assert!(
        APP.contains("blocks.push({ type: \"turn\", turn: current });"),
        "assistant calls are grouped into one turn"
    );
    assert!(
        APP.contains("blocks.push({ type: \"user\", at: e.created_at"),
        "a person's message is its own row"
    );
    assert!(CSS.contains(".turn {"), "the turn has its own layout");
    assert!(CSS.contains(".msg-user {"), "the person's row stays separate");
}

#[test]
fn a_streamed_reply_is_replaced_by_the_log_not_duplicated() {
    let start = APP.find("function onLive(event)").expect("the live handler");
    let end = start + APP[start..].find("\n  }").expect("the end of the handler");
    let body = &APP[start..end];
    assert!(body.contains("AI.streamTail += piece"), "tokens are added to the turn being written");
    assert!(body.contains("clearStreamTail(false)"), "the live copy is dropped when the reply is logged");
    assert!(body.contains("pollThread(false)"), "the record is re-read after every event");
    assert!(
        APP.contains("if (e.id <= AI.lastId) return;"),
        "overlapping polls cannot append the same event twice"
    );
}

#[test]
fn text_areas_for_line_lists_have_room() {
    for (id, rows) in [("mcp-args", 5), ("mcp-env", 4), ("mcp-edit-args", 5), ("team-done", 4), ("sched-done", 4)] {
        let needle = format!("id=\"{id}\" rows=\"{rows}\"");
        assert!(APP.contains(&needle), "{id} has at least {rows} rows");
    }
}

#[test]
fn alert_copy_flows_as_one_paragraph() {
    // The base notice is a paragraph. Under `display: flex`, every text node and
    // <b> becomes its own column, which is what broke the restart banner and the
    // notices with an emphasised lead.
    let rule = &CSS[CSS.find(".notice {").unwrap()..];
    let rule = &rule[..rule.find('}').unwrap()];
    assert!(rule.contains("display: block"), "{rule}");
    // A notice that carries its own action keeps the row: text grows, action stays.
    assert!(CSS.contains(".notice:has(> .grow, > .btn, > button)"));
    // The top banner has a title and a body, never a bare <b> beside the text.
    assert!(APP.contains("offline-title") && APP.contains("offline-text"));
}

#[test]
fn the_composer_has_two_triggers_and_each_offers_the_right_thing() {
    // The placeholder has said "Type / or @" since the composer was written, and
    // neither worked: "/" did nothing at all, and "@" offered capabilities,
    // skills, projects and roles but never a file - the one thing "@" means.
    assert!(APP.contains("function triggerAt(text, caret)"), "one pipeline detects both triggers");
    assert!(APP.contains("function triggerItems(kind, query)"), "one pipeline, sources by kind");
    // "/" is commands: skills and the tools an agent can run.
    assert!(APP.contains("if (kind === \"/\")"), "a slash source exists");
    assert!(APP.contains("group: \"Skills\""), "skills are offered by name");
    assert!(APP.contains("group: \"Tools\""), "tools are offered under the slash too");
    // "@" is references, and a file is one of them.
    assert!(APP.contains("function fileMentions(q)"), "files are searchable");
    assert!(APP.contains("group: \"Files\""), "a file is a reference kind");
    assert!(APP.contains("read(\"fs.tree\""), "files come from the project's tree");
    // A path query is a path: it may contain the separators a path has.
    assert!(!APP.contains("if (/[\\s]/.test(query)) return null;\n    return { start: at, query: query };"),
        "the old mention trigger rejected the characters a path is made of");
    // Both composers open the list, and the one typed into is the one edited.
    assert!(APP.contains("var ht = triggerAt(t.value, t.selectionStart);"), "the home box opens the list");
    assert!(APP.contains("composerInput(prefix)"), "the pick goes into the box that was typed in");
    // The placeholder now says what each trigger does.
    assert!(APP.contains("/ for a skill or tool, @ for a file"), "the promise matches the behaviour");
}

#[test]
fn a_destructive_button_says_so_with_a_class_not_with_its_name() {
    // Buttons used to be coloured red by `[data-action*="remove"]`: colour chosen
    // by spelling. A button called `remove-from-view` would have turned red for
    // removing nothing, and one called `purge` would have stayed neutral.
    assert!(!CSS.contains("data-action*="), "no button is styled by the name of its action");

    // Every destructive action now carries the class that means destructive.
    for action in [
        "chat-remove-file", "mcp-remove", "remove-site", "remove-service", "remove-runtime",
        "ops-cron-remove", "skill-remove", "tool-remove", "prov-remove",
    ] {
        let needle = format!("data-action=\"{action}\"");
        let at = APP.find(&needle).unwrap_or_else(|| panic!("{action} is gone"));
        // The class is in the same tag, either before or after the action.
        let tag_start = APP[..at].rfind("<button").expect("a button tag");
        let tag_end = APP[at..].find('>').map(|i| at + i).expect("a closing bracket");
        let tag = &APP[tag_start..tag_end];
        assert!(
            tag.contains("btn-danger"),
            "{action} removes something, so it must say btn-danger, not rely on its name: {tag}"
        );
    }
}

#[test]
fn the_inbox_cannot_hang_waiting_for_one_failed_read() {
    // `renderInbox` draws nothing until `AI.requests` is set, and it is set only
    // when every read in `loadInbox` succeeds. Two of the six had `.catch`
    // handlers and four did not, so one failed request rejected the batch and the
    // screen sat on "Loading" for ever with no error and nothing to retry.
    let start = APP.find("function loadInbox()").expect("loadInbox is gone");
    let body = &APP[start..start + 1400];
    assert!(body.contains("var soft = function"), "each read is guarded to its own empty value");
    let batch = &body[body.find("return Promise.all").expect("the batch")..];
    let batch = &batch[..batch.find(".then(function (r)").expect("the then")];
    assert!(
        !batch.contains("read(\"") || batch.matches("soft(read(").count() == batch.matches("read(\"").count(),
        "an unguarded read in the batch can hang the screen: {batch}"
    );
}

#[test]
fn every_run_row_has_the_same_delete_control_as_a_conversation() {
    // The Assistant list offers delete on every conversation. The Agents list
    // offered it only on runs that were already over, so the same gesture existed
    // in one place and not the other. The capability still refuses a run that is
    // working, with a sentence saying to cancel it first.
    let start = APP.find("function paintTeamList()").expect("paintTeamList is gone");
    let body = &APP[start..start + 4000];
    assert!(
        body.contains("data-action=\"team-delete\""),
        "a run row carries a delete control"
    );
    assert!(
        !body.contains("(over ? '<button"),
        "the control is not conditional on the run being over: {body}"
    );

    let assistant = APP.find("function threadRowHtml(t, child)").expect("threadRowHtml is gone");
    let assistant_body = &APP[assistant..assistant + 2400];
    assert!(
        assistant_body.contains("data-action=\"chat-delete\""),
        "and a conversation still does, the way it always has"
    );
}

#[test]
fn the_provider_card_has_one_way_to_use_a_model_id_and_not_two() {
    // A permanent text field on every provider card, beside a picker that already
    // offers "use the id you typed", was two ways to do one thing.
    assert!(
        !APP.contains("Add a model id this provider does not"),
        "the inline add-a-model field is gone"
    );
    assert!(
        !APP.contains("data-action=\"prov-model-add\""),
        "and nothing still calls it"
    );
    assert!(
        APP.contains("composer-custom-model"),
        "the picker still offers an id the provider does not list"
    );
    // Test, Fetch models, Verify model, Edit and Remove belong to the provider's
    // header. As a second grid row they fell under the facts.
    assert!(APP.contains("class=\"provider-head\""), "the provider actions live in the header");
    assert!(CSS.contains(".provider-head { display: flex;"), "the header puts the name and actions on one line");
}

#[test]
fn a_project_opens_on_the_first_tab_the_strip_shows() {
    // `files` is first in TAB_LABELS. Opening a repository on `changes` put the
    // person one tab along from where the strip implies the screen starts.
    let labels = APP.find("var TAB_LABELS = {").expect("TAB_LABELS is gone");
    let labels = &APP[labels..labels + 300];
    assert!(
        labels.find("files:").unwrap() < labels.find("changes:").unwrap(),
        "Files is the first tab"
    );
    let open = APP.find("function openProject(path, tab)").expect("openProject is gone");
    let open = &APP[open..open + 600];
    assert!(
        open.contains("state.tab = tab || \"files\""),
        "opening a project starts on Files unless a link names its tab: {open}"
    );
    assert!(
        APP.contains("loadTab(state.tab)"),
        "a named tab loads before it is drawn"
    );
}

#[test]
fn the_inbox_needs_card_does_not_read_a_field_the_server_never_sends() {
    // `needs` has no `primary` key: the server computes it with a Rust method and
    // never serialises it. `needsCard` read `n.primary.id` for every action, so
    // the first paused run threw "Cannot read properties of undefined" *inside*
    // the render - the markup was never written and the Inbox sat on "Loading".
    assert!(
        !APP.contains("n.primary.id"),
        "the needs card must not read a primary field the server does not send"
    );
    let start = APP.find("function needsCard(n)").expect("needsCard is gone");
    let body = &APP[start..start + 1200];
    assert!(
        body.contains("!a.dangerous"),
        "the primary is the first action that is not dangerous, as the server decides it: {body}"
    );
}

#[test]
fn typing_in_the_assistant_composer_does_not_rebuild_the_screen() {
    // The input handler is an if/else-if chain. The chat-input branch never closed,
    // so every keystroke also ran the filter branch: it set `state.filters[undefined]`
    // and scheduled `render()` 120ms later, which rebuilt `main` - and the textarea
    // with it, twice a second. Home's composer was unaffected because its branch
    // ends the chain.
    let start = APP.find("else if (t.id === \"chat-input\")").expect("the chat composer handler is gone");
    let body = &APP[start..start + 900];
    // The branch must be closed before the filter branch starts.
    let filter_at = body.find("dataset.filter").expect("the filter branch is gone");
    let close_at = body.find("\n    }").expect("the chat branch never closes");
    assert!(
        close_at < filter_at,
        "the composer branch must end before the filter branch, or typing re-renders the screen"
    );
    assert!(
        body.contains("else if (t.dataset && t.dataset.filter)"),
        "the filter branch must require a filter box: {body}"
    );
}

#[test]
fn a_pid_file_carries_a_start_token_because_a_pid_alone_is_not_an_identity() {
    // The operating system reuses process ids, so a pid file left by a dead devd
    // read as owned by whatever held that number later - which is how a stale
    // agent-host.pid blocked recovery while naming an unrelated process.
    let src = include_str!("../../devd-core/src/process.rs");
    assert!(
        src.contains("pub fn holder_line()"),
        "there is one way to write a holder line, and it includes the start token"
    );
    assert!(
        src.contains("(Some(wrote), Some(is)) => (wrote == is).then_some(pid)"),
        "a holder is live only when the start token still matches"
    );
}

#[test]
fn the_badge_and_the_inbox_count_the_same_rows() {
    // The rail counted every open row with needs_reply; the Inbox hid some by
    // subject prefix; Home applied a third rule. The number beside "Inbox" is
    // now derived from the rows the Inbox actually renders.
    let start = APP.find("function inboxQuestionRows(list)").expect("the shared question filter is gone");
    let body = &APP[start..start + 500];
    assert!(body.contains("m.needs_reply && !aboutShownRun(m)"), "one definition of a visible question: {body}");
    let load = APP.find("AI.counts.questions = (AI.needs || []).length + inboxQuestionRows(AI.inbox).length")
        .expect("the inbox no longer derives the badge from its own rows");
    assert!(load > start);
    assert!(!APP.contains("AI.counts.questions = (r[1] || []).filter"), "the health read is not a second count");
}

#[test]
fn deleting_a_run_takes_its_notifications_with_it() {
    // `delete_run` removed the run, its tasks and its events, but not the inbox
    // rows that were about it. Those orphans then showed up as fresh Decisions.
    let src = include_str!("../../devd-core/src/store_team.rs");
    let start = src.find("pub fn delete_run(&self, id: &str)").expect("delete_run is gone");
    let body = &src[start..start + 900];
    assert!(body.contains("forget_run_inbox"), "the run's messages and reports are deleted with it: {body}");
    let store = include_str!("../../devd-core/src/store_ai.rs");
    assert!(store.contains("pub fn forget_run_inbox"), "there is one place that owns that deletion");
    assert!(store.contains("DELETE FROM team_reports WHERE run_id"), "an undelivered report cannot outlive its run");
}

#[test]
fn a_run_that_is_over_does_not_keep_asking_for_a_person() {
    // A cancelled run still offered Retry and Accept for a task that would never
    // run again. The question comes from one function, and it answers none.
    let src = include_str!("../../devd-agent/src/needs.rs");
    let start = src.find("pub fn of(run: &TeamRun").expect("needs::of is gone");
    let body = &src[start..start + 500];
    assert!(body.contains("\"done\" | \"failed\" | \"cancelled\""), "a finished run needs nothing: {body}");
    assert!(body.contains("return None"), "and the answer is None, not a card");
}

#[test]
fn both_composers_have_a_mention_menu_and_a_stable_query() {
    // Home's "/" and "@" opened a pipeline with nowhere to paint: only the chat
    // composer rendered `#chat-mentions`. And `openMentions` never stored the
    // query, so the "has it changed?" test was always true and the list reset.
    assert!(APP.contains("id=\"home-mentions\""), "Home has its own menu");
    assert!(APP.contains("id=\"chat-mentions\""), "and the Assistant still has its own");
    let start = APP.find("function openMentions(start, query, kind)").expect("openMentions is gone");
    let body = &APP[start..start + 400];
    assert!(body.contains("MENTIONS.query = String(query || \"\")"), "the query is remembered: {body}");
    assert!(APP.contains("function mentionBox()"), "one lookup serves both composers");
    // "/" then "@" at the same position is a different list. The change test
    // compared the start and the query only, so typing one after the other kept
    // the first list on screen - skills while typing "@".
    assert!(
        APP.contains("MENTIONS.kind !== t.value.charAt(ht.start)")
            && APP.contains("MENTIONS.kind !== t.value.charAt(trig.start)"),
        "the trigger kind is part of deciding whether the list changed"
    );
    assert!(APP.contains("if (MENTIONS.kind !== nextKind) MENTIONS.items = []"), "a new kind does not inherit the old items");
}

#[test]
fn refreshing_a_run_keeps_the_readers_place() {
    // The run detail was rebuilt every 1.2 s, and replacing its markup reset the
    // scroll container to the top - the reported "sent back up while scrolling".
    // The run is patched in place (see `morph`), so nothing the reader is looking at is
    // rebuilt; only a reader already at the end is followed.
    let start = APP.find("function paintTeam()").expect("paintTeam is gone");
    let body = &APP[start..start + 3000];
    assert!(body.contains("else morph(box, html);"), "the same run is patched, not replaced: {body}");
    assert!(body.contains("following(scroll)"), "only a reader at the end is followed");
    assert!(body.contains("html !== TEAM.lastHtml"), "an unchanged run is not repainted at all");
}

#[test]
fn the_run_form_is_not_redrawn_by_the_timer_that_refreshes_the_run_list() {
    // `paintTeam` runs every 1.2 s. Rebuilding the new-run form each time closed an
    // open template dropdown before a choice could be made.
    let start = APP.find("function paintTeam()").expect("paintTeam is gone");
    let body = &APP[start..start + 2400];
    assert!(
        body.contains("TEAM.lastHtml !== \"<form>\""),
        "the form is drawn once and only again when something that it shows changes"
    );
    assert!(
        APP.matches("TEAM.lastHtml = \"\";").count() >= 4,
        "opening the page, choosing a template, a project and New redraw the form on purpose"
    );
}

#[test]
fn each_composer_keeps_its_own_files() {
    // One list for the whole app put a file attached on Home into the Assistant.
    assert!(!APP.contains("ATTACHMENTS"), "attachments are kept per composer (ATT.home, ATT.chat)");
    assert!(APP.contains("var ATT = { home: [], chat: [], chatFor: \"\" }"));
    assert!(APP.contains("takeAttachments(text, \"home\")") && APP.contains("takeAttachments(text, \"chat\")"));
}

#[test]
fn a_row_of_buttons_has_a_gap_wherever_it_is_drawn() {
    // `.ui-actions` was only styled inside a sheet, so on a page its buttons touched.
    let rule = CSS.find("\n.ui-actions {").expect(".ui-actions needs a rule that applies on every screen");
    let body = &CSS[rule..rule + 140];
    assert!(body.contains("display: flex") && body.contains("gap:"), "the row spaces its buttons itself: {body}");
}

#[test]
fn every_table_shares_one_head_and_one_action_column() {
    // The action head used to be `display: flex` (from the generic `.cell-actions`),
    // which left its line short and its label floating away from its column.
    assert!(
        CSS.contains(":is(.table, .mu, .drift-table) :is(th, td).cell-actions {\n  display: table-cell;"),
        "an action head or cell is a real table cell"
    );
    // One definition of the head, not one per screen.
    assert_eq!(
        CSS.matches("text-transform: uppercase;\n  vertical-align: middle;").count(),
        1,
        "the table head is defined once"
    );
    // Numbers line up by their digits.
    assert!(CSS.contains(".cell-num"));
}

#[test]
fn a_table_head_sits_centred_between_its_lines() {
    // The head had no space above it and ten pixels below, so beside the rule of a
    // panel heading the words hung from the top. Equal space above and below.
    let at = CSS.find(".table th, .mu th, .drift-table th {").expect("the shared head rule");
    let body = &CSS[at..at + 120];
    assert!(body.contains("padding: 10px var(--s4) 10px 0;"), "the head has the same space above and below: {body}");
}

#[test]
fn an_open_section_stays_as_the_person_left_it_when_the_page_is_redrawn() {
    // A timer redraws a page, and a section the person had opened used to snap shut.
    assert!(APP.contains("var FOLDS = {};") && APP.contains("function restoreFolds()"));
    assert!(APP.contains("restoreFolds(); }).observe(mainNode"), "restored after every redraw of the page");
    assert!(APP.contains("FOLDS[foldKey(details)] = !details.open;"), "only a click on the heading is a decision");
}

#[test]
fn a_timer_never_redraws_a_page_under_a_field_that_holds_unsaved_input() {
    let start = APP.find("function liveIsEditing()").expect("liveIsEditing is gone");
    let body = &APP[start..start + 900];
    assert!(body.contains("liveIsDirty()"), "an unsaved field holds the page still");
    // The composer is kept in the page's state, so it must not stop Home from refreshing.
    assert!(APP.contains("f.closest(\".composer\")"), "the composer is not a reason to skip a refresh");
    assert!(APP.contains("load().then(function () { if (state.view === view) repaintKeepingFocus(); })"));
}

#[test]
fn the_previews_helper_is_only_believed_when_it_comes_from_the_preview_frame() {
    assert!(APP.contains("event.source !== f.contentWindow"), "a message from anywhere else is ignored");
    assert!(APP.contains("(The element and error details above came from the page itself."), "page text is handed on as data");
    // The panel is filled in place: replacing it with the page would reload the frame.
    assert!(APP.contains("'<div id=\"pv-slot\"></div>'"));
}

#[test]
fn uploads_come_back_by_name_through_the_token_and_expire() {
    assert!(APP.contains("/api/upload/\" + encodeURIComponent(id)"));
    assert!(APP.contains("Authorization: \"Bearer \" + TOKEN"));
    assert!(APP.contains("msg-file-open"), "a conversation shows the files a message carried");
}

#[test]
fn a_teams_report_in_a_conversation_is_not_drawn_as_the_person() {
    // The team hands its report to the conversation that asked as a message; it is not the
    // person speaking, so it must not be labelled "You".
    assert!(APP.contains("from: e.data.from || \"\""), "the event says who it is from");
    assert!(APP.contains("if (block.type === \"user\" && block.from === \"devd-team\") {"), "a message from the team has its own shape");
    // Every message carries a sender, the person's included; a bare truthiness test drew the
    // person's own words as a note from the team.
    assert!(!APP.contains("if (block.type === \"user\" && block.from) {"));
    assert!(CSS.contains(".team-note {"));
}

#[test]
fn components_use_the_semantic_colour_names() {
    // Colour is named by role (--color-primary, --color-base-200, --color-success...).
    // The palette variables remain as the source of the values, but a component rule
    // must not reach past the semantic names: that is what keeps a screen and a new
    // component on the same palette.
    let marker = "/* ---- semantic colour names";
    let cut = CSS.find(marker).expect("the semantic colour block is defined");
    let components = &CSS[..cut];
    for token in [
        "var(--accent)",
        "var(--bg)",
        "var(--bg-surface)",
        "var(--bg-raised)",
        "var(--fg)",
        "var(--ok)",
        "var(--warn)",
        "var(--danger)",
        "var(--info)",
    ] {
        assert!(!components.contains(token), "a component rule still uses {token}; use its --color-* name");
    }
    for token in ["--color-primary:", "--color-base-100:", "--color-success:", "--color-error:"] {
        assert!(CSS.contains(token), "the semantic palette defines {token}");
    }
}

#[test]
fn an_agent_chart_is_declared_and_drawn_by_the_shared_renderer() {
    // An agent may emit a ```chart block, but only data: labels, finite values
    // and a tone. The renderer chooses the markup and the classes.
    assert!(APP.contains("if (lang === \"chart\")"), "the fenced chart block is recognised");
    assert!(APP.contains("function chartHtml(spec)"), "charts have one renderer");
    assert!(APP.contains("Math.round(Number(b.value) / max * 1000) / 10"), "bar width is computed from the value");
    assert!(APP.contains("VIZ_NAMES.indexOf(c) >= 0"), "a bar colour is one of the named palette colours, never free text");
    assert!(!APP.contains("spec.html") && !APP.contains("spec.innerHTML"), "chart data never carries markup");
    assert!(CSS.contains(".viz-bar-row") && CSS.contains(".viz-bar-fill"));
    let manifest = include_str!("../../../docs/design-manifest.json");
    assert!(manifest.contains("\"chart\""), "the agent contract declares the chart block");
}

#[test]
fn every_select_is_wired_to_something() {
    // A select that has no id, no data-change and no data-action is a dead
    // control: the menu opens and choosing an option changes nothing. Every
    // select must be identifiable to its handler, or be a form field the
    // capability form reads by `data-arg` / `data-create`.
    let bytes = APP.as_bytes();
    let mut at = 0usize;
    let mut count = 0usize;
    while let Some(found) = APP[at..].find("<select") {
        let start = at + found;
        at = start + 7;
        let end = APP[start..].find('>').map(|n| start + n).unwrap_or(APP.len());
        let tag = &APP[start..end];
        // `<select>` in a comment is prose, not a control.
        if tag == "<select" {
            continue;
        }
        count += 1;
        assert!(
            tag.contains("id=\"") || tag.contains("data-change=") || tag.contains("data-action=")
                || tag.contains("data-arg=") || tag.contains("data-create=") || tag.contains("data-pf="),
            "a select is not wired to a handler: {tag}"
        );
    }
    assert!(count > 20, "the scan found the application's selects (found {count})");
    let _ = bytes;
}

#[test]
fn every_data_change_value_has_a_handler() {
    // The delegated change listener looks the value up in the action table. A
    // value with no entry is silently ignored, which is how a dropdown "does
    // nothing".
    let mut seen = std::collections::BTreeSet::new();
    let mut rest = APP;
    while let Some(i) = rest.find("data-change=\"") {
        let after = &rest[i + 13..];
        let value = after.split('"').next().unwrap_or("");
        if !value.is_empty() && !value.contains('\'') {
            seen.insert(value.to_string());
        }
        rest = after;
    }
    assert!(!seen.is_empty());
    for value in seen {
        let handler = format!("\"{value}\": function");
        assert!(APP.contains(&handler), "no handler for data-change `{value}`");
    }
}

#[test]
fn opening_a_specialist_never_detaches_it_from_its_request_in_the_list() {
    // Clicking through conversations repainted the list under the patcher. Two things
    // broke the margins: the open conversation's copy replaced the list entry and lost its
    // team link, and a request and its specialists were siblings the patcher could pair up
    // wrongly. A request and its list are one keyed item, and the entry is merged.
    assert!(
        APP.contains("AI.threads[idx] = Object.assign({}, AI.threads[idx], d.thread)"),
        "the open thread is merged into the list entry, not swapped for it"
    );
    assert!(
        APP.contains("<div class=\"thread-item\" data-key=\"i") && APP.contains("<div class=\"thread-kids-list\" data-key=\"k"),
        "a request and its specialists are one keyed item"
    );
    assert!(
        !CSS.contains(".thread-child { margin-left: 14px"),
        "only the specialists' list indents them, never the row on its own"
    );
}

#[test]
fn reply_visuals_read_their_payload_from_the_fence_line() {
    // Models write the JSON on the opening line: ```steps {"items": [...]}.
    // The parser used to keep only the language, so the card rendered empty.
    assert!(
        APP.contains("var fence = m[1], lang = m[2], rest = (m[3] || \"\").trim(), body = [], closed = false;"),
        "the info line's payload is captured"
    );
    assert!(APP.contains("try { var atInfo = JSON.parse(rest); onInfo = true;"), "the info line is tried first");
    assert!(APP.contains("var tries = [inner, rest ? rest + (inner"), "the body and both shapes are tried");
    assert!(APP.contains("if (rest) body.unshift(rest);"), "an unparseable block keeps its source");
}

#[test]
fn a_failed_test_is_red_in_the_collapsed_run_and_its_dots() {
    assert!(APP.contains("function resultFailed(r)"), "a failed result is one rule");
    assert!(APP.contains("resultFailed(results[c.id])"), "a tool group sees a failed command");
    assert!(CSS.contains(".msg-tools.is-bad .tools-dot"), "the group dot turns red");
    assert!(APP.contains("(resultFailed(r) ? \"bad\" : \"ok\")"), "the step dot turns red");
}

#[test]
fn the_composer_keeps_stop_in_the_primary_slot_while_it_works() {
    assert!(APP.contains("send.hidden = busy || !hasText;"), "send never replaces stop while busy");
    assert!(APP.contains("if (stop) stop.hidden = !busy;"), "stop is the one action while busy");
    assert!(CSS.contains("background: var(--color-error);"), "stop is the red round control");
}

#[test]
fn the_side_card_changes_link_opens_the_git_changes_tab() {
    assert!(APP.contains("data-tab=\"changes\""), "the link names the destination tab");
    assert!(APP.contains("openProject(node.dataset.path, node.dataset.tab)"), "the action forwards it");
    assert!(APP.contains("return state.tab !== \"files\" ? loadTab(state.tab) : null;"), "the tab loads before it is drawn");
}

#[test]
fn secondary_list_controls_wait_for_hover_or_focus() {
    assert!(CSS.contains(".thread-kids {"), "the disclosure has its own rule");
    assert!(CSS.contains("opacity: 0; pointer-events: none;"), "it is hidden and inert until needed");
    assert!(CSS.contains(".thread-row:hover .thread-kids"), "hover reveals it");
}

#[test]
fn a_blocked_run_is_a_contained_card() {
    assert!(CSS.contains(".decision.decision-needs {"), "the needs decision is a card, not a label column");
    // It belongs to the notice/banner family: the same hairline, tint and radius,
    // and no bar down the side.
    assert!(
        CSS.contains("border: 1px solid color-mix(in srgb, var(--color-warning) 22%, var(--line));"),
        "a quiet tone border, like a banner"
    );
    assert!(!CSS.contains("border-left: 3px solid var(--color-warning)"), "no warning stripe");
    assert!(CSS.contains("text-transform: none"), "the label is not shouted in capitals");
}

#[test]
fn a_need_carries_its_question_and_detail_to_the_window() {
    // `question` and `detail` are Rust methods; without writing them into the
    // JSON the card rendered blank.
    let needs = include_str!("../../devd-agent/src/needs.rs");
    assert!(needs.contains("serde::ser::SerializeStruct"), "Needs serialises itself");
    assert!(needs.contains("st.serialize_field(\"question\""), "the question is in the JSON");
    assert!(needs.contains("st.serialize_field(\"detail\""), "the detail is in the JSON");
    assert!(APP.contains("md(n.detail)"), "the report renders as markdown");
}

#[test]
fn run_progress_uses_tasks_as_its_one_denominator() {
    // The strip used to mix changes, checks and stage numbers. Every stage now
    // reports task counts and fills its rail by the same fraction.
    assert!(APP.contains("var total = work.length;"), "tasks are the denominator");
    assert!(APP.contains("with evidence"), "evidence counts tasks, not raw changes");
    assert!(APP.contains("verified", ), "verification counts verified tasks");
    assert!(APP.contains("style=\"--p:"), "each stage fills by its own fraction");
    assert!(CSS.contains("scaleX(var(--p, 0))"), "the rail honours that fraction");
}

#[test]
fn the_side_panel_mirrors_the_conversation_list() {
    // The right column is a rail like the left one: the same head height and a
    // hairline on the side that faces the conversation, not a floating card.
    assert!(CSS.contains(".ws-head {"), "the panel has a head");
    assert!(CSS.contains("height: 52px; min-height: 52px;"), "the head matches the list's");
    assert!(CSS.contains("border-left: 1px solid var(--line); border-radius: 0;"), "the divider mirrors the left rail");
    assert!(!CSS.contains("margin: 64px 16px 0 0"), "the floating card is gone");
    // Outputs and the workspace surfaces are wired to real data.
    assert!(APP.contains("function outputsSection"), "outputs are a section");
    assert!(APP.contains("function paintWork"), "the workspace surfaces are painted from state");
    assert!(APP.contains("function noteOutput(event)"), "outputs come from the conversation's own tool calls");
    assert!(APP.contains("\"output-open\""), "an output opens its file");
    assert!(APP.contains("data-action=\"work-browser\""), "the browser is opened in place");
    assert!(APP.contains("data-action=\"work-terminal\""), "the terminal is opened in place");
    assert!(!APP.contains("No terminal is running in this project"), "the empty Terminals section is gone");
}

#[test]
fn heavy_renderers_are_loaded_when_a_screen_needs_them() {
    // Mermaid, KaTeX and xterm are three quarters of a megabyte between them and
    // most visits use none of them.
    assert!(!HTML.contains("/vendor/mermaid.min.js\""), "the diagram runtime is not in the shell");
    assert!(!HTML.contains("/vendor/katex.min.js\""), "the maths runtime is not in the shell");
    assert!(!HTML.contains("/vendor/xterm.js\""), "the terminal runtime is not in the shell");
    assert!(APP.contains("function loadVendor"), "there is one loader for all three");
    assert!(APP.contains("loadVendor(\"mermaid\")"), "a diagram fetches the diagram runtime");
    assert!(APP.contains("loadVendor(\"katex\")"), "maths fetches the maths runtime");
    assert!(APP.contains("loadVendor(\"xterm\")"), "a shell fetches the terminal runtime");
}

#[test]
fn dictation_offers_to_install_itself() {
    // The microphone tells the truth about what it needs instead of failing
    // after the person has spoken.
    assert!(APP.contains("read(\"dictation.status\""), "the microphone checks the tools");
    assert!(APP.contains("run(\"dictation.install\""), "it offers the one download");
    assert!(APP.contains("function encodeWav"), "the recording is encoded for Whisper");
    assert!(APP.contains("chunks.forEach(function (c) { all.set(c, at); at += c.length; });"), "every recorded sample is transcribed");
    assert!(APP.contains("sink.gain.value = 0"), "the microphone is not played back");
}

#[test]
fn a_rounded_hover_uses_the_one_radius_scale() {
    // Interactive surfaces share the radius tokens; the conversation list keeps
    // its hover fill rounded wherever it is drawn.
    assert!(CSS.contains(".thread { position: relative; border: 0; border-radius: var(--radius);"), "the run list row is rounded");
    assert!(CSS.contains("border: 0; border-radius: var(--radius); background: none; box-shadow: none;"), "the conversation row is rounded");
    assert!(CSS.contains("#panel .panel-item {") && CSS.contains("height: 32px; margin: 1px 0; padding: 0 10px;"), "panel rows are rounded");
    assert!(CSS.contains(".assist-start {") && CSS.contains("border-radius: var(--radius); color: var(--color-base-content);"), "the start rows share the shape");
    assert!(!CSS.contains("border-radius: 7px"), "the ad-hoc radius is gone");
}

#[test]
fn assets_are_compressed_and_revalidated() {
    let ui = include_str!("../src/ui.rs");
    assert!(ui.contains("flate2::write::GzEncoder"), "the assets are gzipped once");
    assert!(ui.contains("ETAG"), "a reload costs a 304, not the whole file");
    assert!(ui.contains("no-cache"), "the browser revalidates instead of re-downloading");
    assert!(ui.contains("CONTENT_ENCODING"), "the compressed body is labelled");
}

#[test]
fn an_unclosed_fence_never_swallows_the_rest_of_the_message() {
    // A model that forgets the closing fence must not make the rest of its
    // reply disappear into the last block.
    assert!(APP.contains("body = [], closed = false;"), "the fence loop records whether it closed");
    assert!(APP.contains("if (onInfo && body.length) out.push(mdBlocks(body));"), "the rest of an unclosed block is still rendered");
    assert!(APP.contains("onInfo = true"), "the parser knows whether the payload was on the info line");
}

#[test]
fn the_tray_is_a_background_app_with_useful_actions() {
    let desktop = include_str!("../../devd-desktop/src/lib.rs");
    assert!(desktop.contains("TrayIconBuilder"), "there is a hidden-icons tray");
    assert!(desktop.contains("show_menu_on_left_click(false)"), "right click is the menu");
    assert!(desktop.contains("\"Open Assistant\""), "the tray opens the assistant");
    assert!(desktop.contains("\"Open projects folder\""), "the tray opens the projects folder");
    assert!(desktop.contains("\"Quit devd\""), "quitting is explicit");
    assert!(desktop.contains("WindowEvent::CloseRequested"), "closing the window hides it");
    assert!(desktop.contains("window.hide()"), "the window goes to the tray, not away");
}

#[test]
fn the_rail_reports_live_work_on_every_screen() {
    assert!(APP.contains("AI.liveThreadSig"), "conversations are read outside the assistant too");
    assert!(APP.contains("live: working > 0"), "the assistant shows a live dot while it works");
    assert!(APP.contains("live: live.length > 0"), "runs show a live dot while they work");
    assert!(APP.contains("rail-badge-"), "the counts are tinted by state");
    assert!(APP.contains("waiting for you"), "a waiting run says so in its hint");
    assert!(APP.contains("threadRole(t) === \"devd\""), "specialists count as runs, not conversations");
}

#[test]
fn the_rail_draws_one_indicator_per_icon() {
    // A working conversation used to render the count badge and the live dot at
    // the same corner, which read as two bubbles.
    assert!(APP.contains("One indicator, never two"), "the renderer chooses one");
    assert!(APP.contains("? '<span class=\"rail-badge rail-badge-'"), "the badge is the indicated state");
    assert!(CSS.contains(".rail-icon .rail-badge ~ .rail-live"), "CSS refuses a second circle if markup ever allows it");
}

#[test]
fn the_tray_can_start_devd_at_login() {
    let desktop = include_str!("../../devd-desktop/src/lib.rs");
    assert!(desktop.contains("Start at login"), "the tray offers the setting");
    assert!(desktop.contains("--start-hidden"), "login starts in the tray, not over the desktop");
    assert!(desktop.contains("CurrentVersion\\Run"), "the per-user Run key is the mechanism");
    assert!(desktop.contains("\"Open agent runs\""), "the tray links to the runs view");
}

#[test]
fn self_healing_is_visible_in_the_status_panel() {
    assert!(APP.contains("Self-healing"), "the status panel names the incidents");
    assert!(APP.contains("stops after three tries"), "it says what the policy is");
    let health = include_str!("../../devd-agent/src/health.rs");
    assert!(health.contains("crate::supervisor::incidents()"), "health carries the incidents");
    let supervisor = include_str!("../../devd-agent/src/supervisor.rs");
    assert!(supervisor.contains("MAX_ATTEMPTS"), "restarts are bounded");
    assert!(supervisor.contains("control_allowed"), "an observe decision is respected");
    assert!(supervisor.contains("toolchain.service"), "a restart goes through the audited capability");
}

#[test]
fn quick_share_is_native_and_wired_all_the_way_down() {
    let sites = include_str!("../../devd-sites/src/caps.rs");
    assert!(sites.contains("\"share.quick\""), "the panel has a state read");
    assert!(sites.contains("\"share.ngrok_setup\""), "the agent can be installed");
    assert!(sites.contains("\"share.ngrok_token_set\""), "the token is stored");
    assert!(sites.contains("\"share.ngrok_start\""), "a project can be shared");
    assert!(sites.contains("\"share.ngrok_stop\""), "a share can be closed");
    assert!(sites.contains("\"share.qr\""), "a QR code is drawn");
    assert!(sites.contains("fn upstream_for"), "the local service is found, not guessed");
    let ngrok = include_str!("../../devd-services/src/ngrok.rs");
    assert!(ngrok.contains("NGROK_AUTHTOKEN"), "the token goes through the environment, not argv");
    assert!(ngrok.contains("fn find_url"), "the address is read from the agent");
    assert!(ngrok.contains("fn qr_svg"), "QR codes are drawn locally");
    assert!(ngrok.contains("MAX_ATTEMPTS") || ngrok.contains("TIMEOUT"), "waiting is bounded");
    assert!(APP.contains("function quickShareSheet"), "the panel exists");
    assert!(APP.contains("data-action=\"quick-share\""), "projects and sites have a Share action");
    assert!(APP.contains("share.ngrok_start"), "the panel starts the tunnel");
    assert!(APP.contains("data-action=\"qs-copy\""), "the address can be copied");
    assert!(APP.contains("qs-qr"), "the QR code is shown");
    let desktop = include_str!("../../devd-desktop/src/lib.rs");
    assert!(desktop.contains("\"Quick Share\""), "the tray has a Quick Share submenu");
    assert!(desktop.contains("share.ngrok_start"), "the tray can share a site");
    assert!(desktop.contains("\"Start All\""), "the tray can start the stack");
    assert!(desktop.contains("\"Services\""), "the tray lists services");
}

#[test]
fn the_tray_menu_reflects_live_state() {
    let desktop = include_str!("../../devd-desktop/src/lib.rs");
    assert!(
        desktop.contains("fn live_from_api"),
        "the tray reads the same API the window does"
    );
    assert!(
        desktop.contains("\"service.list\""),
        "a service's menu entry is built from its real state in one read"
    );
    assert!(
        !desktop.contains("\"service.status\""),
        "the tray does not probe every service separately"
    );
    assert!(
        desktop.contains("set_menu(Some(menu))"),
        "the menu is replaced when that state changes"
    );
    assert!(
        desktop.contains("\"stack.up\""),
        "Start All uses the dependency-ordered stack action"
    );
    assert!(
        desktop.contains("\"stack.down\""),
        "Stop All uses the dependency-ordered stack action"
    );
    assert!(
        desktop.contains("format!(\"service.{action}\")"),
        "per-service controls use the capability that needs no separate approval"
    );
    assert!(
        desktop.contains("\"share:stop:\""),
        "an active share can be stopped from the tray"
    );
    assert!(
        desktop.contains("\"health:page\""),
        "an incident links to the diagnostics that explain it"
    );
}

#[test]
fn danger_buttons_win_over_the_base_button_rule() {
    // The second `.btn` block is the one the interface uses; because it comes
    // later it silently overrode the earlier `.btn-danger` colours, so a
    // destructive button rendered neutral. The danger rule has to be restated
    // after that block.
    let base = CSS
        .find(".btn-mini { height: 26px;")
        .expect("the later base button block exists");
    let danger = CSS
        .rfind(".btn-danger { background: var(--danger-quiet)")
        .expect("the danger rule exists after the base rule");
    assert!(
        danger > base,
        "the danger colours must come after the later .btn block"
    );
}

#[test]
fn portability_is_detected_and_repairable() {
    let p = include_str!("../../devd-agent/src/portability.rs");
    assert!(p.contains("\"portability.status\""), "the home can be checked");
    assert!(p.contains("\"portability.repair\""), "recorded paths can be moved");
    assert!(p.contains("fn remap"), "a prefix is matched on a path boundary");
    assert!(APP.contains("data-action=\"port-check\""), "the diagnostics page offers the check");
    assert!(APP.contains("data-action=\"port-repair\""), "and the repair");
    let bootstrap = include_str!("../../devd-bootstrap/src/lib.rs");
    assert!(bootstrap.contains("portability::register"), "the capabilities are registered");
}

#[test]
fn collapsible_content_is_indented_under_its_heading() {
    assert!(
        CSS.contains("details > :not(summary) { margin-left: 20px; }"),
        "every disclosure steps its content to the right"
    );
    assert!(
        CSS.contains(".ui-fold > :not(summary) { margin-top: var(--s3); margin-left: 20px; }"),
        "the common fold keeps its top gap and gains the indent"
    );
}

#[test]
fn removing_a_project_is_a_danger_action() {
    assert!(
        APP.contains("class=\"btn btn-mini btn-danger\" data-action=\"project-forget\""),
        "Remove uses the danger colour, like Delete"
    );
    assert!(
        APP.contains("class=\"btn btn-mini btn-danger\" data-action=\"project-remove-sites\""),
        "Remove site uses the danger colour too"
    );
}

#[test]
fn the_assistant_has_browser_and_terminal_surfaces() {
    assert!(APP.contains("id=\"work-surface\""), "a work surface exists beside the conversation");
    assert!(APP.contains("class=\"work-pane work-browser\""), "the browser is a surface");
    assert!(APP.contains("class=\"work-pane work-terminal\""), "the terminal is a surface");
    assert!(APP.contains("data-action=\"work-browser\""), "the card opens the browser in place");
    assert!(APP.contains("data-action=\"work-terminal\""), "the card opens the terminal in place");
    assert!(APP.contains("Open browser"), "the card action is named for what it does");
    assert!(APP.contains("Open terminal"), "the card action is named for what it does");
    assert!(!APP.contains("No terminal is running in this project"), "the empty Terminals section is gone");
    assert!(!APP.contains(">Terminal here<"), "the old Terminal here row is gone");
    assert!(APP.contains("allow-scripts allow-same-origin"), "the browser reuses the sandboxed preview frame");
    assert!(APP.contains("at !== AI.loadSeq"), "a stale chat response is discarded");
    assert!(APP.contains("This shell belongs to"), "a shell from another project says which project it belongs to");
    assert!(APP.contains("data-action=\"work-terminal-new\""), "the person can open a shell in the current project");
    assert!(APP.contains("AI.work.browser.project !== project"), "the browser follows the open conversation's project");
}

#[test]
fn the_mcp_screen_does_not_print_machine_paths() {
    // The path is a platform detail the backend knows; the screen is the same on
    // Windows, Linux and macOS and should not look like a Windows registry dump.
    assert!(!APP.contains("attr(c.config)"), "MCP clients do not print their config path");
    assert!(!APP.contains("data-file=\"' + attr(c.config)"), "the dialog does not carry the path either");
    assert!(APP.contains("not found. Open "), "the state still says what to do");
}

#[test]
fn outputs_are_history_and_changes_is_the_working_tree() {
    // "Changes clean" and a long Outputs list are both true: the conversation's
    // writes are history, the working tree is current. The card says which is which.
    assert!(APP.contains("Written by this conversation"), "Outputs explains what it records");
    assert!(APP.contains("is the working tree now"), "Changes is named as the working tree");
    assert!(!APP.contains("o.kind === \"edit\" ? \"changed\""), "Outputs does not claim a current change");
    assert!(APP.contains("o.kind === \"edit\" ? \"edited\""), "Outputs names the conversation's edit");
    assert!(APP.contains("return \"i-file\""), "a source file gets a file icon, not the terminal glyph");
}

#[test]
fn containers_share_one_inset_and_folds_share_one_chevron() {
    // A card's head and body start on the same left edge. A flush body keeps the
    // inset and lets only tables bleed; otherwise a label sits indented above
    // flush text and the container reads as broken.
    assert!(CSS.contains(".panel > .panel-head { padding: 14px 16px 10px; }"));
    assert!(CSS.contains(".panel > .panel-body { padding: 14px 16px; }"));
    assert!(CSS.contains(".panel > .panel-body.flush:not(.files-tree-body) { padding: 0 16px; }"));
    // An implicit grid column sized itself to the longest title and pushed the
    // Home needs cards past the reading column.
    assert!(CSS.contains(".home-needs { grid-template-columns: minmax(0, 1fr); }"));
    // A summary band is a set of readings, not a table.
    assert!(CSS.contains(".summary-band .summary-item, .summary-band .summary-item:first-child { border: 0; }"));
    // A disclosure is the same control everywhere it appears.
    assert!(CSS.contains(
        ":is(.ui-fold, .brief-body, .team-note-body, .run-fold, .mail-env, .decision-args, .run-event-data) > summary"
    ));
    assert!(CSS.contains(".run-goal-fold > summary::before"));
    assert!(CSS.contains(".team-context { border-left: 0; }"));
}

#[test]
fn the_run_toggle_is_a_switch_and_stage_folds_match_section_folds() {
    // .check input sized every checkbox to 14px, which shrank the 36x20 switch
    // into a white knob over a tiny ring.
    assert!(CSS.contains(".check input.ds-toggle { width: 36px; height: 20px; flex: none; }"));
    // Skills groups fold with the same line as Models, not a card.
    assert!(CSS.contains(".skill-stage { background: none; border: 0; border-radius: 0; }"));
    assert!(CSS.contains(".skill-stage > summary.panel-head {"));
}

#[test]
fn list_hover_is_one_rounded_inset_band() {
    // A hovered row paints the same soft, inset, rounded band as uiRow; the row
    // itself stays transparent so its old square background cannot show through.
    assert!(CSS.contains(":is(.row, .svc, .note-row, .file-row, .mail-row, .st-row)::after"));
    assert!(CSS.contains("inset: 1px -12px"));
    assert!(CSS.contains(".row:hover, button.svc:hover, .note-row:hover, .file-row:hover, .mail-row:hover, .st-row:hover { background-color: transparent; }"));
    assert!(CSS.contains(".table tbody tr:hover td:first-child"));
    // A table directly in a panel keeps the same inset as a panel body.
    assert!(CSS.contains(".panel > .table-wrap .table > thead > tr > :first-child"));
}
