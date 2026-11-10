//! Adopt one visible performance without replacing its own sound.
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Relation {
    pub id: String,
    pub source_digest: String,
    pub video_digest: String,
    pub audio_digest: String,
    pub fps: f64,
    pub frames: u64,
    pub samples: u64,
    pub audio_offset_samples: i64,
}
pub async fn adopt(source: &Path, directory: &Path, fps: f64) -> Result<BTreeMap<String, PathBuf>> {
    if !fps.is_finite() || fps <= 0.0 {
        bail!("Invalid performance fps");
    }
    let out = tokio::process::Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "stream=codec_type,start_time",
            "-of",
            "json",
        ])
        .arg(source)
        .output()
        .await?;
    if !out.status.success() {
        bail!("Cannot inspect adopted performance");
    }
    let v: Value = serde_json::from_slice(&out.stdout)?;
    let streams = v["streams"].as_array().context("No performance streams")?;
    let start = |kind: &str| -> Result<f64> {
        let s = streams
            .iter()
            .find(|s| s["codec_type"] == kind)
            .with_context(|| {
                format!(
                    "Adopted performance has no {} stream; do not substitute independent TTS",
                    kind
                )
            })?;
        let t = s["start_time"]
            .as_str()
            .context("Unknown performance stream origin")?
            .parse::<f64>()?;
        if !t.is_finite() {
            bail!("Unknown stream origin");
        }
        Ok(t)
    };
    let delta = ((start("audio")? - start("video")?) * 48000.0).round() as i64;
    let video = directory.join("video.mp4");
    let audio = directory.join("audio.wav");
    let document = directory.join("performance.json");
    for path in [&video, &audio, &document] {
        if path.exists() && path.canonicalize()? == source.canonicalize()? {
            bail!("Performance output would overwrite its source");
        }
    }
    std::fs::create_dir_all(directory)?;
    let filter = format!("fps={},settb=1/{},setpts=N", fps, fps);
    let out = tokio::process::Command::new("ffmpeg")
        .args(["-v", "error", "-y", "-i"])
        .arg(source)
        .args([
            "-map", "0:v:0", "-an", "-vf", &filter, "-c:v", "libx264", "-crf", "0", "-pix_fmt",
            "yuv420p",
        ])
        .arg(&video)
        .output()
        .await?;
    if !out.status.success() {
        bail!(
            "Performance picture normalization failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let media = crate::probe::probe_media(&video).await?;
    let frames = media
        .frame_count
        .context("Missing normalized performance frame count")?;
    if frames == 0 {
        bail!("Empty normalized performance");
    }
    let samples = (frames as f64 / fps * 48000.0).round() as u64;
    let mut filter =
        "aresample=48000,aformat=channel_layouts=stereo,asetpts=PTS-STARTPTS".to_owned();
    if delta > 0 {
        filter += &format!(",adelay=delays={}S:all=1", delta);
    } else if delta < 0 {
        filter += &format!(
            ",atrim=start_sample={},asetpts=PTS-STARTPTS",
            delta.unsigned_abs()
        );
    }
    filter += &format!(
        ",apad=whole_len={},atrim=end_sample={},asetpts=PTS-STARTPTS",
        samples, samples
    );
    let out = tokio::process::Command::new("ffmpeg")
        .args(["-v", "error", "-y", "-i"])
        .arg(source)
        .args(["-map", "0:a:0", "-vn", "-af", &filter, "-c:a", "pcm_s16le"])
        .arg(&audio)
        .output()
        .await?;
    if !out.status.success() {
        bail!(
            "Performance audio preparation failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let mut relation = Relation {
        id: String::new(),
        source_digest: crate::artifacts::content_digest(source)?,
        video_digest: crate::artifacts::content_digest(&video)?,
        audio_digest: crate::artifacts::content_digest(&audio)?,
        fps,
        frames,
        samples,
        audio_offset_samples: delta,
    };
    relation.id = crate::artifacts::digest(&serde_json::to_vec(&relation)?);
    let mode = crate::artifacts::mode(source);
    for path in [&video, &audio] {
        crate::artifacts::record_derived(
            path,
            mode,
            "adopted-native-performance",
            &[source.to_owned()],
            json!({"performance":relation}),
        )?;
    }
    std::fs::write(
        &document,
        serde_json::to_vec_pretty(
            &json!({"format":"clone-any-video.performance@1","relation":relation,"source":source,"video":video,"audio":audio,"mode":mode}),
        )?,
    )?;
    Ok(BTreeMap::from([
        ("video".into(), video),
        ("audio".into(), audio),
        ("document".into(), document),
    ]))
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn adoption_preserves_measured_stream_offset_and_rejects_silent_video() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("offset.mkv");
        let status = tokio::process::Command::new("ffmpeg")
            .args([
                "-v",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "color=red:s=32x32:r=30:d=1",
                "-itsoffset",
                "0.25",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:sample_rate=48000:duration=0.75",
                "-map",
                "0:v",
                "-map",
                "1:a",
                "-c:v",
                "libx264",
                "-c:a",
                "pcm_s16le",
            ])
            .arg(&source)
            .status()
            .await
            .unwrap();
        assert!(status.success());
        crate::artifacts::record(
            &source,
            crate::artifacts::Mode::Mock,
            "synthetic stream-offset fixture",
        )
        .unwrap();
        let outputs = super::adopt(&source, &dir.path().join("adopted"), 30.0)
            .await
            .unwrap();
        let details = crate::artifacts::details(&outputs["audio"]).unwrap();
        assert_eq!(details["performance"]["audio_offset_samples"], 12000);
        assert_eq!(details["performance"]["frames"], 30);
        assert_eq!(details["performance"]["samples"], 48000);
        let out = tokio::process::Command::new("ffmpeg")
            .args(["-v", "error", "-i"])
            .arg(&outputs["audio"])
            .args(["-f", "f32le", "-ac", "1", "pipe:1"])
            .output()
            .await
            .unwrap();
        assert!(out.status.success());
        let samples: Vec<_> = out
            .stdout
            .chunks_exact(4)
            .map(|s| f32::from_le_bytes(s.try_into().unwrap()))
            .collect();
        assert_eq!(samples.len(), 48000);
        assert!(samples[..12000].iter().all(|s| s.abs() < 0.0001));
        assert!(samples[12000..24000].iter().any(|s| s.abs() > 0.01));
        let silent = dir.path().join("silent.mp4");
        let status = tokio::process::Command::new("ffmpeg")
            .args(["-v", "error", "-y", "-i"])
            .arg(&source)
            .args(["-an", "-c:v", "copy"])
            .arg(&silent)
            .status()
            .await
            .unwrap();
        assert!(status.success());
        let error = super::adopt(&silent, &dir.path().join("invalid"), 30.0)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("no audio stream"));
        assert!(!dir.path().join("invalid/video.mp4").exists());
    }
}
