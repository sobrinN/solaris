use std::fmt;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, SystemTime};

use chrono::{DateTime, Utc};
use futures_util::StreamExt;
use ring::{digest, hmac};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_util::io::ReaderStream;
use uuid::Uuid;

use crate::error::MediaError;
use crate::storage::{AssetStorage, LocalFsStorage};

const DEFAULT_S3_CACHE_MAX_BYTES: u64 = 4 * 1024 * 1024 * 1024; // 4 GiB
const MULTIPART_THRESHOLD_BYTES: u64 = 100 * 1024 * 1024; // 100 MiB
const MULTIPART_PART_SIZE_BYTES: usize = 16 * 1024 * 1024; // 16 MiB

fn shared_s3_runtime() -> Result<&'static tokio::runtime::Runtime, MediaError> {
    static RT: OnceLock<Result<tokio::runtime::Runtime, String>> = OnceLock::new();
    match RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("solaris-s3-io")
            .enable_all()
            .build()
            .map_err(|e| e.to_string())
    }) {
        Ok(rt) => Ok(rt),
        Err(e) => Err(MediaError::StorageError(format!(
            "S3 worker runtime init failed: {e}"
        ))),
    }
}

#[derive(Clone)]
pub struct S3StorageConfig {
    pub bucket: String,
    pub region: String,
    pub endpoint: Option<String>,
    pub access_key_id: String,
    pub secret_access_key: String,
    pub session_token: Option<String>,
    pub key_prefix: String,
    pub force_path_style: bool,
    pub timeout_secs: u64,
}

impl fmt::Debug for S3StorageConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("S3StorageConfig")
            .field("bucket", &self.bucket)
            .field("region", &self.region)
            .field("endpoint", &self.endpoint)
            .field("access_key_id", &self.access_key_id)
            .field("secret_access_key", &"[REDACTED]")
            .field(
                "session_token",
                &self.session_token.as_ref().map(|_| "[REDACTED]"),
            )
            .field("key_prefix", &self.key_prefix)
            .field("force_path_style", &self.force_path_style)
            .field("timeout_secs", &self.timeout_secs)
            .finish()
    }
}

fn should_disable_s3_backend(backend_env: &str, has_bucket: bool) -> bool {
    matches!(backend_env, "local" | "local_fs") || (backend_env != "s3" && !has_bucket)
}

impl S3StorageConfig {
    pub fn from_env() -> Result<Option<Self>, MediaError> {
        let backend_env = std::env::var("SOLARIS_STORAGE_BACKEND")
            .ok()
            .map(|v| v.trim().to_ascii_lowercase())
            .unwrap_or_default();
        let bucket_env = std::env::var("SOLARIS_S3_BUCKET")
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty());

        if should_disable_s3_backend(&backend_env, bucket_env.is_some()) {
            return Ok(None);
        }

        let bucket = bucket_env.ok_or_else(|| {
            MediaError::StorageError("SOLARIS_S3_BUCKET is required for S3 storage".to_string())
        })?;
        let access_key_id = std::env::var("SOLARIS_S3_ACCESS_KEY_ID")
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .ok_or_else(|| {
                MediaError::StorageError(
                    "SOLARIS_S3_ACCESS_KEY_ID is required for S3 storage".to_string(),
                )
            })?;
        let secret_access_key = std::env::var("SOLARIS_S3_SECRET_ACCESS_KEY")
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .ok_or_else(|| {
                MediaError::StorageError(
                    "SOLARIS_S3_SECRET_ACCESS_KEY is required for S3 storage".to_string(),
                )
            })?;
        let session_token = std::env::var("SOLARIS_S3_SESSION_TOKEN")
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty());
        let endpoint = std::env::var("SOLARIS_S3_ENDPOINT")
            .ok()
            .map(|v| v.trim().trim_end_matches('/').to_string())
            .filter(|v| !v.is_empty());
        let region = std::env::var("SOLARIS_S3_REGION")
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| {
                if endpoint.is_some() {
                    "auto".to_string()
                } else {
                    "us-east-1".to_string()
                }
            });
        let key_prefix = std::env::var("SOLARIS_S3_PREFIX")
            .ok()
            .map(|v| v.trim().trim_matches('/').to_string())
            .unwrap_or_default();
        let force_path_style = parse_force_path_style(
            std::env::var("SOLARIS_S3_FORCE_PATH_STYLE").ok().as_deref(),
            endpoint.is_some(),
        );
        let timeout_secs = std::env::var("SOLARIS_S3_TIMEOUT_SECS")
            .ok()
            .and_then(|v| v.trim().parse::<u64>().ok())
            .filter(|&v| v > 0)
            .unwrap_or(300);

        Ok(Some(Self {
            bucket,
            region,
            endpoint,
            access_key_id,
            secret_access_key,
            session_token,
            key_prefix,
            force_path_style,
            timeout_secs,
        }))
    }
}

fn parse_force_path_style(raw: Option<&str>, has_endpoint: bool) -> bool {
    raw.map(str::trim)
        .filter(|v| !v.is_empty())
        .map(|v| matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "yes"))
        .unwrap_or(has_endpoint)
}

fn configured_cache_max_bytes() -> u64 {
    std::env::var("SOLARIS_S3_CACHE_MAX_BYTES")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|&v| v > 0)
        .unwrap_or(DEFAULT_S3_CACHE_MAX_BYTES)
}

