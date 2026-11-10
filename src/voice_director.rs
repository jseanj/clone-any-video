use anyhow::{bail, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use crate::probe::probe_media;
use crate::project::{AssetTaskType, ProjectManifest, TaskStatus};
use crate::script::ScriptDocument;
use crate::timeline::{project_script_to_timeline, ResolvedTimeline};

/// 声音导演与角色音色规范 (Voice Prompt Specification)
/// Voice Direction: Cast the person through the sound & Compress into vivid direction
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct VoicePromptSpec {
    /// 剧本段落 ID
    pub segment_id: String,
    /// 说话角色标识 (Role Cue)
    pub role: String,
    /// 发音台词文本 (必须使用 SVML .speech 投影，确保读音准确)
    pub spoken_text: String,
    /// 角色音色、情绪与语调描述 (Vivid voice direction)
    pub voice_direction: String,
    /// 目标发音语言 ("en", "zh")
    #[serde(default = "default_language")]
    pub language: String,
    /// 预估规划时长秒数
    pub target_duration_seconds: f64,
}

fn default_language() -> String {
    "en".to_string()
}

/// 实际测得的段落语音时间跨度 (Speech Extent)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[allow(dead_code)]
pub struct SpeechExtent {
    pub segment_id: String,
    pub audio_path: PathBuf,
    pub duration_seconds: f64,
    pub sample_rate: u32,
    pub channels: u16,
    pub is_measured: bool,
}

/// 声音生成请求对象
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoiceGenerationRequest {
    pub spec: VoicePromptSpec,
    pub output_path: PathBuf,
    pub voice_preset: Option<String>,
}

/// 声音合成结果元数据清单
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoiceGenerationResult {
    pub segment_id: String,
    pub output_path: String,
    pub metadata_path: String,
    pub duration_seconds: f64,
    pub sample_rate: u32,
    pub channels: u16,
    pub provider: String,
    pub model: String,
    pub elapsed_ms: u64,
    pub spoken_text: String,
    pub voice_direction: String,
    #[serde(default)]
    pub voice_preset: Option<String>,
    #[serde(default)]
    pub language: String,
}

/// 审计发音文本与声音导演规范
pub fn audit_voice_spec(spec: &VoicePromptSpec) -> Result<Vec<String>> {
    let mut warnings = Vec::new();

    if spec.spoken_text.trim().is_empty() {
        bail!(
            "Spoken text cannot be empty for segment '{}'",
            spec.segment_id
        );
    }

    if spec.voice_direction.trim().is_empty() {
        warnings.push(format!(
            "Segment '{}' has empty voice_direction; defaulting to standard conversational narrator.",
            spec.segment_id
        ));
    }

    if spec.target_duration_seconds <= 0.2 {
        warnings.push(format!(
            "Target duration {:.2}s is extremely short for segment '{}'.",
            spec.target_duration_seconds, spec.segment_id
        ));
    }

    Ok(warnings)
}

/// 本地模拟/确定性离线声音生成器 (MockVoiceGenerator)
/// 优先使用 macOS 原生 `say` 命令录制 48kHz 16-bit PCM WAV 真实语音；
/// 在非 macOS 或 say 失败时，回退到 FFmpeg 音频合成，确保 100% 离线确定性可测。
#[derive(Debug, Default)]
pub struct MockVoiceGenerator;

impl MockVoiceGenerator {
    pub fn new() -> Self {
        Self
    }

    pub async fn generate(&self, req: &VoiceGenerationRequest) -> Result<VoiceGenerationResult> {
        let start = Instant::now();

        if let Some(parent) = req.output_path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .with_context(|| format!("Failed to create voice output dir: {:?}", parent))?;
        }

        let output_str = req.output_path.to_string_lossy().to_string();

        // 策略 1: 尝试调用 macOS 原生 say 命令直接录制 48kHz WAV 真实语音
        let say_status = Command::new("say")
            .arg("-o")
            .arg(&output_str)
            .arg("--data-format=LEI16@48000")
            .arg(&req.spec.spoken_text)
            .output();

        let used_provider = if let Ok(status) = say_status {
            if status.status.success() && req.output_path.exists() {
                "macos-native-tts (say)".to_string()
            } else {
                self.fallback_ffmpeg_tone(&req, &output_str)?
            }
        } else {
            self.fallback_ffmpeg_tone(&req, &output_str)?
        };

        // 通过我们的媒体探针测量合成音频的真实物理时长与参数
        let probe = probe_media(&req.output_path)
            .await
            .with_context(|| format!("Failed to probe synthesized voice: {:?}", req.output_path))?;

        let duration_seconds = probe.duration_seconds;
        let sample_rate = probe.audio_sample_rate.unwrap_or(48000);
        let channels = probe.audio_channels.unwrap_or(1) as u16;
        let elapsed_ms = start.elapsed().as_millis() as u64;

        let metadata_path = req.output_path.with_extension("json");
        let result = VoiceGenerationResult {
            segment_id: req.spec.segment_id.clone(),
            output_path: output_str,
            metadata_path: metadata_path.to_string_lossy().to_string(),
            duration_seconds,
            sample_rate,
            channels,
            provider: used_provider,
            model: "speech-synthesizer-v1".to_string(),
            elapsed_ms,
            spoken_text: req.spec.spoken_text.clone(),
            voice_direction: req.spec.voice_direction.clone(),
            voice_preset: req.voice_preset.clone(),
            language: req.spec.language.clone(),
        };

        let meta_json = serde_json::to_string_pretty(&result)?;
        tokio::fs::write(&metadata_path, meta_json).await?;

        crate::artifacts::record(
            &req.output_path,
            crate::artifacts::Mode::Mock,
            &result.provider,
        )?;
        Ok(result)
    }

    fn fallback_ffmpeg_tone(
        &self,
        req: &VoiceGenerationRequest,
        output_str: &str,
    ) -> Result<String> {
        let dur = req.spec.target_duration_seconds.max(0.5);
        let status = Command::new("ffmpeg")
            .arg("-f")
            .arg("lavfi")
            .arg("-i")
            .arg(format!("sine=frequency=440:duration={:.3}", dur))
            .arg("-ar")
            .arg("48000")
            .arg("-c:a")
            .arg("pcm_s16le")
            .arg("-y")
            .arg(output_str)
            .output()
            .with_context(|| "Failed to execute fallback ffmpeg tone generation")?;

        if !status.status.success() {
            let stderr = String::from_utf8_lossy(&status.stderr);
            bail!("FFmpeg fallback voice synthesis failed:\n{}", stderr);
        }

        Ok("local-ffmpeg-pcm".to_string())
    }
}

