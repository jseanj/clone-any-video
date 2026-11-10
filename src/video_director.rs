use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::process::Command;
use std::time::Instant;

/// 摄像机运镜行为 (Camera Motion Behaviors)
/// Video Direction: Direct camera and cuts as part of the passage
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum CameraMotion {
    /// 固定机位与微动/自然呼吸感 (Fixed tripod, subtle natural breathing)
    StaticTripod,
    /// 推进运镜 / 镜头推向主体 (Slow cinematic push-in towards subject)
    PushIn,
    /// 拉远运镜 / 展现更宽广环境 (Smooth pull-out revealing environment)
    PullOut,
    /// 向左横摇 (Pan camera from right to left)
    PanLeft,
    /// 向右横摇 (Pan camera from left to right)
    PanRight,
    /// 向上俯仰 (Tilt camera upward)
    TiltUp,
    /// 向下俯仰 (Tilt camera downward)
    TiltDown,
    /// 环绕运镜 (Arc or orbit motion around subject)
    Orbit,
    /// 手机实拍轻微手持微晃 (Natural handheld micro-shake, UGC cadence)
    HandheldUgc,
}

impl CameraMotion {
    pub fn as_str(&self) -> &'static str {
        match self {
            CameraMotion::StaticTripod => "static_tripod",
            CameraMotion::PushIn => "push_in",
            CameraMotion::PullOut => "pull_out",
            CameraMotion::PanLeft => "pan_left",
            CameraMotion::PanRight => "pan_right",
            CameraMotion::TiltUp => "tilt_up",
            CameraMotion::TiltDown => "tilt_down",
            CameraMotion::Orbit => "orbit",
            CameraMotion::HandheldUgc => "handheld_ugc",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().replace('-', "_").as_str() {
            "push_in" | "pushin" | "zoom_in" => CameraMotion::PushIn,
            "pull_out" | "pullout" | "zoom_out" => CameraMotion::PullOut,
            "pan_left" | "panleft" => CameraMotion::PanLeft,
            "pan_right" | "panright" => CameraMotion::PanRight,
            "tilt_up" | "tiltup" => CameraMotion::TiltUp,
            "tilt_down" | "tiltdown" => CameraMotion::TiltDown,
            "orbit" | "arc" => CameraMotion::Orbit,
            "handheld" | "handheld_ugc" | "ugc" => CameraMotion::HandheldUgc,
            _ => CameraMotion::StaticTripod,
        }
    }

    pub fn description(&self) -> &'static str {
        match self {
            CameraMotion::StaticTripod => "Fixed camera position with natural subtle idle drift and performer breathing.",
            CameraMotion::PushIn => "Slow, deliberate cinematic push-in toward the focal subject, tightening the composition.",
            CameraMotion::PullOut => "Smooth, steady camera pull-out revealing broader contextual space and surrounding elements.",
            CameraMotion::PanLeft => "Smooth horizontal pan from right to left across the active visual scene.",
            CameraMotion::PanRight => "Smooth horizontal pan from left to right across the active visual scene.",
            CameraMotion::TiltUp => "Gentle vertical camera tilt moving upward to reveal vertical height and architectural features.",
            CameraMotion::TiltDown => "Gentle vertical camera tilt moving downward following motion to ground level.",
            CameraMotion::Orbit => "Curved arc motion maintaining focus on the primary subject while shifting the background perspective.",
            CameraMotion::HandheldUgc => "Organic micro-shake and natural handheld camera feel typical of mobile phone video.",
        }
    }
}

