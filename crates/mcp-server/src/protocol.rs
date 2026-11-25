use std::sync::{Arc, RwLock};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use solaris_agent_runtime::{
    SolarisBackend, all_tools, execute_tool, get_skill, get_skill_resource, list_skill_resources,
    list_skills, tools::normalize_tool_name,
};
use solaris_core_ir::{AgentRole, CommandMeta};

pub const DEFAULT_PROTOCOL_VERSION: &str = "2025-06-18";
const SUPPORTED_PROTOCOL_VERSIONS: &[&str] = &["2024-11-05", "2025-03-26", "2025-06-18"];

fn supports_structured_content(version: &str) -> bool {
    version >= "2025-06-18"
}

fn is_valid_rpc_id(val: &Value) -> bool {
    matches!(val, Value::String(_) | Value::Number(_))
}

#[derive(Debug, Clone, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    #[serde(default)]
    pub id: Option<Value>,
    pub method: String,
    #[serde(default)]
    pub params: Option<Value>,
}

#[derive(Debug, Clone, Serialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: &'static str,
    pub id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

#[derive(Debug, Clone, Serialize)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
}

impl JsonRpcResponse {
    fn ok(id: Value, result: Value) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: Some(result),
            error: None,
        }
    }

    fn err(id: Value, code: i32, message: impl Into<String>) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: None,
            error: Some(JsonRpcError {
                code,
                message: message.into(),
            }),
        }
    }
}

#[derive(Clone)]
pub struct McpServer {
    backend: Arc<dyn SolarisBackend>,
    protocol_version: Arc<RwLock<String>>,
}

impl McpServer {
    pub fn new(backend: Arc<dyn SolarisBackend>) -> Self {
        Self {
            backend,
            protocol_version: Arc::new(RwLock::new(DEFAULT_PROTOCOL_VERSION.to_string())),
        }
    }

    pub fn protocol_version(&self) -> String {
        self.protocol_version
            .read()
            .map(|g| g.clone())
            .unwrap_or_else(|_| DEFAULT_PROTOCOL_VERSION.to_string())
    }

    pub fn set_protocol_version(&self, version: &str) {
        if let Ok(mut g) = self.protocol_version.write() {
            *g = version.to_string();
        }
    }

    pub async fn handle_json_str(&self, raw: &str) -> Option<JsonRpcResponse> {
        self.handle_json_str_with_version(raw, None).await
    }

    pub async fn handle_json_str_with_version(
        &self,
        raw: &str,
        protocol_version: Option<&str>,
    ) -> Option<JsonRpcResponse> {
        let raw_val: Value = match serde_json::from_str(raw) {
            Ok(v) => v,
            Err(e) => {
                return Some(JsonRpcResponse::err(
                    Value::Null,
                    -32700,
                    format!("Parse error: {e}"),
                ));
            }
        };
        self.handle_json_rpc_with_version(raw_val, protocol_version)
            .await
    }

    pub async fn handle_json_rpc(&self, req: Value) -> Option<JsonRpcResponse> {
        self.handle_json_rpc_with_version(req, None).await
    }

