//! Downloads Tidy - Rule-based Downloads folder sorter with preview and one-click undo.
//!
//!   downloads_tidy.exe                 open the app
//!   downloads_tidy.exe --auto-update   internal: background update check/download

#![windows_subsystem = "windows"]

mod app;
mod feedback;
// Enough of the house theme for feedback.rs; `polish` brings the real one.
#[path = "theme_lite.rs"]
mod theme;
mod recycle;
mod rules;
mod settings;
mod tidy;
mod update;

fn main() {
    // First thing, always: Velopack may need to run an install/uninstall/
    // update hook and exit before any of our own argument handling runs.
    update::run_app_hooks();

    let args: Vec<String> = std::env::args().skip(1).collect();

    // The detached child spawned below; must not spawn another.
    if args.first().map(String::as_str) == Some("--auto-update") {
        update::run_background_update();
        return;
    }
    update::maybe_spawn_background_update();

    if let Err(e) = app::run(args) {
        eprintln!("{e}");
    }
}
