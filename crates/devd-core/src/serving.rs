//! What a site serves, and why devd believes it.
//!
//! A directory is not a web site. A Go service, a Node application and a PHP
//! library are all projects; only some have anything for a web server to serve,
//! and of those only some run PHP. Treating every directory as a PHP site put
//! "PHP 8.5" beside a Go module and served its source tree over HTTPS.
//!
//! Three questions are kept apart here, because they have different owners:
//!
//! * **What is in the directory** ([`detect`]): evidence read from the files.
//!   Never stored, and never acted on by itself.
//! * **What the person chose** ([`Content`] on the site): an explicit
//!   `php` or `static`, or nothing.
//! * **What is served** ([`effective`]): the choice if there is one, else what
//!   detection found, else nothing. This is the only answer the web-server
//!   renderers and the surfaces use, so they cannot disagree.
//!
//! A proxy site (one with an upstream) serves a running program and has no
//! directory content at all.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::state::Site;

/// What a directory site serves when a person has said so.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Content {
    /// PHP, through FastCGI: `index.php` and every `.php` file.
    Php,
    /// Files as they are.
    Static,
}

impl Content {
    /// The stored spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            Content::Php => "php",
            Content::Static => "static",
        }
    }

    /// Parse the stored spelling.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "php" => Some(Content::Php),
            "static" => Some(Content::Static),
            _ => None,
        }
    }
}

/// What a site is actually served as.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Serving {
    /// Every request goes to a running program.
    Proxy,
    /// PHP through FastCGI.
    Php,
    /// Files as they are.
    Static,
    /// A directory with nothing a web server can serve. The site answers with
    /// an explanation instead of exposing the directory.
    NoEntry,
}

impl Serving {
    /// Whether PHP runs for this site, which is the only case a PHP version means anything.
    pub fn uses_php(self) -> bool {
        self == Serving::Php
    }
}

/// Where the answer came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    /// The site has an upstream.
    Upstream,
    /// A person chose it.
    Configured,
    /// It was read from the files.
    Detected,
}

/// One file that says what a directory is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Evidence {
    /// The language or ecosystem the file points to.
    pub language: String,
    /// The file, relative to the project.
    pub marker: String,
}

/// What a project directory contains, as far as serving it goes.
#[derive(Debug, Clone, Serialize)]
pub struct Detection {
    /// Where the web server would be rooted: `public/` when it holds the
    /// application's entry point, otherwise the project folder.
    pub web_root: PathBuf,
    /// What there is to serve, when anything is.
    pub content: Option<Content>,
    /// The file that makes it servable.
    pub entry: Option<String>,
    /// Every ecosystem the project's files point to. More than one is normal.
    pub languages: Vec<Evidence>,
    /// Things a person should know before relying on the answer.
    pub notes: Vec<String>,
}

impl Detection {
    /// The languages, without repeats, in the order found.
    pub fn language_names(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for e in &self.languages {
            if !out.contains(&e.language) {
                out.push(e.language.clone());
            }
        }
        out
    }

    /// A sentence for a person: what this directory is and why it is or is not served.
    pub fn summary(&self) -> String {
        let languages = self.language_names();
        let what = if languages.is_empty() { "no recognised project files".to_string() } else { languages.join(" + ") };
        match (&self.content, &self.entry) {
            (Some(Content::Php), Some(entry)) => format!("{what}; serves PHP ({entry})"),
            (Some(Content::Static), Some(entry)) => format!("{what}; serves files ({entry})"),
            _ => format!("{what}; no web entry point (no index.php or index.html in {})", self.web_root.display()),
        }
    }
}

/// The folder a web server should be rooted at for a project.
///
/// `public/` is the application's web root when it holds the entry point, or
/// when the project has none of its own. A `public/` that holds only static
/// assets beside a project that has its own `index.html` (a Vite app, say) is
/// not a document root: the project folder is.
pub fn web_root(project: &Path) -> PathBuf {
    let public = project.join("public");
    let has_entry = |dir: &Path| ["index.php", "index.html", "index.htm"].iter().any(|n| dir.join(n).is_file());
    if public.is_dir() && (has_entry(&public) || !has_entry(project)) {
        public
    } else {
        project.to_path_buf()
    }
}

