use std::path::Path;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use reqwest::multipart::{Form, Part};
use solaris_core_ir::TranscriptWord;
use solaris_media_engine::parse_whisper_json;

use crate::advisor::{ModelAdvisor, ModelRoleKind};
use crate::error::AgentError;
use crate::openrouter::{
    ChatMessage, ChatRequest, ContentPart, InputAudioPayload, OpenRouterClient, backoff_delay_ms,
    parse_retry_after, truncate_msg,
};

const MAX_STT_BYTES: u64 = 25 * 1024 * 1024; // 25 MB OpenRouter STT limit

pub async fn transcribe_audio_openrouter(
    client: &OpenRouterClient,
    advisor: &ModelAdvisor,
    audio_path: &Path,
    language: Option<&str>,
) -> Result<Vec<TranscriptWord>, AgentError> {
    if !client.is_configured() {
        return Err(AgentError::NotConfigured);
    }

    let meta = tokio::fs::metadata(audio_path)
        .await
        .map_err(|e| AgentError::Backend(format!("failed to stat audio file: {e}")))?;
    if meta.len() == 0 {
        return Err(AgentError::ToolError(
            "audio file is empty (0 bytes); cannot transcribe empty audio".to_string(),
        ));
    }
    if meta.len() > MAX_STT_BYTES {
        return Err(AgentError::ToolError(format!(
            "audio upload {} bytes exceeds 25 MB OpenRouter STT limit",
            meta.len()
        )));
    }

    let bytes = tokio::fs::read(audio_path)
        .await
        .map_err(|e| AgentError::Backend(format!("failed to read audio file: {e}")))?;
    if bytes.is_empty() {
        return Err(AgentError::ToolError(
            "audio file is empty (0 bytes); cannot transcribe empty audio".to_string(),
        ));
    }
    let file_name = audio_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("audio.mp3")
        .to_string();

    let mime = infer_stt_mime(audio_path)?;
    let rec = advisor.recommend(ModelRoleKind::TranscriptionStt).await;
    let mut models = vec![rec.primary];
    for fb in rec.fallbacks {
        if !models.iter().any(|m| m == &fb) {
            models.push(fb);
        }
    }

    let timeout = Duration::from_secs(client.config().timeout_secs.max(30));
    let cleaned_lang = language
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    if let Ok(words) = transcribe_via_multipart_stt(
        client,
        &models,
        &bytes,
        &file_name,
        mime,
        cleaned_lang.as_deref(),
        timeout,
    )
    .await
        && !words.is_empty()
    {
        return Ok(words);
    }

    transcribe_audio_via_chat_input_audio(
        client,
        advisor,
        &bytes,
        audio_path,
        cleaned_lang.as_deref(),
    )
    .await
}

async fn transcribe_via_multipart_stt(
    client: &OpenRouterClient,
    models: &[String],
    bytes: &[u8],
    file_name: &str,
    mime: &str,
    cleaned_lang: Option<&str>,
    timeout: Duration,
) -> Result<Vec<TranscriptWord>, AgentError> {
    let cfg = client.config();
    let url = format!("{}/audio/transcriptions", cfg.base_url);
    let mut last_err: Option<AgentError> = None;

    for model in models {
        let mut attempt = 0u32;
        loop {
            let file_part = Part::bytes(bytes.to_vec())
                .file_name(file_name.to_string())
                .mime_str(mime)
                .map_err(|e| AgentError::Serialization(e.to_string()))?;

            let mut form = Form::new()
                .part("file", file_part)
                .text("model", model.clone())
                .text("response_format", "verbose_json")
                .text("timestamp_granularities[]", "word");

            if let Some(lang) = cleaned_lang {
                form = form.text("language", lang.to_string());
            }

            let headers = client.attribution_headers(true)?;
            let send_res = client
                .http_client()
                .post(&url)
                .headers(headers)
                .timeout(timeout)
                .multipart(form)
                .send()
                .await;

            match send_res {
                Ok(resp) => {
                    let status = resp.status();
                    let retry_after = parse_retry_after(resp.headers());
                    let body = match resp.text().await {
                        Ok(text) => text,
                        Err(e) => {
                            let err = AgentError::Transport(e.to_string());
                            if attempt < 2 {
                                attempt += 1;
                                tokio::time::sleep(Duration::from_millis(backoff_delay_ms(
                                    attempt, None,
                                )))
                                .await;
                                continue;
                            }
                            last_err = Some(err);
                            break;
                        }
                    };
                    if status.is_success() {
                        match parse_whisper_json(&body) {
                            Ok(words) => return Ok(words),
                            Err(parse_err) => {
                                last_err = Some(AgentError::Serialization(parse_err.to_string()));
                                break;
                            }
                        }
                    }
                    let err = AgentError::HttpStatus {
                        status: status.as_u16(),
                        message: truncate_msg(&body),
                        retry_after_secs: retry_after,
                    };
                    if err.is_retryable() && attempt < 2 {
                        attempt += 1;
                        tokio::time::sleep(Duration::from_millis(backoff_delay_ms(
                            attempt,
                            retry_after,
                        )))
                        .await;
                        continue;
                    }
                    last_err = Some(err);
                    break;
                }
                Err(e) => {
                    let err = AgentError::Transport(e.to_string());
                    if attempt < 2 {
                        attempt += 1;
                        tokio::time::sleep(Duration::from_millis(backoff_delay_ms(attempt, None)))
                            .await;
                        continue;
                    }
                    last_err = Some(err);
                    break;
                }
            }
        }
    }

    Err(last_err.unwrap_or_else(|| AgentError::Backend("STT transcription failed".to_string())))
}

