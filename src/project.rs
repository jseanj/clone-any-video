use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

use crate::script::ScriptDocument;
use crate::timeline::ResolvedTimeline;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum VisualType {
    GeneratedImage,
    GeneratedVideo,
    HtmlComponent,
    RecordedMedia,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum AssetTaskType {
    TtsSpeech,
    AiImage,
    AiVideo,
    DynamicHtml,
    SoundEffect,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum TaskStatus {
    Pending,
    Ready,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProjectMeta {
    pub id: String,
    pub title: String,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub total_seconds: f64,
    pub total_frames: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ShotDefinition {
    pub id: String,
    pub segment_id: String,
    pub shot_index: usize,
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub start_frame: u64,
    pub end_frame: u64,
    pub duration_seconds: f64,
    pub visual_type: VisualType,
    pub description: String,
    pub prompt_hint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TrackItem {
    pub id: String,
    pub name: String,
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub start_frame: u64,
    pub end_frame: u64,
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LayerTracks {
    pub visual_bg: Vec<TrackItem>,
    pub visual_components: Vec<TrackItem>,
    pub captions: Vec<TrackItem>,
    pub voice: Vec<TrackItem>,
    pub sfx: Vec<TrackItem>,
    pub bgm: Vec<TrackItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AssetTask {
    pub id: String,
    pub task_type: AssetTaskType,
    pub target: String,
    pub status: TaskStatus,
    pub spec: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProjectManifest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub master_audio: Option<std::path::PathBuf>,
    pub format: String,
    pub meta: ProjectMeta,
    pub shots: Vec<ShotDefinition>,
    pub tracks: LayerTracks,
    pub asset_tasks: Vec<AssetTask>,
}

/// Create a comprehensive ProjectManifest binding script semantics, timeline math, and production tasks.
pub fn create_project_manifest(
    script: &ScriptDocument,
    timeline: &ResolvedTimeline,
    title: Option<&str>,
    width: u32,
    height: u32,
) -> Result<ProjectManifest> {
    let proj_title = title.unwrap_or(&script.id).to_string();

    let meta = ProjectMeta {
        id: script.id.clone(),
        title: proj_title,
        width,
        height,
        fps: timeline.frame_rate,
        total_seconds: timeline.total_seconds,
        total_frames: timeline.total_frames,
    };

    let mut shots = Vec::new();
    let mut visual_bg = Vec::new();
    let mut visual_components = Vec::new();
    let mut captions = Vec::new();
    let mut voice = Vec::new();
    let mut sfx = Vec::new();
    let mut bgm = Vec::new();
    let mut asset_tasks = Vec::new();

    let mut shot_counter = 1usize;

    // 1. Process Main Windows -> Shots & Visual BG
    for win in &timeline.windows {
        if win.id.contains(':') {
            // This is a sub-selection, mapped to visual_components below
            continue;
        }

        let shot_id = format!("shot_{:02}", shot_counter);
        let shot_desc = format!("Visual presentation for segment '{}'", win.id);

        let v_type = if shot_counter == 1 || win.id == "hook" {
            VisualType::GeneratedVideo
        } else if win.id.contains("outro") || win.id.contains("cta") {
            VisualType::HtmlComponent
        } else {
            VisualType::GeneratedImage
        };

        let prompt_hint = format!(
            "Cinema 4K, realistic lighting, {} theme, clean composition, matching role {}",
            win.id, meta.title
        );

        shots.push(ShotDefinition {
            id: shot_id.clone(),
            segment_id: win.id.clone(),
            shot_index: shot_counter,
            start_seconds: win.start_seconds,
            end_seconds: win.end_seconds,
            start_frame: win.start_frame,
            end_frame: win.end_frame,
            duration_seconds: win.duration_seconds,
            visual_type: v_type.clone(),
            description: shot_desc,
            prompt_hint: Some(prompt_hint.clone()),
        });

        visual_bg.push(TrackItem {
            id: format!("track_bg_{:02}", shot_counter),
            name: format!("BG: {}", win.id),
            start_seconds: win.start_seconds,
            end_seconds: win.end_seconds,
            start_frame: win.start_frame,
            end_frame: win.end_frame,
            payload: serde_json::json!({
                "shot_id": shot_id,
                "visual_type": v_type,
            }),
        });

        // Add corresponding image / video asset task
        match v_type {
            VisualType::GeneratedVideo => {
                asset_tasks.push(AssetTask {
                    id: format!("task_video_{:02}", shot_counter),
                    task_type: AssetTaskType::AiVideo,
                    target: shot_id.clone(),
                    status: TaskStatus::Pending,
                    spec: serde_json::json!({
                        "prompt": prompt_hint,
                        "duration_seconds": win.duration_seconds,
                        "width": width,
                        "height": height,
                        "fps": timeline.frame_rate,
                    }),
                });
            }
            VisualType::GeneratedImage => {
                asset_tasks.push(AssetTask {
                    id: format!("task_image_{:02}", shot_counter),
                    task_type: AssetTaskType::AiImage,
                    target: shot_id.clone(),
                    status: TaskStatus::Pending,
                    spec: serde_json::json!({
                        "prompt": prompt_hint,
                        "width": width,
                        "height": height,
                    }),
                });
            }
            VisualType::HtmlComponent => {
                asset_tasks.push(AssetTask {
                    id: format!("task_html_{:02}", shot_counter),
                    task_type: AssetTaskType::DynamicHtml,
                    target: shot_id.clone(),
                    status: TaskStatus::Pending,
                    spec: serde_json::json!({
                        "component": "EndCardWidget",
                        "title": meta.title,
                        "duration_seconds": win.duration_seconds,
                    }),
                });
            }
            VisualType::RecordedMedia => {}
        }

        shot_counter += 1;
    }

    // 2. Process Sub-selections -> Visual Components (e.g. badges, highlights)
    for win in &timeline.windows {
        if let Some((seg_id, sel_name)) = win.id.split_once(':') {
            visual_components.push(TrackItem {
                id: format!("comp_{}_{}", seg_id, sel_name),
                name: format!("Overlay: {}", sel_name),
                start_seconds: win.start_seconds,
                end_seconds: win.end_seconds,
                start_frame: win.start_frame,
                end_frame: win.end_frame,
                payload: serde_json::json!({
                    "component_type": "SelectionBadge",
                    "selection_name": sel_name,
                    "parent_segment": seg_id,
                    "timing": {"from":format!("selection:{}:{}:start",seg_id,sel_name), "until":format!("selection:{}:{}:end",seg_id,sel_name)},
                }),
            });
        }
    }

    // 3. Process Script Turns -> Voice Track & TTS Tasks
    let mut cue_idx = 1usize;
    for seg in &script.segments {
        let seg_win = timeline.windows.iter().find(|w| w.id == seg.id);
        let seg_start = seg_win.map(|w| w.start_seconds).unwrap_or(0.0);
        let seg_dur = seg_win
            .map(|w| w.duration_seconds)
            .unwrap_or(seg.estimated_seconds);

        // Turn voice track item
        voice.push(TrackItem {
            id: format!("voice_{}", seg.id),
            name: format!("Speech: {}", seg.id),
            start_seconds: seg_start,
            end_seconds: seg_start + seg_dur,
            start_frame: (seg_start * timeline.frame_rate).round() as u64,
            end_frame: ((seg_start + seg_dur) * timeline.frame_rate).round() as u64,
            payload: serde_json::json!({
                "spoken_text": seg.spoken_text,
                "turns_count": seg.turns.len(),
            }),
        });

        asset_tasks.push(AssetTask {
            id: format!("task_tts_{}", seg.id),
            task_type: AssetTaskType::TtsSpeech,
            target: format!("voice_{}", seg.id),
            status: TaskStatus::Pending,
            spec: serde_json::json!({
                "text": seg.spoken_text,
                "role": seg.turns.first().and_then(|t| t.role.clone()).unwrap_or_else(|| "NARRATOR".to_string()),
                "target_duration_seconds": seg_dur,
            }),
        });

        // Captions from cues
        let mut turn_offset = 0.0;
        for turn in &seg.turns {
            let cue_count = turn.cues.len().max(1);
            let cue_slice_dur = seg_dur / cue_count as f64;

            for cue in &turn.cues {
                let cue_start = seg_start + turn_offset;
                let cue_end = (cue_start + cue_slice_dur).min(seg_start + seg_dur);

                captions.push(TrackItem {
                    id: format!("cue_{:02}", cue_idx),
                    name: format!("Caption #{:02}", cue_idx),
                    start_seconds: (cue_start * 1000.0).round() / 1000.0,
                    end_seconds: (cue_end * 1000.0).round() / 1000.0,
                    start_frame: (cue_start * timeline.frame_rate).round() as u64,
                    end_frame: (cue_end * timeline.frame_rate).round() as u64,
                    payload: serde_json::json!({
                        "text": cue.text,
                        "tokens_count": cue.tokens.len(),
                    }),
                });
                cue_idx += 1;
                turn_offset += cue_slice_dur;
            }
        }
    }

    // 4. Process Instants -> SFX (Sound Effects)
    for inst in &timeline.instants {
        sfx.push(TrackItem {
            id: format!("sfx_{}", inst.id),
            name: format!("SFX: {}", inst.id),
            start_seconds: inst.at_seconds,
            end_seconds: inst.at_seconds + 0.5,
            start_frame: inst.at_frame,
            end_frame: inst.at_frame + (0.5 * timeline.frame_rate).round() as u64,
            payload: serde_json::json!({
                "sound_tag": inst.id,
                "timing": {"from":format!("moment:{}",inst.id), "duration_frames":(0.5*timeline.frame_rate).round() as u64},
                "category": "impact_whoosh",
            }),
        });

        asset_tasks.push(AssetTask {
            id: format!("task_sfx_{}", inst.id),
            task_type: AssetTaskType::SoundEffect,
            target: format!("sfx_{}", inst.id),
            status: TaskStatus::Pending,
            spec: serde_json::json!({
                "trigger_at": inst.at_seconds,
                "sfx_type": "transition_pop",
            }),
        });
    }

    // 5. Global BGM (Background Music)
    bgm.push(TrackItem {
        id: "bgm_main".to_string(),
        name: "Main BGM Track".to_string(),
        start_seconds: 0.0,
        end_seconds: timeline.total_seconds,
        start_frame: 0,
        end_frame: timeline.total_frames,
        payload: serde_json::json!({
            "genre": "modern_tech_ambient",
            "ducking_during_voice": true,
            "ducking_db": -12.0,
        }),
    });

    Ok(ProjectManifest {
        master_audio: None,
        format: "clone-any-video.project@1".to_string(),
        meta,
        shots,
        tracks: LayerTracks {
            visual_bg,
            visual_components,
            captions,
            voice,
            sfx,
            bgm,
        },
        asset_tasks,
    })
}

/// Configure visible speaking performance without requesting an independent TTS track.
pub fn use_speaking_video(manifest: &mut ProjectManifest, script: &ScriptDocument) -> Result<()> {
    if script.segments.iter().any(|s| s.turns.len() != 1 || s.spoken_text.trim().is_empty()) {
        bail!("Speaking-video blueprint requires one nonempty speaking turn per segment");
    }
    manifest.asset_tasks.retain(|t| t.task_type != AssetTaskType::TtsSpeech);
    for shot in &mut manifest.shots {
        let segment = script.segments.iter().find(|s| s.id == shot.segment_id)
            .ok_or_else(|| anyhow::anyhow!("Speaking shot has no Script segment"))?;
        shot.visual_type = VisualType::GeneratedVideo;
        let task = manifest.asset_tasks.iter_mut().find(|t| t.target == shot.id)
            .ok_or_else(|| anyhow::anyhow!("Speaking shot has no declared generation task"))?;
        task.task_type = AssetTaskType::AiVideo;
        task.spec = serde_json::json!({"shot_id":shot.id,"duration_seconds":shot.duration_seconds,
            "width":manifest.meta.width,"height":manifest.meta.height,"fps":manifest.meta.fps,
            "dialogue":segment.spoken_text,"provides_audio_for":format!("voice_{}",segment.id),
            "voice_reference":null,"performance":"native-synchronized-audio-video"});
        for bg in &mut manifest.tracks.visual_bg {
            if bg.payload["shot_id"] == shot.id {
                bg.payload["visual_type"] = serde_json::json!(VisualType::GeneratedVideo);
            }
        }
        for voice in &mut manifest.tracks.voice {
            if voice.id == format!("voice_{}", segment.id) {
                voice.payload["performance_source"] = serde_json::json!(shot.id);
            }
        }
    }
    Ok(())
}

/// Validate integrity of ProjectManifest against audio-visual constraints.
pub fn validate_project(manifest: &ProjectManifest) -> Result<Vec<String>> {
    let mut warnings = Vec::new();

    if manifest.meta.total_seconds <= 0.0 {
        bail!("Project total_seconds must be positive");
    }
    if manifest.shots.is_empty() {
        bail!("Project must have at least one shot");
    }

    // 1. Check shot continuity and overlapping
    for i in 0..manifest.shots.len() {
        let shot = &manifest.shots[i];
        if shot.duration_seconds <= 0.0 {
            bail!(
                "Shot '{}' has non-positive duration: {:.3}s",
                shot.id,
                shot.duration_seconds
            );
        }
        if shot.end_frame < shot.start_frame {
            bail!("Shot '{}' has end_frame < start_frame", shot.id);
        }
        if i > 0 {
            let prev = &manifest.shots[i - 1];
            if shot.start_seconds < prev.start_seconds {
                bail!(
                    "Shot '{}' starts before previous shot '{}'",
                    shot.id,
                    prev.id
                );
            }
        }
    }

    // 2. Check asset task coverage
    for shot in &manifest.shots {
        let has_task = manifest.asset_tasks.iter().any(|t| t.target == shot.id);
        if !has_task {
            warnings.push(format!(
                "Notice: Shot '{}' has no dedicated asset generation task",
                shot.id
            ));
        }
    }

    Ok(warnings)
}

/// Render a clean production blueprint summary for CLI display.
pub fn render_project_summary(manifest: &ProjectManifest) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "=== Video Production Blueprint: [{}] ===\n",
        manifest.meta.title
    ));
    out.push_str(&format!(
        "Format: {} | Resolution: {}x{} | FPS: {:.2} | Duration: {:.3}s ({} frames)\n\n",
        manifest.format,
        manifest.meta.width,
        manifest.meta.height,
        manifest.meta.fps,
        manifest.meta.total_seconds,
        manifest.meta.total_frames
    ));

    out.push_str("📺 [Shot List & Timeline Plan]\n");
    out.push_str("┌───────┬────────────┬─────────────────────┬───────────────────┬────────────────────┬─────────────────────────────┐\n");
    out.push_str("│ Shot  │ Segment    │ Time Range (s)      │ Frame Range (f)   │ Visual Type        │ Description                 │\n");
    out.push_str("├───────┼────────────┼─────────────────────┼───────────────────┼────────────────────┼─────────────────────────────┤\n");
    for shot in &manifest.shots {
        let v_str = match shot.visual_type {
            VisualType::GeneratedImage => "AI Image",
            VisualType::GeneratedVideo => "AI Video",
            VisualType::HtmlComponent => "HTML/SVG",
            VisualType::RecordedMedia => "Footage",
        };
        out.push_str(&format!(
            "│ {:<5} │ {:<10} │ {:>7.3}s ~ {:>7.3}s │ {:>7} ~ {:>7} │ {:<18} │ {:<27} │\n",
            shot.id,
            shot.segment_id,
            shot.start_seconds,
            shot.end_seconds,
            shot.start_frame,
            shot.end_frame,
            v_str,
            shot.description.chars().take(27).collect::<String>()
        ));
    }
    out.push_str("└───────┴────────────┴─────────────────────┴───────────────────┴────────────────────┴─────────────────────────────┘\n\n");

    out.push_str("🎚️ [Multi-Track Layer Allocation]\n");
    out.push_str(&format!(
        "  - Visual Background Track : {} items\n",
        manifest.tracks.visual_bg.len()
    ));
    out.push_str(&format!(
        "  - Component / Overlay Track: {} items\n",
        manifest.tracks.visual_components.len()
    ));
    out.push_str(&format!(
        "  - Word Caption Track      : {} cues\n",
        manifest.tracks.captions.len()
    ));
    out.push_str(&format!(
        "  - Voice Speech Track      : {} audio takes\n",
        manifest.tracks.voice.len()
    ));
    out.push_str(&format!(
        "  - Sound Effects (SFX)     : {} hits\n",
        manifest.tracks.sfx.len()
    ));
    out.push_str(&format!(
        "  - Background Music (BGM)  : {} master track\n\n",
        manifest.tracks.bgm.len()
    ));

    out.push_str("📋 [Asset Store Production Tasks Queue]\n");
    for (idx, task) in manifest.asset_tasks.iter().enumerate() {
        let t_name = match task.task_type {
            AssetTaskType::TtsSpeech => "[TTS Voice]",
            AssetTaskType::AiImage => "[AI Image ]",
            AssetTaskType::AiVideo => "[AI Video ]",
            AssetTaskType::DynamicHtml => "[HTML/SVG ]",
            AssetTaskType::SoundEffect => "[SFX Audio]",
        };
        out.push_str(&format!(
            "  {:02}. {:<11} -> Target: {:<16} | Status: {:?}\n",
            idx + 1,
            t_name,
            task.target,
            task.status
        ));
    }

    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn speaking_blueprint_keeps_picture_and_audio_in_one_generation_request() {
        let script = crate::script::parse_script("<script><presentation><HOST>Show the serum. || Try a few drops.</presentation></script>").unwrap();
        let tl = crate::timeline::project_script_to_timeline(&script,24.0,0.0,None).unwrap();
        let mut p = super::create_project_manifest(&script,&tl,None,360,640).unwrap();
        super::use_speaking_video(&mut p,&script).unwrap();
        assert!(!p.asset_tasks.iter().any(|t| t.task_type == super::AssetTaskType::TtsSpeech));
        let video = p.asset_tasks.iter().find(|t|t.task_type == super::AssetTaskType::AiVideo).unwrap();
        assert_eq!(video.spec["provides_audio_for"],"voice_presentation");
        assert_eq!(video.spec["dialogue"],script.segments[0].spoken_text);
        assert_eq!(p.tracks.voice[0].payload["performance_source"],p.shots[0].id);
        assert!(super::validate_project(&p).unwrap().is_empty());
    }
    use super::*;
    use crate::script::parse_script;
    use crate::timeline::project_script_to_timeline;
    use std::collections::HashMap;

    #[test]
    fn test_project_manifest_generation() {
        let svml = r#"
        <script id="promo_project">
          <hook>
            <HOST> @{claim!} Want five tools in one?
          </hook>
          <demo>
            <HOST> @{badge} Pure Rust speed! @{/badge}
          </demo>
          <cta>
            <HOST> Check out GitHub!
          </cta>
        </script>
        "#;

        let script = parse_script(svml).unwrap();
        let mut extents = HashMap::new();
        extents.insert("hook".to_string(), 3.0);
        extents.insert("demo".to_string(), 4.0);
        extents.insert("cta".to_string(), 3.0);

        let timeline = project_script_to_timeline(&script, 30.0, 0.3, Some(&extents)).unwrap();
        let manifest =
            create_project_manifest(&script, &timeline, Some("Clone Any Video"), 1280, 720)
                .unwrap();

        assert_eq!(manifest.meta.width, 1280);
        assert_eq!(manifest.meta.height, 720);
        assert_eq!(manifest.meta.fps, 30.0);
        assert_eq!(manifest.shots.len(), 3);
        assert_eq!(manifest.tracks.voice.len(), 3);
        assert!(!manifest.asset_tasks.is_empty());

        // Verify semantic timing anchors on components and moments
        assert_eq!(manifest.tracks.sfx.len(), 1);
        assert_eq!(
            manifest.tracks.sfx[0].payload["timing"]["from"],
            "moment:claim"
        );
        assert_eq!(
            manifest.tracks.sfx[0].payload["timing"]["duration_frames"],
            15
        );

        assert_eq!(manifest.tracks.visual_components.len(), 1);
        assert_eq!(
            manifest.tracks.visual_components[0].payload["timing"]["from"],
            "selection:demo:badge:start"
        );
        assert_eq!(
            manifest.tracks.visual_components[0].payload["timing"]["until"],
            "selection:demo:badge:end"
        );

        let validation = validate_project(&manifest).unwrap();
        assert!(validation.is_empty() || validation.iter().all(|n| n.starts_with("Notice")));
    }
}