/// Read a project directory. Nothing is written, and the result is not stored.
pub fn detect(project: &Path) -> Detection {
    let web_root = web_root(project);
    let mut languages: Vec<Evidence> = Vec::new();
    let mut note = |language: &str, marker: &str| {
        if project.join(marker).exists() {
            languages.push(Evidence { language: language.to_string(), marker: marker.to_string() });
        }
    };
    for (language, marker) in [
        ("php", "artisan"),
        ("php", "composer.json"),
        ("node", "package.json"),
        ("go", "go.mod"),
        ("rust", "Cargo.toml"),
        ("python", "pyproject.toml"),
        ("python", "requirements.txt"),
        ("python", "manage.py"),
        ("ruby", "Gemfile"),
        ("java", "pom.xml"),
        ("java", "build.gradle"),
        ("java", "build.gradle.kts"),
        ("dotnet", "global.json"),
    ] {
        note(language, marker);
    }

    let php_entry = ["index.php"]
        .iter()
        .find(|n| web_root.join(n).is_file())
        .map(|n| (*n).to_string())
        .or_else(|| first_php_file(&web_root));
    let static_entry = ["index.html", "index.htm"].iter().find(|n| web_root.join(n).is_file()).map(|n| (*n).to_string());

    let (content, entry) = match (php_entry, static_entry) {
        (Some(php), _) => (Some(Content::Php), Some(php)),
        (None, Some(html)) => (Some(Content::Static), Some(html)),
        (None, None) => (None, None),
    };

    let mut notes = Vec::new();
    if content == Some(Content::Static) && has_dev_server(project) {
        notes.push("a Node project with a dev server: its index.html is a development entry, so a proxy site to the running dev server usually serves it better".to_string());
    }
    if content.is_none() && !languages.is_empty() {
        notes.push("a project without a web entry point is not a site; a program that listens on a port can be served as a proxy site".to_string());
    }
    Detection { web_root, content, entry, languages, notes }
}

/// The first `.php` file directly in a folder, by name, so a plain folder of scripts is PHP.
fn first_php_file(dir: &Path) -> Option<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter(|e| e.path().is_file())
        .filter_map(|e| e.file_name().to_str().map(str::to_string))
        .filter(|n| n.to_ascii_lowercase().ends_with(".php"))
        .collect();
    names.sort();
    names.into_iter().next()
}

/// Whether `package.json` declares a script that starts a development server.
fn has_dev_server(project: &Path) -> bool {
    let Ok(text) = std::fs::read_to_string(project.join("package.json")) else { return false };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) else { return false };
    json.get("scripts")
        .and_then(|s| s.as_object())
        .is_some_and(|s| ["dev", "start", "serve"].iter().any(|k| s.contains_key(*k)))
}

/// The answer the renderers and the surfaces use.
#[derive(Debug, Clone, Serialize)]
pub struct Effective {
    /// What is served.
    pub serving: Serving,
    /// Where that came from.
    pub source: Source,
    /// What the directory holds. `None` for a proxy site, which has no directory content.
    pub detection: Option<Detection>,
}

