use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use axum::http::StatusCode;
use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD as BASE64_STANDARD, URL_SAFE_NO_PAD};
use chrono::{DateTime, Duration, Utc};
use ring::digest;
use ring::rand::{SecureRandom, SystemRandom};
use solaris_agent_runtime::{ModelAdvisor, OpenRouterClient, OpenRouterConfig, lint_gfx_html};
use solaris_mcp_server::McpSessionStore;
use tokio::sync::{Mutex, RwLock, Semaphore, broadcast};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use solaris_core_ir::{
    BrandPlaybook, BrandReferenceKind, BrandTokens, ClipKind, EngineEvent, EngineEventEnvelope,
    ExportJobStatus, LogEntry, LogLevel, Project, ProjectEngine, SavedCodeAsset, SwarmRunStatus,
    TimelineCommand, load_project, save_project_atomic,
};
use solaris_media_engine::{
    AssetStorage, BrandFontData, LocalFsStorage, MediaError, MediaToolchain, S3Storage,
    S3StorageConfig, ToolchainCapabilities,
};

const MAX_EPHEMERAL_TOKENS: usize = 256;
const MAX_GFX_SANDBOXES: usize = 128;
const MAX_LOG_ENTRIES: usize = 500;
const MAX_LOG_DETAILS_CHARS: usize = 12_000;
pub const MAX_GFX_SANDBOX_BYTES: usize = 512 * 1024;
const DEFAULT_MAX_CONCURRENT_EXPORTS: usize = 2;
const AGENT_CREDENTIALS_FILE: &str = "agent_credentials.json";
const DISMISSED_STARTERS_FILE: &str = ".dismissed_starters.json";
const MAX_BATCH_COMMANDS: usize = 256;

fn load_dismissed_starters(assets_dir: &Path) -> (HashSet<String>, Option<String>) {
    let path = assets_dir.join(DISMISSED_STARTERS_FILE);
    let raw = match fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(_) => return (HashSet::new(), None),
    };
    match serde_json::from_str::<Vec<String>>(&raw) {
        Ok(ids) => (ids.into_iter().collect(), None),
        Err(err) => {
            let unix = Utc::now().timestamp();
            let backup_name = format!(".dismissed_starters.corrupt-{unix}.json");
            let backup_path = assets_dir.join(&backup_name);
            let moved = fs::rename(&path, &backup_path).is_ok();
            let warning = if moved {
                format!(
                    "Corrupt {DISMISSED_STARTERS_FILE} moved to {backup_name}; continuing with empty dismissed-starters list: {err}"
                )
            } else {
                format!(
                    "Corrupt {DISMISSED_STARTERS_FILE} could not be renamed to {backup_name}; continuing with empty dismissed-starters list: {err}"
                )
            };
            (HashSet::new(), Some(warning))
        }
    }
}

#[derive(Default, serde::Serialize, serde::Deserialize)]
struct PersistedAgentCredentials {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    openrouter_api_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    default_model: Option<String>,
}

#[derive(Clone)]
pub struct MediaTicketEntry {
    pub expires_at: DateTime<Utc>,
    pub session_id: Option<String>,
}

#[derive(Clone)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub workspace_dir: PathBuf,
    pub allow_external_media_reads: bool,
    pub auth_token: Option<String>,
    pub openrouter_api_key: Option<String>,
    pub s3_config: Option<S3StorageConfig>,
    pub static_dir: Option<PathBuf>,
    pub cors_origins: Vec<String>,
}

impl std::fmt::Debug for ServerConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServerConfig")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("workspace_dir", &self.workspace_dir)
            .field(
                "allow_external_media_reads",
                &self.allow_external_media_reads,
            )
            .field(
                "auth_token",
                &self.auth_token.as_ref().map(|_| "<redacted>"),
            )
            .field(
                "openrouter_api_key",
                &self.openrouter_api_key.as_ref().map(|_| "<redacted>"),
            )
            .field(
                "s3_config",
                &self.s3_config.as_ref().map(|_| "<configured>"),
            )
            .field("static_dir", &self.static_dir)
            .field("cors_origins", &self.cors_origins)
            .finish()
    }
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".to_string(),
            port: 41731,
            workspace_dir: PathBuf::from(".solaris-data"),
            allow_external_media_reads: true,
            auth_token: None,
            openrouter_api_key: None,
            s3_config: None,
            static_dir: None,
            cors_origins: Vec::new(),
        }
    }
}

#[derive(Clone)]
pub struct AppState {
    pub inner: Arc<AppStateInner>,
}

pub struct AppStateInner {
    pub workspace_dir: PathBuf,
    pub projects: RwLock<HashMap<String, ProjectEngine>>,
    pub brands: RwLock<HashMap<String, BrandPlaybook>>,
    pub assets: RwLock<HashMap<String, SavedCodeAsset>>,
    /// Built-in starter GFX IDs the user deleted; persisted so restarts never resurrect them.
    pub dismissed_starters: RwLock<HashSet<String>>,
    pub export_jobs: RwLock<HashMap<String, ExportJobStatus>>,
    pub swarm_runs: RwLock<HashMap<String, SwarmRunStatus>>,
    pub swarm_cancels: RwLock<HashMap<String, CancellationToken>>,
    pub media_tickets: RwLock<HashMap<String, MediaTicketEntry>>,
    pub sessions: RwLock<HashMap<String, DateTime<Utc>>>,
    pub gfx_sandboxes: RwLock<VecDeque<(String, String)>>,
    pub logs: std::sync::RwLock<VecDeque<LogEntry>>,
    pub persist_revisions: Mutex<HashMap<String, u64>>,
    pub brand_persist_lock: Mutex<()>,
    pub asset_persist_lock: Mutex<()>,
    pub credentials_persist_lock: Mutex<()>,
    pub export_semaphore: Arc<Semaphore>,
    pub shutdown: CancellationToken,
    pub storage: Arc<dyn AssetStorage>,
    pub toolchain: MediaToolchain,
    pub capabilities: ToolchainCapabilities,
    pub openrouter: OpenRouterClient,
    pub model_advisor: Arc<ModelAdvisor>,
    pub mcp_sessions: Arc<McpSessionStore>,
    pub events_tx: broadcast::Sender<EngineEventEnvelope>,
    pub auth_token: Option<String>,
    pub is_loopback: bool,
    pub cors_origins: Vec<String>,
    pub static_dir: Option<PathBuf>,
}

impl AppState {
    pub fn initialize(config: &ServerConfig) -> std::io::Result<Self> {
        fs::create_dir_all(&config.workspace_dir)?;
        let workspace_dir = fs::canonicalize(&config.workspace_dir)
            .unwrap_or_else(|_| config.workspace_dir.clone());
        let projects_dir = workspace_dir.join("projects");
        let brands_dir = workspace_dir.join("brands");
        let assets_dir = workspace_dir.join("assets");
        let uploads_dir = workspace_dir.join("uploads");
        let brands_media_dir = workspace_dir.join("brands_media");
        let exports_dir = workspace_dir.join("exports");
        fs::create_dir_all(&projects_dir)?;
        fs::create_dir_all(&brands_dir)?;
        fs::create_dir_all(&assets_dir)?;
        fs::create_dir_all(&uploads_dir)?;
        fs::create_dir_all(&brands_media_dir)?;
        fs::create_dir_all(&exports_dir)?;

        let mut projects = HashMap::new();
        if let Ok(entries) = fs::read_dir(&projects_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("json") {
                    match load_project(&path) {
                        Ok(project) => {
                            projects.insert(project.id.clone(), ProjectEngine::new(project));
                        }
                        Err(err) => {
                            tracing::warn!(
                                path = %path.display(),
                                "skipping corrupted project JSON on startup: {err}"
                            );
                        }
                    }
                }
            }
        }

