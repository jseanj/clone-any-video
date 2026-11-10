use anyhow::{Context, Result};
use reqwest::multipart;
use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::audio::extract_speech_evidence_audio;

/// 转录规范: "clone-any-video.transcript@1"
pub const TRANSCRIPT_FORMAT: &str = "clone-any-video.transcript@1";

/// 本地 ASR 服务的默认回环地址
pub const DEFAULT_LOCAL_WHISPERX_URL: &str = "http://127.0.0.1:8765";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptDocument {
    pub format: String,
    pub source: String,
    pub language: String,
    pub audio_seconds: f64,
    pub passages: Vec<TranscriptPassage>,
    #[serde(default)]
    pub evidence: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptPassage {
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_seconds: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_seconds: Option<f64>,
    pub words: Vec<TranscriptWord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptWord {
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_seconds: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_seconds: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
}

/// 兼容 Whisper API / WhisperX 本地服务的原始响应结构
#[derive(Debug, Deserialize)]
struct GenericWhisperResponse {
    #[serde(default)]
    engine: serde_json::Value,
    #[serde(default)]
    is_mock: bool,
    duration: Option<f64>,
    text: Option<String>,
    segments: Option<Vec<GenericSegment>>,
    words: Option<Vec<GenericWord>>,
}

#[derive(Debug, Deserialize)]
struct GenericSegment {
    text: String,
    start: f64,
    end: f64,
    words: Option<Vec<GenericWord>>,
}

#[derive(Debug, Deserialize)]
struct GenericWord {
    #[serde(alias = "word")]
    text: Option<String>,
    start: f64,
    end: f64,
    score: Option<f64>,
}

#[derive(Debug, Serialize)]
struct LocalTranscribePayload<'a> {
    audio_path: &'a str,
    language: &'a str,
}

pub struct TranscribeOptions<'a> {
    pub source_path: &'a Path,
    pub language: &'a str,
    pub api_key: Option<String>,
    pub api_url: Option<String>,
    pub local: bool,
    pub mock: bool,
}

/// 检查本地 ASR 热服务是否在线
async fn check_local_whisperx_healthy(client: &reqwest::Client, base_url: &str) -> bool {
    let health_url = format!("{}/health", base_url.trim_end_matches('/'));
    match client
        .get(&health_url)
        .timeout(std::time::Duration::from_millis(500))
        .send()
        .await
    {
        Ok(resp) => resp.status().is_success(),
        Err(_) => false,
    }
}

