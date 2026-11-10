use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use tokio::process::Command;

pub const EVIDENCE_SAMPLE_RATE: u32 = 16_000;

/// 将任意音视频媒体投影为 Whisper 规范的 16 kHz 单声道 16-bit PCM WAV 证据音频
/// 提取语音证据临时音频文件
pub async fn extract_speech_evidence_audio<P: AsRef<Path>>(
    source_path: P,
    output_wav: Option<PathBuf>,
) -> Result<PathBuf> {
    let source = source_path.as_ref();
    if !source.exists() {
        anyhow::bail!("Media file does not exist: {}", source.display());
    }

    let target_wav = match output_wav {
        Some(path) => path,
        None => {
            let file_stem = source.file_stem().and_then(|s| s.to_str()).unwrap_or("audio");
            std::env::temp_dir().join(format!("speech_{}_{}.wav", file_stem, std::process::id()))
        }
    };

    if let Some(parent) = target_wav.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    // 运行 FFmpeg 提取单声道 16kHz s16le PCM 音频
    let output = Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-i",
        ])
        .arg(source)
        .args([
            "-map",
            "0:a:0",
            "-vn",
            "-ac",
            "1",
            "-ar",
            &EVIDENCE_SAMPLE_RATE.to_string(),
            "-c:a",
            "pcm_s16le",
            "-bitexact",
        ])
        .arg(&target_wav)
        .output()
        .await
        .context("Failed to execute ffmpeg for speech evidence extraction")?;

    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("Failed to project audio to 16kHz PCM: {}", err);
    }

    Ok(target_wav)
}