        let mut brands = HashMap::new();
        if let Ok(entries) = fs::read_dir(&brands_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("json") {
                    match fs::read_to_string(&path)
                        .map_err(|e| e.to_string())
                        .and_then(|raw| {
                            serde_json::from_str::<BrandPlaybook>(&raw).map_err(|e| e.to_string())
                        }) {
                        Ok(brand) => {
                            brands.insert(brand.id.clone(), brand);
                        }
                        Err(err) => {
                            tracing::warn!(
                                path = %path.display(),
                                "skipping corrupted brand JSON on startup: {err}"
                            );
                        }
                    }
                }
            }
        }
        if !brands.contains_key("solaris-default") {
            let default_brand = BrandPlaybook::solaris_default();
            let _ = save_json_atomic(
                &default_brand,
                &brands_dir.join(format!("{}.json", default_brand.id)),
            );
            brands.insert(default_brand.id.clone(), default_brand);
        }

        let mut assets = HashMap::new();
        if let Ok(entries) = fs::read_dir(&assets_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                let is_hidden = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with('.'));
                if !is_hidden && path.extension().and_then(|e| e.to_str()) == Some("json") {
                    match fs::read_to_string(&path)
                        .map_err(|e| e.to_string())
                        .and_then(|raw| {
                            serde_json::from_str::<SavedCodeAsset>(&raw).map_err(|e| e.to_string())
                        }) {
                        Ok(asset) => {
                            assets.insert(asset.id.clone(), asset);
                        }
                        Err(err) => {
                            tracing::warn!(
                                path = %path.display(),
                                "skipping corrupted asset JSON on startup: {err}"
                            );
                        }
                    }
                }
            }
        }
        let (dismissed_starters, dismissed_starters_warning) = load_dismissed_starters(&assets_dir);
        for starter in starter_code_assets() {
            if !assets.contains_key(&starter.id) && !dismissed_starters.contains(&starter.id) {
                let _ =
                    save_json_atomic(&starter, &assets_dir.join(format!("{}.json", starter.id)));
                assets.insert(starter.id.clone(), starter);
            }
        }

        let toolchain = MediaToolchain::from_env();
        let capabilities = toolchain.detect_capabilities();
        let (events_tx, _) = broadcast::channel(2048);
        let storage: Arc<dyn AssetStorage> = match config.s3_config.clone() {
            Some(s3_cfg) => Arc::new(S3Storage::new(
                s3_cfg,
                &workspace_dir,
                config.allow_external_media_reads,
            )),
            None => Arc::new(LocalFsStorage::new(
                &workspace_dir,
                config.allow_external_media_reads,
            )),
        };
        let mut openrouter_cfg = OpenRouterConfig::from_env();
        let persisted_creds = load_persisted_credentials(&workspace_dir);
        if openrouter_cfg.api_key.is_none()
            && let Some(key) = persisted_creds
                .openrouter_api_key
                .as_deref()
                .map(str::trim)
                .filter(|k| !k.is_empty())
        {
            openrouter_cfg.api_key = Some(key.to_string());
        }
        if openrouter_cfg.default_model.is_none()
            && let Some(model) = persisted_creds
                .default_model
                .as_deref()
                .map(str::trim)
                .filter(|m| !m.is_empty())
        {
            openrouter_cfg.default_model = Some(model.to_string());
        }
        if let Some(key) = config
            .openrouter_api_key
            .as_deref()
            .map(str::trim)
            .filter(|k| !k.is_empty())
        {
            openrouter_cfg.api_key = Some(key.to_string());
        }
        let openrouter = OpenRouterClient::new(openrouter_cfg);
        let model_advisor = Arc::new(ModelAdvisor::new(openrouter.clone()));

        let max_concurrent_exports = std::env::var("SOLARIS_MAX_CONCURRENT_EXPORTS")
            .ok()
            .and_then(|v| v.trim().parse::<usize>().ok())
            .filter(|&n| n > 0)
            .map(|n| n.min(16))
            .unwrap_or(DEFAULT_MAX_CONCURRENT_EXPORTS);

        let state = Self {
            inner: Arc::new(AppStateInner {
                workspace_dir,
                projects: RwLock::new(projects),
                brands: RwLock::new(brands),
                assets: RwLock::new(assets),
                dismissed_starters: RwLock::new(dismissed_starters),
                export_jobs: RwLock::new(HashMap::new()),
                swarm_runs: RwLock::new(HashMap::new()),
                swarm_cancels: RwLock::new(HashMap::new()),
                media_tickets: RwLock::new(HashMap::new()),
                sessions: RwLock::new(HashMap::new()),
                gfx_sandboxes: RwLock::new(VecDeque::new()),
                logs: std::sync::RwLock::new(VecDeque::with_capacity(MAX_LOG_ENTRIES)),
                persist_revisions: Mutex::new(HashMap::new()),
                brand_persist_lock: Mutex::new(()),
                asset_persist_lock: Mutex::new(()),
                credentials_persist_lock: Mutex::new(()),
                export_semaphore: Arc::new(Semaphore::new(max_concurrent_exports)),
                shutdown: CancellationToken::new(),
                storage,
                toolchain,
                capabilities,
                openrouter,
                model_advisor,
                mcp_sessions: Arc::new(McpSessionStore::new()),
                events_tx,
                auth_token: config.auth_token.clone(),
                is_loopback: is_loopback_host(&config.host),
                cors_origins: config.cors_origins.clone(),
                static_dir: config
                    .static_dir
                    .as_ref()
                    .and_then(|p| fs::canonicalize(p).ok()),
            }),
        };
        if let Some(warning) = dismissed_starters_warning {
            state.push_log(LogEntry {
                id: Uuid::new_v4().to_string(),
                timestamp: Utc::now(),
                level: LogLevel::Warn,
                source: "assets".to_string(),
                project_id: None,
                run_id: None,
                agent_id: None,
                message: warning,
                details: None,
            });
        }
        Ok(state)
    }

    pub async fn resolve_readable_path_async(
        &self,
        raw_path: String,
    ) -> Result<PathBuf, MediaError> {
        let storage = self.inner.storage.clone();
        tokio::task::spawn_blocking(move || storage.resolve_readable_path(&raw_path))
            .await
            .map_err(|e| MediaError::StorageError(format!("storage task error: {e}")))?
    }

    pub fn project_file_path(&self, project_id: &str) -> PathBuf {
        self.inner
            .workspace_dir
            .join("projects")
            .join(format!("{project_id}.json"))
    }

    pub async fn persist_project_locked(&self, project: &Project) {
        let mut revs = self.inner.persist_revisions.lock().await;
        if let Some(&last_rev) = revs.get(&project.id)
            && project.revision <= last_rev
        {
            return;
        }
        let path = self.project_file_path(&project.id);
        let project_clone = project.clone();
        let project_id = project.id.clone();
        let revision = project.revision;
        let res =
            tokio::task::spawn_blocking(move || save_project_atomic(&project_clone, &path)).await;
        match res {
            Ok(Ok(())) => {
                revs.insert(project_id, revision);
            }
            Ok(Err(err)) => {
                tracing::error!(project_id = %project_id, "failed to persist project: {err}");
            }
            Err(err) => {
                tracing::error!(project_id = %project_id, "project persist task failed: {err}");
            }
        }
    }

    pub async fn force_persist_project(&self, project: &Project) {
        let mut revs = self.inner.persist_revisions.lock().await;
        let path = self.project_file_path(&project.id);
        let project_clone = project.clone();
        let project_id = project.id.clone();
        let revision = project.revision;
        let res =
            tokio::task::spawn_blocking(move || save_project_atomic(&project_clone, &path)).await;
        match res {
            Ok(Ok(())) => {
                revs.insert(project_id, revision);
            }
            Ok(Err(err)) => {
                revs.remove(&project_id);
                tracing::error!(project_id = %project_id, "failed to force-persist project: {err}");
            }
            Err(err) => {
                revs.remove(&project_id);
                tracing::error!(project_id = %project_id, "project force-persist task failed: {err}");
            }
        }
    }

    pub fn emit(&self, event: EngineEvent) {
        let _ = self.inner.events_tx.send(EngineEventEnvelope::now(event));
    }

    pub fn emit_assets_changed(&self, action: &str, asset_id: Option<&str>) {
        self.emit(EngineEvent::AssetsChanged {
            action: action.to_string(),
            asset_id: asset_id.map(str::to_string),
        });
    }

    fn assets_dir(&self) -> PathBuf {
        self.inner.workspace_dir.join("assets")
    }

    async fn persist_dismissed_starters(&self) -> Result<(), String> {
        let mut ids: Vec<String> = self
            .inner
            .dismissed_starters
            .read()
            .await
            .iter()
            .cloned()
            .collect();
        ids.sort();
        save_json_atomic_async(ids, self.assets_dir().join(DISMISSED_STARTERS_FILE)).await
    }

    /// Must be called while holding `asset_persist_lock`. Re-saving a dismissed
    /// starter (e.g. via import or the AI) un-dismisses it.
    pub async fn undismiss_starter_locked(&self, asset_id: &str) -> Result<(), String> {
        let removed = self.inner.dismissed_starters.write().await.remove(asset_id);
        if removed {
            self.persist_dismissed_starters().await?;
        }
        Ok(())
    }

    /// Deletes a saved GFX asset from disk + memory. Existing timeline clips keep
    /// their `inline_html` copy so playback/export never breaks.
    pub async fn delete_asset(&self, asset_id: &str) -> Result<(), (StatusCode, String)> {
        validate_safe_id(asset_id)?;
        let _persist = self.inner.asset_persist_lock.lock().await;
        if !self.inner.assets.read().await.contains_key(asset_id) {
            return Err((
                StatusCode::NOT_FOUND,
                format!("GFX asset '{asset_id}' not found"),
            ));
        }
        let is_starter = starter_code_assets().iter().any(|s| s.id == asset_id);
        let newly_dismissed = if is_starter {
            let mut candidate: Vec<String> = self
                .inner
                .dismissed_starters
                .read()
                .await
                .iter()
                .cloned()
                .collect();
            let inserted = !candidate.iter().any(|id| id == asset_id);
            if inserted {
                candidate.push(asset_id.to_string());
            }
            candidate.sort();
            save_json_atomic_async(candidate, self.assets_dir().join(DISMISSED_STARTERS_FILE))
                .await
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
            if inserted {
                self.inner
                    .dismissed_starters
                    .write()
                    .await
                    .insert(asset_id.to_string());
            }
            inserted
        } else {
            false
        };

        let path = self.assets_dir().join(format!("{asset_id}.json"));
        let remove_res = tokio::task::spawn_blocking(move || match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.to_string()),
        })
        .await
        .map_err(|e| e.to_string())
        .and_then(|inner| inner);
        if let Err(err) = remove_res {
            if newly_dismissed {
                self.inner.dismissed_starters.write().await.remove(asset_id);
                let _ = self.persist_dismissed_starters().await;
            }
            return Err((StatusCode::INTERNAL_SERVER_ERROR, err));
        }

        self.inner.assets.write().await.remove(asset_id);
        drop(_persist);
        self.emit_assets_changed("deleted", Some(asset_id));
        Ok(())
    }

    /// Clears dismissed starters and re-seeds every missing built-in GFX.
    pub async fn restore_starter_assets(
        &self,
    ) -> Result<(usize, Vec<SavedCodeAsset>), (StatusCode, String)> {
        let _persist = self.inner.asset_persist_lock.lock().await;
        let missing: Vec<SavedCodeAsset> = {
            let guard = self.inner.assets.read().await;
            starter_code_assets()
                .into_iter()
                .filter(|s| !guard.contains_key(&s.id))
                .collect()
        };
        for starter in &missing {
            save_json_atomic_async(
                starter.clone(),
                self.assets_dir().join(format!("{}.json", starter.id)),
            )
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
        }
        self.inner.dismissed_starters.write().await.clear();
        self.persist_dismissed_starters()
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
        let list = {
            let mut guard = self.inner.assets.write().await;
            for starter in &missing {
                guard.insert(starter.id.clone(), starter.clone());
            }
            let mut list: Vec<SavedCodeAsset> = guard.values().cloned().collect();
            list.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.id.cmp(&b.id)));
            list
        };
        drop(_persist);
        self.emit_assets_changed("restored", None);
        Ok((missing.len(), list))
    }

    pub fn push_log(&self, mut entry: LogEntry) {
        if entry.id.trim().is_empty() {
            entry.id = Uuid::new_v4().to_string();
        }
        entry.message = sanitize_log_text(&entry.message, 2000);
        entry.details = entry
            .details
            .take()
            .map(|d| sanitize_log_text(&d, MAX_LOG_DETAILS_CHARS))
            .filter(|d| !d.trim().is_empty());

        match entry.level {
            LogLevel::Error => {
                tracing::error!(
                    source = %entry.source,
                    project_id = ?entry.project_id,
                    agent_id = ?entry.agent_id,
                    "{}",
                    entry.message
                );
            }
            LogLevel::Warn => {
                tracing::warn!(
                    source = %entry.source,
                    project_id = ?entry.project_id,
                    agent_id = ?entry.agent_id,
                    "{}",
                    entry.message
                );
            }
            LogLevel::Info => {
                tracing::info!(
                    source = %entry.source,
                    project_id = ?entry.project_id,
                    agent_id = ?entry.agent_id,
                    "{}",
                    entry.message
                );
            }
            LogLevel::Debug => {
                tracing::debug!(
                    source = %entry.source,
                    project_id = ?entry.project_id,
                    agent_id = ?entry.agent_id,
                    "{}",
                    entry.message
                );
            }
        }

        {
            let mut guard = self.inner.logs.write().unwrap_or_else(|e| e.into_inner());
            while guard.len() >= MAX_LOG_ENTRIES {
                guard.pop_front();
            }
            guard.push_back(entry.clone());
        }
        self.emit(EngineEvent::LogEmitted {
            project_id: entry.project_id.clone(),
            entry,
        });
    }

    pub fn list_logs(
        &self,
        project_id: Option<&str>,
        min_level: Option<LogLevel>,
        limit: usize,
    ) -> Vec<LogEntry> {
        if limit == 0 {
            return Vec::new();
        }
        let cap = limit.min(MAX_LOG_ENTRIES);
        let guard = self.inner.logs.read().unwrap_or_else(|e| e.into_inner());
        let min_rank = min_level.map(log_level_rank).unwrap_or(0);
        let mut items: Vec<LogEntry> = guard
            .iter()
            .rev()
            .filter(|entry| {
                let proj_ok = match project_id {
                    Some(pid) => entry.project_id.as_deref().is_none_or(|ep| ep == pid),
                    None => true,
                };
                proj_ok && log_level_rank(entry.level) >= min_rank
            })
            .take(cap)
            .cloned()
            .collect();
        items.reverse();
        items
    }

    pub fn clear_logs(&self, project_id: Option<&str>) {
        let mut guard = self.inner.logs.write().unwrap_or_else(|e| e.into_inner());
        match project_id {
            Some(pid) => guard.retain(|e| e.project_id.as_deref() != Some(pid)),
            None => guard.clear(),
        }
    }

    pub async fn validate_timeline_command(
        &self,
        cmd: &TimelineCommand,
    ) -> Result<(), (StatusCode, String)> {
        if let TimelineCommand::Batch { commands, .. } = cmd {
            if commands.is_empty() {
                return Err((
                    StatusCode::BAD_REQUEST,
                    "batch command must contain at least one sub-command".to_string(),
                ));
            }
            if commands.len() > MAX_BATCH_COMMANDS {
                return Err((
                    StatusCode::BAD_REQUEST,
                    format!("batch command exceeds maximum of {MAX_BATCH_COMMANDS} sub-commands"),
                ));
            }
            for sub in commands {
                if matches!(sub, TimelineCommand::Batch { .. }) {
                    return Err((
                        StatusCode::BAD_REQUEST,
                        "nested batch commands are not allowed".to_string(),
                    ));
                }
                Box::pin(self.validate_timeline_command(sub)).await?;
            }
            return Ok(());
        }
        match cmd {
            TimelineCommand::AddMedia { asset } => {
                if !asset.path.trim().is_empty() {
                    self.resolve_readable_path_async(asset.path.clone())
                        .await
                        .map_err(map_storage_error)?;
                }
                if let Some(proxy) = &asset.intelligence.proxy_path
                    && !proxy.trim().is_empty()
                {
                    self.resolve_readable_path_async(proxy.clone())
                        .await
                        .map_err(map_storage_error)?;
                }
            }
            TimelineCommand::UpdateMediaIntelligence { intelligence, .. } => {
                if let Some(proxy) = &intelligence.proxy_path
                    && !proxy.trim().is_empty()
                {
                    self.resolve_readable_path_async(proxy.clone())
                        .await
                        .map_err(map_storage_error)?;
                }
            }
            TimelineCommand::SetColorGrade { grade, .. } => {
                if let Some(lut) = &grade.lut_path
                    && !lut.trim().is_empty()
                {
                    self.resolve_readable_path_async(lut.clone())
                        .await
                        .map_err(map_storage_error)?;
                }
            }
            TimelineCommand::PlaceClip { clip, .. } => match &clip.payload {
                ClipKind::Grade { grade } => {
                    if let Some(lut) = &grade.lut_path
                        && !lut.trim().is_empty()
                    {
                        self.resolve_readable_path_async(lut.clone())
                            .await
                            .map_err(map_storage_error)?;
                    }
                }
                ClipKind::Gfx { gfx } => {
                    if let Some(html) = &gfx.inline_html {
                        let lint = lint_gfx_html(html);
                        if !lint.valid {
                            return Err((
                                StatusCode::BAD_REQUEST,
                                format!("GFX HTML failed lint: {}", lint.errors.join("; ")),
                            ));
                        }
                    }
                }
                _ => {}
            },
            TimelineCommand::PlaceGfx { gfx, .. } => {
                if let Some(html) = &gfx.inline_html {
                    let lint = lint_gfx_html(html);
                    if !lint.valid {
                        return Err((
                            StatusCode::BAD_REQUEST,
                            format!("GFX HTML failed lint: {}", lint.errors.join("; ")),
                        ));
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    pub async fn load_brand_render_context(
        &self,
        brand_playbook_id: &str,
    ) -> (BrandTokens, Option<String>, Vec<BrandFontData>) {
        let (brand_tokens, brand_default_lut, brand_font_refs) = {
            let guard = self.inner.brands.read().await;
            if let Some(brand) = guard.get(brand_playbook_id) {
                let fonts: Vec<(String, String)> = brand
                    .reference_media
                    .iter()
                    .filter(|r| r.kind == BrandReferenceKind::Font)
                    .map(|r| (r.name.clone(), r.path.clone()))
                    .collect();
                (brand.tokens.clone(), brand.default_lut.clone(), fonts)
            } else {
                (
                    BrandTokens {
                        bg: "#080404".into(),
                        surface: "#1A0E0E".into(),
                        ink: "#F7F5EF".into(),
                        accent: "#F05A00".into(),
                        accent_secondary: "#F7B733".into(),
                        font_display: "Sora".into(),
                        font_body: "Sora".into(),
                    },
                    None,
                    Vec::new(),
                )
            }
        };

        let mut brand_fonts: Vec<BrandFontData> = Vec::new();
        for (font_name, raw_font_path) in brand_font_refs {
            if let Ok(resolved_font) = self.resolve_readable_path_async(raw_font_path).await
                && let Ok(meta) = tokio::fs::metadata(&resolved_font).await
                && meta.is_file()
                && meta.len() <= 8 * 1024 * 1024
                && let Ok(bytes) = tokio::fs::read(&resolved_font).await
            {
                let ext = resolved_font
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("")
                    .to_ascii_lowercase();
                let mime = match ext.as_str() {
                    "woff2" => "font/woff2",
                    "woff" => "font/woff",
                    "otf" => "font/otf",
                    _ => "font/ttf",
                };
                let stem = Path::new(&font_name)
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or(&font_name)
                    .to_string();
                brand_fonts.push(BrandFontData {
                    family: stem,
                    mime: mime.to_string(),
                    data_base64: BASE64_STANDARD.encode(&bytes),
                });
            }
        }
        (brand_tokens, brand_default_lut, brand_fonts)
    }

    pub async fn register_gfx_sandbox(&self, html: String) -> Result<String, (StatusCode, String)> {
        if html.len() > MAX_GFX_SANDBOX_BYTES {
            return Err((
                StatusCode::PAYLOAD_TOO_LARGE,
                format!("GFX sandbox HTML exceeds maximum size of {MAX_GFX_SANDBOX_BYTES} bytes"),
            ));
        }
        let hash = digest::digest(&digest::SHA256, html.as_bytes());
        let mut id = String::with_capacity(32);
        for b in &hash.as_ref()[..16] {
            use std::fmt::Write as _;
            let _ = write!(&mut id, "{b:02x}");
        }

        let mut guard = self.inner.gfx_sandboxes.write().await;
        if let Some(pos) = guard.iter().position(|(k, _)| k == &id) {
            if let Some(entry) = guard.remove(pos) {
                guard.push_back(entry);
            }
        } else {
            while guard.len() >= MAX_GFX_SANDBOXES {
                guard.pop_front();
            }
            guard.push_back((id.clone(), html));
        }
        Ok(id)
    }

    pub async fn get_gfx_sandbox(&self, sandbox_id: &str) -> Option<String> {
        let guard = self.inner.gfx_sandboxes.read().await;
        guard
            .iter()
            .rev()
            .find(|(k, _)| k == sandbox_id)
            .map(|(_, html)| html.clone())
    }

    pub async fn issue_media_ticket(&self, session_id: Option<String>) -> String {
        let now = Utc::now();
        let token = generate_opaque_token();
        let expires_at = now + Duration::minutes(30);
        let mut guard = self.inner.media_tickets.write().await;
        guard.retain(|_, entry| entry.expires_at > now);
        while guard.len() >= MAX_EPHEMERAL_TOKENS {
            if let Some(oldest) = guard
                .iter()
                .min_by_key(|(_, entry)| entry.expires_at)
                .map(|(k, _)| k.clone())
            {
                guard.remove(&oldest);
            } else {
                break;
            }
        }
        guard.insert(
            token.clone(),
            MediaTicketEntry {
                expires_at,
                session_id,
            },
        );
        token
    }

    pub async fn verify_media_ticket(&self, candidate: &str) -> bool {
        if candidate.is_empty() {
            return false;
        }
        let now = Utc::now();
        let guard = self.inner.media_tickets.read().await;
        let mut matched = false;
        for (token, entry) in guard.iter() {
            if entry.expires_at > now && constant_time_eq(token.as_bytes(), candidate.as_bytes()) {
                matched = true;
            }
        }
        matched
    }

    pub async fn issue_session_id(&self) -> String {
        issue_bounded_token(&self.inner.sessions, Duration::minutes(30)).await
    }

    pub async fn verify_session_id(&self, candidate: &str) -> bool {
        verify_bounded_token(&self.inner.sessions, candidate).await
    }

    /// Persists updates to the local OpenRouter API key and/or default model inside
    /// `<workspace_dir>/agent_credentials.json` with owner-only (`0o600`) file permissions.
    /// `None` leaves a field unchanged; `Some(None)` clears it; `Some(Some(val))` updates it.
    pub async fn persist_agent_credentials(
        &self,
        update_api_key: Option<Option<String>>,
        update_default_model: Option<Option<String>>,
    ) -> Result<(), (StatusCode, String)> {
        if update_api_key.is_none() && update_default_model.is_none() {
            return Ok(());
        }
        let _guard = self.inner.credentials_persist_lock.lock().await;
        let workspace_dir = self.inner.workspace_dir.clone();
        tokio::task::spawn_blocking(move || {
            let mut creds = load_persisted_credentials(&workspace_dir);
            if let Some(next_key) = update_api_key {
                creds.openrouter_api_key = next_key;
            }
            if let Some(next_model) = update_default_model {
                creds.default_model = next_model;
            }
            save_persisted_credentials_atomic(&workspace_dir, &creds)
        })
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("credentials persist task failed: {e}"),
            )
        })?
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to persist local agent credentials: {e}"),
            )
        })
    }

    pub async fn revoke_session_id(&self, candidate: &str) {
        {
            let mut guard = self.inner.sessions.write().await;
            let to_remove: Option<String> = guard
                .keys()
                .find(|k| constant_time_eq(k.as_bytes(), candidate.as_bytes()))
                .cloned();
            if let Some(key) = to_remove {
                guard.remove(&key);
            }
        }
        let mut tickets = self.inner.media_tickets.write().await;
        tickets.retain(|_, entry| {
            !entry
                .session_id
                .as_deref()
                .is_some_and(|sid| constant_time_eq(sid.as_bytes(), candidate.as_bytes()))
        });
    }
}

