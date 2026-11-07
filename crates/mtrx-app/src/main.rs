//! Mtrx desktop app.

// No console window next to the app on Windows.
#![cfg_attr(windows, windows_subsystem = "windows")]
// matrix-sdk futures are deeply nested.
#![recursion_limit = "256"]

mod actions;
mod app;
mod appearance;
mod arrange;
mod avatar;
mod badge;
mod calls;
mod clipboard;
mod context_menu;
mod dialog;
mod directory;
mod downloads;
mod drag_scroll;
mod edit_menu;
mod ellipsis;
mod emoji;
mod feedback;
mod files;
mod find;
mod fonts;
mod frame;
mod highlight;
mod hint;
mod icons;
mod info;
mod join;
mod last_view;
mod link_status;
#[cfg(target_os = "linux")]
mod linux;
mod login;
mod logo;
mod math;
mod media;
mod members;
mod mention;
mod menu;
#[cfg(target_os = "macos")]
mod menu_bar;
mod message;
mod message_field;
mod message_menu;
mod overview;
mod pdf;
mod pdf_viewer;
mod pins;
mod placement;
mod popover;
mod presence;
mod reorder;
mod room;
mod rows;
mod selectable;
mod selection;
mod send_later;
mod settings;
mod sidebar;
mod spelling;
mod status;
mod style;
mod task;
mod text_size;
mod tint;
mod topic;
mod update;
mod want;
#[cfg(windows)]
mod windows;

fn main() -> iced::Result {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                "warn,matrix_sdk=error,matrix_sdk::http_client=off,matrix_sdk::latest_events=off,\
                 wgpu=error,naga=error"
                    .into()
            }),
        )
        .init();
    let environment = app::Environment::load().expect("valid MTRX_PROFILE and data directory");
    log_panics(environment.data_root().join(CRASH_LOG));
    let window = environment.window();
    iced::application(
        move || app::App::boot(environment.clone()),
        app::App::update,
        app::App::view,
    )
    .title("Mtrx")
    .subscription(app::App::subscription)
    .theme(app::App::window_theme)
    .scale_factor(app::App::scale_factor)
    .settings(fonts::settings())
    .window(app::closing(iced::window::Settings {
        icon: logo::window_icon(),
        #[cfg(target_os = "linux")]
        platform_specific: linux::window_settings(),
        // Size, position and maximised as the window was left.
        ..window
    }))
    .run()
}

/// Where panics are written down, in the data directory.
const CRASH_LOG: &str = "crash.log";
/// Past this size the log starts over.
const CRASH_LOG_MAX: u64 = 1 << 20;

/// Panics also go to `path`, with a backtrace: started from the Finder or
/// a menu, the app has no terminal to print them to.
fn log_panics(path: std::path::PathBuf) {
    let print = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        print(info);
        let report = format!(
            "{} Mtrx {}, {}\n{}\n",
            chrono::Local::now().to_rfc3339(),
            env!("CARGO_PKG_VERSION"),
            info,
            std::backtrace::Backtrace::force_capture()
        );
        let _ = append_report(&path, &report);
    }));
}

fn append_report(path: &std::path::Path, report: &str) -> std::io::Result<()> {
    use std::io::Write;
    let full = std::fs::metadata(path).is_ok_and(|m| m.len() > CRASH_LOG_MAX);
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .append(!full)
        .truncate(full)
        .open(path)?;
    writeln!(file, "{report}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crash_log_appends_and_starts_over_when_full() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join(CRASH_LOG);
        append_report(&path, "first").unwrap();
        append_report(&path, "second").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "first\nsecond\n");
        std::fs::write(&path, vec![b'x'; CRASH_LOG_MAX as usize + 1]).unwrap();
        append_report(&path, "third").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "third\n");
    }
}
