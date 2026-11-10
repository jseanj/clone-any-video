use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;
use tokio::process::Command;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaInfo {
    pub path: String,
    pub duration_seconds: f64,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub fps: Option<f64>,
    pub is_cfr: Option<bool>,
    #[serde(default)]
    pub frame_count: Option<u64>,
    pub video_codec: Option<String>,
    pub has_audio: bool,
    pub audio_codec: Option<String>,
    pub audio_channels: Option<u32>,
    pub audio_sample_rate: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct FfprobeOutput {
    streams: Vec<FfprobeStream>,
    format: Option<FfprobeFormat>,
}

#[derive(Debug, Deserialize)]
struct FfprobeFormat {
    duration: Option<String>,
}

#[derive(Debug, Deserialize)]
struct FfprobeStream {
    codec_type: Option<String>,
    codec_name: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    r_frame_rate: Option<String>,
    avg_frame_rate: Option<String>,
    nb_frames: Option<String>,
    channels: Option<u32>,
    sample_rate: Option<String>,
    duration: Option<String>,
}

fn parse_frame_rate(rate: &str) -> Option<f64> {
    let parts: Vec<&str> = rate.split('/').collect();
    if parts.len() == 2 {
        let num: f64 = parts[0].parse().ok()?;
        let den: f64 = parts[1].parse().ok()?;
        if den > 0.0 {
            return Some(num / den);
        }
    } else if let Ok(val) = rate.parse::<f64>() {
        return Some(val);
    }
    None
}

/// 使用 ffprobe 分析音视频文件的元数据
pub async fn probe_media<P: AsRef<Path>>(path: P) -> Result<MediaInfo> {
    let path_ref = path.as_ref();
    if !path_ref.exists() {
        anyhow::bail!("Media file does not exist: {}", path_ref.display());
    }

    let output = Command::new("ffprobe")
        .args([
            "-v",
            "quiet",
            "-print_format",
            "json",
            "-show_format",
            "-show_streams",
        ])
        .arg(path_ref)
        .output()
        .await
        .context("Failed to execute ffprobe. Is ffmpeg/ffprobe installed on your system?")?;

    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("ffprobe failed with error: {}", err);
    }

    let parsed: FfprobeOutput =
        serde_json::from_slice(&output.stdout).context("Failed to parse ffprobe JSON output")?;

    let mut width = None;
    let mut height = None;
    let mut fps = None;
    let mut video_codec = None;
    let mut has_audio = false;
    let mut audio_codec = None;
    let mut audio_channels = None;
    let mut audio_sample_rate = None;
    let mut stream_duration = 0.0f64;
    let mut is_cfr = None;
    let mut frame_count = None;

    for stream in parsed.streams {
        match stream.codec_type.as_deref() {
            Some("video") => {
                frame_count = stream.nb_frames.as_deref().and_then(|n| n.parse().ok());
                width = stream.width;
                height = stream.height;
                video_codec = stream.codec_name;
                is_cfr = match (&stream.r_frame_rate, &stream.avg_frame_rate) {
                    (Some(r), Some(avg)) => Some(r == avg && parse_frame_rate(r).is_some()),
                    _ => None,
                };
                if let Some(rate_str) = stream.avg_frame_rate.or(stream.r_frame_rate) {
                    fps = parse_frame_rate(&rate_str);
                }
                if let Some(dur_str) = stream.duration {
                    if let Ok(d) = dur_str.parse::<f64>() {
                        if d > stream_duration {
                            stream_duration = d;
                        }
                    }
                }
            }
            Some("audio") => {
                has_audio = true;
                audio_codec = stream.codec_name;
                audio_channels = stream.channels;
                audio_sample_rate = stream.sample_rate.and_then(|s| s.parse().ok());
                if let Some(dur_str) = stream.duration {
                    if let Ok(d) = dur_str.parse::<f64>() {
                        if d > stream_duration {
                            stream_duration = d;
                        }
                    }
                }
            }
            _ => {}
        }
    }

    let format_duration = parsed
        .format
        .and_then(|f| f.duration)
        .and_then(|d| d.parse::<f64>().ok())
        .unwrap_or(stream_duration);

    Ok(MediaInfo {
        path: path_ref.to_string_lossy().to_string(),
        duration_seconds: format_duration,
        width,
        height,
        fps,
        is_cfr,
        frame_count,
        video_codec,
        has_audio,
        audio_codec,
        audio_channels,
        audio_sample_rate,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_frame_rate() {
        assert_eq!(parse_frame_rate("30/1"), Some(30.0));
        assert_eq!(parse_frame_rate("25/1"), Some(25.0));
        assert_eq!(parse_frame_rate("invalid"), None);
    }

    #[tokio::test]
    async fn test_probe_sample_fixture() {
        let fixture = if Path::new("fixtures/sample.mp4").exists() { Path::new("fixtures/sample.mp4") } else { Path::new("../../fixtures/sample.mp4") };
        if fixture.exists() {
            let info = probe_media(fixture)
                .await
                .expect("probe sample.mp4 should succeed");
            assert!(info.duration_seconds > 0.0);
            assert_eq!(info.width, Some(640));
            assert_eq!(info.height, Some(360));
        }
    }
}