fn log_level_rank(level: LogLevel) -> u8 {
    match level {
        LogLevel::Debug => 0,
        LogLevel::Info => 1,
        LogLevel::Warn => 2,
        LogLevel::Error => 3,
    }
}

fn find_ascii_ci(haystack: &str, needle_lower: &[u8]) -> Option<usize> {
    let h = haystack.as_bytes();
    let n = needle_lower.len();
    if n == 0 || h.len() < n {
        return None;
    }
    for i in 0..=h.len() - n {
        if h[i..i + n]
            .iter()
            .zip(needle_lower.iter())
            .all(|(a, b)| a.to_ascii_lowercase() == *b)
        {
            return Some(i);
        }
    }
    None
}

fn find_auth_scheme_ci(haystack: &str, scheme_lower: &[u8]) -> Option<(usize, usize)> {
    let h = haystack.as_bytes();
    let n = scheme_lower.len();
    if n == 0 || h.len() <= n {
        return None;
    }
    for i in 0..=h.len() - n - 1 {
        if i > 0 && (h[i - 1].is_ascii_alphanumeric() || h[i - 1] == b'_') {
            continue;
        }
        if h[i..i + n]
            .iter()
            .zip(scheme_lower.iter())
            .all(|(a, b)| a.to_ascii_lowercase() == *b)
        {
            let mut j = i + n;
            while j < h.len() && matches!(h[j], b' ' | b'\t') {
                j += 1;
            }
            if j > i + n {
                return Some((i, j));
            }
        }
    }
    None
}