#[derive(Debug, Clone)]
pub struct S3Storage {
    config: S3StorageConfig,
    local_cache: LocalFsStorage,
    http_client: Arc<reqwest::Client>,
}

impl S3Storage {
    pub fn new(
        mut config: S3StorageConfig,
        cache_root: impl Into<PathBuf>,
        allow_external_reads: bool,
    ) -> Self {
        config.session_token = config
            .session_token
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        let http_client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(30))
            .read_timeout(Duration::from_secs(120))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self {
            config,
            local_cache: LocalFsStorage::new(cache_root, allow_external_reads),
            http_client: Arc::new(http_client),
        }
    }

    pub fn config(&self) -> &S3StorageConfig {
        &self.config
    }

    fn normalize_relative_key(raw: &str) -> Result<String, MediaError> {
        let rel = Path::new(raw);
        if rel.is_absolute() {
            return Err(MediaError::StorageError(
                "S3 storage key must not be absolute".to_string(),
            ));
        }
        let mut parts = Vec::new();
        for comp in rel.components() {
            match comp {
                Component::Normal(seg) => {
                    let s = seg.to_str().ok_or_else(|| {
                        MediaError::StorageError("non-UTF8 storage key segment".to_string())
                    })?;
                    if !s.is_empty() {
                        parts.push(s.to_string());
                    }
                }
                Component::CurDir => {}
                Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                    return Err(MediaError::StorageError(
                        "S3 storage key must not contain '..' or root prefixes".to_string(),
                    ));
                }
            }
        }
        if parts.is_empty() {
            return Err(MediaError::StorageError(
                "S3 storage key must not be empty".to_string(),
            ));
        }
        Ok(parts.join("/"))
    }

    fn object_key(&self, relative_key: &str) -> Result<String, MediaError> {
        let clean = Self::normalize_relative_key(relative_key)?;
        let prefix = self.config.key_prefix.trim_matches('/');
        if prefix.is_empty() {
            Ok(clean)
        } else {
            Ok(format!("{prefix}/{clean}"))
        }
    }

    /// Namespaces cached S3 objects by full object key so keys with/without `key_prefix`
    /// never alias each other in the local cache directory.
    fn cache_relative_key(&self, full_key: &str) -> String {
        format!(".s3-cache/{full_key}")
    }

    fn evict_s3_cache_if_needed(&self) {
        if let Ok(root) = self.local_cache.canonical_root() {
            let cache_dir = root.join(".s3-cache");
            evict_cache_dir_lru(&cache_dir, configured_cache_max_bytes());
        }
    }

    fn parse_s3_uri(&self, uri: &str) -> Result<(String, String), MediaError> {
        let rest = uri
            .strip_prefix("s3://")
            .ok_or_else(|| MediaError::StorageError(format!("invalid s3 uri '{uri}'")))?;
        let (bucket, raw_key) = rest.split_once('/').ok_or_else(|| {
            MediaError::StorageError(format!("invalid s3 uri '{uri}': missing object key"))
        })?;
        if bucket != self.config.bucket {
            return Err(MediaError::StorageError(format!(
                "s3 bucket mismatch: expected '{}', got '{bucket}'",
                self.config.bucket
            )));
        }
        let clean_key = Self::normalize_relative_key(raw_key)?;
        Ok((bucket.to_string(), clean_key))
    }

    fn build_object_url(&self, full_key: &str) -> Result<(String, String, String), MediaError> {
        let encoded_key = encode_s3_path(full_key);
        let encoded_bucket = encode_uri_component(&self.config.bucket);
        let (scheme, authority, canonical_uri) = if let Some(endpoint) = &self.config.endpoint {
            let trimmed = endpoint.trim_end_matches('/');
            let (scheme, rest) = trimmed.split_once("://").unwrap_or(("https", trimmed));
            let host_only = rest.split('/').next().unwrap_or(rest);
            let raw_base = rest
                .find('/')
                .map(|idx| rest[idx..].trim_matches('/'))
                .unwrap_or("");
            let encoded_base = if raw_base.is_empty() {
                String::new()
            } else {
                format!("/{}", encode_s3_path(raw_base))
            };
            if self.config.force_path_style {
                (
                    scheme.to_string(),
                    host_only.to_string(),
                    format!("{encoded_base}/{encoded_bucket}/{encoded_key}"),
                )
            } else {
                (
                    scheme.to_string(),
                    format!("{}.{host_only}", self.config.bucket),
                    format!("{encoded_base}/{encoded_key}"),
                )
            }
        } else if self.config.force_path_style {
            let host = format!("s3.{}.amazonaws.com", self.config.region);
            (
                "https".to_string(),
                host,
                format!("/{encoded_bucket}/{encoded_key}"),
            )
        } else {
            let host = format!(
                "{}.s3.{}.amazonaws.com",
                self.config.bucket, self.config.region
            );
            ("https".to_string(), host, format!("/{encoded_key}"))
        };

        let url = format!("{scheme}://{authority}{canonical_uri}");
        Ok((url, authority, canonical_uri))
    }

    fn put_bytes_sync(&self, full_key: &str, bytes: Vec<u8>) -> Result<(), MediaError> {
        let (url, host, canonical_uri) = self.build_object_url(full_key)?;
        let config = self.config.clone();
        let client = self.http_client.clone();
        run_blocking_http(move || async move {
            let now = Utc::now();
            let signed = sign_s3_request(&config, "PUT", &host, &canonical_uri, &bytes, now);
            let req = attach_signed_headers(client.put(&url), &config, &host, &signed);
            let resp = req
                .body(bytes)
                .send()
                .await
                .map_err(|e| MediaError::StorageError(format!("S3 PUT failed: {e}")))?;

            if !resp.status().is_success() {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                return Err(MediaError::StorageError(format!(
                    "S3 PUT returned HTTP {status}: {body}"
                )));
            }
            Ok(())
        })
    }

    fn put_file_stream_sync(&self, full_key: &str, local_path: PathBuf) -> Result<(), MediaError> {
        let (url, host, canonical_uri) = self.build_object_url(full_key)?;
        let config = self.config.clone();
        let client = self.http_client.clone();
        run_blocking_http(move || async move {
            let meta = tokio::fs::metadata(&local_path).await?;
            let file_len = meta.len();
            if file_len > MULTIPART_THRESHOLD_BYTES {
                return put_file_multipart_async(
                    &client,
                    &config,
                    &url,
                    &host,
                    &canonical_uri,
                    &local_path,
                    MULTIPART_PART_SIZE_BYTES,
                )
                .await;
            }

            let file = tokio::fs::File::open(&local_path).await?;
            let stream = ReaderStream::new(file);
            let now = Utc::now();
            let signed = sign_s3_request_with_hash(
                &config,
                "PUT",
                &host,
                &canonical_uri,
                "UNSIGNED-PAYLOAD",
                now,
            );
            let req = attach_signed_headers(client.put(&url), &config, &host, &signed)
                .header("Content-Length", file_len.to_string());
            let resp = req
                .body(reqwest::Body::wrap_stream(stream))
                .send()
                .await
                .map_err(|e| MediaError::StorageError(format!("S3 streaming PUT failed: {e}")))?;

            if !resp.status().is_success() {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                return Err(MediaError::StorageError(format!(
                    "S3 PUT returned HTTP {status}: {body}"
                )));
            }
            Ok(())
        })
    }

    fn get_object_to_cache_sync(&self, full_key: &str, cache_rel: &str) -> Result<(), MediaError> {
        let (root, dest) = self.local_cache.validate_confined_write_target(cache_rel)?;
        if let Some(parent) = dest.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)?;
            let canon_parent = std::fs::canonicalize(parent).map_err(|e| {
                MediaError::StorageError(format!(
                    "cannot canonicalize cache parent '{}': {e}",
                    parent.display()
                ))
            })?;
            if !canon_parent.starts_with(&root) {
                return Err(MediaError::StorageError(format!(
                    "cache parent '{}' escapes workspace root '{}'",
                    canon_parent.display(),
                    root.display()
                )));
            }
        }
        let file_name = dest
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("s3_obj");
        let tmp = dest.with_file_name(format!(".{file_name}.{}.tmp", Uuid::new_v4()));

        let (url, host, canonical_uri) = self.build_object_url(full_key)?;
        let config = self.config.clone();
        let client = self.http_client.clone();
        let tmp_clone = tmp.clone();
        let dest_clone = dest.clone();

        run_blocking_http(move || async move {
            let now = Utc::now();
            let signed = sign_s3_request(&config, "GET", &host, &canonical_uri, b"", now);
            let req = attach_signed_headers(client.get(&url), &config, &host, &signed);
            let resp = req
                .send()
                .await
                .map_err(|e| MediaError::StorageError(format!("S3 GET failed: {e}")))?;

            if !resp.status().is_success() {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                return Err(MediaError::StorageError(format!(
                    "S3 GET returned HTTP {status}: {body}"
                )));
            }

            let mut file = tokio::fs::File::create(&tmp_clone).await?;
            let mut stream = resp.bytes_stream();
            while let Some(chunk) = stream.next().await {
                let bytes = match chunk {
                    Ok(b) => b,
                    Err(e) => {
                        drop(file);
                        let _ = tokio::fs::remove_file(&tmp_clone).await;
                        return Err(MediaError::StorageError(format!(
                            "S3 GET stream error: {e}"
                        )));
                    }
                };
                if let Err(e) = file.write_all(&bytes).await {
                    drop(file);
                    let _ = tokio::fs::remove_file(&tmp_clone).await;
                    return Err(MediaError::Io(e));
                }
            }
            if let Err(e) = file.flush().await {
                drop(file);
                let _ = tokio::fs::remove_file(&tmp_clone).await;
                return Err(MediaError::Io(e));
            }
            drop(file);
            if let Err(e) = tokio::fs::rename(&tmp_clone, &dest_clone).await {
                let _ = tokio::fs::remove_file(&tmp_clone).await;
                return Err(MediaError::Io(e));
            }
            Ok(())
        })?;

        self.evict_s3_cache_if_needed();
        Ok(())
    }
}

