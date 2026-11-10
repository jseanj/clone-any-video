use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::project::{AssetTaskType, ProjectManifest, TaskStatus, TrackItem};
use crate::script::ScriptDocument;

pub const CANONICAL_SAMPLE_RATE: u32 = 48_000;
pub const CANONICAL_CHANNELS: u16 = 2;

/// 音频时钟增益控制点 (AudioGainPoint)
/// 音频电平自动化控制模型
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AudioGainPoint {
    pub sample: u64,
    pub gain: f64,
}

/// 音频轨道类型
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum AudioTrackType {
    Voice,
    Bgm,
    Sfx,
    Ambience,
}

/// 音频片段实例 (Audio Clip)
/// 音频片段规范:
/// Audio Clip = normalized audio source + absolute Window + source-time + mix
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AudioClip {
    pub id: String,
    pub track_type: AudioTrackType,
    pub source_path: PathBuf,
    pub target_start_sample: u64,
    pub target_end_sample_exclusive: u64,
    pub target_start_seconds: f64,
    pub target_end_seconds: f64,
    pub gain: f64,
    pub fade_in_samples: u64,
    pub fade_out_samples: u64,
    pub gain_envelope: Option<Vec<AudioGainPoint>>,
}

/// 智能闪避策略参数 (Audio Ducking Configuration)
/// 混音原则: Shape what leads the ear
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DuckingConfig {
    /// 语音提前闪避前置量 (秒, 预判淡出)
    pub lead_margin_seconds: f64,
    /// 语音结束后音量恢复滞后量 (秒, 延迟淡入)
    pub tail_margin_seconds: f64,
    /// 渐变平滑时长 (秒)
    pub ramp_duration_seconds: f64,
    /// 说话时的 BGM 压制音量增益 (线性值, 0.18 约 -15dB)
    pub ducked_gain: f64,
    /// 无人说话时的 BGM 正常音量增益 (线性值, 0.45 约 -7dB)
    pub normal_gain: f64,
}

impl Default for DuckingConfig {
    fn default() -> Self {
        Self {
            lead_margin_seconds: 0.08,
            tail_margin_seconds: 0.15,
            ramp_duration_seconds: 0.10,
            ducked_gain: 0.18,
            normal_gain: 0.45,
        }
    }
}

/// 音效时刻规范 (SFX Cue Specification)
/// 音效原则: Attach sound to the event it answers
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[allow(dead_code)]
pub struct SfxCueSpec {
    pub id: String,
    pub name: String,
    pub moment_marker: String,
    pub timestamp_seconds: f64,
    pub duration_seconds: f64,
    pub gain: f64,
    pub sound_style: String, // "whoosh", "pop", "ding", "bell", "shutter"
}

/// 完整混音工程方案 (Audio Program Plan)
/// 多轨混音方案模型: AudioProgramPlan
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AudioProgramPlan {
    #[serde(default)]
    pub observed_speech_activity: Vec<(f64, f64)>,
    pub sample_rate: u32,
    pub channels: u16,
    pub total_seconds: f64,
    pub total_samples: u64,
    pub voice_clips: Vec<AudioClip>,
    pub bgm_clip: Option<AudioClip>,
    pub sfx_clips: Vec<AudioClip>,
}

/// 计算指定时间采样的插值增益
#[allow(dead_code)]
pub fn audio_envelope_gain_at(points: &[AudioGainPoint], sample: u64) -> f64 {
    if points.is_empty() {
        return 1.0;
    }
    let first = &points[0];
    if sample <= first.sample {
        return first.gain;
    }
    for i in 1..points.len() {
        let left = &points[i - 1];
        let right = &points[i];
        if sample <= right.sample {
            if right.sample == left.sample {
                return right.gain;
            }
            let progress = (sample - left.sample) as f64 / (right.sample - left.sample) as f64;
            return left.gain + (right.gain - left.gain) * progress;
        }
    }
    points.last().unwrap().gain
}