#[derive(Clone, Copy)]
enum SecretKind {
    OpenRouterKey,
    BearerAuth,
    BasicAuth,
    SolarisSession,
    TicketParam,
    SolarisTicketProto,
    AmzSignature,
    AmzSecurityToken,
}

fn sanitize_log_text(raw: &str, max_chars: usize) -> String {
    // Cap raw scan length first on a valid UTF-8 char boundary to bound CPU/memory on huge payloads
    let scan_limit_chars = max_chars.saturating_mul(8).max(256);
    let mut was_pre_truncated = false;
    let bounded = match raw.char_indices().nth(scan_limit_chars) {
        Some((byte_idx, _)) => {
            was_pre_truncated = true;
            &raw[..byte_idx]
        }
        None => raw,
    };

    // Redact accidental OpenRouter API keys, auth headers, sessions, tickets, and AWS signatures
    let mut scrubbed = String::with_capacity(bounded.len().min(max_chars.saturating_add(32)));
    let mut rest = bounded;
    while !rest.is_empty() {
        let mut earliest: Option<(usize, usize, SecretKind)> = None;
        let mut consider = |start: usize, val_start: usize, kind: SecretKind| {
            if earliest.is_none_or(|(best_start, _, _)| start < best_start) {
                earliest = Some((start, val_start, kind));
            }
        };

        if let Some(i) = find_ascii_ci(rest, b"sk-or-") {
            consider(i, i + 6, SecretKind::OpenRouterKey);
        }
        if let Some((i, val_start)) = find_auth_scheme_ci(rest, b"bearer") {
            consider(i, val_start, SecretKind::BearerAuth);
        }
        if let Some((i, val_start)) = find_auth_scheme_ci(rest, b"basic") {
            consider(i, val_start, SecretKind::BasicAuth);
        }
        if let Some(i) = find_ascii_ci(rest, b"solaris_session=") {
            consider(i, i + "solaris_session=".len(), SecretKind::SolarisSession);
        }
        if let Some(i) = find_ascii_ci(rest, b"ticket=") {
            consider(i, i + "ticket=".len(), SecretKind::TicketParam);
        }
        if let Some(i) = find_ascii_ci(rest, b"solaris.ticket.") {
            consider(
                i,
                i + "solaris.ticket.".len(),
                SecretKind::SolarisTicketProto,
            );
        }
        if let Some(i) = find_ascii_ci(rest, b"x-amz-signature=") {
            consider(i, i + "x-amz-signature=".len(), SecretKind::AmzSignature);
        }
        if let Some(i) = find_ascii_ci(rest, b"x-amz-security-token=") {
            consider(
                i,
                i + "x-amz-security-token=".len(),
                SecretKind::AmzSecurityToken,
            );
        }

        let Some((idx, val_start, kind)) = earliest else {
            scrubbed.push_str(rest);
            break;
        };
        scrubbed.push_str(&rest[..idx]);
        let after = &rest[val_start..];
        let (replacement, end) = match kind {
            SecretKind::OpenRouterKey => (
                "<redacted-key>",
                after
                    .find(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '-' | '_')))
                    .unwrap_or(after.len()),
            ),
            SecretKind::BearerAuth => (
                "Bearer <redacted>",
                after
                    .find(|c: char| {
                        c.is_whitespace()
                            || matches!(c, '"' | '\'' | ',' | '}' | ';' | '&' | ')' | ']')
                    })
                    .unwrap_or(after.len()),
            ),
            SecretKind::BasicAuth => (
                "Basic <redacted>",
                after
                    .find(|c: char| {
                        c.is_whitespace()
                            || matches!(c, '"' | '\'' | ',' | '}' | ';' | '&' | ')' | ']')
                    })
                    .unwrap_or(after.len()),
            ),
            SecretKind::SolarisSession => (
                "solaris_session=<redacted>",
                after
                    .find(|c: char| {
                        c.is_whitespace()
                            || matches!(c, ';' | '&' | '"' | '\'' | ',' | '}' | ')' | ']')
                    })
                    .unwrap_or(after.len()),
            ),
            SecretKind::TicketParam => (
                "ticket=<redacted>",
                after
                    .find(|c: char| {
                        c.is_whitespace()
                            || matches!(c, '&' | ';' | '"' | '\'' | ',' | '}' | ')' | ']' | '#')
                    })
                    .unwrap_or(after.len()),
            ),
            SecretKind::SolarisTicketProto => (
                "solaris.ticket.<redacted>",
                after
                    .find(|c: char| {
                        c.is_whitespace()
                            || matches!(c, ',' | ';' | '"' | '\'' | '}' | ')' | ']' | '&')
                    })
                    .unwrap_or(after.len()),
            ),
            SecretKind::AmzSignature => (
                "X-Amz-Signature=<redacted>",
                after
                    .find(|c: char| {
                        c.is_whitespace()
                            || matches!(c, '&' | ';' | '"' | '\'' | ',' | '}' | ')' | ']' | '#')
                    })
                    .unwrap_or(after.len()),
            ),
            SecretKind::AmzSecurityToken => (
                "X-Amz-Security-Token=<redacted>",
                after
                    .find(|c: char| {
                        c.is_whitespace()
                            || matches!(c, '&' | ';' | '"' | '\'' | ',' | '}' | ')' | ']' | '#')
                    })
                    .unwrap_or(after.len()),
            ),
        };
        scrubbed.push_str(replacement);
        rest = &after[end..];
    }

    if scrubbed.chars().count() > max_chars {
        let mut out: String = scrubbed.chars().take(max_chars).collect();
        out.push_str("… [truncated]");
        out
    } else if was_pre_truncated {
        scrubbed.push_str("… [truncated]");
        scrubbed
    } else {
        scrubbed
    }
}