fn attach_signed_headers(
    mut req: reqwest::RequestBuilder,
    config: &S3StorageConfig,
    host: &str,
    signed: &SignedS3Headers,
) -> reqwest::RequestBuilder {
    req = req
        .header("Host", host)
        .header("x-amz-date", &signed.amz_date)
        .header("x-amz-content-sha256", &signed.payload_sha256)
        .header("Authorization", &signed.authorization);
    if let Some(tok) = config
        .session_token
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        req = req.header("x-amz-security-token", tok);
    }
    req
}

async fn put_file_multipart_async(
    client: &reqwest::Client,
    config: &S3StorageConfig,
    base_url: &str,
    host: &str,
    canonical_uri: &str,
    local_path: &Path,
    part_size: usize,
) -> Result<(), MediaError> {
    let effective_part_size = part_size.max(1024);

    // 1. CreateMultipartUpload: POST {url}?uploads
    let init_url = format!("{base_url}?uploads");
    let init_signed = sign_s3_request_with_query(
        config,
        "POST",
        host,
        canonical_uri,
        "uploads=",
        &sha256_hex(b""),
        Utc::now(),
    );
    let init_resp = attach_signed_headers(client.post(&init_url), config, host, &init_signed)
        .send()
        .await
        .map_err(|e| MediaError::StorageError(format!("S3 CreateMultipartUpload failed: {e}")))?;
    if !init_resp.status().is_success() {
        let status = init_resp.status();
        let body = init_resp.text().await.unwrap_or_default();
        return Err(MediaError::StorageError(format!(
            "S3 CreateMultipartUpload returned HTTP {status}: {body}"
        )));
    }
    let init_xml = init_resp.text().await.unwrap_or_default();
    let upload_id = extract_xml_tag(&init_xml, "UploadId").ok_or_else(|| {
        MediaError::StorageError("S3 CreateMultipartUpload response missing <UploadId>".to_string())
    })?;
    let encoded_upload_id = encode_uri_component(&upload_id);

    // 2. Upload parts sequentially in fixed-size chunks
    let upload_res: Result<Vec<(u32, String)>, MediaError> = async {
        let mut file = tokio::fs::File::open(local_path).await?;
        let mut completed_parts: Vec<(u32, String)> = Vec::new();
        let mut part_number: u32 = 0;

        loop {
            let mut chunk = Vec::with_capacity(effective_part_size);
            while chunk.len() < effective_part_size {
                let mut buf = vec![0u8; (effective_part_size - chunk.len()).min(256 * 1024)];
                let n = file.read(&mut buf).await?;
                if n == 0 {
                    break;
                }
                chunk.extend_from_slice(&buf[..n]);
            }
            if chunk.is_empty() && !completed_parts.is_empty() {
                break;
            }
            part_number += 1;
            let query = format!("partNumber={part_number}&uploadId={encoded_upload_id}");
            let part_url = format!("{base_url}?{query}");
            let payload_hash = sha256_hex(&chunk);
            let signed = sign_s3_request_with_query(
                config,
                "PUT",
                host,
                canonical_uri,
                &query,
                &payload_hash,
                Utc::now(),
            );
            let resp = attach_signed_headers(client.put(&part_url), config, host, &signed)
                .header("Content-Length", chunk.len().to_string())
                .body(chunk)
                .send()
                .await
                .map_err(|e| {
                    MediaError::StorageError(format!("S3 UploadPart {part_number} failed: {e}"))
                })?;
            if !resp.status().is_success() {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                return Err(MediaError::StorageError(format!(
                    "S3 UploadPart {part_number} returned HTTP {status}: {body}"
                )));
            }
            let etag = resp
                .headers()
                .get("etag")
                .and_then(|v| v.to_str().ok())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .ok_or_else(|| {
                    MediaError::StorageError(format!(
                        "S3 UploadPart {part_number} response missing ETag header"
                    ))
                })?;
            completed_parts.push((part_number, etag));
            if part_number >= 10_000 {
                return Err(MediaError::StorageError(
                    "S3 multipart upload exceeded 10,000 part limit".to_string(),
                ));
            }
        }
        Ok(completed_parts)
    }
    .await;

    let parts = match upload_res {
        Ok(p) => p,
        Err(err) => {
            abort_multipart_upload(
                client,
                config,
                base_url,
                host,
                canonical_uri,
                &encoded_upload_id,
            )
            .await;
            return Err(err);
        }
    };

    // 3. CompleteMultipartUpload: POST {url}?uploadId={id}
    let mut xml = String::from("<CompleteMultipartUpload>");
    for (num, etag) in &parts {
        xml.push_str(&format!(
            "<Part><PartNumber>{num}</PartNumber><ETag>{etag}</ETag></Part>"
        ));
    }
    xml.push_str("</CompleteMultipartUpload>");

    let complete_query = format!("uploadId={encoded_upload_id}");
    let complete_url = format!("{base_url}?{complete_query}");
    let complete_bytes = xml.into_bytes();
    let complete_signed = sign_s3_request_with_query(
        config,
        "POST",
        host,
        canonical_uri,
        &complete_query,
        &sha256_hex(&complete_bytes),
        Utc::now(),
    );
    let complete_resp =
        attach_signed_headers(client.post(&complete_url), config, host, &complete_signed)
            .header("Content-Type", "application/xml")
            .body(complete_bytes)
            .send()
            .await;

    match complete_resp {
        Ok(resp) if resp.status().is_success() => {
            let body = resp.text().await.unwrap_or_default();
            if body.contains("<Error>") {
                abort_multipart_upload(
                    client,
                    config,
                    base_url,
                    host,
                    canonical_uri,
                    &encoded_upload_id,
                )
                .await;
                return Err(MediaError::StorageError(format!(
                    "S3 CompleteMultipartUpload returned error body: {body}"
                )));
            }
            Ok(())
        }
        Ok(resp) => {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            abort_multipart_upload(
                client,
                config,
                base_url,
                host,
                canonical_uri,
                &encoded_upload_id,
            )
            .await;
            Err(MediaError::StorageError(format!(
                "S3 CompleteMultipartUpload returned HTTP {status}: {body}"
            )))
        }
        Err(e) => {
            abort_multipart_upload(
                client,
                config,
                base_url,
                host,
                canonical_uri,
                &encoded_upload_id,
            )
            .await;
            Err(MediaError::StorageError(format!(
                "S3 CompleteMultipartUpload failed: {e}"
            )))
        }
    }
}

