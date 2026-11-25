use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, RwLock};
use tokio_util::sync::CancellationToken;

use crate::error::AgentError;
use crate::openrouter::{OpenRouterClient, OpenRouterModel};

const CATALOG_CACHE_TTL: Duration = Duration::from_secs(900); // 15 min
const CATALOG_ERROR_COOLDOWN: Duration = Duration::from_secs(60);

/// Curated roster of vetted high-quality OpenRouter models for Director planning & Vision QA:
/// - Tier 1: Native Omni-Modal (video + audio + image + text + tools)
/// - Tier 2: Fast, low-cost Programmatic / Vision editors (image + text + tools)
const CURATED_DIRECTOR_MODELS: &[&str] = &[
    "google/gemini-4-hydrogen",
    "google/gemini-4-neon",
    "xiaomi/mimo-v2.6-pro",
    "meta/muse-ember",
    "anthropic/claude-haiku-5.5",
    "moonshotai/kimi-k3.1-pro",
    "tencent/hy4",
    "deepseek/deepseek-v4.1-flash-1017",
];

/// Curated roster for parallel specialist tool workers, prioritizing low-latency, low-cost
/// high-reliability tool-callers first while keeping flagship omni-modal models available.
const CURATED_WORKER_MODELS: &[&str] = &[
    "meta/muse-ember",
    "anthropic/claude-haiku-5.5",
    "deepseek/deepseek-v4.1-flash-1017",
    "xiaomi/mimo-v2.6-pro",
    "tencent/hy4",
    "moonshotai/kimi-k3.1-pro",
    "google/gemini-4-hydrogen",
    "google/gemini-4-neon",
];