    pub async fn handle_json_rpc_with_version(
        &self,
        raw_val: Value,
        protocol_version: Option<&str>,
    ) -> Option<JsonRpcResponse> {
        let Some(obj) = raw_val.as_object() else {
            return Some(JsonRpcResponse::err(
                Value::Null,
                -32600,
                "Invalid Request: JSON-RPC payload must be an object",
            ));
        };

        // Distinguish missing "id" (notification) vs explicit "id": null (invalid request per JSON-RPC 2.0 / MCP)
        let explicit_id = obj.get("id");
        if let Some(id_val) = explicit_id
            && !is_valid_rpc_id(id_val)
        {
            return Some(JsonRpcResponse::err(
                Value::Null,
                -32600,
                "Invalid Request: 'id' must be a string or number and must not be null",
            ));
        }

        let jsonrpc = obj.get("jsonrpc").and_then(|v| v.as_str());
        if jsonrpc != Some("2.0") {
            let id = explicit_id.cloned().unwrap_or(Value::Null);
            return Some(JsonRpcResponse::err(
                id,
                -32600,
                "Invalid Request: jsonrpc must be '2.0'",
            ));
        }

        let Some(method) = obj
            .get("method")
            .and_then(|v| v.as_str())
            .filter(|m| !m.is_empty())
        else {
            let id = explicit_id.cloned().unwrap_or(Value::Null);
            return Some(JsonRpcResponse::err(
                id,
                -32600,
                "Invalid Request: missing or non-string 'method'",
            ));
        };

        let params = obj.get("params").cloned();
        if let Some(ref p) = params
            && !p.is_object()
            && !p.is_array()
            && !p.is_null()
        {
            let id = explicit_id.cloned().unwrap_or(Value::Null);
            return Some(JsonRpcResponse::err(
                id,
                -32600,
                "Invalid Request: 'params' must be a structured value",
            ));
        }

        let req = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: explicit_id.cloned(),
            method: method.to_string(),
            params,
        };
        self.handle_request_with_version(req, protocol_version)
            .await
    }

    pub async fn handle_request(&self, req: JsonRpcRequest) -> Option<JsonRpcResponse> {
        self.handle_request_with_version(req, None).await
    }

    pub async fn handle_request_with_version(
        &self,
        req: JsonRpcRequest,
        protocol_version: Option<&str>,
    ) -> Option<JsonRpcResponse> {
        if req.jsonrpc != "2.0" {
            let id = req.id.unwrap_or(Value::Null);
            return Some(JsonRpcResponse::err(
                id,
                -32600,
                "Invalid Request: jsonrpc must be '2.0'",
            ));
        }

        if let Some(ref id_val) = req.id
            && !is_valid_rpc_id(id_val)
        {
            return Some(JsonRpcResponse::err(
                Value::Null,
                -32600,
                "Invalid Request: 'id' must be a string or number and must not be null",
            ));
        }

        // Notifications have no id and expect no JSON-RPC response body.
        let id = req.id?;

        let stored_version = self.protocol_version();
        let effective_version = protocol_version.unwrap_or(&stored_version);

        let params = req.params.unwrap_or_else(|| json!({}));
        let res = match req.method.as_str() {
            "initialize" => {
                let requested = params
                    .get("protocolVersion")
                    .and_then(|v| v.as_str())
                    .unwrap_or(DEFAULT_PROTOCOL_VERSION);
                let negotiated = if SUPPORTED_PROTOCOL_VERSIONS.contains(&requested) {
                    requested
                } else {
                    DEFAULT_PROTOCOL_VERSION
                };
                // Only mutate shared server version when running stateful stdio without explicit per-request version.
                if protocol_version.is_none() {
                    self.set_protocol_version(negotiated);
                }
                Ok(json!({
                    "protocolVersion": negotiated,
                    "capabilities": {
                        "tools": { "listChanged": false },
                        "resources": { "subscribe": false, "listChanged": false },
                        "prompts": { "listChanged": false }
                    },
                    "serverInfo": {
                        "name": "solaris-mcp",
                        "version": env!("CARGO_PKG_VERSION")
                    }
                }))
            }
            "ping" => Ok(json!({})),
            "tools/list" => Ok(self.list_tools()),
            "tools/call" => self.call_tool(params, effective_version).await,
            "resources/list" => Ok(self.list_resources()),
            "resources/templates/list" => Ok(self.list_resource_templates()),
            "resources/read" => self.read_resource(params).await,
            "prompts/list" => Ok(self.list_prompts()),
            "prompts/get" => self.get_prompt(params),
            other => Err((-32601, format!("Method not found: {other}"))),
        };

        Some(match res {
            Ok(val) => JsonRpcResponse::ok(id, val),
            Err((code, msg)) => JsonRpcResponse::err(id, code, msg),
        })
    }

    fn list_tools(&self) -> Value {
        let tools: Vec<Value> = all_tools()
            .into_iter()
            .map(|t| {
                json!({
                    "name": t.mcp_name,
                    "description": t.description,
                    "inputSchema": t.input_schema,
                    "annotations": t.annotations,
                })
            })
            .collect();
        json!({ "tools": tools })
    }

    async fn call_tool(
        &self,
        params: Value,
        protocol_version: &str,
    ) -> Result<Value, (i32, String)> {
        let name = params
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or((-32602, "Missing tool 'name'".to_string()))?;
        let norm_name = normalize_tool_name(name.trim());
        if !all_tools().iter().any(|t| t.mcp_name == norm_name) {
            return Err((-32602, format!("Unknown tool: {name}")));
        }
        let args = params
            .get("arguments")
            .cloned()
            .unwrap_or_else(|| json!({}));

        if let Some(pid) = args.get("project_id").and_then(|v| v.as_str()) {
            let anchor = anchor_for_tool(norm_name);
            let _ = self
                .backend
                .emit_cursor_pulse(
                    pid,
                    "mcp-external",
                    AgentRole::McpExternal,
                    anchor,
                    norm_name,
                )
                .await;
        }

        let meta = CommandMeta {
            agent_id: "mcp-external".to_string(),
            role: AgentRole::McpExternal,
            reason: Some(format!("mcp:{norm_name}")),
        };

        let include_structured = supports_structured_content(protocol_version);

        match execute_tool(self.backend.as_ref(), norm_name, args, meta).await {
            Ok(out) => {
                let mut content = vec![json!({
                    "type": "text",
                    "text": out.text,
                })];
                for png_b64 in out.all_images_base64() {
                    content.push(json!({
                        "type": "image",
                        "mimeType": "image/png",
                        "data": png_b64,
                    }));
                }
                let mut payload = serde_json::Map::new();
                payload.insert("content".to_string(), Value::Array(content));
                if include_structured {
                    payload.insert("structuredContent".to_string(), out.structured);
                }
                payload.insert("isError".to_string(), Value::Bool(false));
                Ok(Value::Object(payload))
            }
            Err(err) => Ok(json!({
                "content": [{ "type": "text", "text": err.to_string() }],
                "isError": true,
            })),
        }
    }

    fn list_resources(&self) -> Value {
        let mut resources: Vec<Value> = list_skills()
            .into_iter()
            .map(|s| {
                json!({
                    "uri": s.uri,
                    "name": s.name,
                    "description": s.description,
                    "mimeType": "text/markdown",
                })
            })
            .collect();

        for r in list_skill_resources() {
            resources.push(json!({
                "uri": r.uri,
                "name": format!("{}/{}", r.skill_name, r.relative_path),
                "description": r.description,
                "mimeType": r.mime_type,
            }));
        }

        resources.push(json!({
            "uri": "solaris://projects",
            "name": "Solaris Projects",
            "description": "List of all projects in the Solaris workspace",
            "mimeType": "application/json",
        }));
        resources.push(json!({
            "uri": "solaris://brands",
            "name": "Solaris Brand Playbooks",
            "description": "All configured Brand Playbooks and design tokens",
            "mimeType": "application/json",
        }));
        resources.push(json!({
            "uri": "solaris://templates",
            "name": "Solaris Starter Templates",
            "description": "Built-in Talking-Head Short and Faceless Explainer project templates",
            "mimeType": "application/json",
        }));
        resources.push(json!({
            "uri": "solaris://assets/pack",
            "name": "Solaris Workspace Asset Pack",
            "description": "Portable solaris.asset-pack/v1 JSON manifest of all saved GFX assets",
            "mimeType": "application/json",
        }));
        json!({ "resources": resources })
    }

    fn list_resource_templates(&self) -> Value {
        json!({
            "resourceTemplates": [
                {
                    "uriTemplate": "solaris://project/{project_id}",
                    "name": "Solaris Project State",
                    "description": "Live Timeline IR JSON for a specific Solaris project",
                    "mimeType": "application/json"
                }
            ]
        })
    }

    async fn read_resource(&self, params: Value) -> Result<Value, (i32, String)> {
        let uri = params
            .get("uri")
            .and_then(|v| v.as_str())
            .ok_or((-32602, "Missing resource 'uri'".to_string()))?;

        if uri.starts_with("skill://") {
            let doc =
                get_skill_resource(uri).ok_or((-32002, format!("Resource not found: {uri}")))?;
            return Ok(json!({
                "contents": [{
                    "uri": doc.uri,
                    "mimeType": doc.mime_type,
                    "text": doc.content,
                }]
            }));
        }

        if uri == "solaris://projects" {
            let list = self
                .backend
                .list_projects()
                .await
                .map_err(|e| (-32603, e.to_string()))?;
            return Ok(json!({
                "contents": [{
                    "uri": uri,
                    "mimeType": "application/json",
                    "text": serde_json::to_string_pretty(&list).unwrap_or_default(),
                }]
            }));
        }

        if uri == "solaris://brands" {
            let brands = self
                .backend
                .list_brands()
                .await
                .map_err(|e| (-32603, e.to_string()))?;
            return Ok(json!({
                "contents": [{
                    "uri": uri,
                    "mimeType": "application/json",
                    "text": serde_json::to_string_pretty(&brands).unwrap_or_default(),
                }]
            }));
        }

        if uri == "solaris://templates" {
            let templates = solaris_core_ir::builtin_templates();
            return Ok(json!({
                "contents": [{
                    "uri": uri,
                    "mimeType": "application/json",
                    "text": serde_json::to_string_pretty(&templates).unwrap_or_default(),
                }]
            }));
        }

        if uri == "solaris://assets/pack" {
            let assets = self
                .backend
                .list_assets()
                .await
                .map_err(|e| (-32603, e.to_string()))?;
            let pack = solaris_core_ir::build_asset_pack_manifest(
                "workspace-pack",
                "Solaris Workspace Assets",
                assets,
            );
            return Ok(json!({
                "contents": [{
                    "uri": uri,
                    "mimeType": "application/json",
                    "text": serde_json::to_string_pretty(&pack).unwrap_or_default(),
                }]
            }));
        }

        if let Some(pid) = uri.strip_prefix("solaris://project/") {
            let proj = self
                .backend
                .get_project(pid)
                .await
                .map_err(|e| (-32002, format!("Resource not found ({uri}): {e}")))?;
            return Ok(json!({
                "contents": [{
                    "uri": uri,
                    "mimeType": "application/json",
                    "text": serde_json::to_string_pretty(&proj).unwrap_or_default(),
                }]
            }));
        }

        Err((-32002, format!("Resource not found: {uri}")))
    }

    fn list_prompts(&self) -> Value {
        let prompts: Vec<Value> = list_skills()
            .into_iter()
            .map(|s| {
                json!({
                    "name": s.name,
                    "description": s.description,
                    "arguments": [{
                        "name": "project_id",
                        "description": "Target Solaris project ID",
                        "required": false
                    }]
                })
            })
            .collect();
        json!({ "prompts": prompts })
    }

    fn get_prompt(&self, params: Value) -> Result<Value, (i32, String)> {
        let name = params
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or((-32602, "Missing prompt 'name'".to_string()))?;
        let skill = get_skill(name).ok_or((-32602, format!("Prompt/skill not found: {name}")))?;
        let pid = params
            .get("arguments")
            .and_then(|a| a.get("project_id"))
            .and_then(|v| v.as_str());

        let text = if let Some(id) = pid {
            format!("Target project_id: {id}\n\n{}", skill.content)
        } else {
            skill.content
        };

        Ok(json!({
            "description": skill.description,
            "messages": [{
                "role": "user",
                "content": { "type": "text", "text": text }
            }]
        }))
    }
}