async fn abort_multipart_upload(
    client: &reqwest::Client,
    config: &S3StorageConfig,
    base_url: &str,
    host: &str,
    canonical_uri: &str,
    encoded_upload_id: &str,
) {
    let query = format!("uploadId={encoded_upload_id}");
    let url = format!("{base_url}?{query}");
    let signed = sign_s3_request_with_query(
        config,
        "DELETE",
        host,
        canonical_uri,
        &query,
        &sha256_hex(b""),
        Utc::now(),
    );
    let _ = attach_signed_headers(client.delete(&url), config, host, &signed)
        .send()
        .await;
}

fn extract_xml_tag(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find(&close)? + start;
    let val = xml[start..end].trim();
    (!val.is_empty()).then(|| val.to_string())
}

fn collect_cache_files(dir: &Path, out: &mut Vec<(PathBuf, u64, SystemTime)>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_cache_files(&path, out);
        } else if path.is_file() {
            if path.extension().and_then(|e| e.to_str()) == Some("tmp") {
                continue;
            }
            if let Ok(meta) = entry.metadata() {
                let mtime = meta
                    .modified()
                    .or_else(|_| meta.accessed())
                    .unwrap_or(SystemTime::UNIX_EPOCH);
                out.push((path, meta.len(), mtime));
            }
        }
    }
}