/// 远程 API 语音生成适配器 (OpenAI Audio Speech / ElevenLabs / Fish Audio)
pub struct RemoteVoiceGenerator {
    api_key: String,
    base_url: String,
    model: String,
    mimo: bool,
}

impl RemoteVoiceGenerator {
    pub fn new(api_key: String, base_url: Option<String>, model: Option<String>) -> Self {
        Self {
            api_key,
            base_url: base_url.unwrap_or_else(|| "https://api.openai.com/v1".to_string()),
            model: model.unwrap_or_else(|| "tts-1".to_string()),
            mimo: false,
        }
    }

    /// TTS credentials and routing are independent from image/video providers.
    pub fn from_environment() -> Result<Self> {
        let provider = std::env::var("TTS_PROVIDER").unwrap_or_else(|_| "openai".into());
        match provider.as_str() {
            "mimo" => {
                let key = std::env::var("MIMO_API_KEY").context("Set MIMO_API_KEY for MiMo TTS")?;
                if key.trim().is_empty() {
                    bail!("MIMO_API_KEY is empty");
                }
                let model =
                    std::env::var("MIMO_TTS_MODEL").unwrap_or_else(|_| "mimo-v2.5-tts".into());
                if model != "mimo-v2.5-tts" {
                    bail!("This adapter supports mimo-v2.5-tts preset voices; voice design/clone require separate inputs");
                }
                Ok(Self {
                    api_key: key,
                    base_url: std::env::var("MIMO_BASE_URL")
                        .unwrap_or_else(|_| "https://api.xiaomimimo.com/v1".into()),
                    model,
                    mimo: true,
                })
            }
            "openai" => Ok(Self::new(
                std::env::var("OPENAI_API_KEY")
                    .context("Set OPENAI_API_KEY or select TTS_PROVIDER=mimo with MIMO_API_KEY")?,
                std::env::var("OPENAI_BASE_URL").ok(),
                None,
            )),
            _ => bail!(
                "Unsupported TTS_PROVIDER '{}'; choose mimo or openai",
                provider
            ),
        }
    }

