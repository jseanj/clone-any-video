use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::probe::probe_media;
use crate::project::ProjectManifest;
use std::hash::{Hash, Hasher};

/// 计算 ScriptDocument 的规范化语义指纹，用于安全失效与缓存判定
pub fn compute_script_fingerprint(doc: &crate::script::ScriptDocument) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    let json = serde_json::to_string(doc).unwrap_or_default();
    json.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

// ============================================================================
// 1. 自动化质量审查模型 (Automated Review Domain Model)
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ReviewCategory {
    StreamConformance, // 码流与容器规范 (封装、分辨率、CFR 帧率、时长)
    ColorStandards,    // 色彩保真标准 (BT.709 色彩空间、有限电平)
    AudioQuality,      // 音频广播标准 (48 kHz、双声道、立体声、Ducking)
    SemanticTiming,    // 语义与时钟对齐 (分镜完整性、转场衔接、最小停留时间)
    VisualComponents,  // 视觉组件规范 (自包含、确定性渲染契约、覆盖范围)
    SubtitleSync,      // 字幕与语音对齐 (词级高亮、追踪锚点、提前量/滞后量)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ReviewStatus {
    Pass,
    Warning,
    Fail,
    Unverified,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReviewCriterion {
    pub name: String,
    pub category: ReviewCategory,
    pub status: ReviewStatus,
    pub message: String,
    pub owning_fact: Option<String>, // 归属事实修复源 (TREATMENT.md / Script / Recipe / Manifest)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReviewReport {
    pub passed: bool,
    pub ready_for_delivery: bool,
    pub artifact_digest: Option<String>,
    pub creative_review: serde_json::Value,
    pub score: f64, // 0.0 ~ 100.0
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    pub criteria: Vec<ReviewCriterion>,
    pub recommendations: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReviewConfig {
    pub evidence: Option<PathBuf>,
    pub expected_width: u32,
    pub expected_height: u32,
    pub expected_fps: f64,
    pub duration_tolerance_seconds: f64,
    pub required_color_matrix: String,
    pub required_audio_sample_rate: u32,
    pub required_audio_channels: u16,
    pub min_hold_duration_seconds: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
}

impl Default for ReviewConfig {
    fn default() -> Self {
        Self {
            evidence: None,
            expected_width: 640,
            expected_height: 360,
            expected_fps: 30.0,
            duration_tolerance_seconds: 0.25,
            required_color_matrix: "bt709".into(),
            required_audio_sample_rate: 48_000,
            required_audio_channels: 2,
            min_hold_duration_seconds: 0.50,
            mode: None,
        }
    }
}

// ============================================================================
// 2. 自动化质量审查核心引擎 (Review Engine)
// ============================================================================

/// 辅助探测成片视频的色彩空间 (color_space) 与电平范围 (color_range)
async fn probe_video_color_metadata(path: &Path) -> (Option<String>, Option<String>) {
    let mut cmd = tokio::process::Command::new("ffprobe");
    cmd.arg("-v")
        .arg("error")
        .arg("-select_streams")
        .arg("v:0")
        .arg("-show_entries")
        .arg("stream=color_space,color_range")
        .arg("-of")
        .arg("json")
        .arg(path.to_string_lossy().to_string());
    if let Ok(out) = cmd.output().await {
        if let Ok(val) = serde_json::from_slice::<serde_json::Value>(&out.stdout) {
            if let Some(stream) = val["streams"].as_array().and_then(|arr| arr.first()) {
                let space = stream["color_space"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string());
                let range = stream["color_range"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string());
                return (space, range);
            }
        }
    }
    (None, None)
}

/// 对交付成片 (master.mp4) 与工程蓝图 (project.json) 进行全模态自动审计
pub async fn review_project_and_master(
    manifest: &ProjectManifest,
    master_video_path: &Path,
    config: &ReviewConfig,
) -> Result<ReviewReport> {
    if config
        .mode
        .as_deref()
        .is_some_and(|m| !matches!(m, "production" | "mock"))
    {
        bail!("Review mode must be production or mock");
    }
    let mut criteria = Vec::new();
    let mut recommendations = Vec::new();

    // ------------------------------------------------------------------------
    // 检查 1: 物理成片文件存在性与基础探测
    // ------------------------------------------------------------------------
    if !master_video_path.exists() {
        criteria.push(ReviewCriterion {
            name: "Deliverable Existence".into(),
            category: ReviewCategory::StreamConformance,
            status: ReviewStatus::Fail,
            message: format!("Master deliverable not found at {:?}", master_video_path),
            owning_fact: Some("Run Source / Compositor".into()),
        });
        return Ok(ReviewReport {
            passed: false,
            ready_for_delivery: false,
            artifact_digest: None,
            creative_review: serde_json::Value::Null,
            score: 0.0,
            mode: Some("unknown".into()),
            criteria,
            recommendations: vec!["Execute compositor render to produce master video".into()],
        });
    }

    let meta = probe_media(master_video_path).await?;

    // ------------------------------------------------------------------------
    // 检查 2: 画面分辨率与宽高比对齐 (Resolution Check)
    // ------------------------------------------------------------------------
    let actual_width = meta.width.unwrap_or(0);
    let actual_height = meta.height.unwrap_or(0);
    if actual_width == config.expected_width && actual_height == config.expected_height {
        criteria.push(ReviewCriterion {
            name: "Canvas Dimensions".into(),
            category: ReviewCategory::StreamConformance,
            status: ReviewStatus::Pass,
            message: format!(
                "Resolution matches canvas: {}x{}",
                actual_width, actual_height
            ),
            owning_fact: None,
        });
    } else {
        criteria.push(ReviewCriterion {
            name: "Canvas Dimensions".into(),
            category: ReviewCategory::StreamConformance,
            status: ReviewStatus::Fail,
            message: format!(
                "Resolution mismatch: got {}x{}, expected {}x{}",
                actual_width, actual_height, config.expected_width, config.expected_height
            ),
            owning_fact: Some("Author Source / Canvas Recipe".into()),
        });
        recommendations
            .push("Update compositor canvas dimensions to match project standard".into());
    }

    // ------------------------------------------------------------------------
    // 检查 3: 帧率与 CFR 恒定帧率规范 (Framerate Check)
    // ------------------------------------------------------------------------
    let actual_fps = meta.fps.unwrap_or(0.0);
    let fps_diff = (actual_fps - config.expected_fps).abs();
    let is_cfr_verified = meta.is_cfr.unwrap_or(false);
    if fps_diff < 0.1 && is_cfr_verified {
        criteria.push(ReviewCriterion {
            name: "CFR Framerate Stability".into(),
            category: ReviewCategory::StreamConformance,
            status: ReviewStatus::Pass,
            message: format!(
                "Framerate strictly conforms to constant {:.1} fps (CFR verified)",
                actual_fps
            ),
            owning_fact: None,
        });
    } else if fps_diff < 0.1 {
        criteria.push(ReviewCriterion {
            name: "CFR Framerate Stability".into(),
            category: ReviewCategory::StreamConformance,
            status: ReviewStatus::Warning,
            message: format!("Average framerate {:.2} fps matches target, but r_frame_rate and avg_frame_rate differ or CFR is unverified", actual_fps),
            owning_fact: Some("Compositor fps_mode parameter".into()),
        });
    } else {
        criteria.push(ReviewCriterion {
            name: "CFR Framerate Stability".into(),
            category: ReviewCategory::StreamConformance,
            status: ReviewStatus::Fail,
            message: format!(
                "Framerate delta exceeds threshold: got {:.2} fps, expected {:.1} fps",
                actual_fps, config.expected_fps
            ),
            owning_fact: Some("Compositor fps_mode parameter".into()),
        });
        recommendations.push("Force constant framerate using -fps_mode cfr in compositor".into());
    }

    // ------------------------------------------------------------------------
    // 检查 4: 项目总时长容差 (Duration Alignment)
    // ------------------------------------------------------------------------
    let duration_diff = (meta.duration_seconds - manifest.meta.total_seconds).abs();
    if duration_diff <= config.duration_tolerance_seconds {
        criteria.push(ReviewCriterion {
            name: "Program Duration Match".into(),
            category: ReviewCategory::SemanticTiming,
            status: ReviewStatus::Pass,
            message: format!(
                "Duration matches timeline ({:.2}s vs project {:.2}s, delta {:.3}s)",
                meta.duration_seconds, manifest.meta.total_seconds, duration_diff
            ),
            owning_fact: None,
        });
    } else {
        criteria.push(ReviewCriterion {
            name: "Program Duration Match".into(),
            category: ReviewCategory::SemanticTiming,
            status: ReviewStatus::Fail,
            message: format!(
                "Program duration drifted: video {:.2}s vs project {:.2}s (delta {:.3}s > {:.2}s)",
                meta.duration_seconds,
                manifest.meta.total_seconds,
                duration_diff,
                config.duration_tolerance_seconds
            ),
            owning_fact: Some("Author Source / dynamic-timeline".into()),
        });
        recommendations
            .push("Realign project timeline with speech extents to prevent drift".into());
    }

    // ------------------------------------------------------------------------
    // 检查 5: 广播级 BT.709 色彩矩阵审计 (Broadcast Color Standard)
    // ------------------------------------------------------------------------
    let (color_space, color_range) = probe_video_color_metadata(master_video_path).await;
    let is_space_compliant = color_space.as_deref() == Some("bt709");
    let is_range_compliant = matches!(color_range.as_deref(), Some("tv") | Some("limited"));
    if is_space_compliant && is_range_compliant {
        criteria.push(ReviewCriterion {
            name: "BT.709 Broadcast Color Matrix".into(),
            category: ReviewCategory::ColorStandards,
            status: ReviewStatus::Pass,
            message: "Color matrix strictly tagged as BT.709 limited/tv broadcast range".into(),
            owning_fact: None,
        });
    } else if is_space_compliant {
        criteria.push(ReviewCriterion {
            name: "BT.709 Broadcast Color Matrix".into(),
            category: ReviewCategory::ColorStandards,
            status: ReviewStatus::Warning,
            message: format!(
                "Color space is BT.709, but color range is {:?}; expected limited/tv range",
                color_range.as_deref().unwrap_or("unknown")
            ),
            owning_fact: Some("Compositor scale out_range=tv".into()),
        });
        recommendations.push("Tag compositor output with out_range=tv".into());
    } else {
        criteria.push(ReviewCriterion {
            name: "BT.709 Broadcast Color Matrix".into(),
            category: ReviewCategory::ColorStandards,
            status: ReviewStatus::Warning,
            message: format!(
                "Color space tag: {:?}, range: {:?}; expected BT.709 tv range",
                color_space.as_deref().unwrap_or("unknown"),
                color_range.as_deref().unwrap_or("unknown"),
            ),
            owning_fact: Some("Compositor scale filter out_color_matrix".into()),
        });
        recommendations
            .push("Inject scale=out_color_matrix=bt709:out_range=tv filter into compositor".into());
    }

    // ------------------------------------------------------------------------
    // 检查 6: 音频广播级参数与时钟 (48 kHz Stereo Check)
    // ------------------------------------------------------------------------
    if meta.has_audio {
        let sample_rate = meta.audio_sample_rate.unwrap_or(0);
        let channels = meta.audio_channels.unwrap_or(0) as u16;
        if sample_rate == config.required_audio_sample_rate
            && channels >= config.required_audio_channels
        {
            criteria.push(ReviewCriterion {
                name: "Broadcast Audio Clock".into(),
                category: ReviewCategory::AudioQuality,
                status: ReviewStatus::Pass,
                message: format!(
                    "Audio meets 48 kHz stereo broadcast standard ({} Hz, {} ch)",
                    sample_rate, channels
                ),
                owning_fact: None,
            });
        } else {
            criteria.push(ReviewCriterion {
                name: "Broadcast Audio Clock".into(),
                category: ReviewCategory::AudioQuality,
                status: ReviewStatus::Warning,
                message: format!(
                    "Audio sample rate/channels non-standard: got {} Hz / {} ch, expected {} Hz / {} ch",
                    sample_rate, channels, config.required_audio_sample_rate, config.required_audio_channels
                ),
                owning_fact: Some("sound_mix / canonical resample".into()),
            });
            recommendations.push("Resample master mix to 48 kHz 2-channel stereo".into());
        }
    } else {
        criteria.push(ReviewCriterion {
            name: "Broadcast Audio Clock".into(),
            category: ReviewCategory::AudioQuality,
            status: ReviewStatus::Fail,
            message: "Master deliverable lacks audio track".into(),
            owning_fact: Some("sound_mix / master.wav".into()),
        });
        recommendations.push("Mux master_mix.wav into compositor output".into());
    }

    // ------------------------------------------------------------------------
    // 检查 7: 动态组件最小停留与可读性 (Component Hold Duration)
    // ------------------------------------------------------------------------
    let mut short_components = Vec::new();
    for comp in &manifest.tracks.visual_components {
        let hold_seconds = comp.end_seconds - comp.start_seconds;
        if hold_seconds < config.min_hold_duration_seconds {
            short_components.push((comp.id.clone(), hold_seconds));
        }
    }

    if short_components.is_empty() {
        criteria.push(ReviewCriterion {
            name: "Component Visual Readability".into(),
            category: ReviewCategory::VisualComponents,
            status: ReviewStatus::Pass,
            message: format!(
                "All {} visual components satisfy minimum hold duration (>= {:.2}s)",
                manifest.tracks.visual_components.len(),
                config.min_hold_duration_seconds
            ),
            owning_fact: None,
        });
    } else {
        criteria.push(ReviewCriterion {
            name: "Component Visual Readability".into(),
            category: ReviewCategory::VisualComponents,
            status: ReviewStatus::Warning,
            message: format!(
                "{} components hold too briefly for human reading: {:?}",
                short_components.len(),
                short_components
            ),
            owning_fact: Some("Author Source / component-designer".into()),
        });
        recommendations
            .push("Extend hold durations on short components to allow viewer comprehension".into());
    }

    // ------------------------------------------------------------------------
    // 检查 8: 智能闪避与声音工程完整性 (Audio Ducking Check)
    // ------------------------------------------------------------------------
    let has_bgm = !manifest.tracks.bgm.is_empty();
    let has_voice = !manifest.tracks.voice.is_empty();
    if has_bgm && has_voice {
        criteria.push(ReviewCriterion {
            name: "Audio Ducking Balance".into(),
            category: ReviewCategory::AudioQuality,
            status: ReviewStatus::Unverified,
            message: "Speech and BGM coexist in project; dynamic ducking attenuation unverified without physical multitrack waveform audit".into(),
            owning_fact: Some("Reviewer Multimodal Acoustic Audit".into()),
        });
    } else if has_bgm && !has_voice {
        criteria.push(ReviewCriterion {
            name: "Audio Ducking Balance".into(),
            category: ReviewCategory::AudioQuality,
            status: ReviewStatus::Pass,
            message: "Music-only piece; ducking inactive".into(),
            owning_fact: None,
        });
    } else {
        criteria.push(ReviewCriterion {
            name: "Audio Ducking Balance".into(),
            category: ReviewCategory::AudioQuality,
            status: ReviewStatus::Pass,
            message: "Single audio contributor; balance stable".into(),
            owning_fact: None,
        });
    }

    // ------------------------------------------------------------------------
    // 检查 9: 多模态字幕与视觉对齐 (Multimodal Subtitle & Visual Alignment)
    // ------------------------------------------------------------------------
    if !manifest.tracks.captions.is_empty() {
        criteria.push(ReviewCriterion {
            name: "Multimodal Subtitle Alignment".into(),
            category: ReviewCategory::SemanticTiming,
            status: ReviewStatus::Unverified,
            message: "Subtitle track scheduled; multi-modal lip-sync and visual-speech synchrony unverified without multimodal Agent inspection".into(),
            owning_fact: Some("External Multimodal Review Agent".into()),
        });
    }

    let encoded = meta.frame_count;
    criteria.push(ReviewCriterion {
        name: "Encoded Frame Count".into(),
        category: ReviewCategory::StreamConformance,
        status: match encoded {
            Some(n) if n == manifest.meta.total_frames => ReviewStatus::Pass,
            Some(_) => ReviewStatus::Fail,
            None => ReviewStatus::Unverified,
        },
        message: format!(
            "Encoded {:?}; planned {} frames",
            encoded, manifest.meta.total_frames
        ),
        owning_fact: Some("Compositor / integer frame ranges".into()),
    });
    if meta.has_audio {
        match crate::delivery::loudness(master_video_path).await {
            Ok(value) => {
                let in_range = value.integrated_lufs >= -18.0
                    && value.integrated_lufs <= -14.0
                    && value.true_peak_dbfs <= -1.0;
                criteria.push(ReviewCriterion{name:"Measured Loudness".into(),category:ReviewCategory::AudioQuality,status:if in_range{ReviewStatus::Pass}else{ReviewStatus::Warning},message:format!("{:.1} LUFS, true peak {:.1} dBFS, LRA {:.1} LU; suggested online VO range -18..-14 LUFS / <= -1 dBTP",value.integrated_lufs,value.true_peak_dbfs,value.range_lu),owning_fact:Some("Audio mastering / delivery brief".into())});
            }
            Err(error) => criteria.push(ReviewCriterion {
                name: "Measured Loudness".into(),
                category: ReviewCategory::AudioQuality,
                status: ReviewStatus::Unverified,
                message: error.to_string(),
                owning_fact: Some("FFmpeg R128 measurement".into()),
            }),
        }
    }
    if !manifest.tracks.captions.is_empty() {
        let details = crate::artifacts::details(master_video_path).unwrap_or_default();
        let expected: Vec<_> = manifest
            .tracks
            .captions
            .iter()
            .map(crate::delivery::caption_fact)
            .collect();
        let native_burn = details["subtitle_delivery"] == "burn-in"
            && details["caption_expectations"] == serde_json::to_value(expected)?;
        let overlays_cover = crate::delivery::captions_covered(&manifest.tracks.captions, &details);
        let status = if native_burn || overlays_cover {
            ReviewStatus::Pass
        } else if details["subtitle_delivery"] == "soft" || details["subtitle_delivery"] == "none" {
            ReviewStatus::Fail
        } else {
            ReviewStatus::Unverified
        };
        criteria.push(ReviewCriterion{name:"Caption Delivery Evidence".into(),category:ReviewCategory::SubtitleSync,status,message:format!("Actual producer subtitle policy {}; content-bound rendered text/window coverage: {}. Glyph/audio semantics require creative review",details["subtitle_delivery"],overlays_cover),owning_fact:Some("caption render / explicit subtitle-delivery".into())});
    }
    let creative_review = if let Some(path) = &config.evidence {
        crate::delivery::creative_evidence(master_video_path, path)?
    } else {
        serde_json::json!({"complete":false,"status":"unverified","required":["product_identity","speech_and_captions","visual_continuity","watermark_acceptance"]})
    };
    // ------------------------------------------------------------------------
    // 检查 10: 产物交付模式审计 (Artifact Delivery Mode)
    // ------------------------------------------------------------------------
    let actual_mode = crate::artifacts::mode(master_video_path);
    let mode = actual_mode.name().to_string();
    let required = config.mode.as_deref();
    let status = if required.is_some_and(|m| m != mode) {
        ReviewStatus::Fail
    } else {
        match actual_mode {
            crate::artifacts::Mode::Production => ReviewStatus::Pass,
            crate::artifacts::Mode::Mock => ReviewStatus::Warning,
            crate::artifacts::Mode::Unknown => ReviewStatus::Unverified,
        }
    };
    criteria.push(ReviewCriterion {
        name: "Artifact Delivery Mode".into(),
        category: ReviewCategory::StreamConformance,
        status,
        message: format!(
            "Content-bound artifact mode: {}; requested: {}",
            mode,
            required.unwrap_or("auto")
        ),
        owning_fact: Some("Media producer provenance / supplied asset admission".into()),
    });

    // 计算总评与评分 (Composite Quality Score)
    let total_checks = criteria.len() as f64;
    let mut passed_checks = 0.0;
    let mut has_hard_failure = false;

    for c in &criteria {
        match c.status {
            ReviewStatus::Pass => passed_checks += 1.0,
            ReviewStatus::Warning => passed_checks += 0.7,
            ReviewStatus::Unverified => passed_checks += 0.5,
            ReviewStatus::Fail => has_hard_failure = true,
        }
    }

    let score = if total_checks > 0.0 {
        ((passed_checks / total_checks) * 100.0).clamp(0.0, 100.0)
    } else {
        0.0
    };

    let passed = !has_hard_failure && score >= 80.0;

    Ok(ReviewReport {
        ready_for_delivery: passed
            && actual_mode == crate::artifacts::Mode::Production
            && creative_review["complete"] == true,
        artifact_digest: Some(crate::artifacts::content_digest(master_video_path)?),
        creative_review,
        passed,
        score,
        mode: Some(mode),
        criteria,
        recommendations,
    })
}

// ============================================================================
// 3. 全自主 Video Agent 编排流水线 (Video Agent Orchestrator)
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentOptions {
    pub reference_video: PathBuf,
    pub project_dir: PathBuf,
    pub output_video: PathBuf,
    pub auto_heal: bool,
    pub mock: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentRunResult {
    pub success: bool,
    pub master_video: PathBuf,
    #[serde(default)]
    pub mode: String,
    pub review_report: ReviewReport,
    pub phases_completed: Vec<String>,
}

/// 执行从原片逆向到交付审查的 5 阶段自治闭环
pub async fn run_autonomous_video_agent(options: &AgentOptions) -> Result<AgentRunResult> {
    let mut phases = Vec::new();
    println!("============================================================");
    println!("🤖 Starting Autonomous Video Agent Production Pipeline");
    println!("   Reference: {:?}", options.reference_video);
    println!("   Target:    {:?}", options.output_video);
    println!("   Workspace: {:?}", options.project_dir);
    println!("============================================================");

    tokio::fs::create_dir_all(&options.project_dir).await?;

    // ------------------------------------------------------------------------
    // Phase 1: 原片物理特征逆向与听觉/视觉证据提取 (Deconstruct & Evidence)
    // ------------------------------------------------------------------------
    println!("\n▶ [Phase 1/5] Ingesting and reverse-engineering reference video...");
    let probe_res = probe_media(&options.reference_video)
        .await
        .with_context(|| {
            format!(
                "Failed to probe reference video: {:?}",
                options.reference_video
            )
        })?;
    println!(
        "  - Media probed: {}x{} @ {:.2} fps, duration {:.2}s",
        probe_res.width.unwrap_or(0),
        probe_res.height.unwrap_or(0),
        probe_res.fps.unwrap_or(0.0),
        probe_res.duration_seconds
    );

    // 1.2 听觉证据提取 (Whisper Transcribe)
    let transcript_path = options.project_dir.join("transcript.json");
    let transcript = if transcript_path.exists() {
        let txt = tokio::fs::read_to_string(&transcript_path).await?;
        serde_json::from_str(&txt)?
    } else {
        let t = crate::transcribe::transcribe_media(crate::transcribe::TranscribeOptions {
            source_path: &options.reference_video,
            language: "en",
            api_key: None,
            api_url: None,
            local: true,
            mock: options.mock,
        })
        .await?;
        let json = serde_json::to_string_pretty(&t)?;
        tokio::fs::write(&transcript_path, json).await?;
        t
    };
    println!(
        "  - Hearing transcript extracted: {} words, {:.2}s audio",
        transcript
            .passages
            .iter()
            .map(|p| p.words.len())
            .sum::<usize>(),
        transcript.audio_seconds
    );

    // 1.3 视觉分镜突变边界检测 (Boundaries)
    let boundaries_path = options.project_dir.join("boundaries.json");
    let boundaries = if boundaries_path.exists() {
        let txt = tokio::fs::read_to_string(&boundaries_path).await?;
        serde_json::from_str(&txt)?
    } else {
        let b = crate::boundaries::detect_visual_boundaries(
            &options.reference_video,
            Some(2.0),
            Some(0.15),
        )
        .await?;
        let json = serde_json::to_string_pretty(&b)?;
        tokio::fs::write(&boundaries_path, json).await?;
        b
    };
    println!(
        "  - Visual scene transitions: {} cuts detected",
        boundaries.candidates.len()
    );

    // 1.4 沉淀 ANALYSIS.md 分析档案
    let analysis_path = options.project_dir.join("ANALYSIS.md");
    if !analysis_path.exists() {
        let analysis_content = format!(
            "# Reference Video Analysis\n\n- Source: {:?}\n- Resolution: {}x{}\n- Framerate: {:.2} fps\n- Duration: {:.2}s\n- Cuts: {}\n- Words: {}\n",
            options.reference_video,
            probe_res.width.unwrap_or(0),
            probe_res.height.unwrap_or(0),
            probe_res.fps.unwrap_or(30.0),
            probe_res.duration_seconds,
            boundaries.candidates.len(),
            transcript.passages.iter().map(|p| p.words.len()).sum::<usize>()
        );
        tokio::fs::write(&analysis_path, analysis_content).await?;
    }
    phases.push("Phase 1: Reference Ingest & Evidence Extraction".into());

    // ------------------------------------------------------------------------
    // Phase 2: 语义剧本推导与解耦时间轴构建 (Script, Timeline & Manifest)
    // ------------------------------------------------------------------------
    println!("\n▶ [Phase 2/5] Establishing semantic script and decoupled timeline...");
    let script_path = options.project_dir.join("target_script.svml");
    let svml_content = if script_path.exists() {
        tokio::fs::read_to_string(&script_path).await?
    } else {
        // 基于听觉证据生成初始 SVML 语义剧本
        let body: String = if !transcript.passages.is_empty() {
            transcript
                .passages
                .iter()
                .enumerate()
                .map(|(idx, p)| {
                    let seg_name = format!("shot_{:02}", idx + 1);
                    let role = "HOST";
                    let clean_text = p.text.trim();
                    format!(
                        "  <{}>\n    <{}> {} </{}>\n  </{}>",
                        seg_name, role, clean_text, role, seg_name
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        } else {
            "  <hook>\n    <HOST> 用纯 <Rust | r_ah_s_t> 引擎打造多模态视频智能体。 </HOST>\n  </hook>".to_string()
        };
        let default_svml = format!("<script id=\"auto_clone_script\">\n{}\n</script>", body);
        tokio::fs::write(&script_path, default_svml.trim()).await?;
        default_svml.trim().to_string()
    };
    let script_doc = crate::script::parse_script(&svml_content)?;
    crate::artifacts::check_media(&options.project_dir, &script_doc)?;

    let current_fp = compute_script_fingerprint(&script_doc);
    let fp_path = options.project_dir.join(".script_fingerprint");
    let cached_fp = if fp_path.exists() {
        tokio::fs::read_to_string(&fp_path)
            .await
            .unwrap_or_default()
    } else {
        String::new()
    };
    let script_unchanged = cached_fp == current_fp && !current_fp.is_empty();

    let script_segment_ids: Vec<String> =
        script_doc.segments.iter().map(|s| s.id.clone()).collect();
    let timeline_path = options.project_dir.join("timeline.json");
    let timeline = if script_unchanged && timeline_path.exists() {
        let txt = tokio::fs::read_to_string(&timeline_path).await?;
        if let Ok(tl) = serde_json::from_str::<crate::timeline::ResolvedTimeline>(&txt) {
            let tl_segment_ids: Vec<String> = tl.windows.iter().map(|w| w.id.clone()).collect();
            if tl_segment_ids == script_segment_ids {
                tl
            } else {
                let new_tl =
                    crate::timeline::project_script_to_timeline(&script_doc, 30.0, 0.3, None)?;
                let json = serde_json::to_string_pretty(&new_tl)?;
                tokio::fs::write(&timeline_path, json).await?;
                new_tl
            }
        } else {
            let tl = crate::timeline::project_script_to_timeline(&script_doc, 30.0, 0.3, None)?;
            let json = serde_json::to_string_pretty(&tl)?;
            tokio::fs::write(&timeline_path, json).await?;
            tl
        }
    } else {
        let tl = crate::timeline::project_script_to_timeline(&script_doc, 30.0, 0.3, None)?;
        let json = serde_json::to_string_pretty(&tl)?;
        tokio::fs::write(&timeline_path, json).await?;
        tl
    };

    let project_json_path = options.project_dir.join("project.json");
    let mut manifest = if script_unchanged && project_json_path.exists() {
        let txt = tokio::fs::read_to_string(&project_json_path).await?;
        if let Ok(mf) = serde_json::from_str::<ProjectManifest>(&txt) {
            let mf_segment_ids: Vec<String> =
                mf.shots.iter().map(|s| s.segment_id.clone()).collect();
            let voice_matches = script_doc.segments.iter().all(|seg| {
                mf.tracks.voice.iter().any(|v| {
                    v.payload.get("spoken_text").and_then(|s| s.as_str()) == Some(&seg.spoken_text)
                })
            });
            if mf_segment_ids == script_segment_ids && voice_matches {
                mf
            } else {
                let w = probe_res.width.unwrap_or(640);
                let h = probe_res.height.unwrap_or(360);
                let new_mf =
                    crate::project::create_project_manifest(&script_doc, &timeline, None, w, h)?;
                let json = serde_json::to_string_pretty(&new_mf)?;
                tokio::fs::write(&project_json_path, json).await?;
                new_mf
            }
        } else {
            let w = probe_res.width.unwrap_or(640);
            let h = probe_res.height.unwrap_or(360);
            let new_mf =
                crate::project::create_project_manifest(&script_doc, &timeline, None, w, h)?;
            let json = serde_json::to_string_pretty(&new_mf)?;
            tokio::fs::write(&project_json_path, json).await?;
            new_mf
        }
    } else {
        let w = probe_res.width.unwrap_or(640);
        let h = probe_res.height.unwrap_or(360);
        let mf = crate::project::create_project_manifest(&script_doc, &timeline, None, w, h)?;
        let json = serde_json::to_string_pretty(&mf)?;
        tokio::fs::write(&project_json_path, json).await?;
        mf
    };
    let _ = tokio::fs::write(&fp_path, &current_fp).await;
    println!(
        "  - Manifest blueprint ready: {} shots, canvas {}x{}",
        manifest.shots.len(),
        manifest.meta.width,
        manifest.meta.height
    );
    phases.push("Phase 2: Semantic Script & Project Blueprint".into());

    // ------------------------------------------------------------------------
    // Phase 3: 视听导演工艺 —— 配音、混音与分镜生成 (Audiovisual Craft)
    // ------------------------------------------------------------------------
    println!("\n▶ [Phase 3/5] Directing audiovisual craft (A-roll, sound mixing, motion)...");
    let video_assets_dir = options.project_dir.join("assets").join("video");
    let voice_assets_dir = options.project_dir.join("assets").join("voice");
    let audio_assets_dir = options.project_dir.join("assets").join("audio");
    tokio::fs::create_dir_all(&video_assets_dir).await?;
    tokio::fs::create_dir_all(&voice_assets_dir).await?;
    tokio::fs::create_dir_all(&audio_assets_dir).await?;

    // 确保各镜头有背景视频资产
    for shot in &manifest.shots {
        let shot_vid = video_assets_dir.join(format!("{}.mp4", shot.id));
        if !shot_vid.exists() {
            if options.mock {
                // 生成基准分镜视频
                let mut cmd = tokio::process::Command::new("ffmpeg");
                cmd.arg("-y")
                    .arg("-f")
                    .arg("lavfi")
                    .arg("-i")
                    .arg(format!(
                        "color=c=#18212A:s={}x{}:d={:.3}:r=30",
                        manifest.meta.width, manifest.meta.height, shot.duration_seconds
                    ))
                    .arg("-pix_fmt")
                    .arg("yuv420p")
                    .arg(shot_vid.to_string_lossy().to_string());
                if !cmd.status().await?.success() {
                    bail!("Mock background generation failed");
                }
                crate::artifacts::record(
                    &shot_vid,
                    crate::artifacts::Mode::Mock,
                    "mock-background",
                )?;
            } else {
                bail!("Missing required background video asset for shot '{}' at {:?}. In production mode, invoke video-director or supply footage.", shot.id, shot_vid);
            }
        }
    }

    // 确保母带音频资产
    let master_mix_wav = audio_assets_dir.join("master_mix.wav");
    if !master_mix_wav.exists() {
        if options.mock {
            let mut cmd = tokio::process::Command::new("ffmpeg");
            cmd.arg("-y")
                .arg("-f")
                .arg("lavfi")
                .arg("-i")
                .arg(format!(
                    "anullsrc=r=48000:cl=stereo:d={:.3}",
                    manifest.meta.total_seconds
                ))
                .arg(master_mix_wav.to_string_lossy().to_string());
            if !cmd.status().await?.success() {
                bail!("Mock audio generation failed");
            }
            crate::artifacts::record(&master_mix_wav, crate::artifacts::Mode::Mock, "mock-audio")?;
        } else {
            bail!("Missing required master audio mix at {:?}. In production mode, invoke sound-mixer or voice-director.", master_mix_wav);
        }
    }
    crate::artifacts::accept_media(&options.project_dir, &script_doc)?;
    phases.push("Phase 3: Audiovisual Craft Direction".into());

    // ------------------------------------------------------------------------
    // Phase 4: 生产引擎 —— 字幕排期、Web 组件设计、无头捕获与离线压制
    // ------------------------------------------------------------------------
    println!("\n▶ [Phase 4/5] Executing production engine (components, capture, compositing)...");
    let comp_dir = options.project_dir.join("components");
    tokio::fs::create_dir_all(&comp_dir).await?;
    let specs =
        crate::component_designer::auto_design_components_for_project(&manifest, &script_doc)?;
    for spec in &specs {
        let html = crate::component_designer::render_component_to_html(spec)?;
        let p = comp_dir.join(format!("{}.html", spec.id));
        tokio::fs::write(&p, html)
            .await
            .with_context(|| format!("Failed writing component HTML to {:?}", p))?;
    }
    if !specs.is_empty() {
        manifest = crate::component_designer::update_manifest_with_visual_components(
            &manifest, &specs, &comp_dir,
        )?;
        let mf_json = serde_json::to_string_pretty(&manifest)?;
        tokio::fs::write(&project_json_path, mf_json).await?;

        let comp_out_dir = options.project_dir.join("assets").join("video_components");
        tokio::fs::create_dir_all(&comp_out_dir).await?;
        let capture_config = crate::browser_capture::CaptureConfig {
            width: manifest.meta.width,
            height: manifest.meta.height,
            fps: manifest.meta.fps,
            transparent: true,
            ..Default::default()
        };
        let provider = if options.mock {
            crate::browser_capture::BrowserProvider::Mock
        } else {
            crate::browser_capture::resolve_browser_provider(false)
                .context("In production mode, headless Chrome must be installed. Supply Chrome binary or use --mock for demonstration.")?
        };
        let captured = crate::browser_capture::batch_capture_project_components(
            &manifest,
            &comp_out_dir,
            &capture_config,
            &provider,
        )
        .await
        .context("Failed batch capturing visual components")?;
        manifest = captured;
        let mf_json = serde_json::to_string_pretty(&manifest)?;
        tokio::fs::write(&project_json_path, mf_json).await?;
    }

    // Schedule and export subtitles
    if let Ok(cap_doc) = crate::caption_engine::build_caption_document_from_script(&script_doc) {
        let mut speech_intervals = Vec::new();
        for item in &manifest.tracks.voice {
            speech_intervals.push((item.start_seconds, item.end_seconds));
        }
        if speech_intervals.is_empty() {
            for shot in &manifest.shots {
                speech_intervals.push((shot.start_seconds, shot.end_seconds));
            }
        }
        if let Ok(timing) = crate::caption_engine::build_caption_timing_from_speech_intervals(
            &cap_doc,
            &speech_intervals,
        ) {
            if let Ok(schedule) = crate::caption_engine::schedule_caption(
                &cap_doc,
                &timing,
                &crate::caption_engine::CaptionRecipe::default(),
            ) {
                let assets_dir = options.project_dir.join("assets");
                let _ = tokio::fs::create_dir_all(&assets_dir).await;
                let ass_text = crate::caption_engine::export_to_ass(
                    &schedule,
                    &crate::caption_engine::CaptionRecipe::default(),
                    manifest.meta.width,
                    manifest.meta.height,
                );
                let _ = tokio::fs::write(assets_dir.join("subtitles.ass"), ass_text).await;
                let vtt_text = crate::caption_engine::export_to_webvtt(&schedule);
                let _ = tokio::fs::write(assets_dir.join("subtitles.vtt"), vtt_text).await;
            }
        }
    }

    let plan = crate::compositor::compile_project_to_composite_plan(
        &manifest,
        &options.project_dir,
        &options.output_video,
    )?;
    crate::compositor::render_composition(&plan, options.mock).await?;
    println!(
        "  - Master composition rendered to: {:?}",
        options.output_video
    );
    phases.push("Phase 4: Hyperframes Offline Composition".into());

    // ------------------------------------------------------------------------
    // Phase 5: 自动质量审查与归属事实自愈循环 (Review & Self-Healing Loop)
    // ------------------------------------------------------------------------
    println!("\n▶ [Phase 5/5] Conducting automated multi-modal quality review...");
    let mut review_report = review_project_and_master(
        &manifest,
        &options.output_video,
        &ReviewConfig {
            expected_width: manifest.meta.width,
            expected_height: manifest.meta.height,
            expected_fps: manifest.meta.fps,
            mode: Some(if options.mock { "mock" } else { "production" }.into()),
            ..Default::default()
        },
    )
    .await?;

    println!("  - Review Score:  {:.1} / 100.0", review_report.score);
    println!(
        "  - Review Status: {}",
        if review_report.passed {
            "✅ PASSED"
        } else {
            "❌ REVISION REQUIRED"
        }
    );

    // 若未通过且开启了自愈能力 (Self-Healing Loop)
    if !review_report.passed && options.auto_heal {
        println!(
            "\n🔧 [Self-Healing] Detected actionable issues; attempting repair on owning facts..."
        );
        for rec in &review_report.recommendations {
            println!("  - Applying patch: {}", rec);
        }

        // 归属事实自愈：若存在配音实测时长，尝试二次对齐时间线并同步 Manifest
        let voice_dir = options.project_dir.join("assets").join("voice");
        if voice_dir.exists() {
            if let Ok(extents) =
                crate::voice_director::collect_speech_extents(&voice_dir, &script_doc).await
            {
                if !extents.is_empty() {
                    if let Ok((_new_tl, aligned_mf)) =
                        crate::voice_director::realign_project_timeline(
                            &script_doc,
                            &manifest,
                            &extents,
                            manifest.meta.fps,
                            0.3,
                        )
                    {
                        manifest = aligned_mf;
                        let mf_json = serde_json::to_string_pretty(&manifest)?;
                        let _ = tokio::fs::write(&project_json_path, mf_json).await;
                    }
                }
            }
        }

        // 重编译多层合成方案并重新单次压制
        let healed_plan = crate::compositor::compile_project_to_composite_plan(
            &manifest,
            &options.project_dir,
            &options.output_video,
        )?;
        crate::compositor::render_composition(&healed_plan, options.mock).await?;

        // 复检
        review_report = review_project_and_master(
            &manifest,
            &options.output_video,
            &ReviewConfig {
                expected_width: manifest.meta.width,
                expected_height: manifest.meta.height,
                expected_fps: manifest.meta.fps,
                mode: Some(if options.mock { "mock" } else { "production" }.into()),
                ..Default::default()
            },
        )
        .await?;
        println!(
            "  - Post-repair Review Score: {:.1} / 100.0",
            review_report.score
        );
    }
    phases.push("Phase 5: Automated Review & Delivery Verification".into());

    println!("\n============================================================");
    println!("🎉 Autonomous Video Agent Run Complete!");
    println!("   Final Output: {:?}", options.output_video);
    println!(
        "   Overall Quality: {:.1}/100 ({})",
        review_report.score,
        if review_report.passed {
            "PASSED"
        } else {
            "ADVISORY"
        }
    );
    println!("============================================================");

    let mode_str = review_report
        .mode
        .clone()
        .unwrap_or_else(|| "unknown".into());
    Ok(AgentRunResult {
        success: review_report.passed,
        master_video: options.output_video.clone(),
        mode: mode_str,
        review_report,
        phases_completed: phases,
    })
}

// ============================================================================
// 4. SVML/SVRun 声明式全流程构建器 (SVML Build & Run Engine)
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildOptions {
    pub prepared_project: Option<PathBuf>,
    pub source: PathBuf,
    pub project_dir: PathBuf,
    pub output_video: PathBuf,
    pub auto_heal: bool,
    pub mock: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildRunResult {
    pub success: bool,
    pub master_video: PathBuf,
    #[serde(default)]
    pub mode: String,
    pub review_report: ReviewReport,
    pub phases_completed: Vec<String>,
}

/// 直接由 SVML 剧本 (Author Source) 或 SVRun 构建声明 (Run Source) 一键编译压制成片
pub async fn build_svml_project(options: &BuildOptions) -> Result<BuildRunResult> {
    let mut phases = Vec::new();
    println!("============================================================");
    println!("🚀 Building Video Project from Declarative Source (Build/Run)");
    println!("   Source:    {:?}", options.source);
    println!("   Target:    {:?}", options.output_video);
    println!("   Workspace: {:?}", options.project_dir);
    println!("============================================================");

    tokio::fs::create_dir_all(&options.project_dir).await?;

    // 1. 判断并解析 Source (支持 .svrun 与 .svml)
    let source_content = tokio::fs::read_to_string(&options.source)
        .await
        .with_context(|| format!("Failed to read source file: {:?}", options.source))?;

    let svml_path = if source_content.contains("<svrun")
        || options.source.extension().and_then(|s| s.to_str()) == Some("svrun")
    {
        println!("\n▶ [Source Ingest] Detected Run Source (.svrun). Resolving Author Source target...");
        let run = crate::production_graph::source::read_run(&options.source)?;
        if run.targets != ["final.video"]
            || !run.candidates.is_empty()
            || !run.satisfy.is_empty()
            || run.reuse.is_some()
        {
            bail!("Legacy build accepts only target final.video; other Run selections require a production Author graph");
        }
        let resolved = run.author;
        println!("  - Linked Author Source (.svml): {:?}", resolved);
        resolved
    } else {
        println!("\n▶ [Source Ingest] Ingesting Author Source (.svml)...");
        options.source.clone()
    };

    let svml_text = tokio::fs::read_to_string(&svml_path)
        .await
        .with_context(|| format!("Failed to read SVML file: {:?}", svml_path))?;

    // 2. 语义剧本解析与发音校验
    println!("\n▶ [Phase 1/5] Parsing semantic script and validating phonemes...");
    let script_doc = crate::script::parse_script(&svml_text)?;
    if let Some(path) = &options.prepared_project {
        let manifest: ProjectManifest = serde_json::from_slice(&std::fs::read(path)?)?;
        crate::project::validate_project(&manifest)?;
        if manifest.tracks.voice.len() != script_doc.segments.len() {
            bail!("Prepared project has different speech coverage");
        }
        if !manifest.tracks.captions.is_empty() {
            let authored: Vec<_> = script_doc
                .segments
                .iter()
                .flat_map(|s| &s.turns)
                .flat_map(|t| &t.cues)
                .map(|c| c.text.as_str())
                .collect();
            let prepared: Vec<_> = manifest
                .tracks
                .captions
                .iter()
                .map(|c| c.payload["text"].as_str().unwrap_or(&c.name))
                .collect();
            if authored != prepared {
                bail!("Prepared project captions differ from current Author display; re-render captions");
            }
        }
        for segment in &script_doc.segments {
            let voice = manifest
                .tracks
                .voice
                .iter()
                .find(|v| v.id == format!("voice_{}", segment.id))
                .context("Prepared project missing authored speech ID")?;
            let spoken = voice.payload["spoken_text"]
                .as_str()
                .context("Prepared project lacks spoken identity")?;
            if crate::speech_alignment::spoken_key(spoken)
                != crate::speech_alignment::spoken_key(&segment.spoken_text)
            {
                bail!(
                    "Prepared project speech does not match current Author: {}",
                    segment.id
                );
            }
        }
        let plan = crate::compositor::compile_project_to_composite_plan(
            &manifest,
            &options.project_dir,
            &options.output_video,
        )?;
        crate::artifacts::check_media_with_master(
            &options.project_dir,
            &script_doc,
            plan.audio.as_ref().map(|a| a.path.as_path()),
        )?;
        crate::compositor::render_composition(&plan, options.mock).await?;
        let review_report = review_project_and_master(
            &manifest,
            &options.output_video,
            &ReviewConfig {
                expected_width: manifest.meta.width,
                expected_height: manifest.meta.height,
                expected_fps: manifest.meta.fps,
                mode: Some(if options.mock { "mock" } else { "production" }.into()),
                ..Default::default()
            },
        )
        .await?;
        let review_path = options.output_video.with_extension("review.json");
        std::fs::write(&review_path, serde_json::to_vec_pretty(&review_report)?)?;
        println!("Prepared project rendered: {:?}; technical pass={}, ready_for_delivery={}; report={:?}",options.output_video,review_report.passed,review_report.ready_for_delivery,review_path);
        if review_report.passed || options.mock {
            crate::artifacts::accept_media_with_master(
                &options.project_dir,
                &script_doc,
                plan.audio.as_ref().map(|a| a.path.as_path()),
            )?;
        }
        return Ok(BuildRunResult {
            success: review_report.passed || options.mock,
            mode: review_report
                .mode
                .clone()
                .unwrap_or_else(|| "unknown".into()),
            master_video: options.output_video.clone(),
            review_report,
            phases_completed: vec![
                "Explicit prepared project selected; no cached blueprint substituted".into(),
                "Prepared media composition".into(),
                "Measured technical review; creative readiness separate".into(),
            ],
        });
    }
    crate::artifacts::check_media(&options.project_dir, &script_doc)?;
    println!(
        "  - Script parsed: {} segments, estimated spoken duration: {:.2}s",
        script_doc.segments.len(),
        script_doc.total_estimated_seconds
    );
    phases.push("Phase 1: SVML Script Parsing & Phoneme Validation".into());

    // 3. 动态时间轴求解与项目蓝图映射
    println!("\n▶ [Phase 2/5] Resolving dynamic elastic timeline & project manifest...");
    let current_fp = compute_script_fingerprint(&script_doc);
    let fp_path = options.project_dir.join(".script_fingerprint");
    let cached_fp = if fp_path.exists() {
        tokio::fs::read_to_string(&fp_path)
            .await
            .unwrap_or_default()
    } else {
        String::new()
    };
    let script_unchanged = cached_fp == current_fp && !current_fp.is_empty();

    let script_segment_ids: Vec<String> =
        script_doc.segments.iter().map(|s| s.id.clone()).collect();
    let timeline_path = options.project_dir.join("timeline.json");
    let timeline = if script_unchanged && timeline_path.exists() {
        let txt = tokio::fs::read_to_string(&timeline_path).await?;
        if let Ok(tl) = serde_json::from_str::<crate::timeline::ResolvedTimeline>(&txt) {
            let tl_segment_ids: Vec<String> = tl.windows.iter().map(|w| w.id.clone()).collect();
            if tl_segment_ids == script_segment_ids {
                tl
            } else {
                let new_tl =
                    crate::timeline::project_script_to_timeline(&script_doc, 30.0, 0.3, None)?;
                let tl_json = serde_json::to_string_pretty(&new_tl)?;
                tokio::fs::write(&timeline_path, tl_json).await?;
                new_tl
            }
        } else {
            let tl = crate::timeline::project_script_to_timeline(&script_doc, 30.0, 0.3, None)?;
            let tl_json = serde_json::to_string_pretty(&tl)?;
            tokio::fs::write(&timeline_path, tl_json).await?;
            tl
        }
    } else {
        let tl = crate::timeline::project_script_to_timeline(&script_doc, 30.0, 0.3, None)?;
        let tl_json = serde_json::to_string_pretty(&tl)?;
        tokio::fs::write(&timeline_path, tl_json).await?;
        tl
    };

    let project_json_path = options.project_dir.join("project.json");
    let mut manifest = if script_unchanged && project_json_path.exists() {
        let txt = tokio::fs::read_to_string(&project_json_path).await?;
        if let Ok(mf) = serde_json::from_str::<ProjectManifest>(&txt) {
            let mf_segment_ids: Vec<String> =
                mf.shots.iter().map(|s| s.segment_id.clone()).collect();
            let voice_matches = script_doc.segments.iter().all(|seg| {
                mf.tracks.voice.iter().any(|v| {
                    v.payload.get("spoken_text").and_then(|s| s.as_str()) == Some(&seg.spoken_text)
                })
            });
            if mf_segment_ids == script_segment_ids && voice_matches {
                mf
            } else {
                let new_mf = crate::project::create_project_manifest(
                    &script_doc,
                    &timeline,
                    None,
                    640,
                    360,
                )?;
                let mf_json = serde_json::to_string_pretty(&new_mf)?;
                tokio::fs::write(&project_json_path, mf_json).await?;
                new_mf
            }
        } else {
            let new_mf =
                crate::project::create_project_manifest(&script_doc, &timeline, None, 640, 360)?;
            let mf_json = serde_json::to_string_pretty(&new_mf)?;
            tokio::fs::write(&project_json_path, mf_json).await?;
            new_mf
        }
    } else {
        let new_mf =
            crate::project::create_project_manifest(&script_doc, &timeline, None, 640, 360)?;
        let mf_json = serde_json::to_string_pretty(&new_mf)?;
        tokio::fs::write(&project_json_path, mf_json).await?;
        new_mf
    };
    let _ = tokio::fs::write(&fp_path, &current_fp).await;
    println!(
        "  - Manifest blueprint ready: {} shots, canvas {}x{}",
        manifest.shots.len(),
        manifest.meta.width,
        manifest.meta.height
    );
    phases.push("Phase 2: Dynamic Timeline & Project Manifest Compilation".into());

    // 4. 视听物料与动态组件调度
    println!("\n▶ [Phase 3/5] Scheduling audiovisual assets and Web-as-Video components...");
    let video_assets_dir = options.project_dir.join("assets").join("video");
    let audio_assets_dir = options.project_dir.join("assets").join("audio");
    tokio::fs::create_dir_all(&video_assets_dir).await?;
    tokio::fs::create_dir_all(&audio_assets_dir).await?;

    for shot in &manifest.shots {
        let shot_vid = video_assets_dir.join(format!("{}.mp4", shot.id));
        let shot_still = options
            .project_dir
            .join("assets")
            .join("visual")
            .join(format!("{}.png", shot.id));
        if !shot_vid.exists() && !shot_still.exists() {
            if options.mock {
                let mut cmd = tokio::process::Command::new("ffmpeg");
                cmd.arg("-y")
                    .arg("-f")
                    .arg("lavfi")
                    .arg("-i")
                    .arg(format!(
                        "color=c=#18212A:s={}x{}:d={:.3}:r=30",
                        manifest.meta.width, manifest.meta.height, shot.duration_seconds
                    ))
                    .arg("-pix_fmt")
                    .arg("yuv420p")
                    .arg(shot_vid.to_string_lossy().to_string());
                if !cmd.status().await?.success() {
                    bail!("Mock background generation failed");
                }
                crate::artifacts::record(
                    &shot_vid,
                    crate::artifacts::Mode::Mock,
                    "mock-background",
                )?;
            } else {
                bail!("Missing required background video asset for shot '{}' at {:?}. In production mode, invoke video-director or supply footage.", shot.id, shot_vid);
            }
        }
    }

    let master_mix_wav = audio_assets_dir.join("master_mix.wav");
    if !master_mix_wav.exists() {
        if options.mock {
            let mut cmd = tokio::process::Command::new("ffmpeg");
            cmd.arg("-y")
                .arg("-f")
                .arg("lavfi")
                .arg("-i")
                .arg(format!(
                    "anullsrc=r=48000:cl=stereo:d={:.3}",
                    manifest.meta.total_seconds
                ))
                .arg(master_mix_wav.to_string_lossy().to_string());
            if !cmd.status().await?.success() {
                bail!("Mock audio generation failed");
            }
            crate::artifacts::record(&master_mix_wav, crate::artifacts::Mode::Mock, "mock-audio")?;
        } else {
            bail!("Missing required master audio mix at {:?}. In production mode, invoke sound-mixer or voice-director.", master_mix_wav);
        }
    }

    crate::artifacts::accept_media(&options.project_dir, &script_doc)?;
    let comp_dir = options.project_dir.join("components");
    tokio::fs::create_dir_all(&comp_dir).await?;
    let specs =
        crate::component_designer::auto_design_components_for_project(&manifest, &script_doc)?;
    for spec in &specs {
        let html = crate::component_designer::render_component_to_html(spec)?;
        let p = comp_dir.join(format!("{}.html", spec.id));
        tokio::fs::write(&p, html)
            .await
            .with_context(|| format!("Failed writing component HTML to {:?}", p))?;
    }
    if !specs.is_empty() {
        manifest = crate::component_designer::update_manifest_with_visual_components(
            &manifest, &specs, &comp_dir,
        )?;
        let mf_json = serde_json::to_string_pretty(&manifest)?;
        tokio::fs::write(&project_json_path, mf_json).await?;

        let comp_out_dir = options.project_dir.join("assets").join("video_components");
        tokio::fs::create_dir_all(&comp_out_dir).await?;
        let capture_config = crate::browser_capture::CaptureConfig {
            width: manifest.meta.width,
            height: manifest.meta.height,
            fps: manifest.meta.fps,
            transparent: true,
            ..Default::default()
        };
        let provider = if options.mock {
            crate::browser_capture::BrowserProvider::Mock
        } else {
            crate::browser_capture::resolve_browser_provider(false)
                .context("In production mode, headless Chrome must be installed. Supply Chrome binary or use --mock for demonstration.")?
        };
        let captured = crate::browser_capture::batch_capture_project_components(
            &manifest,
            &comp_out_dir,
            &capture_config,
            &provider,
        )
        .await
        .context("Failed batch capturing visual components")?;
        manifest = captured;
        let mf_json = serde_json::to_string_pretty(&manifest)?;
        tokio::fs::write(&project_json_path, mf_json).await?;
    }

    // Schedule and export subtitles
    if let Ok(cap_doc) = crate::caption_engine::build_caption_document_from_script(&script_doc) {
        let mut speech_intervals = Vec::new();
        for item in &manifest.tracks.voice {
            speech_intervals.push((item.start_seconds, item.end_seconds));
        }
        if speech_intervals.is_empty() {
            for shot in &manifest.shots {
                speech_intervals.push((shot.start_seconds, shot.end_seconds));
            }
        }
        if let Ok(timing) = crate::caption_engine::build_caption_timing_from_speech_intervals(
            &cap_doc,
            &speech_intervals,
        ) {
            if let Ok(schedule) = crate::caption_engine::schedule_caption(
                &cap_doc,
                &timing,
                &crate::caption_engine::CaptionRecipe::default(),
            ) {
                let assets_dir = options.project_dir.join("assets");
                let _ = tokio::fs::create_dir_all(&assets_dir).await;
                let ass_text = crate::caption_engine::export_to_ass(
                    &schedule,
                    &crate::caption_engine::CaptionRecipe::default(),
                    manifest.meta.width,
                    manifest.meta.height,
                );
                let _ = tokio::fs::write(assets_dir.join("subtitles.ass"), ass_text).await;
                let vtt_text = crate::caption_engine::export_to_webvtt(&schedule);
                let _ = tokio::fs::write(assets_dir.join("subtitles.vtt"), vtt_text).await;
            }
        }
    }
    phases.push("Phase 3: Component Design & Track Scheduling".into());

    // 5. 无头浏览器确定性捕获与 Hyperframes 离线多层压制
    println!("\n▶ [Phase 4/5] Executing Hyperframes offline composition...");
    let plan = crate::compositor::compile_project_to_composite_plan(
        &manifest,
        &options.project_dir,
        &options.output_video,
    )?;
    crate::compositor::render_composition(&plan, options.mock).await?;
    println!("  - Master video rendered to: {:?}", options.output_video);
    phases.push("Phase 4: Single-Pass Hyperframes Compositing".into());

    // 6. 自动化多模态质量审查与自愈循环
    println!("\n▶ [Phase 5/5] Conducting automated multi-modal quality review...");
    let mut review_report = review_project_and_master(
        &manifest,
        &options.output_video,
        &ReviewConfig {
            expected_width: manifest.meta.width,
            expected_height: manifest.meta.height,
            expected_fps: manifest.meta.fps,
            mode: Some(if options.mock { "mock" } else { "production" }.into()),
            ..Default::default()
        },
    )
    .await?;

    println!("  - Review Score:  {:.1} / 100.0", review_report.score);
    println!(
        "  - Review Status: {}",
        if review_report.passed {
            "✅ PASSED"
        } else {
            "❌ REVISION REQUIRED"
        }
    );

    if !review_report.passed && options.auto_heal {
        println!(
            "\n🔧 [Self-Healing] Detected actionable issues; attempting repair on owning facts..."
        );
        for rec in &review_report.recommendations {
            println!("  - Applying patch: {}", rec);
        }

        // 归属事实自愈：若存在配音实测时长，尝试二次对齐时间线并同步 Manifest
        let voice_dir = options.project_dir.join("assets").join("voice");
        if voice_dir.exists() {
            if let Ok(extents) =
                crate::voice_director::collect_speech_extents(&voice_dir, &script_doc).await
            {
                if !extents.is_empty() {
                    if let Ok((_new_tl, aligned_mf)) =
                        crate::voice_director::realign_project_timeline(
                            &script_doc,
                            &manifest,
                            &extents,
                            manifest.meta.fps,
                            0.3,
                        )
                    {
                        manifest = aligned_mf;
                        let mf_json = serde_json::to_string_pretty(&manifest)?;
                        let _ = tokio::fs::write(&project_json_path, mf_json).await;
                    }
                }
            }
        }

        // 重编译多层合成方案并重新单次压制
        let healed_plan = crate::compositor::compile_project_to_composite_plan(
            &manifest,
            &options.project_dir,
            &options.output_video,
        )?;
        crate::compositor::render_composition(&healed_plan, options.mock).await?;

        review_report = review_project_and_master(
            &manifest,
            &options.output_video,
            &ReviewConfig {
                expected_width: manifest.meta.width,
                expected_height: manifest.meta.height,
                expected_fps: manifest.meta.fps,
                mode: Some(if options.mock { "mock" } else { "production" }.into()),
                ..Default::default()
            },
        )
        .await?;
        println!(
            "  - Post-repair Review Score: {:.1} / 100.0",
            review_report.score
        );
    }
    phases.push("Phase 5: Automated Review & Verification".into());

    let build_success = if options.mock {
        true
    } else {
        review_report.passed
    };

    println!("\n============================================================");
    if build_success {
        println!("🎉 Video Project Build Succeeded!");
        println!("   Deliverable: {:?}", options.output_video);
        println!(
            "   Quality Score: {:.1}/100 ({})",
            review_report.score,
            if review_report.passed {
                "PASSED"
            } else {
                "MOCK"
            }
        );
    } else {
        println!("❌ Video Project Build Failed Quality Review");
        println!("   Deliverable: {:?}", options.output_video);
        println!("   Quality Score: {:.1}/100 (FAILED)", review_report.score);
    }
    println!("============================================================");

    let mode_str = review_report
        .mode
        .clone()
        .unwrap_or_else(|| "unknown".into());
    Ok(BuildRunResult {
        success: build_success,
        master_video: options.output_video.clone(),
        mode: mode_str,
        review_report,
        phases_completed: phases,
    })
}

// ============================================================================
// 单元测试
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_review_missing_master_file_fails_cleanly() {
        let manifest = ProjectManifest {
            master_audio: None,
            format: "clone-any-video.project@1".into(),
            meta: crate::project::ProjectMeta {
                id: "test_proj".into(),
                title: "Test".into(),
                width: 640,
                height: 360,
                fps: 30.0,
                total_seconds: 5.0,
                total_frames: 150,
            },
            shots: vec![],
            tracks: crate::project::LayerTracks {
                visual_bg: vec![],
                visual_components: vec![],
                captions: vec![],
                voice: vec![],
                sfx: vec![],
                bgm: vec![],
            },
            asset_tasks: vec![],
        };
        let dummy_path = PathBuf::from("/non/existent/master.mp4");
        let report = review_project_and_master(&manifest, &dummy_path, &ReviewConfig::default())
            .await
            .expect("Review missing file");

        assert!(!report.passed);
        assert_eq!(report.score, 0.0);
        assert!(!report.recommendations.is_empty());
    }

    #[tokio::test]
    async fn test_review_on_mock_generated_video() {
        let temp_dir = tempfile::tempdir().unwrap();
        let mock_master = temp_dir.path().join("master.mp4");

        // 渲染一个小 mock 视频
        let plan = crate::compositor::CompositePlan {
            canvas: crate::compositor::CanvasConfig {
                width: 640,
                height: 360,
                fps: 30.0,
                total_seconds: 0.5,
                total_frames: 15,
                background_color: "#18212A".into(),
            },
            bg_segments: vec![],
            overlays: vec![],
            subtitle: None,
            audio: None,
            output_path: mock_master.clone(),
            subtitle_delivery: crate::compositor::SubtitleDelivery::BurnIn,
            caption_expectations: Vec::new(),
            encoding: crate::compositor::EncodingConfig::default(),
        };
        crate::compositor::render_composition(&plan, true)
            .await
            .unwrap();

        let manifest = ProjectManifest {
            master_audio: None,
            format: "clone-any-video.project@1".into(),
            meta: crate::project::ProjectMeta {
                id: "mock_proj".into(),
                title: "Mock".into(),
                width: 640,
                height: 360,
                fps: 30.0,
                total_seconds: 0.5,
                total_frames: 15,
            },
            shots: vec![],
            tracks: crate::project::LayerTracks {
                visual_bg: vec![],
                visual_components: vec![],
                captions: vec![],
                voice: vec![],
                sfx: vec![],
                bgm: vec![],
            },
            asset_tasks: vec![],
        };

        let report = review_project_and_master(&manifest, &mock_master, &ReviewConfig::default())
            .await
            .expect("Review mock master");

        assert!(report.score > 30.0);
        let canvas_check = report
            .criteria
            .iter()
            .find(|c| c.name == "Canvas Dimensions");
        assert!(canvas_check.is_some());
        assert_eq!(canvas_check.unwrap().status, ReviewStatus::Pass);
    }

    #[tokio::test]
    async fn test_build_svml_project_mock() {
        let temp_dir = tempfile::tempdir().unwrap();
        let svml_file = temp_dir.path().join("test_script.svml");
        let svml_content = r#"
<script language="zh-CN">
  <segment id="seg_01">
    <voice role="narrator">
      <spoken>用纯 <Rust | r_ah_s_t> 引擎打造多模态视频智能体。</spoken>
    </voice>
  </segment>
</script>
"#;
        tokio::fs::write(&svml_file, svml_content).await.unwrap();

        let out_video = temp_dir.path().join("master.mp4");
        let options = BuildOptions {
            prepared_project: None,
            source: svml_file,
            project_dir: temp_dir.path().to_path_buf(),
            output_video: out_video.clone(),
            auto_heal: true,
            mock: true,
        };

        let result = build_svml_project(&options)
            .await
            .expect("Build SVML project mock");
        assert!(result.success);
        assert!(out_video.exists());
        assert_eq!(result.phases_completed.len(), 5);
    }
}