fn generate_opaque_token() -> String {
    let rng = SystemRandom::new();
    let mut bytes = [0u8; 32];
    if rng.fill(&mut bytes).is_ok() {
        URL_SAFE_NO_PAD.encode(bytes)
    } else {
        format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple())
    }
}

async fn issue_bounded_token(
    store: &RwLock<HashMap<String, DateTime<Utc>>>,
    ttl: Duration,
) -> String {
    let now = Utc::now();
    let token = generate_opaque_token();
    let expires = now + ttl;
    let mut guard = store.write().await;
    guard.retain(|_, exp| *exp > now);
    while guard.len() >= MAX_EPHEMERAL_TOKENS {
        if let Some(oldest) = guard
            .iter()
            .min_by_key(|(_, exp)| **exp)
            .map(|(k, _)| k.clone())
        {
            guard.remove(&oldest);
        } else {
            break;
        }
    }
    guard.insert(token.clone(), expires);
    token
}

async fn verify_bounded_token(
    store: &RwLock<HashMap<String, DateTime<Utc>>>,
    candidate: &str,
) -> bool {
    if candidate.is_empty() {
        return false;
    }
    let now = Utc::now();
    let guard = store.read().await;
    let mut matched = false;
    for (token, exp) in guard.iter() {
        if *exp > now && constant_time_eq(token.as_bytes(), candidate.as_bytes()) {
            matched = true;
        }
    }
    matched
}