/// 执行语音转录并保留 ASR 词级时间戳，优先使用本地服务，生成符合结构体
pub async fn transcribe_media(options: TranscribeOptions<'_>) -> Result<TranscriptDocument> {
    let source_path_str = options.source_path.to_string_lossy().to_string();

    if options.mock {
        return Ok(generate_mock_transcript(
            &source_path_str,
            options.language,
            5.0,
        ));
    }

    // 1. 抽取 16kHz 单声道 PCM 证据音频 (Whisper 规范最佳输入)
    let wav_path = extract_speech_evidence_audio(options.source_path, None).await?;
    let wav_bytes = tokio::fs::read(&wav_path)
        .await
        .context("Failed to read extracted speech evidence WAV")?;

    let sample_frames = (wav_bytes.len().saturating_sub(44)) / 2;
    let audio_seconds = (sample_frames as f64 / 16_000.0 * 1000.0).round() / 1000.0;

    let client = reqwest::Client::new();

    // 2. 检查本地与远程路由策略
    let local_base_url = options
        .api_url
        .clone()
        .unwrap_or_else(|| DEFAULT_LOCAL_WHISPERX_URL.to_string());
    let is_local_online = check_local_whisperx_healthy(&client, &local_base_url).await;

    // 分支 A: 本地 WhisperX 热服务检查
    if options.local && !is_local_online {
        anyhow::bail!(
            "--local flag specified, but local ASR daemon is unreachable at '{}'.\n\
             Please start it via 'scripts/whisper_daemon.py' or check daemon logs.",
            local_base_url
        );
    }

    if is_local_online {
        eprintln!(
            "🚀 发现正在运行的本地 ASR 服务 ({})，采用纯本地常驻推理...",
            local_base_url
        );
        let transcribe_url = format!("{}/transcribe", local_base_url.trim_end_matches('/'));
        let wav_abs_path = wav_path.canonicalize().unwrap_or_else(|_| wav_path.clone());
        let payload = LocalTranscribePayload {
            audio_path: &wav_abs_path.to_string_lossy(),
            language: options.language,
        };

        let resp = client
            .post(&transcribe_url)
            .json(&payload)
            .send()
            .await
            .context("Failed to send request to local ASR daemon")?;

        let _ = tokio::fs::remove_file(&wav_path).await;

        if !resp.status().is_success() {
            let err_text = resp.text().await.unwrap_or_default();
            anyhow::bail!("Local ASR service returned error: {}", err_text);
        }

        let parsed: GenericWhisperResponse = resp
            .json()
            .await
            .context("Failed to parse local ASR response")?;
        return convert_to_transcript_doc(
            parsed,
            &source_path_str,
            options.language,
            audio_seconds,
        );
    }

    // 分支 B: 指定了外部 API 或配置了 OPENAI_API_KEY
    let api_key = options
        .api_key
        .or_else(|| std::env::var("OPENAI_API_KEY").ok());
    if !options.mock && api_key.is_some() {
        let key = api_key.unwrap();
        let api_url = options
            .api_url
            .unwrap_or_else(|| "https://api.openai.com/v1/audio/transcriptions".to_string());
        eprintln!(
            "🌐 未检测到本地 ASR 守护进程，调用远程转录端点 ({}) ...",
            api_url
        );

        let file_part = multipart::Part::bytes(wav_bytes)
            .file_name("speech.wav")
            .mime_str("audio/wav")?;

        let form = multipart::Form::new()
            .part("file", file_part)
            .text("model", "whisper-1")
            .text("language", options.language.to_string())
            .text("response_format", "verbose_json")
            .text("timestamp_granularities[]", "word")
            .text("timestamp_granularities[]", "segment");

        let resp = client
            .post(&api_url)
            .bearer_auth(key)
            .multipart(form)
            .send()
            .await
            .context("Failed to send request to remote Whisper API")?;

        let _ = tokio::fs::remove_file(&wav_path).await;

        if !resp.status().is_success() {
            let err_text = resp.text().await.unwrap_or_default();
            anyhow::bail!("Whisper API returned error: {}", err_text);
        }

        let parsed: GenericWhisperResponse = resp
            .json()
            .await
            .context("Failed to parse Whisper API JSON response")?;
        return convert_to_transcript_doc(
            parsed,
            &source_path_str,
            options.language,
            audio_seconds,
        );
    }

    // 分支 C: 离线 / 教学 Mock 回退 (必须显式通过 --mock 触发，生产模式严禁静默降级)
    let _ = tokio::fs::remove_file(&wav_path).await;
    if !options.mock {
        anyhow::bail!(
            "Speech transcription failed: Local ASR daemon unreachable at '{}', and OPENAI_API_KEY is not configured.\n\
             Production mode prohibits silent mock degradation. Please start whisper_daemon.py, set OPENAI_API_KEY, or pass --mock explicitly.",
            local_base_url
        );
    }

    Ok(generate_mock_transcript(
        &source_path_str,
        options.language,
        audio_seconds,
    ))
}

