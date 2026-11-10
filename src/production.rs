//! Native preparation commands: authored edits produce measured, traceable media.
use crate::{artifacts, probe::probe_media};
use anyhow::{bail, Context, Result};
use clap::Subcommand;
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

#[derive(Subcommand, Debug)]
pub enum MediaCommands {
    /// Measure word times with a real language-specific acoustic model.
    ForceAlign {
        #[arg(long)]
        audio: PathBuf,
        #[arg(long)]
        transcript: PathBuf,
        #[arg(long)]
        output: PathBuf,
        #[arg(long, default_value = "http://127.0.0.1:8766")]
        service_url: String,
        #[arg(long, default_value = "WAV2VEC2_ASR_BASE_960H")]
        expected_model: String,
        #[arg(long, default_value = "0")]
        min_word_score: f64,
    },
    /// Normalize and pair the picture and actual sound of one performance.
    AdoptPerformance {
        #[arg(long)]
        input: PathBuf,
        #[arg(long)]
        out_dir: PathBuf,
        #[arg(long, default_value = "30")]
        fps: f64,
    },

    /// Edit actual audio; retain source and record the measured result and parameters.
    PrepareAudio {
        #[arg(long)]
        input: PathBuf,
        #[arg(long)]
        output: PathBuf,
        #[arg(long, default_value = "1.0")]
        tempo: f64,
        #[arg(long)]
        trim_silence: bool,
        #[arg(long, allow_hyphen_values = true)]
        target_lufs: Option<f64>,
        #[arg(long, default_value = "-1.0", allow_hyphen_values = true)]
        max_true_peak: f64,
    },
    /// Explicit lexical corrections and temporal projection; never relabel as forced alignment.
    ReviseTranscript {
        #[arg(long)]
        input: PathBuf,
        #[arg(long)]
        output: PathBuf,
        #[arg(long)]
        corrections: Option<PathBuf>,
        #[arg(long, default_value = "1.0")]
        tempo: f64,
        #[arg(long, default_value = "0.0")]
        offset: f64,
        #[arg(long)]
        audio: Option<PathBuf>,
    },
    /// Render the entire caption schedule via real Chrome into a transparent video layer.
    RenderCaptions {
        #[arg(long)]
        project: PathBuf,
        #[arg(long)]
        schedule: PathBuf,
        #[arg(long)]
        recipe: Option<PathBuf>,
        #[arg(long)]
        output: PathBuf,
        #[arg(long)]
        update_project: PathBuf,
        #[arg(long)]
        mock: bool,
    },
    InspectBoundaries {
        #[arg(long)]
        project: PathBuf,
        #[arg(long)]
        video: PathBuf,
        #[arg(long)]
        out_dir: PathBuf,
    },
    /// Import the actual image bytes selected by a completed FlowKit receipt.
    ImportFlowkitImage {
        #[arg(long)]
        receipt: PathBuf,
        #[arg(long)]
        output: PathBuf,
        #[arg(long, default_value = "0")]
        index: usize,
    },
    /// Normalize once, then execute explicit half-open source/target frame ranges.
    Edit {
        #[arg(long)]
        plan: PathBuf,
        #[arg(long)]
        project: PathBuf,
        #[arg(long)]
        out_dir: PathBuf,
        #[arg(long)]
        update_project: PathBuf,
    },
}
fn write(path: &Path, v: &impl serde::Serialize) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, serde_json::to_vec_pretty(v)?)?;
    Ok(())
}
fn distinct(input: &Path, output: &Path) -> Result<()> {
    if input == output || output.exists() && input.canonicalize()? == output.canonicalize()? {
        bail!("Prepared output must differ from its accepted source");
    }
    Ok(())
}
#[derive(Debug, Deserialize, serde::Serialize)]
struct Correction {
    passage: usize,
    word: usize,
    expected: String,
    replacement: String,
    reason: String,
}

