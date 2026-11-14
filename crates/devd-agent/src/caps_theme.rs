//! Themes from files, as capabilities: list them, check one, save one, wear one, remove one.
//!
//! The assistant never writes a theme's files itself. It gives `theme.save` values, devd checks them
//! (every colour read, every token worked out, everything a person reads measured) and only then
//! writes the folder. A theme saved this way is always one the window can use, and when the check
//! refuses one, every problem comes back at once so the colours can be fixed in a single pass.
//! The format and the check are `devd_core::theme`; see `docs/themes.md`.

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{json, Value};

use devd_core::capability::{Effect, Outcome, Plan, Tier};
use devd_core::theme::{self, Manifest, ThemeMod, ThemeSpec};
use devd_core::{DevdError, Registry, Result};

#[derive(Debug, Clone, Deserialize, JsonSchema)]
struct NoArgs {}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
struct ThemeValues {
    /// The theme's folder in `mods/`: 1 to 40 lowercase letters, digits and hyphens, such as `paper`.
    id: String,
    /// `mod.toml`: what a person calls the theme, its version, and one sentence about how it looks.
    #[serde(rename = "mod")]
    manifest: Manifest,
    /// `theme.toml`: its feel, its colours (background, text and accent at least) and its picture.
    theme: ThemeSpec,
    /// Replace a theme already in that folder. Without it, an existing theme is never overwritten.
    #[serde(default)]
    replace: bool,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
struct CheckArgs {
    /// Check the theme already in `mods/<id>`. Give this alone to check a folder.
    #[serde(default)]
    id: Option<String>,
    /// `mod.toml` values to check instead of a folder.
    #[serde(default, rename = "mod")]
    manifest: Option<Manifest>,
    /// `theme.toml` values to check instead of a folder.
    #[serde(default)]
    theme: Option<ThemeSpec>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
struct UseArgs {
    /// A theme from a file, by its folder (`paper`), or a built-in theme: sling, dawn, psalm, sharon or daybreak.
    id: String,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
struct RemoveArgs {
    /// The theme's folder in `mods/`.
    id: String,
}

/// The built-in themes, which `theme.use` also accepts.
const BUILT_IN: [&str; 5] = ["sling", "dawn", "psalm", "sharon", "daybreak"];

fn plan(id: &str, text: impl Into<String>) -> Result<Plan> {
    Ok(Plan::new(id, text))
}

/// What an agent is told about a theme that passed: where it is and the colours it came to, with the
/// contrast of what is read, so it can tell the person why the theme reads well.
fn described(m: &ThemeMod, dir: &std::path::Path) -> Value {
    let t = &m.palette.tokens;
    let ratio = |a, b| (theme::contrast(a, b) * 10.0).round() / 10.0;
    json!({
        "id": m.id,
        "theme": format!("mod:{}", m.id),
        "name": m.manifest.name,
        "folder": dir.display().to_string(),
        "dark": m.palette.dark,
        "scene": m.palette.scene.name(),
        // The values as saved, in the shape `theme.save` takes: change what was asked for, keep the rest,
        // and save again with `replace: true`.
        "values": { "id": m.id, "mod": m.manifest, "theme": m.spec },
        "tokens": t,
        "contrast": {
            "text_on_background": ratio(t.text, t.bg),
            "muted_on_background": ratio(t.muted, t.bg),
            "faint_on_background": ratio(t.faint, t.bg),
            "accent_on_background": ratio(t.accent, t.bg),
            "text_on_accent": ratio(t.on_accent, t.accent),
        },
    })
}

/// The theme the window is wearing, as it saved it: `mod:paper`, or a built-in id such as `sling`.
fn in_use(paths: &devd_core::paths::Paths) -> String {
    std::fs::read_to_string(paths.root().join("window-theme"))
        .ok()
        .and_then(|text| text.split_whitespace().next().map(str::to_string))
        .unwrap_or_else(|| "sling".into())
}

/// A refusal that lists every problem, one per line, so all of them can be fixed at once.
fn refused(problems: Vec<String>) -> DevdError {
    DevdError::invalid(format!(
        "devd cannot use this theme yet. Fix every one of these and try again:\n- {}",
        problems.join("\n- ")
    ))
}

pub fn register(reg: &mut Registry) -> Result<()> {
    reg.define::<NoArgs>("theme.list", "List the themes from files")
        .description("Every theme in devd's mods folder, with whether the window can use it and, if not, exactly why, and which theme the window is wearing (`in_use`: `mod:<folder>` for a theme from a file, or a built-in id). The built-in themes (sling, dawn, psalm, sharon, daybreak) are not listed; they are always there. To change a theme, read its values with theme.check and save them again with theme.save and `replace: true`.")
        .group("theme")
        .tier(Tier::Read)
        .effect(Effect::Read)
        .agent_safe()
        .dry_run()
        .idempotent()
        .handlers(
            |_, _| plan("theme.list", "list the themes in the mods folder"),
            |ctx, _| {
                let dir = ctx.paths.mods();
                let found = theme::scan(&dir);
                let list: Vec<Value> = found
                    .iter()
                    .map(|f| match &f.theme {
                        Ok(m) => json!({ "id": f.id, "usable": true, "name": m.manifest.name, "feel": m.spec.feel, "dark": m.palette.dark, "scene": m.palette.scene.name() }),
                        Err(problems) => json!({ "id": f.id, "usable": false, "problems": problems }),
                    })
                    .collect();
                let usable = found.iter().filter(|f| f.theme.is_ok()).count();
                Ok(Outcome::with_data(
                    format!("{} theme(s) in {}, {usable} usable", found.len(), dir.display()),
                    json!({ "folder": dir.display().to_string(), "in_use": in_use(&ctx.paths), "themes": list }),
                ))
            },
        )?;

    reg.define::<CheckArgs>("theme.check", "Check a theme without saving it")
        .description("Checks a theme the way the window will: every colour read, the other tokens worked out, and the contrast of everything a person reads measured. Give `id` alone to check the theme in that folder: the answer includes its saved `values`, in the shape theme.save takes, which is how to change a theme (edit the values, save with `replace: true`). Or give `mod` and `theme` values to check a theme before saving it. Nothing is written.")
        .group("theme")
        .tier(Tier::Read)
        .effect(Effect::Read)
        .agent_safe()
        .dry_run()
        .idempotent()
        .handlers(
            |_, _| plan("theme.check", "check a theme; nothing is written"),
            |ctx, a| {
                let dir = ctx.paths.mods();
                let checked = match (&a.id, &a.manifest, &a.theme) {
                    (id, Some(manifest), Some(spec)) => {
                        let id = id.clone().unwrap_or_else(|| "draft".into());
                        theme::check_values(&id, manifest, spec)
                            .map(|palette| ThemeMod { id, manifest: manifest.clone(), spec: spec.clone(), palette })
                    }
                    (Some(id), None, None) => {
                        if !theme::valid_id(id) {
                            return Err(DevdError::invalid(format!("\"{id}\" is not a theme's folder name")));
                        }
                        if !dir.join(id).join(theme::THEME).is_file() {
                            return Err(DevdError::not_found(format!("there is no theme in {}", dir.join(id).display())));
                        }
                        theme::load(&dir.join(id))
                    }
                    _ => return Err(DevdError::invalid("give `id` to check a folder, or both `mod` and `theme` to check values")),
                };
                match checked {
                    Ok(m) => Ok(Outcome::with_data("the theme can be used", described(&m, &dir.join(&m.id)))),
                    Err(problems) => Ok(Outcome::with_data(
                        format!("the theme cannot be used yet: {} problem(s)", problems.len()),
                        json!({ "ok": false, "problems": problems }),
                    )),
                }
            },
        )?;

    reg.define::<ThemeValues>("theme.save", "Save a theme to the mods folder")
        .description("Checks a theme and, only if it passes, writes its folder in devd's mods folder (`mod.toml` and `theme.toml`), where the window picks it up within a second. Three colours are needed: background, text and accent; every other colour is worked out in the proportions devd's own themes use unless given. When the check refuses the theme, nothing is written and every problem is returned at once, each with the contrast reached, the contrast needed and which way to move. An existing theme is replaced only with `replace: true`. Use `theme.use` afterwards to show it.")
        .group("theme")
        .tier(Tier::Write)
        .effect(Effect::Write)
        .agent_safe()
        .dry_run()
        .handlers(
            |_, a| plan("theme.save", format!("save the theme \"{}\" in mods/{}", a.manifest.name, a.id)),
            |ctx, a| {
                let dir = ctx.paths.mods();
                if !a.replace && dir.join(&a.id).join(theme::THEME).is_file() {
                    return Err(DevdError::conflict(format!(
                        "mods/{} already holds a theme; pass `replace: true` to change it, or choose another id",
                        a.id
                    )));
                }
                let saved = theme::write(&dir, &a.id, &a.manifest, &a.theme).map_err(refused)?;
                ctx.events.emit_with("theme.saved", format!("theme {} saved", saved.id), json!({ "id": saved.id }));
                Ok(Outcome::with_data(
                    format!("Saved \"{}\" in {}; it is in Settings under Your themes", saved.manifest.name, dir.join(&saved.id).display()),
                    described(&saved, &dir.join(&saved.id)),
                ))
            },
        )?;

    reg.define::<UseArgs>("theme.use", "Show a theme in the window")
        .description("Asks the native window to wear a theme: one from the mods folder by its folder name, or a built-in one (sling, dawn, psalm, sharon, daybreak). It changes only how the window looks, and the person can choose another in Settings at any time. A theme that cannot be used is refused with its problems.")
        .group("theme")
        .tier(Tier::Read)
        .effect(Effect::Read)
        .agent_safe()
        .dry_run()
        .handlers(
            |_, a| plan("theme.use", format!("show the theme {} in the window", a.id)),
            |ctx, a| {
                let id = a.id.trim().trim_start_matches("mod:");
                let wanted = if BUILT_IN.contains(&id) {
                    id.to_string()
                } else {
                    let dir = ctx.paths.mods().join(id);
                    if !theme::valid_id(id) || !dir.join(theme::THEME).is_file() {
                        return Err(DevdError::not_found(format!(
                            "there is no theme called `{id}`; theme.list shows the ones in the mods folder, and the built-in ones are {}",
                            BUILT_IN.join(", ")
                        )));
                    }
                    theme::load(&dir).map_err(refused)?;
                    format!("mod:{id}")
                };
                ctx.events.emit_with("app.theme", format!("wear {wanted}"), json!({ "theme": wanted }));
                Ok(Outcome::with_data(format!("Asked the window to wear {wanted}"), json!({ "theme": wanted })))
            },
        )?;

    reg.define::<RemoveArgs>("theme.remove", "Remove a theme from the mods folder")
        .description("Deletes a theme's folder from the mods folder. A window wearing it goes back to Sling. This cannot be undone, so it waits for the person to approve it.")
        .group("theme")
        .tier(Tier::Destructive)
        .effect(Effect::Write)
        .agent_safe()
        .dry_run()
        .handlers(
            |ctx, a| {
                let dir = ctx.paths.mods().join(&a.id);
                Ok(Plan::new("theme.remove", format!("remove the theme in {}", dir.display()))
                    .effect(format!("deletes {} and everything in it", dir.display()))
                    .approval(true))
            },
            |ctx, a| {
                if !theme::valid_id(&a.id) {
                    return Err(DevdError::invalid(format!("\"{}\" is not a theme's folder name", a.id)));
                }
                let dir = ctx.paths.mods().join(&a.id);
                if !dir.join(theme::THEME).is_file() {
                    return Err(DevdError::not_found(format!("there is no theme in {}", dir.display())));
                }
                std::fs::remove_dir_all(&dir).map_err(|e| DevdError::internal(format!("could not remove {}: {e}", dir.display())))?;
                ctx.events.emit_with("theme.removed", format!("theme {} removed", a.id), json!({ "id": a.id }));
                Ok(Outcome::ok(format!("Removed the theme in {}", dir.display())))
            },
        )?;
    Ok(())
}
