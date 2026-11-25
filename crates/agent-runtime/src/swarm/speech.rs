use std::collections::HashSet;

use solaris_core_ir::{ClipKind, MediaAsset, MediaKind, Project, TranscriptWord};

use super::{is_broll_hint, is_voice_hint};

const MIN_NARRATION_WORDS: usize = 3;
const MUSIC_SFX_NAME_TOKENS: [&str; 6] =
    ["music", "bgm", "soundtrack", "sfx", "ambience", "ambient"];

fn is_music_or_sfx_hint(hint: &str) -> bool {
    matches!(
        hint.to_ascii_lowercase().as_str(),
        "music" | "bgm" | "beat" | "soundtrack" | "sfx" | "sound_effect" | "ambience"
    )
}

fn name_marks_music_or_sfx(name: &str) -> bool {
    name.split(|c: char| !c.is_ascii_alphanumeric())
        .any(|token| {
            MUSIC_SFX_NAME_TOKENS
                .iter()
                .any(|t| token.eq_ignore_ascii_case(t))
        })
}

pub(crate) fn is_music_or_sfx_asset(asset: &MediaAsset) -> bool {
    if is_voice_hint(&asset.role_hint) {
        return false;
    }
    is_music_or_sfx_hint(&asset.role_hint) || name_marks_music_or_sfx(&asset.name)
}

fn strip_sound_tags(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find(['[', '(', '*']) {
        let opener = char::from(rest.as_bytes()[at]);
        let closer = match opener {
            '[' => ']',
            '(' => ')',
            _ => '*',
        };
        out.push_str(&rest[..at]);
        let inner = &rest[at + 1..];
        match inner.find(closer) {
            Some(end) => rest = &inner[end + 1..],
            None => {
                out.push(opener);
                rest = inner;
            }
        }
    }
    out.push_str(rest);
    out.retain(|c| !matches!(c, '♪' | '♫' | '♬' | '♩'));
    out
}

