use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use futures_util::StreamExt;
use reqwest::Client;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue};
use serde::{Deserialize, Deserializer, Serialize};
use tokio_util::sync::CancellationToken;

use crate::error::AgentError;

const MAX_TOOL_CALL_SLOTS: usize = 64;

fn deserialize_null_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    T: Default + Deserialize<'de>,
    D: Deserializer<'de>,
{
    Ok(Option::deserialize(deserializer)?.unwrap_or_default())
}

#[derive(Clone)]
pub struct OpenRouterConfig {
    pub api_key: Option<String>,
    pub base_url: String,
    pub http_referer: String,
    pub app_title: String,
    pub app_categories: String,
    pub default_model: Option<String>,
    pub stt_model: Option<String>,
    pub timeout_secs: u64,
}

impl fmt::Debug for OpenRouterConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OpenRouterConfig")
            .field("api_key", &self.api_key.as_ref().map(|_| "<redacted>"))
            .field("base_url", &self.base_url)
            .field("http_referer", &self.http_referer)
            .field("app_title", &self.app_title)
            .field("app_categories", &self.app_categories)
            .field("default_model", &self.default_model)
            .field("stt_model", &self.stt_model)
            .field("timeout_secs", &self.timeout_secs)
            .finish()
    }
}

impl Default for OpenRouterConfig {
    fn default() -> Self {
        Self {
            api_key: None,
            base_url: "https://openrouter.ai/api/v1".to_string(),
            http_referer: "https://solaris.local".to_string(),
            app_title: "Solaris Studio".to_string(),
            app_categories: "creative-tools,video-editing".to_string(),
            default_model: None,
            stt_model: None,
            timeout_secs: 90,
        }
    }
}

