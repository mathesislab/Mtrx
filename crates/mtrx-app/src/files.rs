//! Files on this computer: showing one in its folder.

use std::{path::Path, process::Command};

/// Show the file at `path` selected in its folder: in Finder, Explorer, or
/// the Linux desktop's file manager (else its folder opens). Blocks until
/// the file manager is asked.
pub fn show_in_folder(path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Err("Moved or deleted".into());
    }
    show(path).map_err(|e| format!("Could not show it: {e}"))
}

#[cfg(target_os = "macos")]
fn show(path: &Path) -> std::io::Result<()> {
    Command::new("open").arg("-R").arg(path).status().map(drop)
}

#[cfg(windows)]
fn show(path: &Path) -> std::io::Result<()> {
    use std::os::windows::process::CommandExt;
    // One argument, quoted as Explorer wants it. Its exit code means nothing.
    let mut select = std::ffi::OsString::from("/select,\"");
    select.push(path);
    select.push("\"");
    Command::new("explorer").raw_arg(select).spawn().map(drop)
}

#[cfg(all(unix, not(target_os = "macos")))]
fn show(path: &Path) -> std::io::Result<()> {
    let uri = mtrx_core::Url::from_file_path(path)
        .map_err(|()| std::io::Error::other("not an absolute path"))?;
    let shown = Command::new("gdbus")
        .args([
            "call",
            "--session",
            "--dest",
            "org.freedesktop.FileManager1",
            "--object-path",
            "/org/freedesktop/FileManager1",
            "--method",
            "org.freedesktop.FileManager1.ShowItems",
            &format!("['{uri}']"),
            "",
        ])
        .output()
        .is_ok_and(|output| output.status.success());
    if shown {
        Ok(())
    } else {
        open::that_detached(path.parent().unwrap_or(path))
    }
}
