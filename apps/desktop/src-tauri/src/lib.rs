use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::Serialize;
use solaris_server::{EmbeddedServerHandle, ServerConfig, spawn_embedded_handle};
use tauri::{Manager, RunEvent, WebviewWindowBuilder};
use tracing_subscriber::EnvFilter;
use uuid::Uuid;

const KEYCHAIN_SERVICE: &str = "dev.solaris.studio";
const KEYCHAIN_OPENROUTER_USER: &str = "openrouter_api_key";

#[derive(Clone, Serialize)]
pub struct EngineConnectionInfo {
    pub url: String,
    pub token: String,
    pub desktop_keychain_available: bool,
}

struct DesktopServerState {
    workspace_dir: PathBuf,
    server: Mutex<Option<EmbeddedServerHandle>>,
    selected_export_dir: Mutex<Option<PathBuf>>,
}

fn load_keychain_openrouter_key() -> Option<String> {
    let entry = keyring::Entry::new(KEYCHAIN_SERVICE, KEYCHAIN_OPENROUTER_USER).ok()?;
    entry
        .get_password()
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

#[tauri::command]
fn get_engine_connection(state: tauri::State<'_, EngineConnectionInfo>) -> EngineConnectionInfo {
    state.inner().clone()
}

#[tauri::command]
fn save_openrouter_key(api_key: String) -> Result<(), String> {
    let trimmed = api_key.trim();
    if trimmed.is_empty() {
        return Err("OpenRouter API key cannot be empty".to_string());
    }
    let entry = keyring::Entry::new(KEYCHAIN_SERVICE, KEYCHAIN_OPENROUTER_USER)
        .map_err(|e| format!("failed to access OS keychain: {e}"))?;
    entry
        .set_password(trimmed)
        .map_err(|e| format!("failed to store key in OS keychain: {e}"))
}

#[tauri::command]
fn delete_openrouter_key() -> Result<(), String> {
    let entry = keyring::Entry::new(KEYCHAIN_SERVICE, KEYCHAIN_OPENROUTER_USER)
        .map_err(|e| format!("failed to access OS keychain: {e}"))?;
    match entry.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(format!("failed to delete key from OS keychain: {e}")),
    }
}

fn sanitize_download_filename(raw: &str) -> String {
    let base = Path::new(raw)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("solaris-export.mp4");
    let cleaned: String = base
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .take(128)
        .collect();
    let trimmed = cleaned.trim_start_matches('.');
    if trimmed.is_empty() {
        "solaris-export.mp4".to_string()
    } else {
        trimmed.to_string()
    }
}

#[tauri::command]
async fn pick_export_folder(
    app: tauri::AppHandle,
    state: tauri::State<'_, DesktopServerState>,
) -> Result<Option<String>, String> {
    let starting_dir = state
        .selected_export_dir
        .lock()
        .ok()
        .and_then(|g| g.clone())
        .filter(|p| p.is_dir())
        .or_else(|| app.path().download_dir().ok())
        .or_else(|| app.path().desktop_dir().ok());

    let mut dialog = rfd::AsyncFileDialog::new().set_title("Choose Solaris Export Folder");
    if let Some(dir) = starting_dir {
        dialog = dialog.set_directory(dir);
    }

    let Some(handle) = dialog.pick_folder().await else {
        return Ok(None);
    };
    let chosen = handle.path().to_path_buf();
    let canon = std::fs::canonicalize(&chosen)
        .map_err(|e| format!("selected export folder is not accessible: {e}"))?;
    if !canon.is_dir() {
        return Err("selected path is not a folder".to_string());
    }
    if let Ok(mut guard) = state.selected_export_dir.lock() {
        *guard = Some(canon.clone());
    }
    Ok(Some(canon.to_string_lossy().to_string()))
}

