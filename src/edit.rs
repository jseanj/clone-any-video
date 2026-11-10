//! Explicit frame-domain source sampling and authored continuity intentions.
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};
#[derive(Debug, Deserialize, Serialize)]
pub struct EditPlan {
    pub fps: f64,
    pub clips: Vec<Clip>,
}
#[derive(Debug, Deserialize, Serialize)]
pub struct Clip {
    pub shot_id: String,
    pub source: PathBuf,
    pub source_start_frame: u64,
    pub source_end_frame: u64,
    pub target_start_frame: u64,
    pub target_end_frame: u64,
    #[serde(default = "one")]
    pub zoom: f64,
    #[serde(default)]
    pub continuity_group: Option<String>,
    #[serde(default)]
    pub cut_intent: Option<String>,
}
fn one() -> f64 {
    1.0
}
pub fn validate(plan: &EditPlan) -> Result<()> {
    if !plan.fps.is_finite() || plan.fps <= 0.0 || plan.clips.is_empty() {
        bail!("Invalid edit frame clock");
    }
    let mut previous: Option<&Clip> = None;
    let mut identities = std::collections::HashSet::new();
    for clip in &plan.clips {
        if clip.shot_id.is_empty()
            || clip.shot_id == "."
            || clip.shot_id == ".."
            || clip.shot_id.contains(['/', '\\'])
            || !identities.insert(&clip.shot_id)
        {
            bail!("Shot IDs must be unique simple file names");
        }
        if clip.source_end_frame <= clip.source_start_frame
            || clip.target_end_frame <= clip.target_start_frame
            || clip.source_end_frame - clip.source_start_frame
                != clip.target_end_frame - clip.target_start_frame
        {
            bail!("Clip '{}' needs equal source/target frame extents for rate=1; prepare retiming explicitly",clip.shot_id);
        }
        if !clip.zoom.is_finite() || !(1.0..=3.0).contains(&clip.zoom) {
            bail!("Invalid explicit zoom");
        }
        if let Some(prev) = previous {
            if clip.target_start_frame != prev.target_end_frame {
                bail!("Edit ranges must be contiguous and non-overlapping");
            }
            if clip.continuity_group.is_some()
                && clip.continuity_group == prev.continuity_group
                && clip.cut_intent.as_deref() != Some("intentional-jump")
            {
                if clip.source != prev.source
                    || clip.source_start_frame != prev.source_end_frame
                    || (clip.zoom - prev.zoom).abs() > 0.000001
                {
                    bail!("Unintended source-time or scale discontinuity at '{}'; keep continuity or explicitly author cut_intent=intentional-jump",clip.shot_id);
                }
            }
        } else if clip.target_start_frame != 0 {
            bail!("Edit must begin at frame zero");
        }
        previous = Some(clip);
    }
    Ok(())
}
pub async fn render(
    plan_path: &Path,
    project_path: &Path,
    output: &Path,
    updated: &Path,
) -> Result<()> {
    let mut plan: EditPlan = serde_json::from_slice(&std::fs::read(plan_path)?)?;
    for clip in &mut plan.clips {
        if clip.source.is_relative() {
            clip.source = plan_path
                .parent()
                .unwrap_or(Path::new("."))
                .join(&clip.source);
        }
        clip.source = clip.source.canonicalize()?;
    }
    validate(&plan)?;
    let mut project: crate::project::ProjectManifest =
        serde_json::from_slice(&std::fs::read(project_path)?)?;
    if (plan.fps - project.meta.fps).abs() > 0.000001
        || plan.clips.last().unwrap().target_end_frame != project.meta.total_frames
    {
        bail!("Edit target clock differs from prepared project");
    }
    for clip in &plan.clips {
        let target = output.join(format!("{}.mp4", clip.shot_id));
        if target.exists() {
            let target = target.canonicalize()?;
            if plan.clips.iter().any(|input| input.source == target) {
                bail!(
                    "Edit output would overwrite an accepted source: {:?}",
                    target
                );
            }
        }
    }
    std::fs::create_dir_all(output)?;
    let normalized = output.join(".normalized");
    std::fs::create_dir_all(&normalized)?;
    let mut sources: HashMap<PathBuf, PathBuf> = HashMap::new();
    let mut shots = Vec::new();
    for (i, clip) in plan.clips.iter().enumerate() {
        let source = if let Some(path) = sources.get(&clip.source) {
            path.clone()
        } else {
            let digest = crate::artifacts::content_digest(&clip.source)?;
            let path = normalized.join(format!(
                "{}-{}-{}x{}.mp4",
                digest, plan.fps, project.meta.width, project.meta.height
            ));
            // Content-addressed normalization is prepared once per source for this edit.
            let filter=format!("fps={},scale={}:{}:force_original_aspect_ratio=decrease:force_divisible_by=2:out_color_matrix=bt709:out_range=tv,pad={}:{}:(ow-iw)/2:(oh-ih)/2,setsar=1,format=yuv420p,setparams=color_primaries=bt709:color_trc=bt709:colorspace=bt709:range=tv",plan.fps,project.meta.width,project.meta.height,project.meta.width,project.meta.height);
            let result = tokio::process::Command::new("ffmpeg")
                .args(["-v", "error", "-y", "-i"])
                .arg(&clip.source)
                .args(["-an", "-vf", &filter, "-c:v", "libx264", "-crf", "0"])
                .arg(&path)
                .output()
                .await?;
            if !result.status.success() {
                bail!(
                    "Source normalization failed: {}",
                    String::from_utf8_lossy(&result.stderr)
                );
            }
            crate::artifacts::record_derived(
                &path,
                crate::artifacts::mode(&clip.source),
                "frame-normalization",
                &[clip.source.clone()],
                json!({"fps":plan.fps,"width":project.meta.width,"height":project.meta.height}),
            )?;
            sources.insert(clip.source.clone(), path.clone());
            path
        };
        let count = crate::probe::probe_media(&source)
            .await?
            .frame_count
            .context("Normalized source lacks frame count")?;
        if clip.source_end_frame > count {
            bail!(
                "Source range exceeds actual normalized frames for {}",
                clip.shot_id
            );
        }
        let target = output.join(format!("{}.mp4", clip.shot_id));
        let mut filter = format!(
            "trim=start_frame={}:end_frame={},setpts=PTS-STARTPTS",
            clip.source_start_frame, clip.source_end_frame
        );
        if clip.zoom != 1.0 {
            let w = (project.meta.width as f64 / clip.zoom / 2.0).floor() as u32 * 2;
            let h = (project.meta.height as f64 / clip.zoom / 2.0).floor() as u32 * 2;
            filter += &format!(
                ",crop={}:{},scale={}:{}",
                w, h, project.meta.width, project.meta.height
            );
        }
        let result = tokio::process::Command::new("ffmpeg")
            .args(["-v", "error", "-y", "-i"])
            .arg(&source)
            .args([
                "-an",
                "-vf",
                &filter,
                "-frames:v",
                &(clip.target_end_frame - clip.target_start_frame).to_string(),
                "-c:v",
                "libx264",
                "-crf",
                "0",
            ])
            .arg(&target)
            .output()
            .await?;
        if !result.status.success() {
            bail!(
                "Frame edit failed: {}",
                String::from_utf8_lossy(&result.stderr)
            );
        }
        let actual = crate::probe::probe_media(&target).await?;
        if actual.frame_count != Some(clip.target_end_frame - clip.target_start_frame) {
            bail!("Edited output has wrong frame count");
        }
        crate::artifacts::record_derived(
            &target,
            crate::artifacts::mode(&source),
            "frame-edit",
            &[source, plan_path.to_path_buf()],
            serde_json::to_value(clip)?,
        )?;
        let mut shot = project
            .shots
            .iter()
            .find(|s| s.id == clip.shot_id)
            .cloned()
            .or_else(|| project.shots.first().cloned())
            .context("Prepared project has no shot template")?;
        shot.id = clip.shot_id.clone();
        shot.shot_index = i + 1;
        shot.start_frame = clip.target_start_frame;
        shot.end_frame = clip.target_end_frame;
        shot.start_seconds = shot.start_frame as f64 / plan.fps;
        shot.end_seconds = shot.end_frame as f64 / plan.fps;
        shot.duration_seconds = shot.end_seconds - shot.start_seconds;
        shots.push(shot);
    }
    project.shots = shots;
    project.tracks.visual_bg.clear();
    for clip in &plan.clips {
        project.tracks.visual_bg.push(crate::project::TrackItem{id:format!("bg_{}",clip.shot_id),name:clip.shot_id.clone(),start_seconds:clip.target_start_frame as f64/plan.fps,end_seconds:clip.target_end_frame as f64/plan.fps,start_frame:clip.target_start_frame,end_frame:clip.target_end_frame,payload:json!({"source":output.join(format!("{}.mp4",clip.shot_id)).canonicalize()?,"source_time":clip})});
    }
    if let Some(parent) = updated.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(updated, serde_json::to_vec_pretty(&project)?)?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn continuity_is_an_authored_contract() {
        let mut plan:EditPlan=serde_json::from_value(json!({"fps":30,"clips":[{"shot_id":"a","source":"x.mp4","source_start_frame":0,"source_end_frame":56,"target_start_frame":0,"target_end_frame":56,"continuity_group":"product"},{"shot_id":"b","source":"x.mp4","source_start_frame":56,"source_end_frame":116,"target_start_frame":56,"target_end_frame":116,"continuity_group":"product"}]})).unwrap();
        validate(&plan).unwrap();
        plan.clips[1].zoom = 1.08;
        assert!(validate(&plan).is_err());
        plan.clips[1].cut_intent = Some("intentional-jump".into());
        validate(&plan).unwrap();
    }
    #[tokio::test]
    async fn edit_cannot_overwrite_any_accepted_source() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.mp4");
        std::fs::write(&source, b"accepted source").unwrap();
        let script =
            crate::script::parse_script("<script><hook><HOST>Hello.</hook></script>").unwrap();
        let timeline =
            crate::timeline::project_script_to_timeline(&script, 30.0, 0.0, None).unwrap();
        let project =
            crate::project::create_project_manifest(&script, &timeline, None, 64, 64).unwrap();
        let project_path = dir.path().join("project.json");
        std::fs::write(&project_path, serde_json::to_vec(&project).unwrap()).unwrap();
        let plan_path = dir.path().join("edit.json");
        std::fs::write(&plan_path,serde_json::to_vec(&json!({"fps":30,"clips":[{"shot_id":"source","source":source,"source_start_frame":0,"source_end_frame":project.meta.total_frames,"target_start_frame":0,"target_end_frame":project.meta.total_frames}]})).unwrap()).unwrap();
        let error = render(
            &plan_path,
            &project_path,
            dir.path(),
            &dir.path().join("updated.json"),
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("overwrite an accepted source"));
        assert_eq!(std::fs::read(source).unwrap(), b"accepted source");
        assert!(!dir.path().join(".normalized").exists());
    }
}
