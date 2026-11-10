//! Measured delivery facts and explicit, content-bound creative review evidence.
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Loudness {
    pub integrated_lufs: f64,
    pub true_peak_dbfs: f64,
    pub range_lu: f64,
}
pub async fn loudness(path: &Path) -> Result<Loudness> {
    let output = tokio::process::Command::new("ffmpeg")
        .args(["-hide_banner", "-nostats", "-i"])
        .arg(path)
        .args(["-vn", "-af", "ebur128=peak=true", "-f", "null", "-"])
        .output()
        .await?;
    if !output.status.success() {
        bail!(
            "Loudness measurement failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let text = String::from_utf8_lossy(&output.stderr);
    let summary = text
        .rsplit("Summary:")
        .next()
        .context("Missing R128 summary")?;
    let number = |prefix: &str| -> Result<f64> {
        let token = summary
            .lines()
            .find_map(|line| line.trim().strip_prefix(prefix))
            .context("Missing R128 value")?
            .split_whitespace()
            .next()
            .context("Missing numeric R128 value")?;
        Ok(token.parse()?)
    };
    Ok(Loudness {
        integrated_lufs: number("I:")?,
        true_peak_dbfs: number("Peak:")?,
        range_lu: number("LRA:")?,
    })
}

/// An asserted creative judgment, not an automatic image classifier or signature.
#[derive(Debug, Deserialize)]
pub struct CreativeEvidence {
    pub artifact_digest: String,
    pub reviewer: String,
    pub checks: Vec<CreativeCheck>,
    #[serde(default)]
    pub attachments: Vec<Attachment>,
}
#[derive(Debug, Deserialize)]
pub struct CreativeCheck {
    pub name: String,
    pub status: String,
    pub note: String,
}
#[derive(Debug, Deserialize)]
pub struct Attachment {
    pub path: PathBuf,
    pub digest: String,
}
pub fn creative_evidence(video: &Path, evidence: &Path) -> Result<Value> {
    let mut document: CreativeEvidence = serde_json::from_slice(&std::fs::read(evidence)?)?;
    if document.artifact_digest != crate::artifacts::content_digest(video)? {
        bail!("Creative evidence belongs to different video bytes; re-review the current output");
    }
    if document.reviewer.trim().is_empty() {
        bail!("Creative evidence needs a named reviewer");
    }
    for attachment in &mut document.attachments {
        if attachment.path.is_relative() {
            attachment.path = evidence
                .parent()
                .unwrap_or(Path::new("."))
                .join(&attachment.path);
        }
        if crate::artifacts::content_digest(&attachment.path)? != attachment.digest {
            bail!("Review attachment changed: {:?}", attachment.path);
        }
    }
    let required = [
        "product_identity",
        "speech_and_captions",
        "visual_continuity",
        "watermark_acceptance",
    ];
    let mut identities = std::collections::HashSet::new();
    for check in &document.checks {
        if !identities.insert(&check.name) {
            bail!("Duplicate creative review check: {}", check.name);
        }
        if !matches!(check.status.as_str(), "pass" | "fail" | "unverified")
            || check.note.trim().is_empty()
        {
            bail!("Invalid creative review status/note for {}", check.name);
        }
    }
    let complete = required.iter().all(|name| {
        document
            .checks
            .iter()
            .any(|c| c.name == *name && c.status == "pass")
    });
    let rejected = document.checks.iter().any(|c| c.status == "fail");
    Ok(
        json!({"reviewer":document.reviewer,"complete":complete&&!rejected,"rejected":rejected,"checks":document.checks.iter().map(|c|json!({"name":c.name,"status":c.status,"note":c.note})).collect::<Vec<_>>() }),
    )
}

pub async fn inspect_boundaries(
    project: &crate::project::ProjectManifest,
    video: &Path,
    output: &Path,
) -> Result<Value> {
    std::fs::create_dir_all(output)?;
    let meta = crate::probe::probe_media(video).await?;
    let count = meta
        .frame_count
        .context("Encoded video has no frame count; normalize before inspecting")?;
    if count == 0 {
        bail!("Empty encoded video");
    }
    let mut cuts = Vec::new();
    for shot in project.shots.iter().skip(1) {
        let boundary = shot.start_frame;
        let frames: Vec<u64> =
            (boundary.saturating_sub(3)..=(boundary + 3).min(count - 1)).collect();
        if frames.is_empty() {
            continue;
        }
        let selection = frames
            .iter()
            .map(|n| format!("eq(n,{})", n))
            .collect::<Vec<_>>()
            .join("+");
        let image = output.join(format!("boundary-{}.png", boundary));
        let filter = format!(
            "select='{}',scale=320:180,tile={}x1",
            selection,
            frames.len()
        );
        let result = tokio::process::Command::new("ffmpeg")
            .args(["-v", "error", "-y", "-i"])
            .arg(video)
            .args(["-vf", &filter, "-frames:v", "1"])
            .arg(&image)
            .output()
            .await?;
        if !result.status.success() {
            bail!(
                "Boundary extraction failed: {}",
                String::from_utf8_lossy(&result.stderr)
            );
        }
        cuts.push(json!({"next_shot":shot.id,"planned_frame":boundary,"sampled_encoded_frames":frames,"sheet":image,"digest":crate::artifacts::content_digest(&image)?}));
    }
    let result = json!({"format":"clone-any-video.boundaries@1","artifact_digest":crate::artifacts::content_digest(video)?,"planned_frames":project.meta.total_frames,"encoded_frames":count,"frame_clock_matches":count==project.meta.total_frames,"semantic_continuity":"unverified","boundaries":cuts});
    std::fs::write(
        output.join("boundaries.json"),
        serde_json::to_vec_pretty(&result)?,
    )?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn caption_windows_match_player_predicate_at_fractional_boundaries() {
        for (start, end) in [
            (0.0, 56.0 / 30.0),
            (163.0 / 30.0, 234.0 / 30.0),
            (428.0 / 30.0, 448.0 / 30.0),
            (1.867, 2.234),
        ] {
            let first = caption_frame(start, 30.0, 464);
            let last = caption_frame(end, 30.0, 464);
            for frame in 0..464 {
                let time = frame as f64 / 30.0;
                assert_eq!(frame >= first && frame < last, time >= start && time < end);
            }
        }
    }
    #[test]
    fn creative_judgment_binds_video_and_review_attachments() {
        let dir = tempfile::tempdir().unwrap();
        let video = dir.path().join("video.mp4");
        let sheet = dir.path().join("sheet.png");
        let evidence = dir.path().join("review.json");
        std::fs::write(&video, b"accepted-video-bytes").unwrap();
        std::fs::write(&sheet, b"reviewed-sheet").unwrap();
        let mut doc = json!({"artifact_digest":crate::artifacts::content_digest(&video).unwrap(),"reviewer":"Test reviewer","checks":(["product_identity","speech_and_captions","visual_continuity","watermark_acceptance"].map(|name|json!({"name":name,"status":"pass","note":"Explicit test judgment"}))),"attachments":[{"path":"sheet.png","digest":crate::artifacts::content_digest(&sheet).unwrap()}]});
        std::fs::write(&evidence, serde_json::to_vec(&doc).unwrap()).unwrap();
        assert_eq!(
            creative_evidence(&video, &evidence).unwrap()["complete"],
            true
        );
        doc["checks"][3]["status"] = json!("unverified");
        std::fs::write(&evidence, serde_json::to_vec(&doc).unwrap()).unwrap();
        assert_eq!(
            creative_evidence(&video, &evidence).unwrap()["complete"],
            false
        );
        doc["checks"].as_array_mut().unwrap().push(
            json!({"name":"watermark_acceptance","status":"pass","note":"Conflicting duplicate"}),
        );
        std::fs::write(&evidence, serde_json::to_vec(&doc).unwrap()).unwrap();
        assert!(creative_evidence(&video, &evidence)
            .unwrap_err()
            .to_string()
            .contains("Duplicate"));
        doc["checks"].as_array_mut().unwrap().pop();
        std::fs::write(&evidence, serde_json::to_vec(&doc).unwrap()).unwrap();
        std::fs::write(&sheet, b"changed-sheet").unwrap();
        assert!(creative_evidence(&video, &evidence).is_err());
        std::fs::write(&video, b"new-render").unwrap();
        assert!(creative_evidence(&video, &evidence).is_err());
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CaptionFact {
    pub id: String,
    pub text: String,
    pub start_frame: u64,
    pub end_frame: u64,
}
pub fn caption_fact(item: &crate::project::TrackItem) -> CaptionFact {
    CaptionFact {
        id: item.id.clone(),
        text: item.payload["text"]
            .as_str()
            .unwrap_or(&item.name)
            .to_owned(),
        start_frame: item.start_frame,
        end_frame: item.end_frame,
    }
}
pub fn captions_covered(items: &[crate::project::TrackItem], details: &Value) -> bool {
    let rendered: Vec<CaptionFact> =
        serde_json::from_value(details["caption_windows"].clone()).unwrap_or_default();
    !items.is_empty()
        && items
            .iter()
            .all(|item| rendered.contains(&caption_fact(item)))
}

// Match the exact frame/fps predicate used by the HTML player, including f64
// boundaries where ceil(time*fps) alone can add a spurious frame.
pub fn caption_frame(time: f64, fps: f64, total: u64) -> u64 {
    let mut frame = (time * fps).ceil().min(total as f64) as u64;
    while frame > 0 && (frame - 1) as f64 / fps >= time {
        frame -= 1;
    }
    while frame < total && (frame as f64 / fps) < time {
        frame += 1;
    }
    frame
}