/// 计算 BGM 智能闪避包络线控制点 (Ducking Gain Envelope)
/// 当 A-roll 人声说话时，BGM 提前平滑降到 ducked_gain；语音结束后滞后恢复到 normal_gain
pub fn calculate_ducking_envelope(
    voice_clips: &[AudioClip],
    total_samples: u64,
    config: &DuckingConfig,
    sample_rate: u32,
) -> Vec<AudioGainPoint> {
    if voice_clips.is_empty() || total_samples == 0 {
        return vec![
            AudioGainPoint {
                sample: 0,
                gain: config.normal_gain,
            },
            AudioGainPoint {
                sample: total_samples,
                gain: config.normal_gain,
            },
        ];
    }

    let lead_samples = (config.lead_margin_seconds * sample_rate as f64).round() as u64;
    let tail_samples = (config.tail_margin_seconds * sample_rate as f64).round() as u64;
    let ramp_samples = (config.ramp_duration_seconds * sample_rate as f64).round() as u64;

    // 1. 提取所有有声区间的扩展闪避区间 [duck_start, duck_end]
    let mut duck_intervals = Vec::new();
    for vc in voice_clips {
        let start = vc.target_start_sample.saturating_sub(lead_samples);
        let end = (vc.target_end_sample_exclusive + tail_samples).min(total_samples);
        duck_intervals.push((start, end));
    }

    // 2. 合并重叠区间；短呼吸间隙保留，斜坡在间隙中压缩。
    duck_intervals.sort_by_key(|&(s, _)| s);
    let mut merged: Vec<(u64, u64)> = Vec::new();
    for (start, end) in duck_intervals {
        if let Some(last) = merged.last_mut() {
            if start <= last.1 {
                last.1 = last.1.max(end);
                continue;
            }
        }
        merged.push((start, end));
    }

    // 3. 构建时间线上升与下降的平滑增益控制点
    let mut raw_points = Vec::new();
    raw_points.push(AudioGainPoint {
        sample: 0,
        gain: config.normal_gain,
    });

    for (index, &(duck_start, duck_end)) in merged.iter().enumerate() {
        let down_samples = if index > 0 {
            ramp_samples.min((duck_start - merged[index - 1].1) / 2)
        } else {
            ramp_samples
        };
        let up_samples = if index + 1 < merged.len() {
            ramp_samples.min((merged[index + 1].0 - duck_end) / 2)
        } else {
            ramp_samples
        };
        let ramp_down_start = duck_start.saturating_sub(down_samples);
        let ramp_up_end = (duck_end + up_samples).min(total_samples);

        // 闪避下降前夕
        if ramp_down_start > 0 {
            raw_points.push(AudioGainPoint {
                sample: ramp_down_start,
                gain: config.normal_gain,
            });
        }
        // 到达压制音量
        raw_points.push(AudioGainPoint {
            sample: duck_start,
            gain: config.ducked_gain,
        });
        // 压制结束时刻
        raw_points.push(AudioGainPoint {
            sample: duck_end,
            gain: config.ducked_gain,
        });
        // 恢复至正常音量
        if ramp_up_end > duck_end {
            raw_points.push(AudioGainPoint {
                sample: ramp_up_end,
                gain: config.normal_gain,
            });
        }
    }

    if raw_points.last().map(|p| p.sample).unwrap_or(0) < total_samples {
        raw_points.push(AudioGainPoint {
            sample: total_samples,
            gain: config.normal_gain,
        });
    }

    // 4. 清理冗余采样点并保证单调递增
    raw_points.sort_by_key(|p| p.sample);
    let mut clean_points: Vec<AudioGainPoint> = Vec::new();
    for p in raw_points {
        if let Some(last) = clean_points.last_mut() {
            if last.sample == p.sample {
                last.gain = p.gain;
                continue;
            }
        }
        clean_points.push(p);
    }

    clean_points
}

/// 本地合成高质量基础音效 (Mock SFX Synthesizer)
pub fn synthesize_mock_sfx(style: &str, out_path: &Path, sample_rate: u32) -> Result<f64> {
    if let Some(p) = out_path.parent() {
        std::fs::create_dir_all(p)?;
    }

    // 根据不同音效风格定制高表现力音频合成参数
    let (filter, duration) = match style {
        "whoosh" => (
            "anoisesrc=d=0.35:c=white:r=48000,lowpass=f=1800,afade=t=in:ss=0:d=0.15,afade=t=out:st=0.15:d=0.20",
            0.35,
        ),
        "ding" | "chime" => (
            "sine=frequency=1400:duration=0.5:sample_rate=48000,afade=t=out:st=0.05:d=0.45",
            0.50,
        ),
        "pop" | "bubble" => (
            "sine=frequency=500:duration=0.12:sample_rate=48000,afade=t=out:st=0.02:d=0.10",
            0.12,
        ),
        "bell" => (
            "sine=frequency=880:duration=0.6:sample_rate=48000,afade=t=out:st=0.10:d=0.50",
            0.60,
        ),
        "shutter" | "click" => (
            "anoisesrc=d=0.15:c=pink:r=48000,afade=t=out:st=0.02:d=0.13",
            0.15,
        ),
        _ => (
            "sine=frequency=1000:duration=0.3:sample_rate=48000,afade=t=out:st=0.05:d=0.25",
            0.30,
        ),
    };

    let status = Command::new("ffmpeg")
        .args([
            "-y",
            "-f",
            "lavfi",
            "-i",
            filter,
            "-ar",
            &sample_rate.to_string(),
            "-ac",
            "2",
            "-c:a",
            "pcm_s16le",
            out_path.to_str().unwrap(),
        ])
        .output()
        .context("Failed to execute ffmpeg to synthesize mock SFX")?;

    if !status.status.success() {
        let err = String::from_utf8_lossy(&status.stderr);
        bail!("FFmpeg mock SFX synthesis failed: {}", err);
    }

    crate::artifacts::record(out_path, crate::artifacts::Mode::Mock, "mock-sfx")?;
    Ok(duration)
}

