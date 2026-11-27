pub mod advisor;
pub mod backend;
pub mod error;
pub mod lint;
pub mod openrouter;
pub mod planner;
pub mod skills;
pub mod stt;
pub mod swarm;
pub mod tools;

pub use advisor::{ModelAdvisor, ModelCandidate, ModelRecommendation, ModelRoleKind};
pub use backend::{BoxFuture, CapturedFrame, KeyframeResult, SolarisBackend};
pub use error::AgentError;
pub use lint::{GfxLintReport, lint_gfx_html};
pub use openrouter::{
    CacheControl, ChatCompletionResponse, ChatMessage, ChatRequest, ChatUsage,
    CompletionTokensDetails, ContentPart, InputAudioPayload, MediaUrlRef, MessageContent,
    OpenRouterClient, OpenRouterConfig, OpenRouterModel, ProviderPreferences, ReasoningConfig,
    ToolCall, ToolDefinition,
};
pub use planner::{
    CreativePlan, CreativePlanBeat, CreativePlanInput, build_local_creative_plan,
    generate_creative_plan,
};
pub use skills::{
    SkillDoc, SkillResourceDoc, get_skill, get_skill_resource, install_skills_to_dir,
    list_skill_resources, list_skills, skill_references_for_role, skills_for_role,
};
pub use stt::transcribe_audio_openrouter;
pub use swarm::{
    QaPassOptions, SwarmRunRequest, format_qa_diagnostics, prepare_swarm_request, run_qa_pass,
    run_qa_pass_with_options, run_swarm,
};
pub use tools::{
    ToolAnnotations, ToolOutput, ToolSpec, all_tools, execute_tool, is_mutating_tool,
    tools_for_role,
};