fn anchor_for_tool(tool_name: &str) -> &'static str {
    if tool_name.starts_with("color.") {
        "color.inspector"
    } else if tool_name.starts_with("audio.") {
        "audio.mixer"
    } else if tool_name.starts_with("gfx.") || tool_name.starts_with("assets.") {
        "effects.library"
    } else if tool_name.starts_with("captions.") {
        "captions.inspector"
    } else if tool_name.starts_with("brand.") {
        "brand.playbook"
    } else if tool_name.starts_with("preview.") || tool_name == "project.run_qa" {
        "program.stage"
    } else if tool_name.starts_with("media.") {
        "media.pool"
    } else {
        "timeline"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use solaris_agent_runtime::{BoxFuture, CapturedFrame, KeyframeResult};
    use solaris_core_ir::{
        AspectPreset, BrandPlaybook, ExportJobStatus, ExportRequest, MediaAsset, OpResult, Project,
        ProjectEngine, SavedCodeAsset, TimelineCommand, TranscriptWord,
    };
    use tokio::sync::RwLock;

    struct TestBackend {
        projects: RwLock<Vec<ProjectEngine>>,
    }

    impl SolarisBackend for TestBackend {
        fn list_projects(
            &self,
        ) -> BoxFuture<'_, Result<Vec<Value>, solaris_agent_runtime::AgentError>> {
            Box::pin(async { Ok(vec![]) })
        }
        fn create_project(
            &self,
            name: String,
            aspect: Option<AspectPreset>,
            _brand: Option<String>,
            _prompt: Option<String>,
            _template: Option<String>,
        ) -> BoxFuture<'_, Result<Project, solaris_agent_runtime::AgentError>> {
            Box::pin(async move {
                let mut p =
                    Project::new_default(name, aspect.unwrap_or(AspectPreset::Vertical9x16));
                p.media_pool.push(MediaAsset {
                    id: "m1".to_string(),
                    name: "hook.mp4".to_string(),
                    kind: solaris_core_ir::MediaKind::Video,
                    role_hint: "a_roll".to_string(),
                    path: String::new(),
                    added_at: p.created_at,
                    intelligence: solaris_core_ir::MediaIntelligenceManifest::default(),
                });
                self.projects
                    .write()
                    .await
                    .push(ProjectEngine::new(p.clone()));
                Ok(p)
            })
        }
        fn get_project(
            &self,
            id: &str,
        ) -> BoxFuture<'_, Result<Project, solaris_agent_runtime::AgentError>> {
            let id = id.to_string();
            Box::pin(async move {
                let g = self.projects.read().await;
                g.iter()
                    .find(|e| e.project().id == id)
                    .map(|e| e.project().clone())
                    .ok_or_else(|| solaris_agent_runtime::AgentError::ToolError("not found".into()))
            })
        }
        fn apply_op(
            &self,
            id: &str,
            cmd: TimelineCommand,
            meta: CommandMeta,
        ) -> BoxFuture<'_, Result<(OpResult, Project), solaris_agent_runtime::AgentError>> {
            let id = id.to_string();
            Box::pin(async move {
                let mut g = self.projects.write().await;
                let eng = g.iter_mut().find(|e| e.project().id == id).ok_or_else(|| {
                    solaris_agent_runtime::AgentError::ToolError("not found".into())
                })?;
                let (res, _) = eng
                    .apply(cmd, meta)
                    .map_err(|e| solaris_agent_runtime::AgentError::ToolError(e.to_string()))?;
                Ok((res, eng.project().clone()))
            })
        }
        fn undo(
            &self,
            _id: &str,
        ) -> BoxFuture<'_, Result<Project, solaris_agent_runtime::AgentError>> {
            Box::pin(async { Err(solaris_agent_runtime::AgentError::Backend("unused".into())) })
        }
        fn redo(
            &self,
            _id: &str,
        ) -> BoxFuture<'_, Result<Project, solaris_agent_runtime::AgentError>> {
            Box::pin(async { Err(solaris_agent_runtime::AgentError::Backend("unused".into())) })
        }
        fn acquire_track_lock(
            &self,
            _pid: &str,
            _tid: &str,
            _aid: &str,
            _role: AgentRole,
        ) -> BoxFuture<'_, Result<(), solaris_agent_runtime::AgentError>> {
            Box::pin(async { Ok(()) })
        }
        fn release_track_lock(
            &self,
            _pid: &str,
            _tid: &str,
            _aid: &str,
        ) -> BoxFuture<'_, Result<(), solaris_agent_runtime::AgentError>> {
            Box::pin(async { Ok(()) })
        }
        fn import_media_paths(
            &self,
            _pid: &str,
            _paths: Vec<(String, Option<String>)>,
        ) -> BoxFuture<'_, Result<Vec<MediaAsset>, solaris_agent_runtime::AgentError>> {
            Box::pin(async { Ok(vec![]) })
        }
        fn transcribe_media(
            &self,
            _pid: &str,
            _mid: &str,
            _lang: Option<String>,
        ) -> BoxFuture<'_, Result<Vec<TranscriptWord>, solaris_agent_runtime::AgentError>> {
            Box::pin(async { Ok(vec![]) })
        }
        fn extract_keyframes(
            &self,
            _pid: &str,
            _mid: &str,
            ts: Vec<f64>,
        ) -> BoxFuture<'_, Result<Vec<KeyframeResult>, solaris_agent_runtime::AgentError>> {
            Box::pin(async move {
                Ok(ts
                    .into_iter()
                    .enumerate()
                    .map(|(i, t)| KeyframeResult {
                        time_sec: t,
                        png_base64: format!("IMG_{i}"),
                    })
                    .collect())
            })
        }
        fn capture_preview_frame(
            &self,
            _pid: &str,
            _ts: f64,
        ) -> BoxFuture<'_, Result<CapturedFrame, solaris_agent_runtime::AgentError>> {
            Box::pin(async { Err(solaris_agent_runtime::AgentError::Backend("unused".into())) })
        }
        fn list_brands(
            &self,
        ) -> BoxFuture<'_, Result<Vec<BrandPlaybook>, solaris_agent_runtime::AgentError>> {
            Box::pin(async { Ok(vec![BrandPlaybook::solaris_default()]) })
        }
        fn get_brand(
            &self,
            _bid: &str,
        ) -> BoxFuture<'_, Result<BrandPlaybook, solaris_agent_runtime::AgentError>> {
            Box::pin(async { Ok(BrandPlaybook::solaris_default()) })
        }
        fn list_assets(
            &self,
        ) -> BoxFuture<'_, Result<Vec<SavedCodeAsset>, solaris_agent_runtime::AgentError>> {
            Box::pin(async { Ok(vec![]) })
        }
        fn save_asset(
            &self,
            a: SavedCodeAsset,
        ) -> BoxFuture<'_, Result<SavedCodeAsset, solaris_agent_runtime::AgentError>> {
            Box::pin(async { Ok(a) })
        }
        fn start_export(
            &self,
            _pid: &str,
            _req: ExportRequest,
        ) -> BoxFuture<'_, Result<ExportJobStatus, solaris_agent_runtime::AgentError>> {
            Box::pin(async { Err(solaris_agent_runtime::AgentError::Backend("unused".into())) })
        }
        fn emit_cursor_pulse(
            &self,
            _pid: &str,
            _aid: &str,
            _role: AgentRole,
            _anchor: &str,
            _label: &str,
        ) -> BoxFuture<'_, Result<(), solaris_agent_runtime::AgentError>> {
            Box::pin(async { Ok(()) })
        }
        fn emit_playhead_seek(
            &self,
            _pid: &str,
            _ts: f64,
            _aid: &str,
            _role: AgentRole,
        ) -> BoxFuture<'_, Result<(), solaris_agent_runtime::AgentError>> {
            Box::pin(async { Ok(()) })
        }
    }

    #[tokio::test]
    async fn mcp_initialize_tools_skills_and_project_mutation_work_end_to_end() {
        let server = McpServer::new(Arc::new(TestBackend {
            projects: RwLock::new(Vec::new()),
        }));

        let init = server
            .handle_json_str(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}"#)
            .await
            .unwrap();
        let init_res = init.result.unwrap();
        assert_eq!(init_res["serverInfo"]["name"], "solaris-mcp");
        assert_eq!(init_res["protocolVersion"], "2025-06-18");

        // Explicit "id": null must return -32600 (Invalid Request), not be dropped as notification
        let null_id = server
            .handle_json_str(r#"{"jsonrpc":"2.0","id":null,"method":"ping"}"#)
            .await
            .unwrap();
        assert_eq!(null_id.error.unwrap().code, -32600);

        // Unknown tool must return -32602
        let unknown_tool = server
            .handle_json_str(
                r#"{"jsonrpc":"2.0","id":99,"method":"tools/call","params":{"name":"does.not.exist","arguments":{}}}"#,
            )
            .await
            .unwrap();
        assert_eq!(unknown_tool.error.unwrap().code, -32602);

        // Unknown resource must return -32002
        let unknown_res = server
            .handle_json_str(
                r#"{"jsonrpc":"2.0","id":98,"method":"resources/read","params":{"uri":"solaris://does-not-exist"}}"#,
            )
            .await
            .unwrap();
        assert_eq!(unknown_res.error.unwrap().code, -32002);

        let templates = server
            .handle_json_str(r#"{"jsonrpc":"2.0","id":10,"method":"resources/templates/list"}"#)
            .await
            .unwrap();
        assert_eq!(
            templates.result.unwrap()["resourceTemplates"][0]["uriTemplate"],
            "solaris://project/{project_id}"
        );
        assert_eq!(anchor_for_tool("color.set_grade"), "color.inspector");
        assert_eq!(anchor_for_tool("audio.enable_ducking"), "audio.mixer");

        let tools = server
            .handle_json_str(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#)
            .await
            .unwrap();
        assert_eq!(tools.result.unwrap()["tools"].as_array().unwrap().len(), 34);

        let skill = server
            .handle_json_str(
                r#"{"jsonrpc":"2.0","id":3,"method":"resources/read","params":{"uri":"skill://director-core"}}"#,
            )
            .await
            .unwrap();
        let text = skill.result.unwrap()["contents"][0]["text"]
            .as_str()
            .unwrap()
            .to_string();
        assert!(text.contains("Director Core"));

        let created = server
            .handle_json_str(
                r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"project.create","arguments":{"name":"mcp_demo"}}}"#,
            )
            .await
            .unwrap();
        let pid = created.result.unwrap()["structuredContent"]["id"]
            .as_str()
            .unwrap()
            .to_string();

        let placed = server
            .handle_json_str(&format!(
                r#"{{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{{"name":"timeline.place_clip","arguments":{{"project_id":"{pid}","track_id":"v1","clip_id":"c1","media_id":"m1","start_sec":0,"duration_sec":3.0}}}}}}"#
            ))
            .await
            .unwrap();
        assert_eq!(placed.result.unwrap()["isError"], false);

        // Missing media_id must fail cleanly
        let bad_media = server
            .handle_json_str(&format!(
                r#"{{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{{"name":"timeline.place_clip","arguments":{{"project_id":"{pid}","track_id":"v1","clip_id":"c2","media_id":"missing","start_sec":3,"duration_sec":2.0}}}}}}"#
            ))
            .await
            .unwrap();
        assert_eq!(bad_media.result.unwrap()["isError"], true);

        // Protocol 2024-11-05 must omit structuredContent
        let _ = server
            .handle_json_str(
                r#"{"jsonrpc":"2.0","id":7,"method":"initialize","params":{"protocolVersion":"2024-11-05"}}"#,
            )
            .await
            .unwrap();
        let legacy_call = server
            .handle_json_str(
                r#"{"jsonrpc":"2.0","id":8,"method":"tools/call","params":{"name":"project.list","arguments":{}}}"#,
            )
            .await
            .unwrap();
        assert!(
            legacy_call
                .result
                .unwrap()
                .get("structuredContent")
                .is_none()
        );

        let templates_res = server
            .handle_json_str(
                r#"{"jsonrpc":"2.0","id":11,"method":"resources/read","params":{"uri":"solaris://templates"}}"#,
            )
            .await
            .unwrap();
        assert!(
            templates_res.result.unwrap()["contents"][0]["text"]
                .as_str()
                .unwrap()
                .contains("talking-head-short")
        );

        let pack_res = server
            .handle_json_str(
                r#"{"jsonrpc":"2.0","id":12,"method":"resources/read","params":{"uri":"solaris://assets/pack"}}"#,
            )
            .await
            .unwrap();
        assert!(
            pack_res.result.unwrap()["contents"][0]["text"]
                .as_str()
                .unwrap()
                .contains("solaris.asset-pack/v1")
        );

        let qa_res = server
            .handle_json_str(&format!(
                r#"{{"jsonrpc":"2.0","id":13,"method":"tools/call","params":{{"name":"project.run_qa","arguments":{{"project_id":"{pid}","auto_fix":true}}}}}}"#
            ))
            .await
            .unwrap();
        assert_eq!(qa_res.result.unwrap()["isError"], false);

        // Per-request protocol_version overrides do NOT mutate shared server state
        let modern = server
            .handle_json_rpc_with_version(
                json!({
                    "jsonrpc": "2.0",
                    "id": 14,
                    "method": "tools/call",
                    "params": {
                        "name": "media.extract_keyframes",
                        "arguments": { "project_id": pid, "media_id": "m1", "timestamps_sec": [0.5, 1.5] }
                    }
                }),
                Some("2025-06-18"),
            )
            .await
            .unwrap();
        let modern_res = modern.result.unwrap();
        assert!(modern_res.get("structuredContent").is_some());
        let content = modern_res["content"].as_array().unwrap();
        assert_eq!(content.len(), 3);
        assert_eq!(content[1]["type"], "image");
        assert_eq!(content[1]["data"], "IMG_0");
        assert_eq!(content[2]["data"], "IMG_1");
        // Shared version remained 2024-11-05 from earlier initialize call
        assert_eq!(server.protocol_version(), "2024-11-05");
    }
}