pub async fn run(command: MediaCommands) -> Result<()> {
    match command {
        MediaCommands::ForceAlign {
            audio,
            transcript,
            output,
            service_url,
            expected_model,
            min_word_score,
        } => {
            distinct(&audio, &output)?;
            distinct(&transcript, &output)?;
            let doc = serde_json::from_slice(&std::fs::read(&transcript)?)?;
            let aligned = crate::force_alignment::align(
                &audio,
                &doc,
                &service_url,
                &expected_model,
                min_word_score,
                &output.with_extension("alignment-response.json"),
            )
            .await?;
            write(&output, &aligned)?;
        }
        MediaCommands::AdoptPerformance {
            input,
            out_dir,
            fps,
        } => {
            let outputs = crate::performance::adopt(&input, &out_dir, fps).await?;
            println!("{}", serde_json::to_string_pretty(&outputs)?);
        }

        MediaCommands::PrepareAudio {
            input,
            output,
            tempo,
            trim_silence,
            target_lufs,
            max_true_peak,
        } => {
            distinct(&input, &output)?;
            if !tempo.is_finite() || !(0.5..=2.0).contains(&tempo) {
                bail!("tempo must be 0.5..2.0");
            }
            let original = probe_media(&input).await?;
            if !original.has_audio {
                bail!("Input has no audio");
            }
            let mut filters = Vec::new();
            if trim_silence {
                filters.push("silenceremove=start_periods=1:start_duration=0.02:start_threshold=-45dB:start_silence=0.04:stop_periods=-1:stop_duration=0.4:stop_threshold=-45dB:stop_silence=0.12".to_owned());
            }
            filters.push(format!("atempo={}", tempo));
            if let Some(lufs) = target_lufs {
                if !(-30.0..=-9.0).contains(&lufs) || !(-9.0..=0.0).contains(&max_true_peak) {
                    bail!("Invalid mastering targets");
                }
                filters.push(format!("loudnorm=I={}:TP={}:LRA=9", lufs, max_true_peak));
            }
            let parent = output
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new("."));
            std::fs::create_dir_all(parent)?;
            let stage = tempfile::Builder::new()
                .suffix(".wav")
                .tempfile_in(parent)?;
            let result = tokio::process::Command::new("ffmpeg")
                .args(["-v", "error", "-y", "-i"])
                .arg(&input)
                .args([
                    "-vn",
                    "-af",
                    &filters.join(","),
                    "-ar",
                    "48000",
                    "-ac",
                    "2",
                    "-c:a",
                    "pcm_s16le",
                ])
                .arg(stage.path())
                .output()
                .await?;
            if !result.status.success() {
                bail!(
                    "Audio preparation failed: {}",
                    String::from_utf8_lossy(&result.stderr)
                );
            }
            let measured = probe_media(stage.path()).await?;
            if measured.duration_seconds <= 0.0 {
                bail!("Prepared audio is empty");
            }
            stage.persist(&output).map_err(|e| e.error)?;
            let mut metadata: Value = std::fs::read(input.with_extension("json"))
                .ok()
                .and_then(|s| serde_json::from_slice(&s).ok())
                .unwrap_or(json!({}));
            metadata["output_path"] = json!(output);
            metadata["duration_seconds"] = json!(measured.duration_seconds);
            metadata["sample_rate"] = json!(48000);
            metadata["channels"] = json!(2);
            let mut detail = json!({"source":input,"source_digest":artifacts::content_digest(&input)?,"source_seconds":original.duration_seconds,"measured_seconds":measured.duration_seconds,"tempo":tempo,"silence_edit":trim_silence,"filter":filters,"target_lufs":target_lufs,"alignment_invalidated":true});
            if tempo == 1.0 && !trim_silence {
                if let Some(original) = artifacts::details(&input) {
                    if let Some(voices) = original.get("voice_media_digests") {
                        detail["voice_media_digests"] = voices.clone();
                    }
                }
            }
            metadata["preparation"] = detail.clone();
            write(&output.with_extension("json"), &metadata)?;
            artifacts::record_derived(
                &output,
                artifacts::mode(&input),
                "audio-preparation",
                &[input],
                detail,
            )?;
            println!("{}", serde_json::to_string_pretty(&metadata)?);
        }
        MediaCommands::ReviseTranscript {
            input,
            output,
            corrections,
            tempo,
            offset,
            audio,
        } => {
            distinct(&input, &output)?;
            if !tempo.is_finite() || tempo <= 0.0 || !offset.is_finite() || offset < 0.0 {
                bail!("Invalid evidence time transform");
            }
            if tempo != 1.0 && audio.is_none() {
                bail!("Retimed transcript needs --audio pointing to the actual edited audio; silence edits need fresh ASR");
            }
            let mut document: crate::transcribe::TranscriptDocument =
                serde_json::from_slice(&std::fs::read(&input)?)?;
            let changes: Vec<Correction> = if let Some(p) = &corrections {
                serde_json::from_slice(&std::fs::read(p)?)?
            } else {
                Vec::new()
            };
            for change in &changes {
                if change.reason.trim().is_empty() || change.replacement.trim().is_empty() {
                    bail!("Corrections require text and a reason");
                }
                let passage = document
                    .passages
                    .get_mut(change.passage)
                    .context("Correction passage missing")?;
                let word = passage
                    .words
                    .get_mut(change.word)
                    .context("Correction word missing")?;
                if word.text != change.expected {
                    bail!(
                        "Correction input mismatch: expected {:?}, found {:?}",
                        change.expected,
                        word.text
                    );
                }
                word.text = change.replacement.clone();
                word.score = None;
                passage.text = passage
                    .words
                    .iter()
                    .map(|w| w.text.as_str())
                    .collect::<Vec<_>>()
                    .join(" ");
            }
            for passage in &mut document.passages {
                for time in [&mut passage.start_seconds, &mut passage.end_seconds] {
                    if let Some(t) = time {
                        *t = *t / tempo + offset;
                    }
                }
                for word in &mut passage.words {
                    for time in [&mut word.start_seconds, &mut word.end_seconds] {
                        if let Some(t) = time {
                            *t = *t / tempo + offset;
                        }
                    }
                    if let (Some(start), Some(end)) = (word.start_seconds, word.end_seconds) {
                        if !start.is_finite() || !end.is_finite() || start < 0.0 || end < start {
                            bail!("Invalid mapped word window");
                        }
                    }
                }
            }
            let parent_evidence = document.evidence.clone();
            document.audio_seconds = document.audio_seconds / tempo + offset;
            let audio_fact = if let Some(audio) = audio {
                let preparation: Value = std::fs::read(audio.with_extension("json"))
                    .ok()
                    .and_then(|bytes| serde_json::from_slice(&bytes).ok())
                    .unwrap_or(Value::Null);
                if tempo != 1.0
                    && (preparation["preparation"]["silence_edit"] == true
                        || artifacts::details(&audio).is_some_and(|d| d["silence_edit"] == true))
                {
                    bail!("Silence/cut edits cannot use a global tempo projection; re-transcribe the edited media first");
                }
                let actual = probe_media(&audio).await?;
                if !actual.has_audio {
                    bail!("Mapped source has no audio");
                }
                let end = actual.duration_seconds + offset;
                if document
                    .passages
                    .iter()
                    .flat_map(|p| &p.words)
                    .any(|w| w.end_seconds.is_some_and(|t| t > end + 0.05))
                {
                    bail!("Mapped evidence exceeds the edited media; re-transcribe after silence/cut edits");
                }
                document.audio_seconds = end;
                document.source = audio.to_string_lossy().into();
                json!({"path":audio,"digest":artifacts::content_digest(&audio)?,"seconds":actual.duration_seconds})
            } else {
                Value::Null
            };
            document.evidence = json!({"kind":"curated/projected-asr","forced_alignment":false,"parent":input,"parent_digest":artifacts::content_digest(&input)?,"parent_evidence":parent_evidence,"corrections":changes,"mapping":{"tempo":tempo,"offset":offset,"audio":audio_fact}});
            if let Some(digest) = audio_fact["digest"].as_str() {
                document.evidence["source_media_digest"] = json!(digest);
            }
            write(&output, &document)?;
            println!("Saved explicit evidence revision {:?}", output);
        }
        MediaCommands::RenderCaptions {
            project,
            schedule,
            recipe,
            output,
            update_project,
            mock,
        } => {
            let mut manifest: crate::project::ProjectManifest =
                serde_json::from_slice(&std::fs::read(&project)?)?;
            let doc: crate::caption_engine::CaptionSchedule =
                serde_json::from_slice(&std::fs::read(&schedule)?)?;
            let style: crate::caption_engine::CaptionRecipe = if let Some(p) = &recipe {
                serde_json::from_slice(&std::fs::read(p)?)?
            } else {
                let mut v = crate::caption_engine::CaptionRecipe::default();
                v.typography.font_size_px = manifest.meta.height as f64 * 0.053;
                v
            };
            for cue in &doc.cues {
                if cue.visible_start < 0.0
                    || cue.visible_end > manifest.meta.total_seconds + 1.0 / manifest.meta.fps
                    || cue.visible_end <= cue.visible_start
                {
                    bail!("Caption cue outside canvas clock: {}", cue.cue_id);
                }
            }
            let mut html = crate::caption_engine::export_to_html_overlay(
                &doc,
                &style,
                manifest.meta.width,
                manifest.meta.height,
            );
            let driver=format!("<script>window.renderFrame=(frame)=>window.renderCaptionFrame(frame/{});</script></body>",manifest.meta.fps);
            html = html.replace("</body>", &driver);
            let page = output.with_extension("html");
            if let Some(p) = page.parent() {
                std::fs::create_dir_all(p)?;
            }
            std::fs::write(&page, html)?;
            let config = crate::browser_capture::CaptureConfig {
                width: manifest.meta.width,
                height: manifest.meta.height,
                fps: manifest.meta.fps,
                transparent: true,
                ..Default::default()
            };
            let provider = crate::browser_capture::resolve_browser_provider(mock)?;
            crate::browser_capture::render_html_to_mp4(
                &page,
                manifest.meta.total_frames,
                &config,
                &output,
                &provider,
            )
            .await?;
            let cues: Vec<String> = doc.cues.iter().map(|c| c.cue_id.clone()).collect();
            let caption_windows: Vec<_> = doc
                .cues
                .iter()
                .map(|c| crate::delivery::CaptionFact {
                    id: c.cue_id.clone(),
                    text: c.display_text.clone(),
                    start_frame: crate::delivery::caption_frame(
                        c.visible_start,
                        manifest.meta.fps,
                        manifest.meta.total_frames,
                    ),
                    end_frame: crate::delivery::caption_frame(
                        c.visible_end,
                        manifest.meta.fps,
                        manifest.meta.total_frames,
                    ),
                })
                .collect();
            if caption_windows.iter().any(|c| c.end_frame <= c.start_frame) {
                bail!("Caption has no visible frames on canvas");
            }
            let mut inputs = vec![schedule.clone(), page.clone()];
            inputs.extend(recipe.clone());
            artifacts::record_derived(
                &output,
                if mock {
                    artifacts::Mode::Mock
                } else {
                    artifacts::Mode::Production
                },
                "caption-renderer",
                &inputs,
                json!({"caption_cues":cues,"caption_windows":caption_windows,"fps":manifest.meta.fps,"schedule_digest":artifacts::content_digest(&schedule)?,"frames":manifest.meta.total_frames,"precision":"as declared by schedule; semantic alignment unverified"}),
            )?;
            manifest
                .tracks
                .visual_components
                .retain(|c| c.payload["role"] != "caption");
            manifest.tracks.visual_components.push(crate::project::TrackItem{id:"caption_overlay".into(),name:"Authored caption schedule".into(),start_seconds:0.0,end_seconds:manifest.meta.total_seconds,start_frame:0,end_frame:manifest.meta.total_frames,payload:json!({"role":"caption","file_path":page.canonicalize()?,"video_path":output.canonicalize()?,"duration_frames":manifest.meta.total_frames})});
            manifest.tracks.captions = doc
                .cues
                .iter()
                .zip(&caption_windows)
                .map(|(c,fact)| crate::project::TrackItem {
                    id: c.cue_id.clone(),
                    name: c.display_text.clone(),
                    start_seconds: c.visible_start,
                    end_seconds: c.visible_end,
                    start_frame:fact.start_frame,
                    end_frame:fact.end_frame,
                    payload: json!({"rendered_by":"caption_overlay","schedule":schedule,"text":c.display_text}),
                })
                .collect();
            write(&update_project, &manifest)?;
            println!(
                "Rendered {} cues as real transparent pixels",
                doc.cues.len()
            );
        }
        MediaCommands::InspectBoundaries {
            project,
            video,
            out_dir,
        } => {
            let project = serde_json::from_slice(&std::fs::read(project)?)?;
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &crate::delivery::inspect_boundaries(&project, &video, &out_dir).await?
                )?
            );
        }
        MediaCommands::ImportFlowkitImage {
            receipt,
            output,
            index,
        } => crate::flowkit::import_image(&receipt, &output, index).await?,
        MediaCommands::Edit {
            plan,
            project,
            out_dir,
            update_project,
        } => crate::edit::render(&plan, &project, &out_dir, &update_project).await?,
    }
    Ok(())
}
