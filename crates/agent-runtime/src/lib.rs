pub mod advisor;
pub mod backend;
pub mod error;
pub mod lint;
pub mod openrouter;
pub mod skills;
pub mod stt;
pub mod swarm;
pub mod tools;

pub use advisor::{ModelAdvisor, ModelCandidate, ModelRecommendation, ModelRoleKind};
pub use backend::{BoxFuture, CapturedFrame, KeyframeResult, SolarisBackend};
pub use error::AgentError;
pub use lint::{GfxLintReport, lint_gfx_html};
pub use openrouter::{
    CacheControl, ChatCompletionResponse, ChatMessage, ChatRequest, ContentPart, InputAudioPayload,
    MediaUrlRef, MessageContent, OpenRouterClient, OpenRouterConfig, OpenRouterModel,
    ProviderPreferences, ToolCall, ToolDefinition,
};
pub use skills::{SkillDoc, get_skill, install_skills_to_dir, list_skills, skills_for_role};
pub use stt::transcribe_audio_openrouter;
pub use swarm::{SwarmRunRequest, run_qa_pass, run_swarm};
pub use tools::{
    ToolAnnotations, ToolOutput, ToolSpec, all_tools, execute_tool, is_mutating_tool,
    tools_for_role,
};