/// 根据剧本中的 Moment 标记自动吸附音效事件并构建 SFX 音频片段
/// 音效触发: 响应台词语义的音效吸附于 Script Selection 或 Moment
pub fn attach_sfx_to_moments(
    script: &ScriptDocument,
    manifest: &ProjectManifest,
    dir: &Path,
    sample_rate: u32,
) -> Result<Vec<AudioClip>> {
    attach_sfx_with_policy(script, manifest, dir, sample_rate, false)
}
pub fn attach_sfx_with_policy(
    script_doc: &ScriptDocument,
    aligned_manifest: &ProjectManifest,
    sfx_assets_dir: &Path,
    sample_rate: u32,
    allow_mock: bool,
) -> Result<Vec<AudioClip>> {
    let mut sfx_clips = Vec::new();
    std::fs::create_dir_all(sfx_assets_dir)?;

    // A visual shot may cover several segments; only performance windows own speech time.
    for seg in &script_doc.segments {
        let voice_id = format!("voice_{}", seg.id);
        let voice = aligned_manifest
            .tracks
            .voice
            .iter()
            .find(|v| v.id == voice_id)
            .with_context(|| format!("Missing aligned voice window {}", voice_id))?;

        // 处理该段落包含的全部 Moment 标记
        for (idx, moment) in seg.moments.iter().enumerate() {
            let marker_clean = moment.name.trim_matches(&['@', '{', '}', '!'][..]);
            let sfx_id = format!("sfx_{}_{}_{}", seg.id, marker_clean, idx + 1);

            // 根据标记语义推导音效风格
            let style = match marker_clean {
                "question" => "whoosh",
                "merge" | "combine" => "pop",
                "identity" | "brand" => "ding",
                "read" | "scan" => "shutter",
                "build" | "create" => "pop",
                "compare" | "diff" => "whoosh",
                "rust" | "fast" => "bell",
                "open" | "outro" => "ding",
                _ => "pop",
            };

            // 计算 Moment 在分镜内的相对时间戳比例
            let total_units = seg
                .turns
                .iter()
                .map(|t| t.units.len())
                .sum::<usize>()
                .max(1);
            let unit_ratio = (moment.at_unit_index as f64 / total_units as f64).clamp(0.0, 0.95);
            let event_time =
                voice.start_seconds + ((voice.end_seconds - voice.start_seconds) * unit_ratio);

            let sfx_wav_path = sfx_assets_dir.join(format!("{}.wav", sfx_id));
            let sfx_duration = if !sfx_wav_path.exists() {
                if !allow_mock {
                    bail!("Missing prepared SFX {:?}; supply --sfx-plan or explicitly use --mock-sfx for teaching",sfx_wav_path);
                }
                synthesize_mock_sfx(style, &sfx_wav_path, sample_rate)?
            } else {
                if !allow_mock
                    && crate::artifacts::mode(&sfx_wav_path) == crate::artifacts::Mode::Mock
                {
                    bail!("Known Mock SFX requires explicit --mock-sfx");
                }
                let raw = Command::new("ffprobe")
                    .args([
                        "-v",
                        "error",
                        "-show_format",
                        "-show_streams",
                        "-of",
                        "json",
                    ])
                    .arg(&sfx_wav_path)
                    .output()?;
                let probe: serde_json::Value = serde_json::from_slice(&raw.stdout)?;
                if !raw.status.success()
                    || !probe["streams"]
                        .as_array()
                        .is_some_and(|xs| xs.iter().any(|s| s["codec_type"] == "audio"))
                {
                    bail!("Invalid prepared SFX");
                }
                probe["format"]["duration"]
                    .as_str()
                    .context("SFX duration missing")?
                    .parse::<f64>()?
            };

            let start_sample = (event_time * sample_rate as f64).round() as u64;
            let end_sample = start_sample + (sfx_duration * sample_rate as f64).round() as u64;

            sfx_clips.push(AudioClip {
                id: sfx_id,
                track_type: AudioTrackType::Sfx,
                source_path: sfx_wav_path,
                target_start_sample: start_sample,
                target_end_sample_exclusive: end_sample,
                target_start_seconds: event_time,
                target_end_seconds: event_time + sfx_duration,
                gain: 0.40, // SFX 恰到好处的点缀增益
                fade_in_samples: 0,
                fade_out_samples: (0.05 * sample_rate as f64).round() as u64,
                gain_envelope: None,
            });
        }
    }

    Ok(sfx_clips)
}