    fn request_body(&self, req: &VoiceGenerationRequest) -> serde_json::Value {
        if self.mimo {
            let voice = req
                .voice_preset
                .clone()
                .or_else(|| std::env::var("MIMO_TTS_VOICE").ok())
                .unwrap_or_else(|| "mimo_default".into());
            serde_json::json!({"model":self.model,"messages":[
                {"role":"user","content":req.spec.voice_direction},
                {"role":"assistant","content":req.spec.spoken_text}
            ],"audio":{"format":"wav","voice":voice},"stream":false})
        } else {
            serde_json::json!({"model":self.model,"input":req.spec.spoken_text,"voice":req.voice_preset.as_deref().unwrap_or("alloy"),"response_format":"wav"})
        }
    }

    pub async fn generate(&self, req: &VoiceGenerationRequest) -> Result<VoiceGenerationResult> {
        let start = Instant::now();

        audit_voice_spec(&req.spec)?;
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()?;
        let route = if self.mimo {
            "chat/completions"
        } else {
            "audio/speech"
        };
        let endpoint = format!("{}/{}", self.base_url.trim_end_matches('/'), route);
        let body = self.request_body(req);

        let resp = client
            .post(&endpoint)
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await
            .with_context(|| format!("Failed to send TTS request to {}", endpoint))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let err_text = resp
                .text()
                .await
                .unwrap_or_default()
                .replace(&self.api_key, "[REDACTED]");
            bail!("Voice generation API error {}: {}", status, err_text);
        }

        let audio_bytes = if self.mimo {
            let response: serde_json::Value =
                resp.json().await.context("Invalid MiMo JSON response")?;
            let data = response
                .pointer("/choices/0/message/audio/data")
                .and_then(|v| v.as_str())
                .context("MiMo response missing choices[0].message.audio.data")?;
            STANDARD
                .decode(data)
                .context("Invalid base64 audio from MiMo")?
        } else {
            resp.bytes().await?.to_vec()
        };
        if audio_bytes.is_empty() {
            bail!("TTS returned empty audio");
        }

        if let Some(parent) = req.output_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let parent = req
            .output_path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let staging = tempfile::Builder::new()
            .suffix(".wav")
            .tempfile_in(parent)?;
        tokio::fs::write(staging.path(), &audio_bytes).await?;
        let probe = probe_media(staging.path())
            .await
            .context("TTS returned invalid audio")?;
        if !probe.has_audio || probe.duration_seconds <= 0.0 {
            bail!("TTS response has no valid audio stream");
        }
        staging
            .persist(&req.output_path)
            .map_err(|e| e.error)
            .context("Failed publishing validated TTS audio")?;
        let duration_seconds = probe.duration_seconds;
        let sample_rate = probe.audio_sample_rate.unwrap_or(48000);
        let channels = probe.audio_channels.unwrap_or(1) as u16;
        let elapsed_ms = start.elapsed().as_millis() as u64;

