//! Installing an update in place of this copy: its file downloaded and its
//! signature checked, then as this copy was installed. The new version
//! starts once this one has quit.

use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
    process::Command,
};

use base64::{Engine, engine::general_purpose::STANDARD};
use ed25519_dalek::{Signature, VerifyingKey};
use iced::futures::{SinkExt, Stream, channel::mpsc::Sender};

use super::{File, Install, Message, Step, Update};

/// Download `update`'s file, check it against `public_key` and put it in
/// place of this copy; the new version starts once this process has ended.
/// How far it got as it goes, then whether it did.
pub fn install(update: Update, public_key: &'static str) -> impl Stream<Item = Message> {
    iced::stream::channel(8, async move |mut steps: Sender<Message>| {
        let result = put(update, public_key, &mut steps).await;
        let _ = steps.send(Message::Installed(result)).await;
    })
}

async fn put(
    update: Update,
    public_key: &'static str,
    steps: &mut Sender<Message>,
) -> Result<(), String> {
    let file = update
        .file
        .ok_or("The release has no file for this installation.")?;
    let data = download(&file.url, Some(steps)).await?;
    let signature = file
        .signature
        .as_deref()
        .ok_or("The release is not signed.")?;
    let signature = String::from_utf8(download(signature, None).await?)
        .map_err(|_| "The release's signature is not text.".to_owned())?;
    verify(&data, &signature, public_key)?;
    let _ = steps.send(Message::Step(Step::Installing)).await;
    tokio::task::spawn_blocking(move || put_in_place(&update.install, &file, &data))
        .await
        .map_err(|e| e.to_string())?
}

/// The file at `url`, telling `steps` how much has come, if given.
async fn download(url: &str, mut steps: Option<&mut Sender<Message>>) -> Result<Vec<u8>, String> {
    let failed = |e: reqwest::Error| format!("Could not download the update: {e}");
    let mut response = reqwest::get(url)
        .await
        .and_then(|response| response.error_for_status())
        .map_err(failed)?;
    let total = response.content_length();
    let mut data = Vec::with_capacity(total.unwrap_or_default() as usize);
    while let Some(chunk) = response.chunk().await.map_err(failed)? {
        data.extend_from_slice(&chunk);
        if let Some(steps) = steps.as_mut() {
            let done = data.len() as u64;
            let _ = steps
                .send(Message::Step(Step::Downloading { done, total }))
                .await;
        }
    }
    Ok(data)
}

/// Whether `signature` (base64, from `cargo xtask sign`) is `data`'s, made
/// with the private key of `public_key` (base64).
fn verify(data: &[u8], signature: &str, public_key: &str) -> Result<(), String> {
    let decode = |text: &str| STANDARD.decode(text.trim()).ok();
    let key = decode(public_key)
        .and_then(|bytes| <[u8; 32]>::try_from(bytes).ok())
        .and_then(|bytes| VerifyingKey::from_bytes(&bytes).ok())
        .ok_or("The app's update key is not valid.")?;
    let signature = decode(signature)
        .and_then(|bytes| <[u8; 64]>::try_from(bytes).ok())
        .ok_or("The release's signature is not valid.")?;
    key.verify_strict(data, &Signature::from_bytes(&signature))
        .map_err(|_| "The update's signature does not match: not installed.".to_owned())
}

fn put_in_place(install: &Install, file: &File, data: &[u8]) -> Result<(), String> {
    match install {
        Install::MacApp(app) => replace_app(app, file, data),
        Install::WindowsSetup => run_setup(file, data),
        Install::WindowsPortable(exe) => replace_exe(exe, file, data),
        Install::AppImage(appimage) => replace_appimage(appimage, data),
        Install::Deb | Install::Source => {
            Err("This installation does not update itself.".to_owned())
        }
    }
}

/// Run the Windows installer silently; it waits for this copy to quit,
/// installs over it and starts the new version (`/update=yes`).
fn run_setup(file: &File, data: &[u8]) -> Result<(), String> {
    let setup = std::env::temp_dir().join(&file.name);
    std::fs::write(&setup, data).map_err(|e| e.to_string())?;
    Command::new(&setup)
        .args([
            "/VERYSILENT",
            "/SUPPRESSMSGBOXES",
            "/NORESTART",
            "/update=yes",
        ])
        .spawn()
        .map(drop)
        .map_err(|e| format!("Could not start the installer: {e}"))
}

