//! "Periksa pembaruan" (7.0.1): the one place the application goes online,
//! and only when the user presses the button in About.
//!
//! SPEC Section 2 said no update check at all; the user changed that on
//! 3 October 2026 to a check *on request* — nothing runs at startup or in the
//! background (SPEC 2, dated amendment). What is fetched is
//! `latest.json` from the newest GitHub Release, which names the installer and
//! its minisign signature; `tauri-plugin-updater` refuses an installer whose
//! signature does not match the public key in `tauri.conf.json`, so a
//! tampered download is never run. The private key lives only in the
//! repository's secrets, used by CI when a release is published.
//!
//! A portable copy is never updated in place: the installer would install a
//! second, regular copy next to it. It is told to download the new ZIP.

use std::time::Duration;

use parking_lot::Mutex;
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};
use tauri_plugin_updater::{Update, UpdaterExt};

type CmdResult<T> = Result<T, String>;

/// Where a release is listed, for the portable copy and for errors.
pub const RELEASES_URL: &str = "https://github.com/Zulaziz18/PDF-VIEWER-IZUL/releases";

/// The update found by the last check, held until the user says install.
#[derive(Default)]
pub struct Pending(Mutex<Option<Update>>);

impl std::fmt::Debug for Pending {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pending")
            .field("held", &self.0.lock().is_some())
            .finish()
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateCheck {
    pub current: String,
    /// The newer version, or `None` when this one is the newest.
    pub available: Option<String>,
    pub notes: Option<String>,
    /// A portable copy: updates are downloaded by hand.
    pub portable: bool,
    pub releases_url: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub struct Progress {
    pub done: u64,
    pub total: Option<u64>,
}

fn portable() -> bool {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(std::path::Path::to_path_buf));
    izul_store::is_portable(exe_dir.as_deref())
}

/// Asks GitHub whether a newer release exists. The only network request the
/// application makes, and only from this command.
#[tauri::command]
pub async fn update_check(app: AppHandle, pending: State<'_, Pending>) -> CmdResult<UpdateCheck> {
    let current = app.package_info().version.to_string();
    if portable() {
        return Ok(UpdateCheck {
            current,
            available: None,
            notes: None,
            portable: true,
            releases_url: RELEASES_URL,
        });
    }
    let updater = app
        .updater_builder()
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|e| format!("pemeriksa pembaruan tidak siap: {e}"))?;
    let found = updater
        .check()
        .await
        .map_err(|e| format!("tidak bisa menghubungi GitHub: {e}"))?;
    tracing::info!(current = %current, found = ?found.as_ref().map(|u| &u.version), "pemeriksaan pembaruan");
    let result = UpdateCheck {
        current,
        available: found.as_ref().map(|u| u.version.clone()),
        notes: found.as_ref().and_then(|u| u.body.clone()),
        portable: false,
        releases_url: RELEASES_URL,
    };
    *pending.0.lock() = found;
    Ok(result)
}

/// Downloads the update found by [`update_check`], checks its signature, and
/// runs the installer — which, on Windows, ends this process. Progress goes
/// out as `izul://update-progress`.
///
/// The frontend has already asked about unsaved documents before calling this.
#[tauri::command]
pub async fn update_install(app: AppHandle, pending: State<'_, Pending>) -> CmdResult<()> {
    let update = pending
        .0
        .lock()
        .take()
        .ok_or_else(|| "Periksa pembaruan dulu.".to_string())?;
    if !cfg!(windows) {
        return Err("Pemasangan pembaruan hanya didukung di Windows.".into());
    }
    tracing::info!(version = %update.version, "memasang pembaruan");
    let mut done = 0u64;
    let emitter = app.clone();
    update
        .download_and_install(
            move |chunk, total| {
                done += chunk as u64;
                let _ = emitter.emit("izul://update-progress", Progress { done, total });
            },
            || tracing::info!("pembaruan terunduh, tanda tangan cocok"),
        )
        .await
        .map_err(|e| format!("pembaruan gagal dipasang: {e}"))
}
