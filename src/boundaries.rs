use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;
use tokio::process::Command;

/// 默认采样帧率：每秒采样 12 帧
pub const DEFAULT_BOUNDARY_RATE: f64 = 12.0;

/// 默认视觉突变阈值：0.1 (10% 平均色彩空间变化)
pub const DEFAULT_BOUNDARY_THRESHOLD: f64 = 0.1;

/// 缩略图基准尺寸：32x32，消除高频噪点与微观抖动，聚焦宏观构图与镜头切换
pub const THUMBNAIL_SIZE: usize = 32;
pub const STRIDE: usize = THUMBNAIL_SIZE * THUMBNAIL_SIZE * 3; // 3072 bytes (RGB24)

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct VisualBoundary {
    /// 候选时间点（秒）
    pub at: f64,
    /// 画面差异变化得分 (0.0 ~ 1.0)
    pub score: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoundariesResult {
    pub path: String,
    #[serde(rename = "sampleRate")]
    pub sample_rate: f64,
    pub threshold: f64,
    pub candidates: Vec<VisualBoundary>,
}

/// 执行机械性相邻帧视觉变化检测 (Mechanical Visual Change Detection)
/// 视觉突变检测算法：计算相邻采样帧间的平均绝对差值
pub async fn detect_visual_boundaries(
    source: &Path,
    sample_rate: Option<f64>,
    threshold: Option<f64>,
) -> Result<BoundariesResult> {
    let rate = sample_rate.unwrap_or(DEFAULT_BOUNDARY_RATE).clamp(1.0, 120.0);
    let thresh = threshold.unwrap_or(DEFAULT_BOUNDARY_THRESHOLD).clamp(0.0, 1.0);

    let vf_filter = format!("fps={},scale={}:{},format=rgb24", rate, THUMBNAIL_SIZE, THUMBNAIL_SIZE);

    let output = Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-i",
        ])
        .arg(source)
        .args([
            "-vf",
            &vf_filter,
            "-f",
            "rawvideo",
            "-",
        ])
        .output()
        .await
        .with_context(|| format!("Failed to execute ffmpeg for boundaries on {:?}", source))?;

    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("FFmpeg boundaries filter failed: {}", err);
    }

    let raw_bytes = output.stdout;
    let total_frames = raw_bytes.len() / STRIDE;

    let mut candidates = Vec::new();

    if total_frames > 1 {
        for index in 1..total_frames {
            let before_start = (index - 1) * STRIDE;
            let before = &raw_bytes[before_start..before_start + STRIDE];

            let after_start = index * STRIDE;
            let after = &raw_bytes[after_start..after_start + STRIDE];

            // 计算 L1 绝对差总和 (Manhattan Distance)
            let mut diff_sum: u64 = 0;
            for i in 0..STRIDE {
                diff_sum += (after[i] as i32 - before[i] as i32).abs() as u64;
            }

            // 归一化得分: total / 3072 / 255
            let score = (diff_sum as f64) / (STRIDE as f64) / 255.0;
            let rounded_score = (score * 1000.0).round() / 1000.0;

            if score >= thresh {
                let at_seconds = ((index as f64) / rate * 1000.0).round() / 1000.0;
                candidates.push(VisualBoundary {
                    at: at_seconds,
                    score: rounded_score,
                });
            }
        }
    }

    Ok(BoundariesResult {
        path: source.to_string_lossy().to_string(),
        sample_rate: rate,
        threshold: thresh,
        candidates,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stride_calculation() {
        assert_eq!(STRIDE, 32 * 32 * 3);
        assert_eq!(STRIDE, 3072);
    }

    #[test]
    fn test_boundary_scoring() {
        let black_frame = vec![0u8; STRIDE];
        let white_frame = vec![255u8; STRIDE];

        let mut diff_sum: u64 = 0;
        for i in 0..STRIDE {
            diff_sum += (white_frame[i] as i32 - black_frame[i] as i32).abs() as u64;
        }

        let score = (diff_sum as f64) / (STRIDE as f64) / 255.0;
        assert!((score - 1.0).abs() < 1e-6);
    }

    #[tokio::test]
    async fn test_detect_visual_boundaries_on_fixture() {
        let fixture_path = if Path::new("fixtures/cuts_sample.mp4").exists() { Path::new("fixtures/cuts_sample.mp4") } else { Path::new("../../fixtures/cuts_sample.mp4") };
        if fixture_path.exists() {
            let res = detect_visual_boundaries(fixture_path, Some(12.0), Some(0.1))
                .await
                .expect("boundaries detection should succeed");
            assert_eq!(res.candidates.len(), 2);
            assert!((res.candidates[0].at - 2.0).abs() <= 0.1);
            assert!((res.candidates[1].at - 4.0).abs() <= 0.1);
        }
    }
}