#[tauri::command]
async fn save_export_file(
    app: tauri::AppHandle,
    state: tauri::State<'_, DesktopServerState>,
    output_path: String,
    suggested_name: Option<String>,
    target_dir: Option<String>,
) -> Result<String, String> {
    let workspace_dir = state.workspace_dir.clone();
    let exports_root = workspace_dir.join("exports");
    let selected_dir = state
        .selected_export_dir
        .lock()
        .ok()
        .and_then(|g| g.clone());

    let target_dir = match target_dir
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(requested) => {
            let canon_req = std::fs::canonicalize(requested)
                .map_err(|e| format!("selected export folder unavailable: {e}"))?;
            let authorized = selected_dir
                .as_ref()
                .is_some_and(|allowed| allowed == &canon_req);
            if !authorized || !canon_req.is_dir() {
                return Err(
                    "requested export folder must be chosen via the folder picker".to_string(),
                );
            }
            canon_req
        }
        None => match selected_dir.filter(|p| p.is_dir()) {
            Some(dir) => dir,
            None => app
                .path()
                .download_dir()
                .or_else(|_| app.path().desktop_dir())
                .map_err(|e| format!("failed to resolve Downloads directory: {e}"))?,
        },
    };

    tauri::async_runtime::spawn_blocking(move || {
        use std::fs::OpenOptions;
        use std::io::ErrorKind;

        let canon_exports = std::fs::canonicalize(&exports_root)
            .map_err(|e| format!("exports directory unavailable: {e}"))?;
        let raw_src = PathBuf::from(&output_path);
        let src_candidate = if raw_src.is_absolute() {
            raw_src
        } else {
            let joined = workspace_dir.join(&raw_src);
            if joined.exists() { joined } else { raw_src }
        };
        let canon_src = std::fs::canonicalize(&src_candidate)
            .map_err(|e| format!("export output not found: {e}"))?;
        if !canon_src.starts_with(&canon_exports) || !canon_src.is_file() {
            return Err(
                "requested export path is outside the workspace exports directory".to_string(),
            );
        }

        std::fs::create_dir_all(&target_dir)
            .map_err(|e| format!("failed to prepare export folder: {e}"))?;

        let raw_name = suggested_name
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .or_else(|| canon_src.file_name().and_then(|n| n.to_str()))
            .unwrap_or("solaris-export.mp4");
        let safe_name = sanitize_download_filename(raw_name);

        let stem = Path::new(&safe_name)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("solaris-export");
        let ext = Path::new(&safe_name).extension().and_then(|e| e.to_str());

        let mut dest = target_dir.join(&safe_name);
        let mut dest_file = None;
        for counter in 0u32..1000 {
            let candidate = if counter == 0 {
                target_dir.join(&safe_name)
            } else {
                let name = match ext {
                    Some(e) => format!("{stem}-{counter}.{e}"),
                    None => format!("{stem}-{counter}"),
                };
                target_dir.join(name)
            };
            match OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&candidate)
            {
                Ok(f) => {
                    dest = candidate;
                    dest_file = Some(f);
                    break;
                }
                Err(e) if e.kind() == ErrorKind::AlreadyExists => continue,
                Err(e) => {
                    return Err(format!(
                        "failed to create export file '{}': {e}",
                        candidate.display()
                    ));
                }
            }
        }

        let mut out_file = match dest_file {
            Some(f) => f,
            None => {
                let short_id = &Uuid::new_v4().simple().to_string()[..8];
                let fallback_name = match ext {
                    Some(e) => format!("{stem}-{short_id}.{e}"),
                    None => format!("{stem}-{short_id}"),
                };
                dest = target_dir.join(fallback_name);
                OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&dest)
                    .map_err(|e| {
                        format!(
                            "failed to create unique export file '{}': {e}",
                            dest.display()
                        )
                    })?
            }
        };

        let mut src_file = std::fs::File::open(&canon_src)
            .map_err(|e| format!("failed to open export output: {e}"))?;
        if let Err(e) = std::io::copy(&mut src_file, &mut out_file) {
            drop(out_file);
            let _ = std::fs::remove_file(&dest);
            return Err(format!(
                "failed to save export to '{}': {e}",
                dest.display()
            ));
        }

        Ok(dest.to_string_lossy().to_string())
    })
    .await
    .map_err(|e| format!("export save task failed: {e}"))?
}

pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let session_token = Uuid::new_v4().to_string();

    let app = tauri::Builder::default()
        .setup(move |app| {
            let raw_data_dir = app
                .path()
                .app_data_dir()
                .unwrap_or_else(|_| PathBuf::from(".solaris-data"));
            let _ = std::fs::create_dir_all(&raw_data_dir);
            let data_dir = std::fs::canonicalize(&raw_data_dir).unwrap_or(raw_data_dir);

            let keychain_key = load_keychain_openrouter_key();
            let config = ServerConfig {
                host: "127.0.0.1".to_string(),
                port: 0,
                workspace_dir: data_dir.clone(),
                allow_external_media_reads: true,
                auth_token: Some(session_token.clone()),
                openrouter_api_key: keychain_key,
                ..ServerConfig::default()
            };

            let (addr, server_handle) =
                tauri::async_runtime::block_on(spawn_embedded_handle(config))
                    .map_err(|e| Box::new(e) as Box<dyn std::error::Error>)?;

            let info = EngineConnectionInfo {
                url: format!("http://{addr}"),
                token: session_token.clone(),
                desktop_keychain_available: true,
            };
            tracing::info!("embedded solaris-server listening on {}", info.url);
            app.manage(info);
            app.manage(DesktopServerState {
                workspace_dir: data_dir,
                server: Mutex::new(Some(server_handle)),
                selected_export_dir: Mutex::new(None),
            });

            // Create the main window AFTER embedded server + managed state are ready (eliminates startup race)
            if let Some(window_cfg) = app.config().app.windows.first().cloned() {
                WebviewWindowBuilder::from_config(app.handle(), &window_cfg)?.build()?;
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_engine_connection,
            save_openrouter_key,
            delete_openrouter_key,
            pick_export_folder,
            save_export_file
        ])
        .build(tauri::generate_context!())
        .expect("error while building Solaris desktop");

    app.run(|app_handle, event| {
        if let RunEvent::Exit = event
            && let Some(srv) = app_handle.try_state::<DesktopServerState>()
            && let Ok(mut guard) = srv.server.lock()
            && let Some(handle) = guard.take()
        {
            tauri::async_runtime::block_on(
                handle.shutdown_with_timeout(std::time::Duration::from_secs(3)),
            );
        }
    });
}