/// Unpack `Mtrx.exe` from the Windows zip and put it in place of `exe`,
/// which Windows lets us rename while it runs; the old one is removed at
/// the next start ([`clean_up`]).
fn replace_exe(exe: &Path, file: &File, data: &[u8]) -> Result<(), String> {
    let staging = std::env::temp_dir().join("mtrx-update");
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging).map_err(|e| e.to_string())?;
    let zip = staging.join(&file.name);
    std::fs::write(&zip, data).map_err(|e| e.to_string())?;
    let unpacked = staging.join("new");
    let status = Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command"])
        .arg("Expand-Archive -LiteralPath $env:ZIP -DestinationPath $env:OUT")
        .env("ZIP", &zip)
        .env("OUT", &unpacked)
        .status()
        .map_err(|e| e.to_string())?;
    if !status.success() {
        return Err("Could not unpack the update.".to_owned());
    }
    // The zip holds the folder Mtrx with Mtrx.exe.
    let new = unpacked.join("Mtrx").join("Mtrx.exe");
    let old = old_exe(exe);
    let _ = std::fs::remove_file(&old);
    std::fs::rename(exe, &old).map_err(|e| format!("Could not replace Mtrx: {e}"))?;
    std::fs::copy(&new, exe).map_err(|e| {
        let _ = std::fs::rename(&old, exe);
        format!("Could not replace Mtrx: {e}")
    })?;
    let _ = std::fs::remove_dir_all(&staging);
    start_after_exit(exe, &[])
}

/// Where the replaced `exe` waits to be removed.
fn old_exe(exe: &Path) -> PathBuf {
    exe.with_extension("exe.old")
}

/// Remove what an update left: the replaced `Mtrx.exe` of the Windows zip.
pub fn clean_up(install: &Install) {
    if let Install::WindowsPortable(exe) = install {
        let _ = std::fs::remove_file(old_exe(exe));
    }
}

/// Write the new AppImage next to `appimage` and rename it over it; this
/// copy keeps running from the old one until it quits.
fn replace_appimage(appimage: &Path, data: &[u8]) -> Result<(), String> {
    swap_appimage(appimage, data)?;
    start_after_exit(appimage, &[])
}

fn swap_appimage(appimage: &Path, data: &[u8]) -> Result<(), String> {
    let new = appimage.with_extension("AppImage.new");
    std::fs::write(&new, data)
        .and_then(|()| make_executable(&new))
        .and_then(|()| std::fs::rename(&new, appimage))
        .map_err(|e| {
            let _ = std::fs::remove_file(&new);
            format!("Could not replace Mtrx: {e}")
        })
}

#[cfg(unix)]
fn make_executable(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

/// Unpack the zipped `Mtrx.app` next to `app`, swap it in, and start it
/// once this process has ended. Without permission to write there, the zip
/// is saved in Downloads and shown in Finder instead.
fn replace_app(app: &Path, file: &File, data: &[u8]) -> Result<(), String> {
    let dir = app.parent().ok_or("The app is not in a folder.")?;
    // On the app's volume, so that the swap is two renames.
    let staging = dir.join(".mtrx-update");
    let staged = std::fs::remove_dir_all(&staging)
        .or_else(|e| match e.kind() {
            std::io::ErrorKind::NotFound => Ok(()),
            _ => Err(e),
        })
        .and_then(|()| std::fs::create_dir(&staging));
    if staged.is_err() {
        return reveal_in_downloads(file, data);
    }
    let result = unpack_and_swap(app, &staging, file, data);
    let _ = std::fs::remove_dir_all(&staging);
    result?;
    start_after_exit(Path::new("open"), &[app.as_os_str()])
}

fn unpack_and_swap(app: &Path, staging: &Path, file: &File, data: &[u8]) -> Result<(), String> {
    let zip = staging.join(&file.name);
    std::fs::write(&zip, data).map_err(|e| e.to_string())?;
    let unpacked = staging.join("new");
    // ditto keeps the bundle's links, permissions and signature.
    let status = Command::new("ditto")
        .arg("-x")
        .arg("-k")
        .arg(&zip)
        .arg(&unpacked)
        .status()
        .map_err(|e| e.to_string())?;
    if !status.success() {
        return Err("Could not unpack the update.".to_owned());
    }
    swap(app, &unpacked.join("Mtrx.app"), &staging.join("old.app"))
}

/// Put `new` in place of `app`, moving `app` to `old`; if that fails, `app`
/// stays as it was.
fn swap(app: &Path, new: &Path, old: &Path) -> Result<(), String> {
    if !new.join("Contents/MacOS").is_dir() {
        return Err("The update has no Mtrx.app.".to_owned());
    }
    std::fs::rename(app, old).map_err(|e| format!("Could not replace Mtrx: {e}"))?;
    std::fs::rename(new, app).map_err(|e| {
        let _ = std::fs::rename(old, app);
        format!("Could not replace Mtrx: {e}")
    })
}

/// The zip in the user's Downloads, shown in Finder, to install by hand.
fn reveal_in_downloads(file: &File, data: &[u8]) -> Result<(), String> {
    let downloads = std::env::var_os("HOME")
        .map(|home| PathBuf::from(home).join("Downloads"))
        .ok_or("No Downloads folder.")?;
    let zip = downloads.join(&file.name);
    std::fs::write(&zip, data).map_err(|e| e.to_string())?;
    let _ = crate::files::show_in_folder(&zip);
    Err(format!(
        "Mtrx cannot replace itself where it is; the update is in Downloads: {}",
        file.name
    ))
}

/// Run `program` with `args` once this process has ended (it holds the
/// account's stores), to start the new version.
#[cfg(unix)]
fn start_after_exit(program: &Path, args: &[&OsStr]) -> Result<(), String> {
    Command::new("/bin/sh")
        .arg("-c")
        .arg(r#"pid=$1; shift; while kill -0 "$pid" 2>/dev/null; do sleep 0.2; done; exec "$@""#)
        .arg("sh")
        .arg(std::process::id().to_string())
        .arg(program)
        .args(args)
        .spawn()
        .map(drop)
        .map_err(|e| format!("Could not start the new version: {e}"))
}

#[cfg(windows)]
fn start_after_exit(program: &Path, args: &[&OsStr]) -> Result<(), String> {
    debug_assert!(args.is_empty(), "the program alone on Windows");
    Command::new("powershell")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-WindowStyle",
            "Hidden",
            "-Command",
        ])
        .arg(
            "Wait-Process -Id $env:MTRX_PID -ErrorAction SilentlyContinue; \
             Start-Process -FilePath $env:MTRX_EXE",
        )
        .env("MTRX_PID", std::process::id().to_string())
        .env("MTRX_EXE", program)
        .spawn()
        .map(drop)
        .map_err(|e| format!("Could not start the new version: {e}"))
}