pub fn is_loopback_host(host: &str) -> bool {
    let trimmed_host = host.trim().trim_start_matches('[').trim_end_matches(']');
    if trimmed_host.eq_ignore_ascii_case("localhost") {
        true
    } else if let Ok(ip) = trimmed_host.parse::<std::net::IpAddr>() {
        ip.is_loopback()
    } else {
        false
    }
}

pub fn validate_bind_auth(host: &str, auth_token: Option<&str>) -> Result<(), std::io::Error> {
    let is_loopback = is_loopback_host(host);
    let has_non_empty_token = auth_token.is_some_and(|t| !t.trim().is_empty());
    if !is_loopback && !has_non_empty_token {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "auth_token is required when binding outside loopback",
        ));
    }
    Ok(())
}

pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    static CT_KEY: OnceLock<ring::hmac::Key> = OnceLock::new();
    let key = CT_KEY.get_or_init(|| {
        let rng = SystemRandom::new();
        ring::hmac::Key::generate(ring::hmac::HMAC_SHA256, &rng).unwrap_or_else(|_| {
            ring::hmac::Key::new(ring::hmac::HMAC_SHA256, b"solaris-constant-time-key")
        })
    });
    let tag_a = ring::hmac::sign(key, a);
    ring::hmac::verify(key, b, tag_a.as_ref()).is_ok()
}

pub fn validate_safe_id(id: &str) -> Result<(), (StatusCode, String)> {
    if id.is_empty()
        || id.len() > 64
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("invalid identifier '{id}': expected 1..=64 ASCII [a-zA-Z0-9_-]"),
        ));
    }
    Ok(())
}

pub fn map_core_error(err: solaris_core_ir::CoreError) -> (StatusCode, String) {
    use solaris_core_ir::CoreError;
    let status = match &err {
        CoreError::TrackNotFound(_) | CoreError::ClipNotFound(_) | CoreError::MediaNotFound(_) => {
            StatusCode::NOT_FOUND
        }
        CoreError::TrackLocked { .. } => StatusCode::CONFLICT,
        CoreError::InvalidOperation(_) | CoreError::NothingToUndo | CoreError::NothingToRedo => {
            StatusCode::BAD_REQUEST
        }
        CoreError::Persistence(_) => StatusCode::INTERNAL_SERVER_ERROR,
    };
    (status, err.to_string())
}

pub fn map_storage_error(err: MediaError) -> (StatusCode, String) {
    let msg = err.to_string();
    let lower = msg.to_ascii_lowercase();
    let status = if lower.contains("escapes workspace")
        || lower.contains("not allowed")
        || lower.contains("bucket mismatch")
    {
        StatusCode::FORBIDDEN
    } else if lower.contains("no such file") || lower.contains("not found") || lower.contains("404")
    {
        StatusCode::NOT_FOUND
    } else {
        StatusCode::BAD_REQUEST
    };
    (status, msg)
}

pub fn save_json_atomic<T: serde::Serialize>(value: &T, path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("data.json");
    let tmp = path.with_file_name(format!(".{file_name}.{}.tmp", Uuid::new_v4()));
    let raw = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
    {
        use std::io::Write;
        let mut f = fs::File::create(&tmp).map_err(|e| e.to_string())?;
        if let Err(err) = f.write_all(raw.as_bytes()).and_then(|_| f.sync_all()) {
            let _ = fs::remove_file(&tmp);
            return Err(err.to_string());
        }
    }
    if let Err(err) = fs::rename(&tmp, path) {
        let _ = fs::remove_file(&tmp);
        return Err(err.to_string());
    }
    Ok(())
}

pub async fn save_json_atomic_async<T: serde::Serialize + Send + 'static>(
    value: T,
    path: PathBuf,
) -> Result<(), String> {
    tokio::task::spawn_blocking(move || save_json_atomic(&value, &path))
        .await
        .map_err(|e| format!("disk write task error: {e}"))?
}