/// 组装并验证完整混音工程方案 (Compile Audio Program Plan)
pub fn compile_audio_program_plan(
    aligned_manifest: &ProjectManifest,
    voice_clips: Vec<AudioClip>,
    bgm_source: Option<PathBuf>,
    sfx_clips: Vec<AudioClip>,
    ducking_config: &DuckingConfig,
) -> Result<AudioProgramPlan> {
    let sample_rate = CANONICAL_SAMPLE_RATE;
    let total_seconds = aligned_manifest.meta.total_seconds;
    let total_samples = (total_seconds * sample_rate as f64).round() as u64;

    // 1. 如果指定了 BGM，则计算其智能闪避包络线
    let bgm_clip = if let Some(bgm_path) = bgm_source {
        if bgm_path.exists() {
            let envelope = calculate_ducking_envelope(
                &voice_clips,
                total_samples,
                ducking_config,
                sample_rate,
            );
            Some(AudioClip {
                id: "bgm_soundtrack".to_string(),
                track_type: AudioTrackType::Bgm,
                source_path: bgm_path,
                target_start_sample: 0,
                target_end_sample_exclusive: total_samples,
                target_start_seconds: 0.0,
                target_end_seconds: total_seconds,
                gain: ducking_config.normal_gain,
                fade_in_samples: (0.5 * sample_rate as f64).round() as u64,
                fade_out_samples: (1.0 * sample_rate as f64).round() as u64,
                gain_envelope: Some(envelope),
            })
        } else {
            bail!("Explicit BGM source does not exist: {:?}", bgm_path);
        }
    } else {
        None
    };

    Ok(AudioProgramPlan {
        observed_speech_activity: Vec::new(),
        sample_rate,
        channels: CANONICAL_CHANNELS,
        total_seconds,
        total_samples,
        voice_clips,
        bgm_clip,
        sfx_clips,
    })
}

/// FFmpeg aeval uses n as the local sample clock after normalization.
fn envelope_expression(points: &[AudioGainPoint], fallback: f64) -> String {
    let Some(last) = points.last() else {
        return format!("{:.9}", fallback);
    };
    let mut expr = format!("{:.9}", last.gain);
    for pair in points.windows(2).rev() {
        let (a, b) = (&pair[0], &pair[1]);
        if b.sample > a.sample {
            expr = format!(
                "if(lt(n,{}),{:.9}+({:.9})*clip((n-{})/{},0,1),{})",
                b.sample,
                a.gain,
                b.gain - a.gain,
                a.sample,
                b.sample - a.sample,
                expr
            );
        }
    }
    expr
}

/// Read every authored spoken segment; a missing recording is not a silent clip.
pub async fn prepared_voice_clips(
    manifest: &ProjectManifest,
    script: &ScriptDocument,
    directory: &Path,
) -> Result<Vec<AudioClip>> {
    let mut clips = Vec::new();
    for segment in &script.segments {
        if segment.spoken_text.trim().is_empty() {
            continue;
        }
        let id = format!("voice_{}", segment.id);
        let path = directory.join(format!("{}.wav", id));
        if !path.is_file() {
            bail!("Missing required prepared voice {:?}", path);
        }
        let window = manifest
            .tracks
            .voice
            .iter()
            .find(|v| v.id == id)
            .context("Missing aligned voice window")?;
        if window.payload["spoken_text"].as_str().is_some_and(|text| {
            crate::speech_alignment::spoken_key(text)
                != crate::speech_alignment::spoken_key(&segment.spoken_text)
        }) {
            bail!("Aligned voice identity differs from current Author: {}", id);
        }
        let measured = crate::probe::probe_media(&path).await?;
        if !measured.has_audio
            || !measured.duration_seconds.is_finite()
            || measured.duration_seconds <= 0.0
        {
            bail!("Prepared voice has no usable audio: {:?}", path);
        }
        let start = window.start_seconds;
        let end = start + measured.duration_seconds;
        if !start.is_finite()
            || start < 0.0
            || end > manifest.meta.total_seconds + 1.0 / manifest.meta.fps
        {
            bail!(
                "Prepared voice exceeds project clock; run voice realign first: {}",
                id
            );
        }
        clips.push(AudioClip {
            id,
            track_type: AudioTrackType::Voice,
            source_path: path,
            target_start_sample: (start * CANONICAL_SAMPLE_RATE as f64).round() as u64,
            target_end_sample_exclusive: (end * CANONICAL_SAMPLE_RATE as f64).round() as u64,
            target_start_seconds: start,
            target_end_seconds: end,
            gain: 1.0,
            fade_in_samples: 0,
            fade_out_samples: 0,
            gain_envelope: None,
        });
    }
    Ok(clips)
}

fn clip_filter(clip: &AudioClip, input: usize, rate: u32, label: &str) -> Result<String> {
    let length = clip
        .target_end_sample_exclusive
        .checked_sub(clip.target_start_sample)
        .filter(|n| *n > 0)
        .context("Audio clip must have a positive sample window")?;
    if !clip.gain.is_finite() || clip.gain < 0.0 {
        bail!("Invalid audio gain for {}", clip.id);
    }
    // Envelope points are absolute linear gains, consistently with the BGM track.
    let base = if clip.gain_envelope.is_some() {
        1.0
    } else {
        clip.gain
    };
    let mut chain=format!("[{}:a]aresample={},aformat=channel_layouts=stereo,atrim=end_sample={},asetpts=PTS-STARTPTS,volume={:.9}",input,rate,length,base);
    if clip.fade_in_samples > 0 {
        chain += &format!(",afade=t=in:ss=0:ns={}", clip.fade_in_samples.min(length));
    }
    if clip.fade_out_samples > 0 {
        let fade = clip.fade_out_samples.min(length);
        chain += &format!(",afade=t=out:ss={}:ns={}", length - fade, fade);
    }
    if let Some(points) = &clip.gain_envelope {
        let gain = envelope_expression(points, clip.gain);
        chain += &format!(",aeval=exprs='val(0)*({})|val(1)*({})'", gain, gain);
    }
    chain += &format!(",adelay={}S:all=1[{}]", clip.target_start_sample, label);
    Ok(chain)
}

