//! Explicit source-frame sampling after normalization to the program fps.
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
pub fn is_still(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| matches!(e, "png" | "jpg" | "jpeg" | "webp"))
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Shortage {
    #[default]
    Error,
    Hold,
    Loop,
}
fn one() -> u64 {
    1
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Sampling {
    #[serde(default)]
    pub from_frame: u64,
    #[serde(default)]
    pub until_frame: Option<u64>,
    #[serde(default = "one")]
    pub rate_numerator: u64,
    #[serde(default = "one")]
    pub rate_denominator: u64,
    #[serde(default)]
    pub shortage: Shortage,
}
impl Default for Sampling {
    fn default() -> Self {
        Self {
            from_frame: 0,
            until_frame: None,
            rate_numerator: 1,
            rate_denominator: 1,
            shortage: Shortage::Error,
        }
    }
}
impl Sampling {
    pub fn filters(
        &self,
        target: u64,
        available: Option<u64>,
        still: bool,
        fps: f64,
    ) -> Result<String> {
        if target == 0
            || !fps.is_finite()
            || fps <= 0.0
            || self.rate_numerator == 0
            || self.rate_denominator == 0
        {
            bail!("Invalid sampling clock/rate");
        }
        if still {
            if self != &Self::default() {
                bail!("Still images have no source clock; omit sampling");
            }
            return Ok(format!(
                "loop=loop=-1:size=1:start=0,trim=end_frame={},settb=1/{},setpts=N",
                target, fps
            ));
        }
        let end = self.until_frame.or(available);
        if self.until_frame.zip(available).is_some_and(|(e, n)| e > n) {
            bail!("Sampling interval exceeds actual source frames");
        }
        if end.is_some_and(|e| e <= self.from_frame) {
            bail!("Sampling needs a nonempty source interval");
        }
        let length = end.map(|e| e - self.from_frame);
        let capacity = length
            .map(|n| {
                let frames = (n as u128 * self.rate_denominator as u128)
                    .div_ceil(self.rate_numerator as u128);
                u64::try_from(frames).context("Sampling rate overflows frame count")
            })
            .transpose()?;
        if capacity.is_some_and(|n| n < target) && self.shortage == Shortage::Error {
            bail!("Source has {} sampled frames for {} target frames; choose an explicit hold/loop or repair the edit",capacity.unwrap(),target);
        }
        let trim = match end {
            Some(end) => format!("trim=start_frame={}:end_frame={}", self.from_frame, end),
            None => format!("trim=start_frame={}", self.from_frame),
        };
        let mut filter = format!("{},settb=1/{},setpts=N", trim, fps);
        if self.rate_numerator != self.rate_denominator {
            let capacity = capacity.context(
                "Rate sampling needs a resolved source frame count or explicit until_frame",
            )?;
            // Preserve the last source frame for its scaled interval, then bound the retimed domain.
            let tail = (self.rate_denominator as f64 / self.rate_numerator as f64 + 1.0) / fps;
            filter+=&format!(",settb=AVTB,setpts=PTS*{}/{},tpad=stop_mode=clone:stop_duration={:.12},fps={}:round=up,trim=end_frame={},settb=1/{},setpts=N",self.rate_denominator,self.rate_numerator,tail,fps,capacity,fps);
        }
        match self.shortage {
            Shortage::Error => {}
            Shortage::Hold => filter += ",tpad=stop_mode=clone:stop=-1",
            Shortage::Loop => {
                filter += &format!(
                    ",loop=loop=-1:size={}:start=0",
                    capacity.context("Loop sampling needs a resolved source interval")?
                )
            }
        }
        filter += &format!(",trim=end_frame={},settb=1/{},setpts=N", target, fps);
        Ok(filter)
    }
}
/// Normalize moving sources only when their measured frame domain is not already suitable.
pub async fn prepare(
    source: &std::path::Path,
    fps: f64,
    alpha: bool,
    dir: &std::path::Path,
    index: usize,
) -> Result<(std::path::PathBuf, u64)> {
    let info = crate::probe::probe_media(source).await?;
    if info.is_cfr == Some(true) && info.fps.is_some_and(|f| (f - fps).abs() < 1e-6) {
        if let Some(n) = info.frame_count {
            return Ok((source.to_owned(), n));
        }
    }
    let path = dir.join(format!(
        "normalized-{}.{}",
        index,
        if alpha { "mov" } else { "mp4" }
    ));
    let filter = format!("fps={},settb=1/{},setpts=N", fps, fps);
    let mut cmd = tokio::process::Command::new("ffmpeg");
    cmd.args(["-v", "error", "-y", "-i"])
        .arg(source)
        .args(["-an", "-vf", &filter]);
    if alpha {
        cmd.args(["-c:v", "qtrle", "-pix_fmt", "argb"]);
    } else {
        cmd.args(["-c:v", "libx264", "-crf", "0", "-pix_fmt", "yuv420p"]);
    }
    let out = cmd.arg(&path).output().await?;
    if !out.status.success() {
        bail!(
            "Frame normalization failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let n = crate::probe::probe_media(&path)
        .await?
        .frame_count
        .context("Normalized video has no measured frame count")?;
    Ok((path, n))
}

#[cfg(test)]
mod tests {
    use super::*;
    /// Two distinct source frames make accidental freezes and resets observable.
    #[tokio::test]
    async fn actual_frames_distinguish_error_hold_loop_in_out_and_rate() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.mp4");
        let status = tokio::process::Command::new("ffmpeg")
            .args([
                "-v",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "color=red:s=16x16:r=10:d=0.1",
                "-f",
                "lavfi",
                "-i",
                "color=blue:s=16x16:r=10:d=0.1",
                "-filter_complex",
                "[0:v][1:v]concat=n=2:v=1:a=0",
                "-c:v",
                "libx264",
                "-crf",
                "0",
                "-pix_fmt",
                "yuv420p",
            ])
            .arg(&source)
            .status()
            .await
            .unwrap();
        assert!(status.success());
        assert!(Sampling::default()
            .filters(4, Some(2), false, 10.0)
            .is_err());
        let cases = [
            (
                Sampling {
                    shortage: Shortage::Hold,
                    ..Default::default()
                },
                4,
                vec![true, false, false, false],
            ),
            (
                Sampling {
                    shortage: Shortage::Loop,
                    ..Default::default()
                },
                4,
                vec![true, false, true, false],
            ),
            (
                Sampling {
                    from_frame: 1,
                    until_frame: Some(2),
                    ..Default::default()
                },
                1,
                vec![false],
            ),
            (
                Sampling {
                    rate_denominator: 2,
                    ..Default::default()
                },
                4,
                vec![true, true, false, false],
            ),
            (
                Sampling {
                    rate_numerator: 2,
                    ..Default::default()
                },
                1,
                vec![true],
            ),
        ];
        for (index, (sampling, n, red)) in cases.into_iter().enumerate() {
            let output = dir.path().join(format!("{}.rgb", index));
            let filter = sampling.filters(n, Some(2), false, 10.0).unwrap();
            let out = tokio::process::Command::new("ffmpeg")
                .args(["-v", "error", "-y", "-i"])
                .arg(&source)
                .args([
                    "-vf",
                    &filter,
                    "-frames:v",
                    &n.to_string(),
                    "-f",
                    "rawvideo",
                    "-pix_fmt",
                    "rgb24",
                ])
                .arg(&output)
                .output()
                .await
                .unwrap();
            assert!(
                out.status.success(),
                "{}",
                String::from_utf8_lossy(&out.stderr)
            );
            let bytes = std::fs::read(output).unwrap();
            assert_eq!(bytes.len(), n as usize * 16 * 16 * 3, "{}", filter);
            for (frame, want_red) in red.into_iter().enumerate() {
                let pixel = &bytes[frame * 16 * 16 * 3..];
                assert_eq!(pixel[0] > pixel[2], want_red, "frame {}, {}", frame, filter);
            }
        }
    }
}