/// What a site is served as: its upstream, else the person's choice, else what its files are.
pub fn effective(site: &Site) -> Effective {
    if site.upstream.is_some() {
        return Effective { serving: Serving::Proxy, source: Source::Upstream, detection: None };
    }
    let detection = detect(Path::new(&site.root));
    match site.serving {
        Some(Content::Php) => Effective { serving: Serving::Php, source: Source::Configured, detection: Some(detection) },
        Some(Content::Static) => Effective { serving: Serving::Static, source: Source::Configured, detection: Some(detection) },
        None => {
            let serving = match detection.content {
                Some(Content::Php) => Serving::Php,
                Some(Content::Static) => Serving::Static,
                None => Serving::NoEntry,
            };
            Effective { serving, source: Source::Detected, detection: Some(detection) }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("devd-serving-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn site(root: &Path) -> Site {
        Site {
            id: "1".into(),
            name: "s".into(),
            host: "s.test".into(),
            root: root.display().to_string(),
            web_server: "nginx".into(),
            php: None,
            serving: None,
            tls_mode: "off".into(),
            db_name: None,
            upstream: None,
            created_at: String::new(),
        }
    }

    #[test]
    fn a_laravel_project_serves_php_from_public() {
        let d = dir("laravel");
        fs::create_dir_all(d.join("public")).unwrap();
        fs::write(d.join("public").join("index.php"), "<?php").unwrap();
        fs::write(d.join("artisan"), "").unwrap();
        fs::write(d.join("composer.json"), "{}").unwrap();
        fs::write(d.join("package.json"), "{}").unwrap();
        let e = effective(&site(&d));
        assert_eq!(e.serving, Serving::Php);
        assert_eq!(e.source, Source::Detected);
        let det = e.detection.unwrap();
        assert!(det.web_root.ends_with("public"));
        assert_eq!(det.language_names(), vec!["php", "node"], "both are evidence, neither is hidden");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn a_go_module_has_no_web_entry_and_is_never_php() {
        let d = dir("go");
        fs::write(d.join("go.mod"), "module x").unwrap();
        fs::write(d.join("main.go"), "package main").unwrap();
        let e = effective(&site(&d));
        assert_eq!(e.serving, Serving::NoEntry);
        assert!(!e.serving.uses_php());
        assert!(e.detection.unwrap().summary().contains("go; no web entry point"));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn a_node_app_with_an_index_is_static_and_says_a_dev_server_is_the_better_fit() {
        let d = dir("node");
        fs::write(d.join("package.json"), r#"{"scripts":{"dev":"vite"}}"#).unwrap();
        fs::write(d.join("index.html"), "<html>").unwrap();
        let e = effective(&site(&d));
        assert_eq!(e.serving, Serving::Static);
        assert!(e.detection.unwrap().notes.iter().any(|n| n.contains("proxy site")));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn a_folder_of_php_scripts_without_an_index_is_still_php() {
        let d = dir("scripts");
        fs::write(d.join("report.php"), "<?php").unwrap();
        assert_eq!(effective(&site(&d)).serving, Serving::Php);
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn a_php_library_is_a_project_and_not_a_site() {
        let d = dir("lib");
        fs::write(d.join("composer.json"), "{}").unwrap();
        fs::create_dir_all(d.join("src")).unwrap();
        fs::write(d.join("src").join("Thing.php"), "<?php").unwrap();
        let e = effective(&site(&d));
        assert_eq!(e.serving, Serving::NoEntry, "files under src/ are not a web entry point");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn a_choice_wins_over_detection_and_a_proxy_wins_over_both() {
        let d = dir("choice");
        fs::write(d.join("go.mod"), "module x").unwrap();
        let mut s = site(&d);
        s.serving = Some(Content::Static);
        let e = effective(&s);
        assert_eq!((e.serving, e.source), (Serving::Static, Source::Configured));
        s.upstream = Some("http://127.0.0.1:3000".into());
        let e = effective(&s);
        assert_eq!((e.serving, e.source), (Serving::Proxy, Source::Upstream));
        assert!(e.detection.is_none(), "a proxy site has no directory content to describe");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn the_stored_spelling_round_trips() {
        for c in [Content::Php, Content::Static] {
            assert_eq!(Content::parse(c.as_str()), Some(c));
        }
        assert_eq!(Content::parse("proxy"), None, "a proxy is an upstream, not a content choice");
    }

    #[test]
    fn a_public_folder_of_assets_beside_a_project_index_is_not_the_web_root() {
        let d = dir("vite");
        fs::create_dir_all(d.join("public")).unwrap();
        fs::write(d.join("public").join("logo.svg"), "").unwrap();
        fs::write(d.join("index.html"), "").unwrap();
        assert_eq!(web_root(&d), d);
        let _ = fs::remove_dir_all(&d);
    }
}