/// 执行物理级多轨母带混音压制 (Render Master Mix)
/// FFmpeg filtergraph 与 amix 混音规范
pub async fn render_master_mix(plan: &AudioProgramPlan, out_master_wav: &Path) -> Result<()> {
    for clip in plan
        .voice_clips
        .iter()
        .chain(plan.bgm_clip.iter())
        .chain(plan.sfx_clips.iter())
    {
        if !clip.target_start_seconds.is_finite()
            || !clip.target_end_seconds.is_finite()
            || clip.target_start_seconds < 0.0
            || clip.target_end_seconds <= clip.target_start_seconds
            || clip.target_end_sample_exclusive <= clip.target_start_sample
        {
            bail!("Invalid audio window for {}", clip.id);
        }
        let start = (clip.target_start_seconds * plan.sample_rate as f64).round() as u64;
        let end = (clip.target_end_seconds * plan.sample_rate as f64).round() as u64;
        if let Some(points) = &clip.gain_envelope {
            if points.iter().any(|p| !p.gain.is_finite() || p.gain < 0.0)
                || points.windows(2).any(|p| p[1].sample <= p[0].sample)
            {
                bail!("Invalid audio gain envelope for {}", clip.id);
            }
        }
        if start.abs_diff(clip.target_start_sample) > 1
            || end.abs_diff(clip.target_end_sample_exclusive) > 1
        {
            bail!("Audio seconds/sample clocks disagree for {}", clip.id);
        }
    }
    if let Some(p) = out_master_wav.parent() {
        tokio::fs::create_dir_all(p).await?;
    }

    let mut cmd = Command::new("ffmpeg");
    cmd.arg("-y");

    // 收集全部输入文件列表与映射
    let mut input_index = 0;
    let mut filter_chains = Vec::new();
    let mut mix_inputs = Vec::new();

    // 1. 处理 Voice 轨道片段
    for (i, vc) in plan.voice_clips.iter().enumerate() {
        cmd.arg("-i").arg(&vc.source_path);
        let in_idx = input_index;
        input_index += 1;

        let chain = clip_filter(vc, in_idx, plan.sample_rate, &format!("vc{}", i))?;
        filter_chains.push(chain);
        mix_inputs.push(format!("[vc{}]", i));
    }

    // 2. 处理 BGM 轨道 (含循环与音量闪避)
    if let Some(ref bgm) = plan.bgm_clip {
        cmd.arg("-stream_loop")
            .arg("-1")
            .arg("-i")
            .arg(&bgm.source_path);
        let in_idx = input_index;
        input_index += 1;

        // Apply the actual envelope per sample, rather than averaging its points.
        let gain_expr = envelope_expression(bgm.gain_envelope.as_deref().unwrap_or(&[]), bgm.gain);
        let chain = format!(
            "[{}:a]aformat=sample_rates={}:channel_layouts=stereo,atrim=end_sample={},asetpts=PTS-STARTPTS{}{},aeval=exprs='val(0)*({})|val(1)*({})'[bgm]",
            in_idx, plan.sample_rate, plan.total_samples,
            if bgm.fade_in_samples>0{format!(",afade=t=in:ss=0:ns={}",bgm.fade_in_samples.min(plan.total_samples))}else{String::new()},
            if bgm.fade_out_samples>0{let fade=bgm.fade_out_samples.min(plan.total_samples);format!(",afade=t=out:ss={}:ns={}",plan.total_samples-fade,fade)}else{String::new()}, gain_expr, gain_expr
        );
        filter_chains.push(chain);
        mix_inputs.push("[bgm]".to_string());
    }

    // 3. 处理 SFX 轨道
    for (i, sfx) in plan.sfx_clips.iter().enumerate() {
        cmd.arg("-i").arg(&sfx.source_path);
        let in_idx = input_index;
        input_index += 1;

        let chain = clip_filter(sfx, in_idx, plan.sample_rate, &format!("sfx{}", i))?;
        filter_chains.push(chain);
        mix_inputs.push(format!("[sfx{}]", i));
    }

    if mix_inputs.is_empty() {
        bail!("Cannot mix empty audio plan: no inputs found");
    }

    // 4. 汇总为标准 amix: inputs=N:duration=longest:dropout_transition=0:normalize=0
    let amix_chain = format!(
        "{}amix=inputs={}:duration=longest:dropout_transition=0:normalize=0,apad=whole_len={},atrim=end_sample={},asetpts=PTS-STARTPTS[out]",
        mix_inputs.join(""),
        mix_inputs.len(),
        plan.total_samples,
        plan.total_samples
    );
    filter_chains.push(amix_chain);

    let full_filter = filter_chains.join(";");
    std::fs::write(
        out_master_wav.with_extension("filtergraph.txt"),
        &full_filter,
    )?;
    std::fs::write(
        out_master_wav.with_extension("plan.json"),
        serde_json::to_string_pretty(plan)?,
    )?;
    cmd.args([
        "-filter_complex",
        &full_filter,
        "-map",
        "[out]",
        "-ar",
        &plan.sample_rate.to_string(),
        "-ac",
        &plan.channels.to_string(),
        "-c:a",
        "pcm_s16le",
        out_master_wav.to_str().unwrap(),
    ]);

    let output = cmd.output().context("Failed to spawn ffmpeg audio mixer")?;
    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        bail!("FFmpeg master audio mix failed: {}", err);
    }
    let inputs = plan
        .voice_clips
        .iter()
        .chain(plan.bgm_clip.iter())
        .chain(plan.sfx_clips.iter())
        .map(|c| c.source_path.as_path());
    let mode = crate::artifacts::combined(inputs);
    let sources: Vec<PathBuf> = plan
        .voice_clips
        .iter()
        .chain(plan.bgm_clip.iter())
        .chain(plan.sfx_clips.iter())
        .map(|c| c.source_path.clone())
        .collect();
    crate::artifacts::record_derived(
        out_master_wav,
        mode,
        "sound-mix",
        &sources,
        serde_json::json!({"plan":plan,"activity_detector":{"method":"FFmpeg silencedetect","threshold_db":-40,"minimum_silence_seconds":0.12,"forced_alignment":false}}),
    )?;
    Ok(())
}