impl OpenRouterConfig {
    pub fn from_env() -> Self {
        let api_key = std::env::var("OPENROUTER_API_KEY")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let base_url = std::env::var("OPENROUTER_BASE_URL")
            .ok()
            .map(|s| s.trim().trim_end_matches('/').to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "https://openrouter.ai/api/v1".to_string());
        let http_referer = std::env::var("SOLARIS_APP_URL")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "https://solaris.local".to_string());
        let default_model = std::env::var("SOLARIS_OPENROUTER_MODEL")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let stt_model = std::env::var("SOLARIS_STT_MODEL")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        Self {
            api_key,
            base_url,
            http_referer,
            default_model,
            stt_model,
            ..Self::default()
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProviderPreferences {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sort: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allow_fallbacks: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub require_parameters: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub zdr: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheControl {
    #[serde(rename = "type")]
    pub kind: String,
}

impl CacheControl {
    pub fn ephemeral() -> Self {
        Self {
            kind: "ephemeral".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentPart {
    Text {
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cache_control: Option<CacheControl>,
    },
    ImageUrl {
        image_url: MediaUrlRef,
    },
    InputAudio {
        input_audio: InputAudioPayload,
    },
    VideoUrl {
        video_url: MediaUrlRef,
    },
}

impl ContentPart {
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text {
            text: text.into(),
            cache_control: None,
        }
    }

    pub fn text_cached(text: impl Into<String>) -> Self {
        Self::Text {
            text: text.into(),
            cache_control: Some(CacheControl::ephemeral()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaUrlRef {
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InputAudioPayload {
    pub data: String,
    pub format: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MessageContent {
    Text(String),
    Parts(Vec<ContentPart>),
}

impl MessageContent {
    pub fn as_text(&self) -> String {
        match self {
            Self::Text(s) => s.clone(),
            Self::Parts(parts) => parts
                .iter()
                .filter_map(|p| match p {
                    ContentPart::Text { text, .. } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<MessageContent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_details: Option<serde_json::Value>,
}

impl ChatMessage {
    pub fn system(text: impl Into<String>) -> Self {
        Self {
            role: "system".to_string(),
            content: Some(MessageContent::Text(text.into())),
            tool_calls: None,
            tool_call_id: None,
            reasoning_details: None,
        }
    }

    pub fn system_cached(text: impl Into<String>) -> Self {
        Self {
            role: "system".to_string(),
            content: Some(MessageContent::Parts(vec![ContentPart::text_cached(text)])),
            tool_calls: None,
            tool_call_id: None,
            reasoning_details: None,
        }
    }

    pub fn user_text(text: impl Into<String>) -> Self {
        Self {
            role: "user".to_string(),
            content: Some(MessageContent::Text(text.into())),
            tool_calls: None,
            tool_call_id: None,
            reasoning_details: None,
        }
    }

    pub fn user_text_cached(text: impl Into<String>) -> Self {
        Self {
            role: "user".to_string(),
            content: Some(MessageContent::Parts(vec![ContentPart::text_cached(text)])),
            tool_calls: None,
            tool_call_id: None,
            reasoning_details: None,
        }
    }

    pub fn user_parts(parts: Vec<ContentPart>) -> Self {
        Self {
            role: "user".to_string(),
            content: Some(MessageContent::Parts(parts)),
            tool_calls: None,
            tool_call_id: None,
            reasoning_details: None,
        }
    }

    pub fn tool_result(tool_call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: "tool".to_string(),
            content: Some(MessageContent::Text(content.into())),
            tool_calls: None,
            tool_call_id: Some(tool_call_id.into()),
            reasoning_details: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    #[serde(rename = "type")]
    pub kind: String,
    pub function: ToolFunctionDef,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolFunctionDef {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub function: ToolCallFunction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallFunction {
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ReasoningConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exclude: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ChatRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub models: Option<Vec<String>>,
    pub messages: Vec<ChatMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<ToolDefinition>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<ProviderPreferences>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<ReasoningConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_format: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plugins: Option<Vec<serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream: Option<bool>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CompletionTokensDetails {
    #[serde(default)]
    pub reasoning_tokens: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ChatUsage {
    #[serde(default)]
    pub prompt_tokens: Option<u64>,
    #[serde(default)]
    pub completion_tokens: Option<u64>,
    #[serde(default)]
    pub total_tokens: Option<u64>,
    #[serde(default)]
    pub completion_tokens_details: Option<CompletionTokensDetails>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatCompletionResponse {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default, deserialize_with = "deserialize_null_default")]
    pub choices: Vec<ChatChoice>,
    #[serde(default)]
    pub usage: Option<ChatUsage>,
}

impl ChatCompletionResponse {
    pub fn finish_reason(&self) -> Option<&str> {
        self.choices
            .first()
            .and_then(|c| c.finish_reason.as_deref())
    }

    pub fn usage_summary(&self) -> String {
        let Some(ref u) = self.usage else {
            return "usage=n/a".to_string();
        };
        let p = u.prompt_tokens.unwrap_or(0);
        let c = u.completion_tokens.unwrap_or(0);
        let r = u
            .completion_tokens_details
            .as_ref()
            .and_then(|d| d.reasoning_tokens);
        match r {
            Some(rt) => format!("prompt={p}, completion={c} (reasoning={rt})"),
            None => format!("prompt={p}, completion={c}"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatChoice {
    #[serde(default)]
    pub index: u32,
    pub message: ChatMessage,
    #[serde(default)]
    pub finish_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenRouterModel {
    pub id: String,
    #[serde(default, deserialize_with = "deserialize_null_default")]
    pub name: String,
    #[serde(default)]
    pub context_length: Option<u64>,
    #[serde(default)]
    pub architecture: Option<ModelArchitecture>,
    #[serde(default)]
    pub pricing: Option<ModelPricing>,
    #[serde(default, deserialize_with = "deserialize_null_default")]
    pub supported_parameters: Vec<String>,
    #[serde(default)]
    pub expiration_date: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ModelArchitecture {
    #[serde(default, deserialize_with = "deserialize_null_default")]
    pub input_modalities: Vec<String>,
    #[serde(default, deserialize_with = "deserialize_null_default")]
    pub output_modalities: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ModelPricing {
    #[serde(default)]
    pub prompt: Option<String>,
    #[serde(default)]
    pub completion: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ModelsListEnvelope {
    #[serde(default, deserialize_with = "deserialize_null_default")]
    data: Vec<OpenRouterModel>,
}

#[derive(Debug, Deserialize)]
struct SseChunk {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    error: Option<serde_json::Value>,
    #[serde(default, deserialize_with = "deserialize_null_default")]
    choices: Vec<SseChoice>,
    #[serde(default)]
    usage: Option<ChatUsage>,
}

#[derive(Debug, Deserialize)]
struct SseChoice {
    #[serde(default, deserialize_with = "deserialize_null_default")]
    delta: SseDelta,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct SseDelta {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<SseToolCallDelta>>,
    #[serde(default)]
    reasoning_details: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct SseToolCallDelta {
    #[serde(default)]
    index: Option<usize>,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    function: Option<SseToolCallFnDelta>,
}

#[derive(Debug, Deserialize)]
struct SseToolCallFnDelta {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
}

#[derive(Clone)]
pub struct OpenRouterClient {
    config: Arc<RwLock<OpenRouterConfig>>,
    http: Client,
}

impl OpenRouterClient {
    pub fn new(config: OpenRouterConfig) -> Self {
        let read_secs = config.timeout_secs.max(10);
        let http = Client::builder()
            .connect_timeout(Duration::from_secs(15))
            .read_timeout(Duration::from_secs(read_secs))
            .build()
            .unwrap_or_else(|_| Client::new());
        Self {
            config: Arc::new(RwLock::new(config)),
            http,
        }
    }

    pub fn config(&self) -> OpenRouterConfig {
        self.config
            .read()
            .map(|g| g.clone())
            .unwrap_or_else(|e| e.into_inner().clone())
    }

    pub fn set_api_key(&self, api_key: Option<String>) {
        let cleaned = api_key
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        if let Ok(mut g) = self.config.write() {
            g.api_key = cleaned;
        }
    }

    pub fn set_default_model(&self, model: Option<String>) {
        let cleaned = model
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        if let Ok(mut g) = self.config.write() {
            g.default_model = cleaned;
        }
    }

    pub fn set_stt_model(&self, model: Option<String>) {
        let cleaned = model
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        if let Ok(mut g) = self.config.write() {
            g.stt_model = cleaned;
        }
    }

    pub fn is_configured(&self) -> bool {
        self.config()
            .api_key
            .as_ref()
            .is_some_and(|k| !k.trim().is_empty())
    }

    pub fn http_client(&self) -> &Client {
        &self.http
    }

    pub fn attribution_headers(&self, include_auth: bool) -> Result<HeaderMap, AgentError> {
        let cfg = self.config();
        let mut headers = HeaderMap::new();
        if include_auth {
            let key = cfg
                .api_key
                .as_deref()
                .filter(|k| !k.trim().is_empty())
                .ok_or(AgentError::NotConfigured)?;
            let mut auth_val = HeaderValue::from_str(&format!("Bearer {key}"))
                .map_err(|e| AgentError::Serialization(e.to_string()))?;
            auth_val.set_sensitive(true);
            headers.insert(AUTHORIZATION, auth_val);
        }
        if let Ok(v) = HeaderValue::from_str(&cfg.http_referer) {
            headers.insert("HTTP-Referer", v);
        }
        if let Ok(v) = HeaderValue::from_str(&cfg.app_title) {
            headers.insert("X-OpenRouter-Title", v);
        }
        if let Ok(v) = HeaderValue::from_str(&cfg.app_categories) {
            headers.insert("X-OpenRouter-Categories", v);
        }
        Ok(headers)
    }

    pub async fn list_models(
        &self,
        input_modalities: Option<&str>,
        output_modalities: Option<&str>,
        supported_parameters: Option<&str>,
    ) -> Result<Vec<OpenRouterModel>, AgentError> {
        let cfg = self.config();
        let mut url = reqwest::Url::parse(&format!("{}/models", cfg.base_url))
            .map_err(|e| AgentError::Transport(e.to_string()))?;
        {
            let mut pairs = url.query_pairs_mut();
            if let Some(v) = input_modalities {
                pairs.append_pair("input_modalities", v);
            }
            if let Some(v) = output_modalities {
                pairs.append_pair("output_modalities", v);
            }
            if let Some(v) = supported_parameters {
                pairs.append_pair("supported_parameters", v);
            }
        }
        let headers = self.attribution_headers(false)?;
        let resp = self
            .http
            .get(url)
            .headers(headers)
            .timeout(Duration::from_secs(12))
            .send()
            .await
            .map_err(|e| AgentError::Transport(e.to_string()))?;
        let status = resp.status();
        if !status.is_success() {
            let retry_after = parse_retry_after(resp.headers());
            let body = resp.text().await.unwrap_or_default();
            return Err(AgentError::HttpStatus {
                status: status.as_u16(),
                message: truncate_msg(&body),
                retry_after_secs: retry_after,
            });
        }
        let env: ModelsListEnvelope = resp
            .json()
            .await
            .map_err(|e| AgentError::Serialization(e.to_string()))?;
        Ok(env.data)
    }

    pub async fn chat_stream<F>(
        &self,
        req: &ChatRequest,
        cancel: Option<&CancellationToken>,
        mut on_text_delta: F,
    ) -> Result<ChatCompletionResponse, AgentError>
    where
        F: FnMut(&str) + Send,
    {
        let mut payload = req.clone();
        payload.stream = Some(true);

        let url = format!("{}/chat/completions", self.config().base_url);
        let mut attempt = 0u32;
        loop {
            if cancel.is_some_and(CancellationToken::is_cancelled) {
                return Err(AgentError::Cancelled);
            }
            let headers = self.attribution_headers(true)?;
            let send_fut = self
                .http
                .post(&url)
                .headers(headers)
                .header(CONTENT_TYPE, "application/json")
                .json(&payload)
                .send();

            let res = if let Some(tok) = cancel {
                tokio::select! {
                    _ = tok.cancelled() => return Err(AgentError::Cancelled),
                    r = send_fut => r,
                }
            } else {
                send_fut.await
            };

            let resp = match res {
                Ok(r) if r.status().is_success() => r,
                Ok(r) => {
                    let status = r.status().as_u16();
                    let retry_after = parse_retry_after(r.headers());
                    let body_fut = r.text();
                    let body = if let Some(tok) = cancel {
                        tokio::select! {
                            _ = tok.cancelled() => return Err(AgentError::Cancelled),
                            t = body_fut => t.unwrap_or_default(),
                        }
                    } else {
                        body_fut.await.unwrap_or_default()
                    };
                    let err = AgentError::HttpStatus {
                        status,
                        message: truncate_msg(&body),
                        retry_after_secs: retry_after,
                    };
                    if err.is_retryable() && attempt < 3 {
                        attempt += 1;
                        let delay_ms = backoff_delay_ms(attempt, retry_after);
                        sleep_cancellable(Duration::from_millis(delay_ms), cancel).await?;
                        continue;
                    }
                    return Err(err);
                }
                Err(e) => {
                    let err = AgentError::Transport(e.to_string());
                    if attempt < 3 {
                        attempt += 1;
                        let delay_ms = backoff_delay_ms(attempt, None);
                        sleep_cancellable(Duration::from_millis(delay_ms), cancel).await?;
                        continue;
                    }
                    return Err(err);
                }
            };

            let is_json_response = resp
                .headers()
                .get(CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .is_some_and(|ct| ct.to_ascii_lowercase().contains("application/json"));

            if is_json_response {
                let body_fut = resp.text();
                let body_res = if let Some(tok) = cancel {
                    tokio::select! {
                        _ = tok.cancelled() => return Err(AgentError::Cancelled),
                        t = body_fut => t,
                    }
                } else {
                    body_fut.await
                };
                let body = match body_res {
                    Ok(b) => b,
                    Err(e) => {
                        let err = AgentError::Transport(e.to_string());
                        if attempt < 3 {
                            attempt += 1;
                            let delay_ms = backoff_delay_ms(attempt, None);
                            sleep_cancellable(Duration::from_millis(delay_ms), cancel).await?;
                            continue;
                        }
                        return Err(err);
                    }
                };
                match parse_http200_json_response(&body, &mut on_text_delta) {
                    Ok(parsed) => return Ok(parsed),
                    Err(err) => {
                        let retryable = extract_json_body_error_code(&body)
                            .is_some_and(AgentError::is_retryable_status);
                        if retryable && attempt < 3 {
                            attempt += 1;
                            let delay_ms = backoff_delay_ms(attempt, None);
                            sleep_cancellable(Duration::from_millis(delay_ms), cancel).await?;
                            continue;
                        }
                        return Err(err);
                    }
                }
            }

            let mut acc = SseAccumulator::default();
            let mut byte_buf: Vec<u8> = Vec::new();
            let mut byte_stream = resp.bytes_stream();

            let stream_res: Result<ChatCompletionResponse, AgentError> = 'sse: {
                loop {
                    let next_chunk = if let Some(tok) = cancel {
                        tokio::select! {
                            _ = tok.cancelled() => break 'sse Err(AgentError::Cancelled),
                            c = byte_stream.next() => c,
                        }
                    } else {
                        byte_stream.next().await
                    };

                    let Some(chunk_res) = next_chunk else {
                        break;
                    };
                    let bytes = match chunk_res {
                        Ok(b) => b,
                        Err(e) => break 'sse Err(AgentError::Transport(e.to_string())),
                    };
                    byte_buf.extend_from_slice(&bytes);

                    match drain_utf8_lines(&mut byte_buf, &mut acc, &mut on_text_delta) {
                        Ok(true) => break 'sse acc.finish_inner(&mut on_text_delta),
                        Ok(false) => {}
                        Err(err) => break 'sse Err(err),
                    }
                }

                if !byte_buf.is_empty() {
                    let tail = String::from_utf8_lossy(&byte_buf);
                    if !tail.trim().is_empty()
                        && let Err(err) = acc.feed_line(tail.trim(), &mut on_text_delta)
                    {
                        break 'sse Err(err);
                    }
                }

                acc.finish_inner(&mut on_text_delta)
            };

            match stream_res {
                Ok(parsed) => return Ok(parsed),
                Err(AgentError::Cancelled) => return Err(AgentError::Cancelled),
                Err(err) => {
                    let retryable = !acc.emitted_any
                        && (matches!(err, AgentError::Transport(_))
                            || acc
                                .last_error_code
                                .is_some_and(AgentError::is_retryable_status));
                    if retryable && attempt < 3 {
                        attempt += 1;
                        let delay_ms = backoff_delay_ms(attempt, None);
                        sleep_cancellable(Duration::from_millis(delay_ms), cancel).await?;
                        continue;
                    }
                    return Err(err);
                }
            }
        }
    }
}

fn extract_error_code(err: &serde_json::Value) -> Option<u16> {
    let code_val = err.get("code")?;
    if let Some(n) = code_val.as_u64() {
        return u16::try_from(n).ok();
    }
    code_val.as_str().and_then(|s| s.trim().parse::<u16>().ok())
}

fn extract_json_body_error_code(body: &str) -> Option<u16> {
    let val: serde_json::Value = serde_json::from_str(body).ok()?;
    let err = val.get("error").filter(|v| !v.is_null())?;
    extract_error_code(err)
}

fn merge_reasoning_details(slot: &mut Option<serde_json::Value>, incoming: serde_json::Value) {
    if incoming.is_null() {
        return;
    }
    match (slot.as_mut(), incoming) {
        (Some(serde_json::Value::Array(acc)), serde_json::Value::Array(items)) => {
            for item in items {
                let merged = if let (
                    Some(serde_json::Value::Object(last)),
                    serde_json::Value::Object(next),
                ) = (acc.last_mut(), &item)
                {
                    let same_type = last.get("type") == next.get("type");
                    let same_id = last.get("id") == next.get("id");
                    let same_index = last.get("index") == next.get("index");
                    if same_type && same_id && same_index {
                        match last.get("type").and_then(|v| v.as_str()) {
                            Some("reasoning.text") => {
                                if let (
                                    Some(serde_json::Value::String(prev_t)),
                                    Some(serde_json::Value::String(next_t)),
                                ) = (last.get_mut("text"), next.get("text"))
                                {
                                    prev_t.push_str(next_t);
                                    true
                                } else {
                                    false
                                }
                            }
                            Some("reasoning.summary") => {
                                if let (
                                    Some(serde_json::Value::String(prev_s)),
                                    Some(serde_json::Value::String(next_s)),
                                ) = (last.get_mut("summary"), next.get("summary"))
                                {
                                    prev_s.push_str(next_s);
                                    true
                                } else {
                                    false
                                }
                            }
                            _ => false,
                        }
                    } else {
                        false
                    }
                } else {
                    false
                };
                if !merged {
                    acc.push(item);
                }
            }
        }
        (_, other) => {
            *slot = Some(other);
        }
    }
}

fn parse_http200_json_response<F>(
    body: &str,
    on_text_delta: &mut F,
) -> Result<ChatCompletionResponse, AgentError>
where
    F: FnMut(&str),
{
    let val: serde_json::Value = serde_json::from_str(body)
        .map_err(|e| AgentError::Serialization(format!("JSON response parse error: {e}")))?;
    if let Some(err) = val.get("error").filter(|v| !v.is_null()) {
        return Err(AgentError::StreamError(truncate_msg(&err.to_string())));
    }
    let parsed: ChatCompletionResponse = serde_json::from_value(val)
        .map_err(|e| AgentError::Serialization(format!("chat completion parse error: {e}")))?;
    if let Some(text) = parsed
        .choices
        .first()
        .and_then(|c| c.message.content.as_ref())
        .map(MessageContent::as_text)
        && !text.is_empty()
    {
        on_text_delta(&text);
    }
    Ok(parsed)
}

fn drain_utf8_lines<F>(
    byte_buf: &mut Vec<u8>,
    acc: &mut SseAccumulator,
    on_text_delta: &mut F,
) -> Result<bool, AgentError>
where
    F: FnMut(&str),
{
    while let Some(nl_pos) = byte_buf.iter().position(|&b| b == b'\n') {
        let raw_line: Vec<u8> = byte_buf.drain(..=nl_pos).collect();
        let line_owned = String::from_utf8_lossy(&raw_line[..raw_line.len() - 1]).into_owned();
        let line = line_owned.trim_end_matches('\r');
        if acc.feed_line(line, on_text_delta)? {
            return Ok(true);
        }
    }
    Ok(false)
}

#[derive(Default)]
struct SseAccumulator {
    id: Option<String>,
    model: Option<String>,
    content: String,
    finish_reason: Option<String>,
    tool_calls: BTreeMap<usize, ToolCall>,
    tool_id_slots: HashMap<String, usize>,
    reasoning_details: Option<serde_json::Value>,
    usage: Option<ChatUsage>,
    event_data: String,
    raw_non_sse: String,
    seen_sse_data: bool,
    emitted_any: bool,
    last_error_code: Option<u16>,
    done: bool,
}

impl SseAccumulator {
    fn feed_line<F>(&mut self, line: &str, on_delta: &mut F) -> Result<bool, AgentError>
    where
        F: FnMut(&str),
    {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            if !self.event_data.is_empty() {
                let pending = std::mem::take(&mut self.event_data);
                if self.dispatch_event_payload(&pending, on_delta)? {
                    self.done = true;
                    return Ok(true);
                }
            }
            return Ok(false);
        }
        if trimmed.starts_with(':') {
            return Ok(false);
        }
        if let Some(raw_data) = trimmed.strip_prefix("data:") {
            self.seen_sse_data = true;
            let data = raw_data.trim();
            if data == "[DONE]" {
                if !self.event_data.is_empty() {
                    let pending = std::mem::take(&mut self.event_data);
                    let _ = self.dispatch_event_payload(&pending, on_delta)?;
                }
                self.done = true;
                return Ok(true);
            }
            if !self.event_data.is_empty() {
                // If previous buffered payload is already a complete JSON object, dispatch it now
                // (supports streams without blank line separators); otherwise join multi-line SSE data.
                if serde_json::from_str::<serde_json::Value>(&self.event_data).is_ok() {
                    let pending = std::mem::take(&mut self.event_data);
                    if self.dispatch_event_payload(&pending, on_delta)? {
                        self.done = true;
                        return Ok(true);
                    }
                    self.event_data.push_str(data);
                } else {
                    self.event_data.push('\n');
                    self.event_data.push_str(data);
                }
            } else {
                self.event_data.push_str(data);
            }

            // Eagerly dispatch once buffered data forms complete JSON so deltas stream immediately.
            if serde_json::from_str::<serde_json::Value>(&self.event_data).is_ok() {
                let pending = std::mem::take(&mut self.event_data);
                if self.dispatch_event_payload(&pending, on_delta)? {
                    self.done = true;
                    return Ok(true);
                }
            }
            return Ok(false);
        }

        // Ignore standard SSE control fields (event:, id:, retry:)
        if trimmed.starts_with("event:")
            || trimmed.starts_with("id:")
            || trimmed.starts_with("retry:")
        {
            return Ok(false);
        }

        // Buffer non-SSE body lines (e.g. HTTP 200 raw JSON {"error": ...} without SSE framing)
        if !self.seen_sse_data {
            if !self.raw_non_sse.is_empty() {
                self.raw_non_sse.push('\n');
            }
            self.raw_non_sse.push_str(trimmed);
        }
        Ok(false)
    }

    fn dispatch_event_payload<F>(
        &mut self,
        payload: &str,
        on_delta: &mut F,
    ) -> Result<bool, AgentError>
    where
        F: FnMut(&str),
    {
        let data = payload.trim();
        if data.is_empty() {
            return Ok(false);
        }
        if data == "[DONE]" {
            return Ok(true);
        }
        let chunk: SseChunk = serde_json::from_str(data)
            .map_err(|e| AgentError::Serialization(format!("SSE JSON parse error: {e}")))?;

        if let Some(err) = chunk.error.filter(|v| !v.is_null()) {
            self.last_error_code = extract_error_code(&err);
            return Err(AgentError::StreamError(truncate_msg(&err.to_string())));
        }
        if self.id.is_none() {
            self.id = chunk.id;
        }
        if self.model.is_none() {
            self.model = chunk.model;
        }
        if chunk.usage.is_some() {
            self.usage = chunk.usage;
        }
        if let Some(choice) = chunk.choices.into_iter().next() {
            if let Some(reason) = choice.finish_reason.as_deref().filter(|s| !s.is_empty()) {
                if reason == "error" {
                    return Err(AgentError::StreamError(
                        "provider terminated stream with finish_reason=\"error\"".to_string(),
                    ));
                }
                self.finish_reason = Some(reason.to_string());
            }
            if let Some(rd) = choice.delta.reasoning_details {
                merge_reasoning_details(&mut self.reasoning_details, rd);
            }
            if let Some(delta_text) = choice.delta.content
                && !delta_text.is_empty()
            {
                self.emitted_any = true;
                on_delta(&delta_text);
                self.content.push_str(&delta_text);
            }
            if let Some(tcs) = choice.delta.tool_calls {
                if !tcs.is_empty() {
                    self.emitted_any = true;
                }
                for tc in tcs {
                    let slot_idx = self.resolve_tool_slot(tc.index, tc.id.as_deref())?;
                    let slot = self.tool_calls.entry(slot_idx).or_insert_with(|| ToolCall {
                        id: String::new(),
                        kind: "function".to_string(),
                        function: ToolCallFunction {
                            name: String::new(),
                            arguments: String::new(),
                        },
                    });
                    if let Some(id) = tc.id
                        && !id.is_empty()
                    {
                        self.tool_id_slots.insert(id.clone(), slot_idx);
                        slot.id = id;
                    }
                    if let Some(func) = tc.function {
                        if let Some(name) = func.name {
                            slot.function.name.push_str(&name);
                        }
                        if let Some(args) = func.arguments {
                            slot.function.arguments.push_str(&args);
                        }
                    }
                }
            }
        }
        Ok(false)
    }

    fn resolve_tool_slot(
        &mut self,
        index: Option<usize>,
        id: Option<&str>,
    ) -> Result<usize, AgentError> {
        if let Some(idx) = index {
            if idx >= MAX_TOOL_CALL_SLOTS {
                return Err(AgentError::Serialization(format!(
                    "SSE tool_call index {idx} exceeds cap ({MAX_TOOL_CALL_SLOTS})"
                )));
            }
            if let Some(id_str) = id.filter(|s| !s.is_empty()) {
                self.tool_id_slots.insert(id_str.to_string(), idx);
            }
            return Ok(idx);
        }
        if let Some(id_str) = id.filter(|s| !s.is_empty())
            && let Some(&existing) = self.tool_id_slots.get(id_str)
        {
            return Ok(existing);
        }
        // Fallback when provider omits `index`: append to last slot if no new id, else allocate next free slot.
        if id.filter(|s| !s.is_empty()).is_none()
            && let Some((&last_idx, _)) = self.tool_calls.iter().next_back()
        {
            return Ok(last_idx);
        }
        let next = (0..MAX_TOOL_CALL_SLOTS)
            .find(|i| !self.tool_calls.contains_key(i))
            .ok_or_else(|| {
                AgentError::Serialization(format!(
                    "SSE tool_calls exceeded maximum slot count ({MAX_TOOL_CALL_SLOTS})"
                ))
            })?;
        if let Some(id_str) = id.filter(|s| !s.is_empty()) {
            self.tool_id_slots.insert(id_str.to_string(), next);
        }
        Ok(next)
    }

    #[cfg(test)]
    fn finish<F>(mut self, on_delta: &mut F) -> Result<ChatCompletionResponse, AgentError>
    where
        F: FnMut(&str),
    {
        self.finish_inner(on_delta)
    }

    fn finish_inner<F>(&mut self, on_delta: &mut F) -> Result<ChatCompletionResponse, AgentError>
    where
        F: FnMut(&str),
    {
        if !self.event_data.is_empty() {
            let pending = std::mem::take(&mut self.event_data);
            let _ = self.dispatch_event_payload(&pending, on_delta)?;
        }

        if !self.seen_sse_data && !self.raw_non_sse.trim().is_empty() {
            let raw = self.raw_non_sse.trim();
            if raw.starts_with('{') {
                self.last_error_code = extract_json_body_error_code(raw);
                return parse_http200_json_response(raw, on_delta);
            }
        }

        let mut assembled_tools: Vec<ToolCall> = Vec::with_capacity(self.tool_calls.len());
        for (idx, mut tc) in std::mem::take(&mut self.tool_calls) {
            if tc.id.trim().is_empty() {
                tc.id = format!("call_slot_{idx}");
            }
            if tc.function.name.trim().is_empty() {
                return Err(AgentError::StreamError(
                    "stream ended with tool_call missing function name".to_string(),
                ));
            }
            let args_trimmed = tc.function.arguments.trim();
            if !args_trimmed.is_empty()
                && serde_json::from_str::<serde_json::Value>(args_trimmed).is_err()
            {
                return Err(AgentError::StreamError(format!(
                    "truncated or invalid JSON arguments for tool '{}': {}",
                    tc.function.name,
                    truncate_msg(args_trimmed)
                )));
            }
            assembled_tools.push(tc);
        }

        let tool_calls = if assembled_tools.is_empty() {
            None
        } else {
            Some(assembled_tools)
        };
        let content_str = std::mem::take(&mut self.content);
        let content = if content_str.is_empty() && tool_calls.is_some() {
            None
        } else {
            Some(MessageContent::Text(content_str))
        };
        Ok(ChatCompletionResponse {
            id: self.id.take(),
            model: self.model.take(),
            choices: vec![ChatChoice {
                index: 0,
                message: ChatMessage {
                    role: "assistant".to_string(),
                    content,
                    tool_calls,
                    tool_call_id: None,
                    reasoning_details: self.reasoning_details.take(),
                },
                finish_reason: self.finish_reason.take(),
            }],
            usage: self.usage.take(),
        })
    }
}

pub(crate) fn parse_retry_after(headers: &HeaderMap) -> Option<u64> {
    headers
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.trim().parse::<u64>().ok())
}

pub(crate) fn backoff_delay_ms(attempt: u32, retry_after: Option<u64>) -> u64 {
    let base = retry_after
        .map(|s| s.saturating_mul(1000))
        .unwrap_or_else(|| 500u64.saturating_mul(1u64 << attempt.min(4)));
    let jitter = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| (d.subsec_nanos() % 250) as u64)
        .unwrap_or(0);
    base.saturating_add(jitter).min(10_000)
}

async fn sleep_cancellable(
    dur: Duration,
    cancel: Option<&CancellationToken>,
) -> Result<(), AgentError> {
    if let Some(tok) = cancel {
        tokio::select! {
            _ = tok.cancelled() => Err(AgentError::Cancelled),
            _ = tokio::time::sleep(dur) => Ok(()),
        }
    } else {
        tokio::time::sleep(dur).await;
        Ok(())
    }
}

pub(crate) fn truncate_msg(s: &str) -> String {
    if s.chars().count() > 360 {
        let mut out: String = s.chars().take(360).collect();
        out.push_str("...");
        out
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_api_key_in_debug_output() {
        let cfg = OpenRouterConfig {
            api_key: Some("sk-or-v1-supersecret-12345".to_string()),
            ..OpenRouterConfig::default()
        };
        let dbg = format!("{cfg:?}");
        assert!(!dbg.contains("supersecret"));
        assert!(dbg.contains("<redacted>"));
    }

    #[test]
    fn deserializes_openrouter_model_with_null_fields() {
        let raw = r#"{
            "id": "provider/null-fields",
            "name": null,
            "context_length": null,
            "architecture": { "input_modalities": null, "output_modalities": null },
            "pricing": { "prompt": null, "completion": null },
            "supported_parameters": null
        }"#;
        let model: OpenRouterModel = serde_json::from_str(raw).unwrap();
        assert_eq!(model.id, "provider/null-fields");
        assert_eq!(model.name, "");
        assert!(model.supported_parameters.is_empty());
        assert!(model.architecture.unwrap().input_modalities.is_empty());
    }

    #[test]
    fn sse_accumulator_handles_comments_tool_fragments_and_midstream_errors() {
        let mut acc = SseAccumulator::default();
        let mut streamed = String::new();
        let mut on_delta = |d: &str| streamed.push_str(d);

        assert!(
            !acc.feed_line(": OPENROUTER PROCESSING", &mut on_delta)
                .unwrap()
        );
        assert!(
            !acc.feed_line(
                r#"data: {"id":"gen-1","model":"test/model","choices":[{"delta":{"content":"Plan: "}}]}"#,
                &mut on_delta
            )
            .unwrap()
        );
        assert!(
            !acc.feed_line(
                r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_1","function":{"name":"timeline_place","arguments":"{\"track_id\":"}}]}}]}"#,
                &mut on_delta
            )
            .unwrap()
        );
        assert!(
            !acc.feed_line(
                r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"name":"_clip","arguments":"\"v1\"}"}}]},"finish_reason":"tool_calls"}]}"#,
                &mut on_delta
            )
            .unwrap()
        );
        assert!(acc.feed_line("data: [DONE]", &mut on_delta).unwrap());

        let resp = acc.finish(&mut on_delta).unwrap();
        assert_eq!(streamed, "Plan: ");
        let msg = &resp.choices[0].message;
        assert_eq!(msg.content.as_ref().unwrap().as_text(), "Plan: ");
        let tcs = msg.tool_calls.as_ref().unwrap();
        assert_eq!(tcs.len(), 1);
        assert_eq!(tcs[0].function.name, "timeline_place_clip");
        assert_eq!(tcs[0].function.arguments, r#"{"track_id":"v1"}"#);

        let mut err_acc = SseAccumulator::default();
        let err = err_acc.feed_line(
            r#"data: {"error":{"code":502,"message":"Provider overloaded"}}"#,
            &mut |_| {},
        );
        assert!(matches!(err, Err(AgentError::StreamError(_))));
    }

    #[test]
    fn sse_accumulator_handles_multiline_data_missing_index_truncated_args_and_http200_error() {
        // Multi-line SSE data event + missing tool_call index keyed by id
        let mut acc = SseAccumulator::default();
        let mut out = String::new();
        let mut cb = |d: &str| out.push_str(d);
        assert!(
            !acc.feed_line(r#"data: {"choices":[{"delta":"#, &mut cb)
                .unwrap()
        );
        assert!(
            !acc.feed_line(r#"data: {"content":"hi"}}]}"#, &mut cb)
                .unwrap()
        );
        assert!(!acc.feed_line("", &mut cb).unwrap());

        assert!(
            !acc.feed_line(
                r#"data: {"choices":[{"delta":{"tool_calls":[{"id":"c_no_idx","function":{"name":"project_get","arguments":"{\"project_id\":\"p1\"}"}}]}}]}"#,
                &mut cb
            )
            .unwrap()
        );
        let resp = acc.finish(&mut cb).unwrap();
        assert_eq!(out, "hi");
        assert_eq!(
            resp.choices[0].message.tool_calls.as_ref().unwrap().len(),
            1
        );

        // Truncated tool arguments must fail instead of silently passing `{}`
        let mut bad_acc = SseAccumulator::default();
        bad_acc
            .feed_line(
                r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"c1","function":{"name":"project_get","arguments":"{\"project_id\":"}}]}}]}"#,
                &mut |_| {},
            )
            .unwrap();
        assert!(matches!(
            bad_acc.finish(&mut |_| {}),
            Err(AgentError::StreamError(_))
        ));

        // Raw HTTP 200 JSON error body without SSE framing
        let mut json_err_acc = SseAccumulator::default();
        json_err_acc
            .feed_line(
                r#"{"error":{"code":429,"message":"Rate limited"}}"#,
                &mut |_| {},
            )
            .unwrap();
        assert!(matches!(
            json_err_acc.finish(&mut |_| {}),
            Err(AgentError::StreamError(_))
        ));
    }

    #[test]
    fn truncate_msg_handles_multibyte_utf8_and_drain_lines_preserves_split_utf8() {
        let emoji_str = "🔥".repeat(400);
        let truncated = truncate_msg(&emoji_str);
        assert!(truncated.ends_with("..."));
        assert_eq!(truncated.chars().count(), 363);

        let payload = "data: {\"choices\":[{\"delta\":{\"content\":\"olá 🚀\"}}]}\n";
        let bytes = payload.as_bytes();
        let split = bytes.len() - 4; // split inside multi-byte emoji
        let mut buf = Vec::new();
        let mut acc = SseAccumulator::default();
        let mut out = String::new();

        buf.extend_from_slice(&bytes[..split]);
        assert!(!drain_utf8_lines(&mut buf, &mut acc, &mut |d| out.push_str(d)).unwrap());
        assert_eq!(out, "");

        buf.extend_from_slice(&bytes[split..]);
        assert!(!drain_utf8_lines(&mut buf, &mut acc, &mut |d| out.push_str(d)).unwrap());
        assert_eq!(out, "olá 🚀");
    }

    #[test]
    fn cache_control_serializes_ephemeral_and_synthesizes_missing_tool_id_and_reasoning() {
        let cached = ChatMessage::system_cached("System prompt");
        let val = serde_json::to_value(&cached).unwrap();
        assert_eq!(val["content"][0]["type"], "text");
        assert_eq!(val["content"][0]["cache_control"]["type"], "ephemeral");

        let plain = ContentPart::text("hello");
        let plain_val = serde_json::to_value(&plain).unwrap();
        assert!(plain_val.get("cache_control").is_none());

        let mut acc = SseAccumulator::default();
        acc.feed_line(
            r#"data: {"choices":[{"delta":{"reasoning_details":[{"type":"reasoning.text","id":"r1","index":0,"text":"Step 1. "}]}}]}"#,
            &mut |_| {},
        )
        .unwrap();
        acc.feed_line(
            r#"data: {"choices":[{"delta":{"reasoning_details":[{"type":"reasoning.text","id":"r1","index":0,"text":"Step 2."}],"tool_calls":[{"index":0,"function":{"name":"project_get","arguments":"{\"project_id\":\"p1\"}"}}]}}]}"#,
            &mut |_| {},
        )
        .unwrap();
        let resp = acc.finish(&mut |_| {}).unwrap();
        let msg = &resp.choices[0].message;
        let tcs = msg.tool_calls.as_ref().unwrap();
        assert_eq!(tcs[0].id, "call_slot_0");
        let rd = msg.reasoning_details.as_ref().unwrap();
        assert_eq!(rd[0]["text"], "Step 1. Step 2.");

        assert_eq!(
            extract_json_body_error_code(r#"{"error":{"code":503,"message":"overloaded"}}"#),
            Some(503)
        );
        assert_eq!(
            extract_json_body_error_code(r#"{"error":{"code":"429","message":"rate limited"}}"#),
            Some(429)
        );
    }

    #[test]
    fn sse_accumulator_captures_usage_and_reasoning_tokens() {
        let mut acc = SseAccumulator::default();
        acc.feed_line(
            r#"data: {"id":"gen-9","model":"meta/muse-ember","choices":[{"delta":{"content":"{\"ok\":true}"},"finish_reason":"length"}],"usage":{"prompt_tokens":950,"completion_tokens":1400,"total_tokens":2350,"completion_tokens_details":{"reasoning_tokens":1180}}}"#,
            &mut |_| {},
        )
        .unwrap();
        let resp = acc.finish(&mut |_| {}).unwrap();
        assert_eq!(resp.finish_reason(), Some("length"));
        assert_eq!(
            resp.usage_summary(),
            "prompt=950, completion=1400 (reasoning=1180)"
        );

        let req = ChatRequest {
            model: Some("test/model".into()),
            messages: vec![ChatMessage::user_text("hi")],
            reasoning: Some(ReasoningConfig {
                effort: Some("low".into()),
                exclude: None,
            }),
            ..ChatRequest::default()
        };
        let v = serde_json::to_value(&req).unwrap();
        assert_eq!(v["reasoning"]["effort"], "low");
        assert!(v["reasoning"].get("exclude").is_none());
    }

    #[test]
    fn sse_handles_null_choices_delta_empty_and_error_finish_reason_and_sensitive_auth() {
        let mut acc = SseAccumulator::default();
        let mut streamed = String::new();
        let mut on_delta = |d: &str| streamed.push_str(d);

        assert!(
            !acc.feed_line(
                r#"data: {"id":"gen-null","model":"test/m","choices":[{"delta":{"content":"ok"},"finish_reason":"tool_calls"}]}"#,
                &mut on_delta
            )
            .unwrap()
        );
        // Trailing chunk with null delta and empty finish_reason must not abort or overwrite "tool_calls"
        assert!(
            !acc.feed_line(
                r#"data: {"choices":[{"delta":null,"finish_reason":""}]}"#,
                &mut on_delta
            )
            .unwrap()
        );
        // Final usage chunk with "choices": null must deserialize cleanly
        assert!(
            !acc.feed_line(
                r#"data: {"choices":null,"usage":{"prompt_tokens":12,"completion_tokens":4,"total_tokens":16}}"#,
                &mut on_delta
            )
            .unwrap()
        );
        assert!(acc.feed_line("data: [DONE]", &mut on_delta).unwrap());

        let resp = acc.finish(&mut on_delta).unwrap();
        assert_eq!(streamed, "ok");
        assert_eq!(resp.finish_reason(), Some("tool_calls"));
        assert_eq!(resp.usage_summary(), "prompt=12, completion=4");

        // ChatCompletionResponse also tolerates "choices": null
        let null_choices_resp: ChatCompletionResponse =
            serde_json::from_str(r#"{"id":"r1","choices":null}"#).unwrap();
        assert!(null_choices_resp.choices.is_empty());

        // finish_reason == "error" must surface as StreamError
        let mut err_acc = SseAccumulator::default();
        let err = err_acc.feed_line(
            r#"data: {"choices":[{"delta":{},"finish_reason":"error"}]}"#,
            &mut |_| {},
        );
        assert!(matches!(err, Err(AgentError::StreamError(_))));

        // Authorization header must be marked sensitive
        let client = OpenRouterClient::new(OpenRouterConfig {
            api_key: Some("sk-or-test-secret".to_string()),
            ..OpenRouterConfig::default()
        });
        let headers = client.attribution_headers(true).unwrap();
        assert!(headers.get(AUTHORIZATION).unwrap().is_sensitive());
    }
}
