//! What the window's frame says about the program right now: the rail's badges, the faces across
//! the top, and the strip along the bottom.
//!
//! The native window draws these from its own stores; the browser client is a page and is handed
//! the same facts by the server (`/shell.js`). The rules are here, once, so the two frames cannot
//! count differently: a failed run is red in both, a held run is amber in both, and "2 agents
//! working" means the same thing in both.

use serde::Serialize;
use serde_json::Value;

/// A count on the rail, in the colour of the most urgent thing it counts.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Badge {
    /// How many.
    pub count: usize,
    /// `neutral`, `ok`, `warn`, `danger` or `info`.
    pub tone: &'static str,
    /// Something else also true of the screen, drawn as a dot at the foot of its button.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub also: Option<String>,
}

/// One face in the top bar: who is at work, and where to go to watch.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Face {
    /// The name, for the tooltip.
    pub label: String,
    /// One or two letters.
    pub initials: String,
    /// The role's colour, or the assistant's, as `#rrggbb`.
    pub colour: String,
    /// Whether it is working (a moving ring).
    pub active: bool,
    /// Where pressing it goes.
    pub route: String,
}

/// The strip along the bottom.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Strip {
    /// The dot's tone.
    pub tone: &'static str,
    /// The sentence beside it.
    pub headline: String,
    /// Agents working now.
    pub working: usize,
    /// Agents stopped for a decision.
    pub waiting: usize,
}

/// Everything the frame shows besides the screen itself.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Chrome {
    /// The rail's badges, by screen segment.
    pub badges: std::collections::BTreeMap<String, Badge>,
    /// The faces in the top bar.
    pub faces: Vec<Face>,
    /// The strip.
    pub strip: Strip,
    /// The release's version, for the strip's right end.
    pub version: String,
    /// A newer release, when there is one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub update: Option<String>,
}

fn tone_of(level: &str) -> &'static str {
    match level {
        "ok" | "healthy" => "ok",
        "warn" | "degraded" | "setup_incomplete" => "warn",
        "danger" | "critical" | "failed" => "danger",
        "info" | "running" | "live" => "info",
        _ => "neutral",
    }
}

fn worse(a: &'static str, b: &'static str) -> &'static str {
    let rank = |t: &str| match t {
        "neutral" => 0,
        "info" => 1,
        "ok" => 2,
        "warn" => 3,
        "danger" => 4,
        _ => 0,
    };
    if rank(b) > rank(a) { b } else { a }
}

/// Who is at work, as the top bar shows them, from the set of conversations that are running.
pub fn faces(running: &[Value], dark: bool) -> Vec<Face> {
    let mut out: Vec<Face> = Vec::new();
    for thread in running {
        let id = thread["id"].as_str().unwrap_or_default();
        let role = thread["team"]["role"].as_str().unwrap_or("devd");
        if out.iter().any(|f| f.label == role_label(role)) {
            continue;
        }
        out.push(Face {
            label: role_label(role),
            initials: initials(&role_label(role)),
            colour: role_colour(role, dark),
            active: thread["state"].as_str() == Some("running"),
            route: format!("/assistant/{}", crate::nav::encode(id)),
        });
    }
    out
}

/// The name shown for a role: `implementer` is "Implementer", the assistant is "devd".
fn role_label(role: &str) -> String {
    if role == "devd" || role.is_empty() {
        return "devd".to_string();
    }
    let mut name = role.replace(['-', '_'], " ");
    if let Some(first) = name.get_mut(0..1) {
        first.make_ascii_uppercase();
    }
    name
}

fn initials(label: &str) -> String {
    let mut words = label.split_whitespace();
    let first = words.next().and_then(|w| w.chars().next());
    let second = words.next().and_then(|w| w.chars().next());
    first.into_iter().chain(second).flat_map(char::to_uppercase).collect()
}

/// The hue a role is drawn in, from 0 to 1: the same on its card in a run's chain, on its face in
/// the top bar and beside its name. Known roles keep a hue of their own; any other role (a custom
/// one) gets one from its name, the same every time.
pub fn role_hue(role: &str) -> f32 {
    match role {
        "planner" => 0.76,
        "product" => 0.83,
        "architect" => 0.66,
        "designer" | "motion" => 0.92,
        "frontend" => 0.58,
        "backend" => 0.47,
        "implementer" => 0.60,
        "qa" => 0.34,
        "security" => 0.01,
        "reviewer" => 0.08,
        "docs" | "content" => 0.12,
        other => {
            // FNV-1a over the name, so a custom role keeps its colour from one run to the next.
            let mut h: u32 = 0x811c_9dc5;
            for b in other.bytes() {
                h = (h ^ u32::from(b)).wrapping_mul(0x0100_0193);
            }
            (h % 360) as f32 / 360.0
        }
    }
}

/// The colour of a role's face on the page: the window's hue, at the contrast a face needs
/// (lighter on a dark palette, darker on a light one).
pub fn role_colour(role: &str, dark: bool) -> String {
    if role == "devd" || role.is_empty() {
        return if dark { "#d4f24a".to_string() } else { "#5c6b00".to_string() };
    }
    hsl(role_hue(role) * 360.0, 0.58, if dark { 0.68 } else { 0.38 })
}