fn curated_rank(role: ModelRoleKind, model_id: &str) -> Option<usize> {
    let roster = match role {
        ModelRoleKind::DirectorMultimodal => CURATED_DIRECTOR_MODELS,
        ModelRoleKind::SpecialistToolWorker => CURATED_WORKER_MODELS,
        ModelRoleKind::TranscriptionStt => return None,
    };
    roster
        .iter()
        .position(|candidate| candidate.eq_ignore_ascii_case(model_id))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelRoleKind {
    DirectorMultimodal,
    SpecialistToolWorker,
    TranscriptionStt,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelCandidate {
    pub id: String,
    pub name: String,
    pub context_length: u64,
    pub prompt_usd_per_m: Option<f64>,
    pub completion_usd_per_m: Option<f64>,
    pub input_modalities: Vec<String>,
    pub output_modalities: Vec<String>,
    pub supports_tools: bool,
    pub match_tier: String,
    pub score: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelRecommendation {
    pub role: ModelRoleKind,
    pub primary: String,
    pub fallbacks: Vec<String>,
    pub candidates: Vec<ModelCandidate>,
    pub source: String,
    pub fetched_at: DateTime<Utc>,
}

#[derive(Default)]
struct CatalogCache {
    fetched_at: Option<Instant>,
    models: Arc<Vec<OpenRouterModel>>,
    last_error_at: Option<Instant>,
}

pub struct ModelAdvisor {
    client: OpenRouterClient,
    cache: RwLock<CatalogCache>,
    refresh_lock: Mutex<()>,
}

impl ModelAdvisor {
    pub fn new(client: OpenRouterClient) -> Self {
        Self {
            client,
            cache: RwLock::new(CatalogCache::default()),
            refresh_lock: Mutex::new(()),
        }
    }

    pub async fn clear_error_cooldown(&self) {
        let mut guard = self.cache.write().await;
        guard.last_error_at = None;
    }

    pub async fn model_input_modalities(&self, model_id: &str) -> Vec<String> {
        let catalog = self.load_catalog(None).await.ok();
        modalities_from_catalog_or_heuristic(catalog.as_deref().map(Vec::as_slice), model_id)
    }

    pub async fn model_supports_parameter(&self, model_id: &str, param: &str) -> bool {
        self.model_supports_parameter_with_cancel(model_id, param, None)
            .await
    }

    pub async fn model_supports_parameter_with_cancel(
        &self,
        model_id: &str,
        param: &str,
        cancel: Option<&CancellationToken>,
    ) -> bool {
        let Ok(catalog) = self.load_catalog(cancel).await else {
            return false;
        };
        find_catalog_model(&catalog, model_id)
            .is_some_and(|m| m.supported_parameters.iter().any(|p| p == param))
    }

    pub async fn filter_video_capable_models(&self, model_ids: &[String]) -> Vec<String> {
        let catalog = self.load_catalog(None).await.ok();
        model_ids
            .iter()
            .filter(|id| {
                modalities_from_catalog_or_heuristic(catalog.as_deref().map(Vec::as_slice), id)
                    .iter()
                    .any(|m| m == "video")
            })
            .cloned()
            .collect()
    }

    pub async fn filter_audio_capable_models(&self, model_ids: &[String]) -> Vec<String> {
        let catalog = self.load_catalog(None).await.ok();
        model_ids
            .iter()
            .filter(|id| {
                modalities_from_catalog_or_heuristic(catalog.as_deref().map(Vec::as_slice), id)
                    .iter()
                    .any(|m| m == "audio")
            })
            .cloned()
            .collect()
    }

    pub async fn recommend(&self, role: ModelRoleKind) -> ModelRecommendation {
        self.recommend_with_cancel(role, None).await
    }

    pub async fn recommend_with_cancel(
        &self,
        role: ModelRoleKind,
        cancel: Option<&CancellationToken>,
    ) -> ModelRecommendation {
        let cfg = self.client.config();
        let catalog_opt = self
            .load_catalog(cancel)
            .await
            .ok()
            .filter(|m| !m.is_empty());

        if let Some(override_model) = cfg.default_model.as_deref()
            && role != ModelRoleKind::TranscriptionStt
        {
            let ranked = catalog_opt
                .as_deref()
                .map(|m| rank_models(m, role))
                .unwrap_or_default();
            let fallbacks: Vec<String> = if !ranked.is_empty() {
                ranked
                    .iter()
                    .map(|c| c.id.clone())
                    .filter(|id| id != override_model)
                    .take(2)
                    .collect()
            } else {
                default_fallbacks(role)
                    .into_iter()
                    .filter(|f| f != override_model)
                    .collect()
            };
            return ModelRecommendation {
                role,
                primary: override_model.to_string(),
                fallbacks,
                candidates: ranked.into_iter().take(12).collect(),
                source: "env_override".to_string(),
                fetched_at: Utc::now(),
            };
        }

        if let Some(stt_override) = cfg.stt_model.as_deref()
            && role == ModelRoleKind::TranscriptionStt
        {
            let ranked = catalog_opt
                .as_deref()
                .map(|m| rank_models(m, role))
                .unwrap_or_default();
            return ModelRecommendation {
                role,
                primary: stt_override.to_string(),
                fallbacks: ranked
                    .iter()
                    .map(|c| c.id.clone())
                    .filter(|id| id != stt_override)
                    .take(2)
                    .collect(),
                candidates: ranked.into_iter().take(6).collect(),
                source: "env_override".to_string(),
                fetched_at: Utc::now(),
            };
        }

        if let Some(models) = catalog_opt {
            let ranked = rank_models(&models, role);
            if let Some(first) = ranked.first() {
                let primary = first.id.clone();
                let fallbacks = ranked
                    .iter()
                    .skip(1)
                    .take(2)
                    .map(|c| c.id.clone())
                    .collect();
                return ModelRecommendation {
                    role,
                    primary,
                    fallbacks,
                    candidates: ranked.into_iter().take(12).collect(),
                    source: "openrouter_live_catalog".to_string(),
                    fetched_at: Utc::now(),
                };
            }
        }

        let fallbacks = default_fallbacks(role);
        let primary = fallbacks
            .first()
            .cloned()
            .unwrap_or_else(|| "google/gemini-4-hydrogen".to_string());
        ModelRecommendation {
            role,
            primary,
            fallbacks: fallbacks.into_iter().skip(1).collect(),
            candidates: Vec::new(),
            source: "offline_alias_fallback".to_string(),
            fetched_at: Utc::now(),
        }
    }

    async fn load_catalog(
        &self,
        cancel: Option<&CancellationToken>,
    ) -> Result<Arc<Vec<OpenRouterModel>>, AgentError> {
        {
            let guard = self.cache.read().await;
            if let Some(ts) = guard.fetched_at
                && ts.elapsed() < CATALOG_CACHE_TTL
                && !guard.models.is_empty()
            {
                return Ok(Arc::clone(&guard.models));
            }
            if let Some(err_ts) = guard.last_error_at
                && err_ts.elapsed() < CATALOG_ERROR_COOLDOWN
            {
                if !guard.models.is_empty() {
                    return Ok(Arc::clone(&guard.models));
                }
                return Err(AgentError::Transport(
                    "OpenRouter catalog refresh in negative-cache cooldown".to_string(),
                ));
            }
        }

        if cancel.is_some_and(CancellationToken::is_cancelled) {
            return Err(AgentError::Cancelled);
        }

        let _refresh_guard = if let Some(tok) = cancel {
            tokio::select! {
                _ = tok.cancelled() => return Err(AgentError::Cancelled),
                g = self.refresh_lock.lock() => g,
            }
        } else {
            self.refresh_lock.lock().await
        };

        {
            let guard = self.cache.read().await;
            if let Some(ts) = guard.fetched_at
                && ts.elapsed() < CATALOG_CACHE_TTL
                && !guard.models.is_empty()
            {
                return Ok(Arc::clone(&guard.models));
            }
            if let Some(err_ts) = guard.last_error_at
                && err_ts.elapsed() < CATALOG_ERROR_COOLDOWN
            {
                if !guard.models.is_empty() {
                    return Ok(Arc::clone(&guard.models));
                }
                return Err(AgentError::Transport(
                    "OpenRouter catalog refresh in negative-cache cooldown".to_string(),
                ));
            }
        }

        let fetch_fut = self.client.list_models(None, Some("all"), None);
        let fetched = if let Some(tok) = cancel {
            tokio::select! {
                _ = tok.cancelled() => return Err(AgentError::Cancelled),
                r = fetch_fut => r,
            }
        } else {
            fetch_fut.await
        };

        let mut guard = self.cache.write().await;
        match fetched {
            Ok(models) if !models.is_empty() => {
                let arc = Arc::new(models);
                guard.fetched_at = Some(Instant::now());
                guard.last_error_at = None;
                guard.models = Arc::clone(&arc);
                Ok(arc)
            }
            Ok(_) => {
                guard.last_error_at = Some(Instant::now());
                if !guard.models.is_empty() {
                    tracing::warn!(
                        "OpenRouter catalog refresh returned empty list; serving stale cached catalog"
                    );
                    Ok(Arc::clone(&guard.models))
                } else {
                    Err(AgentError::Transport(
                        "OpenRouter catalog returned empty model list".to_string(),
                    ))
                }
            }
            Err(err) => {
                guard.last_error_at = Some(Instant::now());
                if !guard.models.is_empty() {
                    tracing::warn!(
                        "OpenRouter catalog refresh failed ({err}); serving stale cached catalog"
                    );
                    Ok(Arc::clone(&guard.models))
                } else {
                    Err(err)
                }
            }
        }
    }
}

fn default_fallbacks(role: ModelRoleKind) -> Vec<String> {
    match role {
        ModelRoleKind::DirectorMultimodal => vec![
            "google/gemini-4-hydrogen".to_string(),
            "xiaomi/mimo-v2.6-pro".to_string(),
            "meta/muse-ember".to_string(),
        ],
        ModelRoleKind::SpecialistToolWorker => vec![
            "meta/muse-ember".to_string(),
            "anthropic/claude-haiku-5.5".to_string(),
            "deepseek/deepseek-v4.1-flash-1017".to_string(),
        ],
        ModelRoleKind::TranscriptionStt => vec!["openai/whisper-1".to_string()],
    }
}

fn strip_model_variant(model_id: &str) -> &str {
    model_id.split(':').next().unwrap_or(model_id)
}

fn is_model_free_tier(m: &OpenRouterModel) -> bool {
    let prompt_m = price_per_million(m.pricing.as_ref().and_then(|p| p.prompt.as_deref()));
    let completion_m = price_per_million(m.pricing.as_ref().and_then(|p| p.completion.as_deref()));
    is_free_tier_model(&m.id, prompt_m, completion_m)
}

fn find_catalog_model<'a>(
    catalog: &'a [OpenRouterModel],
    model_id: &str,
) -> Option<&'a OpenRouterModel> {
    let base_id = strip_model_variant(model_id);
    catalog
        .iter()
        .find(|m| m.id == model_id)
        .or_else(|| catalog.iter().find(|m| m.id == base_id))
        .or_else(|| {
            catalog.iter().find(|m| {
                strip_model_variant(&m.id) == base_id
                    && !is_batch_variant_model(&m.id)
                    && !is_model_free_tier(m)
            })
        })
        .or_else(|| {
            catalog
                .iter()
                .find(|m| strip_model_variant(&m.id) == base_id)
        })
}

fn modalities_from_catalog_or_heuristic(
    catalog: Option<&[OpenRouterModel]>,
    model_id: &str,
) -> Vec<String> {
    if let Some(models) = catalog
        && let Some(arch) =
            find_catalog_model(models, model_id).and_then(|m| m.architecture.as_ref())
    {
        return arch.input_modalities.clone();
    }
    // Heuristic for offline fallback when live catalog is unavailable
    let lower = strip_model_variant(model_id).to_ascii_lowercase();
    if lower.contains("gemini") || lower.contains("muse-ember") || lower.contains("mimo-v2.6") {
        vec![
            "text".to_string(),
            "image".to_string(),
            "audio".to_string(),
            "video".to_string(),
        ]
    } else {
        vec!["text".to_string(), "image".to_string()]
    }
}

pub fn price_per_million(price_per_token: Option<&str>) -> Option<f64> {
    price_per_token
        .and_then(|s| s.trim().parse::<f64>().ok())
        .filter(|v| v.is_finite() && *v >= 0.0)
        .map(|v| (v * 1_000_000.0 * 1000.0).round() / 1000.0)
}

fn is_free_tier_model(id: &str, prompt_m: Option<f64>, completion_m: Option<f64>) -> bool {
    let lower = id.to_ascii_lowercase();
    if lower == "openrouter/free" || lower.ends_with(":free") || lower.contains(":free") {
        return true;
    }
    matches!((prompt_m, completion_m), (Some(p), Some(c)) if p <= 0.0 && c <= 0.0)
}

fn is_batch_variant_model(id: &str) -> bool {
    id.split(':')
        .skip(1)
        .any(|s| s.eq_ignore_ascii_case("batch"))
}

fn is_expired_model(expiration_date: Option<&str>, now: DateTime<Utc>) -> bool {
    let Some(raw) = expiration_date.map(str::trim).filter(|s| !s.is_empty()) else {
        return false;
    };
    if let Ok(dt) = DateTime::parse_from_rfc3339(raw) {
        return dt.with_timezone(&Utc) <= now;
    }
    if let Ok(date) = chrono::NaiveDate::parse_from_str(raw, "%Y-%m-%d") {
        return date < now.date_naive();
    }
    false
}

pub fn rank_models(models: &[OpenRouterModel], role: ModelRoleKind) -> Vec<ModelCandidate> {
    let now = Utc::now();
    let mut curated_out: Vec<(usize, ModelCandidate)> = Vec::new();
    let mut fallback_out: Vec<ModelCandidate> = Vec::new();
    for m in models {
        if is_expired_model(m.expiration_date.as_deref(), now) || is_batch_variant_model(&m.id) {
            continue;
        }
        let arch = m.architecture.as_ref();
        let inputs = arch.map(|a| a.input_modalities.clone()).unwrap_or_default();
        let outputs = arch
            .map(|a| a.output_modalities.clone())
            .unwrap_or_default();
        let supports_tools = m.supported_parameters.iter().any(|p| p == "tools");
        let outputs_text = outputs.iter().any(|o| o == "text");
        let prompt_m = price_per_million(m.pricing.as_ref().and_then(|p| p.prompt.as_deref()));
        let completion_m =
            price_per_million(m.pricing.as_ref().and_then(|p| p.completion.as_deref()));
        let ctx = m.context_length.unwrap_or(0);
        let has_text = inputs.iter().any(|i| i == "text");
        let has_image = inputs.iter().any(|i| i == "image");
        let has_audio = inputs.iter().any(|i| i == "audio");
        let has_video = inputs.iter().any(|i| i == "video");

        let eligible = match role {
            ModelRoleKind::DirectorMultimodal => supports_tools && outputs_text && has_image,
            ModelRoleKind::SpecialistToolWorker => supports_tools && outputs_text && has_text,
            ModelRoleKind::TranscriptionStt => {
                let id_lower = m.id.to_ascii_lowercase();
                outputs.iter().any(|o| o == "transcription")
                    || (has_audio
                        && (id_lower.contains("whisper") || id_lower.contains("transcribe")))
            }
        };
        if !eligible {
            continue;
        }

        let match_tier = if supports_tools && has_video && has_audio && has_image && has_text {
            "best_match".to_string()
        } else if supports_tools && has_image && has_text {
            "smart_fallback".to_string()
        } else {
            "text_only".to_string()
        };

        let is_free = is_free_tier_model(&m.id, prompt_m, completion_m);
        if !is_free && let Some(rank_idx) = curated_rank(role, &m.id) {
            let curated_score = 200.0 - (rank_idx as f64) * 5.0;
            curated_out.push((
                rank_idx,
                ModelCandidate {
                    id: m.id.clone(),
                    name: m.name.clone(),
                    context_length: ctx,
                    prompt_usd_per_m: prompt_m,
                    completion_usd_per_m: completion_m,
                    input_modalities: inputs,
                    output_modalities: outputs,
                    supports_tools,
                    match_tier,
                    score: curated_score,
                },
            ));
            continue;
        }

        let mut score = 0.0_f64;
        if role == ModelRoleKind::DirectorMultimodal {
            if has_video {
                score += 45.0;
            }
            if has_audio {
                score += 20.0;
            }
            if has_image {
                score += 15.0;
            }
        }
        if role != ModelRoleKind::TranscriptionStt {
            if supports_tools {
                score += 25.0;
            }
            if m.supported_parameters
                .iter()
                .any(|p| p == "structured_outputs" || p == "response_format")
            {
                score += 8.0;
            }
            if m.supported_parameters.iter().any(|p| p == "tool_choice") {
                score += 5.0;
            }
        }
        // Cap context-length contribution at 15 points so weak 1M-context models do not beat frontier tool-callers.
        score += ((ctx as f64).min(1_000_000.0) / 1_000_000.0) * 15.0;
        if !is_free && let Some(p) = prompt_m {
            // Favor practical, fast, cost-effective paid models when capabilities are equal
            score += (15.0 - p.min(15.0)) * 0.4;
        }
        // Heavily penalize free-tier endpoints so reliable paid endpoints win top spot
        if is_free {
            score -= 80.0;
        }

        fallback_out.push(ModelCandidate {
            id: m.id.clone(),
            name: m.name.clone(),
            context_length: ctx,
            prompt_usd_per_m: prompt_m,
            completion_usd_per_m: completion_m,
            input_modalities: inputs,
            output_modalities: outputs,
            supports_tools,
            match_tier,
            score,
        });
    }

    // When curated models are available in the catalog, recommend ONLY vetted models
    // (preventing noisy/legacy/experimental catalog entries from polluting Solaris's advisor).
    if !curated_out.is_empty() {
        curated_out.sort_by(|(ra, a), (rb, b)| ra.cmp(rb).then_with(|| a.id.cmp(&b.id)));
        return curated_out.into_iter().map(|(_, c)| c).collect();
    }

    // If non-free candidates exist, keep free-tier candidates strictly below all paid models
    let has_non_free = fallback_out
        .iter()
        .any(|c| !is_free_tier_model(&c.id, c.prompt_usd_per_m, c.completion_usd_per_m));
    fallback_out.sort_by(|a, b| {
        if has_non_free {
            let a_free = is_free_tier_model(&a.id, a.prompt_usd_per_m, a.completion_usd_per_m);
            let b_free = is_free_tier_model(&b.id, b.prompt_usd_per_m, b.completion_usd_per_m);
            if a_free != b_free {
                return a_free.cmp(&b_free);
            }
        }
        b.score.total_cmp(&a.score).then_with(|| a.id.cmp(&b.id))
    });
    fallback_out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openrouter::{ModelArchitecture, ModelPricing, OpenRouterConfig};

    #[test]
    fn converts_prices_and_ranks_multimodal_video_tool_models_first() {
        let models = vec![
            OpenRouterModel {
                id: "text/only-tools".to_string(),
                name: "Text Only".to_string(),
                context_length: Some(128_000),
                architecture: Some(ModelArchitecture {
                    input_modalities: vec!["text".to_string()],
                    output_modalities: vec!["text".to_string()],
                }),
                pricing: Some(ModelPricing {
                    prompt: Some("0.000001".to_string()),
                    completion: Some("0.000002".to_string()),
                }),
                supported_parameters: vec!["tools".to_string()],
                expiration_date: None,
            },
            OpenRouterModel {
                id: "google/omni-video:free".to_string(),
                name: "Omni Free".to_string(),
                context_length: Some(1_000_000),
                architecture: Some(ModelArchitecture {
                    input_modalities: vec![
                        "text".to_string(),
                        "image".to_string(),
                        "audio".to_string(),
                        "video".to_string(),
                    ],
                    output_modalities: vec!["text".to_string()],
                }),
                pricing: Some(ModelPricing {
                    prompt: Some("0".to_string()),
                    completion: Some("0".to_string()),
                }),
                supported_parameters: vec!["tools".to_string()],
                expiration_date: None,
            },
            OpenRouterModel {
                id: "openrouter/free".to_string(),
                name: "OpenRouter Free Router".to_string(),
                context_length: Some(1_000_000),
                architecture: Some(ModelArchitecture {
                    input_modalities: vec!["text".to_string(), "image".to_string()],
                    output_modalities: vec!["text".to_string()],
                }),
                pricing: Some(ModelPricing {
                    prompt: Some("0".to_string()),
                    completion: Some("0".to_string()),
                }),
                supported_parameters: vec!["tools".to_string()],
                expiration_date: None,
            },
            OpenRouterModel {
                id: "legacy/expired-omni".to_string(),
                name: "Expired Omni".to_string(),
                context_length: Some(1_000_000),
                architecture: Some(ModelArchitecture {
                    input_modalities: vec![
                        "text".to_string(),
                        "image".to_string(),
                        "audio".to_string(),
                        "video".to_string(),
                    ],
                    output_modalities: vec!["text".to_string()],
                }),
                pricing: Some(ModelPricing {
                    prompt: Some("0.0000001".to_string()),
                    completion: Some("0.0000002".to_string()),
                }),
                supported_parameters: vec!["tools".to_string()],
                expiration_date: Some("2020-01-01".to_string()),
            },
            OpenRouterModel {
                id: "image/gen-only".to_string(),
                name: "Image Generator".to_string(),
                context_length: Some(64_000),
                architecture: Some(ModelArchitecture {
                    input_modalities: vec!["text".to_string(), "image".to_string()],
                    output_modalities: vec!["image".to_string()],
                }),
                pricing: None,
                supported_parameters: vec!["tools".to_string()],
                expiration_date: None,
            },
            OpenRouterModel {
                id: "google/omni-video".to_string(),
                name: "Omni Video".to_string(),
                context_length: Some(1_000_000),
                architecture: Some(ModelArchitecture {
                    input_modalities: vec![
                        "text".to_string(),
                        "image".to_string(),
                        "audio".to_string(),
                        "video".to_string(),
                    ],
                    output_modalities: vec!["text".to_string()],
                }),
                pricing: Some(ModelPricing {
                    prompt: Some("0.0000005".to_string()),
                    completion: Some("0.000003".to_string()),
                }),
                supported_parameters: vec!["tools".to_string()],
                expiration_date: None,
            },
            OpenRouterModel {
                id: "openai/whisper-large-v3".to_string(),
                name: "Whisper".to_string(),
                context_length: Some(0),
                architecture: Some(ModelArchitecture {
                    input_modalities: vec!["audio".to_string()],
                    output_modalities: vec!["transcription".to_string()],
                }),
                pricing: None,
                supported_parameters: vec![],
                expiration_date: None,
            },
        ];

        let dir_ranked = rank_models(&models, ModelRoleKind::DirectorMultimodal);
        assert_eq!(dir_ranked.len(), 3);
        // Paid omni-video wins over `:free` and `openrouter/free`, and expired/image-only models are excluded
        assert_eq!(dir_ranked[0].id, "google/omni-video");
        assert_eq!(dir_ranked[0].match_tier, "best_match");
        assert_eq!(dir_ranked[1].id, "google/omni-video:free");
        assert_eq!(dir_ranked[2].id, "openrouter/free");
        assert_eq!(dir_ranked[2].match_tier, "smart_fallback");
        assert_eq!(dir_ranked[0].prompt_usd_per_m, Some(0.5));
        assert_eq!(dir_ranked[0].completion_usd_per_m, Some(3.0));

        let worker_ranked = rank_models(&models, ModelRoleKind::SpecialistToolWorker);
        assert_eq!(worker_ranked.len(), 4);
        assert!(!worker_ranked.iter().any(|c| c.id == "image/gen-only"));
        assert!(!worker_ranked.iter().any(|c| c.id == "legacy/expired-omni"));
        assert_eq!(
            worker_ranked
                .iter()
                .find(|c| c.id == "text/only-tools")
                .unwrap()
                .match_tier,
            "text_only"
        );

        let stt_ranked = rank_models(&models, ModelRoleKind::TranscriptionStt);
        assert_eq!(stt_ranked.len(), 1);
        assert_eq!(stt_ranked[0].id, "openai/whisper-large-v3");

        // Variant suffix stripping resolves catalog modalities accurately
        let variant_mods =
            modalities_from_catalog_or_heuristic(Some(&models), "google/omni-video:nitro");
        assert!(variant_mods.iter().any(|m| m == "video"));

        let _advisor = ModelAdvisor::new(OpenRouterClient::new(OpenRouterConfig::default()));
    }

    #[test]
    fn rank_models_excludes_batch_variants() {
        let models = vec![
            OpenRouterModel {
                id: "z-ai/glm-5.3-flash:batch".to_string(),
                name: "GLM Flash Batch".to_string(),
                context_length: Some(1_000_000),
                architecture: Some(ModelArchitecture {
                    input_modalities: vec!["text".to_string(), "image".to_string()],
                    output_modalities: vec!["text".to_string()],
                }),
                pricing: Some(ModelPricing {
                    prompt: Some("0.0000001".to_string()),
                    completion: Some("0.0000002".to_string()),
                }),
                supported_parameters: vec![
                    "tools".to_string(),
                    "response_format".to_string(),
                    "tool_choice".to_string(),
                ],
                expiration_date: None,
            },
            OpenRouterModel {
                id: "qwen/qwen3-coder".to_string(),
                name: "Qwen Coder".to_string(),
                context_length: Some(256_000),
                architecture: Some(ModelArchitecture {
                    input_modalities: vec!["text".to_string()],
                    output_modalities: vec!["text".to_string()],
                }),
                pricing: Some(ModelPricing {
                    prompt: Some("0.0000005".to_string()),
                    completion: Some("0.000001".to_string()),
                }),
                supported_parameters: vec!["tools".to_string()],
                expiration_date: None,
            },
        ];
        let ranked = rank_models(&models, ModelRoleKind::SpecialistToolWorker);
        assert_eq!(ranked.len(), 1);
        assert_eq!(ranked[0].id, "qwen/qwen3-coder");
    }

    #[test]
    fn variant_lookup_prioritizes_exact_then_base_then_non_batch_non_free() {
        let catalog = vec![
            OpenRouterModel {
                id: "acme/vision:batch".to_string(),
                name: "Vision Batch".to_string(),
                context_length: Some(128_000),
                architecture: Some(ModelArchitecture {
                    input_modalities: vec!["text".to_string()],
                    output_modalities: vec!["text".to_string()],
                }),
                pricing: Some(ModelPricing {
                    prompt: Some("0.000001".to_string()),
                    completion: Some("0.000002".to_string()),
                }),
                supported_parameters: vec![],
                expiration_date: None,
            },
            OpenRouterModel {
                id: "acme/vision:free".to_string(),
                name: "Vision Free".to_string(),
                context_length: Some(128_000),
                architecture: Some(ModelArchitecture {
                    input_modalities: vec!["text".to_string()],
                    output_modalities: vec!["text".to_string()],
                }),
                pricing: Some(ModelPricing {
                    prompt: Some("0".to_string()),
                    completion: Some("0".to_string()),
                }),
                supported_parameters: vec![],
                expiration_date: None,
            },
            OpenRouterModel {
                id: "acme/vision".to_string(),
                name: "Vision Base".to_string(),
                context_length: Some(128_000),
                architecture: Some(ModelArchitecture {
                    input_modalities: vec![
                        "text".to_string(),
                        "image".to_string(),
                        "video".to_string(),
                    ],
                    output_modalities: vec!["text".to_string()],
                }),
                pricing: Some(ModelPricing {
                    prompt: Some("0.000001".to_string()),
                    completion: Some("0.000002".to_string()),
                }),
                supported_parameters: vec!["reasoning".to_string(), "tools".to_string()],
                expiration_date: None,
            },
            OpenRouterModel {
                id: "beta/only-variants:batch".to_string(),
                name: "Beta Batch".to_string(),
                context_length: Some(64_000),
                architecture: Some(ModelArchitecture {
                    input_modalities: vec!["text".to_string()],
                    output_modalities: vec!["text".to_string()],
                }),
                pricing: None,
                supported_parameters: vec![],
                expiration_date: None,
            },
            OpenRouterModel {
                id: "beta/only-variants:free".to_string(),
                name: "Beta Free".to_string(),
                context_length: Some(64_000),
                architecture: Some(ModelArchitecture {
                    input_modalities: vec!["text".to_string()],
                    output_modalities: vec!["text".to_string()],
                }),
                pricing: Some(ModelPricing {
                    prompt: Some("0".to_string()),
                    completion: Some("0".to_string()),
                }),
                supported_parameters: vec![],
                expiration_date: None,
            },
            OpenRouterModel {
                id: "beta/only-variants:exacto".to_string(),
                name: "Beta Exacto".to_string(),
                context_length: Some(64_000),
                architecture: Some(ModelArchitecture {
                    input_modalities: vec!["text".to_string(), "audio".to_string()],
                    output_modalities: vec!["text".to_string()],
                }),
                pricing: Some(ModelPricing {
                    prompt: Some("0.000002".to_string()),
                    completion: Some("0.000004".to_string()),
                }),
                supported_parameters: vec!["reasoning".to_string()],
                expiration_date: None,
            },
        ];

        // Even though `:batch` appears first, querying `:nitro` must resolve to the base model
        let resolved = find_catalog_model(&catalog, "acme/vision:nitro").unwrap();
        assert_eq!(resolved.id, "acme/vision");
        assert!(
            resolved
                .supported_parameters
                .iter()
                .any(|p| p == "reasoning")
        );
        let mods = modalities_from_catalog_or_heuristic(Some(&catalog), "acme/vision:nitro");
        assert!(mods.iter().any(|m| m == "video"));

        // Exact match still wins when explicitly querying `:batch`
        assert_eq!(
            find_catalog_model(&catalog, "acme/vision:batch")
                .unwrap()
                .id,
            "acme/vision:batch"
        );

        // When base is absent, prefer non-batch & non-free variant over `:batch` and `:free`
        let beta_resolved = find_catalog_model(&catalog, "beta/only-variants:nitro").unwrap();
        assert_eq!(beta_resolved.id, "beta/only-variants:exacto");
    }

    #[test]
    fn is_batch_variant_model_matches_only_colon_delimited_batch_segment() {
        assert!(is_batch_variant_model("provider/model:batch"));
        assert!(is_batch_variant_model("provider/model:BATCH"));
        assert!(is_batch_variant_model("provider/model:nitro:batch"));
        assert!(!is_batch_variant_model("provider/model:batcher"));
        assert!(!is_batch_variant_model("provider/batch-model"));
        assert!(!is_batch_variant_model("batch/model"));
    }

    #[test]
    fn stt_ranking_ignores_chat_tool_bonuses_and_tie_breaks_by_id() {
        let models = vec![
            OpenRouterModel {
                id: "z-vendor/whisper-b".to_string(),
                name: "Whisper B".to_string(),
                context_length: Some(0),
                architecture: Some(ModelArchitecture {
                    input_modalities: vec!["audio".to_string()],
                    output_modalities: vec!["transcription".to_string()],
                }),
                pricing: None,
                supported_parameters: vec![
                    "tools".to_string(),
                    "response_format".to_string(),
                    "tool_choice".to_string(),
                ],
                expiration_date: None,
            },
            OpenRouterModel {
                id: "a-vendor/whisper-a".to_string(),
                name: "Whisper A".to_string(),
                context_length: Some(0),
                architecture: Some(ModelArchitecture {
                    input_modalities: vec!["audio".to_string()],
                    output_modalities: vec!["transcription".to_string()],
                }),
                pricing: None,
                supported_parameters: vec![],
                expiration_date: None,
            },
        ];
        let ranked = rank_models(&models, ModelRoleKind::TranscriptionStt);
        assert_eq!(ranked.len(), 2);
        // Same score because STT does not award chat-tool bonuses, so tie-breaker sorts by id ascending
        assert!((ranked[0].score - ranked[1].score).abs() < 1e-9);
        assert_eq!(ranked[0].id, "a-vendor/whisper-a");
        assert_eq!(ranked[1].id, "z-vendor/whisper-b");
    }

    #[test]
    fn rank_models_curates_only_vetted_omni_and_programmatic_models() {
        let mk = |id: &str, inputs: &[&str], prompt: &str| OpenRouterModel {
            id: id.to_string(),
            name: id.to_string(),
            context_length: Some(1_048_576),
            architecture: Some(ModelArchitecture {
                input_modalities: inputs.iter().map(|s| (*s).to_string()).collect(),
                output_modalities: vec!["text".to_string()],
            }),
            pricing: Some(ModelPricing {
                prompt: Some(prompt.to_string()),
                completion: Some("0.000001".to_string()),
            }),
            supported_parameters: vec![
                "tools".to_string(),
                "tool_choice".to_string(),
                "response_format".to_string(),
            ],
            expiration_date: None,
        };

        let catalog = vec![
            // Noisy / experimental / low-quality / batch models that must be excluded
            mk(
                "xiaomi/mimo-v2.6-flash-mopd",
                &["text", "image", "video", "audio"],
                "0.00000001",
            ),
            mk(
                "z-ai/glm-5.3-flashx-legal",
                &["text", "image", "video"],
                "0.00000005",
            ),
            mk(
                "meta/muse-spark-1.2-contributor",
                &["text", "image", "video", "audio"],
                "0.00000001",
            ),
            mk(
                "google/gemini-4-neon:batch",
                &["text", "image", "video", "audio"],
                "0.0000015",
            ),
            // Vetted 4 Omni-Modal + 4 Cheap Programmatic/Vision models (intentionally shuffled)
            mk(
                "deepseek/deepseek-v4.1-flash-1017",
                &["text", "image"],
                "0.00000015",
            ),
            mk("tencent/hy4", &["text", "image"], "0.000000417"),
            mk(
                "moonshotai/kimi-k3.1-pro",
                &["text", "image", "video"],
                "0.00000024",
            ),
            mk(
                "anthropic/claude-haiku-5.5",
                &["text", "image"],
                "0.0000001",
            ),
            mk(
                "meta/muse-ember",
                &["text", "image", "video", "audio"],
                "0.0000001",
            ),
            mk(
                "xiaomi/mimo-v2.6-pro",
                &["text", "image", "video", "audio"],
                "0.000000435",
            ),
            mk(
                "google/gemini-4-neon",
                &["text", "image", "video", "audio"],
                "0.000003",
            ),
            mk(
                "google/gemini-4-hydrogen",
                &["text", "image", "video", "audio"],
                "0.0000015",
            ),
        ];

        let dir = rank_models(&catalog, ModelRoleKind::DirectorMultimodal);
        let dir_ids: Vec<&str> = dir.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(
            dir_ids,
            vec![
                "google/gemini-4-hydrogen",
                "google/gemini-4-neon",
                "xiaomi/mimo-v2.6-pro",
                "meta/muse-ember",
                "anthropic/claude-haiku-5.5",
                "moonshotai/kimi-k3.1-pro",
                "tencent/hy4",
                "deepseek/deepseek-v4.1-flash-1017",
            ]
        );
        // First 4 are omni-modal (`best_match`), next 4 are programmatic vision (`smart_fallback`)
        assert!(dir[..4].iter().all(|c| c.match_tier == "best_match"));
        assert!(dir[4..].iter().all(|c| c.match_tier == "smart_fallback"));

        let worker = rank_models(&catalog, ModelRoleKind::SpecialistToolWorker);
        let worker_ids: Vec<&str> = worker.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(
            worker_ids,
            vec![
                "meta/muse-ember",
                "anthropic/claude-haiku-5.5",
                "deepseek/deepseek-v4.1-flash-1017",
                "xiaomi/mimo-v2.6-pro",
                "tencent/hy4",
                "moonshotai/kimi-k3.1-pro",
                "google/gemini-4-hydrogen",
                "google/gemini-4-neon",
            ]
        );
    }
}