        let metadata_path = req.output_path.with_extension("json");
        let result = VoiceGenerationResult {
            segment_id: req.spec.segment_id.clone(),
            output_path: req.output_path.to_string_lossy().to_string(),
            metadata_path: metadata_path.to_string_lossy().to_string(),
            duration_seconds,
            sample_rate,
            channels,
            provider: if self.mimo {
                "xiaomi-mimo"
            } else {
                "remote-tts-api"
            }
            .to_string(),
            model: self.model.clone(),
            elapsed_ms,
            spoken_text: req.spec.spoken_text.clone(),
            voice_direction: req.spec.voice_direction.clone(),
            voice_preset: if self.mimo {
                self.request_body(req)["audio"]["voice"]
                    .as_str()
                    .map(str::to_owned)
            } else {
                Some(req.voice_preset.clone().unwrap_or_else(|| "alloy".into()))
            },
            language: req.spec.language.clone(),
        };

        let meta_json = serde_json::to_string_pretty(&result)?;
        tokio::fs::write(&metadata_path, meta_json).await?;

        crate::artifacts::record_derived(
            &req.output_path,
            crate::artifacts::Mode::Production,
            &result.provider,
            &[],
            serde_json::json!({"voice_preset":result.voice_preset,"language":result.language,"model":result.model,"spoken_text":result.spoken_text,"direction":result.voice_direction,"target_seconds":req.spec.target_duration_seconds,"measured_seconds":duration_seconds,"within_budget":(duration_seconds-req.spec.target_duration_seconds).abs()<0.5}),
        )?;
        Ok(result)
    }
}

/// 扫描指定目录下的全部语音资产并精确提取实际时长 Extents 字典
pub async fn collect_speech_extents(
    audio_dir: &Path,
    script_doc: &ScriptDocument,
) -> Result<HashMap<String, f64>> {
    let mut extents = HashMap::new();

    for seg in &script_doc.segments {
        let possible_candidates = [
            audio_dir.join(format!("voice_{}.wav", seg.id)),
            audio_dir.join(format!("{}.wav", seg.id)),
            audio_dir.join(format!("voice_{}.mp3", seg.id)),
            audio_dir.join(format!("{}.mp3", seg.id)),
        ];

        let mut found_path = None;
        for c in &possible_candidates {
            if c.exists() {
                found_path = Some(c.clone());
                break;
            }
        }

        if let Some(path) = found_path {
            let meta = probe_media(&path)
                .await
                .with_context(|| format!("Failed to probe audio file: {:?}", path))?;
            extents.insert(seg.id.clone(), meta.duration_seconds);
        } else {
            bail!(
                "Missing prepared speech for segment {} in {:?}",
                seg.id,
                audio_dir
            );
        }
    }

    Ok(extents)
}