fn convert_to_transcript_doc(
    parsed: GenericWhisperResponse,
    source: &str,
    language: &str,
    fallback_duration: f64,
) -> Result<TranscriptDocument> {
    let duration = parsed.duration.unwrap_or(fallback_duration);
    if parsed.is_mock {
        anyhow::bail!("ASR endpoint returned simulated evidence in production mode");
    }
    let engine = parsed.engine.clone();
    let mut passages = Vec::new();

    let full_text = parsed.text.unwrap_or_default();
    let root_words: Vec<TranscriptWord> = if let Some(words) = parsed.words {
        words
            .into_iter()
            .filter_map(|w| {
                let text = w.text?.trim().to_string();
                if text.is_empty() {
                    None
                } else {
                    Some(TranscriptWord {
                        text,
                        start_seconds: Some((w.start * 1000.0).round() / 1000.0),
                        end_seconds: Some((w.end * 1000.0).round() / 1000.0),
                        score: w.score,
                    })
                }
            })
            .collect()
    } else {
        Vec::new()
    };

    if let Some(segments) = parsed.segments {
        for seg in segments {
            let mut words = if let Some(seg_words) = seg.words {
                seg_words
                    .into_iter()
                    .filter_map(|w| {
                        let text = w.text?.trim().to_string();
                        if text.is_empty() {
                            None
                        } else {
                            Some(TranscriptWord {
                                text,
                                start_seconds: Some((w.start * 1000.0).round() / 1000.0),
                                end_seconds: Some((w.end * 1000.0).round() / 1000.0),
                                score: w.score,
                            })
                        }
                    })
                    .collect()
            } else {
                Vec::new()
            };

            // 如果分段内部没有词级列表，但顶层有词级证据，依据时间范围将顶层词分配给本分段
            if words.is_empty() && !root_words.is_empty() {
                let seg_start = seg.start - 0.05;
                let seg_end = seg.end + 0.05;
                words = root_words
                    .iter()
                    .filter(|w| {
                        let ws = w.start_seconds.unwrap_or(0.0);
                        let we = w.end_seconds.unwrap_or(ws);
                        ws >= seg_start && we <= seg_end
                    })
                    .cloned()
                    .collect();
            }

            passages.push(TranscriptPassage {
                text: seg.text.trim().to_string(),
                start_seconds: Some((seg.start * 1000.0).round() / 1000.0),
                end_seconds: Some((seg.end * 1000.0).round() / 1000.0),
                words,
            });
        }

        // 如果所有分段都没匹配到词，但 root_words 存在，分配 root_words
        if !root_words.is_empty() {
            if passages.is_empty() {
                let start_s = root_words
                    .first()
                    .and_then(|w| w.start_seconds)
                    .unwrap_or(0.0);
                let end_s = root_words
                    .last()
                    .and_then(|w| w.end_seconds)
                    .unwrap_or(duration);
                passages.push(TranscriptPassage {
                    text: full_text.clone(),
                    start_seconds: Some(start_s),
                    end_seconds: Some(end_s),
                    words: root_words,
                });
            } else if passages.iter().all(|p| p.words.is_empty()) {
                if passages.len() == 1 {
                    passages[0].words = root_words;
                } else {
                    let total_p = passages.len();
                    let chunk_size = (root_words.len() + total_p - 1) / total_p;
                    for (idx, p) in passages.iter_mut().enumerate() {
                        let start = idx * chunk_size;
                        let end = (start + chunk_size).min(root_words.len());
                        if start < root_words.len() {
                            p.words = root_words[start..end].to_vec();
                        }
                    }
                }
            }
        }
    } else if !root_words.is_empty() {
        passages.push(TranscriptPassage {
            text: full_text.clone(),
            start_seconds: Some(0.0),
            end_seconds: Some(duration),
            words: root_words,
        });
    }

    if passages.is_empty() {
        passages.push(TranscriptPassage {
            text: full_text,
            start_seconds: Some(0.0),
            end_seconds: Some(duration),
            words: Vec::new(),
        });
    }

    Ok(TranscriptDocument {
        evidence: serde_json::json!({"engine":engine,"kind":"asr-word-timestamps","forced_alignment":false}),
        format: TRANSCRIPT_FORMAT.to_string(),
        source: source.to_string(),
        language: language.to_string(),
        audio_seconds: (duration * 1000.0).round() / 1000.0,
        passages,
    })
}