fn evict_cache_dir_lru(cache_dir: &Path, max_bytes: u64) {
    if !cache_dir.is_dir() {
        return;
    }
    let mut files = Vec::new();
    collect_cache_files(cache_dir, &mut files);
    let mut total: u64 = files.iter().map(|(_, len, _)| *len).sum();
    if total <= max_bytes {
        return;
    }
    // Sort oldest-first by modification time
    files.sort_by_key(|(_, _, mtime)| *mtime);
    for (path, len, _) in files {
        if total <= max_bytes {
            break;
        }
        if std::fs::remove_file(&path).is_ok() {
            total = total.saturating_sub(len);
        }
    }
}

fn touch_cache_file(path: &Path) {
    if let Ok(f) = std::fs::OpenOptions::new().write(true).open(path) {
        let _ = f.set_modified(SystemTime::now());
    }
}

impl AssetStorage for S3Storage {
    fn backend_kind(&self) -> &'static str {
        "s3"
    }

    fn resolve_readable_path(&self, raw_path: &str) -> Result<PathBuf, MediaError> {
        if raw_path.starts_with("s3://") {
            let (_bucket, clean_key) = self.parse_s3_uri(raw_path)?;
            let cache_rel = self.cache_relative_key(&clean_key);
            if let Ok(existing) = self.local_cache.resolve_readable_path(&cache_rel) {
                touch_cache_file(&existing);
                return Ok(existing);
            }
            self.get_object_to_cache_sync(&clean_key, &cache_rel)?;
            return self.local_cache.resolve_readable_path(&cache_rel);
        }

        match self.local_cache.resolve_readable_path(raw_path) {
            Ok(path) => Ok(path),
            Err(local_err) => {
                if Path::new(raw_path).is_relative() {
                    let full_key = self.object_key(raw_path)?;
                    let cache_rel = self.cache_relative_key(&full_key);
                    if let Ok(cached) = self.local_cache.resolve_readable_path(&cache_rel) {
                        touch_cache_file(&cached);
                        return Ok(cached);
                    }
                    self.get_object_to_cache_sync(&full_key, &cache_rel)
                        .map_err(|s3_err| {
                            MediaError::StorageError(format!(
                                "local lookup failed ({local_err}); S3 fallback failed ({s3_err})"
                            ))
                        })?;
                    return self.local_cache.resolve_readable_path(&cache_rel);
                }
                Err(local_err)
            }
        }
    }

    fn save_bytes(&self, relative_key: &str, bytes: &[u8]) -> Result<PathBuf, MediaError> {
        let (root, dest) = self
            .local_cache
            .validate_confined_write_target(relative_key)?;
        if let Some(parent) = dest.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)?;
            let canon_parent = std::fs::canonicalize(parent).map_err(|e| {
                MediaError::StorageError(format!("cannot canonicalize '{}': {e}", parent.display()))
            })?;
            if !canon_parent.starts_with(&root) {
                return Err(MediaError::StorageError(format!(
                    "parent '{}' escapes workspace root '{}'",
                    canon_parent.display(),
                    root.display()
                )));
            }
        }
        let file_name = dest.file_name().and_then(|n| n.to_str()).unwrap_or("blob");
        let tmp = dest.with_file_name(format!(".{file_name}.{}.tmp", Uuid::new_v4()));
        if let Err(err) = std::fs::write(&tmp, bytes) {
            let _ = std::fs::remove_file(&tmp);
            return Err(MediaError::Io(err));
        }

        let full_key = self.object_key(relative_key)?;
        if let Err(err) = self.put_bytes_sync(&full_key, bytes.to_vec()) {
            let _ = std::fs::remove_file(&tmp);
            return Err(err);
        }

        if let Err(err) = std::fs::rename(&tmp, &dest) {
            let _ = std::fs::remove_file(&tmp);
            return Err(MediaError::Io(err));
        }
        Ok(dest)
    }

    fn sync_file(&self, relative_key: &str, local_path: &Path) -> Result<String, MediaError> {
        let full_key = self.object_key(relative_key)?;
        self.put_file_stream_sync(&full_key, local_path.to_path_buf())?;
        Ok(format!("s3://{}/{full_key}", self.config.bucket))
    }
}

