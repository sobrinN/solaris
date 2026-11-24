use std::path::Path;
use std::time::Duration;

use reqwest::multipart::{Form, Part};
use solaris_core_ir::TranscriptWord;
use solaris_media_engine::parse_whisper_json;

use crate::advisor::{ModelAdvisor, ModelRoleKind};
use crate::error::AgentError;
use crate::openrouter::{OpenRouterClient, backoff_delay_ms, parse_retry_after, truncate_msg};

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

    let cfg = client.config();
    let url = format!("{}/audio/transcriptions", cfg.base_url);
    let timeout = Duration::from_secs(cfg.timeout_secs.max(30));
    let cleaned_lang = language
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    let mut last_err: Option<AgentError> = None;

    for model in models {
        let mut attempt = 0u32;
        loop {
            let file_part = Part::bytes(bytes.clone())
                .file_name(file_name.clone())
                .mime_str(mime)
                .map_err(|e| AgentError::Serialization(e.to_string()))?;

            let mut form = Form::new()
                .part("file", file_part)
                .text("model", model.clone())
                .text("response_format", "verbose_json")
                .text("timestamp_granularities[]", "word");

            if let Some(ref lang) = cleaned_lang {
                form = form.text("language", lang.clone());
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
