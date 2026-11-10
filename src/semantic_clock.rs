//! Measured local speech domains projected once into the program frame clock.
use crate::{
    project::{ProjectManifest, TrackItem},
    script::ScriptDocument,
    timeline::ResolvedTimeline,
    transcribe::TranscriptDocument,
};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{collections::HashMap, path::Path};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalAlignment {
    pub format: String,
    pub script_identity: String,
    pub segment_id: String,
    pub media_digest: String,
    pub duration_seconds: f64,
    pub fps: f64,
    pub frames: u64,
    pub units: Vec<(f64, f64)>,
    pub transcript: TranscriptDocument,
    #[serde(default)]
    pub performance: Option<crate::performance::Relation>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Occurrence {
    pub start_frame: u64,
    pub end_frame: u64,
    pub alignment: LocalAlignment,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProgramClock {
    pub format: String,
    pub script_identity: String,
    pub fps: f64,
    pub total_frames: u64,
    pub segments: Vec<Occurrence>,
    pub timeline: ResolvedTimeline,
    pub transcript: TranscriptDocument,
}
/// Display is deliberately excluded; unit membership, roles and semantic boundaries persist.
pub fn identity(script: &ScriptDocument) -> Result<String> {
    let segments: Vec<_> = script.segments.iter().map(|s| json!({"id":s.id,"turns":s.turns.iter().map(|t| json!({"role":t.role,"spoken":t.spoken_text,"units":t.units.iter().map(|u| &u.spoken).collect::<Vec<_>>()})).collect::<Vec<_>>(),"moments":s.moments,"selections":s.selections})).collect();
    Ok(crate::artifacts::digest(&serde_json::to_vec(
        &json!({"id":script.id,"segments":segments}),
    )?))
}
fn units(script: &ScriptDocument) -> Vec<String> {
    script
        .segments
        .iter()
        .flat_map(|s| &s.turns)
        .flat_map(|t| &t.units)
        .map(|u| u.spoken.clone())
        .collect()
}
pub async fn align(
    script: &ScriptDocument,
    media: &Path,
    transcript: TranscriptDocument,
    fps: f64,
) -> Result<LocalAlignment> {
    if script.segments.len() != 1 || units(script).is_empty() {
        bail!("align-speech needs one segment with semantic units; use full/timing Script view");
    }
    if !fps.is_finite() || fps <= 0.0 {
        bail!("Invalid alignment frame clock");
    }
    let info = crate::probe::probe_media(media).await?;
    if !info.has_audio || !info.duration_seconds.is_finite() || info.duration_seconds <= 0.0 {
        bail!("Alignment requires finite measured audio");
    }
    let digest = crate::artifacts::content_digest(media)?;
    if let Some(expected) = transcript.evidence["source_media_digest"].as_str() {
        if expected != digest {
            bail!("Transcript belongs to different media bytes; transcribe the adopted performance again");
        }
    }
    let spans = crate::speech_alignment::align_units(&units(script), &transcript)?;
    let times: Vec<_> = spans.iter().map(|s| (s.start, s.end)).collect();
    if times
        .iter()
        .any(|(s, e)| *s < 0.0 || *e > info.duration_seconds + 0.001)
    {
        bail!("Speech evidence exceeds measured local media domain");
    }
    let performance = crate::artifacts::details(media)
        .and_then(|d| d.get("performance").cloned())
        .map(serde_json::from_value::<crate::performance::Relation>)
        .transpose()?;
    if let Some(p) = &performance {
        if p.audio_digest != digest
            || (p.fps - fps).abs() > 1e-6
            || (p.samples as f64 / 48000.0 - info.duration_seconds).abs() > 1.0 / 48000.0 + 1e-6
        {
            bail!("Adopted audio differs from its paired performance frame domain");
        }
    }
    let frames = performance
        .as_ref()
        .map(|p| p.frames)
        .unwrap_or_else(|| (info.duration_seconds * fps).ceil() as u64);
    Ok(LocalAlignment {
        format: "clone-any-video.speech-alignment@1".into(),
        script_identity: identity(script)?,
        segment_id: script.segments[0].id.clone(),
        media_digest: digest,
        duration_seconds: info.duration_seconds,
        fps,
        frames,
        performance,
        units: times,
        transcript,
    })
}
fn excerpt(script: &ScriptDocument, id: &str) -> ScriptDocument {
    let mut doc = script.clone();
    doc.segments.retain(|s| s.id == id);
    doc
}
pub fn project(
    script: &ScriptDocument,
    layout: &ProjectManifest,
    alignments: &HashMap<String, LocalAlignment>,
    fps: f64,
    gap: f64,
) -> Result<(ProgramClock, ProjectManifest)> {
    if !fps.is_finite() || fps <= 0.0 || !gap.is_finite() || gap < 0.0 || script.segments.is_empty()
    {
        bail!("Invalid program clock");
    }
    let mut segments = Vec::new();
    let mut extents = HashMap::new();
    let mut unit_times = HashMap::new();
    let mut cursor = 0u64;
    let gap_frames = (gap * fps).round() as u64;
    let mut transcript = TranscriptDocument {
        format: crate::transcribe::TRANSCRIPT_FORMAT.into(),
        source: "explicit local-domain projection".into(),
        language: String::new(),
        audio_seconds: 0.0,
        passages: Vec::new(),
        evidence: json!({"kind":"projected-local-domains"}),
    };
    for segment in &script.segments {
        let a = alignments
            .get(&segment.id)
            .with_context(|| format!("Missing alignment:{}", segment.id))?;
        if a.format != "clone-any-video.speech-alignment@1"
            || a.segment_id != segment.id
            || a.script_identity != identity(&excerpt(script, &segment.id))?
            || (a.fps - fps).abs() > 1e-6
            || a.frames == 0
            || a.frames
                != a.performance
                    .as_ref()
                    .map(|p| p.frames)
                    .unwrap_or_else(|| (a.duration_seconds * fps).ceil() as u64)
        {
            bail!("Alignment identity/domain mismatch for {}", segment.id);
        }
        if a.units.len() != units(&excerpt(script, &segment.id)).len() {
            bail!("Incomplete local unit coverage");
        }
        let observed = crate::speech_alignment::align_units(
            &units(&excerpt(script, &segment.id)),
            &a.transcript,
        )?;
        if observed.iter().zip(&a.units).any(|(s, (start, end))| {
            (s.start - start).abs() > 1e-6
                || (s.end - end).abs() > 1e-6
                || *start < 0.0
                || *end > a.duration_seconds + 0.001
        }) {
            bail!("Alignment unit timing differs from its local evidence/domain");
        }
        let end = cursor
            .checked_add(a.frames)
            .context("Frame clock overflow")?;
        let start = cursor as f64 / fps;
        let mut local = a.transcript.clone();
        for p in &mut local.passages {
            p.start_seconds = p.start_seconds.map(|t| t + start);
            p.end_seconds = p.end_seconds.map(|t| t + start);
            for w in &mut p.words {
                w.start_seconds = w.start_seconds.map(|t| t + start);
                w.end_seconds = w.end_seconds.map(|t| t + start);
            }
        }
        transcript.passages.extend(local.passages);
        transcript.language = a.transcript.language.clone();
        extents.insert(segment.id.clone(), a.frames as f64 / fps);
        unit_times.insert(segment.id.clone(), a.units.clone());
        segments.push(Occurrence {
            start_frame: cursor,
            end_frame: end,
            alignment: a.clone(),
        });
        cursor = end
            .checked_add(gap_frames)
            .context("Frame clock overflow")?;
    }
    let total_frames = cursor - gap_frames;
    transcript.audio_seconds = total_frames as f64 / fps;
    let mut timeline = crate::timeline::project_script_to_timeline_with_alignments(
        script,
        fps,
        gap_frames as f64 / fps,
        Some(&extents),
        Some(&unit_times),
    )?;
    // Warp authored frame boundaries through the measured integer domains. Shared source
    // boundaries have two sides when a formerly gapless edit acquires a real gap.
    if layout.meta.total_frames == 0 {
        bail!("Layout has no frame domain");
    }
    let mut knots = vec![(0u64, 0u64)];
    let mut previous = 0;
    for occurrence in &segments {
        let voice = layout
            .tracks
            .voice
            .iter()
            .find(|v| v.id == format!("voice_{}", occurrence.alignment.segment_id))
            .context("Missing authored voice window")?;
        if voice.end_frame <= voice.start_frame
            || voice.start_frame < previous
            || voice.end_frame > layout.meta.total_frames
        {
            bail!("Authored voice frame windows must be ordered within the layout");
        }
        knots.push((voice.start_frame, occurrence.start_frame));
        knots.push((voice.end_frame, occurrence.end_frame));
        previous = voice.end_frame;
    }
    knots.push((layout.meta.total_frames, total_frames));
    knots.sort_by_key(|p| p.0);
    let warp = |frame: u64, is_end: bool| -> u64 {
        let values: Vec<_> = knots.iter().filter(|p| p.0 == frame).map(|p| p.1).collect();
        if !values.is_empty() {
            return if is_end {
                *values.iter().min().unwrap()
            } else {
                *values.iter().max().unwrap()
            };
        }
        for pair in knots.windows(2) {
            let ((x0, y0), (x1, y1)) = (pair[0], pair[1]);
            if frame < x1 && x1 > x0 {
                return (y0 as f64 + (frame - x0) as f64 / (x1 - x0) as f64 * (y1 - y0) as f64)
                    .round() as u64;
            }
        }
        total_frames
    };
    let mut project = layout.clone();
    project.meta.fps = fps;
    project.meta.total_frames = total_frames;
    project.meta.total_seconds = total_frames as f64 / fps;
    for shot in &mut project.shots {
        shot.start_frame = warp(shot.start_frame, false);
        shot.end_frame = warp(shot.end_frame, true);
        shot.start_seconds = shot.start_frame as f64 / fps;
        shot.end_seconds = shot.end_frame as f64 / fps;
        shot.duration_seconds = shot.end_seconds - shot.start_seconds;
    }
    for track in [
        &mut project.tracks.visual_bg,
        &mut project.tracks.visual_components,
        &mut project.tracks.captions,
        &mut project.tracks.sfx,
        &mut project.tracks.bgm,
    ] {
        for item in track {
            item.start_frame = warp(item.start_frame, false);
            item.end_frame = warp(item.end_frame, true);
            item.start_seconds = item.start_frame as f64 / fps;
            item.end_seconds = item.end_frame as f64 / fps;
        }
    }
    timeline.total_frames = total_frames;
    timeline.total_seconds = total_frames as f64 / fps;
    let mut names = std::collections::HashSet::new();
    if timeline
        .instants
        .iter()
        .any(|i| !names.insert(i.id.clone()))
    {
        bail!("Moment names must be unique in the program clock");
    }
    names.clear();
    if timeline.windows.iter().any(|w| !names.insert(w.id.clone())) {
        bail!("Segment/Selection names must be unique in their clock scope");
    }
    for occurrence in &segments {
        let voice = project
            .tracks
            .voice
            .iter_mut()
            .find(|v| v.id == format!("voice_{}", occurrence.alignment.segment_id))
            .context("Missing voice window")?;
        voice.start_frame = occurrence.start_frame;
        voice.end_frame = occurrence.end_frame;
        voice.start_seconds = voice.start_frame as f64 / fps;
        voice.end_seconds = voice.end_frame as f64 / fps;
        voice.payload["source_media_digest"] = json!(occurrence.alignment.media_digest);
        voice.payload["performance"] = serde_json::to_value(&occurrence.alignment.performance)?;
        voice.payload["spoken_text"] = json!(
            script
                .segments
                .iter()
                .find(|s| s.id == occurrence.alignment.segment_id)
                .unwrap()
                .spoken_text
        );
        voice.payload["timing_status"] = json!("measured-domain-projected");
        if let Some(w) = timeline
            .windows
            .iter_mut()
            .find(|w| w.id == occurrence.alignment.segment_id)
        {
            w.start_frame = occurrence.start_frame;
            w.end_frame = occurrence.end_frame;
            w.duration_frames = w.end_frame - w.start_frame;
            w.start_seconds = w.start_frame as f64 / fps;
            w.end_seconds = w.end_frame as f64 / fps;
            w.duration_seconds = w.duration_frames as f64 / fps;
        }
    }
    let clock = ProgramClock {
        format: "clone-any-video.program-clock@1".into(),
        script_identity: identity(script)?,
        fps,
        total_frames,
        segments,
        timeline,
        transcript,
    };
    for tracks in [
        &mut project.tracks.visual_bg,
        &mut project.tracks.visual_components,
        &mut project.tracks.captions,
        &mut project.tracks.sfx,
        &mut project.tracks.bgm,
    ] {
        for item in tracks {
            clock.bind(item)?;
        }
    }
    for shot in &mut project.shots {
        let original = layout
            .shots
            .iter()
            .find(|s| s.id == shot.id)
            .context("Unknown authored shot")?;
        let rows: Vec<_> = layout
            .tracks
            .visual_bg
            .iter()
            .enumerate()
            .filter(|(_, r)| {
                r.payload["shot_id"] == shot.id
                    || r.id == format!("bg_{}", shot.id)
                    || (r.start_frame == original.start_frame && r.end_frame == original.end_frame)
            })
            .collect();
        if rows.len() > 1 {
            bail!("Ambiguous background binding for {}", shot.id);
        }
        if let Some((index, _)) = rows.first() {
            let bg = &project.tracks.visual_bg[*index];
            if bg.payload.get("timing").is_some() {
                shot.start_frame = bg.start_frame;
                shot.end_frame = bg.end_frame;
                shot.start_seconds = bg.start_seconds;
                shot.end_seconds = bg.end_seconds;
                shot.duration_seconds = bg.end_seconds - bg.start_seconds;
            }
        }
    }
    crate::project::validate_project(&project)?;
    Ok((clock, project))
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Binding {
    from: String,
    until: Option<String>,
    duration_frames: Option<u64>,
    #[serde(default)]
    offset_frames: i64,
}
impl ProgramClock {
    pub fn validate(&self) -> Result<()> {
        if self.format != "clone-any-video.program-clock@1"
            || !self.fps.is_finite()
            || self.fps <= 0.0
            || self.total_frames == 0
            || self.timeline.total_frames != self.total_frames
            || (self.timeline.frame_rate - self.fps).abs() > 1e-6
        {
            bail!("Invalid program clock domain");
        }
        let mut end = 0;
        for s in &self.segments {
            if s.start_frame < end
                || s.end_frame <= s.start_frame
                || s.end_frame > self.total_frames
                || s.end_frame - s.start_frame != s.alignment.frames
            {
                bail!("Local domain and program Window must have equal frame extents");
            }
            end = s.end_frame;
        }
        if end != self.total_frames {
            bail!("Clock domain coverage differs from program extent");
        }
        Ok(())
    }
    pub fn check(&self, script: &ScriptDocument) -> Result<()> {
        self.validate()?;
        if self.format != "clone-any-video.program-clock@1"
            || self.script_identity != identity(script)?
        {
            bail!("Program clock differs from current Script semantics");
        }
        Ok(())
    }
    pub fn point(&self, key: &str) -> Result<u64> {
        let parts: Vec<_> = key.split(':').collect();
        match parts.as_slice() {
            ["program", "start"] => Ok(0),
            ["program", "end"] => Ok(self.total_frames),
            ["moment", id] => self
                .timeline
                .instants
                .iter()
                .find(|p| p.id == *id)
                .map(|p| p.at_frame)
                .context("Unknown Moment"),
            ["segment", id, boundary] => {
                let s = self
                    .segments
                    .iter()
                    .find(|s| s.alignment.segment_id == *id)
                    .context("Unknown Segment")?;
                match *boundary {
                    "start" => Ok(s.start_frame),
                    "end" => Ok(s.end_frame),
                    _ => bail!("Unknown boundary"),
                }
            }
            ["selection", seg, id, boundary] => {
                let id = format!("{}:{}", seg, id);
                let w = self
                    .timeline
                    .windows
                    .iter()
                    .find(|w| w.id == id)
                    .context("Unknown Selection")?;
                match *boundary {
                    "start" => Ok(w.start_frame),
                    "end" => Ok(w.end_frame),
                    _ => bail!("Unknown boundary"),
                }
            }
            _ => bail!("Unknown clock reference {}", key),
        }
    }
    fn bind(&self, item: &mut TrackItem) -> Result<()> {
        let Some(value) = item.payload.get("timing") else {
            return Ok(());
        };
        let binding: Binding = serde_json::from_value(value.clone())?;
        let raw = self.point(&binding.from)? as i128 + binding.offset_frames as i128;
        if raw < 0 || raw > self.total_frames as i128 {
            bail!("Timing starts outside program");
        }
        let start = raw as u64;
        let end = match (&binding.until, binding.duration_frames) {
            (Some(key), None) => self.point(key)?,
            (None, Some(n)) => start.checked_add(n).context("Frame overflow")?,
            _ => bail!("timing requires exactly one until or duration_frames"),
        };
        if end <= start || end > self.total_frames {
            bail!("Bound track {} exceeds program or has no extent", item.id);
        }
        item.start_frame = start;
        item.end_frame = end;
        item.start_seconds = start as f64 / self.fps;
        item.end_seconds = end as f64 / self.fps;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    async fn fixture(
        dir: &Path,
        world_start: f64,
    ) -> (ScriptDocument, ProjectManifest, LocalAlignment) {
        let script=crate::script::parse_script("<script id=\"clock\"><hook><HOST>Hello. || @{reveal!} @{proof}world.@{/proof}</hook></script>").unwrap();
        let timeline = crate::timeline::project_script_to_timeline(
            &script,
            30.0,
            0.0,
            Some(&HashMap::from([("hook".into(), 4.0)])),
        )
        .unwrap();
        let project =
            crate::project::create_project_manifest(&script, &timeline, None, 64, 64).unwrap();
        let audio = dir.join("voice.wav");
        let status = tokio::process::Command::new("ffmpeg")
            .args([
                "-v",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "anullsrc=r=48000:cl=stereo:d=4",
            ])
            .arg(&audio)
            .status()
            .await
            .unwrap();
        assert!(status.success());
        let transcript:TranscriptDocument=serde_json::from_value(json!({"format":"clone-any-video.transcript@1","source":"explicit timing fixture, not measured ASR","language":"en","audio_seconds":4,"passages":[{"text":"Hello world.","start_seconds":0,"end_seconds":4,"words":[{"text":"Hello.","start_seconds":0.1,"end_seconds":0.4},{"text":"world.","start_seconds":world_start,"end_seconds":world_start+0.5}]}]})).unwrap();
        let a = align(&script, &audio, transcript, 30.0).await.unwrap();
        (script, project, a)
    }
    #[tokio::test]
    async fn word_pause_moves_caption_selection_and_effect_without_changing_program_duration() {
        let dir = tempfile::tempdir().unwrap();
        let (script, layout, a) = fixture(dir.path(), 3.0).await;
        let (clock, project) = project(
            &script,
            &layout,
            &HashMap::from([("hook".into(), a)]),
            30.0,
            0.0,
        )
        .unwrap();
        assert_eq!(clock.total_frames, 120);
        assert_eq!(clock.point("moment:reveal").unwrap(), 90);
        assert_eq!(project.tracks.visual_components[0].start_frame, 90);
        assert_eq!(project.tracks.sfx[0].start_frame, 90);
        let caption = crate::caption_engine::build_caption_document_from_script(&script).unwrap();
        let timing = crate::caption_engine::build_caption_timing_from_transcript(
            &caption,
            &clock.transcript,
        )
        .unwrap();
        assert_eq!(timing.units.last().unwrap().start_time, 3.0);
        let (_, _, b) = fixture(dir.path(), 2.0).await;
        let (second, p) = super::project(
            &script,
            &layout,
            &HashMap::from([("hook".into(), b)]),
            30.0,
            0.0,
        )
        .unwrap();
        assert_eq!(second.total_frames, 120);
        assert_eq!(p.tracks.sfx[0].start_frame, 60);
        let mut changed = script.clone();
        changed.segments[0].turns[0].units[1].spoken = "different".into();
        assert!(clock.check(&changed).is_err());
        let mut display = script.clone();
        display.segments[0].turns[0].units[1].display = "EARTH".into();
        clock.check(&display).unwrap();
    }
    #[tokio::test]
    async fn local_evidence_domain_and_semantics_cannot_be_rebound_silently() {
        let dir = tempfile::tempdir().unwrap();
        let (script, layout, a) = fixture(dir.path(), 3.0).await;
        let mut bad = a.clone();
        bad.script_identity = "stale".into();
        assert!(project(
            &script,
            &layout,
            &HashMap::from([("hook".into(), bad)]),
            30.0,
            0.0
        )
        .is_err());
        let mut transcript = a.transcript;
        transcript.evidence = json!({"source_media_digest":"another-take"});
        assert!(
            align(&script, &dir.path().join("voice.wav"), transcript, 30.0)
                .await
                .is_err()
        );
    }
}

#[cfg(test)]
mod frame_regressions {
    use super::*;
    #[test]
    fn integer_domains_and_a_new_gap_preserve_both_sides_of_an_old_cut() {
        let script = crate::script::parse_script(
            "<script id=\"domains\"><hook><HOST>Hello.</hook><outro><HOST>world.</outro></script>",
        )
        .unwrap();
        let timeline =
            crate::timeline::project_script_to_timeline(&script, 30.0, 0.0, None).unwrap();
        let layout =
            crate::project::create_project_manifest(&script, &timeline, None, 64, 64).unwrap();
        let mut alignments = HashMap::new();
        for (id, text) in [("hook", "Hello."), ("outro", "world.")] {
            let transcript=serde_json::from_value(json!({"format":"clone-any-video.transcript@1","source":"explicit domain fixture","language":"en","audio_seconds":1.01,"passages":[{"text":text,"start_seconds":0,"end_seconds":1.01,"words":[{"text":text,"start_seconds":0.1,"end_seconds":0.4}]}]})).unwrap();
            alignments.insert(
                id.into(),
                LocalAlignment {
                    format: "clone-any-video.speech-alignment@1".into(),
                    script_identity: identity(&excerpt(&script, id)).unwrap(),
                    segment_id: id.into(),
                    media_digest: "fixture".into(),
                    performance: None,
                    duration_seconds: 1.01,
                    fps: 30.0,
                    frames: 31,
                    units: vec![(0.1, 0.4)],
                    transcript,
                },
            );
        }
        let (clock, p) = project(&script, &layout, &alignments, 30.0, 4.0 / 30.0).unwrap();
        assert_eq!(clock.total_frames, 66);
        assert_eq!(p.shots[0].end_frame, 31);
        assert_eq!(p.shots[1].start_frame, 35);
        assert_eq!(p.shots[1].end_frame, 66);
        assert_eq!(
            p.tracks.voice[1].end_frame - p.tracks.voice[1].start_frame,
            31
        );
        clock.validate().unwrap();
        let mut legacy = layout.clone();
        legacy.tracks.visual_bg[0].payload =
            json!({"timing":{"from":"program:start","offset_frames":3,"duration_frames":20}});
        let (_, p) = project(&script, &legacy, &alignments, 30.0, 4.0 / 30.0).unwrap();
        assert_eq!(p.shots[0].start_frame, 3);
        assert_eq!(p.shots[0].end_frame, 23);
    }
}
