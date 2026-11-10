use anyhow::{bail, Context, Result};
use image::{Rgb, RgbImage};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::process::Command;

use crate::project::ProjectManifest;

// ============================================================================
// 1. Hyperframes 多层合成领域模型 (Composition Domain Model)
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CanvasConfig {
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub total_seconds: f64,
    pub total_frames: u64,
    pub background_color: String,
}

impl Default for CanvasConfig {
    fn default() -> Self {
        Self {
            width: 640,
            height: 360,
            fps: 30.0,
            total_seconds: 10.0,
            total_frames: 300,
            background_color: "black".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BgSegment {
    pub id: String,
    pub source_path: PathBuf,
    pub start_seconds: f64,
    pub duration_seconds: f64,
    pub start_frame: u64,
    pub end_frame: u64,
    pub is_still: bool,
    #[serde(default)]
    pub sampling: crate::sampling::Sampling,
    #[serde(default)]
    pub source_frames: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OverlaySegment {
    pub id: String,
    pub source_path: PathBuf,
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub start_frame: u64,
    pub end_frame: u64,
    pub x: i32,
    pub y: i32,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub opacity: f64,
    #[serde(default)]
    pub sampling: crate::sampling::Sampling,
    #[serde(default)]
    pub source_frames: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum SubtitleFormat {
    Ass,
    Vtt,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SubtitleConfig {
    pub path: PathBuf,
    pub format: SubtitleFormat,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AudioConfig {
    pub path: PathBuf,
    pub sample_rate: u32,
    pub channels: u16,
    pub bitrate: String,
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            path: PathBuf::new(),
            sample_rate: 48_000,
            channels: 2,
            bitrate: "192k".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EncodingConfig {
    pub crf: u32,
    pub preset: String,
    pub color_matrix: String,
    pub pix_fmt: String,
}

impl Default for EncodingConfig {
    fn default() -> Self {
        Self {
            crf: 21,
            preset: "veryfast".into(),
            color_matrix: "bt709".into(),
            pix_fmt: "yuv420p".into(),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum SubtitleDelivery {
    #[default]
    BurnIn,
    Soft,
    None,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CompositePlan {
    #[serde(default)]
    pub caption_expectations: Vec<crate::delivery::CaptionFact>,
    #[serde(default)]
    pub subtitle_delivery: SubtitleDelivery,
    pub canvas: CanvasConfig,
    pub bg_segments: Vec<BgSegment>,
    pub overlays: Vec<OverlaySegment>,
    pub subtitle: Option<SubtitleConfig>,
    pub audio: Option<AudioConfig>,
    pub output_path: PathBuf,
    pub encoding: EncodingConfig,
}

// ============================================================================
// 2. FFmpeg Filtergraph 编译引擎 (Filtergraph Compiler)
// ============================================================================

#[derive(Debug, Clone, PartialEq)]
pub struct CompiledFiltergraph {
    pub input_args: Vec<String>,
    pub filter_complex: String,
    pub map_args: Vec<String>,
    pub output_args: Vec<String>,
}

/// 探测当前系统中安装的 FFmpeg 是否编译了 libass / subtitles 滤镜
pub fn check_ffmpeg_has_subtitles_filter() -> bool {
    if let Ok(out) = std::process::Command::new("ffmpeg")
        .arg("-filters")
        .output()
    {
        let stdout = String::from_utf8_lossy(&out.stdout);
        for line in stdout.lines() {
            let trimmed = line.trim_start();
            if trimmed.contains(" subtitles ") || trimmed.contains(" ass ") {
                return true;
            }
        }
    }
    false
}

/// 转义 FFmpeg 滤镜参数中的特殊字符 (特别是 Windows/macOS 路径中的冒号与反斜杠)
pub fn escape_ffmpeg_filter_path(path: &Path) -> String {
    let s = path.to_string_lossy().to_string();
    s.replace('\\', "/").replace(':', "\\:")
}

/// 将 CompositePlan 编译为完整的 FFmpeg 执行参数与复杂滤镜链
pub fn compile_composite_filtergraph(plan: &CompositePlan) -> Result<CompiledFiltergraph> {
    if !plan.canvas.fps.is_finite() || plan.canvas.fps <= 0.0 || plan.canvas.total_frames == 0 {
        bail!("Invalid canvas frame clock");
    }
    let mut input_args = Vec::new();
    let mut filter_chains = Vec::new();
    let mut current_input_index = 0usize;

    // 1. 注册背景轨道输入 (Background Track Inputs)
    let mut bg_input_indices = Vec::new();
    for seg in &plan.bg_segments {
        input_args.push("-i".into());
        input_args.push(seg.source_path.to_string_lossy().to_string());
        bg_input_indices.push((current_input_index, seg));
        current_input_index += 1;
    }

    // 2. 注册前景动态组件输入 (Overlay Inputs)
    let mut overlay_input_indices = Vec::new();
    for ov in &plan.overlays {
        input_args.push("-i".into());
        input_args.push(ov.source_path.to_string_lossy().to_string());
        overlay_input_indices.push((current_input_index, ov));
        current_input_index += 1;
    }

    // 3. 注册音频输入 (Audio Input)
    let mut audio_input_index = None;
    if let Some(audio) = &plan.audio {
        if audio.path.exists() {
            input_args.push("-i".into());
            input_args.push(audio.path.to_string_lossy().to_string());
            audio_input_index = Some(current_input_index);
            current_input_index += 1;
        }
    }

    let has_sub_filter = check_ffmpeg_has_subtitles_filter();
    let mut soft_sub_input_index = None;
    if let Some(sub) = &plan.subtitle {
        if plan.subtitle_delivery != SubtitleDelivery::None && !sub.path.exists() {
            bail!("Missing subtitle file {:?}", sub.path);
        }
        if plan.subtitle_delivery == SubtitleDelivery::BurnIn && !has_sub_filter {
            bail!("Burn-in subtitles require FFmpeg libass. Use caption render for a real Chrome overlay, or explicitly select --subtitle-delivery soft. No silent subtitle fallback is permitted");
        }
        if plan.subtitle_delivery == SubtitleDelivery::Soft {
            input_args.extend(["-i".into(), sub.path.to_string_lossy().into()]);
            soft_sub_input_index = Some(current_input_index);
            current_input_index += 1;
        }
    }
    let _ = current_input_index;

    let w = plan.canvas.width;
    let h = plan.canvas.height;
    let fps = plan.canvas.fps;
    let bg_color = &plan.canvas.background_color;

    // Integer, half-open frame ranges own the edit. Normalize before trimming.
    let base_label: String;
    let mut inputs = Vec::new();
    let mut cursor = 0u64;
    let mut sorted = bg_input_indices.clone();
    sorted.sort_by_key(|(_, s)| s.start_frame);
    for (idx, seg) in &sorted {
        if seg.end_frame <= seg.start_frame
            || seg.end_frame > plan.canvas.total_frames
            || seg.start_frame < cursor
        {
            bail!("Invalid/overlapping shot frame range for '{}'", seg.id);
        }
        if seg.start_frame > cursor {
            let label = format!("[gap_{}]", cursor);
            filter_chains.push(format!(
                "color=c={}:s={}x{}:r={},trim=end_frame={},settb=1/{},setpts=N{}",
                bg_color,
                w,
                h,
                fps,
                seg.start_frame - cursor,
                fps,
                label
            ));
            inputs.push(label);
        }
        let label = format!("[bg_{}]", idx);
        let sampling = seg.sampling.filters(
            seg.end_frame - seg.start_frame,
            seg.source_frames,
            seg.is_still,
            fps,
        )?;
        filter_chains.push(format!("[{}:v]fps={},{},scale={}:{}:force_original_aspect_ratio=decrease,pad={}:{}:(ow-iw)/2:(oh-ih)/2:color={},setsar=1{}",idx,fps,sampling,w,h,w,h,bg_color,label));
        inputs.push(label);
        cursor = seg.end_frame;
    }
    if cursor < plan.canvas.total_frames {
        let label = "[gap_tail]".to_string();
        filter_chains.push(format!(
            "color=c={}:s={}x{}:r={},trim=end_frame={},settb=1/{},setpts=N{}",
            bg_color,
            w,
            h,
            fps,
            plan.canvas.total_frames - cursor,
            fps,
            label
        ));
        inputs.push(label);
    }
    if inputs.len() == 1 {
        base_label = inputs[0].clone();
    } else {
        filter_chains.push(format!(
            "{}concat=n={}:v=1:a=0[bg_concat]",
            inputs.join(""),
            inputs.len()
        ));
        base_label = "[bg_concat]".into();
    }

    // 6. 逐层挂载视觉动态组件 (Overlay Track Chaining)
    filter_chains.push(format!("{}settb=1/{},setpts=N[bg_clock]", base_label, fps));
    let mut current_layer_label = "[bg_clock]".to_string();
    for (layer_num, (in_idx, ov)) in overlay_input_indices.iter().enumerate() {
        let overlay_source_label = format!("[ov_src_{}]", in_idx);
        let next_layer_label = format!("[layer_{}]", layer_num);

        // 组件流标准化预处理：将 PTS 准确平移到全局绝对时间 ov.start_seconds，
        // 彻底解决局部媒体时长短于全片时间、到达 enable 窗口前已提前 EOF 消失的 bug
        let target_w = ov.width.unwrap_or(w);
        let target_h = ov.height.unwrap_or(h);
        if ov.end_frame <= ov.start_frame || ov.end_frame > plan.canvas.total_frames {
            bail!("Invalid overlay range for '{}'", ov.id);
        }
        let sampling = ov.sampling.filters(
            ov.end_frame - ov.start_frame,
            ov.source_frames,
            crate::sampling::is_still(&ov.source_path),
            fps,
        )?;
        filter_chains.push(format!(
            "[{}:v]fps={},{},setpts=N+{},scale={}:{},format=rgba{}",
            in_idx, fps, sampling, ov.start_frame, target_w, target_h, overlay_source_label
        ));
        filter_chains.push(format!(
            "{}{}overlay=x={}:y={}:enable='gte(n,{})*lt(n,{})':eof_action=pass{}",
            current_layer_label,
            overlay_source_label,
            ov.x,
            ov.y,
            ov.start_frame,
            ov.end_frame,
            next_layer_label
        ));
        current_layer_label = next_layer_label;
    }

    // 7. 字幕处理 (硬压烧录若支持)
    if has_sub_filter && plan.subtitle_delivery == SubtitleDelivery::BurnIn {
        if let Some(sub) = &plan.subtitle {
            if sub.path.exists() {
                let sub_path_escaped = escape_ffmpeg_filter_path(&sub.path);
                let sub_output_label = "[sub_layer]";
                filter_chains.push(format!(
                    "{}subtitles=filename='{}'{}",
                    current_layer_label, sub_path_escaped, sub_output_label
                ));
                current_layer_label = sub_output_label.into();
            }
        }
    }

    // 8. 广播级 BT.709 色彩矩阵与 TV 电平输出收口 (Broadcast Specification)
    filter_chains.push(format!(
        "{}scale=out_color_matrix={}:out_range=tv,format={},setparams=color_primaries={}:color_trc={}:colorspace={}:range=tv[final_v]",
        current_layer_label,
        plan.encoding.color_matrix,
        plan.encoding.pix_fmt,
        plan.encoding.color_matrix,
        plan.encoding.color_matrix,
        plan.encoding.color_matrix
    ));

    if let Some(index) = audio_input_index {
        let samples = (plan.canvas.total_frames as f64 / plan.canvas.fps * 48000.0).round() as u64;
        filter_chains.push(format!("[{}:a:0]aresample=48000,aformat=channel_layouts=stereo,apad=whole_len={},atrim=end_sample={},asetpts=PTS-STARTPTS[final_a]",index,samples,samples));
    }
    let filter_complex = filter_chains.join(";");

    // 9. 映射参数与编码配置 (Mapping & Encoding Arguments)
    let mut map_args = vec!["-map".into(), "[final_v]".into()];
    let mut output_args = Vec::new();

    if audio_input_index.is_some() {
        map_args.push("-map".into());
        map_args.push("[final_a]".into());
        output_args.push("-c:a".into());
        output_args.push("aac".into());
        output_args.push("-ar".into());
        output_args.push("48000".into());
        output_args.push("-ac".into());
        output_args.push("2".into());
        output_args.push("-b:a".into());
        output_args.push("192k".into());
    } else {
        output_args.push("-an".into());
    }

    // 若启用了软字幕封装流
    if let Some(s_idx) = soft_sub_input_index {
        map_args.push("-map".into());
        map_args.push(format!("{}:s:0", s_idx));
        output_args.push("-c:s".into());
        output_args.push("mov_text".into());
        output_args.push("-metadata:s:s:0".into());
        output_args.push("language=eng".into());
    }

    output_args.push("-c:v".into());
    output_args.push("libx264".into());
    output_args.push("-crf".into());
    output_args.push(format!("{}", plan.encoding.crf));
    output_args.push("-preset".into());
    output_args.push(plan.encoding.preset.clone());
    output_args.push("-pix_fmt".into());
    output_args.push(plan.encoding.pix_fmt.clone());
    output_args.extend([
        "-frames:v".into(),
        plan.canvas.total_frames.to_string(),
        "-fps_mode".into(),
        "cfr".into(),
    ]);
    output_args.push("-movflags".into());
    output_args.push("+faststart".into());
    output_args.push(plan.output_path.to_string_lossy().to_string());

    Ok(CompiledFiltergraph {
        input_args,
        filter_complex,
        map_args,
        output_args,
    })
}

// ============================================================================
// 3. 项目蓝图清单解析为合成方案 (Manifest to Plan Compiler)
// ============================================================================

fn overlay_caption_facts(layer: &OverlaySegment, fps: f64) -> Vec<crate::delivery::CaptionFact> {
    let Some(details) = crate::artifacts::details(&layer.source_path) else {
        return Vec::new();
    };
    if details["fps"].as_f64() != Some(fps) {
        return Vec::new();
    }
    let mut facts: Vec<crate::delivery::CaptionFact> =
        serde_json::from_value(details["caption_windows"].clone()).unwrap_or_default();
    let length = layer.end_frame.saturating_sub(layer.start_frame);
    facts.retain(|f| f.start_frame < length && f.end_frame > f.start_frame);
    for fact in &mut facts {
        fact.start_frame += layer.start_frame;
        fact.end_frame = fact.end_frame.min(length) + layer.start_frame;
    }
    facts
}

/// 从已渲染完成的项目清单（如 project_with_rendered_components.json）构建全局合成计划
pub fn compile_project_to_composite_plan(
    manifest: &ProjectManifest,
    project_dir: &Path,
    output_path: &Path,
) -> Result<CompositePlan> {
    let canvas = CanvasConfig {
        width: manifest.meta.width,
        height: manifest.meta.height,
        fps: manifest.meta.fps,
        total_seconds: manifest.meta.total_seconds,
        total_frames: manifest.meta.total_frames,
        background_color: "#111827".into(), // 深海科技底色
    };

    // 1. 编译背景分镜序列 (Background Shots)
    let mut bg_segments = Vec::new();
    for shot in &manifest.shots {
        // 探测背景视频候选路径
        let shot_video_path = project_dir
            .join("assets")
            .join("video")
            .join(format!("{}.mp4", shot.id));
        let shot_still_path = project_dir
            .join("assets")
            .join("visual")
            .join(format!("{}.png", shot.id));

        let prepared = manifest
            .tracks
            .visual_bg
            .iter()
            .find(|item| item.start_frame == shot.start_frame && item.end_frame == shot.end_frame)
            .and_then(|item| item.payload.get("source"))
            .and_then(|v| v.as_str());
        if let Some(source) = prepared {
            let path = PathBuf::from(source);
            let path = if path.is_absolute() {
                path
            } else {
                project_dir.join(path)
            };
            if !path.is_file() {
                bail!("Missing explicitly prepared background {:?}", path);
            }
            let is_still = path
                .extension()
                .and_then(|v| v.to_str())
                .is_some_and(|v| matches!(v, "png" | "jpg" | "jpeg" | "webp"));
            bg_segments.push(BgSegment {
                id: shot.id.clone(),
                source_path: path,
                start_seconds: shot.start_seconds,
                duration_seconds: shot.duration_seconds,
                start_frame: shot.start_frame,
                end_frame: shot.end_frame,
                is_still,
                sampling: Default::default(),
                source_frames: None,
            });
        } else if shot_video_path.exists() {
            bg_segments.push(BgSegment {
                id: shot.id.clone(),
                source_path: shot_video_path,
                start_seconds: shot.start_seconds,
                duration_seconds: shot.duration_seconds,
                start_frame: shot.start_frame,
                end_frame: shot.end_frame,
                is_still: false,
                sampling: Default::default(),
                source_frames: None,
            });
        } else if shot_still_path.exists() {
            bg_segments.push(BgSegment {
                id: shot.id.clone(),
                source_path: shot_still_path,
                start_seconds: shot.start_seconds,
                duration_seconds: shot.duration_seconds,
                start_frame: shot.start_frame,
                end_frame: shot.end_frame,
                is_still: true,
                sampling: Default::default(),
                source_frames: None,
            });
        } else {
            anyhow::bail!(
                "Missing required background asset for shot '{}': neither video ({:?}) nor still ({:?}) exists",
                shot.id,
                shot_video_path,
                shot_still_path
            );
        }
    }

    // 2. 编译前景组件叠加层 (Visual Components)
    let mut overlays = Vec::new();
    for item in &manifest.tracks.visual_components {
        let video_path_str = item
            .payload
            .get("video_path")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        let comp_video_path = if !video_path_str.is_empty() {
            let path = PathBuf::from(video_path_str);
            if path.is_absolute() {
                path
            } else {
                project_dir.join(path)
            }
        } else {
            project_dir
                .join("assets")
                .join("video_components")
                .join(format!("{}.mp4", item.id))
        };

        if comp_video_path.exists() {
            overlays.push(OverlaySegment {
                id: item.id.clone(),
                source_path: comp_video_path,
                start_seconds: item.start_seconds,
                end_seconds: item.end_seconds,
                start_frame: item.start_frame,
                end_frame: item.end_frame,
                x: 0,
                y: 0,
                width: Some(manifest.meta.width),
                height: Some(manifest.meta.height),
                opacity: 1.0,
                sampling: Default::default(),
                source_frames: None,
            });
        } else {
            anyhow::bail!(
                "Missing required visual component video for '{}' at {:?}",
                item.id,
                comp_video_path
            );
        }
    }

    for segment in &mut bg_segments {
        if let Some(row) = manifest.tracks.visual_bg.iter().find(|r| {
            r.payload["shot_id"] == segment.id
                || (r.start_frame == segment.start_frame && r.end_frame == segment.end_frame)
        }) {
            if let Some(value) = row.payload.get("sampling") {
                segment.sampling = serde_json::from_value(value.clone())?;
            }
        }
    }
    for overlay in &mut overlays {
        if let Some(value) = manifest
            .tracks
            .visual_components
            .iter()
            .find(|r| r.id == overlay.id)
            .and_then(|r| r.payload.get("sampling"))
        {
            overlay.sampling = serde_json::from_value(value.clone())?;
        }
    }

    // 3. 探测字幕文件 (Subtitle Track)
    let ass_path = project_dir.join("assets").join("subtitles.ass");
    let vtt_path = project_dir.join("assets").join("subtitles.vtt");
    let root_ass_path = project_dir.join("subtitles.ass");
    let root_vtt_path = project_dir.join("subtitles.vtt");
    let caption_windows: Vec<_> = overlays
        .iter()
        .flat_map(|layer| overlay_caption_facts(layer, canvas.fps))
        .collect();
    let rendered_captions = crate::delivery::captions_covered(
        &manifest.tracks.captions,
        &serde_json::json!({"caption_windows":caption_windows}),
    );
    let known_caption_layer = overlays.iter().any(|layer| {
        crate::artifacts::details(&layer.source_path)
            .is_some_and(|d| d["caption_cues"].is_array() || d["caption_windows"].is_array())
    });
    if known_caption_layer && !manifest.tracks.captions.is_empty() && !rendered_captions {
        bail!("Prepared caption layer text/timing differs from project; re-render captions before composition");
    }
    let subtitle = if rendered_captions {
        None
    } else if ass_path.exists() {
        Some(SubtitleConfig {
            path: ass_path,
            format: SubtitleFormat::Ass,
        })
    } else if vtt_path.exists() {
        Some(SubtitleConfig {
            path: vtt_path,
            format: SubtitleFormat::Vtt,
        })
    } else if root_ass_path.exists() {
        Some(SubtitleConfig {
            path: root_ass_path,
            format: SubtitleFormat::Ass,
        })
    } else if root_vtt_path.exists() {
        Some(SubtitleConfig {
            path: root_vtt_path,
            format: SubtitleFormat::Vtt,
        })
    } else {
        None
    };

    // 4. 探测母带音频 (Master Audio Track)
    let master_mix_path = if let Some(path) = &manifest.master_audio {
        let path = if path.is_absolute() {
            path.clone()
        } else {
            project_dir.join(path)
        };
        if !path.is_file() {
            bail!("Missing explicitly prepared master audio {:?}", path);
        }
        path
    } else {
        project_dir.join("assets/audio/master_mix.wav")
    };
    let audio = if master_mix_path.exists() {
        Some(AudioConfig {
            path: master_mix_path,
            sample_rate: 48_000,
            channels: 2,
            bitrate: "192k".into(),
        })
    } else {
        None
    };

    Ok(CompositePlan {
        caption_expectations: manifest
            .tracks
            .captions
            .iter()
            .map(crate::delivery::caption_fact)
            .collect(),
        canvas,
        bg_segments,
        overlays,
        subtitle,
        audio,
        output_path: output_path.to_path_buf(),
        subtitle_delivery: SubtitleDelivery::BurnIn,
        encoding: EncodingConfig::default(),
    })
}

// ============================================================================
// 4. 渲染压制引擎 (Execution & Render Engine)
// ============================================================================

/// 执行多层压制合成并导出最终广播级 master.mp4
pub async fn render_composition(plan: &CompositePlan, force_mock: bool) -> Result<()> {
    if let Some(parent) = plan.output_path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    if force_mock {
        render_mock_master_video(plan).await?;
        crate::artifacts::record(
            &plan.output_path,
            crate::artifacts::Mode::Mock,
            "mock-compositor",
        )?;
        return Ok(());
    }

    let normalized = tempfile::tempdir()?;
    let mut resolved = plan.clone();
    for (i, seg) in resolved.bg_segments.iter_mut().enumerate() {
        if !seg.is_still {
            let (path, count) = crate::sampling::prepare(
                &seg.source_path,
                plan.canvas.fps,
                false,
                normalized.path(),
                i,
            )
            .await?;
            seg.source_path = path;
            seg.source_frames = Some(count);
        }
    }
    for (i, ov) in resolved.overlays.iter_mut().enumerate() {
        if crate::sampling::is_still(&ov.source_path) {
            continue;
        }
        let (path, count) = crate::sampling::prepare(
            &ov.source_path,
            plan.canvas.fps,
            true,
            normalized.path(),
            i + resolved.bg_segments.len(),
        )
        .await?;
        ov.source_path = path;
        ov.source_frames = Some(count);
    }
    let compiled = compile_composite_filtergraph(&resolved)?;

    let mut cmd = Command::new("ffmpeg");
    cmd.arg("-y").arg("-v").arg("error");

    // 添加输入参数
    for arg in &compiled.input_args {
        cmd.arg(arg);
    }

    // 添加复杂滤镜
    cmd.arg("-filter_complex").arg(&compiled.filter_complex);

    // 添加映射参数
    for arg in &compiled.map_args {
        cmd.arg(arg);
    }

    // 添加输出编码参数
    for arg in &compiled.output_args {
        cmd.arg(arg);
    }

    cmd.stdout(Stdio::null()).stderr(Stdio::piped());

    let output = cmd
        .output()
        .await
        .context("Failed executing FFmpeg compositor")?;
    if !output.status.success() || !plan.output_path.exists() {
        let err_msg = String::from_utf8_lossy(&output.stderr);
        bail!(
            "FFmpeg composition failed (exit code {:?}): {}",
            output.status.code(),
            err_msg
        );
    }

    let inputs = plan
        .bg_segments
        .iter()
        .map(|s| s.source_path.as_path())
        .chain(plan.overlays.iter().map(|s| s.source_path.as_path()))
        .chain(plan.audio.iter().map(|s| s.path.as_path()));
    let mode = crate::artifacts::combined(inputs);
    let caption_windows: Vec<_> = plan
        .overlays
        .iter()
        .flat_map(|layer| overlay_caption_facts(layer, plan.canvas.fps))
        .collect();
    let caption_cues: Vec<_> = caption_windows.iter().map(|c| c.id.clone()).collect();
    let subtitle = if plan.subtitle.is_some() {
        serde_json::to_value(plan.subtitle_delivery)?
    } else {
        serde_json::json!("none")
    };
    let files: Vec<PathBuf> = plan
        .bg_segments
        .iter()
        .map(|s| s.source_path.clone())
        .chain(plan.overlays.iter().map(|s| s.source_path.clone()))
        .chain(plan.audio.iter().map(|a| a.path.clone()))
        .collect();
    let mut files = files;
    if let Some(sub) = &plan.subtitle {
        if plan.subtitle_delivery != SubtitleDelivery::None {
            files.push(sub.path.clone());
        }
    }
    let actual = crate::probe::probe_media(&plan.output_path).await?;
    if actual.frame_count != Some(plan.canvas.total_frames) {
        bail!(
            "Encoded frame count {:?} != planned {}",
            actual.frame_count,
            plan.canvas.total_frames
        );
    }
    crate::artifacts::record_derived(
        &plan.output_path,
        mode,
        "compositor",
        &files,
        serde_json::json!({"subtitle_delivery":subtitle,"caption_cues":caption_cues,"caption_windows":caption_windows,"caption_expectations":plan.caption_expectations,"frame_count":actual.frame_count,"fps":plan.canvas.fps,"sampling":{"backgrounds":resolved.bg_segments.iter().map(|s|serde_json::json!({"id":s.id,"sampling":s.sampling,"source_frames":s.source_frames})).collect::<Vec<_>>(),"overlays":resolved.overlays.iter().map(|s|serde_json::json!({"id":s.id,"sampling":s.sampling,"source_frames":s.source_frames})).collect::<Vec<_>>() }}),
    )?;
    Ok(())
}

/// 纯 Rust Mock 离线安全合成器 (用于 CI 验证与零依赖运行)
async fn render_mock_master_video(plan: &CompositePlan) -> Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let frames_dir = temp_dir.path().join("mock_master_frames");
    tokio::fs::create_dir_all(&frames_dir).await?;

    let w = plan.canvas.width;
    let h = plan.canvas.height;
    let total_frames = 15.min(plan.canvas.total_frames.max(1));

    // 生成若干代表多层融合的渐变合成帧
    for f in 0..total_frames {
        let mut img = RgbImage::new(w, h);
        let ratio = f as f64 / total_frames as f64;
        for y in 0..h {
            let y_norm = y as f64 / h as f64;
            for x in 0..w {
                let x_norm = x as f64 / w as f64;
                let r = (24.0 + ratio * 50.0 + x_norm * 40.0) as u8;
                let g = (32.0 + y_norm * 40.0) as u8;
                let b = (48.0 + ratio * 80.0) as u8;
                img.put_pixel(x, y, Rgb([r, g, b]));
            }
        }
        let frame_path = frames_dir.join(format!("frame_{:06}.png", f));
        img.save(frame_path)?;
    }

    // 使用基础 ffmpeg 转为 mp4
    let pattern = frames_dir.join("frame_%06d.png");
    let mut cmd = Command::new("ffmpeg");
    cmd.arg("-y")
        .arg("-v")
        .arg("error")
        .arg("-framerate")
        .arg(format!("{:.3}", plan.canvas.fps))
        .arg("-i")
        .arg(pattern.to_string_lossy().to_string())
        .arg("-c:v")
        .arg("libx264")
        .arg("-pix_fmt")
        .arg("yuv420p")
        .arg(plan.output_path.to_string_lossy().to_string());

    let status = cmd.status().await?;
    if !status.success() || !plan.output_path.exists() {
        bail!("Mock master render failed");
    }

    Ok(())
}

// ============================================================================
// 单元测试
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_escape_ffmpeg_filter_path() {
        let p = PathBuf::from("/Users/stoic/Code/clone-any-video/assets/subtitles.ass");
        let escaped = escape_ffmpeg_filter_path(&p);
        assert!(!escaped.contains('\\')); // Unix 路径不含反斜杠

        #[cfg(target_os = "windows")]
        {
            let win_p = PathBuf::from(r"C:\project\sub.ass");
            let win_esc = escape_ffmpeg_filter_path(&win_p);
            assert!(win_esc.contains("C\\:/"));
        }
    }

    #[test]
    fn test_compile_composite_filtergraph_syntax() {
        let plan = CompositePlan {
            caption_expectations: Vec::new(),
            canvas: CanvasConfig {
                width: 640,
                height: 360,
                fps: 30.0,
                total_seconds: 5.0,
                total_frames: 150,
                background_color: "#18212A".into(),
            },
            bg_segments: vec![
                BgSegment {
                    id: "shot_01".into(),
                    source_path: PathBuf::from("shot_01.mp4"),
                    start_seconds: 0.0,
                    duration_seconds: 2.5,
                    start_frame: 0,
                    end_frame: 75,
                    is_still: false,
                    sampling: Default::default(),
                    source_frames: None,
                },
                BgSegment {
                    id: "shot_02".into(),
                    source_path: PathBuf::from("shot_02.png"),
                    start_seconds: 2.5,
                    duration_seconds: 2.5,
                    start_frame: 75,
                    end_frame: 150,
                    is_still: true,
                    sampling: Default::default(),
                    source_frames: None,
                },
            ],
            overlays: vec![OverlaySegment {
                id: "comp_01".into(),
                source_path: PathBuf::from("comp_01.mp4"),
                start_seconds: 0.5,
                end_seconds: 2.0,
                start_frame: 15,
                end_frame: 60,
                x: 0,
                y: 0,
                width: Some(640),
                height: Some(360),
                opacity: 1.0,
                sampling: Default::default(),
                source_frames: None,
            }],
            subtitle: None,
            audio: None,
            output_path: PathBuf::from("master.mp4"),
            subtitle_delivery: SubtitleDelivery::BurnIn,
            encoding: EncodingConfig::default(),
        };

        let compiled = compile_composite_filtergraph(&plan).unwrap();

        // 验证两个背景输入 + 一个组件输入
        assert_eq!(compiled.input_args.len(), 6); // [-i, s1, -i, s2, -i, c1]
        assert!(compiled.filter_complex.contains("concat=n=2:v=1:a=0"));
        assert!(compiled
            .filter_complex
            .contains("overlay=x=0:y=0:enable='gte(n,15)*lt(n,60)'"));
        assert!(compiled.filter_complex.contains("out_color_matrix=bt709"));
        assert!(compiled
            .filter_complex
            .contains("setparams=color_primaries=bt709"));
    }

    #[tokio::test]
    async fn test_render_mock_master_video() {
        let temp_dir = tempfile::tempdir().unwrap();
        let out_mp4 = temp_dir.path().join("master_test.mp4");

        let plan = CompositePlan {
            caption_expectations: Vec::new(),
            canvas: CanvasConfig {
                width: 320,
                height: 180,
                fps: 30.0,
                total_seconds: 0.2,
                total_frames: 6,
                background_color: "black".into(),
            },
            bg_segments: vec![],
            overlays: vec![],
            subtitle: None,
            audio: None,
            output_path: out_mp4.clone(),
            subtitle_delivery: SubtitleDelivery::BurnIn,
            encoding: EncodingConfig::default(),
        };

        render_mock_master_video(&plan)
            .await
            .expect("Mock master render");
        assert!(out_mp4.exists());
        let meta = std::fs::metadata(&out_mp4).unwrap();
        assert!(meta.len() > 1000);
    }

    #[test]
    fn test_background_gaps_and_overlay_pts_shift() {
        let plan = CompositePlan {
            caption_expectations: Vec::new(),
            canvas: CanvasConfig {
                width: 640,
                height: 360,
                fps: 30.0,
                total_seconds: 3.0,
                total_frames: 90,
                background_color: "#18212A".into(),
            },
            bg_segments: vec![
                BgSegment {
                    id: "shot_01".into(),
                    source_path: PathBuf::from("shot_01.mp4"),
                    start_seconds: 0.0,
                    duration_seconds: 1.0,
                    start_frame: 0,
                    end_frame: 30,
                    is_still: false,
                    sampling: Default::default(),
                    source_frames: None,
                },
                BgSegment {
                    id: "shot_02".into(),
                    source_path: PathBuf::from("shot_02.mp4"),
                    start_seconds: 2.0, // 声明 1s 空档
                    duration_seconds: 1.0,
                    start_frame: 60,
                    end_frame: 90,
                    is_still: false,
                    sampling: Default::default(),
                    source_frames: None,
                },
            ],
            overlays: vec![OverlaySegment {
                id: "comp_01".into(),
                source_path: PathBuf::from("comp_01.mp4"),
                start_seconds: 2.0,
                end_seconds: 3.0,
                start_frame: 60,
                end_frame: 90,
                x: 0,
                y: 0,
                width: Some(640),
                height: Some(360),
                opacity: 1.0,
                sampling: Default::default(),
                source_frames: None,
            }],
            subtitle: None,
            audio: None,
            output_path: PathBuf::from("master.mp4"),
            subtitle_delivery: SubtitleDelivery::BurnIn,
            encoding: EncodingConfig::default(),
        };

        let compiled = compile_composite_filtergraph(&plan).unwrap();
        // 验证空档产生了一个 color 块，共计 3 个片段参与 concat
        assert!(compiled.filter_complex.contains("gap_30"));
        assert!(compiled.filter_complex.contains("concat=n=3:v=1:a=0"));
        // 验证组件带有 PTS 延迟平移，使得在 t=2.0 时从帧 0 开始播放
        assert!(compiled.filter_complex.contains("setpts=N+60"));
    }
    #[tokio::test]
    async fn native_composition_preserves_input_provenance() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.png");
        RgbImage::from_pixel(64, 64, Rgb([200, 10, 20]))
            .save(&source)
            .unwrap();
        let plan = CompositePlan {
            caption_expectations: Vec::new(),
            canvas: CanvasConfig {
                width: 64,
                height: 64,
                fps: 30.0,
                total_seconds: 0.2,
                total_frames: 6,
                background_color: "black".into(),
            },
            bg_segments: vec![BgSegment {
                id: "bg".into(),
                source_path: source.clone(),
                start_seconds: 0.0,
                duration_seconds: 0.2,
                start_frame: 0,
                end_frame: 6,
                is_still: true,
                sampling: Default::default(),
                source_frames: None,
            }],
            overlays: vec![],
            subtitle: None,
            audio: None,
            output_path: dir.path().join("master.mp4"),
            subtitle_delivery: SubtitleDelivery::BurnIn,
            encoding: EncodingConfig::default(),
        };
        use crate::artifacts::{self, Mode};
        for mode in [Mode::Unknown, Mode::Mock, Mode::Production] {
            artifacts::record(&source, mode, "fixture").unwrap();
            render_composition(&plan, false).await.unwrap();
            assert_eq!(artifacts::mode(&plan.output_path), mode);
        }
    }

    // Exercises encoded pixels, not just a filtergraph string: the production bug
    // was one extra frame per rounded duration and a one-frame overlay end error.
    #[tokio::test]
    async fn exact_frame_cuts_and_half_open_overlay_are_encoded() {
        let dir = tempfile::tempdir().unwrap();
        let colors = [
            [255, 0, 0],
            [0, 255, 0],
            [0, 0, 255],
            [255, 255, 0],
            [0, 255, 255],
        ];
        let lengths = [56, 71, 105, 116, 116];
        let mut cursor = 0;
        let mut segments = Vec::new();
        for (i, (color, length)) in colors.iter().zip(lengths).enumerate() {
            let path = dir.path().join(format!("{i}.png"));
            RgbImage::from_pixel(32, 32, Rgb(*color))
                .save(&path)
                .unwrap();
            segments.push(BgSegment {
                id: i.to_string(),
                source_path: path,
                start_seconds: cursor as f64 / 30.0,
                duration_seconds: length as f64 / 30.0,
                start_frame: cursor,
                end_frame: cursor + length,
                is_still: true,
                sampling: Default::default(),
                source_frames: None,
            });
            cursor += length;
        }
        let overlay = dir.path().join("overlay.png");
        RgbImage::from_pixel(32, 32, Rgb([255, 0, 255]))
            .save(&overlay)
            .unwrap();
        let audio = dir.path().join("longer-than-video.wav");
        let made = Command::new("ffmpeg")
            .args([
                "-v",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:sample_rate=48000:duration=30",
            ])
            .arg(&audio)
            .output()
            .await
            .unwrap();
        assert!(made.status.success());
        let plan = CompositePlan {
            caption_expectations: Vec::new(),
            canvas: CanvasConfig {
                width: 32,
                height: 32,
                fps: 30.0,
                total_seconds: 464.0 / 30.0,
                total_frames: 464,
                background_color: "black".into(),
            },
            bg_segments: segments,
            overlays: vec![OverlaySegment {
                id: "caption".into(),
                source_path: overlay,
                start_seconds: 56.0 / 30.0,
                end_seconds: 127.0 / 30.0,
                start_frame: 56,
                end_frame: 127,
                x: 0,
                y: 0,
                width: None,
                height: None,
                opacity: 1.0,
                sampling: Default::default(),
                source_frames: None,
            }],
            subtitle: None,
            audio: Some(AudioConfig {
                path: audio,
                ..Default::default()
            }),
            output_path: dir.path().join("master.mp4"),
            subtitle_delivery: SubtitleDelivery::BurnIn,
            encoding: EncodingConfig::default(),
        };
        render_composition(&plan, false).await.unwrap();
        let measured = crate::probe::probe_media(&plan.output_path).await.unwrap();
        assert!(
            (measured.duration_seconds - plan.canvas.total_seconds).abs() < 0.03,
            "Container must end with the authored frame clock"
        );
        assert_eq!(
            crate::probe::probe_media(&plan.output_path)
                .await
                .unwrap()
                .frame_count,
            Some(464)
        );
        let decoded = Command::new("ffmpeg")
            .args(["-v", "error", "-i"])
            .arg(&plan.output_path)
            .args(["-f", "rawvideo", "-pix_fmt", "rgb24", "-"])
            .output()
            .await
            .unwrap();
        assert!(decoded.status.success());
        assert_eq!(decoded.stdout.len(), 464 * 32 * 32 * 3);
        for (frame, color) in [
            (55, colors[0]),
            (56, [255, 0, 255]),
            (126, [255, 0, 255]),
            (127, colors[2]),
            (231, colors[2]),
            (232, colors[3]),
            (347, colors[3]),
            (348, colors[4]),
            (463, colors[4]),
        ] {
            let at = frame * 32 * 32 * 3 + 16 * 32 * 3 + 16 * 3;
            let actual = &decoded.stdout[at..at + 3];
            assert!(
                actual
                    .iter()
                    .zip(color)
                    .all(|(a, b)| (*a as i32 - b as i32).abs() < 12),
                "frame {frame}: {actual:?} expected {color:?}"
            );
        }
    }
    #[test]
    fn caption_proof_tracks_text_timing_and_overlay_placement() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("caption.mov");
        std::fs::write(&path, b"fixture-bytes").unwrap();
        crate::artifacts::record_derived(&path,crate::artifacts::Mode::Mock,"fixture",&[],serde_json::json!({"fps":30.0,"caption_windows":[{"id":"c1","text":"Original product","start_frame":0,"end_frame":6}]})).unwrap();
        let mut overlay = OverlaySegment {
            id: "layer".into(),
            source_path: path,
            start_seconds: 0.0,
            end_seconds: 0.2,
            start_frame: 0,
            end_frame: 6,
            x: 0,
            y: 0,
            width: None,
            height: None,
            opacity: 1.0,
            sampling: Default::default(),
            source_frames: None,
        };
        let mut item = crate::project::TrackItem {
            id: "c1".into(),
            name: "Original product".into(),
            start_seconds: 0.0,
            end_seconds: 0.2,
            start_frame: 0,
            end_frame: 6,
            payload: serde_json::json!({}),
        };
        let proof = |layer: &OverlaySegment| serde_json::json!({"caption_windows":overlay_caption_facts(layer,30.0)});
        assert!(crate::delivery::captions_covered(
            &[item.clone()],
            &proof(&overlay)
        ));
        item.name = "Different product".into();
        assert!(!crate::delivery::captions_covered(
            &[item.clone()],
            &proof(&overlay)
        ));
        item.name = "Original product".into();
        overlay.start_frame = 10;
        overlay.end_frame = 16;
        assert!(!crate::delivery::captions_covered(
            &[item.clone()],
            &proof(&overlay)
        ));
        item.start_frame = 10;
        item.end_frame = 16;
        assert!(crate::delivery::captions_covered(
            &[item.clone()],
            &proof(&overlay)
        ));
        overlay.end_frame = 15;
        assert!(!crate::delivery::captions_covered(
            &[item],
            &proof(&overlay)
        ));
    }
}
