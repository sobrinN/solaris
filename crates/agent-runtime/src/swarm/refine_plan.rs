use std::sync::Arc;

use serde_json::{Value, json};
use solaris_core_ir::LogLevel;
use tokio_util::sync::CancellationToken;

use super::llm::{DIRECTOR_MAX_TOKENS_INITIAL, DIRECTOR_MAX_TOKENS_RETRY, emit_swarm_log};
use super::refine::{REFINEMENT_SYSTEM_PROMPT, RefinementPlan, parse_refinement_plan};
use super::{SwarmRunRequest, route_models, strip_json_fence};
use crate::advisor::{ModelAdvisor, ModelRoleKind};
use crate::backend::SolarisBackend;
use crate::error::AgentError;
use crate::openrouter::{
    ChatMessage, ChatRequest, MessageContent, OpenRouterClient, ProviderPreferences,
    ReasoningConfig,
};

pub(super) struct RefineModels {
    pub(super) director: String,
    pub(super) director_fallbacks: Vec<String>,
    pub(super) director_json_mode: bool,
    pub(super) director_low_reasoning: bool,
    pub(super) worker: String,
    pub(super) worker_fallbacks: Vec<String>,
    pub(super) worker_low_reasoning: bool,
}

pub(super) async fn resolve_refine_models(
    advisor: &ModelAdvisor,
    req: &SwarmRunRequest,
    cancel: &CancellationToken,
) -> RefineModels {
    let director_rec = advisor
        .recommend_with_cancel(ModelRoleKind::DirectorMultimodal, Some(cancel))
        .await;
    let worker_rec = advisor
        .recommend_with_cancel(ModelRoleKind::SpecialistToolWorker, Some(cancel))
        .await;
    let director = req.model_override.clone().unwrap_or(director_rec.primary);
    let worker = req.model_override.clone().unwrap_or(worker_rec.primary);
    RefineModels {
        director_json_mode: advisor
            .model_supports_parameter(&director, "response_format")
            .await
            || advisor
                .model_supports_parameter(&director, "structured_outputs")
                .await,
        director_low_reasoning: advisor
            .model_supports_parameter(&director, "reasoning")
            .await,
        worker_low_reasoning: advisor.model_supports_parameter(&worker, "reasoning").await,
        director,
        director_fallbacks: director_rec.fallbacks,
        worker,
        worker_fallbacks: worker_rec.fallbacks,
    }
}

struct PlanReply {
    text: String,
    finish: String,
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn build_refinement_plan(
    backend: &Arc<dyn SolarisBackend>,
    openrouter: &OpenRouterClient,
    models: &RefineModels,
    run_id: &str,
    project_id: &str,
    request: &str,
    snapshot: &Value,
    cancel: &CancellationToken,
) -> Result<RefinementPlan, AgentError> {
    let cut = json!({ "request": request, "cut": snapshot }).to_string();
    let mut messages = vec![
        ChatMessage::system(REFINEMENT_SYSTEM_PROMPT),
        ChatMessage::user_text(cut),
    ];
    let first = request_plan(
        backend, openrouter, models, run_id, project_id, &messages, 1, cancel,
    )
    .await?;
    match parse_refinement_plan(strip_json_fence(&first.text)) {
        Ok(plan) => return Ok(plan),
        Err(err) => {
            messages.push(ChatMessage {
                role: "assistant".to_string(),
                content: Some(MessageContent::Text(
                    first.text.chars().take(1200).collect(),
                )),
                tool_calls: None,
                tool_call_id: None,
                reasoning_details: None,
            });
            messages.push(ChatMessage::user_text(format!(
                "Your previous output failed to parse ({err}). Output ONLY one compact, valid, complete raw JSON object with keys summary, target_roles and role_instructions, with no markdown fences."
            )));
        }
    }
    let retry = request_plan(
        backend, openrouter, models, run_id, project_id, &messages, 2, cancel,
    )
    .await?;
    parse_refinement_plan(strip_json_fence(&retry.text)).map_err(|err| {
        AgentError::Serialization(format!(
            "Refinement planner returned malformed JSON after retry (finish={}): {err}",
            retry.finish
        ))
    })
}

#[allow(clippy::too_many_arguments)]
async fn request_plan(
    backend: &Arc<dyn SolarisBackend>,
    openrouter: &OpenRouterClient,
    models: &RefineModels,
    run_id: &str,
    project_id: &str,
    messages: &[ChatMessage],
    attempt: u8,
    cancel: &CancellationToken,
) -> Result<PlanReply, AgentError> {
    let (model, fallbacks) = route_models(&models.director, &models.director_fallbacks);
    let first_attempt = attempt == 1;
    let req = ChatRequest {
        model,
        models: fallbacks,
        messages: messages.to_vec(),
        session_id: Some(format!("solaris-refine-{project_id}")),
        provider: Some(ProviderPreferences {
            allow_fallbacks: Some(true),
            ..ProviderPreferences::default()
        }),
        reasoning: models.director_low_reasoning.then(|| ReasoningConfig {
            effort: Some("low".to_string()),
            ..ReasoningConfig::default()
        }),
        max_tokens: Some(if first_attempt {
            DIRECTOR_MAX_TOKENS_INITIAL
        } else {
            DIRECTOR_MAX_TOKENS_RETRY
        }),
        temperature: Some(if first_attempt { 0.2 } else { 0.1 }),
        response_format: (first_attempt && models.director_json_mode)
            .then(|| json!({ "type": "json_object" })),
        ..ChatRequest::default()
    };
    let resp = match openrouter.chat_stream(&req, Some(cancel), |_| {}).await {
        Ok(resp) => resp,
        Err(err) => {
            if !matches!(err, AgentError::Cancelled) {
                emit_swarm_log(
                    backend.as_ref(),
                    LogLevel::Error,
                    "openrouter",
                    project_id,
                    run_id,
                    Some("director-01"),
                    format!("Refinement planner call failed on attempt {attempt}/2: {err}"),
                    None,
                );
            }
            return Err(err);
        }
    };
    let finish = resp.finish_reason().unwrap_or("unknown").to_string();
    let text = resp
        .choices
        .first()
        .and_then(|choice| choice.message.content.as_ref())
        .map(MessageContent::as_text)
        .unwrap_or_default();
    emit_swarm_log(
        backend.as_ref(),
        LogLevel::Info,
        "openrouter",
        project_id,
        run_id,
        Some("director-01"),
        format!(
            "Refinement planner reply (attempt {attempt}/2, finish={finish}, {})",
            resp.usage_summary()
        ),
        None,
    );
    Ok(PlanReply { text, finish })
}