fn run_blocking_http<F, Fut, T>(op: F) -> Result<T, MediaError>
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: std::future::Future<Output = Result<T, MediaError>> + Send + 'static,
    T: Send + 'static,
{
    let rt = shared_s3_runtime()?;
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    rt.spawn(async move {
        let res = op().await;
        let _ = tx.send(res);
    });
    rx.recv()
        .map_err(|_| MediaError::StorageError("S3 HTTP worker task cancelled".to_string()))?
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedS3Headers {
    pub amz_date: String,
    pub payload_sha256: String,
    pub authorization: String,
}

fn sha256_hex(payload: &[u8]) -> String {
    hex_lower(digest::digest(&digest::SHA256, payload).as_ref())
}

pub fn sign_s3_request(
    config: &S3StorageConfig,
    method: &str,
    host: &str,
    canonical_uri: &str,
    payload: &[u8],
    now: DateTime<Utc>,
) -> SignedS3Headers {
    let payload_sha256 = sha256_hex(payload);
    sign_s3_request_with_hash(config, method, host, canonical_uri, &payload_sha256, now)
}

pub fn sign_s3_request_with_hash(
    config: &S3StorageConfig,
    method: &str,
    host: &str,
    canonical_uri: &str,
    payload_sha256: &str,
    now: DateTime<Utc>,
) -> SignedS3Headers {
    sign_s3_request_with_query(config, method, host, canonical_uri, "", payload_sha256, now)
}

pub fn sign_s3_request_with_query(
    config: &S3StorageConfig,
    method: &str,
    host: &str,
    canonical_uri: &str,
    canonical_query: &str,
    payload_sha256: &str,
    now: DateTime<Utc>,
) -> SignedS3Headers {
    let amz_date = now.format("%Y%m%dT%H%M%SZ").to_string();
    let date_stamp = now.format("%Y%m%d").to_string();

    let trimmed_tok = config
        .session_token
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());

    let (canonical_headers, signed_headers) = if let Some(tok) = trimmed_tok {
        (
            format!(
                "host:{}\nx-amz-content-sha256:{}\nx-amz-date:{}\nx-amz-security-token:{}\n",
                host.trim().to_ascii_lowercase(),
                payload_sha256,
                amz_date,
                tok
            ),
            "host;x-amz-content-sha256;x-amz-date;x-amz-security-token",
        )
    } else {
        (
            format!(
                "host:{}\nx-amz-content-sha256:{}\nx-amz-date:{}\n",
                host.trim().to_ascii_lowercase(),
                payload_sha256,
                amz_date
            ),
            "host;x-amz-content-sha256;x-amz-date",
        )
    };

    let canonical_request = format!(
        "{method}\n{canonical_uri}\n{canonical_query}\n{canonical_headers}\n{signed_headers}\n{payload_sha256}"
    );
    let canonical_request_hash =
        hex_lower(digest::digest(&digest::SHA256, canonical_request.as_bytes()).as_ref());

    let credential_scope = format!("{date_stamp}/{}/s3/aws4_request", config.region);
    let string_to_sign =
        format!("AWS4-HMAC-SHA256\n{amz_date}\n{credential_scope}\n{canonical_request_hash}");

    let k_secret = format!("AWS4{}", config.secret_access_key);
    let k_date = hmac_sha256(k_secret.as_bytes(), date_stamp.as_bytes());
    let k_region = hmac_sha256(&k_date, config.region.as_bytes());
    let k_service = hmac_sha256(&k_region, b"s3");
    let k_signing = hmac_sha256(&k_service, b"aws4_request");
    let signature = hex_lower(&hmac_sha256(&k_signing, string_to_sign.as_bytes()));

    let authorization = format!(
        "AWS4-HMAC-SHA256 Credential={}/{credential_scope}, SignedHeaders={signed_headers}, Signature={signature}",
        config.access_key_id
    );

    SignedS3Headers {
        amz_date,
        payload_sha256: payload_sha256.to_string(),
        authorization,
    }
}

fn hmac_sha256(key: &[u8], data: &[u8]) -> Vec<u8> {
    let k = hmac::Key::new(hmac::HMAC_SHA256, key);
    hmac::sign(&k, data).as_ref().to_vec()
}

fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

fn encode_s3_path(key: &str) -> String {
    key.split('/')
        .map(encode_uri_component)
        .collect::<Vec<_>>()
        .join("/")
}

