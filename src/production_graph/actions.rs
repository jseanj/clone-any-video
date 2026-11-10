use super::{model::*, plan};
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, HashMap},
    path::{Path, PathBuf},
};

fn path<'a>(inputs: &'a BTreeMap<String, Material>, name: &str) -> Result<&'a Path> {
    Ok(&inputs
        .get(name)
        .with_context(|| format!("Missing input {}", name))?
        .artifact
        .path)
}
fn value(inputs: &BTreeMap<String, Material>, name: &str) -> Result<Value> {
    let input = inputs.get(name).context("Missing data input")?;
    if let Some(v) = &input.value {
        Ok(v.clone())
    } else {
        plan::canonical_json(&input.artifact.path)
    }
}
fn data<T: serde::de::DeserializeOwned>(
    inputs: &BTreeMap<String, Material>,
    name: &str,
) -> Result<T> {
    Ok(serde_json::from_value(value(inputs, name)?)?)
}
fn save(path: &Path, value: &impl serde::Serialize) -> Result<()> {
    std::fs::write(path, serde_json::to_vec_pretty(value)?)?;
    Ok(())
}
fn html_program(html: &str, inputs: &BTreeMap<String, Material>) -> Result<String> {
    let mut parts = html.split("graph-resource://");
    let mut program = parts.next().unwrap_or_default().to_owned();
    for part in parts {
        let end = part
            .find(|c: char| !c.is_ascii_alphanumeric() && !":._-".contains(c))
            .unwrap_or(part.len());
        let name = &part[..end];
        let material = inputs
            .get(name)
            .filter(|_| name.starts_with("asset:"))
            .with_context(|| format!("HTML references undeclared resource {}", name))?;
        let url = reqwest::Url::from_file_path(&material.artifact.path)
            .map_err(|_| anyhow::anyhow!("Cannot address staged HTML resource"))?;
        program.push_str(url.as_str());
        program.push_str(&part[end..]);
    }
    Ok(program)
}
fn json_template(v: &Value, inputs: &BTreeMap<String, Material>) -> Result<Value> {
    match v {
        Value::Object(o) if o.contains_key("$input") => {
            if o.len() != 1 {
                bail!("$input reference must be the whole object");
            }
            let key = o["$input"]
                .as_str()
                .context("$input must name a declared input")?;
            let m = inputs.get(key).context("Unknown $input")?;
            if m.artifact.ty.media() {
                Ok(serde_json::to_value(&m.artifact)?)
            } else {
                value(inputs, key)
            }
        }
        Value::Object(o) => Ok(Value::Object(
            o.iter()
                .map(|(k, v)| Ok((k.clone(), json_template(v, inputs)?)))
                .collect::<Result<_>>()?,
        )),
        Value::Array(a) => Ok(Value::Array(
            a.iter()
                .map(|v| json_template(v, inputs))
                .collect::<Result<_>>()?,
        )),
        _ => Ok(v.clone()),
    }
}

