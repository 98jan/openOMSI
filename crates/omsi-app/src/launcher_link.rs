//! The launcher is another program: the desktop launcher (`launcher/`, a Tauri window over
//! omsi-launcher-core, `openomsi-launcher-app`) beside the game, or any program
//! `OMSI_LAUNCHER` names. A plain start opens it; the game has no launcher window of its own.

use super::*;

/// The desktop launcher's program name.
const APP_LAUNCHER: &str = if cfg!(windows) { "openomsi-launcher-app.exe" } else { "openomsi-launcher-app" };

/// The desktop launcher: beside this program (in the same bundle on macOS; a development
/// build's folder when it was built into the same target folder, see docs/BUILDING.md).
pub(crate) fn find_app_launcher() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let bin = exe.parent()?.join(APP_LAUNCHER);
    bin.is_file().then_some(bin)
}

/// A plain start: the desktop launcher. Where it is missing, or ends at once with an error
/// (a system without its web view: no WebKitGTK library on Linux, no WebView2 runtime on an
/// old Windows), the player is told why and where it should be, and the program ends.
pub(crate) fn open_app_launcher() -> Result<()> {
    let exe = std::env::current_exe().unwrap_or_default();
    let dir = exe.parent().map(Path::to_path_buf).unwrap_or_default();
    let Some(bin) = find_app_launcher() else {
        fatal_message(&format!(
            "The openOMSI launcher ({APP_LAUNCHER}) was not found. It belongs in the same folder as the game ({}).\n\nPlease download openOMSI again and keep all of its files together.",
            dir.display()
        ));
        return Ok(());
    };
    log::info!("starting the desktop launcher {}", bin.display());
    match std::process::Command::new(&bin).env("OPENOMSI_BIN", &exe).spawn() {
        Ok(mut child) => {
            // (a program that cannot even load - Linux without WebKitGTK's library - ends at
            // once with an error)
            let t0 = std::time::Instant::now();
            while t0.elapsed() < std::time::Duration::from_secs(2) {
                match child.try_wait() {
                    Ok(Some(status)) if !status.success() => {
                        fatal_message(&format!("The openOMSI launcher ({}) stopped right after it started ({status}).\n\n{}", bin.display(), WEBVIEW_HINT));
                        return Ok(());
                    }
                    Ok(Some(_)) | Err(_) => break,
                    Ok(None) => std::thread::sleep(std::time::Duration::from_millis(50)),
                }
            }
            Ok(())
        }
        Err(e) => {
            fatal_message(&format!("The openOMSI launcher ({}) could not be started: {e}.\n\n{}", bin.display(), WEBVIEW_HINT));
            Ok(())
        }
    }
}

/// What the launcher needs of the system to show its window.
const WEBVIEW_HINT: &str = if cfg!(windows) {
    "The launcher's window needs Microsoft Edge WebView2 (part of Windows 10 and 11; for older systems: https://developer.microsoft.com/microsoft-edge/webview2/)."
} else if cfg!(target_os = "linux") {
    "The launcher's window needs WebKitGTK 4.1 (the package libwebkit2gtk-4.1-0 on Debian and Ubuntu, webkit2gtk4.1 on Fedora, webkit2gtk-4.1 on Arch)."
} else {
    "Its log is in the openOMSI data folder (launcher.log)."
};

/// Start the program `OMSI_LAUNCHER` names and return once it is running. `Ok(false)` when
/// it names none that is there.
pub(crate) fn open_launcher() -> Result<bool> {
    let Some(bin) = omsi_cfg::flags::OMSI_LAUNCHER.os().map(PathBuf::from).filter(|p| p.is_file()) else {
        return Ok(false);
    };
    log::info!("starting the launcher {}", bin.display());
    let exe = std::env::current_exe().unwrap_or_default();
    std::process::Command::new(&bin)
        .env("OPENOMSI_BIN", &exe)
        .spawn()
        .map_err(|e| anyhow!("starting {}: {e}", bin.display()))?;
    Ok(true)
}