/// 更新项目蓝图工程模型，将混音成果与多轨数据写回 project_mixed.json
pub fn update_manifest_with_mixed_tracks(
    aligned_manifest: &ProjectManifest,
    plan: &AudioProgramPlan,
    master_mix_path: &Path,
) -> Result<ProjectManifest> {
    let mut mixed = aligned_manifest.clone();
    mixed.master_audio = Some(master_mix_path.canonicalize()?);
    mixed.tracks.bgm.clear();

    // 1. 注册 BGM Track 项
    if let Some(ref bgm) = plan.bgm_clip {
        mixed.tracks.bgm.clear();
        mixed.tracks.bgm.push(TrackItem {
            id: bgm.id.clone(),
            name: "Master Background Music".to_string(),
            start_seconds: bgm.target_start_seconds,
            end_seconds: bgm.target_end_seconds,
            start_frame: 0,
            end_frame: mixed.meta.total_frames,
            payload: serde_json::json!({
                "source": bgm.source_path.to_string_lossy(),
                "gain": bgm.gain,
                "ducked": bgm.gain_envelope.is_some(),
                "envelope_points": bgm.gain_envelope.as_ref().map(|v| v.len()).unwrap_or(0),
                "gain_envelope": bgm.gain_envelope,
                "automation": "sample_linear_interpolation",
            }),
        });
    }

    // 2. 注册 SFX Track 项
    mixed.tracks.sfx.clear();
    for sfx in &plan.sfx_clips {
        let start_f = (sfx.target_start_seconds * mixed.meta.fps).round() as u64;
        let end_f = (sfx.target_end_seconds * mixed.meta.fps).round() as u64;
        mixed.tracks.sfx.push(TrackItem {
            id: sfx.id.clone(),
            name: format!("SFX: {}", sfx.id),
            start_seconds: sfx.target_start_seconds,
            end_seconds: sfx.target_end_seconds,
            start_frame: start_f,
            end_frame: end_f,
            payload: serde_json::json!({
                "source": sfx.source_path.to_string_lossy(),
                "gain": sfx.gain,
            }),
        });
    }

    // 3. 将所有 SoundEffect 任务状态置为 Ready
    for task in &mut mixed.asset_tasks {
        if task.task_type == AssetTaskType::SoundEffect {
            task.status = TaskStatus::Ready;
            task.spec["master_mix"] = serde_json::json!(master_mix_path.to_string_lossy());
        }
    }

    Ok(mixed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn rendered_bgm_follows_time_varying_envelope() {
        let dir = tempdir().unwrap();
        let source = dir.path().join("tone.wav");
        synthesize_mock_sfx("bell", &source, 48000).unwrap();
        let clip = AudioClip {
            id: "bgm".into(),
            track_type: AudioTrackType::Bgm,
            source_path: source,
            target_start_sample: 0,
            target_end_sample_exclusive: 144000,
            target_start_seconds: 0.0,
            target_end_seconds: 3.0,
            gain: 0.7,
            fade_in_samples: 0,
            fade_out_samples: 0,
            gain_envelope: Some(vec![
                AudioGainPoint {
                    sample: 0,
                    gain: 0.1,
                },
                AudioGainPoint {
                    sample: 48000,
                    gain: 0.1,
                },
                AudioGainPoint {
                    sample: 48001,
                    gain: 0.7,
                },
                AudioGainPoint {
                    sample: 144000,
                    gain: 0.7,
                },
            ]),
        };
        let plan = AudioProgramPlan {
            observed_speech_activity: Vec::new(),
            sample_rate: 48000,
            channels: 2,
            total_seconds: 3.0,
            total_samples: 144000,
            voice_clips: vec![],
            bgm_clip: Some(clip),
            sfx_clips: vec![],
        };
        let out = dir.path().join("master.wav");
        render_master_mix(&plan, &out).await.unwrap();
        assert_eq!(crate::artifacts::mode(&out), crate::artifacts::Mode::Mock);
        let decoded = Command::new("ffmpeg")
            .args([
                "-v",
                "error",
                "-i",
                out.to_str().unwrap(),
                "-ac",
                "1",
                "-f",
                "f32le",
                "-",
            ])
            .output()
            .unwrap();
        assert!(decoded.status.success());
        let samples: Vec<f32> = decoded
            .stdout
            .chunks_exact(4)
            .map(|x| f32::from_le_bytes(x.try_into().unwrap()))
            .collect();
        assert_eq!(samples.len(), 144000);
        // Same source phase one loop apart, outside the master fades.
        let rms = |start: usize| {
            (samples[start..start + 2400]
                .iter()
                .map(|x| (*x as f64).powi(2))
                .sum::<f64>()
                / 2400.0)
                .sqrt()
        };
        let ratio = rms(62400) / rms(33600);
        assert!(
            (ratio - 7.0).abs() < 0.15,
            "Expected dynamic 7x gain, got {}",
            ratio
        );
    }

    #[test]
    fn measured_internal_silence_recovers_bgm_without_moving_voice() {
        let dir = tempdir().unwrap();
        let source = dir.path().join("speech-with-gap.wav");
        let made = Command::new("ffmpeg")
            .args([
                "-v",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:sample_rate=48000:duration=3",
                "-af",
                "volume=enable='between(t,1,2)':volume=0",
            ])
            .arg(&source)
            .output()
            .unwrap();
        assert!(made.status.success());
        let voice = AudioClip {
            id: "speech".into(),
            track_type: AudioTrackType::Voice,
            source_path: source,
            target_start_sample: 0,
            target_end_sample_exclusive: 144000,
            target_start_seconds: 0.0,
            target_end_seconds: 3.0,
            gain: 1.0,
            fade_in_samples: 0,
            fade_out_samples: 0,
            gain_envelope: None,
        };
        let mut bgm = voice.clone();
        bgm.track_type = AudioTrackType::Bgm;
        let mut plan = AudioProgramPlan {
            observed_speech_activity: Vec::new(),
            sample_rate: 48000,
            channels: 2,
            total_seconds: 3.0,
            total_samples: 144000,
            voice_clips: vec![voice],
            bgm_clip: Some(bgm),
            sfx_clips: vec![],
        };
        let config = DuckingConfig::default();
        apply_observed_ducking(&mut plan, &config).unwrap();
        assert_eq!(plan.observed_speech_activity.len(), 2);
        let envelope = plan.bgm_clip.unwrap().gain_envelope.unwrap();
        assert!((audio_envelope_gain_at(&envelope, 72000) - config.normal_gain).abs() < 0.001);
        assert!((audio_envelope_gain_at(&envelope, 24000) - config.ducked_gain).abs() < 0.001);
        assert_eq!(plan.voice_clips[0].target_end_sample_exclusive, 144000);
    }

    #[tokio::test]
    async fn prepared_sfx_honors_half_open_window_and_fades() {
        let dir = tempdir().unwrap();
        let source = dir.path().join("long-tone.wav");
        let made = Command::new("ffmpeg")
            .args([
                "-v",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:sample_rate=48000:duration=3",
            ])
            .arg(&source)
            .output()
            .unwrap();
        assert!(made.status.success());
        crate::artifacts::record(&source, crate::artifacts::Mode::Mock, "test-tone").unwrap();
        let clip = AudioClip {
            id: "effect".into(),
            track_type: AudioTrackType::Sfx,
            source_path: source,
            target_start_sample: 48000,
            target_end_sample_exclusive: 96000,
            target_start_seconds: 1.0,
            target_end_seconds: 2.0,
            gain: 1.0,
            fade_in_samples: 4800,
            fade_out_samples: 4800,
            gain_envelope: None,
        };
        let plan = AudioProgramPlan {
            observed_speech_activity: Vec::new(),
            sample_rate: 48000,
            channels: 2,
            total_seconds: 4.0,
            total_samples: 192000,
            voice_clips: vec![],
            bgm_clip: None,
            sfx_clips: vec![clip],
        };
        let out = dir.path().join("bounded.wav");
        render_master_mix(&plan, &out).await.unwrap();
        let decoded = Command::new("ffmpeg")
            .args(["-v", "error", "-i"])
            .arg(out)
            .args(["-ac", "1", "-f", "f32le", "-"])
            .output()
            .unwrap();
        assert!(decoded.status.success());
        let samples: Vec<f32> = decoded
            .stdout
            .chunks_exact(4)
            .map(|s| f32::from_le_bytes(s.try_into().unwrap()))
            .collect();
        assert_eq!(samples.len(), 192000);
        let rms = |at: usize| {
            (samples[at..at + 240]
                .iter()
                .map(|s| (*s as f64).powi(2))
                .sum::<f64>()
                / 240.0)
                .sqrt()
        };
        let middle = rms(72000);
        assert!(middle > 0.05);
        assert!(rms(24000) < 0.00001);
        assert!(rms(100800) < 0.00001);
        assert!(rms(48240) < middle * 0.2);
        assert!(rms(95520) < middle * 0.2);
    }
    #[test]
    fn test_audio_envelope_gain_at_interpolation() {
        let points = vec![
            AudioGainPoint {
                sample: 1000,
                gain: 0.5,
            },
            AudioGainPoint {
                sample: 2000,
                gain: 1.0,
            },
        ];

        assert_eq!(audio_envelope_gain_at(&points, 500), 0.5);
        assert_eq!(audio_envelope_gain_at(&points, 1000), 0.5);
        assert_eq!(audio_envelope_gain_at(&points, 1500), 0.75);
        assert_eq!(audio_envelope_gain_at(&points, 2000), 1.0);
        assert_eq!(audio_envelope_gain_at(&points, 2500), 1.0);
    }

    #[test]
    fn test_calculate_ducking_envelope_points() {
        let config = DuckingConfig {
            lead_margin_seconds: 0.1,
            tail_margin_seconds: 0.1,
            ramp_duration_seconds: 0.1,
            ducked_gain: 0.2,
            normal_gain: 0.8,
        };

        // 模拟一段在 1.0s ~ 3.0s 的语音 (48000 ~ 144000 采样点)
        let vc = AudioClip {
            id: "voice_01".to_string(),
            track_type: AudioTrackType::Voice,
            source_path: PathBuf::from("test.wav"),
            target_start_sample: 48000,
            target_end_sample_exclusive: 144000,
            target_start_seconds: 1.0,
            target_end_seconds: 3.0,
            gain: 1.0,
            fade_in_samples: 0,
            fade_out_samples: 0,
            gain_envelope: None,
        };

        let points = calculate_ducking_envelope(&[vc], 240000, &config, 48000);
        assert!(!points.is_empty());
        assert_eq!(points[0].sample, 0);
        assert_eq!(points[0].gain, 0.8);

        // 验证中间有声段落确实压制到了 0.2
        let mid_gain = audio_envelope_gain_at(&points, 96000);
        assert_eq!(mid_gain, 0.2);
    }

    #[test]
    fn test_synthesize_mock_sfx_creates_valid_wav() {
        let dir = tempdir().unwrap();
        let out_wav = dir.path().join("whoosh.wav");
        let dur = synthesize_mock_sfx("whoosh", &out_wav, 48000).unwrap();
        assert!(dur > 0.0);
        assert!(out_wav.exists());
    }
}

/// Measured amplitude activity, not phoneme alignment. Retains real internal breathing gaps.
pub fn apply_observed_ducking(plan: &mut AudioProgramPlan, config: &DuckingConfig) -> Result<()> {
    let mut activity = Vec::new();
    let mut windows = Vec::new();
    for clip in &plan.voice_clips {
        let result = Command::new("ffmpeg")
            .args(["-hide_banner", "-nostats", "-i"])
            .arg(&clip.source_path)
            .args(["-af", "silencedetect=noise=-40dB:d=0.12", "-f", "null", "-"])
            .output()?;
        if !result.status.success() {
            bail!(
                "Cannot measure actual speech activity for {:?}",
                clip.source_path
            );
        }
        let mut at = 0.0;
        let duration = clip.target_end_seconds - clip.target_start_seconds;
        let mut add = |start: f64, end: f64| {
            if end - start > 0.015 {
                let mut part = clip.clone();
                part.target_start_seconds = clip.target_start_seconds + start;
                part.target_end_seconds = (clip.target_start_seconds + end).min(plan.total_seconds);
                part.target_start_sample =
                    (part.target_start_seconds * plan.sample_rate as f64).round() as u64;
                part.target_end_sample_exclusive =
                    (part.target_end_seconds * plan.sample_rate as f64).round() as u64;
                activity.push((part.target_start_seconds, part.target_end_seconds));
                windows.push(part);
            }
        };
        for line in String::from_utf8_lossy(&result.stderr).lines() {
            if let Some(value) = line
                .split("silence_start: ")
                .nth(1)
                .and_then(|v| v.split_whitespace().next())
                .and_then(|v| v.parse::<f64>().ok())
            {
                add(at, value.min(duration));
                at = duration;
            }
            if let Some(value) = line
                .split("silence_end: ")
                .nth(1)
                .and_then(|v| v.split_whitespace().next())
                .and_then(|v| v.parse::<f64>().ok())
            {
                at = value.min(duration);
            }
        }
        add(at, duration);
    }
    if let Some(bgm) = &mut plan.bgm_clip {
        bgm.gain_envelope = Some(calculate_ducking_envelope(
            &windows,
            plan.total_samples,
            config,
            plan.sample_rate,
        ));
    }
    plan.observed_speech_activity = activity;
    Ok(())
}