pub async fn execute(
    task: &Task,
    inputs: &BTreeMap<String, Material>,
    directory: &Path,
    mock: bool,
) -> Result<BTreeMap<String, PathBuf>> {
    let outputs: BTreeMap<_, _> = task
        .action
        .outputs()
        .into_iter()
        .map(|(port, kind)| {
            let path = directory.join(format!("{}.{}", port, kind.extension()));
            (port, path)
        })
        .collect();
    match &task.action {
        Action::Json { value } => save(&outputs["value"], &json_template(value, inputs)?)?,
        Action::Voice {
            voice,
            language,
            direction,
        } => {
            let doc: crate::script::ScriptDocument = data(inputs, "script")?;
            if doc.segments.len() != 1 || doc.segments[0].turns.len() != 1 {
                bail!("Voice task requires exactly one Script segment/turn; split independent performances explicitly");
            }
            let segment = &doc.segments[0];
            let req = crate::voice_director::VoiceGenerationRequest {
                spec: crate::voice_director::VoicePromptSpec {
                    segment_id: segment.id.clone(),
                    role: segment.turns[0]
                        .role
                        .clone()
                        .unwrap_or_else(|| "HOST".into()),
                    spoken_text: segment.spoken_text.clone(),
                    voice_direction: direction.clone(),
                    language: language.clone(),
                    target_duration_seconds: segment.estimated_seconds,
                },
                output_path: outputs["audio"].clone(),
                voice_preset: Some(voice.clone()),
            };
            if mock {
                crate::voice_director::MockVoiceGenerator
                    .generate(&req)
                    .await?;
            } else {
                crate::voice_director::RemoteVoiceGenerator::from_environment()?
                    .generate(&req)
                    .await?;
            }
        }
        Action::ExtractFrame { at_seconds } => {
            crate::tiles::extract_reference_frame(path(inputs, "video")?, *at_seconds, &outputs["image"]).await?;
        }
        Action::Image {
            spec,
            flow_project_id,
            seed,
        } => {
            let req = crate::image_director::ImageGenerationRequest {
                spec: spec.clone(),
                output_path: outputs["image"].clone(),
                shot_id: Some(task.id.clone()),
                seed: *seed,
            };
            let refs: Vec<_> = inputs
                .iter()
                .filter(|(n, _)| n.starts_with("reference:"))
                .map(|(_, i)| i.artifact.path.clone())
                .collect();
            if mock {
                crate::image_director::MockImageGenerator.generate(&req)?;
            } else if crate::flowkit::selected("IMAGE")? {
                crate::flowkit::image_with_inputs(&req, &refs, &[], Some(flow_project_id)).await?;
            } else {
                if !refs.is_empty() {
                    bail!("Selected image adapter does not carry reference inputs");
                }
                crate::image_director::OpenAiImageGenerator::new(
                    std::env::var("OPENAI_API_KEY")?,
                    std::env::var("OPENAI_BASE_URL").ok(),
                    None,
                )
                .generate(&req)
                .await?;
            }
        }
        Action::Video {
            spec,
            flow_project_id,
            source_edit,
        } => {
            let mut spec = spec.clone();
            if spec.relationship == crate::video_director::VideoGenRelationship::SourceVideoEdit
                && inputs.contains_key("dialogue")
            {
                bail!("Flow policy prevents editing or generating speech for people in uploaded videos; no source-dialogue editing request submitted");
            }
            if inputs.contains_key("dialogue") {
                let doc: crate::script::ScriptDocument = data(inputs, "dialogue")?;
                crate::video_director::attach_dialogue(&mut spec, &doc)?;
            }
            spec.first_frame_path = inputs.get("first_frame").map(|m| m.artifact.path.clone());
            let video_inputs = crate::video_director::VideoInputs {
                source_video: inputs.get("source_video").map(|m| m.artifact.path.clone()),
                source_edit: source_edit.clone(),
                last_frame: inputs.get("last_frame").map(|m| m.artifact.path.clone()),
                reference_images: inputs
                    .iter()
                    .filter(|(n, _)| n.starts_with("reference:"))
                    .map(|(_, m)| m.artifact.path.clone())
                    .collect(),
            };
            crate::video_director::validate_inputs(&spec, &video_inputs)?;
            if (mock || !crate::flowkit::selected("VIDEO")?)
                && (video_inputs.last_frame.is_some()
                    || !video_inputs.reference_images.is_empty()
                    || video_inputs.source_video.is_some())
            {
                bail!("Advanced video references are implemented by the FlowKit adapter; no silent mock/OpenAI fallback");
            }
            let req = crate::video_director::VideoGenerationRequest {
                spec,
                output_path: outputs["video"].clone(),
                seed: None,
            };
            if mock {
                crate::video_director::MockVideoGenerator.generate(&req)?;
            } else if crate::flowkit::selected("VIDEO")? {
                crate::flowkit::video_with_inputs(&req, Some(flow_project_id), &video_inputs)
                    .await?;
            } else {
                crate::video_director::RemoteVideoGenerator::new(
                    std::env::var("OPENAI_API_KEY")?,
                    std::env::var("OPENAI_BASE_URL").ok(),
                    None,
                )
                .generate(&req)
                .await?;
            }
            if inputs.contains_key("dialogue")
                && !crate::probe::probe_media(&outputs["video"])
                    .await?
                    .has_audio
            {
                bail!("Speaking video returned no native sound; independent TTS is not a valid replacement");
            }
        }
        Action::ForceAlign {
            expected_model,
            min_word_score,
        } => {
            if mock {
                bail!("Acoustic alignment cannot be simulated as measured evidence");
            }
            let transcript = data(inputs, "transcript")?;
            let url = std::env::var("ALIGNMENT_BASE_URL")
                .unwrap_or_else(|_| crate::force_alignment::DEFAULT_URL.into());
            let doc = crate::force_alignment::align(
                path(inputs, "audio")?,
                &transcript,
                &url,
                expected_model,
                *min_word_score,
                &directory.join("alignment-response.json"),
            )
            .await?;
            save(&outputs["document"], &doc)?;
        }
        Action::AdoptPerformance { fps } => {
            return crate::performance::adopt(path(inputs, "video")?, directory, *fps).await;
        }
        Action::Transcribe {
            language,
            expected_model,
        } => {
            if !mock {
                let client = reqwest::Client::builder()
                    .timeout(std::time::Duration::from_secs(10))
                    .build()?;
                let health: Value = client
                    .get(format!(
                        "{}/health",
                        crate::transcribe::DEFAULT_LOCAL_WHISPERX_URL
                    ))
                    .send()
                    .await?
                    .error_for_status()?
                    .json()
                    .await?;
                if health["model"].as_str() != Some(expected_model.as_str()) {
                    bail!("Local ASR model differs from the explicit expected_model; no inference submitted");
                }
            }
            let mut doc =
                crate::transcribe::transcribe_media(crate::transcribe::TranscribeOptions {
                    source_path: path(inputs, "audio")?,
                    language,
                    api_key: None,
                    api_url: None,
                    local: true,
                    mock,
                })
                .await?;
            doc.evidence["source_media_digest"] =
                json!(crate::artifacts::content_digest(path(inputs, "audio")?)?);
            save(&outputs["document"], &doc)?;
        }
        Action::AlignSpeech { fps } => {
            let doc: crate::script::ScriptDocument = data(inputs, "script")?;
            let transcript = data(inputs, "transcript")?;
            let a = crate::semantic_clock::align(&doc, path(inputs, "audio")?, transcript, *fps)
                .await?;
            save(&outputs["alignment"], &a)?;
        }
        Action::ProjectTiming { fps, gap } => {
            let doc = data(inputs, "script")?;
            let layout = data(inputs, "layout")?;
            let alignments = inputs
                .keys()
                .filter_map(|n| n.strip_prefix("alignment:").map(|id| (id, n)))
                .map(|(id, n)| Ok((id.to_owned(), data(inputs, n)?)))
                .collect::<Result<HashMap<_, _>>>()?;
            let (clock, mut project) =
                crate::semantic_clock::project(&doc, &layout, &alignments, *fps, *gap)?;
            save(&outputs["clock"], &clock)?;
            // Bind the accepted JSON artifact bytes, without rehashing parsed float values.
            let clock_id = crate::artifacts::content_digest(&outputs["clock"])?;
            for voice in &mut project.tracks.voice {
                voice.payload["program_clock_id"] = json!(clock_id);
            }
            save(&outputs["project"], &project)?;
        }
        Action::Realign { fps, gap } => {
            let doc: crate::script::ScriptDocument = data(inputs, "script")?;
            let layout: crate::project::ProjectManifest = data(inputs, "layout")?;
            let mut durations = HashMap::new();
            for segment in &doc.segments {
                let name = format!("voice:{}", segment.id);
                let info = crate::probe::probe_media(path(inputs, &name)?).await?;
                if !info.has_audio {
                    bail!("{} has no audio", name);
                }
                durations.insert(segment.id.clone(), info.duration_seconds);
            }
            let (_, project) = crate::voice_director::realign_project_timeline(
                &doc, &layout, &durations, *fps, *gap,
            )?;
            save(&outputs["project"], &project)?;
        }
        Action::Mix { ducking } => {
            let doc: crate::script::ScriptDocument = data(inputs, "script")?;
            let project: crate::project::ProjectManifest = data(inputs, "project")?;
            let mut clips = Vec::new();
            for segment in &doc.segments {
                if segment.spoken_text.trim().is_empty() {
                    continue;
                }
                let input = path(inputs, &format!("voice:{}", segment.id))?;
                let voice = project
                    .tracks
                    .voice
                    .iter()
                    .find(|v| v.id == format!("voice_{}", segment.id))
                    .context("Missing aligned voice window")?;
                if let Some(digest) = voice.payload["source_media_digest"].as_str() {
                    if crate::artifacts::content_digest(input)? != digest {
                        bail!("Voice media differs from the shared alignment domain; realign the adopted take");
                    }
                }
                let duration = crate::probe::probe_media(input).await?.duration_seconds;
                let start = voice.start_seconds;
                let end = start + duration;
                if end > project.meta.total_seconds + 1.0 / project.meta.fps {
                    bail!("Voice exceeds project; re-align first");
                }
                clips.push(crate::sound_mix::AudioClip {
                    id: format!("voice_{}", segment.id),
                    track_type: crate::sound_mix::AudioTrackType::Voice,
                    source_path: input.to_owned(),
                    target_start_sample: (start * 48000.0).round() as u64,
                    target_end_sample_exclusive: (end * 48000.0).round() as u64,
                    target_start_seconds: start,
                    target_end_seconds: end,
                    gain: 1.0,
                    fade_in_samples: 0,
                    fade_out_samples: 0,
                    gain_envelope: None,
                });
            }
            let mut sfx = Vec::new();
            for item in &project.tracks.sfx {
                let Some(input) = inputs.get(&format!("sfx:{}", item.id)) else {
                    continue;
                };
                let duration = crate::probe::probe_media(&input.artifact.path)
                    .await?
                    .duration_seconds
                    .min(item.end_seconds - item.start_seconds);
                sfx.push(crate::sound_mix::AudioClip {
                    id: item.id.clone(),
                    track_type: crate::sound_mix::AudioTrackType::Sfx,
                    source_path: input.artifact.path.clone(),
                    target_start_sample: (item.start_seconds * 48000.0).round() as u64,
                    target_end_sample_exclusive: ((item.start_seconds + duration) * 48000.0).round()
                        as u64,
                    target_start_seconds: item.start_seconds,
                    target_end_seconds: item.start_seconds + duration,
                    gain: item.payload["gain"].as_f64().unwrap_or(1.0),
                    fade_in_samples: 0,
                    fade_out_samples: 0,
                    gain_envelope: None,
                });
            }
            let bgm = inputs.get("bgm").map(|m| m.artifact.path.clone());
            let mut program =
                crate::sound_mix::compile_audio_program_plan(&project, clips, bgm, sfx, ducking)?;
            crate::sound_mix::apply_observed_ducking(&mut program, ducking)?;
            crate::sound_mix::render_master_mix(&program, &outputs["audio"]).await?;
            let voices: serde_json::Map<String, Value> = doc
                .segments
                .iter()
                .filter(|s| !s.spoken_text.trim().is_empty())
                .map(|s| {
                    Ok((
                        s.id.clone(),
                        json!(crate::artifacts::content_digest(path(
                            inputs,
                            &format!("voice:{}", s.id)
                        )?)?),
                    ))
                })
                .collect::<Result<_>>()?;
            let mut details =
                crate::artifacts::details(&outputs["audio"]).unwrap_or_else(|| json!({}));
            details["voice_media_digests"] = Value::Object(voices);
            crate::artifacts::record_derived(
                &outputs["audio"],
                crate::artifacts::mode(&outputs["audio"]),
                "graph-mix",
                &inputs
                    .values()
                    .filter(|m| m.artifact.ty.media())
                    .map(|m| m.artifact.path.clone())
                    .collect::<Vec<_>>(),
                details,
            )?;
        }
        Action::CaptionSchedule { recipe } => {
            let doc: crate::script::ScriptDocument = data(inputs, "script")?;
            let transcript: crate::transcribe::TranscriptDocument = if inputs.contains_key("clock")
            {
                let clock: crate::semantic_clock::ProgramClock = data(inputs, "clock")?;
                clock.check(&doc)?;
                clock.transcript
            } else {
                data(inputs, "transcript")?
            };
            let caption = crate::caption_engine::build_caption_document_from_script(&doc)?;
            let timing =
                crate::caption_engine::build_caption_timing_from_transcript(&caption, &transcript)?;
            let schedule = crate::caption_engine::schedule_caption(&caption, &timing, recipe)?;
            save(&outputs["document"], &schedule)?;
        }
        Action::RenderCaptions { recipe } => {
            let recipe_path = directory.join("recipe.json");
            save(&recipe_path, recipe)?;
            crate::production::run(crate::production::MediaCommands::RenderCaptions {
                project: path(inputs, "project")?.to_owned(),
                schedule: path(inputs, "schedule")?.to_owned(),
                recipe: Some(recipe_path),
                output: outputs["video"].clone(),
                update_project: outputs["project"].clone(),
                mock,
            })
            .await?;
        }
        Action::RenderHtml { frames, config } => {
            // Resources are explicit inputs so their content participates in invalidation.
            // Staging changes the HTML directory; never rely on its old relative base URL.
            let mut html = html_program(&std::fs::read_to_string(path(inputs, "html")?)?, inputs)?;
            if inputs.contains_key("clock") {
                let clock: crate::semantic_clock::ProgramClock = data(inputs, "clock")?;
                clock.validate()?;
                if clock.format != "clone-any-video.program-clock@1"
                    || clock.total_frames != *frames
                    || (clock.fps - config.fps).abs() > 1e-6
                {
                    bail!("Clock-driven HTML must cover the full program with the same fps");
                }
                let script = format!(
                    "<script>window.PRODUCTION_CLOCK={};</script>",
                    serde_json::to_string(&clock)?.replace("<", "\\u003c")
                );
                if html.contains("</head>") {
                    html = html.replacen("</head>", &format!("{}</head>", script), 1);
                } else {
                    html = script + &html;
                }
            }
            let html_path = directory.join("program.html");
            std::fs::write(&html_path, html)?;
            let provider = crate::browser_capture::resolve_browser_provider(mock)?;
            crate::browser_capture::render_html_to_mp4(
                &html_path,
                *frames,
                config,
                &outputs["video"],
                &provider,
            )
            .await?;
            if inputs.contains_key("clock") {
                let mut details =
                    crate::artifacts::details(&outputs["video"]).unwrap_or_else(|| json!({}));
                details["frame_domain"] = json!("program");
                details["fps"] = json!(config.fps);
                details["total_frames"] = json!(frames);
                details["program_clock_id"] = json!(inputs["clock"].artifact.digest);
                crate::artifacts::record_derived(
                    &outputs["video"],
                    crate::artifacts::mode(&outputs["video"]),
                    "clock-driven-html",
                    &[path(inputs, "clock")?.to_owned()],
                    details,
                )?;
            }
        }
        Action::Compose => {
            let mut project: crate::project::ProjectManifest = data(inputs, "project")?;
            for shot in &project.shots {
                let name = format!("background:{}", shot.id);
                let source = path(inputs, &name)?;
                let mut payload = project
                    .tracks
                    .visual_bg
                    .iter()
                    .find(|v| {
                        v.payload["shot_id"] == shot.id
                            || v.id == format!("bg_{}", shot.id)
                            || (v.start_frame == shot.start_frame && v.end_frame == shot.end_frame)
                    })
                    .map(|v| v.payload.clone())
                    .unwrap_or_else(|| json!({}));
                payload["source"] = json!(source);
                let input = &inputs[&name].artifact;
                if let Some(relation) = input.details.get("performance") {
                    let p: crate::performance::Relation = serde_json::from_value(relation.clone())?;
                    let voice = project
                        .tracks
                        .voice
                        .iter()
                        .find(|v| v.id == format!("voice_{}", shot.segment_id))
                        .context("Paired picture has no speaking source")?;
                    if p.video_digest != input.digest
                        || p.fps != project.meta.fps
                        || voice.payload["source_media_digest"] != p.audio_digest
                        || inputs["audio"].artifact.details["voice_media_digests"][&shot.segment_id]
                            != p.audio_digest
                    {
                        bail!("Visible performance must retain its own adopted audio in the master mix");
                    }
                    let from = shot
                        .start_frame
                        .checked_sub(voice.start_frame)
                        .context("Performance picture precedes its voice window")?;
                    let until = from + shot.end_frame - shot.start_frame;
                    if until > p.frames {
                        bail!("Paired performance picture exceeds its local domain");
                    }
                    let expected = crate::sampling::Sampling {
                        from_frame: from,
                        until_frame: Some(until),
                        ..Default::default()
                    };
                    if let Some(value) = payload.get("sampling") {
                        let selected: crate::sampling::Sampling =
                            serde_json::from_value(value.clone())?;
                        if selected != expected {
                            bail!("Retiming visible performance requires preparing both picture and sound together");
                        }
                    }
                    payload["sampling"] = serde_json::to_value(expected)?;
                }
                let row = crate::project::TrackItem {
                    id: format!("bg_{}", shot.id),
                    name: shot.id.clone(),
                    start_seconds: shot.start_seconds,
                    end_seconds: shot.end_seconds,
                    start_frame: shot.start_frame,
                    end_frame: shot.end_frame,
                    payload,
                };
                project
                    .tracks
                    .visual_bg
                    .retain(|v| v.start_frame != shot.start_frame || v.end_frame != shot.end_frame);
                project.tracks.visual_bg.push(row);
            }
            for item in &mut project.tracks.visual_components {
                let name = format!("overlay:{}", item.id);
                item.payload["video_path"] = json!(path(inputs, &name)?);
                if inputs[&name].artifact.details["frame_domain"] == "program" {
                    let d = &inputs[&name].artifact.details;
                    let expected = project
                        .tracks
                        .voice
                        .first()
                        .map(|v| v.payload["program_clock_id"].clone())
                        .unwrap_or(Value::Null);
                    if d["fps"].as_f64() != Some(project.meta.fps)
                        || d["total_frames"].as_u64() != Some(project.meta.total_frames)
                        || d["program_clock_id"] != expected
                    {
                        bail!("Program-domain component belongs to a different semantic clock");
                    }
                }
                if inputs[&name].artifact.details["frame_domain"] == "program"
                    && item.payload.get("sampling").is_none()
                {
                    item.payload["sampling"] =
                        json!({"from_frame":item.start_frame,"until_frame":item.end_frame});
                }
            }
            project.master_audio = Some(path(inputs, "audio")?.to_owned());
            save(&outputs["project"], &project)?;
            let plan = crate::compositor::compile_project_to_composite_plan(
                &project,
                directory,
                &outputs["video"],
            )?;
            crate::compositor::render_composition(&plan, false).await?;
        }
        Action::Review { require_delivery } => {
            let project: crate::project::ProjectManifest = data(inputs, "project")?;
            let config = crate::video_agent::ReviewConfig {
                expected_width: project.meta.width,
                expected_height: project.meta.height,
                expected_fps: project.meta.fps,
                mode: Some(if mock { "mock" } else { "production" }.into()),
                evidence: inputs.get("evidence").map(|i| i.artifact.path.clone()),
                ..Default::default()
            };
            let report = crate::video_agent::review_project_and_master(
                &project,
                path(inputs, "video")?,
                &config,
            )
            .await?;
            save(&outputs["report"], &report)?;
            if !report.passed || (*require_delivery && !report.ready_for_delivery) {
                bail!(
                    "Review gate failed; diagnostic retained at {:?}",
                    outputs["report"]
                );
            }
        }
    }
    Ok(outputs)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn html_resources_bind_exact_declared_names() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("product image.png");
        let material = Material {
            artifact: Artifact {
                ty: Kind::Image,
                path: path.clone(),
                digest: "fixture".into(),
                mode: crate::artifacts::Mode::Mock,
                details: Value::Null,
            },
            value: None,
        };
        let inputs = BTreeMap::from([
            ("asset:prod".into(), material.clone()),
            ("asset:product".into(), material),
        ]);
        let program = html_program("<img src='graph-resource://asset:product'>", &inputs).unwrap();
        assert_eq!(
            program,
            format!(
                "<img src='{}'>",
                reqwest::Url::from_file_path(path).unwrap()
            )
        );
        assert!(html_program("<img src='graph-resource://asset:products'>", &inputs).is_err());
    }
}
