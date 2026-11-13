//! The native devd application.
//!
//! A window drawn on the GPU by gpui, with no webview: the same capabilities
//! the command line and the browser interface use, kept in a local cache so
//! every screen opens with what it already knows and then catches up.
//!
//! This file only starts things. What the window decides is in `devd-app-core`,
//! where it is tested without a window; what it draws is in the modules here.

// A release build has no console window on Windows; a debug build keeps one,
// so a panic is visible while developing.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod assets;
mod backend;
mod scheme;
mod screens;
mod shell;
mod state;
mod theme;
#[cfg(test)]
mod truth;
mod ui;

use gpui::{px, size, App, AppContext, Application, Bounds, TitlebarOptions, WindowBounds, WindowOptions};

fn main() {
    // `devd-app --register-links` makes the operating system open devd:// links with this program.
    if std::env::args().nth(1).as_deref() == Some("--register-links") {
        let result = std::env::current_exe().map_err(|e| e.to_string()).and_then(|exe| scheme::register(&exe));
        match result {
            Ok(done) => println!("{done}"),
            Err(why) => {
                eprintln!("{why}");
                std::process::exit(1);
            }
        }
        return;
    }
    let conn = match backend::connect() {
        Ok(conn) => conn,
        Err(message) => {
            // There is no window to say this in yet.
            eprintln!("{message}");
            std::process::exit(1);
        }
    };
    // `devd-app /sites/blog` opens on that place: the same text a link or an
    // agent uses to name it.
    let start = std::env::args().nth(1).and_then(|arg| devd_app_core::nav::Route::parse(&arg)).unwrap_or(devd_app_core::nav::Route::Home);
    let start_theme = theme::load(std::path::Path::new(&conn.home));
    let closing = conn.clone();
    Application::new().with_assets(assets::Assets).run(move |cx: &mut App| {
        cx.set_global(start_theme);
        ui::input::bind_keys(cx);
        shell::bind_keys(cx);
        state::start(conn.clone(), cx);

        let bounds = Bounds::centered(None, size(px(1360.0), px(860.0)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions { title: Some("devd".into()), ..TitlebarOptions::default() }),
            window_min_size: Some(size(px(880.0), px(560.0))),
            app_id: Some("devd".into()),
            ..WindowOptions::default()
        };
        let opened = cx.open_window(options, |window, cx| cx.new(|cx| shell::Shell::new(start.clone(), window, cx)));
        if let Err(e) = opened {
            eprintln!("devd could not open its window: {e}");
            cx.quit();
            return;
        }
        cx.activate(true);

        // Closing the window ends the application. If this process is the
        // instance, the server it started stops with it; a client leaves the
        // running devd as it found it.
        let closing = closing.clone();
        cx.on_window_closed(move |cx| {
            if cx.windows().is_empty() {
                // The browser this instance started must not outlive it: it would hold its profile.
                if closing.owner {
                    let _ = devd_agent::browser::close();
                }
                closing.close();
                cx.quit();
            }
        })
        .detach();
    });
}