fn count_narration_words(words: &[TranscriptWord]) -> usize {
    let text = words
        .iter()
        .map(|w| w.word.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    strip_sound_tags(&text)
        .split_whitespace()
        .filter(|token| token.chars().any(char::is_alphanumeric))
        .count()
}

fn is_narration_candidate(asset: &MediaAsset) -> bool {
    match asset.kind {
        MediaKind::Video => {
            asset.intelligence.has_audio
                && !is_broll_hint(&asset.role_hint)
                && !is_music_or_sfx_asset(asset)
        }
        MediaKind::Audio => is_voice_hint(&asset.role_hint) || !is_music_or_sfx_asset(asset),
        MediaKind::Image | MediaKind::Document => false,
    }
}

pub(crate) fn narrated_transcript_present(project: &Project) -> bool {
    let placed: HashSet<&str> = project
        .tracks
        .iter()
        .filter(|t| t.id == "v1" || t.id == "a1")
        .flat_map(|t| t.clips.iter())
        .filter_map(|clip| match &clip.payload {
            ClipKind::Media { media_id, .. } => Some(media_id.as_str()),
            _ => None,
        })
        .collect();
    project.media_pool.iter().any(|asset| {
        let on_spine = placed.contains(asset.id.as_str());
        let voice_audio = asset.kind == MediaKind::Audio && is_voice_hint(&asset.role_hint);
        (on_spine || voice_audio)
            && count_narration_words(&asset.intelligence.transcript) >= MIN_NARRATION_WORDS
    })
}

pub(crate) fn narration_expected(project: &Project) -> bool {
    narrated_transcript_present(project)
        || project.media_pool.iter().any(|asset| {
            is_narration_candidate(asset)
                && count_narration_words(&asset.intelligence.transcript) >= MIN_NARRATION_WORDS
        })
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use solaris_core_ir::{
        AspectPreset, AudioSettings, Clip, MediaIntelligenceManifest, SwarmRunState, Transform2D,
    };

    use super::*;
    use crate::advisor::ModelAdvisor;
    use crate::swarm::run_swarm;
    use crate::swarm::test_support::{InMemBackend, local_request, offline_client};
    use tokio_util::sync::CancellationToken;

    fn words(text: &str) -> Vec<TranscriptWord> {
        text.split_whitespace()
            .enumerate()
            .map(|(i, w)| TranscriptWord {
                word: w.to_string(),
                start_sec: i as f64 * 0.5,
                end_sec: i as f64 * 0.5 + 0.4,
            })
            .collect()
    }

    fn asset(id: &str, name: &str, kind: MediaKind, role: &str, transcript: &str) -> MediaAsset {
        MediaAsset {
            id: id.to_string(),
            name: name.to_string(),
            kind,
            role_hint: role.to_string(),
            path: format!("/media/{id}"),
            added_at: Utc::now(),
            intelligence: MediaIntelligenceManifest {
                analyzed: true,
                duration_sec: 6.0,
                has_audio: true,
                transcript: words(transcript),
                ..MediaIntelligenceManifest::default()
            },
        }
    }

    fn project_with(media: Vec<MediaAsset>) -> Project {
        let mut project = Project::new_default("speech", AspectPreset::Vertical9x16);
        project.media_pool = media;
        project
    }

    fn place(project: &mut Project, track_id: &str, media_id: &str) {
        let track = project
            .tracks
            .iter_mut()
            .find(|t| t.id == track_id)
            .expect("track exists");
        track.clips.push(Clip {
            id: format!("clip-{media_id}"),
            label: media_id.to_string(),
            start_sec: 0.0,
            duration_sec: 3.0,
            source_in_sec: 0.0,
            transform: Transform2D::default(),
            audio: AudioSettings::default(),
            payload: ClipKind::Media {
                media_id: media_id.to_string(),
                ken_burns: false,
            },
        });
    }

    async fn run_local(media: Vec<MediaAsset>) -> Project {
        let project = project_with(media);
        let pid = project.id.clone();
        let backend = InMemBackend::new(project, Vec::new());
        let status = run_swarm(
            backend.clone(),
            offline_client(),
            std::sync::Arc::new(ModelAdvisor::new(offline_client())),
            pid,
            "run-speech".to_string(),
            local_request("Edit a 9:16 hook reel"),
            CancellationToken::new(),
            |_| {},
        )
        .await;
        assert_eq!(status.state, SwarmRunState::Completed);
        backend.engine.read().await.project().clone()
    }

    fn t1_clip_count(project: &Project) -> usize {
        project
            .tracks
            .iter()
            .find(|t| t.id == "t1")
            .map_or(0, |t| t.clips.len())
    }

    #[test]
    fn sound_tags_never_count_as_narration_words() {
        assert_eq!(
            count_narration_words(&words("[Music] ♪ (applause) *sigh* [BLANK_AUDIO] ♫ ...")),
            0
        );
        assert_eq!(count_narration_words(&words("Hello (uh) there")), 2);
        assert_eq!(count_narration_words(&words("Price 5 * 3 now")), 4);
    }

    #[test]
    fn narration_needs_three_real_words() {
        let two = project_with(vec![asset(
            "vo",
            "vo.wav",
            MediaKind::Audio,
            "voice",
            "Hello there",
        )]);
        assert!(!narrated_transcript_present(&two));
        let three = project_with(vec![asset(
            "vo",
            "vo.wav",
            MediaKind::Audio,
            "voice",
            "Three AI tools",
        )]);
        assert!(narrated_transcript_present(&three));
    }

    #[test]
    fn narration_must_be_placed_on_v1_or_a1_unless_it_is_voice_audio() {
        let mut video = project_with(vec![asset(
            "m1",
            "talk.mp4",
            MediaKind::Video,
            "a_roll",
            "Three AI tools change everything",
        )]);
        assert!(!narrated_transcript_present(&video));
        place(&mut video, "v1", "m1");
        assert!(narrated_transcript_present(&video));

        let mut audio = project_with(vec![asset(
            "m2",
            "vo.wav",
            MediaKind::Audio,
            "audio",
            "Three AI tools change everything",
        )]);
        assert!(!narrated_transcript_present(&audio));
        place(&mut audio, "a1", "m2");
        assert!(narrated_transcript_present(&audio));
    }

    #[test]
    fn music_and_sfx_are_recognised_by_hint_or_name_but_voice_is_kept() {
        let music = asset("m", "beat.wav", MediaKind::Audio, "music", "");
        let sfx = asset("s", "SFX_whoosh.mp4", MediaKind::Video, "", "");
        let voice_named_music = asset("v", "music_intro.mp4", MediaKind::Video, "a_roll", "");
        let plain = asset("p", "talk.mp4", MediaKind::Video, "", "");
        assert!(is_music_or_sfx_asset(&music));
        assert!(is_music_or_sfx_asset(&sfx));
        assert!(!is_music_or_sfx_asset(&voice_named_music));
        assert!(!is_music_or_sfx_asset(&plain));
    }

    #[test]
    fn narration_expected_looks_at_candidates_before_placement() {
        let talk = project_with(vec![asset(
            "m",
            "talk.mp4",
            MediaKind::Video,
            "",
            "Three AI tools change everything",
        )]);
        assert!(!narrated_transcript_present(&talk));
        assert!(narration_expected(&talk));

        let music = project_with(vec![asset(
            "m",
            "music_video.mp4",
            MediaKind::Video,
            "music",
            "[Music] ♪ ♪",
        )]);
        assert!(!narration_expected(&music));

        let broll = project_with(vec![asset(
            "b",
            "cut.mp4",
            MediaKind::Video,
            "b_roll",
            "Three AI tools change everything",
        )]);
        assert!(!narration_expected(&broll));
    }

    #[tokio::test]
    async fn auto_transcription_skips_music_and_sfx_but_keeps_voice_video() {
        let music = asset("music", "bgm-loop.mp4", MediaKind::Video, "", "");
        let talk = asset("talk", "talk.mp4", MediaKind::Video, "a_roll", "");
        let project = project_with(vec![music, talk]);
        let pid = project.id.clone();
        let backend = InMemBackend::new(project, Vec::new());
        run_swarm(
            backend.clone(),
            offline_client(),
            std::sync::Arc::new(ModelAdvisor::new(offline_client())),
            pid,
            "run-transcribe".to_string(),
            local_request("Edit a 9:16 hook reel"),
            CancellationToken::new(),
            |_| {},
        )
        .await;
        let transcribed = backend.transcribed.lock().expect("transcript log").clone();
        assert_eq!(transcribed, vec!["talk".to_string()]);
    }

    #[tokio::test]
    async fn music_only_video_gets_no_captions() {
        let project = run_local(vec![asset(
            "m",
            "music_video.mp4",
            MediaKind::Video,
            "music",
            "[Music] ♪ (music)",
        )])
        .await;
        assert_eq!(t1_clip_count(&project), 0);
    }

    #[tokio::test]
    async fn narrated_voice_audio_on_a1_gets_captions() {
        let project = run_local(vec![asset(
            "vo",
            "voiceover.wav",
            MediaKind::Audio,
            "voice",
            "Three AI tools change everything you build",
        )])
        .await;
        assert!(t1_clip_count(&project) > 0);
    }

    #[tokio::test]
    async fn sound_tag_only_transcript_gets_no_captions() {
        let project = run_local(vec![asset(
            "m",
            "talk.mp4",
            MediaKind::Video,
            "a_roll",
            "[Applause] ♪ (music) [Applause]",
        )])
        .await;
        assert_eq!(t1_clip_count(&project), 0);
    }

    #[tokio::test]
    async fn fewer_than_three_real_words_get_no_captions() {
        let project = run_local(vec![asset(
            "m",
            "talk.mp4",
            MediaKind::Video,
            "a_roll",
            "Hi there",
        )])
        .await;
        assert_eq!(t1_clip_count(&project), 0);
    }
}