fn starter_code_assets() -> Vec<SavedCodeAsset> {
    let now = Utc::now();
    vec![
        SavedCodeAsset {
            id: "chapter-title-solar".to_string(),
            name: "Chapter Title".to_string(),
            category: "titles".to_string(),
            tags: vec!["title".to_string(), "hook".to_string(), "chapter".to_string()],
            props_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "eyebrow": { "type": "string" },
                    "headline": { "type": "string" }
                }
            }),
            default_props: serde_json::json!({
                "eyebrow": "THE LIST",
                "headline": "3 AI TOOLS"
            }),
            html_source: r#"<div class="clip" data-start="0" data-duration="2.5" style="display:grid;place-items:center;font-family:var(--brand-font-display,Sora);color:var(--brand-ink,#F7F5EF);"><div class="solar-card" style="padding:18px 28px;border-radius:20px;background:rgba(16,9,9,0.88);border:1px solid rgba(240,90,0,0.55);box-shadow:0 18px 44px rgba(0,0,0,0.55),0 0 28px rgba(240,90,0,0.18);text-align:center;max-width:440px;"><div style="font-size:12px;font-weight:600;letter-spacing:0.22em;color:var(--brand-accent-secondary,#F7B733);">{{eyebrow}}</div><div style="font-size:34px;font-weight:700;line-height:1.1;letter-spacing:-0.02em;margin-top:6px;">{{headline}}</div></div></div><script>(function(){if(!window.gsap)return;var el=document.querySelector('.solar-card');var tl=gsap.timeline({paused:true});tl.fromTo(el,{opacity:0,y:22,scale:0.94},{opacity:1,y:0,scale:1,duration:0.42,ease:'power3.out'}).to(el,{opacity:1,duration:1.7}).to(el,{opacity:0,y:-14,duration:0.35,ease:'power2.in'});window.__timelines=window.__timelines||{};window.__timelines.main=tl;})();</script>"#.to_string(),
            default_duration_sec: 2.5,
            created_at: now,
        },
        SavedCodeAsset {
            id: "stat-card-solar".to_string(),
            name: "Stat Card".to_string(),
            category: "cards".to_string(),
            tags: vec!["stat".to_string(), "metric".to_string(), "callout".to_string()],
            props_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "value": { "type": "string" },
                    "label": { "type": "string" }
                }
            }),
            default_props: serde_json::json!({
                "value": "10 HRS",
                "label": "SAVED EVERY WEEK"
            }),
            html_source: r#"<div class="clip" data-start="0" data-duration="3.0" style="display:flex;align-items:center;justify-content:center;font-family:var(--brand-font-display,Sora);"><div class="solar-stat" style="display:flex;align-items:center;gap:14px;padding:16px 24px;border-radius:18px;background:rgba(12,6,6,0.90);border:1px solid rgba(247,245,239,0.18);box-shadow:0 18px 40px rgba(0,0,0,0.52);color:#F7F5EF;max-width:440px;"><span style="font-size:32px;font-weight:700;letter-spacing:-0.02em;white-space:nowrap;color:var(--brand-accent,#F05A00);">{{value}}</span><span style="font-size:12px;font-weight:600;letter-spacing:0.12em;line-height:1.3;opacity:0.88;">{{label}}</span></div></div><script>(function(){if(!window.gsap)return;var el=document.querySelector('.solar-stat');var tl=gsap.timeline({paused:true});tl.fromTo(el,{opacity:0,y:18,scale:0.92},{opacity:1,y:0,scale:1,duration:0.4,ease:'back.out(1.4)'}).to(el,{opacity:1,duration:2.25}).to(el,{opacity:0,y:-12,duration:0.35,ease:'power2.in'});window.__timelines=window.__timelines||{};window.__timelines.main=tl;})();</script>"#.to_string(),
            default_duration_sec: 3.0,
            created_at: now,
        },
        SavedCodeAsset {
            id: "comment-bubble-solar".to_string(),
            name: "Comment Bubble".to_string(),
            category: "social".to_string(),
            tags: vec!["social".to_string(), "comment".to_string(), "cta".to_string()],
            props_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "author": { "type": "string" },
                    "comment": { "type": "string" }
                }
            }),
            default_props: serde_json::json!({
                "author": "@creator",
                "comment": "the voice clone one is insane"
            }),
            html_source: r#"<div class="clip" data-start="0" data-duration="2.8" style="display:grid;place-items:center;font-family:var(--brand-font-body,Sora);"><div class="solar-bubble" style="padding:14px 20px;border-radius:20px;background:rgba(20,11,11,0.92);border:1px solid rgba(247,183,51,0.42);box-shadow:0 16px 38px rgba(0,0,0,0.52);color:#F7F5EF;max-width:420px;"><div style="font-size:12px;color:#F7B733;font-weight:600;letter-spacing:0.04em;">{{author}}</div><div style="font-size:16px;line-height:1.35;margin-top:4px;">{{comment}}</div></div></div><script>(function(){if(!window.gsap)return;var el=document.querySelector('.solar-bubble');var tl=gsap.timeline({paused:true});tl.fromTo(el,{opacity:0,y:16,scale:0.94},{opacity:1,y:0,scale:1,duration:0.38,ease:'back.out(1.3)'}).to(el,{opacity:1,duration:2.1}).to(el,{opacity:0,y:-10,duration:0.32,ease:'power2.in'});window.__timelines=window.__timelines||{};window.__timelines.main=tl;})();</script>"#.to_string(),
            default_duration_sec: 2.8,
            created_at: now,
        },
        SavedCodeAsset {
            id: "lower-third-solar".to_string(),
            name: "Lower Third".to_string(),
            category: "titles".to_string(),
            tags: vec!["lower-third".to_string(), "speaker".to_string(), "explainer".to_string()],
            props_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "title": { "type": "string" },
                    "subtitle": { "type": "string" }
                }
            }),
            default_props: serde_json::json!({
                "title": "Speaker Name",
                "subtitle": "Founder & Host"
            }),
            html_source: r#"<div class="clip" data-start="0" data-duration="3.2" style="display:grid;place-items:center;font-family:var(--brand-font-display,Sora);"><div class="solar-lt" style="display:flex;align-items:center;gap:12px;padding:12px 20px;border-radius:16px;background:rgba(12,6,6,0.90);border:1px solid rgba(247,245,239,0.14);border-left:4px solid var(--brand-accent,#F05A00);box-shadow:0 16px 36px rgba(0,0,0,0.5);color:var(--brand-ink,#F7F5EF);max-width:420px;"><div><div style="font-size:18px;font-weight:600;">{{title}}</div><div style="font-size:12px;color:var(--brand-accent-secondary,#F7B733);margin-top:2px;">{{subtitle}}</div></div></div></div><script>(function(){if(!window.gsap)return;var el=document.querySelector('.solar-lt');var tl=gsap.timeline({paused:true});tl.fromTo(el,{opacity:0,x:-24},{opacity:1,x:0,duration:0.42,ease:'power3.out'}).to(el,{opacity:1,duration:2.4}).to(el,{opacity:0,x:-14,duration:0.35,ease:'power2.in'});window.__timelines=window.__timelines||{};window.__timelines.main=tl;})();</script>"#.to_string(),
            default_duration_sec: 3.2,
            created_at: now,
        },
        SavedCodeAsset {
            id: "progress-bar-solar".to_string(),
            name: "Progress Pill".to_string(),
            category: "overlays".to_string(),
            tags: vec!["progress".to_string(), "retention".to_string(), "bar".to_string()],
            props_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "label": { "type": "string" },
                    "percent": { "type": "number" }
                }
            }),
            default_props: serde_json::json!({
                "label": "STEP 01 / 03",
                "percent": 65
            }),
            html_source: r#"<div class="clip" data-start="0" data-duration="3.0" style="display:grid;place-items:center;font-family:var(--brand-font-body,Sora);"><div class="solar-prog" style="padding:12px 20px;border-radius:999px;background:rgba(16,9,9,0.90);border:1px solid rgba(247,245,239,0.18);box-shadow:0 16px 36px rgba(0,0,0,0.48);color:var(--brand-ink,#F7F5EF);"><div style="display:flex;align-items:center;justify-content:space-between;gap:16px;font-size:11px;font-weight:600;letter-spacing:0.14em;color:var(--brand-accent-secondary,#F7B733);"><span>{{label}}</span><span>{{percent}}%</span></div><div style="margin-top:6px;height:6px;width:220px;border-radius:999px;background:rgba(247,245,239,0.14);overflow:hidden;"><div class="solar-prog-bar" style="height:100%;width:{{percent}}%;border-radius:999px;background:linear-gradient(90deg,var(--brand-accent,#F05A00),var(--brand-accent-secondary,#F7B733));transform-origin:left center;"></div></div></div></div><script>(function(){if(!window.gsap)return;var card=document.querySelector('.solar-prog');var bar=document.querySelector('.solar-prog-bar');var tl=gsap.timeline({paused:true});tl.fromTo(card,{opacity:0,y:14},{opacity:1,y:0,duration:0.35,ease:'power3.out'}).fromTo(bar,{scaleX:0.08},{scaleX:1,duration:1.1,ease:'power2.out'},0.15).to(card,{opacity:1,duration:1.3}).to(card,{opacity:0,y:-10,duration:0.3,ease:'power2.in'});window.__timelines=window.__timelines||{};window.__timelines.main=tl;})();</script>"#.to_string(),
            default_duration_sec: 3.0,
            created_at: now,
        },
    ]
}

fn load_persisted_credentials(workspace_dir: &Path) -> PersistedAgentCredentials {
    let path = workspace_dir.join(AGENT_CREDENTIALS_FILE);
    let Ok(raw) = fs::read_to_string(&path) else {
        return PersistedAgentCredentials::default();
    };
    match serde_json::from_str::<PersistedAgentCredentials>(&raw) {
        Ok(creds) => creds,
        Err(err) => {
            tracing::warn!(
                path = %path.display(),
                "ignoring malformed local agent credentials file on startup: {err}"
            );
            PersistedAgentCredentials::default()
        }
    }
}