fn encode_uri_component(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for byte in input.bytes() {
        let is_unreserved =
            byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~');
        if is_unreserved {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use std::collections::HashMap;
    use std::fs;
    use std::sync::Mutex;
    use tokio::net::TcpListener;

    fn sample_config(endpoint: Option<String>) -> S3StorageConfig {
        S3StorageConfig {
            bucket: "solaris-media".to_string(),
            region: "us-east-1".to_string(),
            endpoint,
            access_key_id: "AKIAIOSFODNN7EXAMPLE".to_string(),
            secret_access_key: "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY".to_string(),
            session_token: Some("sts-secret-token".to_string()),
            key_prefix: "studio".to_string(),
            force_path_style: true,
            timeout_secs: 60,
        }
    }

    #[test]
    fn redacts_secret_in_debug_and_computes_deterministic_sigv4() {
        assert!(parse_force_path_style(Some(""), true));
        assert!(!parse_force_path_style(Some("false"), true));
        assert!(should_disable_s3_backend("local", true));
        assert!(should_disable_s3_backend("local_fs", true));
        assert!(!should_disable_s3_backend("s3", false));
        assert!(!should_disable_s3_backend("", true));
        assert!(should_disable_s3_backend("", false));

        let cfg = sample_config(None);
        let dbg = format!("{cfg:?}");
        assert!(dbg.contains("[REDACTED]"));
        assert!(!dbg.contains("wJalrXUtnFEMI"));
        assert!(!dbg.contains("sts-secret-token"));

        // Verify force_path_style works even when endpoint is None
        let temp = std::env::temp_dir().join(format!("solaris_s3_url_{}", std::process::id()));
        let s3 = S3Storage::new(cfg.clone(), &temp, false);
        let (url, host, uri) = s3.build_object_url("studio/clip.mp4").unwrap();
        assert_eq!(host, "s3.us-east-1.amazonaws.com");
        assert_eq!(uri, "/solaris-media/studio/clip.mp4");
        assert_eq!(
            url,
            "https://s3.us-east-1.amazonaws.com/solaris-media/studio/clip.mp4"
        );
        let _ = fs::remove_dir_all(&temp);

        let fixed_time = Utc.with_ymd_and_hms(2026, 11, 23, 12, 0, 0).unwrap();
        let h1 = sign_s3_request(
            &cfg,
            "PUT",
            "solaris-media.s3.us-east-1.amazonaws.com",
            "/studio/clip.mp4",
            b"hello",
            fixed_time,
        );
        let h2 = sign_s3_request(
            &cfg,
            "PUT",
            "solaris-media.s3.us-east-1.amazonaws.com",
            "/studio/clip.mp4",
            b"hello",
            fixed_time,
        );
        assert_eq!(h1, h2);
        assert_eq!(h1.amz_date, "20261123T120000Z");
        assert!(h1.authorization.contains("x-amz-security-token"));
        let streamed = sign_s3_request_with_hash(
            &cfg,
            "PUT",
            "solaris-media.s3.us-east-1.amazonaws.com",
            "/studio/clip.mp4",
            "UNSIGNED-PAYLOAD",
            fixed_time,
        );
        assert_eq!(streamed.payload_sha256, "UNSIGNED-PAYLOAD");
    }

    #[test]
    fn rejects_traversal_bucket_mismatch_and_evicts_lru_cache() {
        let temp = std::env::temp_dir().join(format!("solaris_s3_neg_{}", std::process::id()));
        let _ = fs::remove_dir_all(&temp);
        let s3 = S3Storage::new(sample_config(None), &temp, false);
        assert!(s3.save_bytes("../bad.txt", b"x").is_err());
        assert!(s3.resolve_readable_path("s3://other-bucket/a.mp4").is_err());

        // Test LRU cache eviction deletes oldest files to respect max_bytes
        let cache_dir = temp.join(".s3-cache");
        fs::create_dir_all(&cache_dir).unwrap();
        let old_file = cache_dir.join("old.bin");
        let new_file = cache_dir.join("new.bin");
        fs::write(&old_file, vec![1u8; 60]).unwrap();
        let old_time = SystemTime::now() - Duration::from_secs(120);
        fs::OpenOptions::new()
            .write(true)
            .open(&old_file)
            .unwrap()
            .set_modified(old_time)
            .unwrap();
        fs::write(&new_file, vec![2u8; 60]).unwrap();

        evict_cache_dir_lru(&cache_dir, 80);
        assert!(!old_file.exists(), "oldest cached object should be evicted");
        assert!(new_file.exists(), "newest cached object should remain");

        let _ = fs::remove_dir_all(&temp);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn roundtrips_streaming_sync_cache_hit_multipart_and_atomic_save_bytes() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let store: Arc<Mutex<HashMap<String, Vec<u8>>>> = Arc::new(Mutex::new(HashMap::new()));
        let get_count: Arc<Mutex<usize>> = Arc::new(Mutex::new(0));
        let mp_parts: Arc<Mutex<HashMap<String, Vec<u8>>>> = Arc::new(Mutex::new(HashMap::new()));
        let store_bg = store.clone();
        let get_bg = get_count.clone();
        let mp_bg = mp_parts.clone();

        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else {
                    break;
                };
                let store_conn = store_bg.clone();
                let get_conn = get_bg.clone();
                let mp_conn = mp_bg.clone();
                tokio::spawn(async move {
                    let mut buf = Vec::new();
                    let mut tmp = [0u8; 4096];
                    let header_end;
                    loop {
                        let n = sock.read(&mut tmp).await.unwrap_or(0);
                        if n == 0 {
                            return;
                        }
                        buf.extend_from_slice(&tmp[..n]);
                        if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                            header_end = pos + 4;
                            break;
                        }
                    }
                    let head = String::from_utf8_lossy(&buf[..header_end]).to_string();
                    let mut lines = head.lines();
                    let req_line = lines.next().unwrap_or("");
                    let mut parts = req_line.split_whitespace();
                    let method = parts.next().unwrap_or("");
                    let path = parts.next().unwrap_or("").to_string();
                    let content_len: usize = lines
                        .find_map(|l| {
                            let (k, v) = l.split_once(':')?;
                            if k.eq_ignore_ascii_case("content-length") {
                                v.trim().parse().ok()
                            } else {
                                None
                            }
                        })
                        .unwrap_or(0);
                    while buf.len() < header_end + content_len {
                        let n = sock.read(&mut tmp).await.unwrap_or(0);
                        if n == 0 {
                            break;
                        }
                        buf.extend_from_slice(&tmp[..n]);
                    }
                    let body = buf[header_end..header_end + content_len].to_vec();

                    if method == "POST" && path.contains("?uploads") {
                        let xml = "<InitiateMultipartUploadResult><UploadId>up-123</UploadId></InitiateMultipartUploadResult>";
                        let hdr = format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            xml.len()
                        );
                        let _ = sock.write_all(hdr.as_bytes()).await;
                        let _ = sock.write_all(xml.as_bytes()).await;
                    } else if method == "PUT" && path.contains("uploadId=up-123") {
                        mp_conn.lock().unwrap().insert(path.clone(), body);
                        let _ = sock
                            .write_all(
                                b"HTTP/1.1 200 OK\r\nETag: \"etag-part\"\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                            )
                            .await;
                    } else if method == "POST" && path.contains("uploadId=up-123") {
                        let xml = "<CompleteMultipartUploadResult><ETag>\"done\"</ETag></CompleteMultipartUploadResult>";
                        let hdr = format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            xml.len()
                        );
                        let _ = sock.write_all(hdr.as_bytes()).await;
                        let _ = sock.write_all(xml.as_bytes()).await;
                    } else if method == "PUT" && path.contains("fail-put") {
                        let _ = sock
                            .write_all(
                                b"HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                            )
                            .await;
                    } else if method == "PUT" {
                        store_conn.lock().unwrap().insert(path, body);
                        let _ = sock
                            .write_all(
                                b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                            )
                            .await;
                    } else if method == "GET" {
                        *get_conn.lock().unwrap() += 1;
                        let data = store_conn.lock().unwrap().get(&path).cloned();
                        if let Some(bytes) = data {
                            let hdr = format!(
                                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                                bytes.len()
                            );
                            let _ = sock.write_all(hdr.as_bytes()).await;
                            let _ = sock.write_all(&bytes).await;
                        } else {
                            let _ = sock
                                .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                                .await;
                        }
                    }
                    let _ = sock.shutdown().await;
                });
            }
        });

        let temp = std::env::temp_dir().join(format!("solaris_s3_rt_{}", std::process::id()));
        let _ = fs::remove_dir_all(&temp);
        let cfg = sample_config(Some(format!("http://{addr}")));
        let s3 = S3Storage::new(cfg.clone(), &temp, false);
        assert_eq!(s3.backend_kind(), "s3");

        let local_src = temp.join("uploads/p1/clip.bin");
        fs::create_dir_all(local_src.parent().unwrap()).unwrap();
        fs::write(&local_src, b"solaris-s3-stream-payload").unwrap();

        let s3_put = s3.clone();
        let src_clone = local_src.clone();
        let s3_uri = tokio::task::spawn_blocking(move || {
            s3_put.sync_file("uploads/p1/clip.bin", &src_clone)
        })
        .await
        .unwrap()
        .unwrap();
        assert_eq!(s3_uri, "s3://solaris-media/studio/uploads/p1/clip.bin");
        assert_eq!(
            store
                .lock()
                .unwrap()
                .get("/solaris-media/studio/uploads/p1/clip.bin")
                .map(Vec::as_slice),
            Some(b"solaris-s3-stream-payload".as_slice())
        );

        // Delete local file to force S3 GET into .s3-cache, then verify relative-key cache hit avoids 2nd GET
        fs::remove_file(&local_src).unwrap();
        let s3_get1 = s3.clone();
        let refetched = tokio::task::spawn_blocking(move || {
            s3_get1.resolve_readable_path("uploads/p1/clip.bin")
        })
        .await
        .unwrap()
        .unwrap();
        assert_eq!(fs::read(&refetched).unwrap(), b"solaris-s3-stream-payload");
        assert_eq!(*get_count.lock().unwrap(), 1);

        let s3_get2 = s3.clone();
        let cached_again = tokio::task::spawn_blocking(move || {
            s3_get2.resolve_readable_path("uploads/p1/clip.bin")
        })
        .await
        .unwrap()
        .unwrap();
        assert_eq!(cached_again, refetched);
        assert_eq!(
            *get_count.lock().unwrap(),
            1,
            "expected .s3-cache hit on relative key"
        );

        // save_bytes must preserve pre-existing local file if S3 PUT fails
        let existing_path = temp.join("uploads/fail-put.txt");
        fs::create_dir_all(existing_path.parent().unwrap()).unwrap();
        fs::write(&existing_path, b"original-content").unwrap();
        let s3_fail = s3.clone();
        let save_err = tokio::task::spawn_blocking(move || {
            s3_fail.save_bytes("uploads/fail-put.txt", b"new-content")
        })
        .await
        .unwrap();
        assert!(save_err.is_err());
        assert_eq!(fs::read(&existing_path).unwrap(), b"original-content");

        // Test multipart upload helper across 3 parts
        let big_local = temp.join("uploads/mp.bin");
        fs::write(&big_local, vec![42u8; 3000]).unwrap();
        let (url, host, uri) = s3.build_object_url("studio/uploads/mp.bin").unwrap();
        put_file_multipart_async(&s3.http_client, &cfg, &url, &host, &uri, &big_local, 1024)
            .await
            .unwrap();
        assert_eq!(mp_parts.lock().unwrap().len(), 3);

        let _ = fs::remove_dir_all(&temp);
    }
}