fn hsl(h: f32, s: f32, l: f32) -> String {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = l - c / 2.0;
    let (r, g, b) = match h as u32 / 60 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let p = |v: f32| ((v + m) * 255.0).round().clamp(0.0, 255.0) as u8;
    format!("#{:02x}{:02x}{:02x}", p(r), p(g), p(b))
}

/// The frame's whole state, from what the server knows.
pub fn of(session: &Value, running: &[Value], held: usize, version: &str, update: Option<String>, dark: bool) -> Chrome {
    let mut badges = std::collections::BTreeMap::new();
    // The Inbox counts everything open, and speaks in the colour of the most urgent thing in it.
    let mut waiting = held;
    let mut inbox_tone = "neutral";
    if let Some(list) = session["inbox"].as_array() {
        for message in list.iter().filter(|m| m["state"] == "open") {
            waiting += 1;
            inbox_tone = worse(inbox_tone, tone_of(message["level"].as_str().unwrap_or("")));
        }
    }
    if held > 0 {
        inbox_tone = worse(inbox_tone, "warn");
    }
    if waiting > 0 {
        badges.insert("inbox".to_string(), Badge { count: waiting, tone: inbox_tone, also: None });
    }
    // Runs: what holds a run up first, and the colour of the worst state in that group; a dot
    // says work is still going on elsewhere.
    let mut going = 0usize;
    let mut stopped = 0usize;
    let (mut going_tone, mut stopped_tone) = ("neutral", "neutral");
    for run in session["runs"].as_array().into_iter().flatten() {
        match run["state"].as_str().unwrap_or_default() {
            "running" | "planning" | "working" => {
                going += 1;
                going_tone = worse(going_tone, tone_of("running"));
            }
            "paused" | "waiting" => {
                stopped += 1;
                stopped_tone = worse(stopped_tone, "warn");
            }
            _ => {}
        }
    }
    if stopped > 0 {
        badges.insert(
            "runs".to_string(),
            Badge {
                count: stopped,
                tone: stopped_tone,
                also: (going > 0).then(|| format!("#{:02x}{:02x}{:02x}", 0x6a, 0xa6, 0xff)),
            },
        );
    } else if going > 0 {
        badges.insert("runs".to_string(), Badge { count: going, tone: going_tone, also: None });
    }
    let clean = session["health"]["state"].as_str().unwrap_or_default();
    let tone = match clean {
        "healthy" => "ok",
        "critical" => "danger",
        "degraded" | "setup_incomplete" => "warn",
        _ => "neutral",
    };
    let headline = if clean == "healthy" {
        session["health"]["notes"]
            .as_u64()
            .filter(|n| *n > 0)
            .map(|_| "Everything is running".to_string())
            .unwrap_or_else(|| "Everything is running".to_string())
    } else {
        session["health"]["headline"].as_str().unwrap_or("Checking").to_string()
    };
    Chrome {
        badges,
        faces: faces(running, dark),
        strip: Strip { tone, headline, working: running.len(), waiting: held },
        version: version.to_string(),
        update,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_healthy_machine_says_so_in_one_short_sentence() {
        let chrome = of(
            &json!({ "health": { "state": "healthy", "notes": 4 }, "inbox": [], "runs": [] }),
            &[],
            0,
            "0.45.0",
            None,
            true,
        );
        assert_eq!(chrome.strip.tone, "ok");
        assert_eq!(chrome.strip.headline, "Everything is running");
        assert!(chrome.badges.is_empty());
        assert_eq!(chrome.version, "0.45.0");
    }

    #[test]
    fn the_inbox_and_the_runs_are_counted_in_the_colour_of_the_worst_in_them() {
        let chrome = of(
            &json!({
                "health": { "state": "degraded", "headline": "One service is down" },
                "inbox": [
                    { "state": "open", "level": "info" },
                    { "state": "open", "level": "danger" },
                    { "state": "read", "level": "danger" }
                ],
                "runs": [ { "state": "running" }, { "state": "running" }, { "state": "done" } ]
            }),
            &[],
            1,
            "0.45.0",
            None,
            true,
        );
        assert_eq!(chrome.strip.tone, "warn");
        assert_eq!(chrome.strip.headline, "One service is down");
        // Two open messages plus one request held for a person.
        assert_eq!(chrome.badges["inbox"].count, 3);
        assert_eq!(chrome.badges["inbox"].tone, "danger");
        assert_eq!(chrome.badges["runs"].count, 2);
        assert_eq!(chrome.badges["runs"].tone, "info", "a running run is the window's info colour");
    }

    #[test]
    fn a_face_is_one_per_member_and_names_what_it_is_doing() {
        let running = vec![
            json!({ "id": "a", "state": "running", "team": { "role": "implementer" } }),
            json!({ "id": "b", "state": "running", "team": { "role": "implementer" } }),
            json!({ "id": "c", "state": "running", "team": { "role": null } }),
        ];
        let faces = faces(&running, true);
        let names: Vec<&str> = faces.iter().map(|f| f.label.as_str()).collect();
        assert_eq!(names, ["Implementer", "devd"]);
        assert_eq!(faces[0].initials, "I");
        assert_eq!(faces[1].initials, "D");
        assert!(faces.iter().all(|f| f.active));
        assert!(faces[0].colour.starts_with('#'));
        assert_eq!(faces[1].route, "/assistant/c");
    }
}
