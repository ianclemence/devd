//! Structural checks on the interface files. They cannot see pixels, but they
//! catch the regressions that actually happen: a sidebar entry that opens
//! nothing, an action with no handler, a logo asset with transparent padding.

use base64::Engine;

const APP: &str = include_str!("../ui/app.js");
const HTML: &str = include_str!("../ui/index.html");
const CSS: &str = include_str!("../ui/style.css");
const README: &str = include_str!("../../../README.md");
const LIMITATIONS: &str = include_str!("../../../docs/limitations.md");

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
fn the_readme_and_limitations_are_not_double_encoded() {
    // The same ANSI round-trip that hit the interface files had reached the
    // README and the limitations page. The user reads those first.
    for (name, src) in [("README.md", README), ("docs/limitations.md", LIMITATIONS)] {
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
    assert!(HTML.contains("id=\"health-chip\"") && !HTML.contains("health-pill"));
    assert!(HTML.contains("id=\"subnav\""));
    let rail = &APP[APP.find("var groups = [").unwrap()..];
    let rail = &rail[..rail.find("el(\"rail\")").unwrap()];
    let items = rail.matches("{ view: \"").count();
    assert!(items <= 14, "{items} rail entries; related pages belong in tabs");
    assert!(CSS.contains(".subtab"));
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
    let start = APP.find("  function inventoryRows()").expect("inventoryRows");
    let end = start + APP[start..].find("  function renderServices()").expect("renderServices");
    let body = &APP[start..end];
    assert!(APP.contains("function managedServicePid(pid)"), "inventory needs to match a port owner against managed services");
    assert!(body.contains("managedServicePid(v.port_owner.pid)"), "managed services must not appear in the host inventory");
    assert!(
        body.contains("!(v.status === \"missing\" && !v.path && !v.port_owner)"),
        "missing programs must not be counted as already on this computer"
    );
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
    for label in ["All sites", "Hosts", "Certificates"] {
        let needle = format!("\"{label}\"");
        let found = sites[at..]
            .find(&needle)
            .unwrap_or_else(|| panic!("{label} is missing or out of order in the Sites tabs"));
        at += found + needle.len();
    }
    assert!(APP.contains("{ view: \"sites\", label: \"Sites\""), "Sites is a first-class rail destination");
    assert!(APP.contains("page(\"Environment\""), "the environment screen does not call itself a single server");

    let site = quoted_after(APP, "var SITE_TABS = [");
    assert_eq!(
        site,
        vec![
            "overview", "Overview", "env", "Environment", "deploy", "Deploy", "cron", "Cron",
            "queue", "Queue", "logs", "Logs", "laravel", "Laravel", "redirects", "Redirects",
            "nginx", "Web server", "db", "Database", "preview", "Preview"
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
    assert!(APP.contains("return SITE.name ? \"/server/sites/\" + encodeURIComponent(SITE.name) : \"/server/sites\";"));
    assert!(APP.contains("p.indexOf(\"/server/sites/\") === 0"));
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
    for line in CSS.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("--") {
            if let Some((name, _)) = rest.split_once(':') {
                defined.push(name.trim().to_string());
            }
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
    let rule = &CSS[CSS.find(".brand-mark {").unwrap()..];
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
    assert!(
        CSS.contains("@media (hover: none) {\n  .ui-row-actions.hover { opacity: 1; }\n}"),
        "touch screens need the row actions visible"
    );
    let narrow = &CSS[CSS.find("@media (max-width: 720px)").expect("the narrow layout")..];
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
    assert!(APP.contains("href=\"#i-pin\""), "the chapter action uses the pin");
    assert!(!APP.contains("href=\"#i-layers\""), "the chapter action no longer uses the layers icon");
}

#[test]
fn normal_message_sending_does_not_toast() {
    // The conversation's own running state is the acknowledgement. The
    // capability's message is still available to a CLI caller, but the window
    // does not turn every submission into a toast.
    assert!(APP.contains("run(\"chat.send\", args, { quiet: true })"));
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
    assert_eq!(
        APP.matches("data-action=\"open-terminal\"").count(),
        1,
        "there is one terminal action, on the project page"
    );
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
    assert!(body.contains("stop.hidden = !busy || hasText"), "stop follows the draft");
    assert!(body.contains("send.hidden = busy ? hasText : !hasText"), "send follows the draft");
    let later = &APP[end..];
    assert!(
        !later.contains("stop.hidden ="),
        "no later code hides the stop after syncComposerAction decided"
    );
}

#[test]
fn agent_actions_are_grouped_with_their_count() {
    assert!(APP.contains("class=\"msg-tools\""), "steps are wrapped in one group");
    assert!(APP.contains("closeGroup()"), "the group is closed by whatever is not a step");
    assert!(CSS.contains(".msg-tools > .msg-used"), "the count is the group's heading");
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