#[cfg(test)]
mod tests {
    use ed25519_dalek::{Signer, SigningKey};

    use super::*;

    #[test]
    fn only_a_matching_signature_verifies() {
        let key = SigningKey::from_bytes(&[7; 32]);
        let public = STANDARD.encode(key.verifying_key().to_bytes());
        let signature = STANDARD.encode(key.sign(b"Mtrx").to_bytes());
        assert_eq!(verify(b"Mtrx", &signature, &public), Ok(()));
        assert!(verify(b"Mtrx!", &signature, &public).is_err());
        assert!(verify(b"Mtrx", "not base64", &public).is_err());
        let other = STANDARD.encode(SigningKey::from_bytes(&[8; 32]).verifying_key().to_bytes());
        assert!(verify(b"Mtrx", &signature, &other).is_err());
    }

    #[test]
    fn the_new_app_takes_the_old_ones_place() {
        let dir = tempfile::TempDir::new().unwrap();
        let app = dir.path().join("Mtrx.app");
        let new = dir.path().join("new/Mtrx.app");
        let old = dir.path().join("old.app");
        for (bundle, version) in [(&app, "old"), (&new, "new")] {
            std::fs::create_dir_all(bundle.join("Contents/MacOS")).unwrap();
            std::fs::write(bundle.join("Contents/version"), version).unwrap();
        }
        swap(&app, &new, &old).unwrap();
        assert_eq!(
            std::fs::read_to_string(app.join("Contents/version")).unwrap(),
            "new"
        );
        assert_eq!(
            std::fs::read_to_string(old.join("Contents/version")).unwrap(),
            "old"
        );

        // Not a bundle: nothing moves.
        let empty = dir.path().join("empty");
        std::fs::create_dir(&empty).unwrap();
        assert!(swap(&app, &empty, &dir.path().join("older.app")).is_err());
        assert_eq!(
            std::fs::read_to_string(app.join("Contents/version")).unwrap(),
            "new"
        );
    }

    /// The release's zip, as `ditto` makes it in CI, unpacked and swapped in.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_zipped_app_replaces_this_one() {
        let dir = tempfile::TempDir::new().unwrap();
        let build = dir.path().join("build/Mtrx.app");
        std::fs::create_dir_all(build.join("Contents/MacOS")).unwrap();
        std::fs::write(build.join("Contents/version"), "new").unwrap();
        let zip = dir.path().join("Mtrx-macos-arm64.zip");
        let zipped = Command::new("ditto")
            .args(["-c", "-k", "--sequesterRsrc", "--keepParent"])
            .arg(&build)
            .arg(&zip)
            .status()
            .unwrap();
        assert!(zipped.success());

        let app = dir.path().join("Applications/Mtrx.app");
        std::fs::create_dir_all(app.join("Contents/MacOS")).unwrap();
        let staging = dir.path().join("Applications/.mtrx-update");
        std::fs::create_dir(&staging).unwrap();
        let file = File {
            name: "Mtrx-macos-arm64.zip".into(),
            url: String::new(),
            signature: None,
        };
        unpack_and_swap(&app, &staging, &file, &std::fs::read(&zip).unwrap()).unwrap();
        let version = std::fs::read_to_string(app.join("Contents/version")).unwrap();
        assert_eq!(version, "new");
    }

    #[cfg(unix)]
    #[test]
    fn the_new_appimage_takes_the_old_ones_place() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::TempDir::new().unwrap();
        let appimage = dir.path().join("Mtrx-linux-x86_64.AppImage");
        std::fs::write(&appimage, "old").unwrap();
        swap_appimage(&appimage, b"new").unwrap();
        assert_eq!(std::fs::read(&appimage).unwrap(), b"new");
        let mode = std::fs::metadata(&appimage).unwrap().permissions().mode();
        assert_eq!(mode & 0o111, 0o111, "executable");
        assert!(!appimage.with_extension("AppImage.new").exists());
    }
}