async fn transcribe_audio_via_chat_input_audio(
    client: &OpenRouterClient,
    advisor: &ModelAdvisor,
    bytes: &[u8],
    audio_path: &Path,
    language: Option<&str>,
) -> Result<Vec<TranscriptWord>, AgentError> {
    let dir_rec = advisor.recommend(ModelRoleKind::DirectorMultimodal).await;
    let mut audio_models: Vec<String> = Vec::new();
    for cand in dir_rec.candidates {
        if cand.input_modalities.iter().any(|m| m == "audio")
            && !audio_models.iter().any(|existing| existing == &cand.id)
        {
            audio_models.push(cand.id);
        }
    }
    if audio_models.is_empty() {
        for fallback in [
            "google/gemini-4-hydrogen",
            "google/gemini-4-neon",
            "xiaomi/mimo-v2.6-pro",
        ] {
            if !audio_models.iter().any(|m| m == fallback) {
                audio_models.push(fallback.to_string());
            }
        }
    }

    let format = audio_path
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_ascii_lowercase())
        .filter(|s| matches!(s.as_str(), "mp3" | "wav" | "aac" | "ogg" | "flac" | "m4a"))
        .unwrap_or_else(|| "mp3".to_string());

    let lang_hint = language
        .map(|l| format!(" Language hint: {l}."))
        .unwrap_or_default();
    let prompt = format!(
        "Transcribe all spoken and sung words in this audio accurately with timestamps in seconds.{lang_hint} \
         Output ONLY a valid raw JSON object (no markdown fences, no commentary) in this exact schema: \
         {{\"segments\":[{{\"start\":0.0,\"end\":2.4,\"text\":\"exact words spoken\"}}]}}. \
         Split into short natural phrases (3-8 words each) with accurate start and end times in seconds. If no speech or singing is present, return {{\"segments\":[]}}."
    );

    let b64 = BASE64.encode(bytes);
    let req = ChatRequest {
        model: None,
        models: Some(audio_models.into_iter().take(3).collect()),
        messages: vec![ChatMessage::user_parts(vec![
            ContentPart::text(prompt),
            ContentPart::InputAudio {
                input_audio: InputAudioPayload { data: b64, format },
            },
        ])],
        tools: None,
        tool_choice: None,
        provider: None,
        reasoning: None,
        session_id: None,
        max_tokens: Some(4096),
        temperature: Some(0.0),
        response_format: None,
        plugins: None,
        stream: Some(true),
    };

    let resp = client.chat_stream(&req, None, |_| {}).await?;
    let raw = resp
        .choices
        .first()
        .and_then(|c| c.message.content.as_ref())
        .map(|c| c.as_text())
        .unwrap_or_default();
    let stripped = strip_json_code_fence(&raw);
    parse_whisper_json(stripped).map_err(|e| AgentError::Serialization(e.to_string()))
}

fn strip_json_code_fence(raw: &str) -> &str {
    let trimmed = raw.trim();
    if let Some(after_open) = trimmed.strip_prefix("```") {
        let after_lang = after_open
            .find('\n')
            .map(|idx| &after_open[idx + 1..])
            .unwrap_or(after_open);
        if let Some(before_close) = after_lang.strip_suffix("```") {
            return before_close.trim();
        }
        return after_lang.trim();
    }
    trimmed
}

fn infer_stt_mime(path: &Path) -> Result<&'static str, AgentError> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    match ext.as_str() {
        "mp3" => Ok("audio/mpeg"),
        "wav" => Ok("audio/wav"),
        "m4a" => Ok("audio/mp4"),
        "mp4" => Err(AgentError::ToolError(
            "unsupported STT format '.mp4': OpenRouter /audio/transcriptions accepts audio formats only (mp3, wav, m4a, aac, ogg, flac, webm); extract audio track first".to_string(),
        )),
        "aac" => Ok("audio/aac"),
        "ogg" => Ok("audio/ogg"),
        "flac" => Ok("audio/flac"),
        "webm" => Ok("audio/webm"),
        other => Err(AgentError::ToolError(format!(
            "unsupported STT audio format '.{other}'; expected mp3, wav, m4a, aac, ogg, flac, or webm"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn infer_stt_mime_accepts_audio_and_rejects_mp4() {
        assert_eq!(
            infer_stt_mime(Path::new("voice.mp3")).unwrap(),
            "audio/mpeg"
        );
        assert_eq!(infer_stt_mime(Path::new("voice.wav")).unwrap(), "audio/wav");
        assert_eq!(infer_stt_mime(Path::new("voice.m4a")).unwrap(), "audio/mp4");
        assert_eq!(infer_stt_mime(Path::new("voice.ogg")).unwrap(), "audio/ogg");
        assert_eq!(
            infer_stt_mime(Path::new("voice.flac")).unwrap(),
            "audio/flac"
        );
        assert_eq!(
            infer_stt_mime(Path::new("voice.webm")).unwrap(),
            "audio/webm"
        );
        assert!(infer_stt_mime(Path::new("video.mp4")).is_err());
        assert!(infer_stt_mime(Path::new("clip.mov")).is_err());
    }
}