/// 视频生成关系类型 (Video Generation Relationships)
/// 运镜模式: Text-directed, First-/last-frame, Reference-directed
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum VideoGenRelationship {
    /// 图生视频：以首帧静态图为起点驱动连续动态 (First-frame driven image-to-video)
    ImageToVideo,
    /// 双关键帧驱动：约束起始帧与结束帧 (First/Last frame boundary constrained)
    FirstLastFrame,
    /// 纯提示词文本生视频 (Autonomous text-to-video)
    TextToVideo,
    /// 参考视频动态迁移 (Reference motion/temporal transfer)
    ReferenceTransfer,
    /// Multiple declared still-image references (not reference-video motion transfer).
    ReferenceImages,
    /// Edit the actual source footage, preserving everything outside the requested changes.
    SourceVideoEdit,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FlowUploadBinding {
    pub project_id: String,
    pub media_id: String,
    pub content_digest: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceVideoEdit {
    #[serde(default)]
    pub start_frame: u32,
    /// Exclusive end in Flow's 30 fps editing timebase, independent of source fps.
    pub end_frame: u32,
    /// Explicit existing uploads, bound to project and the declared local media bytes.
    #[serde(default)]
    pub uploads: Vec<FlowUploadBinding>,
}
impl SourceVideoEdit {
    pub fn validate(&self, duration_seconds: f64) -> Result<()> {
        for binding in &self.uploads {
            crate::flowkit::validate_uuid(&binding.project_id)?;
            crate::flowkit::validate_uuid(&binding.media_id)?;
            if binding.content_digest.len() != 16
                || !binding
                    .content_digest
                    .chars()
                    .all(|c| c.is_ascii_hexdigit())
            {
                bail!("Flow upload bindings need a content_digest from media inspect/admit");
            }
        }
        if self.end_frame <= self.start_frame || self.end_frame - self.start_frame > 300 {
            bail!("Source video edit needs 1..300 frames (at most 10 seconds) in Flow's 30 fps edit timebase");
        }
        let duration = (self.end_frame - self.start_frame) as f64 / 30.0;
        if !duration_seconds.is_finite() || (duration_seconds - duration).abs() > 1.0 / 30.0 + 1e-6
        {
            bail!("Source video edit duration must match its declared frame range (within one edit frame)");
        }
        Ok(())
    }
}

/// 动态视频导演规格 (Video Prompt Specification)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct VideoPromptSpec {
    pub shot_id: String,
    pub first_frame_path: Option<PathBuf>,
    pub motion_prompt: String,
    pub camera_motion: CameraMotion,
    pub relationship: VideoGenRelationship,
    pub duration_seconds: f64,
    pub fps: f64,
    pub width: u32,
    pub height: u32,
}

/// Additional media ports remain separate from the legacy first-frame specification.
#[derive(Debug, Clone, Default)]
pub struct VideoInputs {
    pub last_frame: Option<PathBuf>,
    pub reference_images: Vec<PathBuf>,
    pub source_video: Option<PathBuf>,
    pub source_edit: Option<SourceVideoEdit>,
}
pub fn validate_inputs(spec: &VideoPromptSpec, inputs: &VideoInputs) -> Result<()> {
    let first = spec.first_frame_path.is_some();
    let last = inputs.last_frame.is_some();
    let refs = !inputs.reference_images.is_empty();
    let source = inputs.source_video.is_some();
    let edit = inputs.source_edit.is_some();
    if inputs.reference_images.len() > 7 {
        bail!("FlowKit accepts at most seven reference images");
    }
    match spec.relationship {
        VideoGenRelationship::ImageToVideo if first && !last && !refs && !source && !edit => Ok(()),
        VideoGenRelationship::FirstLastFrame if first && last && !refs && !source && !edit => Ok(()),
        VideoGenRelationship::TextToVideo if !first && !last && !refs && !source && !edit => Ok(()),
        VideoGenRelationship::ReferenceImages if refs && !first && !last && !source && !edit => Ok(()),
        VideoGenRelationship::SourceVideoEdit if source && edit && !first && !last => {
            inputs.source_edit.as_ref().unwrap().validate(spec.duration_seconds)
        }
        VideoGenRelationship::ReferenceTransfer => bail!("Unsupported video relationship: reference-video motion inputs are not wired"),
        _ => bail!("Video relationship requires matching declared media: ImageToVideo needs only first_frame; FirstLastFrame needs first_frame and last_frame; ReferenceImages needs only references; TextToVideo has none"),
    }
}
pub fn validate_relationship(spec: &VideoPromptSpec) -> Result<()> {
    validate_inputs(spec, &VideoInputs::default())
}

/// Fresh visible performance: Script supplies words; the model supplies the voice.
/// This never supplies original audio or asks a source-video editor to change speech.
pub fn attach_dialogue(spec: &mut VideoPromptSpec, doc: &crate::script::ScriptDocument) -> Result<()> {
    if spec.relationship == VideoGenRelationship::SourceVideoEdit {
        bail!("Flow policy prevents editing or generating speech for people in uploaded videos; use visual-only source editing or an explicitly authored new performance");
    }
    if doc.segments.len() != 1 || doc.segments[0].turns.len() != 1 || doc.segments[0].spoken_text.trim().is_empty() {
        bail!("Speaking video needs one nonempty explicit Script segment/turn");
    }
    spec.motion_prompt += &format!("\nVisible speaking performance with its own audible speech. Exact spoken dialogue: {}. The visible presenter speaks these words in order with synchronized lips. Choose a natural voice; no voice matching or audio reference is requested. No subtitles or music.",doc.segments[0].spoken_text);
    Ok(())
}

impl Default for VideoPromptSpec {
    fn default() -> Self {
        Self {
            shot_id: "shot_01".to_string(),
            first_frame_path: None,
            motion_prompt: "Smooth continuous motion, presenter gestures with free hand, calm expressive attitude."
                .to_string(),
            camera_motion: CameraMotion::PushIn,
            relationship: VideoGenRelationship::ImageToVideo,
            duration_seconds: 3.0,
            fps: 30.0,
            width: 640,
            height: 360,
        }
    }
}

/// 视频提示词审计报告
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct VideoPromptAudit {
    pub is_valid: bool,
    pub duration_seconds: f64,
    pub total_frames: u64,
    pub has_first_frame: bool,
    pub has_camera_motion: bool,
    pub has_expressive_reason: bool,
    pub warnings: Vec<String>,
    pub strengths: Vec<String>,
}

/// 视频生成请求对象
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VideoGenerationRequest {
    pub spec: VideoPromptSpec,
    pub output_path: PathBuf,
    pub seed: Option<u64>,
}

/// 视频生成结果元数据清单
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VideoGenerationResult {
    pub shot_id: String,
    pub output_path: String,
    pub metadata_path: String,
    pub duration_seconds: f64,
    pub frames_count: u64,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub camera_motion: String,
    pub relationship: String,
    pub provider: String,
    pub model: String,
    pub prompt_hash: String,
    pub elapsed_ms: u64,
    pub full_prompt: String,
}

/// 组装视频导演完整 Prompt
pub fn assemble_video_prompt(spec: &VideoPromptSpec) -> String {
    if spec.relationship == VideoGenRelationship::SourceVideoEdit {
        // A legacy camera default must never add a push-in or new timing to source editing.
        return spec.motion_prompt.trim().to_owned();
    }
    format!(
        "Camera Motion: {}.\nAction & Performance: {}\nTemporal Continuity: Maintain consistent lighting, color palette, and physical geometry. Duration: {:.2}s at {:.1}fps.",
        spec.camera_motion.description(),
        spec.motion_prompt.trim(),
        spec.duration_seconds,
        spec.fps
    )
}

/// 审计视频导演规格合规性
pub fn audit_video_prompt(spec: &VideoPromptSpec) -> VideoPromptAudit {
    let mut warnings = Vec::new();
    let mut strengths = Vec::new();

    let total_frames = (spec.duration_seconds * spec.fps).round() as u64;

    // 1. 时长合规性 (0.2s ~ 30.0s)
    if spec.duration_seconds <= 0.2 {
        warnings.push(format!(
            "Duration {:.2}s is too short for a readable video clip (minimum 0.3s recommended).",
            spec.duration_seconds
        ));
    } else if spec.duration_seconds > 15.0 {
        warnings.push(format!(
            "Duration {:.2}s exceeds standard single-shot model limits (consider splitting into multiple cuts).",
            spec.duration_seconds
        ));
    } else {
        strengths.push(format!(
            "Duration {:.2}s ({} frames at {:.1}fps) is optimal for generation.",
            spec.duration_seconds, total_frames, spec.fps
        ));
    }

    // 2. 首帧引用校验
    let has_first_frame = if let Some(ref path) = spec.first_frame_path {
        if path.exists() {
            strengths.push(format!("First frame image asset verified: {:?}", path));
            true
        } else {
            warnings.push(format!(
                "First frame path specified but file not found: {:?}",
                path
            ));
            false
        }
    } else {
        if spec.relationship == VideoGenRelationship::ImageToVideo {
            warnings.push(
                "ImageToVideo relationship selected, but no first_frame_path provided.".to_string(),
            );
        }
        false
    };

    // 3. 运镜与意图
    let has_camera_motion = true;
    strengths.push(format!(
        "Authored camera behavior: {:?}",
        spec.camera_motion
    ));

    let lower_motion = spec.motion_prompt.to_lowercase();
    let expressive_keywords = [
        "gesture", "look", "attitude", "speak", "nod", "smile", "turn", "move", "express", "calm",
        "fast", "converge", "brighten", "reveal", "lift",
    ];
    let has_expressive_reason = expressive_keywords.iter().any(|k| lower_motion.contains(k));
    if has_expressive_reason {
        strengths.push("Motion prompt grounds performance in clear expressive action.".to_string());
    } else {
        warnings.push(
            "Motion prompt lacks concrete action verbs (e.g. gesture, turn, reveal, converge)."
                .to_string(),
        );
    }

    let is_valid = spec.duration_seconds > 0.0 && !spec.motion_prompt.trim().is_empty();

    VideoPromptAudit {
        is_valid,
        duration_seconds: spec.duration_seconds,
        total_frames,
        has_first_frame,
        has_camera_motion,
        has_expressive_reason,
        warnings,
        strengths,
    }
}

/// 计算视频提示词哈希指纹
pub fn compute_video_prompt_hash(prompt: &str) -> String {
    let mut hasher = DefaultHasher::new();
    prompt.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// 本地模拟/确定性离线视频生成器 (MockVideoGenerator)
/// 基于 FFmpeg 与图像资产，合成带有精确运镜（zoompan/pan/tilt）、指定时长与帧率的真实 H.264 MP4 视频
#[derive(Debug, Default)]
pub struct MockVideoGenerator;

impl MockVideoGenerator {
    pub fn new() -> Self {
        Self
    }

    pub fn generate(&self, req: &VideoGenerationRequest) -> Result<VideoGenerationResult> {
        validate_relationship(&req.spec)?;
        let start = Instant::now();
        let full_prompt = assemble_video_prompt(&req.spec);
        let prompt_hash = compute_video_prompt_hash(&full_prompt);

        if let Some(parent) = req.output_path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create output dir: {:?}", parent))?;
        }

        let total_frames = (req.spec.duration_seconds * req.spec.fps).round().max(1.0) as u64;
        let width = req.spec.width;
        let height = req.spec.height;
        let fps = req.spec.fps;

        // 根据 CameraMotion 设计 FFmpeg 运镜滤镜 (zoompan filter)
        let vf_filter = match req.spec.camera_motion {
            CameraMotion::PushIn => {
                format!(
                    "zoompan=z='min(zoom+0.0015,1.18)':x='iw/2-(iw/zoom/2)':y='ih/2-(ih/zoom/2)':d={}:s={}x{}:fps={}",
                    total_frames, width, height, fps
                )
            }
            CameraMotion::PullOut => {
                format!(
                    "zoompan=z='if(lte(zoom,1.0),1.18,max(1.001,zoom-0.0015))':x='iw/2-(iw/zoom/2)':y='ih/2-(ih/zoom/2)':d={}:s={}x{}:fps={}",
                    total_frames, width, height, fps
                )
            }
            CameraMotion::PanRight => {
                format!(
                    "zoompan=z=1.12:x='min(x+1.5,iw-iw/zoom)':y='ih/2-(ih/zoom/2)':d={}:s={}x{}:fps={}",
                    total_frames, width, height, fps
                )
            }
            CameraMotion::PanLeft => {
                format!(
                    "zoompan=z=1.12:x='max(x-1.5,0)':y='ih/2-(ih/zoom/2)':d={}:s={}x{}:fps={}",
                    total_frames, width, height, fps
                )
            }
            CameraMotion::TiltDown => {
                format!(
                    "zoompan=z=1.12:x='iw/2-(iw/zoom/2)':y='min(y+1.2,ih-ih/zoom)':d={}:s={}x{}:fps={}",
                    total_frames, width, height, fps
                )
            }
            CameraMotion::TiltUp => {
                format!(
                    "zoompan=z=1.12:x='iw/2-(iw/zoom/2)':y='max(y-1.2,0)':d={}:s={}x{}:fps={}",
                    total_frames, width, height, fps
                )
            }
            CameraMotion::StaticTripod => {
                format!(
                    "zoompan=z=1:x=0:y=0:d={}:s={}x{}:fps={}",
                    total_frames, width, height, fps
                )
            }
            _ => {
                // Offline approximation for motions needing depth or handheld behavior.
                format!(
                    "zoompan=z='1.02+0.01*sin(2*PI*on/60)':x='iw/2-(iw/zoom/2)':y='ih/2-(ih/zoom/2)':d={}:s={}x{}:fps={}",
                    total_frames, width, height, fps
                )
            }
        };

        // 检查输入源图像
        let input_arg = if let Some(ref img_path) = req.spec.first_frame_path {
            if img_path.exists() {
                img_path.to_string_lossy().to_string()
            } else {
                bail!("Specified first_frame_path does not exist: {:?}", img_path);
            }
        } else {
            // 若未提供，利用临时渐变色块
            bail!("MockVideoGenerator requires first_frame_path to simulate Image-to-Video generation");
        };

        // 调用系统 FFmpeg 执行编码合成真实 H.264 MP4 视频
        let output_str = req.output_path.to_string_lossy().to_string();
        let status = Command::new("ffmpeg")
            .arg("-loop")
            .arg("1")
            .arg("-i")
            .arg(&input_arg)
            .arg("-vf")
            .arg(&vf_filter)
            .arg("-frames:v")
            .arg(total_frames.to_string())
            .arg("-c:v")
            .arg("libx264")
            .arg("-pix_fmt")
            .arg("yuv420p")
            .arg("-y")
            .arg(&output_str)
            .output()
            .with_context(|| "Failed to execute ffmpeg command for video generation")?;

        if !status.status.success() {
            let stderr = String::from_utf8_lossy(&status.stderr);
            bail!("FFmpeg video synthesis failed:\n{}", stderr);
        }

        let elapsed_ms = start.elapsed().as_millis() as u64;
        let metadata_path = req.output_path.with_extension("json");

        let result = VideoGenerationResult {
            shot_id: req.spec.shot_id.clone(),
            output_path: output_str,
            metadata_path: metadata_path.to_string_lossy().to_string(),
            duration_seconds: total_frames as f64 / fps,
            frames_count: total_frames,
            width,
            height,
            fps,
            camera_motion: req.spec.camera_motion.as_str().to_string(),
            relationship: format!("{:?}", req.spec.relationship),
            provider: "local-ffmpeg-synthesizer".to_string(),
            model: "motion-director-v1".to_string(),
            prompt_hash,
            elapsed_ms,
            full_prompt,
        };

        let meta_json = serde_json::to_string_pretty(&result)?;
        std::fs::write(&metadata_path, meta_json)?;

        crate::artifacts::record(
            &req.output_path,
            crate::artifacts::Mode::Mock,
            &result.provider,
        )?;
        Ok(result)
    }
}

/// 远程 API 视频生成适配器 (Runway / Luma / MiniMax / Kling / Seedance Proxy)
pub struct RemoteVideoGenerator {
    api_key: String,
    base_url: String,
    model: String,
}

impl RemoteVideoGenerator {
    pub fn new(api_key: String, base_url: Option<String>, model: Option<String>) -> Self {
        Self {
            api_key,
            base_url: base_url.unwrap_or_else(|| "https://api.openai.com/v1".to_string()),
            model: model.unwrap_or_else(|| "sora".to_string()),
        }
    }

    pub async fn generate(&self, req: &VideoGenerationRequest) -> Result<VideoGenerationResult> {
        validate_relationship(&req.spec)?;
        let start = Instant::now();
        let full_prompt = assemble_video_prompt(&req.spec);
        let prompt_hash = compute_video_prompt_hash(&full_prompt);

        let client = reqwest::Client::new();
        let endpoint = format!("{}/videos/generations", self.base_url.trim_end_matches('/'));

        // 针对 Image-to-Video 第一帧输入，若为本地路径，转换为标准 Base64 Data URI 以便远程 API 可靠读取
        let first_frame_ref = if let Some(path) = &req.spec.first_frame_path {
            if path.exists() {
                let bytes = tokio::fs::read(path)
                    .await
                    .with_context(|| format!("Failed to read first frame at {:?}", path))?;
                let b64 = base64_encode(&bytes);
                let mime = if path.extension().and_then(|e| e.to_str()) == Some("jpg")
                    || path.extension().and_then(|e| e.to_str()) == Some("jpeg")
                {
                    "image/jpeg"
                } else {
                    "image/png"
                };
                Some(format!("data:{};base64,{}", mime, b64))
            } else {
                Some(path.to_string_lossy().to_string())
            }
        } else {
            None
        };

        let body = serde_json::json!({
            "model": self.model,
            "prompt": full_prompt,
            "duration": req.spec.duration_seconds,
            "fps": req.spec.fps,
            "camera_motion": req.spec.camera_motion.as_str(),
            "first_frame": first_frame_ref,
        });

        let resp = client
            .post(&endpoint)
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await
            .with_context(|| format!("Failed to send video generation request to {}", endpoint))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let err_text = resp.text().await.unwrap_or_default();
            bail!("Video generation API error {}: {}", status, err_text);
        }

        let resp_json: serde_json::Value = resp.json().await?;
        let video_url = resp_json["data"][0]["url"]
            .as_str()
            .context("Missing video URL in API response")?;

        let download_resp = reqwest::get(video_url).await?;
        if !download_resp.status().is_success() {
            bail!(
                "Failed to download video from {}: HTTP {}",
                video_url,
                download_resp.status()
            );
        }
        let video_bytes = download_resp.bytes().await?;

        // 暂存至临时下载文件，校验通过前严禁写入正式产物
        let temp_download = req.output_path.with_extension("downloading.mp4");
        if let Some(parent) = temp_download.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&temp_download, &video_bytes)?;

        // 真实媒体探测与最低规格校验：严禁盲目信任请求声明时长或 .ok() 吞并错误
        let probed = match crate::probe::probe_media(&temp_download).await {
            Ok(p) => p,
            Err(e) => {
                let _ = std::fs::remove_file(&temp_download);
                bail!(
                    "Downloaded video file from {} is corrupt or invalid media: {}",
                    video_url,
                    e
                );
            }
        };

        if probed.duration_seconds <= 0.0 || probed.width.unwrap_or(0) == 0 {
            let _ = std::fs::remove_file(&temp_download);
            bail!(
                "Downloaded video file has invalid duration ({:.2}s) or dimensions ({:?}x{:?})",
                probed.duration_seconds,
                probed.width,
                probed.height
            );
        }

        // 探测通过，发布正式输出视频
        std::fs::rename(&temp_download, &req.output_path)?;

        let measured_dur = probed.duration_seconds;
        let measured_w = probed.width.unwrap_or(req.spec.width);
        let measured_h = probed.height.unwrap_or(req.spec.height);
        let measured_fps = probed.fps.unwrap_or(req.spec.fps);
        let total_frames = (measured_dur * measured_fps).round() as u64;

        let elapsed_ms = start.elapsed().as_millis() as u64;
        let metadata_path = req.output_path.with_extension("json");

        let result = VideoGenerationResult {
            shot_id: req.spec.shot_id.clone(),
            output_path: req.output_path.to_string_lossy().to_string(),
            metadata_path: metadata_path.to_string_lossy().to_string(),
            duration_seconds: measured_dur,
            frames_count: total_frames,
            width: measured_w,
            height: measured_h,
            fps: measured_fps,
            camera_motion: req.spec.camera_motion.as_str().to_string(),
            relationship: format!("{:?}", req.spec.relationship),
            provider: "remote-video-api".to_string(),
            model: self.model.clone(),
            prompt_hash,
            elapsed_ms,
            full_prompt,
        };

        let meta_json = serde_json::to_string_pretty(&result)?;
        std::fs::write(&metadata_path, meta_json)?;

        crate::artifacts::record(
            &req.output_path,
            crate::artifacts::combined(req.spec.first_frame_path.iter().map(|p| p.as_path())),
            &result.provider,
        )?;
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn new_performance_uses_script_without_voice_reference_and_rejects_source_speech_edit() {
        let script = crate::script::parse_script("<script><hook><HOST><Display name | Spoken name>. Try this serum.</hook></script>").unwrap();
        let mut spec = super::VideoPromptSpec::default();
        super::attach_dialogue(&mut spec,&script).unwrap();
        assert!(spec.motion_prompt.contains("Spoken name"));
        assert!(!spec.motion_prompt.contains("Display name"));
        assert!(spec.motion_prompt.contains("no voice matching or audio reference"));
        spec.relationship = super::VideoGenRelationship::SourceVideoEdit;
        assert!(super::attach_dialogue(&mut spec,&script).is_err());
        let multiple = crate::script::parse_script("<script><hook><HOST>First.</hook><end><HOST>Second.</end></script>").unwrap();
        spec.relationship = super::VideoGenRelationship::ImageToVideo;
        assert!(super::attach_dialogue(&mut spec,&multiple).is_err());
    }
    #[test]
    fn source_edit_preserves_prompt_and_checks_timebase_and_ports() {
        let spec = super::VideoPromptSpec {
            relationship: super::VideoGenRelationship::SourceVideoEdit,
            first_frame_path: None,
            motion_prompt: "Replace only the product; preserve the source camera.".into(),
            duration_seconds: 10.0,
            ..Default::default()
        };
        let mut inputs = super::VideoInputs {
            source_video: Some("source.mp4".into()),
            source_edit: Some(super::SourceVideoEdit {
                start_frame: 0,
                end_frame: 300,
                uploads: vec![],
            }),
            reference_images: vec!["product.png".into()],
            ..Default::default()
        };
        assert!(super::validate_inputs(&spec, &inputs).is_ok());
        assert_eq!(super::assemble_video_prompt(&spec), spec.motion_prompt);
        inputs.last_frame = Some("last.png".into());
        assert!(super::validate_inputs(&spec, &inputs).is_err());
        inputs.last_frame = None;
        inputs.source_edit.as_mut().unwrap().end_frame = 301;
        assert!(super::validate_inputs(&spec, &inputs).is_err());
        inputs.source_edit.as_mut().unwrap().end_frame = 240;
        assert!(super::validate_inputs(&spec, &inputs).is_err());
    }
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_camera_motion_parsing() {
        assert_eq!(CameraMotion::from_str("push_in"), CameraMotion::PushIn);
        assert_eq!(CameraMotion::from_str("zoom_out"), CameraMotion::PullOut);
        assert_eq!(CameraMotion::from_str("pan-left"), CameraMotion::PanLeft);
        assert_eq!(
            CameraMotion::from_str("handheld"),
            CameraMotion::HandheldUgc
        );
        assert_eq!(
            CameraMotion::from_str("unknown"),
            CameraMotion::StaticTripod
        );
    }

    #[test]
    fn test_audit_video_prompt() {
        let spec = VideoPromptSpec {
            shot_id: "shot_01".to_string(),
            first_frame_path: None,
            motion_prompt: "Presenter nods and turns to camera with a warm smile.".to_string(),
            camera_motion: CameraMotion::PushIn,
            relationship: VideoGenRelationship::TextToVideo,
            duration_seconds: 2.5,
            fps: 30.0,
            width: 640,
            height: 360,
        };

        let audit = audit_video_prompt(&spec);
        assert!(audit.is_valid);
        assert_eq!(audit.total_frames, 75);
        assert!(audit.has_camera_motion);
        assert!(audit.has_expressive_reason);

        // 过短时长警告
        let short_spec = VideoPromptSpec {
            duration_seconds: 0.1,
            ..spec.clone()
        };
        let audit2 = audit_video_prompt(&short_spec);
        assert!(audit2.warnings.iter().any(|w| w.contains("too short")));
    }

    #[test]
    fn test_mock_video_generation_with_ffmpeg() {
        let dir = tempdir().unwrap();
        let img_path = dir.path().join("first_frame.png");
        let out_video = dir.path().join("shot_test.mp4");

        // 创建一张简单的 320x180 测试首帧图片
        let img = image::RgbImage::new(320, 180);
        img.save(&img_path).unwrap();

        let spec = VideoPromptSpec {
            shot_id: "shot_test".to_string(),
            first_frame_path: Some(img_path),
            motion_prompt: "Smooth push-in toward center".to_string(),
            camera_motion: CameraMotion::PushIn,
            relationship: VideoGenRelationship::ImageToVideo,
            duration_seconds: 0.5,
            fps: 30.0,
            width: 320,
            height: 180,
        };

        let req = VideoGenerationRequest {
            spec,
            output_path: out_video.clone(),
            seed: Some(123),
        };

        let gen = MockVideoGenerator::new();
        let res = gen
            .generate(&req)
            .expect("FFmpeg mock video generation should succeed");

        assert_eq!(res.width, 320);
        assert_eq!(res.height, 180);
        assert_eq!(res.frames_count, 15);
        assert!(out_video.exists(), "MP4 video file must exist");

        let meta_file = dir.path().join("shot_test.json");
        assert!(meta_file.exists(), "Companion JSON metadata must exist");
    }
    #[test]
    fn test_validate_video_inputs() {
        let first = PathBuf::from("first.png");
        let last = PathBuf::from("last.png");
        let ref1 = PathBuf::from("ref1.png");

        // ImageToVideo: requires first_frame only
        let spec_i2v = VideoPromptSpec {
            relationship: VideoGenRelationship::ImageToVideo,
            first_frame_path: Some(first.clone()),
            ..Default::default()
        };
        assert!(validate_inputs(&spec_i2v, &VideoInputs::default()).is_ok());
        assert!(validate_inputs(
            &spec_i2v,
            &VideoInputs {
                last_frame: Some(last.clone()),
                reference_images: vec![],
                ..Default::default()
            }
        )
        .is_err());

        // FirstLastFrame: requires first and last frame
        let spec_flf = VideoPromptSpec {
            relationship: VideoGenRelationship::FirstLastFrame,
            first_frame_path: Some(first.clone()),
            ..Default::default()
        };
        assert!(validate_inputs(&spec_flf, &VideoInputs::default()).is_err());
        assert!(validate_inputs(
            &spec_flf,
            &VideoInputs {
                last_frame: Some(last.clone()),
                reference_images: vec![],
                ..Default::default()
            }
        )
        .is_ok());

        // ReferenceImages: requires reference_images, no first_frame
        let spec_refs = VideoPromptSpec {
            relationship: VideoGenRelationship::ReferenceImages,
            first_frame_path: None,
            ..Default::default()
        };
        assert!(validate_inputs(
            &spec_refs,
            &VideoInputs {
                last_frame: None,
                reference_images: vec![ref1.clone()],
                ..Default::default()
            }
        )
        .is_ok());
        // Max 7 reference images
        let too_many_refs: Vec<PathBuf> = (0..8)
            .map(|i| PathBuf::from(format!("ref_{}.png", i)))
            .collect();
        assert!(validate_inputs(
            &spec_refs,
            &VideoInputs {
                last_frame: None,
                reference_images: too_many_refs,
                ..Default::default()
            }
        )
        .is_err());

        // TextToVideo: requires no frames
        let spec_t2v = VideoPromptSpec {
            relationship: VideoGenRelationship::TextToVideo,
            first_frame_path: None,
            ..Default::default()
        };
        assert!(validate_inputs(&spec_t2v, &VideoInputs::default()).is_ok());
        assert!(validate_inputs(
            &spec_t2v,
            &VideoInputs {
                last_frame: None,
                reference_images: vec![ref1],
                ..Default::default()
            }
        )
        .is_err());

        // ReferenceTransfer: currently unsupported
        let spec_trans = VideoPromptSpec {
            relationship: VideoGenRelationship::ReferenceTransfer,
            ..Default::default()
        };
        assert!(validate_inputs(&spec_trans, &VideoInputs::default()).is_err());
    }
}

fn base64_encode(data: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((data.len() + 2) / 3 * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as usize;
        let b1 = if chunk.len() > 1 {
            chunk[1] as usize
        } else {
            0
        };
        let b2 = if chunk.len() > 2 {
            chunk[2] as usize
        } else {
            0
        };
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[(triple >> 18) & 0x3F] as char);
        out.push(TABLE[(triple >> 12) & 0x3F] as char);
        if chunk.len() > 1 {
            out.push(TABLE[(triple >> 6) & 0x3F] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(TABLE[triple & 0x3F] as char);
        } else {
            out.push('=');
        }
    }
    out
}
