use anyhow::{Context, Result};
use std::path::Path;
use tokio::process::Command;

pub struct CutOptions<'a> {
    pub input_path: &'a Path,
    pub output_path: &'a Path,
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub accurate: bool,
}

/// SEEK_RUN_UP 双段跳转优化算法
/// 跳到目标点前 2 秒进行快速关键帧定位，再解码短跑道（Run-up），
/// 既避免了从 0 秒开始解码的漫长耗时，又保证了精确切片无黑帧。
const SEEK_RUN_UP: f64 = 2.0;

pub async fn cut_media(options: CutOptions<'_>) -> Result<()> {
    if !options.input_path.exists() {
        anyhow::bail!("Input media file does not exist: {}", options.input_path.display());
    }

    if options.start_seconds < 0.0 {
        anyhow::bail!("Start time cannot be negative: {}", options.start_seconds);
    }

    if options.start_seconds >= options.end_seconds {
        anyhow::bail!(
            "Start time ({:.3}s) must be strictly less than end time ({:.3}s)",
            options.start_seconds,
            options.end_seconds
        );
    }

    if let Some(parent) = options.output_path.parent() {
        if !parent.exists() {
            tokio::fs::create_dir_all(parent)
                .await
                .context("Failed to create parent directory for output file")?;
        }
    }

    let duration = options.end_seconds - options.start_seconds;
    let mut cmd = Command::new("ffmpeg");
    cmd.arg("-hide_banner").arg("-loglevel").arg("error").arg("-y");

    if options.accurate {
        // 双段跳转优化策略
        let jump = (options.start_seconds - SEEK_RUN_UP).max(0.0);
        let run_up = options.start_seconds - jump;

        if jump > 0.0 {
            cmd.args(["-ss", &format!("{:.3}", jump)]);
        }
        cmd.arg("-i").arg(options.input_path);
        if run_up > 0.0 {
            cmd.args(["-ss", &format!("{:.3}", run_up)]);
        }

        cmd.args(["-t", &format!("{:.3}", duration)])
            .args([
                "-c:v",
                "libx264",
                "-preset",
                "veryfast",
                "-crf",
                "20",
                "-c:a",
                "aac",
                "-b:a",
                "128k",
                "-movflags",
                "+faststart",
            ])
            .arg(options.output_path);
    } else {
        // 极速无损流拷贝模式 (Stream Copy)
        cmd.args(["-ss", &format!("{:.3}", options.start_seconds), "-to", &format!("{:.3}", options.end_seconds)])
            .arg("-i")
            .arg(options.input_path)
            .args(["-c", "copy"])
            .arg(options.output_path);
    }

    let output = cmd
        .output()
        .await
        .context("Failed to execute ffmpeg for media cutting")?;

    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("ffmpeg cut failed with error: {}", err);
    }

    Ok(())
}