/// 二次时间线对齐与项目蓝图弹性拉伸解算
/// 将实际测得的音频时长喂给动态时间线求解器，自动更新全片所有 Shot 的时钟范围
pub fn realign_project_timeline(
    script_doc: &ScriptDocument,
    initial_manifest: &ProjectManifest,
    speech_extents: &HashMap<String, f64>,
    fps: f64,
    gap: f64,
) -> Result<(ResolvedTimeline, ProjectManifest)> {
    // 调用第 6 章的 DAG 求解器，传入实际实测的 speech_extents 进行弹性拉伸
    let resolved_tl = project_script_to_timeline(script_doc, fps, gap, Some(speech_extents))
        .context("Failed to re-solve timeline with measured speech extents")?;

    // Keep the authored edit, including shots spanning multiple speech segments.
    // Speech boundaries form a piecewise linear map from design time to performed time.
    let mut aligned_manifest = initial_manifest.clone();
    let mut knots = Vec::new();
    let mut performed_start = 0.0;
    for seg in &script_doc.segments {
        let duration = *speech_extents
            .get(&seg.id)
            .with_context(|| format!("Missing measured speech extent for {}", seg.id))?;
        if !duration.is_finite() || duration <= 0.0 {
            bail!("Invalid speech extent for {}: {}", seg.id, duration);
        }
        let voice_id = format!("voice_{}", seg.id);
        let original = initial_manifest
            .tracks
            .voice
            .iter()
            .find(|v| v.id == voice_id)
            .with_context(|| format!("Missing authored voice window {}", voice_id))?;
        knots.push((original.start_seconds, performed_start));
        knots.push((original.end_seconds, performed_start + duration));
        let voice = aligned_manifest
            .tracks
            .voice
            .iter_mut()
            .find(|v| v.id == voice_id)
            .unwrap();
        voice.start_seconds = performed_start;
        voice.end_seconds = performed_start + duration;
        voice.start_frame = (voice.start_seconds * fps).round() as u64;
        voice.end_frame = (voice.end_seconds * fps).ceil() as u64;
        voice.payload["timing_status"] = serde_json::json!("measured_and_aligned");
        voice.payload["spoken_text"] = serde_json::json!(seg.spoken_text);
        performed_start += duration + gap;
    }
    let performed_end = (performed_start - gap).max(0.0);
    aligned_manifest.meta.fps = fps;
    aligned_manifest.meta.total_frames = (performed_end * fps).ceil() as u64;
    aligned_manifest.meta.total_seconds = aligned_manifest.meta.total_frames as f64 / fps;
    if initial_manifest.meta.total_seconds > knots.last().unwrap().0 + 1e-6 {
        knots.push((
            initial_manifest.meta.total_seconds,
            aligned_manifest.meta.total_seconds,
        ));
    }
    knots.sort_by(|a, b| a.0.total_cmp(&b.0));
    let warp = |t: f64| -> f64 {
        for pair in knots.windows(2) {
            let ((x0, y0), (x1, y1)) = (pair[0], pair[1]);
            if t <= x1 && x1 > x0 {
                return y0 + ((t - x0) / (x1 - x0)).clamp(0.0, 1.0) * (y1 - y0);
            }
        }
        knots.last().unwrap().1
    };
    for shot in &mut aligned_manifest.shots {
        shot.start_frame = (warp(shot.start_seconds) * fps).round() as u64;
        shot.end_frame = (warp(shot.end_seconds) * fps).round() as u64;
        if (shot.end_seconds - initial_manifest.meta.total_seconds).abs() < 1e-6 {
            shot.end_frame = aligned_manifest.meta.total_frames;
        }
        shot.start_seconds = shot.start_frame as f64 / fps;
        shot.end_seconds = shot.end_frame as f64 / fps;
        shot.duration_seconds = shot.end_seconds - shot.start_seconds;
    }
    for tracks in [
        &mut aligned_manifest.tracks.visual_bg,
        &mut aligned_manifest.tracks.visual_components,
        &mut aligned_manifest.tracks.captions,
        &mut aligned_manifest.tracks.sfx,
        &mut aligned_manifest.tracks.bgm,
    ] {
        for track in tracks {
            track.start_seconds = warp(track.start_seconds);
            track.end_seconds = warp(track.end_seconds);
            track.start_frame = (track.start_seconds * fps).round() as u64;
            track.end_frame = (track.end_seconds * fps).round() as u64;
        }
    }
    for bg in &mut aligned_manifest.tracks.visual_bg {
        if let Some(shot) = aligned_manifest
            .shots
            .iter()
            .find(|s| bg.payload["shot_id"] == s.id)
        {
            bg.start_seconds = shot.start_seconds;
            bg.end_seconds = shot.end_seconds;
            bg.start_frame = shot.start_frame;
            bg.end_frame = shot.end_frame;
        }
    }
    for task in &mut aligned_manifest.asset_tasks {
        if task.task_type == AssetTaskType::TtsSpeech {
            task.status = TaskStatus::Ready;
        }
    }

    Ok((resolved_tl, aligned_manifest))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::create_project_manifest;
    use crate::script::parse_script;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_mock_voice_generator_creates_valid_wav() {
        let dir = tempdir().unwrap();
        let out_wav = dir.path().join("test_voice.wav");

        let spec = VoicePromptSpec {
            segment_id: "test_seg".to_string(),
            role: "HOST".to_string(),
            spoken_text: "Hello world, this is a test of voice direction.".to_string(),
            voice_direction: "Warm friendly conversational tone".to_string(),
            language: "en".to_string(),
            target_duration_seconds: 2.0,
        };

        let req = VoiceGenerationRequest {
            spec,
            output_path: out_wav.clone(),
            voice_preset: None,
        };

        let gen = MockVoiceGenerator::new();
        let res = gen.generate(&req).await.expect("Mock TTS should succeed");

        assert!(out_wav.exists(), "Synthesized WAV file must exist");
        assert!(res.duration_seconds > 0.0);
        assert_eq!(res.sample_rate, 48000);

        let meta_file = dir.path().join("test_voice.json");
        assert!(meta_file.exists(), "Sidecar JSON must exist");
    }

    #[test]
    fn realign_preserves_cross_segment_shots_and_overlays() {
        let doc = parse_script("<script id=\"x\"><hook><HOST>One two</hook><identity><HOST>Three four</identity></script>").unwrap();
        let tl = project_script_to_timeline(&doc, 30.0, 0.3, None).unwrap();
        let mut manifest = create_project_manifest(&doc, &tl, None, 640, 360).unwrap();
        let mut spanning = manifest.shots[0].clone();
        spanning.end_seconds = manifest.meta.total_seconds;
        spanning.duration_seconds = spanning.end_seconds;
        manifest.shots = vec![spanning];
        let mut overlay = manifest.tracks.voice[1].clone();
        overlay.id = "authored_overlay".into();
        manifest.tracks.visual_components.push(overlay);
        let extents = HashMap::from([("hook".into(), 3.00001), ("identity".into(), 1.00001)]);
        let (_, aligned) = realign_project_timeline(&doc, &manifest, &extents, 30.0, 0.3).unwrap();
        assert_eq!(aligned.shots.len(), 1);
        assert_eq!(aligned.shots[0].description, manifest.shots[0].description);
        assert_eq!(aligned.tracks.visual_components.len(), 1);
        assert_eq!(aligned.meta.total_frames, 130);
        assert!((aligned.tracks.voice[1].start_seconds - 3.30001).abs() < 1e-9);
        assert!(aligned.meta.total_seconds >= aligned.tracks.voice[1].end_seconds);
        assert!(realign_project_timeline(&doc, &manifest, &HashMap::new(), 30.0, 0.3).is_err());
    }

    #[test]
    fn test_realign_project_timeline_stretches_shots() {
        let svml = r#"
        <script id="promo">
          <hook><HOST>Want five video tasks in one?</hook>
          <identity><HOST>clone any video</identity>
        </script>
        "#;
        let doc = parse_script(svml).unwrap();

        // 初始估算时间线
        let init_tl = project_script_to_timeline(&doc, 30.0, 0.3, None).unwrap();
        let init_manifest =
            create_project_manifest(&doc, &init_tl, Some("Test"), 640, 360).unwrap();

        // 模拟真实测得的语音时长：hook 变长为 4.5s，identity 为 2.0s
        let mut extents = HashMap::new();
        extents.insert("hook".to_string(), 4.5);
        extents.insert("identity".to_string(), 2.0);

        let (aligned_tl, aligned_manifest) =
            realign_project_timeline(&doc, &init_manifest, &extents, 30.0, 0.3).unwrap();

        assert_eq!(aligned_tl.total_frames, 204); // 4.5s + 0.3s + 2.0s = 6.8s -> 204 frames
        assert_eq!(aligned_manifest.shots.len(), 2);
        assert_eq!(aligned_manifest.shots[0].duration_seconds, 4.5);
        assert_eq!(aligned_manifest.shots[1].duration_seconds, 2.0);
    }
}