fn save_persisted_credentials_atomic(
    workspace_dir: &Path,
    creds: &PersistedAgentCredentials,
) -> std::io::Result<()> {
    let path = workspace_dir.join(AGENT_CREDENTIALS_FILE);
    if creds.openrouter_api_key.is_none() && creds.default_model.is_none() {
        match fs::remove_file(&path) {
            Ok(()) => return Ok(()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(err) => return Err(err),
        }
    }

    fs::create_dir_all(workspace_dir)?;
    let bytes = serde_json::to_vec_pretty(creds)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    let tmp = workspace_dir.join(format!(
        ".{AGENT_CREDENTIALS_FILE}.{}.tmp",
        Uuid::new_v4().simple()
    ));

    let write_res = (|| -> std::io::Result<()> {
        use std::io::Write as _;
        let mut opts = fs::OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            opts.mode(0o600);
        }
        let mut file = opts.open(&tmp)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        Ok(())
    })();

    if let Err(err) = write_res {
        let _ = fs::remove_file(&tmp);
        return Err(err);
    }
    if let Err(err) = fs::rename(&tmp, &path) {
        let _ = fs::remove_file(&tmp);
        return Err(err);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_log_text_redacts_all_secret_patterns() {
        let sample = concat!(
            "key=sk-or-v1-abc123_XYZ ",
            "auth1=bearer   \tSuperSecretToken123 ",
            "auth2=BEARER my_tok ",
            "basic=Basic dXNlcjpwYXNz ",
            "cookie=solaris_session=sess_999; other=1 ",
            "url=/api/v1/media?ticket=tkt_abc&x=1 ",
            "proto=solaris.ticket.ws_tok_42,json ",
            "s3=https://b.s3.amazonaws.com/f?X-Amz-Signature=deadbeef01&X-Amz-Security-Token=IQoJb3JpZ2lu%2Fabc&foo=bar ",
            "cupbearer safe_word"
        );
        let out = sanitize_log_text(sample, 4000);
        assert!(out.contains("key=<redacted-key>"));
        assert!(!out.contains("abc123_XYZ"));
        assert!(out.contains("auth1=Bearer <redacted>"));
        assert!(!out.contains("SuperSecretToken123"));
        assert!(out.contains("auth2=Bearer <redacted>"));
        assert!(!out.contains("my_tok"));
        assert!(out.contains("basic=Basic <redacted>"));
        assert!(!out.contains("dXNlcjpwYXNz"));
        assert!(out.contains("cookie=solaris_session=<redacted>; other=1"));
        assert!(!out.contains("sess_999"));
        assert!(out.contains("url=/api/v1/media?ticket=<redacted>&x=1"));
        assert!(!out.contains("tkt_abc"));
        assert!(out.contains("proto=solaris.ticket.<redacted>,json"));
        assert!(!out.contains("ws_tok_42"));
        assert!(out.contains("X-Amz-Signature=<redacted>&X-Amz-Security-Token=<redacted>&foo=bar"));
        assert!(!out.contains("deadbeef01"));
        assert!(!out.contains("IQoJb3JpZ2lu"));
        // Word-boundary check preserves non-header words like "cupbearer"
        assert!(out.contains("cupbearer safe_word"));
    }

    #[test]
    fn sanitize_log_text_caps_huge_utf8_input_safely() {
        let huge = "🔥".repeat(5000);
        let out = sanitize_log_text(&huge, 32);
        assert!(out.ends_with("… [truncated]"));
        assert!(out.chars().count() <= 32 + "… [truncated]".chars().count());
    }

    #[test]
    fn list_logs_respects_zero_limit_and_chronological_order() {
        let dir = std::env::temp_dir().join(format!("solaris_state_logs_{}", Uuid::new_v4()));
        let cfg = ServerConfig {
            workspace_dir: dir.clone(),
            ..ServerConfig::default()
        };
        let state = AppState::initialize(&cfg).expect("init state");
        for i in 0..5 {
            state.push_log(LogEntry {
                id: format!("log-{i}"),
                timestamp: Utc::now(),
                level: if i % 2 == 0 {
                    LogLevel::Info
                } else {
                    LogLevel::Error
                },
                source: "test".into(),
                project_id: Some("p1".into()),
                run_id: None,
                agent_id: None,
                message: format!("msg {i}"),
                details: None,
            });
        }
        assert!(state.list_logs(Some("p1"), None, 0).is_empty());
        let last_two = state.list_logs(Some("p1"), None, 2);
        assert_eq!(last_two.len(), 2);
        assert_eq!(last_two[0].message, "msg 3");
        assert_eq!(last_two[1].message, "msg 4");

        let errors_only = state.list_logs(Some("p1"), Some(LogLevel::Error), 10);
        assert_eq!(errors_only.len(), 2);
        assert_eq!(errors_only[0].message, "msg 1");
        assert_eq!(errors_only[1].message, "msg 3");

        state.clear_logs(Some("p1"));
        assert!(state.list_logs(Some("p1"), None, 10).is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn agent_credentials_persist_atomically_across_restarts_with_owner_only_mode() {
        let dir = std::env::temp_dir().join(format!("solaris_state_creds_{}", Uuid::new_v4()));
        let cfg = ServerConfig {
            workspace_dir: dir.clone(),
            ..ServerConfig::default()
        };

        let state_first = AppState::initialize(&cfg).expect("init first state");
        state_first
            .persist_agent_credentials(
                Some(Some("sk-or-v1-persisted-test-key".to_string())),
                Some(Some("google/gemini-4-hydrogen".to_string())),
            )
            .await
            .expect("persist key and model");

        let cred_path = dir.join(AGENT_CREDENTIALS_FILE);
        assert!(cred_path.is_file());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = fs::metadata(&cred_path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "credentials file must be owner-only 0600");
        }

        // Simulate a server restart (`cargo run -p solaris-server`) in the same workspace
        let loaded = load_persisted_credentials(&dir);
        assert_eq!(
            loaded.openrouter_api_key.as_deref(),
            Some("sk-or-v1-persisted-test-key")
        );
        assert_eq!(
            loaded.default_model.as_deref(),
            Some("google/gemini-4-hydrogen")
        );
        let state_restarted = AppState::initialize(&cfg).expect("init restarted state");
        assert!(state_restarted.inner.openrouter.is_configured());
        assert_eq!(
            state_restarted
                .inner
                .openrouter
                .config()
                .default_model
                .as_deref(),
            Some("google/gemini-4-hydrogen")
        );

        // Clearing API key preserves saved default_model
        state_restarted
            .persist_agent_credentials(Some(None), None)
            .await
            .expect("clear api key only");
        let after_key_clear = load_persisted_credentials(&dir);
        assert!(after_key_clear.openrouter_api_key.is_none());
        assert_eq!(
            after_key_clear.default_model.as_deref(),
            Some("google/gemini-4-hydrogen")
        );

        // Clearing default_model too removes the credentials file cleanly
        state_restarted
            .persist_agent_credentials(None, Some(None))
            .await
            .expect("clear model too");
        assert!(!cred_path.exists());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn corrupt_dismissed_starters_is_quarantined_and_logged() {
        let dir =
            std::env::temp_dir().join(format!("solaris_corrupt_dismissed_{}", Uuid::new_v4()));
        let assets_dir = dir.join("assets");
        fs::create_dir_all(&assets_dir).expect("create assets dir");
        fs::write(assets_dir.join(DISMISSED_STARTERS_FILE), "{not valid json").expect("write bad");

        let state = AppState::initialize(&ServerConfig {
            workspace_dir: dir.clone(),
            ..ServerConfig::default()
        })
        .expect("init state");

        assert!(!assets_dir.join(DISMISSED_STARTERS_FILE).exists());
        let has_backup = fs::read_dir(&assets_dir)
            .expect("read assets dir")
            .flatten()
            .any(|e| {
                e.file_name().to_str().is_some_and(|n| {
                    n.starts_with(".dismissed_starters.corrupt-") && n.ends_with(".json")
                })
            });
        assert!(
            has_backup,
            "corrupt dismissed starters file must be moved aside"
        );
        assert!(
            state
                .list_logs(None, Some(LogLevel::Warn), 10)
                .iter()
                .any(|l| {
                    l.source == "assets" && l.message.contains("Corrupt .dismissed_starters.json")
                })
        );
        assert_eq!(
            state.inner.assets.blocking_read().len(),
            starter_code_assets().len()
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn delete_asset_persists_dismissal_before_removal_and_emits_events() {
        let dir = std::env::temp_dir().join(format!("solaris_asset_events_{}", Uuid::new_v4()));
        let state = AppState::initialize(&ServerConfig {
            workspace_dir: dir.clone(),
            ..ServerConfig::default()
        })
        .expect("init state");
        let mut rx = state.inner.events_tx.subscribe();

        state
            .delete_asset("stat-card-solar")
            .await
            .expect("delete starter");
        let ev = rx.recv().await.expect("deleted event");
        assert!(matches!(
            ev.payload,
            EngineEvent::AssetsChanged { ref action, ref asset_id }
            if action == "deleted" && asset_id.as_deref() == Some("stat-card-solar")
        ));
        assert!(
            !state
                .inner
                .assets
                .read()
                .await
                .contains_key("stat-card-solar")
        );
        assert!(
            state
                .inner
                .dismissed_starters
                .read()
                .await
                .contains("stat-card-solar")
        );

        // Restarting keeps the starter deleted.
        let restarted = AppState::initialize(&ServerConfig {
            workspace_dir: dir.clone(),
            ..ServerConfig::default()
        })
        .expect("restarted state");
        assert!(
            !restarted
                .inner
                .assets
                .read()
                .await
                .contains_key("stat-card-solar")
        );

        // Restoring re-seeds it and emits restored.
        let mut rx2 = restarted.inner.events_tx.subscribe();
        let (restored_count, _) = restarted.restore_starter_assets().await.expect("restore");
        assert_eq!(restored_count, 1);
        let ev2 = rx2.recv().await.expect("restored event");
        assert!(matches!(
            ev2.payload,
            EngineEvent::AssetsChanged { ref action, asset_id: None }
            if action == "restored"
        ));
        assert!(
            restarted
                .inner
                .assets
                .read()
                .await
                .contains_key("stat-card-solar")
        );

        // Re-saving a dismissed starter via undismiss_starter_locked un-dismisses it.
        restarted
            .delete_asset("stat-card-solar")
            .await
            .expect("delete again");
        {
            let _lock = restarted.inner.asset_persist_lock.lock().await;
            restarted
                .undismiss_starter_locked("stat-card-solar")
                .await
                .expect("undismiss");
        }
        assert!(
            !restarted
                .inner
                .dismissed_starters
                .read()
                .await
                .contains("stat-card-solar")
        );

        let _ = fs::remove_dir_all(&dir);
    }
}