/// 供无本地服务且无 API Key 状态下读者体验的 Mock 对齐数据
fn generate_mock_transcript(source: &str, language: &str, duration: f64) -> TranscriptDocument {
    let (p1_text, words_data) = match language {
        "zh" => (
            "这里是测试样片，探索视频智能复刻的未来。",
            vec![
                ("这里是", 0.0, 0.6),
                ("测试", 0.62, 1.1),
                ("样片", 1.15, 1.7),
                ("探索", 1.8, 2.3),
                ("视频", 2.35, 2.8),
                ("智能", 2.85, 3.4),
                ("复刻", 3.45, 4.1),
                ("未来", 4.15, 4.9),
            ],
        ),
        _ => (
            "Ronaldo is D tier. Bro has more hair gel than trophies.",
            vec![
                ("Ronaldo", 0.0, 0.65),
                ("is", 0.68, 0.85),
                ("D", 0.88, 1.20),
                ("tier.", 1.22, 1.75),
                ("Bro", 1.85, 2.15),
                ("has", 2.18, 2.45),
                ("more", 2.48, 2.85),
                ("hair", 2.88, 3.25),
                ("gel", 3.28, 3.75),
                ("than", 3.78, 4.10),
                ("trophies.", 4.15, 4.85),
            ],
        ),
    };

    let words = words_data
        .into_iter()
        .map(|(w, s, e)| TranscriptWord {
            text: w.to_string(),
            start_seconds: Some(s),
            end_seconds: Some(e),
            score: Some(0.98),
        })
        .collect();

    TranscriptDocument {
        evidence: serde_json::json!({"kind":"mock","forced_alignment":false}),
        format: TRANSCRIPT_FORMAT.to_string(),
        source: source.to_string(),
        language: language.to_string(),
        audio_seconds: duration.max(5.0),
        passages: vec![TranscriptPassage {
            text: p1_text.to_string(),
            start_seconds: Some(0.0),
            end_seconds: Some(duration.max(5.0)),
            words,
        }],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mock_transcript_generation() {
        let doc = generate_mock_transcript("test.mp4", "en", 10.0);
        assert_eq!(doc.format, TRANSCRIPT_FORMAT);
        assert_eq!(doc.language, "en");
        assert_eq!(doc.audio_seconds, 10.0);
        assert!(!doc.passages.is_empty());
    }

    #[tokio::test]
    async fn test_transcribe_with_mock_flag() {
        let fixture = if Path::new("fixtures/sample.mp4").exists() { Path::new("fixtures/sample.mp4") } else { Path::new("../../fixtures/sample.mp4") };
        if fixture.exists() {
            let doc = transcribe_media(TranscribeOptions {
                source_path: fixture,
                language: "en",
                api_key: None,
                api_url: None,
                local: false,
                mock: true,
            })
            .await
            .expect("mock transcribe should succeed");
            assert_eq!(doc.format, TRANSCRIPT_FORMAT);
            assert_eq!(doc.passages[0].words.len(), 11);
        }
    }

    #[test]
    fn test_convert_to_transcript_doc_preserves_root_words() {
        let raw_json = serde_json::json!({
            "text": "Hello world.",
            "duration": 2.0,
            "segments": [
                {
                    "id": 0,
                    "text": "Hello world.",
                    "start": 0.0,
                    "end": 2.0
                }
            ],
            "words": [
                { "word": "Hello", "start": 0.0, "end": 0.5 },
                { "word": "world.", "start": 0.5, "end": 1.0 }
            ]
        });
        let parsed: GenericWhisperResponse = serde_json::from_value(raw_json).unwrap();
        let doc = convert_to_transcript_doc(parsed, "test.wav", "en", 2.0).unwrap();
        assert_eq!(doc.passages.len(), 1);
        assert_eq!(doc.passages[0].words.len(), 2);
        assert_eq!(doc.passages[0].words[0].text, "Hello");
        assert_eq!(doc.passages[0].words[1].text, "world.");
    }

    #[test]
    fn test_convert_to_transcript_doc_empty_segments_no_panic() {
        let raw_json = serde_json::json!({
            "text": "Hello world.",
            "duration": 2.0,
            "segments": [],
            "words": [
                { "word": "Hello", "start": 0.0, "end": 0.5 },
                { "word": "world.", "start": 0.5, "end": 1.0 }
            ]
        });
        let parsed: GenericWhisperResponse = serde_json::from_value(raw_json).unwrap();
        let doc = convert_to_transcript_doc(parsed, "test.wav", "en", 2.0).unwrap();
        assert_eq!(doc.passages.len(), 1);
        assert_eq!(doc.passages[0].words.len(), 2);
    }
    #[test]
    fn asr_preserves_engine_and_missing_confidence_and_rejects_mock() {
        let mut raw = serde_json::json!({"text":"Hello","segments":[{"text":"Hello","start":0,"end":1,"words":[{"word":"Hello","start":0,"end":1}]}],"engine":{"name":"faster-whisper","version":"actual","model":"medium","forced_alignment":false}});
        let parsed = serde_json::from_value(raw.clone()).unwrap();
        let doc = convert_to_transcript_doc(parsed, "test.wav", "en", 1.0).unwrap();
        assert_eq!(doc.passages[0].words[0].score, None);
        assert_eq!(doc.evidence["engine"]["model"], "medium");
        assert_eq!(doc.evidence["forced_alignment"], false);
        raw["is_mock"] = serde_json::json!(true);
        assert!(convert_to_transcript_doc(
            serde_json::from_value(raw).unwrap(),
            "test.wav",
            "en",
            1.0
        )
        .is_err());
    }
}
