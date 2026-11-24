use std::collections::{HashMap, VecDeque};
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
    ExportJobStatus, Project, ProjectEngine, SavedCodeAsset, SwarmRunStatus, TimelineCommand,
    load_project, save_project_atomic,
};
use solaris_media_engine::{
    AssetStorage, BrandFontData, LocalFsStorage, MediaError, MediaToolchain, S3Storage,
    S3StorageConfig, ToolchainCapabilities,
};

const MAX_EPHEMERAL_TOKENS: usize = 256;
const MAX_GFX_SANDBOXES: usize = 128;
pub const MAX_GFX_SANDBOX_BYTES: usize = 512 * 1024;
const DEFAULT_MAX_CONCURRENT_EXPORTS: usize = 2;

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
    pub export_jobs: RwLock<HashMap<String, ExportJobStatus>>,
    pub swarm_runs: RwLock<HashMap<String, SwarmRunStatus>>,
    pub swarm_cancels: RwLock<HashMap<String, CancellationToken>>,
    pub media_tickets: RwLock<HashMap<String, MediaTicketEntry>>,
    pub sessions: RwLock<HashMap<String, DateTime<Utc>>>,
    pub gfx_sandboxes: RwLock<VecDeque<(String, String)>>,
    pub persist_revisions: Mutex<HashMap<String, u64>>,
    pub brand_persist_lock: Mutex<()>,
    pub asset_persist_lock: Mutex<()>,
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
        let projects_dir = config.workspace_dir.join("projects");
        let brands_dir = config.workspace_dir.join("brands");
        let assets_dir = config.workspace_dir.join("assets");
        let uploads_dir = config.workspace_dir.join("uploads");
        let brands_media_dir = config.workspace_dir.join("brands_media");
        let exports_dir = config.workspace_dir.join("exports");
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
                if path.extension().and_then(|e| e.to_str()) == Some("json") {
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
        for starter in starter_code_assets() {
            if !assets.contains_key(&starter.id) {
                let _ =
                    save_json_atomic(&starter, &assets_dir.join(format!("{}.json", starter.id)));
                assets.insert(starter.id.clone(), starter);
            }
        }

        let toolchain = MediaToolchain::from_env();
        let capabilities = toolchain.detect_capabilities();
        let (events_tx, _) = broadcast::channel(512);
        let storage: Arc<dyn AssetStorage> = match config.s3_config.clone() {
            Some(s3_cfg) => Arc::new(S3Storage::new(
                s3_cfg,
                &config.workspace_dir,
                config.allow_external_media_reads,
            )),
            None => Arc::new(LocalFsStorage::new(
                &config.workspace_dir,
                config.allow_external_media_reads,
            )),
        };
        let mut openrouter_cfg = OpenRouterConfig::from_env();
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

        Ok(Self {
            inner: Arc::new(AppStateInner {
                workspace_dir: config.workspace_dir.clone(),
                projects: RwLock::new(projects),
                brands: RwLock::new(brands),
                assets: RwLock::new(assets),
                export_jobs: RwLock::new(HashMap::new()),
                swarm_runs: RwLock::new(HashMap::new()),
                swarm_cancels: RwLock::new(HashMap::new()),
                media_tickets: RwLock::new(HashMap::new()),
                sessions: RwLock::new(HashMap::new()),
                gfx_sandboxes: RwLock::new(VecDeque::new()),
                persist_revisions: Mutex::new(HashMap::new()),
                brand_persist_lock: Mutex::new(()),
                asset_persist_lock: Mutex::new(()),
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
        })
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

    pub fn emit(&self, event: EngineEvent) {
        let _ = self.inner.events_tx.send(EngineEventEnvelope::now(event));
    }

    pub async fn validate_timeline_command(
        &self,
        cmd: &TimelineCommand,
    ) -> Result<(), (StatusCode, String)> {
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
            html_source: r#"<div class="clip" data-start="0" data-duration="2.5" style="display:grid;place-items:center;font-family:var(--brand-font-display,Sora);color:var(--brand-ink,#F7F5EF);"><div style="padding:14px 24px;border-radius:16px;background:rgba(26,14,14,0.86);border:1px solid rgba(240,90,0,0.45);text-align:center;"><div style="font-size:11px;letter-spacing:0.2em;color:var(--brand-accent-secondary,#F7B733);">{{eyebrow}}</div><div style="font-size:28px;font-weight:600;margin-top:4px;">{{headline}}</div></div></div>"#.to_string(),
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
            html_source: r#"<div class="clip" data-start="0" data-duration="3.0" style="display:flex;align-items:center;gap:12px;padding:14px 20px;border-radius:16px;background:rgba(8,4,4,0.88);border:1px solid rgba(247,245,239,0.16);color:#F7F5EF;font-family:var(--brand-font-display,Sora);"><span style="font-size:26px;font-weight:700;color:var(--brand-accent,#F05A00);">{{value}}</span><span style="font-size:12px;letter-spacing:0.08em;opacity:0.85;">{{label}}</span></div>"#.to_string(),
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
            html_source: r#"<div class="clip" data-start="0" data-duration="2.8" style="padding:12px 16px;border-radius:18px;background:rgba(26,14,14,0.92);border:1px solid rgba(247,183,51,0.35);color:#F7F5EF;font-family:var(--brand-font-body,Sora);"><div style="font-size:11px;color:#F7B733;font-weight:600;">{{author}}</div><div style="font-size:14px;margin-top:2px;">{{comment}}</div></div>"#.to_string(),
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
            html_source: r#"<div class="clip" data-start="0" data-duration="3.2" style="display:flex;align-items:center;gap:10px;padding:10px 16px;border-radius:14px;background:rgba(8,4,4,0.88);border-left:3px solid var(--brand-accent,#F05A00);color:var(--brand-ink,#F7F5EF);font-family:var(--brand-font-display,Sora);"><div><div style="font-size:15px;font-weight:600;">{{title}}</div><div style="font-size:11px;color:var(--brand-accent-secondary,#F7B733);">{{subtitle}}</div></div></div>"#.to_string(),
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
            html_source: r#"<div class="clip" data-start="0" data-duration="3.0" style="padding:8px 14px;border-radius:999px;background:rgba(26,14,14,0.88);border:1px solid rgba(247,245,239,0.16);color:var(--brand-ink,#F7F5EF);font-family:var(--brand-font-body,Sora);"><div style="font-size:10px;letter-spacing:0.14em;color:var(--brand-accent-secondary,#F7B733);">{{label}}</div><div style="margin-top:4px;height:4px;width:160px;border-radius:999px;background:rgba(247,245,239,0.15);overflow:hidden;"><div style="height:100%;width:{{percent}}%;background:var(--brand-accent,#F05A00);"></div></div></div>"#.to_string(),
            default_duration_sec: 3.0,
            created_at: now,
        },
    ]
}
